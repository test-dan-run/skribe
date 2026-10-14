//! Forward-only schema migrations.
//!
//! A migration of existing data runs in this order: snapshot the database, verify the snapshot,
//! record an in-progress marker, apply every pending migration in one transaction, clear the
//! marker. A failure rolls the transaction back; if the database is then not verifiably in its
//! pre-migration state the snapshot is restored. A marker found at startup means the previous
//! run was interrupted (see `library::recover_interrupted`).

use std::fs;
use std::path::Path;

use rusqlite::{Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use skribe_domain::LibraryId;

use crate::error::FailedMigrationOutcome;
use crate::{StorageRoots, StoreError, db, fsutil, snapshot};

#[derive(Debug)]
pub(crate) struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

/// Every schema version in order. Shipped entries are never edited or removed; schema changes
/// are appended.
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "library_meta",
        sql: include_str!("../migrations/0001_library_meta.sql"),
    },
    Migration {
        version: 2,
        name: "meetings",
        sql: include_str!("../migrations/0002_meetings.sql"),
    },
    Migration {
        version: 3,
        name: "recording",
        sql: include_str!("../migrations/0003_recording.sql"),
    },
];

/// The schema this build reads and writes.
pub fn latest_schema_version() -> u32 {
    latest_version(MIGRATIONS)
}

pub(crate) fn latest_version(migrations: &[Migration]) -> u32 {
    migrations.last().map_or(0, |migration| migration.version)
}

/// A migration that was applied while opening the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    pub from: u32,
    pub to: u32,
    /// The verified pre-migration snapshot in `backups/`. `None` for a new, empty database.
    pub snapshot: Option<String>,
}

/// Written before a migration starts and removed when it ends, so an interrupted run is
/// detected on the next start and knows which snapshot holds the pre-migration data.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MigrationMarker {
    pub from_version: u32,
    pub to_version: u32,
    pub snapshot: String,
    pub app_version: String,
    pub started_at_unix_ms: u64,
}

impl MigrationMarker {
    pub(crate) fn load(path: &Path) -> Result<Option<Self>, StoreError> {
        if !fsutil::exists(path)? {
            return Ok(None);
        }
        let bytes = fs::read(path).map_err(StoreError::io("read", path))?;
        let marker = serde_json::from_slice(&bytes).map_err(|error| {
            StoreError::MigrationRecoveryFailed {
                detail: format!("{} is unreadable: {error}", path.display()),
                snapshot: None,
            }
        })?;
        Ok(Some(marker))
    }

    pub(crate) fn write(&self, path: &Path) -> Result<(), StoreError> {
        let json = serde_json::to_vec_pretty(self).map_err(|error| StoreError::Io {
            action: "encode",
            path: path.to_owned(),
            source: error.into(),
        })?;
        fsutil::write_atomic(path, &json)
    }

    pub(crate) fn clear(path: &Path) -> Result<(), StoreError> {
        if fsutil::exists(path)? {
            fs::remove_file(path).map_err(StoreError::io("remove", path))?;
            if let Some(parent) = path.parent() {
                fsutil::sync_dir(parent)?;
            }
        }
        Ok(())
    }
}

pub(crate) struct MigrationContext<'a> {
    pub roots: &'a StorageRoots,
    pub migrations: &'a [Migration],
    pub library_id: &'a LibraryId,
    pub app_version: &'a str,
}

/// Brings the database at schema `from` up to the latest schema. `from` is the version read by
/// [`db::inspect`]; zero means a new, empty database, which also receives its identity row.
pub(crate) fn migrate(
    mut conn: Connection,
    ctx: &MigrationContext<'_>,
    from: u32,
) -> Result<(Connection, Option<MigrationReport>), StoreError> {
    let to = latest_version(ctx.migrations);
    if from == to {
        return Ok((conn, None));
    }
    let pending: Vec<&Migration> = ctx
        .migrations
        .iter()
        .filter(|migration| migration.version > from)
        .collect();

    if from == 0 {
        // Nothing to lose yet, so nothing to snapshot: a failure leaves the database empty.
        let identity = NewIdentity {
            library_id: ctx.library_id.to_string(),
            created_at_unix_ms: i64::try_from(fsutil::now_unix_ms()).unwrap_or(i64::MAX),
            app_version: ctx.app_version,
        };
        apply(&mut conn, &pending, Some(&identity)).map_err(|detail| {
            StoreError::MigrationFailed {
                from,
                to,
                detail,
                outcome: FailedMigrationOutcome::RolledBack,
                snapshot: None,
            }
        })?;
        return Ok((
            conn,
            Some(MigrationReport {
                from,
                to,
                snapshot: None,
            }),
        ));
    }

    let snapshot = snapshot::create(&conn, ctx.roots, from, ctx.library_id)?;
    let marker_path = ctx.roots.migration_marker_path();
    MigrationMarker {
        from_version: from,
        to_version: to,
        snapshot: snapshot.name.clone(),
        app_version: ctx.app_version.to_owned(),
        started_at_unix_ms: fsutil::now_unix_ms(),
    }
    .write(&marker_path)?;

    let Err(detail) = apply(&mut conn, &pending, None) else {
        MigrationMarker::clear(&marker_path)?;
        return Ok((
            conn,
            Some(MigrationReport {
                from,
                to,
                snapshot: Some(snapshot.name),
            }),
        ));
    };

    let rolled_back = db::user_version(&conn).is_ok_and(|version| version == from)
        && db::integrity_problems(&conn).is_ok_and(|problems| problems.is_empty());
    let outcome = if rolled_back {
        FailedMigrationOutcome::RolledBack
    } else {
        drop(conn);
        let latest = latest_version(ctx.migrations);
        match snapshot::restore(ctx.roots, &snapshot.name, ctx.library_id, latest) {
            Ok(set_aside) => FailedMigrationOutcome::RestoredFromSnapshot { set_aside },
            // The marker stays so the next start tries the restore again.
            Err(error) => {
                return Err(StoreError::MigrationRecoveryFailed {
                    detail: format!("{detail}; restoring the snapshot also failed: {error}"),
                    snapshot: Some(snapshot.name),
                });
            }
        }
    };
    MigrationMarker::clear(&marker_path)?;
    Err(StoreError::MigrationFailed {
        from,
        to,
        detail,
        outcome,
        snapshot: Some(snapshot.name),
    })
}

struct NewIdentity<'a> {
    library_id: String,
    // SQLite integers are signed 64-bit.
    created_at_unix_ms: i64,
    app_version: &'a str,
}

/// Applies `pending` in one transaction. On error the transaction has rolled back and the
/// returned text names the migration that failed.
fn apply(
    conn: &mut Connection,
    pending: &[&Migration],
    identity: Option<&NewIdentity<'_>>,
) -> Result<(), String> {
    // SQLite ignores this pragma inside a transaction. Enforcement is off while tables are
    // rebuilt and the whole database is checked before commit instead.
    conn.pragma_update(None, "foreign_keys", false)
        .map_err(|error| error.to_string())?;
    let result = apply_in_transaction(conn, pending, identity);
    let restored = conn
        .pragma_update(None, "foreign_keys", true)
        .map_err(|e| e.to_string());
    result.and(restored)
}

fn apply_in_transaction(
    conn: &mut Connection,
    pending: &[&Migration],
    identity: Option<&NewIdentity<'_>>,
) -> Result<(), String> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    for migration in pending {
        let describe = |error: rusqlite::Error| {
            format!(
                "migration {} ({}): {error}",
                migration.version, migration.name
            )
        };
        tx.execute_batch(migration.sql).map_err(describe)?;
        tx.pragma_update(None, "user_version", migration.version)
            .map_err(describe)?;
    }
    if let Some(identity) = identity {
        tx.execute(
            "INSERT INTO library_meta (id, library_id, created_at_unix_ms, created_by_app_version) \
             VALUES (1, ?1, ?2, ?3)",
            (
                &identity.library_id,
                identity.created_at_unix_ms,
                identity.app_version,
            ),
        )
        .map_err(|error| format!("recording the library identity: {error}"))?;
    }
    let violations = tx
        .prepare("PRAGMA foreign_key_check")
        .and_then(|mut statement| statement.exists([]))
        .map_err(|error| error.to_string())?;
    if violations {
        return Err("the migrated data violates a foreign key".to_owned());
    }
    tx.commit().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_migrations_are_numbered_from_one_without_gaps() {
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            assert_eq!(migration.version as usize, index + 1, "{}", migration.name);
        }
    }
}
