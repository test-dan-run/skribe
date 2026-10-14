use std::fs;

use skribe_domain::{
    AgendaItemId, MAX_AGENDA_ITEMS, MeetingId, MinutesState, ProcessingState, RecordingState,
    Shortcut, Title,
};

use crate::migrate::latest_schema_version;
use crate::testutil::{Sandbox, v1};
use crate::{
    DeviceSetting, Library, MeetingChanges, MeetingDetail, RecordingDeviceSettings, StoreError,
};

fn title(text: &str) -> Title {
    Title::parse(text).expect("valid title")
}

fn changes(text: &str, planned: Option<u64>) -> MeetingChanges {
    MeetingChanges {
        title: title(text),
        planned_start_unix_ms: planned,
    }
}

fn create(library: &Library, text: &str) -> MeetingDetail {
    library
        .create_meeting(changes(text, None))
        .expect("create meeting")
}

fn with_agenda(library: &Library, items: &[&str]) -> MeetingDetail {
    let mut detail = create(library, "Weekly sync");
    for item in items {
        detail = library
            .add_agenda_item(detail.meeting.id, detail.meeting.revision, title(item))
            .expect("add agenda item");
    }
    detail
}

fn agenda_titles(detail: &MeetingDetail) -> Vec<&str> {
    detail
        .agenda
        .iter()
        .map(|item| item.title.as_str())
        .collect()
}

fn set_recording_state(library: &Library, id: MeetingId, state: RecordingState) {
    library
        .database()
        .call(move |conn| {
            conn.execute(
                "UPDATE meetings SET recording_state = ?2 WHERE id = ?1",
                (id.to_string(), state.as_str()),
            )
            .map_err(StoreError::sqlite("test"))
        })
        .expect("set state");
}

#[test]
fn a_meeting_with_an_agenda_survives_restart() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let planned = Some(1_800_000_000_000);
    let created = library
        .create_meeting(changes("Quarterly review", planned))
        .expect("create");
    assert_eq!(created.meeting.recording_state, RecordingState::Setup);
    assert_eq!(created.meeting.processing_state, ProcessingState::None);
    assert_eq!(created.meeting.minutes_state, MinutesState::None);
    assert_eq!(created.meeting.revision, 1);
    let id = created.meeting.id;
    let mut detail = created;
    for item in ["Results", "Budget", "Hiring"] {
        detail = library
            .add_agenda_item(id, detail.meeting.revision, title(item))
            .expect("add");
    }
    drop(library);

    let reopened = sandbox.open_latest();
    let found = reopened.meeting(id).expect("meeting");

    assert_eq!(found, detail);
    assert_eq!(found.meeting.title, "Quarterly review");
    assert_eq!(found.meeting.planned_start_unix_ms, planned);
    assert_eq!(agenda_titles(&found), vec!["Results", "Budget", "Hiring"]);
    assert_eq!(found.meeting.agenda_len, 3);
    assert_eq!(found.meeting.revision, 4);
}

#[test]
fn meetings_are_listed_newest_first_with_trashed_ones_marked() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let first = create(&library, "First").meeting.id;
    let second = create(&library, "Second").meeting.id;
    library.trash_meeting(first).expect("trash");

    let listed = library.meetings().expect("list");

    let ids: Vec<MeetingId> = listed.iter().map(|meeting| meeting.id).collect();
    assert_eq!(ids, vec![second, first]);
    assert!(listed[0].trashed_at_unix_ms.is_none());
    assert!(listed[1].trashed_at_unix_ms.is_some());
}

#[test]
fn a_stale_revision_is_reported_and_nothing_is_overwritten() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let detail = with_agenda(&library, &["Intro", "Budget"]);
    let id = detail.meeting.id;
    let stale = detail.meeting.revision - 1;
    let first = detail.agenda[0].id;
    let conflict = |result: Result<MeetingDetail, StoreError>| match result {
        Err(StoreError::RevisionConflict { expected, found }) => {
            assert_eq!((expected, found), (stale, detail.meeting.revision));
        }
        other => panic!("expected a revision conflict, got {other:?}"),
    };

    conflict(library.update_meeting(id, stale, changes("Renamed", None)));
    conflict(library.add_agenda_item(id, stale, title("Hiring")));
    conflict(library.edit_agenda_item(id, stale, first, title("Welcome")));
    conflict(library.remove_agenda_item(id, stale, first));
    conflict(library.reorder_agenda(
        id,
        stale,
        detail.agenda.iter().rev().map(|item| item.id).collect(),
    ));

    assert_eq!(library.meeting(id).expect("meeting"), detail);
}

#[test]
fn the_title_and_planned_time_can_be_changed_and_cleared() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let created = library
        .create_meeting(changes("Draft", Some(1_800_000_000_000)))
        .expect("create");

    let updated = library
        .update_meeting(
            created.meeting.id,
            created.meeting.revision,
            changes("Final", None),
        )
        .expect("update");

    assert_eq!(updated.meeting.title, "Final");
    assert_eq!(updated.meeting.planned_start_unix_ms, None);
    assert_eq!(updated.meeting.revision, created.meeting.revision + 1);
}

#[test]
fn agenda_items_can_be_edited_removed_and_reordered() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let detail = with_agenda(&library, &["Intro", "Budget", "Hiring", "Close"]);
    let id = detail.meeting.id;
    let [intro, budget, hiring, close] = [0, 1, 2, 3].map(|index| detail.agenda[index].id);

    let edited = library
        .edit_agenda_item(id, detail.meeting.revision, budget, title("Budget 2027"))
        .expect("edit");
    assert_eq!(
        agenda_titles(&edited),
        vec!["Intro", "Budget 2027", "Hiring", "Close"]
    );

    let reordered = library
        .reorder_agenda(
            id,
            edited.meeting.revision,
            vec![close, intro, hiring, budget],
        )
        .expect("reorder");
    assert_eq!(
        agenda_titles(&reordered),
        vec!["Close", "Intro", "Hiring", "Budget 2027"]
    );

    let removed = library
        .remove_agenda_item(id, reordered.meeting.revision, intro)
        .expect("remove");
    assert_eq!(
        agenda_titles(&removed),
        vec!["Close", "Hiring", "Budget 2027"]
    );

    // Positions are contiguous again, so the next item lands at the end.
    let appended = library
        .add_agenda_item(id, removed.meeting.revision, title("Actions"))
        .expect("add");
    assert_eq!(
        agenda_titles(&appended),
        vec!["Close", "Hiring", "Budget 2027", "Actions"]
    );
    assert_eq!(library.meeting(id).expect("meeting"), appended);
}

#[test]
fn a_reorder_must_list_every_item_exactly_once() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let detail = with_agenda(&library, &["Intro", "Budget", "Hiring"]);
    let id = detail.meeting.id;
    let ids: Vec<AgendaItemId> = detail.agenda.iter().map(|item| item.id).collect();

    for order in [
        vec![ids[0], ids[1]],
        vec![ids[0], ids[1], ids[1]],
        vec![ids[0], ids[1], ids[2], AgendaItemId::generate()],
        vec![ids[0], ids[1], AgendaItemId::generate()],
    ] {
        let result = library.reorder_agenda(id, detail.meeting.revision, order);
        assert!(
            matches!(result, Err(StoreError::AgendaOrderInvalid)),
            "{result:?}"
        );
    }
    assert_eq!(library.meeting(id).expect("meeting"), detail);
}

#[test]
fn agenda_items_belong_to_their_own_meeting() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let mine = with_agenda(&library, &["Intro"]);
    let other = with_agenda(&library, &["Elsewhere"]);
    let foreign = other.agenda[0].id;

    let edit = library.edit_agenda_item(
        mine.meeting.id,
        mine.meeting.revision,
        foreign,
        title("Hijacked"),
    );
    assert!(
        matches!(edit, Err(StoreError::AgendaItemNotFound { .. })),
        "{edit:?}"
    );
    let remove = library.remove_agenda_item(mine.meeting.id, mine.meeting.revision, foreign);
    assert!(
        matches!(remove, Err(StoreError::AgendaItemNotFound { .. })),
        "{remove:?}"
    );
    assert_eq!(library.meeting(other.meeting.id).expect("meeting"), other);
    assert_eq!(library.meeting(mine.meeting.id).expect("meeting"), mine);
}

#[test]
fn the_agenda_is_fixed_once_recording_has_started_but_the_title_is_not() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let detail = with_agenda(&library, &["Intro", "Budget"]);
    let id = detail.meeting.id;
    let revision = detail.meeting.revision;
    let first = detail.agenda[0].id;

    for state in RecordingState::ALL {
        if state.allows_preparation() {
            continue;
        }
        set_recording_state(&library, id, *state);
        let closed = |result: Result<MeetingDetail, StoreError>| {
            assert!(
                matches!(result, Err(StoreError::PreparationClosed { state: found }) if found == *state),
                "{state}: {result:?}"
            );
        };
        closed(library.add_agenda_item(id, revision, title("Hiring")));
        closed(library.edit_agenda_item(id, revision, first, title("Welcome")));
        closed(library.remove_agenda_item(id, revision, first));
        closed(library.reorder_agenda(
            id,
            revision,
            detail.agenda.iter().rev().map(|item| item.id).collect(),
        ));
        assert_eq!(
            agenda_titles(&library.meeting(id).expect("meeting")),
            vec!["Intro", "Budget"]
        );
    }

    let renamed = library
        .update_meeting(id, revision, changes("Renamed afterwards", None))
        .expect("the title stays editable");
    assert_eq!(renamed.meeting.title, "Renamed afterwards");
}

#[test]
fn the_agenda_has_a_size_limit() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let mut detail = create(&library, "Long meeting");
    for index in 0..MAX_AGENDA_ITEMS {
        detail = library
            .add_agenda_item(
                detail.meeting.id,
                detail.meeting.revision,
                title(&format!("Item {index}")),
            )
            .expect("add");
    }

    let result = library.add_agenda_item(
        detail.meeting.id,
        detail.meeting.revision,
        title("One too many"),
    );

    assert!(
        matches!(result, Err(StoreError::AgendaFull { limit }) if limit == MAX_AGENDA_ITEMS),
        "{result:?}"
    );
    assert_eq!(library.meeting(detail.meeting.id).expect("meeting"), detail);
}

#[test]
fn trash_keeps_everything_and_restore_brings_it_back() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let detail = with_agenda(&library, &["Intro", "Budget"]);
    let id = detail.meeting.id;

    library.trash_meeting(id).expect("trash");

    let trashed = library.meeting(id).expect("still readable");
    assert!(trashed.meeting.trashed_at_unix_ms.is_some());
    assert_eq!(trashed.agenda, detail.agenda);
    // A meeting in the trash is not edited behind the user's back.
    let edit = library.update_meeting(id, detail.meeting.revision, changes("Edited", None));
    assert!(matches!(edit, Err(StoreError::MeetingInTrash)), "{edit:?}");
    let again = library.trash_meeting(id);
    assert!(
        matches!(again, Err(StoreError::MeetingInTrash)),
        "{again:?}"
    );

    library.restore_meeting(id).expect("restore");

    assert_eq!(library.meeting(id).expect("meeting"), detail);
    let again = library.restore_meeting(id);
    assert!(
        matches!(again, Err(StoreError::MeetingNotInTrash)),
        "{again:?}"
    );
}

#[test]
fn a_meeting_cannot_be_trashed_while_a_capture_session_owns_it() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let id = create(&library, "Live").meeting.id;

    for state in RecordingState::ALL {
        set_recording_state(&library, id, *state);
        let result = library.trash_meeting(id);
        if state.is_live() {
            assert!(
                matches!(result, Err(StoreError::MeetingBusy { state: found }) if found == *state),
                "{state}: {result:?}"
            );
        } else {
            result.expect("trash");
            library.restore_meeting(id).expect("restore");
        }
    }
}

#[test]
fn permanent_deletion_needs_the_trash_first_and_removes_rows_and_files() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let doomed = with_agenda(&library, &["Intro", "Budget"]);
    let kept = with_agenda(&library, &["Other"]);
    let id = doomed.meeting.id;
    let audio = sandbox.roots.meeting_audio_dir(&id);
    let derived = sandbox.roots.meeting_derived_dir(&id);
    let kept_audio = sandbox.roots.meeting_audio_dir(&kept.meeting.id);
    for folder in [&audio, &derived, &kept_audio] {
        fs::create_dir_all(folder).expect("create folder");
        fs::write(folder.join("track.wav"), b"audio").expect("write file");
    }

    let early = library.delete_meeting_permanently(id);
    assert!(
        matches!(early, Err(StoreError::MeetingNotInTrash)),
        "{early:?}"
    );
    assert!(audio.join("track.wav").exists());

    library.trash_meeting(id).expect("trash");
    library.delete_meeting_permanently(id).expect("delete");

    assert!(matches!(
        library.meeting(id),
        Err(StoreError::MeetingNotFound { .. })
    ));
    assert!(!audio.exists());
    assert!(!derived.exists());
    let orphans: i64 = library
        .database()
        .call(move |conn| {
            conn.query_row(
                "SELECT count(*) FROM agenda_items WHERE meeting_id = ?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .map_err(StoreError::sqlite("test"))
        })
        .expect("count");
    assert_eq!(orphans, 0);
    // Another meeting's rows and files are untouched.
    assert_eq!(library.meeting(kept.meeting.id).expect("meeting"), kept);
    assert!(kept_audio.join("track.wav").exists());
}

#[test]
fn unknown_meetings_are_reported_as_not_found() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let stranger = MeetingId::generate();
    let not_found = |result: Result<(), StoreError>| {
        assert!(
            matches!(result, Err(StoreError::MeetingNotFound { .. })),
            "{result:?}"
        );
    };

    not_found(library.meeting(stranger).map(drop));
    not_found(
        library
            .update_meeting(stranger, 1, changes("x", None))
            .map(drop),
    );
    not_found(library.add_agenda_item(stranger, 1, title("x")).map(drop));
    not_found(library.trash_meeting(stranger));
    not_found(library.restore_meeting(stranger));
    not_found(library.delete_meeting_permanently(stranger));
}

#[test]
fn the_schema_accepts_exactly_the_states_the_domain_defines() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let id = create(&library, "States").meeting.id;
    let set = move |library: &Library, column: &'static str, value: &'static str| {
        library.database().call(move |conn| {
            conn.execute(
                &format!("UPDATE meetings SET {column} = ?2 WHERE id = ?1"),
                (id.to_string(), value),
            )
            .map_err(StoreError::sqlite("test"))
        })
    };

    for state in RecordingState::ALL {
        set(&library, "recording_state", state.as_str())
            .unwrap_or_else(|error| panic!("{state}: {error}"));
    }
    for state in ProcessingState::ALL {
        set(&library, "processing_state", state.as_str())
            .unwrap_or_else(|error| panic!("{state}: {error}"));
    }
    for state in MinutesState::ALL {
        set(&library, "minutes_state", state.as_str())
            .unwrap_or_else(|error| panic!("{state}: {error}"));
    }
    for column in ["recording_state", "processing_state", "minutes_state"] {
        assert!(set(&library, column, "paused").is_err(), "{column}");
    }
}

#[test]
fn a_stored_value_the_domain_does_not_know_is_an_error_not_a_guess() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    library
        .database()
        .call(|conn| {
            conn.execute(
                "INSERT INTO meetings (id, title, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('not-an-id', 'Broken', 1, 1)",
                [],
            )
            .map_err(StoreError::sqlite("test"))
        })
        .expect("insert");

    let result = library.meetings();

    assert!(
        matches!(
            result,
            Err(StoreError::DataInvalid {
                place: "meetings",
                ..
            })
        ),
        "{result:?}"
    );
}

#[test]
fn the_section_shortcut_has_a_default_and_keeps_what_is_saved() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let initial = library.section_shortcut().expect("read");
    assert_eq!(initial.value, Shortcut::DEFAULT);
    assert_eq!(initial.revision, 0);
    let chosen: Shortcut = "Ctrl+Alt+N".parse().expect("shortcut");

    let saved = library.set_section_shortcut(0, chosen).expect("save");
    assert_eq!((saved.value, saved.revision), (chosen, 1));

    let stale = library.set_section_shortcut(0, Shortcut::DEFAULT);
    assert!(
        matches!(
            stale,
            Err(StoreError::RevisionConflict {
                expected: 0,
                found: 1
            })
        ),
        "{stale:?}"
    );
    let again = library
        .set_section_shortcut(1, Shortcut::DEFAULT)
        .expect("save again");
    assert_eq!((again.value, again.revision), (Shortcut::DEFAULT, 2));
    drop(library);

    let reopened = sandbox.open_latest();
    assert_eq!(reopened.section_shortcut().expect("read"), again);
}

#[test]
fn recording_devices_follow_the_system_default_until_chosen_and_keep_what_is_saved() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let initial = library.recording_devices().expect("read");
    assert_eq!(initial.value, RecordingDeviceSettings::default());
    assert_eq!(initial.value.microphone, DeviceSetting::Default);
    assert_eq!(initial.revision, 0);
    let chosen = RecordingDeviceSettings {
        microphone: DeviceSetting::Device {
            id: "{0.0.1.00000000}.{mic}".into(),
            name: "Headset Microphone".into(),
        },
        system: DeviceSetting::Communications,
    };

    let saved = library
        .set_recording_devices(0, chosen.clone())
        .expect("save");
    assert_eq!((&saved.value, saved.revision), (&chosen, 1));

    let stale = library.set_recording_devices(0, RecordingDeviceSettings::default());
    assert!(
        matches!(
            stale,
            Err(StoreError::RevisionConflict {
                expected: 0,
                found: 1
            })
        ),
        "{stale:?}"
    );
    // The two settings have their own revisions.
    assert_eq!(library.section_shortcut().expect("read").revision, 0);
    drop(library);

    let reopened = sandbox.open_latest();
    assert_eq!(reopened.recording_devices().expect("read"), saved);
}

#[test]
fn a_stored_device_setting_that_cannot_be_read_is_an_error_not_the_default() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    library
        .database()
        .call(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value, revision, updated_at_unix_ms) \
                 VALUES ('recording_devices', '{\"microphone\":{\"kind\":\"loudest\"}}', 1, 1)",
                [],
            )
            .map_err(StoreError::sqlite("test"))
        })
        .expect("insert");

    let result = library.recording_devices();

    // Recording from a device the user did not choose would be worse than refusing.
    assert!(
        matches!(
            result,
            Err(StoreError::DataInvalid {
                place: "settings",
                ..
            })
        ),
        "{result:?}"
    );
}

#[test]
fn a_library_from_the_first_build_is_migrated_with_a_snapshot_first() {
    let sandbox = Sandbox::new();
    sandbox.create_v1();

    let library = sandbox.open_latest();

    let migration = library
        .open_report()
        .migration
        .clone()
        .expect("a migration ran");
    assert_eq!((migration.from, migration.to), (1, latest_schema_version()));
    let snapshots = Library::snapshots(&sandbox.roots).expect("snapshots");
    assert_eq!(snapshots.len(), 1);
    assert_eq!(
        migration.snapshot.as_deref(),
        Some(snapshots[0].name.as_str())
    );
    assert_eq!(snapshots[0].schema_version, 1);
    assert!(library.meetings().expect("list").is_empty());
    assert_eq!(
        library.describe().expect("describe").schema_version,
        latest_schema_version()
    );
    drop(library);
    // The older build's view of the library is still restorable from the snapshot.
    assert!(sandbox.open(&[v1()]).is_err(), "schema is now newer than 1");
}
