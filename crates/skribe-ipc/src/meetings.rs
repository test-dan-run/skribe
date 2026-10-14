//! Meeting commands: the meeting list, a meeting's title and planned time, its agenda, and
//! the trash. Every handler validates what the frontend sent before the store sees it.

use serde::{Deserialize, Serialize};
use skribe_domain::{
    AgendaItemId, MAX_AGENDA_ITEMS, MAX_TITLE_CHARS, MeetingId, Title, TitleError,
};
use skribe_store::{Library, LibraryService, MeetingChanges, MeetingRecord};

use crate::recording::{TrackSummary, track_summary};
use crate::states::{MinutesState, ProcessingState, RecordingState};
use crate::{CommandError, open_library};

/// The last millisecond of the year 9999. A planned time beyond it is a mistake, not a plan.
const MAX_PLANNED_UNIX_MS: u64 = 253_402_300_799_999;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummary {
    pub id: String,
    pub title: String,
    pub planned_start_unix_ms: Option<u64>,
    pub recording_state: RecordingState,
    pub processing_state: ProcessingState,
    pub minutes_state: MinutesState,
    /// Quote this when changing the meeting or its agenda.
    pub revision: u32,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    /// Set while the meeting is in the trash.
    pub trashed_at_unix_ms: Option<u64>,
    pub agenda_item_count: u32,
    /// When capture started, by the wall clock.
    pub recording_started_at_unix_ms: Option<u64>,
    /// How long the recording ran on its own clock, in microseconds.
    pub recording_duration_us: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MeetingList {
    /// Meetings not in the trash, newest first.
    pub meetings: Vec<MeetingSummary>,
    /// Meetings in the trash, newest first. They can be restored or permanently deleted.
    pub trashed: Vec<MeetingSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AgendaItem {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetail {
    pub meeting: MeetingSummary,
    /// In agenda order.
    pub agenda: Vec<AgendaItem>,
    /// Whether the agenda can still change: recording has not started and the meeting is not
    /// in the trash.
    pub agenda_editable: bool,
    /// The recording's two original tracks, microphone first. Empty until recording starts.
    pub tracks: Vec<TrackSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeetingIdArgs {
    pub meeting_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeetingCreateArgs {
    pub title: String,
    pub planned_start_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeetingUpdateArgs {
    pub meeting_id: String,
    pub expected_revision: u32,
    pub title: String,
    pub planned_start_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgendaAddArgs {
    pub meeting_id: String,
    pub expected_revision: u32,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgendaEditArgs {
    pub meeting_id: String,
    pub expected_revision: u32,
    pub item_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgendaRemoveArgs {
    pub meeting_id: String,
    pub expected_revision: u32,
    pub item_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgendaReorderArgs {
    pub meeting_id: String,
    pub expected_revision: u32,
    /// Every agenda item of the meeting, once, in the new order.
    pub item_ids: Vec<String>,
}

pub(crate) fn meeting_id(text: &str) -> Result<MeetingId, CommandError> {
    text.parse()
        .map_err(|_| CommandError::invalid_input("That is not a meeting."))
}

fn item_id(text: &str) -> Result<AgendaItemId, CommandError> {
    text.parse()
        .map_err(|_| CommandError::invalid_input("That is not an agenda item."))
}

/// Checks a title, naming what it belongs to in the message: "meeting" or "agenda item".
fn title(text: &str, owner: &str) -> Result<Title, CommandError> {
    Title::parse(text).map_err(|error| {
        CommandError::invalid_input(&match error {
            TitleError::Empty => format!("Give the {owner} a title."),
            TitleError::TooLong { .. } => {
                format!("The {owner}'s title can be at most {MAX_TITLE_CHARS} characters.")
            }
            TitleError::ControlCharacter => {
                format!("The {owner}'s title must be a single line of text.")
            }
        })
    })
}

fn changes(title_text: &str, planned: Option<u64>) -> Result<MeetingChanges, CommandError> {
    if planned.is_some_and(|planned| planned > MAX_PLANNED_UNIX_MS) {
        return Err(CommandError::invalid_input(
            "The planned time is too far in the future.",
        ));
    }
    Ok(MeetingChanges {
        title: title(title_text, "meeting")?,
        planned_start_unix_ms: planned,
    })
}

pub(crate) fn summary(meeting: MeetingRecord) -> MeetingSummary {
    MeetingSummary {
        id: meeting.id.to_string(),
        title: meeting.title,
        planned_start_unix_ms: meeting.planned_start_unix_ms,
        recording_state: meeting.recording_state.into(),
        processing_state: meeting.processing_state.into(),
        minutes_state: meeting.minutes_state.into(),
        revision: meeting.revision,
        created_at_unix_ms: meeting.created_at_unix_ms,
        updated_at_unix_ms: meeting.updated_at_unix_ms,
        trashed_at_unix_ms: meeting.trashed_at_unix_ms,
        agenda_item_count: meeting.agenda_len,
        recording_started_at_unix_ms: meeting.recording_started_at_unix_ms,
        recording_duration_us: meeting.recording_duration_us,
    }
}

pub(crate) fn detail(stored: skribe_store::MeetingDetail) -> MeetingDetail {
    let agenda_editable = stored.meeting.recording_state.allows_preparation()
        && stored.meeting.trashed_at_unix_ms.is_none();
    MeetingDetail {
        meeting: summary(stored.meeting),
        agenda: stored
            .agenda
            .into_iter()
            .map(|item| AgendaItem {
                id: item.id.to_string(),
                title: item.title,
            })
            .collect(),
        agenda_editable,
        tracks: stored.tracks.iter().map(track_summary).collect(),
    }
}

fn stored<T>(result: Result<T, skribe_store::StoreError>) -> Result<T, CommandError> {
    result.map_err(|error| CommandError::from(&error))
}

fn listed(library: &Library) -> Result<MeetingList, CommandError> {
    let (trashed, meetings): (Vec<_>, Vec<_>) = stored(library.meetings())?
        .into_iter()
        .map(summary)
        .partition(|meeting| meeting.trashed_at_unix_ms.is_some());
    Ok(MeetingList { meetings, trashed })
}

/// `meetings_list`
pub fn list(service: &LibraryService) -> Result<MeetingList, CommandError> {
    let library = open_library(service)?;
    listed(&library)
}

/// `meeting_create`: a new meeting in `setup` with an empty agenda.
pub fn create(
    service: &LibraryService,
    args: &MeetingCreateArgs,
) -> Result<MeetingDetail, CommandError> {
    let new = changes(&args.title, args.planned_start_unix_ms)?;
    stored(open_library(service)?.create_meeting(new)).map(detail)
}

/// `meeting_get`
pub fn get(service: &LibraryService, args: &MeetingIdArgs) -> Result<MeetingDetail, CommandError> {
    let id = meeting_id(&args.meeting_id)?;
    stored(open_library(service)?.meeting(id)).map(detail)
}

/// `meeting_update`: the title and the optional planned time.
pub fn update(
    service: &LibraryService,
    args: &MeetingUpdateArgs,
) -> Result<MeetingDetail, CommandError> {
    let id = meeting_id(&args.meeting_id)?;
    let new = changes(&args.title, args.planned_start_unix_ms)?;
    stored(open_library(service)?.update_meeting(id, args.expected_revision, new)).map(detail)
}

/// `agenda_add`: appends an item.
pub fn agenda_add(
    service: &LibraryService,
    args: &AgendaAddArgs,
) -> Result<MeetingDetail, CommandError> {
    let id = meeting_id(&args.meeting_id)?;
    let title = title(&args.title, "agenda item")?;
    stored(open_library(service)?.add_agenda_item(id, args.expected_revision, title)).map(detail)
}

/// `agenda_edit`: changes an item's title.
pub fn agenda_edit(
    service: &LibraryService,
    args: &AgendaEditArgs,
) -> Result<MeetingDetail, CommandError> {
    let id = meeting_id(&args.meeting_id)?;
    let item = item_id(&args.item_id)?;
    let title = title(&args.title, "agenda item")?;
    stored(open_library(service)?.edit_agenda_item(id, args.expected_revision, item, title))
        .map(detail)
}

/// `agenda_remove`
pub fn agenda_remove(
    service: &LibraryService,
    args: &AgendaRemoveArgs,
) -> Result<MeetingDetail, CommandError> {
    let id = meeting_id(&args.meeting_id)?;
    let item = item_id(&args.item_id)?;
    stored(open_library(service)?.remove_agenda_item(id, args.expected_revision, item)).map(detail)
}

/// `agenda_reorder`: the whole agenda's new order, saved in one transaction.
pub fn agenda_reorder(
    service: &LibraryService,
    args: &AgendaReorderArgs,
) -> Result<MeetingDetail, CommandError> {
    let id = meeting_id(&args.meeting_id)?;
    if args.item_ids.len() > MAX_AGENDA_ITEMS {
        return Err(CommandError::invalid_input(
            "That order lists more items than an agenda can hold.",
        ));
    }
    let order = args
        .item_ids
        .iter()
        .map(|text| item_id(text))
        .collect::<Result<Vec<_>, _>>()?;
    stored(open_library(service)?.reorder_agenda(id, args.expected_revision, order)).map(detail)
}

/// `meeting_trash`: moves a meeting to the trash. Nothing is deleted.
pub fn trash(service: &LibraryService, args: &MeetingIdArgs) -> Result<MeetingList, CommandError> {
    let id = meeting_id(&args.meeting_id)?;
    let library = open_library(service)?;
    stored(library.trash_meeting(id))?;
    listed(&library)
}

/// `meeting_restore`: takes a meeting back out of the trash.
pub fn restore(
    service: &LibraryService,
    args: &MeetingIdArgs,
) -> Result<MeetingList, CommandError> {
    let id = meeting_id(&args.meeting_id)?;
    let library = open_library(service)?;
    stored(library.restore_meeting(id))?;
    listed(&library)
}

/// `meeting_delete_permanently`: deletes a meeting that is in the trash, with its files.
pub fn delete_permanently(
    service: &LibraryService,
    args: &MeetingIdArgs,
) -> Result<MeetingList, CommandError> {
    let id = meeting_id(&args.meeting_id)?;
    let library = open_library(service)?;
    stored(library.delete_meeting_permanently(id))?;
    listed(&library)
}

#[cfg(test)]
mod tests {
    use skribe_store::StorageRoots;
    use tempfile::TempDir;

    use super::*;
    use crate::ErrorCode;

    fn service() -> (TempDir, LibraryService) {
        let dir = tempfile::tempdir().expect("temporary folder");
        let roots = StorageRoots::under(dir.path()).expect("valid roots");
        (dir, LibraryService::new(roots, "0.0.0-test"))
    }

    fn new_meeting(service: &LibraryService, title: &str) -> MeetingDetail {
        create(
            service,
            &MeetingCreateArgs {
                title: title.to_owned(),
                planned_start_unix_ms: None,
            },
        )
        .expect("create meeting")
    }

    fn add(service: &LibraryService, meeting: &MeetingDetail, title: &str) -> MeetingDetail {
        agenda_add(
            service,
            &AgendaAddArgs {
                meeting_id: meeting.meeting.id.clone(),
                expected_revision: meeting.meeting.revision,
                title: title.to_owned(),
            },
        )
        .expect("add agenda item")
    }

    fn id_args(meeting: &MeetingDetail) -> MeetingIdArgs {
        MeetingIdArgs {
            meeting_id: meeting.meeting.id.clone(),
        }
    }

    fn titles(meeting: &MeetingDetail) -> Vec<&str> {
        meeting
            .agenda
            .iter()
            .map(|item| item.title.as_str())
            .collect()
    }

    #[test]
    fn a_new_meeting_is_in_setup_with_an_editable_empty_agenda() {
        let (_dir, service) = service();

        let created = create(
            &service,
            &MeetingCreateArgs {
                title: "  Quarterly review ".to_owned(),
                planned_start_unix_ms: Some(1_800_000_000_000),
            },
        )
        .expect("create");

        assert_eq!(created.meeting.title, "Quarterly review");
        assert_eq!(
            created.meeting.planned_start_unix_ms,
            Some(1_800_000_000_000)
        );
        assert_eq!(created.meeting.recording_state, RecordingState::Setup);
        assert!(created.agenda.is_empty());
        assert!(created.agenda_editable);
        assert!(created.tracks.is_empty());
        assert_eq!(get(&service, &id_args(&created)), Ok(created));
    }

    #[test]
    fn the_agenda_is_built_edited_reordered_and_trimmed_through_commands() {
        let (_dir, service) = service();
        let mut meeting = new_meeting(&service, "Weekly sync");
        for item in ["Intro", "Budget", "Hiring"] {
            meeting = add(&service, &meeting, item);
        }
        let ids: Vec<String> = meeting.agenda.iter().map(|item| item.id.clone()).collect();

        let reordered = agenda_reorder(
            &service,
            &AgendaReorderArgs {
                meeting_id: meeting.meeting.id.clone(),
                expected_revision: meeting.meeting.revision,
                item_ids: vec![ids[2].clone(), ids[0].clone(), ids[1].clone()],
            },
        )
        .expect("reorder");
        assert_eq!(titles(&reordered), vec!["Hiring", "Intro", "Budget"]);

        let edited = agenda_edit(
            &service,
            &AgendaEditArgs {
                meeting_id: meeting.meeting.id.clone(),
                expected_revision: reordered.meeting.revision,
                item_id: ids[0].clone(),
                title: "Welcome".to_owned(),
            },
        )
        .expect("edit");
        let removed = agenda_remove(
            &service,
            &AgendaRemoveArgs {
                meeting_id: meeting.meeting.id.clone(),
                expected_revision: edited.meeting.revision,
                item_id: ids[1].clone(),
            },
        )
        .expect("remove");

        assert_eq!(titles(&removed), vec!["Hiring", "Welcome"]);
        assert_eq!(removed.meeting.agenda_item_count, 2);
        assert_eq!(get(&service, &id_args(&meeting)), Ok(removed));
    }

    #[test]
    fn a_stale_change_is_reported_as_a_conflict_and_changes_nothing() {
        let (_dir, service) = service();
        let meeting = new_meeting(&service, "Weekly sync");
        let current = add(&service, &meeting, "Intro");

        let error = update(
            &service,
            &MeetingUpdateArgs {
                meeting_id: meeting.meeting.id.clone(),
                // The revision from before the agenda item was added.
                expected_revision: meeting.meeting.revision,
                title: "Overwritten".to_owned(),
                planned_start_unix_ms: None,
            },
        )
        .expect_err("conflict");

        assert_eq!(error.code, ErrorCode::RevisionConflict);
        assert_eq!(error.actions, vec![crate::RecoveryAction::Reload]);
        assert_eq!(get(&service, &id_args(&meeting)), Ok(current));
    }

    #[test]
    fn what_the_frontend_sends_is_validated_before_anything_is_stored() {
        let (_dir, service) = service();
        let meeting = new_meeting(&service, "Weekly sync");
        let invalid = |result: Result<MeetingDetail, CommandError>, message: &str| {
            let error = result.expect_err("rejected");
            assert_eq!(error.code, ErrorCode::InvalidInput, "{message}");
            assert_eq!(error.message, message);
        };
        let create_with = |title: &str, planned| {
            create(
                &service,
                &MeetingCreateArgs {
                    title: title.to_owned(),
                    planned_start_unix_ms: planned,
                },
            )
        };

        invalid(create_with("   ", None), "Give the meeting a title.");
        invalid(
            create_with(&"x".repeat(MAX_TITLE_CHARS + 1), None),
            "The meeting's title can be at most 200 characters.",
        );
        invalid(
            create_with("Two\nlines", None),
            "The meeting's title must be a single line of text.",
        );
        invalid(
            create_with("Far future", Some(MAX_PLANNED_UNIX_MS + 1)),
            "The planned time is too far in the future.",
        );
        invalid(
            get(
                &service,
                &MeetingIdArgs {
                    meeting_id: "../library.json".to_owned(),
                },
            ),
            "That is not a meeting.",
        );
        invalid(
            agenda_add(
                &service,
                &AgendaAddArgs {
                    meeting_id: meeting.meeting.id.clone(),
                    expected_revision: meeting.meeting.revision,
                    title: String::new(),
                },
            ),
            "Give the agenda item a title.",
        );
        invalid(
            agenda_remove(
                &service,
                &AgendaRemoveArgs {
                    meeting_id: meeting.meeting.id.clone(),
                    expected_revision: meeting.meeting.revision,
                    item_id: "1".to_owned(),
                },
            ),
            "That is not an agenda item.",
        );
        invalid(
            agenda_reorder(
                &service,
                &AgendaReorderArgs {
                    meeting_id: meeting.meeting.id.clone(),
                    expected_revision: meeting.meeting.revision,
                    item_ids: vec![meeting.meeting.id.clone(); MAX_AGENDA_ITEMS + 1],
                },
            ),
            "That order lists more items than an agenda can hold.",
        );

        let all = list(&service).expect("list");
        assert_eq!(all.meetings.len(), 1);
        assert_eq!(get(&service, &id_args(&meeting)), Ok(meeting));
    }

    #[test]
    fn trash_is_recoverable_and_permanent_deletion_is_a_separate_step() {
        let (_dir, service) = service();
        let kept = new_meeting(&service, "Kept");
        let doomed = add(&service, &new_meeting(&service, "Doomed"), "Intro");

        let early = delete_permanently(&service, &id_args(&doomed)).expect_err("not in trash");
        assert_eq!(early.code, ErrorCode::MeetingNotInTrash);

        let after_trash = trash(&service, &id_args(&doomed)).expect("trash");
        assert_eq!(after_trash.meetings.len(), 1);
        assert_eq!(after_trash.meetings[0].id, kept.meeting.id);
        assert_eq!(after_trash.trashed.len(), 1);
        assert_eq!(after_trash.trashed[0].id, doomed.meeting.id);
        // Still readable while in the trash, but not editable.
        let in_trash = get(&service, &id_args(&doomed)).expect("readable");
        assert_eq!(in_trash.agenda, doomed.agenda);
        assert!(!in_trash.agenda_editable);

        let after_restore = restore(&service, &id_args(&doomed)).expect("restore");
        assert!(after_restore.trashed.is_empty());
        assert_eq!(after_restore.meetings.len(), 2);
        assert_eq!(get(&service, &id_args(&doomed)), Ok(doomed.clone()));

        trash(&service, &id_args(&doomed)).expect("trash again");
        let after_delete = delete_permanently(&service, &id_args(&doomed)).expect("delete");
        assert!(after_delete.trashed.is_empty());
        assert_eq!(after_delete.meetings.len(), 1);
        let gone = get(&service, &id_args(&doomed)).expect_err("gone");
        assert_eq!(gone.code, ErrorCode::MeetingNotFound);
    }

    #[test]
    fn meeting_commands_report_why_the_library_is_closed() {
        let (_dir, service) = service();
        let other_window =
            skribe_store::Library::open(service.roots(), "0.0.0-test").expect("open");

        let error = list(&service).expect_err("library is in use");

        assert_eq!(error.code, ErrorCode::LibraryInUse);
        drop(other_window);
    }
}
