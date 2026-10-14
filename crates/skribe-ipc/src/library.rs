//! Library commands: report whether the library opened, retry, and restore a snapshot.

use serde::{Deserialize, Serialize};
use skribe_store::{
    InterruptedMigrationRecovery, Library, LibraryAvailability, LibraryService, OpenReport,
    StorageRoots,
};

use crate::CommandError;

/// Longest snapshot name accepted from the frontend; real names are under 50 characters.
const MAX_SNAPSHOT_NAME_LEN: usize = 128;

/// Whether the meeting library is open. Returned once the open attempt has finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LibraryStatus {
    Ready {
        library: LibraryInfo,
    },
    /// The library is closed. `snapshots` lists what can be restored when the error offers it.
    Unavailable {
        error: CommandError,
        snapshots: Vec<SnapshotSummary>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LibraryInfo {
    pub library_id: String,
    pub format_version: u32,
    pub schema_version: u32,
    pub created_at_unix_ms: u64,
    pub created_by_app_version: String,
    pub locations: StorageLocations,
    /// What opening the library had to do this time, such as applying an update.
    pub open_notices: Vec<OpenNotice>,
}

/// Where data is kept, for display only. The frontend has no file access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct StorageLocations {
    pub library: String,
    pub models: String,
    pub temp: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum OpenNoticeKind {
    LibraryCreated,
    LibraryUpdated,
    InterruptedUpdateRecovered,
    RecordingsInterrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct OpenNotice {
    pub kind: OpenNoticeKind,
    pub message: String,
}

/// A database snapshot in the library's backups folder. Snapshots hold structured data only:
/// no audio and no models.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSummary {
    pub name: String,
    pub schema_version: u32,
    pub created_at_unix_ms: u64,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestoreSnapshotArgs {
    pub snapshot_name: String,
}

/// `library_status`: waits for the open attempt and reports its outcome.
pub fn status(service: &LibraryService) -> Result<LibraryStatus, CommandError> {
    describe(service, service.availability())
}

/// `library_retry_open`: tries again after a failure the user may have cleared.
pub fn retry_open(service: &LibraryService) -> Result<LibraryStatus, CommandError> {
    describe(service, service.retry())
}

/// `library_restore_snapshot`: replaces an unopenable database with a snapshot and reopens.
pub fn restore_snapshot(
    service: &LibraryService,
    args: &RestoreSnapshotArgs,
) -> Result<LibraryStatus, CommandError> {
    if args.snapshot_name.is_empty() || args.snapshot_name.len() > MAX_SNAPSHOT_NAME_LEN {
        return Err(CommandError::invalid_input("That is not a snapshot name."));
    }
    let availability = service
        .restore_snapshot(&args.snapshot_name)
        .map_err(|error| CommandError::from(&error))?;
    describe(service, availability)
}

fn describe(
    service: &LibraryService,
    availability: LibraryAvailability,
) -> Result<LibraryStatus, CommandError> {
    match availability {
        LibraryAvailability::Ready(library) => Ok(LibraryStatus::Ready {
            library: library_info(&library)?,
        }),
        LibraryAvailability::Unavailable(error) => {
            // A failure to list snapshots must not replace the error the user needs to see.
            let snapshots = if error.offers_snapshot_restore() {
                service.snapshots().unwrap_or_default()
            } else {
                Vec::new()
            };
            Ok(LibraryStatus::Unavailable {
                error: CommandError::from(&*error),
                snapshots: snapshots
                    .into_iter()
                    .map(|snapshot| SnapshotSummary {
                        name: snapshot.name,
                        schema_version: snapshot.schema_version,
                        created_at_unix_ms: snapshot.created_at_unix_ms,
                        size_bytes: snapshot.size_bytes,
                    })
                    .collect(),
            })
        }
    }
}

fn library_info(library: &Library) -> Result<LibraryInfo, CommandError> {
    let description = library
        .describe()
        .map_err(|error| CommandError::from(&error))?;
    Ok(LibraryInfo {
        library_id: description.library_id,
        format_version: library.manifest().format_version,
        schema_version: description.schema_version,
        created_at_unix_ms: description.created_at_unix_ms,
        created_by_app_version: description.created_by_app_version,
        locations: locations(library.roots()),
        open_notices: open_notices(library.open_report()),
    })
}

fn locations(roots: &StorageRoots) -> StorageLocations {
    StorageLocations {
        library: roots.library().display().to_string(),
        models: roots.models().display().to_string(),
        temp: roots.temp().display().to_string(),
    }
}

fn open_notices(report: &OpenReport) -> Vec<OpenNotice> {
    let mut notices = Vec::new();
    if report.created {
        notices.push(OpenNotice {
            kind: OpenNoticeKind::LibraryCreated,
            message: "A new, empty library was created.".to_owned(),
        });
    }
    if let Some(recovery) = &report.recovery {
        let message = match recovery {
            InterruptedMigrationRecovery::Completed => {
                "An earlier library update was interrupted after it had finished saving. \
                 Nothing needed to be repaired."
                    .to_owned()
            }
            InterruptedMigrationRecovery::RolledBack => {
                "An earlier library update was interrupted before it saved anything. The \
                 library was intact and the update was run again."
                    .to_owned()
            }
            InterruptedMigrationRecovery::RestoredFromSnapshot {
                snapshot,
                set_aside,
            } => {
                let mut text = format!(
                    "An earlier library update was interrupted and left the database unusable. \
                     It was restored from the snapshot {snapshot}, taken just before that update."
                );
                if let Some(folder) = set_aside {
                    text.push_str(&format!(
                        " The unusable file was kept in {}.",
                        folder.display()
                    ));
                }
                text
            }
        };
        notices.push(OpenNotice {
            kind: OpenNoticeKind::InterruptedUpdateRecovered,
            message,
        });
    }
    // A new library's first schema is part of creating it, not an update to report.
    if let Some(migration) = report
        .migration
        .as_ref()
        .filter(|migration| migration.from > 0)
    {
        let mut message = "The library was updated for this version of Skribe.".to_owned();
        if let Some(snapshot) = &migration.snapshot {
            message.push_str(&format!(
                " A snapshot from before the update is kept in the backups folder as {snapshot}."
            ));
        }
        notices.push(OpenNotice {
            kind: OpenNoticeKind::LibraryUpdated,
            message,
        });
    }
    if report.interrupted_recordings > 0 {
        let message = if report.interrupted_recordings == 1 {
            "A recording was still running when Skribe last closed. Its audio was kept. Open \
             the meeting marked \u{201c}Recording interrupted\u{201d} to recover it."
                .to_owned()
        } else {
            format!(
                "{} recordings were still running when Skribe last closed. Their audio was \
                 kept. Open each meeting marked \u{201c}Recording interrupted\u{201d} to \
                 recover it.",
                report.interrupted_recordings
            )
        };
        notices.push(OpenNotice {
            kind: OpenNoticeKind::RecordingsInterrupted,
            message,
        });
    }
    notices
}

#[cfg(test)]
mod tests {
    use std::fs;

    use skribe_store::MigrationReport;
    use tempfile::TempDir;

    use super::*;
    use crate::{ErrorCode, RecoveryAction};

    fn service() -> (TempDir, LibraryService) {
        let dir = tempfile::tempdir().expect("temporary folder");
        let roots = StorageRoots::under(dir.path()).expect("valid roots");
        (dir, LibraryService::new(roots, "0.0.0-test"))
    }

    fn database_path(service: &LibraryService) -> std::path::PathBuf {
        service.roots().library().join("db").join("skribe.sqlite")
    }

    fn restore(service: &LibraryService, name: &str) -> Result<LibraryStatus, CommandError> {
        restore_snapshot(
            service,
            &RestoreSnapshotArgs {
                snapshot_name: name.to_owned(),
            },
        )
    }

    #[test]
    fn a_new_library_is_ready_and_says_it_was_created() {
        let (_dir, service) = service();

        let Ok(LibraryStatus::Ready { library }) = status(&service) else {
            panic!("expected a ready library");
        };

        assert_eq!(
            library.schema_version,
            skribe_store::latest_schema_version()
        );
        assert_eq!(library.created_by_app_version, "0.0.0-test");
        assert_eq!(
            library.locations.library,
            service.roots().library().display().to_string()
        );
        let kinds: Vec<_> = library
            .open_notices
            .iter()
            .map(|notice| notice.kind)
            .collect();
        assert_eq!(kinds, vec![OpenNoticeKind::LibraryCreated]);
    }

    #[test]
    fn a_corrupt_library_is_unavailable_with_restore_and_retry_offered() {
        let (_dir, service) = service();
        drop(Library::open(service.roots(), "0.0.0-test").expect("create"));
        fs::write(database_path(&service), b"not a database").expect("damage database");

        let Ok(LibraryStatus::Unavailable { error, snapshots }) = status(&service) else {
            panic!("expected an unavailable library");
        };

        assert_eq!(error.code, ErrorCode::DatabaseCorrupt);
        assert_eq!(
            error.actions,
            vec![RecoveryAction::RestoreSnapshot, RecoveryAction::RetryOpen]
        );
        assert!(snapshots.is_empty(), "a new library has no snapshots yet");
        assert_eq!(
            fs::read(database_path(&service)).expect("read"),
            b"not a database"
        );
    }

    #[test]
    fn retry_reports_the_library_once_it_can_be_opened() {
        let (_dir, service) = service();
        let other_window = Library::open(service.roots(), "0.0.0-test").expect("open");
        let Ok(LibraryStatus::Unavailable { error, .. }) = status(&service) else {
            panic!("expected an unavailable library");
        };
        assert_eq!(error.code, ErrorCode::LibraryInUse);

        drop(other_window);

        assert!(matches!(
            retry_open(&service),
            Ok(LibraryStatus::Ready { .. })
        ));
    }

    #[test]
    fn restore_validates_the_name_before_touching_the_library() {
        let (_dir, service) = service();
        drop(Library::open(service.roots(), "0.0.0-test").expect("create"));
        fs::write(database_path(&service), b"not a database").expect("damage database");
        assert!(matches!(
            status(&service),
            Ok(LibraryStatus::Unavailable { .. })
        ));

        for name in [
            "",
            "../library.json",
            &"x".repeat(MAX_SNAPSHOT_NAME_LEN + 1),
        ] {
            let error = restore(&service, name).expect_err("rejected");
            assert_eq!(error.code, ErrorCode::InvalidInput, "{name}");
        }
        let error = restore(&service, "skribe-v1-1.sqlite").expect_err("no such snapshot");
        assert_eq!(error.code, ErrorCode::SnapshotNotFound);

        // The original failure is still what the status reports.
        let Ok(LibraryStatus::Unavailable { error, .. }) = status(&service) else {
            panic!("expected an unavailable library");
        };
        assert_eq!(error.code, ErrorCode::DatabaseCorrupt);
        assert_eq!(
            fs::read(database_path(&service)).expect("read"),
            b"not a database"
        );
    }

    #[test]
    fn restore_is_refused_for_a_library_that_is_open() {
        let (_dir, service) = service();
        assert!(matches!(status(&service), Ok(LibraryStatus::Ready { .. })));

        let error = restore(&service, "skribe-v1-1.sqlite").expect_err("refused");
        assert_eq!(error.code, ErrorCode::RestoreNotAvailable);
    }

    #[test]
    fn notices_describe_updates_and_recoveries_but_not_first_time_setup() {
        let updated = open_notices(&OpenReport {
            created: false,
            recovery: Some(InterruptedMigrationRecovery::RestoredFromSnapshot {
                snapshot: "skribe-v1-1.sqlite".into(),
                set_aside: Some("/library/db/set-aside-1".into()),
            }),
            migration: Some(MigrationReport {
                from: 1,
                to: 2,
                snapshot: Some("skribe-v1-2.sqlite".into()),
            }),
            interrupted_recordings: 2,
        });
        let kinds: Vec<_> = updated.iter().map(|notice| notice.kind).collect();
        assert_eq!(
            kinds,
            vec![
                OpenNoticeKind::InterruptedUpdateRecovered,
                OpenNoticeKind::LibraryUpdated,
                OpenNoticeKind::RecordingsInterrupted
            ]
        );
        assert!(
            updated[2]
                .message
                .starts_with("2 recordings were still running")
        );
        assert!(updated[0].message.contains("skribe-v1-1.sqlite"));
        assert!(updated[0].message.contains("set-aside-1"));
        assert!(updated[1].message.contains("skribe-v1-2.sqlite"));

        let created = open_notices(&OpenReport {
            created: true,
            recovery: None,
            migration: Some(MigrationReport {
                from: 0,
                to: 1,
                snapshot: None,
            }),
            interrupted_recordings: 0,
        });
        let kinds: Vec<_> = created.iter().map(|notice| notice.kind).collect();
        assert_eq!(kinds, vec![OpenNoticeKind::LibraryCreated]);
    }
}
