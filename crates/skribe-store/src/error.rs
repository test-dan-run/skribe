use std::io;
use std::path::{Path, PathBuf};

use skribe_domain::LibraryId;

/// What state a failed migration left the database in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailedMigrationOutcome {
    /// The transaction rolled back and the database was verified unchanged.
    RolledBack,
    /// The database was not in its pre-migration state, so the pre-migration snapshot was
    /// restored. The previous file was moved to `set_aside`, not deleted.
    RestoredFromSnapshot { set_aside: Option<PathBuf> },
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("storage locations are not usable: {reason}")]
    InvalidRoots { reason: String },

    #[error("could not {action} {}: {source}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("the library at {} is in use by another process", path.display())]
    LibraryLocked { path: PathBuf },

    #[error("{} is missing but the library folder contains data", path.display())]
    ManifestMissing { path: PathBuf },

    #[error("{} is not a valid library manifest: {reason}", path.display())]
    ManifestInvalid { path: PathBuf, reason: String },

    #[error("library format {found} is newer than the supported format {supported}")]
    LibraryFormatTooNew { found: u32, supported: u32 },

    #[error("{} is missing but the library folder contains data", path.display())]
    DatabaseMissing { path: PathBuf },

    #[error("{} failed its integrity check: {detail}", path.display())]
    DatabaseCorrupt { path: PathBuf, detail: String },

    #[error("{} is not a Skribe database: {detail}", path.display())]
    DatabaseUnrecognised { path: PathBuf, detail: String },

    #[error("database schema {found} is newer than the supported schema {supported}")]
    DatabaseTooNew { found: u32, supported: u32 },

    #[error("library.json names library {manifest} but the database belongs to {database}")]
    LibraryIdMismatch {
        manifest: LibraryId,
        database: String,
    },

    #[error("{} cannot use write-ahead logging (journal mode is {mode})", path.display())]
    WalUnavailable { path: PathBuf, mode: String },

    #[error("migrating the database from schema {from} to {to} failed: {detail}")]
    MigrationFailed {
        from: u32,
        to: u32,
        detail: String,
        outcome: FailedMigrationOutcome,
        snapshot: Option<String>,
    },

    #[error("an interrupted migration could not be recovered: {detail}")]
    MigrationRecoveryFailed {
        detail: String,
        snapshot: Option<String>,
    },

    #[error("that is not a snapshot name")]
    SnapshotNameInvalid,

    #[error("snapshot {name} was not found")]
    SnapshotNotFound { name: String },

    #[error("snapshot {name} cannot be used: {reason}")]
    SnapshotInvalid { name: String, reason: String },

    #[error("restoring a snapshot is not available in the library's current state")]
    RestoreNotAvailable,

    #[error("database operation failed ({action}): {detail}")]
    Sqlite {
        action: &'static str,
        detail: String,
    },

    #[error("the database worker has stopped")]
    WorkerStopped,
}

impl StoreError {
    /// Whether replacing the database with a verified snapshot is a sensible recovery from this
    /// error. Errors that leave a healthy database in place do not offer it.
    pub fn offers_snapshot_restore(&self) -> bool {
        matches!(
            self,
            Self::DatabaseMissing { .. }
                | Self::DatabaseCorrupt { .. }
                | Self::DatabaseUnrecognised { .. }
                | Self::MigrationRecoveryFailed { .. }
        )
    }

    pub(crate) fn io(action: &'static str, path: &Path) -> impl FnOnce(io::Error) -> Self {
        let path = path.to_owned();
        move |source| Self::Io {
            action,
            path,
            source,
        }
    }

    pub(crate) fn sqlite(action: &'static str) -> impl FnOnce(rusqlite::Error) -> Self {
        move |source| Self::Sqlite {
            action,
            detail: source.to_string(),
        }
    }
}
