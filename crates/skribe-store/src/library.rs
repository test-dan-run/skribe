use std::path::PathBuf;

use skribe_domain::LibraryId;

use crate::lock::LibraryLock;
use crate::migrate::{
    self, MIGRATIONS, Migration, MigrationContext, MigrationMarker, MigrationReport,
};
use crate::worker::Database;
use crate::{
    LibraryManifest, SnapshotInfo, StorageRoots, StoreError, db, fsutil, recording, snapshot,
};

/// How an interrupted migration found at startup was resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterruptedMigrationRecovery {
    /// The migration had committed; only its marker was left behind.
    Completed,
    /// The migration had not committed; the database was intact at its previous schema.
    RolledBack,
    /// The database was unusable and the pre-migration snapshot was restored.
    RestoredFromSnapshot {
        snapshot: String,
        set_aside: Option<PathBuf>,
    },
}

/// What opening the library had to do, for the interface to report truthfully.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenReport {
    /// The library did not exist and was created.
    pub created: bool,
    pub recovery: Option<InterruptedMigrationRecovery>,
    pub migration: Option<MigrationReport>,
    /// Recordings that were under way when the library was last closed. Each is now
    /// `interrupted` and can be recovered from what reached the disk.
    pub interrupted_recordings: u32,
}

/// The result of replacing the database with a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    pub snapshot: String,
    /// Where the replaced database was moved, when one existed.
    pub set_aside: Option<PathBuf>,
}

/// The library's identity as recorded in its database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryDescription {
    pub library_id: String,
    pub created_at_unix_ms: u64,
    pub created_by_app_version: String,
    pub schema_version: u32,
}

/// An open meeting library: an exclusive lock on the folder, a verified manifest and a
/// migrated database behind its worker thread.
#[derive(Debug)]
pub struct Library {
    roots: StorageRoots,
    manifest: LibraryManifest,
    schema_version: u32,
    report: OpenReport,
    // Field order is drop order: the database closes before the lock is released.
    db: Database,
    _lock: LibraryLock,
}

impl Library {
    /// Opens the library, creating it when the folder is new. Blocking: integrity checks and
    /// migrations run here, so call it off the UI thread.
    pub fn open(roots: &StorageRoots, app_version: &str) -> Result<Self, StoreError> {
        Self::open_with(roots, app_version, MIGRATIONS)
    }

    pub(crate) fn open_with(
        roots: &StorageRoots,
        app_version: &str,
        migrations: &[Migration],
    ) -> Result<Self, StoreError> {
        roots.create()?;
        let lock = LibraryLock::acquire(&roots.lock_path())?;
        let latest = migrate::latest_version(migrations);
        let db_path = roots.db_path();
        let marker = MigrationMarker::load(&roots.migration_marker_path())?;

        let manifest_path = roots.manifest_path();
        let mut report = OpenReport::default();
        let manifest = if fsutil::exists(&manifest_path)? {
            LibraryManifest::load(&manifest_path)?
        } else if fsutil::exists(&db_path)? || marker.is_some() || roots.library_has_content()? {
            // Inventing a new identity here would detach the folder's data from its library.
            return Err(StoreError::ManifestMissing {
                path: manifest_path,
            });
        } else {
            report.created = true;
            LibraryManifest::create(&manifest_path, app_version)?
        };

        if let Some(marker) = marker {
            report.recovery = Some(recover_interrupted(
                roots,
                &marker,
                &manifest.library_id,
                latest,
            )?);
        }

        let from = if fsutil::exists(&db_path)? {
            let inspection = db::inspect(&db_path)?;
            check_identity(&db_path, &inspection, &manifest.library_id, latest)?;
            inspection.schema_version
        } else if roots.library_has_content()? {
            // A new, empty database would silently hide the audio and snapshots that remain.
            return Err(StoreError::DatabaseMissing { path: db_path });
        } else {
            0
        };

        let conn = db::open(&db_path)?;
        let ctx = MigrationContext {
            roots,
            migrations,
            library_id: &manifest.library_id,
            app_version,
        };
        let (mut conn, migration) = migrate::migrate(conn, &ctx, from)?;
        report.migration = migration;
        // The lock above means no other process is recording into this library.
        report.interrupted_recordings = recording::settle_orphaned_recordings(&mut conn)?;

        Ok(Self {
            roots: roots.clone(),
            manifest,
            schema_version: latest,
            report,
            db: Database::start(conn)?,
            _lock: lock,
        })
    }

    /// Replaces the database of a library that is not open with the named snapshot from its
    /// `backups/` folder. The snapshot is verified first and the replaced file is kept.
    pub fn restore_snapshot(roots: &StorageRoots, name: &str) -> Result<RestoreReport, StoreError> {
        roots.create()?;
        let _lock = LibraryLock::acquire(&roots.lock_path())?;
        let manifest = LibraryManifest::load(&roots.manifest_path())?;
        let latest = migrate::latest_version(MIGRATIONS);
        let set_aside = snapshot::restore(roots, name, &manifest.library_id, latest)?;
        Ok(RestoreReport {
            snapshot: name.to_owned(),
            set_aside,
        })
    }

    /// Snapshots in a library's `backups/` folder, newest first. Works whether or not the
    /// library can be opened.
    pub fn snapshots(roots: &StorageRoots) -> Result<Vec<SnapshotInfo>, StoreError> {
        snapshot::list(roots)
    }

    pub fn roots(&self) -> &StorageRoots {
        &self.roots
    }

    pub fn manifest(&self) -> &LibraryManifest {
        &self.manifest
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn open_report(&self) -> &OpenReport {
        &self.report
    }

    /// Reads the library's identity row from the database.
    pub fn describe(&self) -> Result<LibraryDescription, StoreError> {
        self.db.call(|conn| {
            let schema_version = db::user_version(conn).map_err(StoreError::sqlite("describe"))?;
            conn.query_row(
                "SELECT library_id, created_at_unix_ms, created_by_app_version \
                 FROM library_meta WHERE id = 1",
                [],
                |row| {
                    let created_at: i64 = row.get(1)?;
                    Ok(LibraryDescription {
                        library_id: row.get(0)?,
                        created_at_unix_ms: u64::try_from(created_at).unwrap_or(0),
                        created_by_app_version: row.get(2)?,
                        schema_version,
                    })
                },
            )
            .map_err(StoreError::sqlite("describe"))
        })
    }

    pub(crate) fn database(&self) -> &Database {
        &self.db
    }
}

/// Refuses a database that this build cannot read or that belongs to a different library.
fn check_identity(
    db_path: &std::path::Path,
    inspection: &db::Inspection,
    library_id: &LibraryId,
    latest: u32,
) -> Result<(), StoreError> {
    let unrecognised = |detail: &str| StoreError::DatabaseUnrecognised {
        path: db_path.to_owned(),
        detail: detail.to_owned(),
    };

    if inspection.schema_version > latest {
        return Err(StoreError::DatabaseTooNew {
            found: inspection.schema_version,
            supported: latest,
        });
    }
    if inspection.schema_version == 0 {
        // An empty file is a database whose creation was interrupted; it is safe to initialise.
        return if inspection.has_tables {
            Err(unrecognised("it has tables but no schema version"))
        } else {
            Ok(())
        };
    }
    match &inspection.library_id {
        Some(found) if *found == library_id.to_string() => Ok(()),
        Some(found) => Err(StoreError::LibraryIdMismatch {
            manifest: *library_id,
            database: found.clone(),
        }),
        None => Err(unrecognised(
            "it does not record which library it belongs to",
        )),
    }
}

/// Resolves a migration marker left by a run that did not finish. SQLite commits atomically,
/// so an intact database is at either the old or the new schema; anything else is replaced by
/// the pre-migration snapshot the marker names.
fn recover_interrupted(
    roots: &StorageRoots,
    marker: &MigrationMarker,
    library_id: &LibraryId,
    latest: u32,
) -> Result<InterruptedMigrationRecovery, StoreError> {
    let db_path = roots.db_path();
    let settled = if fsutil::exists(&db_path)? {
        match db::inspect(&db_path) {
            Ok(found) if found.schema_version == marker.to_version => {
                Some(InterruptedMigrationRecovery::Completed)
            }
            Ok(found) if found.schema_version == marker.from_version => {
                Some(InterruptedMigrationRecovery::RolledBack)
            }
            Ok(_) | Err(StoreError::DatabaseCorrupt { .. }) => None,
            Err(other) => return Err(other),
        }
    } else {
        None
    };

    if let Some(recovery) = settled {
        MigrationMarker::clear(&roots.migration_marker_path())?;
        return Ok(recovery);
    }

    // On failure the marker and the unusable file both stay where they are.
    let set_aside =
        snapshot::restore(roots, &marker.snapshot, library_id, latest).map_err(|error| {
            StoreError::MigrationRecoveryFailed {
                detail: error.to_string(),
                snapshot: Some(marker.snapshot.clone()),
            }
        })?;
    Ok(InterruptedMigrationRecovery::RestoredFromSnapshot {
        snapshot: marker.snapshot.clone(),
        set_aside,
    })
}
