use skribe_domain::{
    MeetingId, RecordingState, Title, TrackFinalisation, TrackHealth, TrackId, TrackSource,
};

use crate::testutil::Sandbox;
use crate::{
    HealthEvent, HealthEventKind, Library, MeetingChanges, NewTrack, StoreError, TrackFormat,
    TrackResult,
};

const FLOAT_STEREO: TrackFormat = TrackFormat {
    sample_rate: 48_000,
    channels: 2,
    bits_per_sample: 32,
    valid_bits: 32,
    float: true,
    channel_mask: 3,
};

fn meeting(library: &Library) -> MeetingId {
    library
        .create_meeting(MeetingChanges {
            title: Title::parse("Weekly sync").expect("title"),
            planned_start_unix_ms: None,
        })
        .expect("create")
        .meeting
        .id
}

fn new_tracks(meeting: &MeetingId) -> Vec<NewTrack> {
    [TrackSource::Microphone, TrackSource::System]
        .into_iter()
        .map(|source| {
            let id = TrackId::generate();
            NewTrack {
                id,
                source,
                device_id: format!("device-{source}"),
                device_name: format!("Test {source}"),
                format: FLOAT_STEREO,
                audio_path: format!("audio/{meeting}/{id}.wav"),
            }
        })
        .collect()
}

fn finalised(id: TrackId) -> TrackResult {
    TrackResult {
        id,
        finalisation: TrackFinalisation::Finalised,
        health: TrackHealth::Healthy,
        problem: None,
        problem_detail: None,
        stopped_at_us: None,
        start_us: Some(1_200),
        end_us: Some(60_000_000),
        frame_count: Some(2_880_000),
        data_bytes: Some(23_040_000),
        dropped_frames: 0,
        sha256: Some("ab".repeat(32)),
    }
}

fn failed(id: TrackId) -> TrackResult {
    TrackResult {
        id,
        finalisation: TrackFinalisation::Failed,
        health: TrackHealth::Failed,
        problem: Some("finalise_failed".into()),
        problem_detail: Some("could not finalise".into()),
        stopped_at_us: None,
        start_us: None,
        end_us: None,
        frame_count: None,
        data_bytes: None,
        dropped_frames: 0,
        sha256: None,
    }
}

/// Takes a meeting to `recording` and returns its track IDs, microphone first.
fn recording(library: &Library) -> (MeetingId, [TrackId; 2]) {
    let id = meeting(library);
    let tracks = new_tracks(&id);
    let ids = [tracks[0].id, tracks[1].id];
    library.begin_recording(id, tracks).expect("begin");
    library
        .mark_recording_started(id, 1_800_000_000_000)
        .expect("started");
    (id, ids)
}

fn state(library: &Library, id: MeetingId) -> RecordingState {
    library
        .meeting(id)
        .expect("meeting")
        .meeting
        .recording_state
}

#[test]
fn starting_registers_both_tracks_and_a_refused_start_leaves_nothing() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let id = meeting(&library);
    let tracks = new_tracks(&id);

    library.begin_recording(id, tracks.clone()).expect("begin");

    let detail = library.meeting(id).expect("meeting");
    assert_eq!(detail.meeting.recording_state, RecordingState::Ready);
    assert_eq!(detail.tracks.len(), 2);
    assert_eq!(detail.tracks[0].source, TrackSource::Microphone);
    assert_eq!(detail.tracks[1].source, TrackSource::System);
    assert_eq!(detail.tracks[0].format, FLOAT_STEREO);
    assert_eq!(detail.tracks[0].audio_path, tracks[0].audio_path);
    assert_eq!(
        detail.tracks[0].result.finalisation,
        TrackFinalisation::Pending
    );

    library.abandon_recording_start(id).expect("abandon");

    let detail = library.meeting(id).expect("meeting");
    assert_eq!(detail.meeting.recording_state, RecordingState::Setup);
    assert!(detail.tracks.is_empty());
    assert_eq!(detail.meeting.recording_started_at_unix_ms, None);
}

#[test]
fn a_finished_recording_keeps_each_tracks_result_and_survives_restart() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let (id, [microphone, system]) = recording(&library);
    assert_eq!(state(&library, id), RecordingState::Recording);
    library
        .add_health_events(vec![HealthEvent {
            track: system,
            at_us: 5_000_000,
            kind: HealthEventKind::Overflow,
            frames: Some(960),
            until_us: Some(5_020_000),
            detail: None,
        }])
        .expect("events");
    library.mark_finalising(id).expect("finalising");
    assert_eq!(state(&library, id), RecordingState::Finalising);
    let degraded = TrackResult {
        health: TrackHealth::Degraded,
        problem: Some("frames_dropped".into()),
        dropped_frames: 960,
        ..finalised(system)
    };

    let finished = library
        .finish_recording(
            id,
            Some(60_500_000),
            vec![degraded.clone(), finalised(microphone)],
        )
        .expect("finish");

    assert_eq!(finished.meeting.recording_state, RecordingState::Recorded);
    assert_eq!(finished.meeting.recording_duration_us, Some(60_500_000));
    assert_eq!(
        finished.meeting.recording_started_at_unix_ms,
        Some(1_800_000_000_000)
    );
    assert_eq!(finished.tracks[0].result, finalised(microphone));
    assert_eq!(finished.tracks[1].result, degraded);
    drop(library);

    let reopened = sandbox.open_latest();
    assert_eq!(reopened.open_report().interrupted_recordings, 0);
    assert_eq!(reopened.meeting(id).expect("meeting"), finished);
    let events = reopened.health_events(system).expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, HealthEventKind::Overflow);
    assert_eq!(
        (events[0].frames, events[0].until_us),
        (Some(960), Some(5_020_000))
    );
    assert!(
        reopened
            .health_events(microphone)
            .expect("events")
            .is_empty()
    );
}

#[test]
fn a_track_without_a_valid_file_fails_the_recording_and_keeps_the_other_track() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let (id, [microphone, system]) = recording(&library);
    library.mark_finalising(id).expect("finalising");

    let finished = library
        .finish_recording(id, Some(1_000), vec![finalised(microphone), failed(system)])
        .expect("finish");

    assert_eq!(finished.meeting.recording_state, RecordingState::Failed);
    assert_eq!(finished.tracks[0].result, finalised(microphone));
    assert_eq!(finished.tracks[1].result.health, TrackHealth::Failed);
    assert_eq!(
        finished.tracks[1].result.problem.as_deref(),
        Some("finalise_failed")
    );
}

#[test]
fn a_track_that_stopped_early_with_a_valid_partial_file_still_gives_a_recorded_meeting() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let (id, [microphone, system]) = recording(&library);
    library.mark_finalising(id).expect("finalising");
    let lost = TrackResult {
        health: TrackHealth::Failed,
        problem: Some("device_lost".into()),
        stopped_at_us: Some(30_000_000),
        ..finalised(system)
    };

    let finished = library
        .finish_recording(id, Some(60_000_000), vec![finalised(microphone), lost])
        .expect("finish");

    // Recorded, with the warning carried by the track itself.
    assert_eq!(finished.meeting.recording_state, RecordingState::Recorded);
    assert_eq!(finished.tracks[1].result.health, TrackHealth::Failed);
    assert_eq!(finished.tracks[1].result.stopped_at_us, Some(30_000_000));
}

#[test]
fn recording_state_changes_only_along_the_state_machine() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let id = meeting(&library);
    let conflict = |result: Result<(), StoreError>, from, to| {
        assert!(
            matches!(
                result,
                Err(StoreError::RecordingStateConflict { from: found, to: wanted })
                    if found == from && wanted == to
            ),
            "{result:?}"
        );
    };
    use RecordingState::{Finalising, Ready, Recorded, Recording, Setup};

    conflict(library.mark_recording_started(id, 1), Setup, Recording);
    conflict(library.mark_finalising(id), Setup, Finalising);
    conflict(library.abandon_recording_start(id), Setup, Setup);
    conflict(
        library.finish_recording(id, None, Vec::new()).map(drop),
        Setup,
        RecordingState::Failed,
    );
    assert_eq!(state(&library, id), Setup);

    library.begin_recording(id, new_tracks(&id)).expect("begin");
    conflict(library.begin_recording(id, new_tracks(&id)), Ready, Ready);
    conflict(library.mark_finalising(id), Ready, Finalising);
    assert_eq!(library.meeting(id).expect("meeting").tracks.len(), 2);

    library.mark_recording_started(id, 1).expect("started");
    conflict(library.abandon_recording_start(id), Recording, Setup);
    library.mark_finalising(id).expect("finalising");
    let tracks: Vec<TrackId> = library
        .meeting(id)
        .expect("meeting")
        .tracks
        .iter()
        .map(|track| track.result.id)
        .collect();
    library
        .finish_recording(id, None, tracks.iter().copied().map(finalised).collect())
        .expect("finish");

    // There is no way back into recording: no pause, no resume, no second take.
    conflict(
        library.begin_recording(id, new_tracks(&id)),
        Recorded,
        Ready,
    );
    conflict(library.mark_recording_started(id, 2), Recorded, Recording);
    assert_eq!(state(&library, id), Recorded);
}

#[test]
fn results_must_cover_each_track_exactly_once() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let (id, [microphone, _system]) = recording(&library);
    library.mark_finalising(id).expect("finalising");

    for results in [
        vec![finalised(microphone)],
        vec![finalised(microphone), finalised(microphone)],
        vec![finalised(microphone), finalised(TrackId::generate())],
    ] {
        let result = library.finish_recording(id, None, results);
        assert!(
            matches!(result, Err(StoreError::DataInvalid { .. })),
            "{result:?}"
        );
    }
    assert_eq!(state(&library, id), RecordingState::Finalising);
}

#[test]
fn a_meeting_in_the_trash_cannot_be_recorded() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let id = meeting(&library);
    library.trash_meeting(id).expect("trash");

    let result = library.begin_recording(id, new_tracks(&id));

    assert!(
        matches!(result, Err(StoreError::MeetingInTrash)),
        "{result:?}"
    );
    assert!(library.meeting(id).expect("meeting").tracks.is_empty());
}

#[test]
fn recordings_left_running_are_settled_when_the_library_is_next_opened() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let untouched = meeting(&library);
    let starting = meeting(&library);
    library
        .begin_recording(starting, new_tracks(&starting))
        .expect("begin");
    let (capturing, capturing_tracks) = recording(&library);
    let (finalising, _) = recording(&library);
    library.mark_finalising(finalising).expect("finalising");
    let (done, done_tracks) = recording(&library);
    library.mark_finalising(done).expect("finalising");
    library
        .finish_recording(done, Some(1), done_tracks.map(finalised).to_vec())
        .expect("finish");
    // The process ends here without stopping anything.
    drop(library);

    let reopened = sandbox.open_latest();

    assert_eq!(reopened.open_report().interrupted_recordings, 2);
    assert_eq!(state(&reopened, untouched), RecordingState::Setup);
    assert_eq!(state(&reopened, done), RecordingState::Recorded);
    assert_eq!(state(&reopened, capturing), RecordingState::Interrupted);
    assert_eq!(state(&reopened, finalising), RecordingState::Interrupted);
    // Audio had been written for these, so their tracks are kept for recovery.
    assert_eq!(
        reopened.meeting(capturing).expect("meeting").tracks.len(),
        2
    );
    // This one never captured anything: it can simply be recorded again.
    let never_started = reopened.meeting(starting).expect("meeting");
    assert_eq!(never_started.meeting.recording_state, RecordingState::Setup);
    assert!(never_started.tracks.is_empty());

    // Recovery settles an interrupted recording, and it stays settled.
    let recovered = TrackResult {
        finalisation: TrackFinalisation::Recovered,
        health: TrackHealth::Degraded,
        problem: Some("interrupted".into()),
        ..finalised(capturing_tracks[0])
    };
    let after = reopened
        .finish_recording(
            capturing,
            Some(60_000_000),
            vec![recovered.clone(), finalised(capturing_tracks[1])],
        )
        .expect("recover");
    assert_eq!(after.meeting.recording_state, RecordingState::Recorded);
    assert_eq!(after.tracks[0].result, recovered);
    drop(reopened);
    let again = sandbox.open_latest();
    // Only recordings newly found running are counted; the other is still waiting.
    assert_eq!(again.open_report().interrupted_recordings, 0);
    assert_eq!(state(&again, capturing), RecordingState::Recorded);
    assert_eq!(state(&again, finalising), RecordingState::Interrupted);
}

#[test]
fn deleting_a_meeting_removes_its_tracks_and_their_events() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let (id, [microphone, system]) = recording(&library);
    library
        .add_health_events(vec![HealthEvent {
            track: microphone,
            at_us: 1,
            kind: HealthEventKind::DeviceLost,
            frames: None,
            until_us: None,
            detail: Some("removed".into()),
        }])
        .expect("events");
    library.mark_finalising(id).expect("finalising");
    library
        .finish_recording(id, None, vec![finalised(microphone), finalised(system)])
        .expect("finish");
    library.trash_meeting(id).expect("trash");

    library.delete_meeting_permanently(id).expect("delete");

    let remaining: (i64, i64) = library
        .database()
        .call(|conn| {
            conn.query_row(
                "SELECT (SELECT count(*) FROM tracks), (SELECT count(*) FROM track_health_events)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(StoreError::sqlite("test"))
        })
        .expect("count");
    assert_eq!(remaining, (0, 0));
}

#[test]
fn the_schema_accepts_exactly_the_track_states_and_event_kinds_the_code_defines() {
    let sandbox = Sandbox::new();
    let library = sandbox.open_latest();
    let (_, [microphone, _]) = recording(&library);
    let set = move |library: &Library, column: &'static str, value: &'static str| {
        library.database().call(move |conn| {
            conn.execute(
                &format!("UPDATE tracks SET {column} = ?2 WHERE id = ?1"),
                (microphone.to_string(), value),
            )
            .map_err(StoreError::sqlite("test"))
        })
    };

    for state in TrackFinalisation::ALL {
        set(&library, "finalisation", state.as_str())
            .unwrap_or_else(|error| panic!("{state}: {error}"));
    }
    for state in TrackHealth::ALL {
        set(&library, "health", state.as_str()).unwrap_or_else(|error| panic!("{state}: {error}"));
    }
    assert!(set(&library, "finalisation", "paused").is_err());
    assert!(set(&library, "health", "unknown").is_err());
    assert!(set(&library, "source", "speaker").is_err());
    for kind in HealthEventKind::ALL {
        library
            .add_health_events(vec![HealthEvent {
                track: microphone,
                at_us: 0,
                kind: *kind,
                frames: None,
                until_us: None,
                detail: None,
            }])
            .unwrap_or_else(|error| panic!("{}: {error}", kind.as_str()));
    }
    assert_eq!(
        library.health_events(microphone).expect("events").len(),
        HealthEventKind::ALL.len()
    );
}
