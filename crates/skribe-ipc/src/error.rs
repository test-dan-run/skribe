use serde::Serialize;
use skribe_audio::{AudioError, CaptureError, CaptureErrorKind, Source};
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
    MeetingNotFound,
    AgendaItemNotFound,
    RevisionConflict,
    PreparationClosed,
    MeetingInTrash,
    MeetingNotInTrash,
    MeetingBusy,
    AgendaFull,
    MeetingFilesNotRemoved,
    CaptureUnavailable,
    CaptureDeviceMissing,
    CaptureAccessDenied,
    CaptureFormatUnsupported,
    CaptureDeviceError,
    DiskSpaceLow,
    RecordingStorage,
    RecordingInProgress,
    RecordingNotActive,
    RecordingStateConflict,
    RecordingNotRegistered,
    TrackUnrecoverable,
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
    /// Load the saved version again, replacing what is on screen.
    Reload,
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

pub(crate) const NOTHING_CHANGED: &str = "Nothing was changed.";

impl CommandError {
    pub(crate) fn new(code: ErrorCode, message: &str, retained: &str, recovery: &str) -> Self {
        Self {
            code,
            message: message.to_owned(),
            retained: retained.to_owned(),
            recovery: recovery.to_owned(),
            actions: Vec::new(),
            detail: None,
        }
    }

    pub(crate) fn with_actions(mut self, actions: &[RecoveryAction]) -> Self {
        self.actions = actions.to_vec();
        self
    }

    pub(crate) fn with_detail(mut self, detail: impl ToString) -> Self {
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
        use RecoveryAction::{Reload, RestoreSnapshot, RetryOpen};

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
            StoreError::DataInvalid { .. } => Self::new(
                ErrorCode::DatabaseError,
                "The library holds a value this version of Skribe does not understand.",
                NOTHING_CHANGED,
                "Restart Skribe. If this keeps happening, the library may have been changed by \
                 another program.",
            ),
            StoreError::MeetingNotFound { .. } => Self::new(
                ErrorCode::MeetingNotFound,
                "That meeting is no longer in your library.",
                NOTHING_CHANGED,
                "Go back to the list of meetings.",
            ),
            StoreError::AgendaItemNotFound { .. } => Self::new(
                ErrorCode::AgendaItemNotFound,
                "That agenda item is no longer in the meeting.",
                "Nothing was changed. The rest of the agenda is as it was saved.",
                "Reload the meeting to see its current agenda.",
            )
            .with_actions(&[Reload]),
            StoreError::RevisionConflict { .. } => Self::new(
                ErrorCode::RevisionConflict,
                "This was changed after it was loaded here, so your change was not saved.",
                "The saved version was kept. Your change was not applied to it.",
                "Reload to see the saved version, then make your change again.",
            )
            .with_actions(&[Reload]),
            StoreError::PreparationClosed { .. } => Self::new(
                ErrorCode::PreparationClosed,
                "The agenda can no longer be changed because recording has started for this \
                 meeting.",
                "The agenda was kept as it was when recording started.",
                "The meeting's title and planned time can still be changed.",
            )
            .with_actions(&[Reload]),
            StoreError::MeetingInTrash => Self::new(
                ErrorCode::MeetingInTrash,
                "This meeting is in the trash.",
                NOTHING_CHANGED,
                "Restore the meeting from the trash to change it.",
            ),
            StoreError::MeetingNotInTrash => Self::new(
                ErrorCode::MeetingNotInTrash,
                "This meeting is not in the trash.",
                NOTHING_CHANGED,
                "Only a meeting in the trash can be restored or permanently deleted.",
            ),
            StoreError::MeetingBusy { .. } => Self::new(
                ErrorCode::MeetingBusy,
                "This meeting is being recorded.",
                NOTHING_CHANGED,
                "Stop the recording first.",
            ),
            StoreError::AgendaFull { limit } => Self::new(
                ErrorCode::AgendaFull,
                &format!("The agenda already has the most items a meeting can hold ({limit})."),
                NOTHING_CHANGED,
                "Remove an item or combine items, then try again.",
            ),
            StoreError::AgendaOrderInvalid => Self::new(
                ErrorCode::InvalidInput,
                "That order does not match the meeting's agenda.",
                NOTHING_CHANGED,
                "Reload the meeting and try again.",
            )
            .with_actions(&[Reload]),
            StoreError::RecordingStateConflict { .. } => Self::new(
                ErrorCode::RecordingStateConflict,
                "This meeting's recording has moved on, so that step is not available.",
                NOTHING_CHANGED,
                "Reload the meeting to see where its recording is now.",
            )
            .with_actions(&[Reload]),
            StoreError::MeetingFilesNotRemoved { .. } => Self::new(
                ErrorCode::MeetingFilesNotRemoved,
                "Skribe could not delete all of this meeting's files.",
                "The meeting is still in the trash. Some of its audio files may already have \
                 been deleted.",
                "Close any program that has the meeting's files open, then delete it again.",
            ),
        };
        described.with_detail(error)
    }
}

const NOTHING_RECORDED: &str = "Nothing was recorded and the meeting was not changed.";

/// How a source's device is named in a sentence.
fn device_phrase(source: Source) -> &'static str {
    match source {
        Source::Microphone => "microphone",
        Source::System => "playback device that system audio is recorded from",
    }
}

fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
}

impl CommandError {
    /// A recording is already starting, running or being saved.
    pub fn recording_in_progress() -> Self {
        Self::new(
            ErrorCode::RecordingInProgress,
            "Another recording is already in progress.",
            "The recording in progress was not affected.",
            "Stop that recording before starting or checking another.",
        )
    }

    /// A stop was asked for a meeting that no capture session owns.
    pub fn recording_not_active() -> Self {
        Self::new(
            ErrorCode::RecordingNotActive,
            "This meeting is not being recorded.",
            NOTHING_CHANGED,
            "Reload the meeting to see where its recording is now.",
        )
        .with_actions(&[RecoveryAction::Reload])
    }

    /// Capture stopped and the files were completed, but the library could not be updated.
    pub fn recording_not_registered(detail: impl ToString) -> Self {
        Self::new(
            ErrorCode::RecordingNotRegistered,
            "The recording was stopped, but Skribe could not update the library.",
            "Both tracks were stopped and what they captured is kept in the library folder.",
            "Restart Skribe. The meeting will be marked as interrupted and can then be \
             recovered without losing audio.",
        )
        .with_detail(detail)
    }
}

impl From<&CaptureError> for CommandError {
    fn from(error: &CaptureError) -> Self {
        let device = device_phrase(error.source_name);
        let described = match error.kind {
            CaptureErrorKind::Unavailable => Self::new(
                ErrorCode::CaptureUnavailable,
                "This build of Skribe cannot record audio.",
                NOTHING_RECORDED,
                "Recording needs Skribe for Windows. Meetings and agendas can still be \
                 prepared here.",
            ),
            CaptureErrorKind::NoDevice => Self::new(
                ErrorCode::CaptureDeviceMissing,
                &format!("Windows has no default {device}."),
                NOTHING_RECORDED,
                "Connect or enable a device and make it the default in Windows sound settings, \
                 or choose a specific device in Skribe's settings. Then try again.",
            ),
            CaptureErrorKind::ChosenDeviceMissing => Self::new(
                ErrorCode::CaptureDeviceMissing,
                match error.source_name {
                    Source::Microphone => {
                        "The microphone chosen in Settings is not connected or is disabled."
                    }
                    Source::System => {
                        "The playback device chosen in Settings for system audio is not \
                         connected or is disabled."
                    }
                },
                "Nothing was recorded and the meeting was not changed. Skribe did not switch \
                 to another device.",
                "Connect the device again, or choose another one in Settings, then try again.",
            ),
            CaptureErrorKind::AccessDenied => Self::new(
                ErrorCode::CaptureAccessDenied,
                &format!("Windows did not allow Skribe to use the {device}."),
                NOTHING_RECORDED,
                "In Windows Settings, open Privacy & security, then Microphone, and allow \
                 desktop apps to use the microphone. Then try again.",
            ),
            CaptureErrorKind::FormatUnsupported => Self::new(
                ErrorCode::CaptureFormatUnsupported,
                &format!("The {device} uses an audio format Skribe cannot record."),
                NOTHING_RECORDED,
                "In Windows sound settings, choose a 16, 24 or 32-bit format for the device, \
                 then try again.",
            ),
            CaptureErrorKind::Device => Self::new(
                ErrorCode::CaptureDeviceError,
                &format!("The {device} could not be used."),
                NOTHING_RECORDED,
                "Check that the device works in Windows sound settings and that no other app \
                 has taken exclusive control of it, then try again.",
            ),
        };
        described.with_detail(error)
    }
}

impl From<&AudioError> for CommandError {
    fn from(error: &AudioError) -> Self {
        let described = match error {
            AudioError::Capture(capture) => return Self::from(capture),
            AudioError::Io { .. } => Self::new(
                ErrorCode::RecordingStorage,
                "Skribe could not write the recording's files.",
                "Audio that had already been saved was kept.",
                "Check that the disk has free space and that the library folder can be \
                 written, then try again.",
            ),
            AudioError::DiskSpaceLow {
                available_bytes,
                required_bytes,
                ..
            } => Self::new(
                ErrorCode::DiskSpaceLow,
                &format!(
                    "There is not enough free disk space to start recording: {} free, and an \
                     hour of recording needs {}.",
                    gigabytes(*available_bytes),
                    gigabytes(*required_bytes)
                ),
                NOTHING_RECORDED,
                "Free up space on the disk that holds your library, then try again.",
            ),
            AudioError::JournalInvalid { .. }
            | AudioError::ClockInvalid { .. }
            | AudioError::WavInvalid { .. } => Self::new(
                ErrorCode::TrackUnrecoverable,
                "A track's files are damaged and could not be read.",
                "The files were left exactly as they are.",
                "The other track, if it is intact, is still kept with the meeting.",
            ),
            AudioError::WriterStopped(_) => Self::new(
                ErrorCode::Internal,
                "Skribe ran into an internal problem while writing a track.",
                "Audio that had already been saved was kept.",
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
                path: path.clone(),
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
            StoreError::DataInvalid {
                place: "meetings",
                detail: "state".into(),
            },
            StoreError::MeetingNotFound { id: "m".into() },
            StoreError::AgendaItemNotFound { id: "a".into() },
            StoreError::RevisionConflict {
                expected: 1,
                found: 2,
            },
            StoreError::PreparationClosed {
                state: skribe_domain::RecordingState::Recording,
            },
            StoreError::MeetingInTrash,
            StoreError::MeetingNotInTrash,
            StoreError::MeetingBusy {
                state: skribe_domain::RecordingState::Recording,
            },
            StoreError::AgendaFull { limit: 100 },
            StoreError::AgendaOrderInvalid,
            StoreError::MeetingFilesNotRemoved {
                path: PathBuf::from("/library/audio/m"),
                source: std::io::Error::other("in use"),
            },
            StoreError::RecordingStateConflict {
                from: skribe_domain::RecordingState::Recorded,
                to: skribe_domain::RecordingState::Ready,
            },
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
    fn a_stale_change_says_the_saved_version_was_kept_and_offers_a_reload() {
        let conflict = CommandError::from(&StoreError::RevisionConflict {
            expected: 1,
            found: 2,
        });
        assert_eq!(conflict.code, ErrorCode::RevisionConflict);
        assert!(conflict.retained.contains("saved version was kept"));
        assert_eq!(conflict.actions, vec![RecoveryAction::Reload]);
    }

    fn every_audio_error() -> Vec<AudioError> {
        let path = PathBuf::from("/library/audio/m/t.wav");
        let mut errors: Vec<AudioError> = [
            CaptureErrorKind::Unavailable,
            CaptureErrorKind::NoDevice,
            CaptureErrorKind::ChosenDeviceMissing,
            CaptureErrorKind::AccessDenied,
            CaptureErrorKind::FormatUnsupported,
            CaptureErrorKind::Device,
        ]
        .into_iter()
        .flat_map(|kind| {
            Source::BOTH.map(|source| {
                AudioError::Capture(CaptureError::new(source, kind.clone(), "0x88890004"))
            })
        })
        .collect();
        errors.extend([
            AudioError::Io {
                action: "write",
                path: path.clone(),
                source: std::io::Error::other("disk full"),
            },
            AudioError::JournalInvalid {
                path: path.clone(),
                reason: "short".into(),
            },
            AudioError::ClockInvalid {
                path: path.clone(),
                reason: "short".into(),
            },
            AudioError::WavInvalid {
                path: path.clone(),
                reason: "short".into(),
            },
            AudioError::DiskSpaceLow {
                path,
                available_bytes: 1_500_000_000,
                required_bytes: 2_792_448_000,
            },
            AudioError::WriterStopped(Source::System),
        ]);
        errors
    }

    #[test]
    fn every_recording_failure_says_what_happened_what_was_kept_and_what_to_do() {
        for error in every_audio_error() {
            let described = CommandError::from(&error);
            assert!(!described.message.is_empty(), "{error:?}");
            assert!(!described.retained.is_empty(), "{error:?}");
            assert!(!described.recovery.is_empty(), "{error:?}");
            assert!(described.detail.is_some(), "{error:?}");
        }
        for error in [
            CommandError::recording_in_progress(),
            CommandError::recording_not_active(),
            CommandError::recording_not_registered("busy"),
        ] {
            assert!(!error.retained.is_empty() && !error.recovery.is_empty());
        }
    }

    #[test]
    fn a_refused_start_names_the_device_and_the_reason() {
        let missing = CommandError::from(&CaptureError::new(
            Source::System,
            CaptureErrorKind::NoDevice,
            "0x80070490",
        ));
        assert_eq!(missing.code, ErrorCode::CaptureDeviceMissing);
        assert!(missing.message.contains("playback device"));
        assert!(missing.retained.contains("Nothing was recorded"));

        let denied = CommandError::from(&CaptureError::new(
            Source::Microphone,
            CaptureErrorKind::AccessDenied,
            "0x80070005",
        ));
        assert_eq!(denied.code, ErrorCode::CaptureAccessDenied);
        assert!(denied.message.contains("microphone"));

        let space = CommandError::from(&AudioError::DiskSpaceLow {
            path: PathBuf::from("/library/audio"),
            available_bytes: 1_500_000_000,
            required_bytes: 2_792_448_000,
        });
        assert_eq!(space.code, ErrorCode::DiskSpaceLow);
        assert!(space.message.contains("1.5 GB free"), "{}", space.message);
        assert!(space.message.contains("needs 2.8 GB"), "{}", space.message);

        let unplugged = CommandError::from(&CaptureError::new(
            Source::System,
            CaptureErrorKind::ChosenDeviceMissing,
            "the chosen device is not among the active devices",
        ));
        assert_eq!(unplugged.code, ErrorCode::CaptureDeviceMissing);
        assert!(unplugged.message.contains("chosen in Settings"));
        assert!(
            unplugged
                .retained
                .contains("did not switch to another device")
        );

        let unavailable = CommandError::from(&CaptureError::new(
            Source::Microphone,
            CaptureErrorKind::Unavailable,
            "no backend",
        ));
        assert_eq!(unavailable.code, ErrorCode::CaptureUnavailable);
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
