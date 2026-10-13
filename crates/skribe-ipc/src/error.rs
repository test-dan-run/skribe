use serde::Serialize;
use skribe_store::{FailedMigrationOutcome, StoreError};

/// Stable identifier for a failure. The frontend may branch on it; the wording may change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    StorageLocationInvalid,
    StorageIo,
    LibraryInUse,
    LibraryManifestMissing,
    LibraryManifestInvalid,
    LibraryTooNew,
    LibraryIdMismatch,
    DatabaseMissing,
    DatabaseCorrupt,
    DatabaseUnrecognised,
    DatabaseError,
    MigrationFailed,
    MigrationRecoveryFailed,
    SnapshotNotFound,
    SnapshotUnusable,
    RestoreNotAvailable,
    Internal,
}

/// A recovery the app itself can carry out when the user asks for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    /// Try to open the library again.
    RetryOpen,
    /// Replace the database with a snapshot the user picks.
    RestoreSnapshot,
}

/// What every failed command returns: what happened, what was kept and what can be done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: ErrorCode,
    /// What happened, for the user.
    pub message: String,
    /// What data was kept.
    pub retained: String,
    /// What the user can do next.
    pub recovery: String,
    /// Recoveries the app can perform, in the order to offer them.
    pub actions: Vec<RecoveryAction>,
    /// Technical detail such as paths and storage error text. Never meeting content.
    pub detail: Option<String>,
}

const NOTHING_CHANGED: &str = "Nothing was changed.";

impl CommandError {
    fn new(code: ErrorCode, message: &str, retained: &str, recovery: &str) -> Self {
        Self {
            code,
            message: message.to_owned(),
            retained: retained.to_owned(),
            recovery: recovery.to_owned(),
            actions: Vec::new(),
            detail: None,
        }
    }

    fn with_actions(mut self, actions: &[RecoveryAction]) -> Self {
        self.actions = actions.to_vec();
        self
    }

    fn with_detail(mut self, detail: impl ToString) -> Self {
        self.detail = Some(detail.to_string());
        self
    }

    pub fn invalid_input(message: &str) -> Self {
        Self::new(
            ErrorCode::InvalidInput,
            message,
            NOTHING_CHANGED,
            "Check the value and try again.",
        )
    }

    /// The app could not work out where its storage folders are.
    pub fn storage_location_unresolved(detail: impl ToString) -> Self {
        Self::new(
            ErrorCode::StorageLocationInvalid,
            "Skribe could not find the folder where it keeps its data.",
            NOTHING_CHANGED,
            "Restart Skribe. If this keeps happening, check that your Windows user profile is \
             available.",
        )
        .with_detail(detail)
    }

    /// A failure in the app itself rather than in the user's data.
    pub fn internal(detail: impl ToString) -> Self {
        Self::new(
            ErrorCode::Internal,
            "Skribe ran into an internal problem.",
            "Data already saved in your library was not changed.",
            "Restart Skribe.",
        )
        .with_detail(detail)
    }
}

impl From<&StoreError> for CommandError {
    fn from(error: &StoreError) -> Self {
        use RecoveryAction::{RestoreSnapshot, RetryOpen};

        let described = match error {
            StoreError::InvalidRoots { .. } => Self::new(
                ErrorCode::StorageLocationInvalid,
                "Skribe cannot use its storage folders as they are arranged.",
                NOTHING_CHANGED,
                "Skribe keeps its library, models and temporary files in separate folders \
                 outside its install folder. Reinstall Skribe to its default location.",
            ),
            StoreError::Io { .. } => Self::new(
                ErrorCode::StorageIo,
                "Skribe could not read or write its library folder.",
                "No meeting data was deleted.",
                "Check that the disk has free space and that you can open the folder, then try \
                 again.",
            )
            .with_actions(&[RetryOpen]),
            StoreError::LibraryLocked { .. } => Self::new(
                ErrorCode::LibraryInUse,
                "Your library is already open in another Skribe window.",
                NOTHING_CHANGED,
                "Close the other Skribe window, then try again.",
            )
            .with_actions(&[RetryOpen]),
            StoreError::ManifestMissing { .. } => Self::new(
                ErrorCode::LibraryManifestMissing,
                "The library folder holds data, but its library.json file is missing.",
                "The database, audio and snapshots in the folder were left as they are. Skribe \
                 did not create a new identity for them.",
                "Put library.json back from a copy of this folder, then try again.",
            )
            .with_actions(&[RetryOpen]),
            StoreError::ManifestInvalid { .. } => Self::new(
                ErrorCode::LibraryManifestInvalid,
                "The library's library.json file cannot be read.",
                "The file and everything else in the folder were left as they are.",
                "Put library.json back from a copy of this folder, then try again.",
            )
            .with_actions(&[RetryOpen]),
            StoreError::LibraryFormatTooNew { .. } | StoreError::DatabaseTooNew { .. } => {
                Self::new(
                    ErrorCode::LibraryTooNew,
                    "This library was last used by a newer version of Skribe.",
                    NOTHING_CHANGED,
                    "Install the newer version of Skribe to open this library.",
                )
            }
            StoreError::LibraryIdMismatch { .. } => Self::new(
                ErrorCode::LibraryIdMismatch,
                "The database in the library folder belongs to a different library than \
                 library.json describes.",
                NOTHING_CHANGED,
                "Put back the library.json or database file that belongs to this folder, then \
                 try again.",
            )
            .with_actions(&[RetryOpen]),
            StoreError::DatabaseMissing { .. } => Self::new(
                ErrorCode::DatabaseMissing,
                "The library's database file is missing, but the folder still holds other data.",
                "Audio, snapshots and trash in the folder were left as they are. Skribe did not \
                 start a new, empty database over them.",
                "Restore a snapshot, or put the database file back and try again.",
            )
            .with_actions(&[RestoreSnapshot, RetryOpen]),
            StoreError::DatabaseCorrupt { .. } => Self::new(
                ErrorCode::DatabaseCorrupt,
                "The library's database is damaged and cannot be opened safely.",
                "The damaged file was left exactly as it was found. Audio is kept in separate \
                 files that this check did not touch.",
                "Restore a snapshot. The damaged file is moved aside and kept, not deleted. \
                 Anything saved after the snapshot was taken is not in it.",
            )
            .with_actions(&[RestoreSnapshot, RetryOpen]),
            StoreError::DatabaseUnrecognised { .. } => Self::new(
                ErrorCode::DatabaseUnrecognised,
                "The file where the library's database should be is not a Skribe database.",
                "The file was left exactly as it was found.",
                "Restore a snapshot. The unrecognised file is moved aside and kept, not deleted.",
            )
            .with_actions(&[RestoreSnapshot, RetryOpen]),
            StoreError::WalUnavailable { .. } => Self::new(
                ErrorCode::DatabaseError,
                "The library's folder does not support the way Skribe saves data safely.",
                NOTHING_CHANGED,
                "Keep the library on a local disk, not a network or removable drive.",
            )
            .with_actions(&[RetryOpen]),
            StoreError::MigrationFailed {
                outcome, snapshot, ..
            } => Self::new(
                ErrorCode::MigrationFailed,
                "Skribe could not update the library for this version.",
                &migration_retained(outcome, snapshot.as_deref()),
                "Try again. If it keeps failing, reinstall the previous version of Skribe to \
                 keep using this library.",
            )
            .with_actions(&[RetryOpen]),
            StoreError::MigrationRecoveryFailed { .. } => Self::new(
                ErrorCode::MigrationRecoveryFailed,
                "An update to the library was interrupted, and Skribe could not recover it \
                 automatically.",
                "The database file and the snapshots were left as they are.",
                "Restore a snapshot. The current file is moved aside and kept, not deleted.",
            )
            .with_actions(&[RestoreSnapshot, RetryOpen]),
            StoreError::SnapshotNameInvalid => Self::new(
                ErrorCode::InvalidInput,
                "That is not a snapshot name.",
                NOTHING_CHANGED,
                "Choose a snapshot from the list.",
            ),
            StoreError::SnapshotNotFound { .. } => Self::new(
                ErrorCode::SnapshotNotFound,
                "That snapshot is no longer in the library's backups folder.",
                NOTHING_CHANGED,
                "Choose a different snapshot.",
            ),
            StoreError::SnapshotInvalid { .. } => Self::new(
                ErrorCode::SnapshotUnusable,
                "That snapshot cannot be restored.",
                "Nothing was changed. The current database file was left as it is.",
                "Choose a different snapshot.",
            ),
            StoreError::RestoreNotAvailable => Self::new(
                ErrorCode::RestoreNotAvailable,
                "Restoring a snapshot is not available right now.",
                NOTHING_CHANGED,
                "Restore is offered only when the library cannot be opened and a snapshot can \
                 fix it.",
            ),
            StoreError::Sqlite { .. } => Self::new(
                ErrorCode::DatabaseError,
                "The library's database reported an error.",
                "No meeting data was deleted.",
                "Try again. If it keeps happening, restart Skribe.",
            )
            .with_actions(&[RetryOpen]),
            StoreError::WorkerStopped => Self::new(
                ErrorCode::Internal,
                "Skribe's database worker stopped unexpectedly.",
                "Data already saved in your library was not changed.",
                "Restart Skribe.",
            ),
        };
        described.with_detail(error)
    }
}

fn migration_retained(outcome: &FailedMigrationOutcome, snapshot: Option<&str>) -> String {
    let mut retained = match outcome {
        FailedMigrationOutcome::RolledBack => {
            "The update was undone and the library is as it was before.".to_owned()
        }
        FailedMigrationOutcome::RestoredFromSnapshot { set_aside } => {
            let mut text =
                "The library was restored from the snapshot taken before the update.".to_owned();
            if let Some(folder) = set_aside {
                text.push_str(&format!(
                    " The previous file was kept in {}.",
                    folder.display()
                ));
            }
            text
        }
    };
    if let Some(name) = snapshot {
        retained.push_str(&format!(
            " The snapshot is kept in the backups folder as {name}."
        ));
    }
    retained
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn every_store_error() -> Vec<StoreError> {
        let path = PathBuf::from("/library/db/skribe.sqlite");
        let library_id = "0192f0c8-7b1a-7c3e-9d2f-1a2b3c4d5e6f"
            .parse()
            .expect("valid id");
        vec![
            StoreError::InvalidRoots {
                reason: "overlap".into(),
            },
            StoreError::Io {
                action: "read",
                path: path.clone(),
                source: std::io::Error::other("disk"),
            },
            StoreError::LibraryLocked { path: path.clone() },
            StoreError::ManifestMissing { path: path.clone() },
            StoreError::ManifestInvalid {
                path: path.clone(),
                reason: "bad".into(),
            },
            StoreError::LibraryFormatTooNew {
                found: 2,
                supported: 1,
            },
            StoreError::DatabaseMissing { path: path.clone() },
            StoreError::DatabaseCorrupt {
                path: path.clone(),
                detail: "page 3".into(),
            },
            StoreError::DatabaseUnrecognised {
                path: path.clone(),
                detail: "tables".into(),
            },
            StoreError::DatabaseTooNew {
                found: 2,
                supported: 1,
            },
            StoreError::LibraryIdMismatch {
                manifest: library_id,
                database: "other".into(),
            },
            StoreError::WalUnavailable {
                path,
                mode: "delete".into(),
            },
            StoreError::MigrationFailed {
                from: 1,
                to: 2,
                detail: "boom".into(),
                outcome: FailedMigrationOutcome::RolledBack,
                snapshot: Some("skribe-v1-1.sqlite".into()),
            },
            StoreError::MigrationRecoveryFailed {
                detail: "boom".into(),
                snapshot: None,
            },
            StoreError::SnapshotNameInvalid,
            StoreError::SnapshotNotFound {
                name: "skribe-v1-1.sqlite".into(),
            },
            StoreError::SnapshotInvalid {
                name: "skribe-v1-1.sqlite".into(),
                reason: "x".into(),
            },
            StoreError::RestoreNotAvailable,
            StoreError::Sqlite {
                action: "describe",
                detail: "busy".into(),
            },
            StoreError::WorkerStopped,
        ]
    }

    #[test]
    fn every_failure_says_what_happened_what_was_kept_and_what_to_do() {
        for error in every_store_error() {
            let described = CommandError::from(&error);
            assert!(!described.message.is_empty(), "{error:?}");
            assert!(!described.retained.is_empty(), "{error:?}");
            assert!(!described.recovery.is_empty(), "{error:?}");
            assert_eq!(described.detail, Some(error.to_string()));
        }
    }

    #[test]
    fn restore_is_offered_exactly_when_the_store_allows_it() {
        for error in every_store_error() {
            let described = CommandError::from(&error);
            assert_eq!(
                described.actions.contains(&RecoveryAction::RestoreSnapshot),
                error.offers_snapshot_restore(),
                "{error:?}"
            );
        }
    }

    #[test]
    fn a_failed_migration_reports_where_the_data_is() {
        let rolled_back = CommandError::from(&StoreError::MigrationFailed {
            from: 1,
            to: 2,
            detail: "boom".into(),
            outcome: FailedMigrationOutcome::RolledBack,
            snapshot: Some("skribe-v1-1.sqlite".into()),
        });
        assert!(rolled_back.retained.contains("as it was before"));
        assert!(rolled_back.retained.contains("skribe-v1-1.sqlite"));

        let restored = CommandError::from(&StoreError::MigrationFailed {
            from: 1,
            to: 2,
            detail: "boom".into(),
            outcome: FailedMigrationOutcome::RestoredFromSnapshot {
                set_aside: Some(PathBuf::from("/library/db/set-aside-1")),
            },
            snapshot: Some("skribe-v1-1.sqlite".into()),
        });
        assert!(restored.retained.contains("set-aside-1"));
    }

    #[test]
    fn serialises_with_stable_codes() {
        let json = serde_json::to_value(CommandError::from(&StoreError::DatabaseCorrupt {
            path: PathBuf::from("/x"),
            detail: "d".into(),
        }))
        .expect("serialise");
        assert_eq!(json["code"], "database_corrupt");
        assert_eq!(
            json["actions"],
            serde_json::json!(["restore_snapshot", "retry_open"])
        );
    }
}
