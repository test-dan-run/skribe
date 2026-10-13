//! Database snapshots in `<library>/backups/`. A snapshot is a complete, verified copy of the
//! structured data only: it holds no audio and no models.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use skribe_domain::LibraryId;

use crate::migrate::MigrationMarker;
use crate::{StorageRoots, StoreError, db, fsutil};

/// A snapshot file found in `backups/`, described by its name alone. It is verified when used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotInfo {
    pub name: String,
    pub schema_version: u32,
    pub created_at_unix_ms: u64,
    pub size_bytes: u64,
}

fn file_name(schema_version: u32, created_at_unix_ms: u64) -> String {
    format!("skribe-v{schema_version}-{created_at_unix_ms}.sqlite")
}

/// Parses `skribe-v<schema>-<unix ms>.sqlite`. Anything else, including any path separator,
/// is not a snapshot name.
fn parse_name(name: &str) -> Option<(u32, u64)> {
    let stem = name.strip_prefix("skribe-v")?.strip_suffix(".sqlite")?;
    let (version, created) = stem.split_once('-')?;
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(version) || !digits(created) {
        return None;
    }
    Some((version.parse().ok()?, created.parse().ok()?))
}

/// Snapshots available for restore, newest first.
pub(crate) fn list(roots: &StorageRoots) -> Result<Vec<SnapshotInfo>, StoreError> {
    let dir = roots.backups_dir();
    if !fsutil::exists(&dir)? {
        return Ok(Vec::new());
    }
    let mut snapshots = Vec::new();
    for entry in fs::read_dir(&dir).map_err(StoreError::io("list", &dir))? {
        let entry = entry.map_err(StoreError::io("list", &dir))?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some((schema_version, created_at_unix_ms)) = parse_name(&name) else {
            continue;
        };
        let metadata = entry
            .metadata()
            .map_err(StoreError::io("read", &entry.path()))?;
        if metadata.is_file() {
            snapshots.push(SnapshotInfo {
                name,
                schema_version,
                created_at_unix_ms,
                size_bytes: metadata.len(),
            });
        }
    }
    snapshots.sort_by(|a, b| {
        b.created_at_unix_ms
            .cmp(&a.created_at_unix_ms)
            .then_with(|| b.name.cmp(&a.name))
    });
    Ok(snapshots)
}

/// Copies the open database at `schema_version` into `backups/` and verifies the copy before
/// giving it its final name, so a file with a snapshot name is always a complete snapshot.
pub(crate) fn create(
    conn: &Connection,
    roots: &StorageRoots,
    schema_version: u32,
    library_id: &LibraryId,
) -> Result<SnapshotInfo, StoreError> {
    let dir = roots.backups_dir();
    let mut created_at_unix_ms = fsutil::now_unix_ms();
    let (name, path) = loop {
        let name = file_name(schema_version, created_at_unix_ms);
        let path = dir.join(&name);
        if !fsutil::exists(&path)? {
            break (name, path);
        }
        created_at_unix_ms += 1;
    };

    let staged = fsutil::with_suffix(&path, ".partial");
    if fsutil::exists(&staged)? {
        fs::remove_file(&staged).map_err(StoreError::io("remove", &staged))?;
    }
    conn.execute("VACUUM INTO ?1", [utf8(&staged)?])
        .map_err(StoreError::sqlite("snapshot"))?;
    make_single_file(&staged)?;
    fsutil::sync_file(&staged)?;

    let found = verify(&staged, &name, library_id, schema_version)?;
    if found != schema_version {
        return Err(StoreError::SnapshotInvalid {
            name,
            reason: format!("it holds schema {found}, not schema {schema_version}"),
        });
    }

    fs::rename(&staged, &path).map_err(StoreError::io("rename", &staged))?;
    fsutil::sync_dir(&dir)?;
    let size_bytes = fs::metadata(&path)
        .map_err(StoreError::io("read", &path))?
        .len();
    Ok(SnapshotInfo {
        name,
        schema_version,
        created_at_unix_ms,
        size_bytes,
    })
}

/// The copy inherits write-ahead logging from its source. A snapshot is one file that is only
/// ever read, so it is switched to a rollback journal and never grows `-wal` or `-shm` files.
fn make_single_file(path: &Path) -> Result<(), StoreError> {
    let conn = Connection::open(path).map_err(StoreError::sqlite("snapshot"))?;
    let _mode: String = conn
        .pragma_update_and_check(None, "journal_mode", "DELETE", |row| row.get(0))
        .map_err(StoreError::sqlite("snapshot"))?;
    conn.close()
        .map_err(|(_, error)| StoreError::sqlite("snapshot")(error))
}

/// Checks that a snapshot file is sound, belongs to this library and is no newer than this
/// build understands. Returns its schema version.
fn verify(
    path: &Path,
    name: &str,
    library_id: &LibraryId,
    latest_schema: u32,
) -> Result<u32, StoreError> {
    let invalid = |reason: String| StoreError::SnapshotInvalid {
        name: name.to_owned(),
        reason,
    };

    let inspection = db::inspect(path).map_err(|error| invalid(error.to_string()))?;
    if inspection.schema_version == 0 {
        return Err(invalid("it holds no Skribe data".to_owned()));
    }
    if inspection.schema_version > latest_schema {
        return Err(invalid(format!(
            "it holds schema {}, newer than the supported schema {latest_schema}",
            inspection.schema_version
        )));
    }
    match inspection.library_id {
        Some(found) if found == library_id.to_string() => Ok(inspection.schema_version),
        Some(found) => Err(invalid(format!(
            "it belongs to a different library ({found})"
        ))),
        None => Err(invalid(
            "it does not record which library it belongs to".to_owned(),
        )),
    }
}

/// Replaces the database with the named snapshot. The existing database and its write-ahead
/// log are moved into a `set-aside-*` folder beside it, never deleted or overwritten; that
/// folder is returned when there was something to move.
pub(crate) fn restore(
    roots: &StorageRoots,
    name: &str,
    library_id: &LibraryId,
    latest_schema: u32,
) -> Result<Option<PathBuf>, StoreError> {
    if parse_name(name).is_none() {
        return Err(StoreError::SnapshotNameInvalid);
    }
    let source = roots.backups_dir().join(name);
    let is_file = fs::symlink_metadata(&source).is_ok_and(|metadata| metadata.is_file());
    if !is_file {
        return Err(StoreError::SnapshotNotFound {
            name: name.to_owned(),
        });
    }
    verify(&source, name, library_id, latest_schema)?;

    let db_path = roots.db_path();
    let db_dir = roots.db_dir();

    // Stage the copy before touching the current database.
    let staged = fsutil::with_suffix(&db_path, ".restoring");
    if fsutil::exists(&staged)? {
        fs::remove_file(&staged).map_err(StoreError::io("remove", &staged))?;
    }
    fs::copy(&source, &staged).map_err(StoreError::io("copy", &source))?;
    fsutil::sync_file(&staged)?;

    let set_aside = set_aside_database(&db_dir, &db_path)?;
    fs::rename(&staged, &db_path).map_err(StoreError::io("rename", &staged))?;
    fsutil::sync_dir(&db_dir)?;

    MigrationMarker::clear(&roots.migration_marker_path())?;
    Ok(set_aside)
}

/// Moves the database and its side files out of the way together. A write-ahead log left
/// beside a different database file would be replayed into it.
fn set_aside_database(db_dir: &Path, db_path: &Path) -> Result<Option<PathBuf>, StoreError> {
    let mut present = Vec::new();
    for file in [
        db_path.to_owned(),
        fsutil::with_suffix(db_path, "-wal"),
        fsutil::with_suffix(db_path, "-shm"),
    ] {
        if fsutil::exists(&file)? {
            present.push(file);
        }
    }
    if present.is_empty() {
        return Ok(None);
    }

    let mut stamp = fsutil::now_unix_ms();
    let folder = loop {
        let folder = db_dir.join(format!("set-aside-{stamp}"));
        match fs::create_dir(&folder) {
            Ok(()) => break folder,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => stamp += 1,
            Err(error) => return Err(StoreError::io("create", &folder)(error)),
        }
    };
    for file in present {
        let Some(file_name) = file.file_name() else {
            continue;
        };
        fs::rename(&file, folder.join(file_name)).map_err(StoreError::io("move", &file))?;
    }
    fsutil::sync_dir(&folder)?;
    Ok(Some(folder))
}

fn utf8(path: &Path) -> Result<&str, StoreError> {
    path.to_str().ok_or_else(|| StoreError::InvalidRoots {
        reason: format!("{} is not valid Unicode", path.display()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_names_round_trip() {
        assert_eq!(
            parse_name(&file_name(3, 1_791_234_567_123)),
            Some((3, 1_791_234_567_123))
        );
    }

    #[test]
    fn rejects_names_that_are_not_snapshots() {
        for name in [
            "",
            "skribe.sqlite",
            "skribe-v1-.sqlite",
            "skribe-v-1.sqlite",
            "skribe-v1-2.sqlite.partial",
            "skribe-v+1-2.sqlite",
            "skribe-v1-2/../../x.sqlite",
            "../skribe-v1-2.sqlite",
            "skribe-v1-2\\..\\x.sqlite",
            "skribe-v99999999999-2.sqlite",
        ] {
            assert_eq!(parse_name(name), None, "{name}");
        }
    }
}
