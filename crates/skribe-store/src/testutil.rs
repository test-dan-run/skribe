use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use tempfile::TempDir;

use crate::migrate::{MIGRATIONS, Migration};
use crate::{Library, SnapshotInfo, StorageRoots, snapshot};

pub(crate) const APP_VERSION: &str = "0.0.0-test";

/// Storage roots in a temporary folder that is removed when the value is dropped.
pub(crate) struct Sandbox {
    _dir: TempDir,
    pub roots: StorageRoots,
}

impl Sandbox {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().expect("temporary folder");
        let roots = StorageRoots::under(dir.path()).expect("valid roots");
        Self { _dir: dir, roots }
    }

    pub(crate) fn open(&self, migrations: &[Migration]) -> Result<Library, crate::StoreError> {
        Library::open_with(&self.roots, APP_VERSION, migrations)
    }

    /// Opens the library with the shipped migrations, creating it if it is new.
    pub(crate) fn open_latest(&self) -> Library {
        self.open(MIGRATIONS).expect("open library")
    }

    /// Creates the library at schema 1 and closes it again.
    pub(crate) fn create_v1(&self) {
        drop(self.open(&[v1()]).expect("create library"));
    }

    pub(crate) fn db_path(&self) -> PathBuf {
        self.roots.db_path()
    }

    pub(crate) fn marker_path(&self) -> PathBuf {
        self.roots.migration_marker_path()
    }

    /// Takes a snapshot of the closed database the way a migration does.
    pub(crate) fn snapshot_closed_database(&self, schema_version: u32) -> SnapshotInfo {
        let conn = Connection::open(self.db_path()).expect("open database");
        let manifest = crate::LibraryManifest::load(&self.roots.manifest_path()).expect("manifest");
        snapshot::create(&conn, &self.roots, schema_version, &manifest.library_id)
            .expect("create snapshot")
    }

    pub(crate) fn set_aside_folders(&self) -> Vec<PathBuf> {
        let mut folders: Vec<PathBuf> = fs::read_dir(self.roots.db_dir())
            .expect("list db folder")
            .map(|entry| entry.expect("entry").path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("set-aside-"))
            })
            .collect();
        folders.sort();
        folders
    }
}

pub(crate) fn v1() -> Migration {
    let shipped = &MIGRATIONS[0];
    Migration {
        version: shipped.version,
        name: shipped.name,
        sql: shipped.sql,
    }
}

pub(crate) fn v2_items() -> Migration {
    Migration {
        version: 2,
        name: "test_items",
        sql: "CREATE TABLE test_items (id INTEGER PRIMARY KEY, body TEXT NOT NULL) STRICT;",
    }
}

/// Creates a table and then fails, to prove the earlier statement is rolled back with it.
pub(crate) fn v2_broken() -> Migration {
    Migration {
        version: 2,
        name: "test_broken",
        sql: "CREATE TABLE test_items (id INTEGER PRIMARY KEY, body TEXT NOT NULL) STRICT; \
              INSERT INTO no_such_table VALUES (1);",
    }
}

/// Copies a folder as it is on disk right now, the way a crash would leave it.
pub(crate) fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create folder");
    for entry in fs::read_dir(from).expect("list folder") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

pub(crate) fn read(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

pub(crate) const GARBAGE: &[u8] = b"this is not a SQLite database, but it must be kept as found";
