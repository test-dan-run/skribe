use std::fs;
use std::path::PathBuf;

use skribe_audio::synthetic::{Script, SyntheticBackend};
use skribe_audio::{CaptureErrorKind, UnavailableBackend, wav};
use skribe_store::StorageRoots;
use tempfile::TempDir;

use super::*;
use crate::library::{LibraryStatus, OpenNoticeKind};
use crate::meetings::{self, AgendaAddArgs, MeetingCreateArgs};
use crate::{ErrorCode, states};

struct Bench {
    dir: TempDir,
    library: Arc<LibraryService>,
    recording: RecordingService,
    script: Script,
}

fn library_service(dir: &TempDir) -> Arc<LibraryService> {
    let roots = StorageRoots::under(dir.path()).expect("valid roots");
    Arc::new(LibraryService::new(roots, "0.0.0-test"))
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().expect("temporary folder");
    let library = library_service(&dir);
    let (backend, script) = SyntheticBackend::scripted();
    let recording = RecordingService::new(Arc::clone(&library), Arc::new(backend));
    Bench {
        dir,
        library,
        recording,
        script,
    }
}

impl Bench {
    fn meeting(&self, title: &str) -> (MeetingId, MeetingIdArgs) {
        let created = meetings::create(
            &self.library,
            &MeetingCreateArgs {
                title: title.to_owned(),
                planned_start_unix_ms: None,
            },
        )
        .expect("create meeting");
        let args = MeetingIdArgs {
            meeting_id: created.meeting.id,
        };
        (meeting_id(&args.meeting_id).expect("id"), args)
    }

    fn detail(&self, args: &MeetingIdArgs) -> MeetingDetail {
        meetings::get(&self.library, args).expect("meeting")
    }

    fn audio_folder(&self, meeting: &MeetingId) -> PathBuf {
        self.library.roots().meeting_audio_dir(meeting)
    }
}

fn live_tracks(status: &RecordingStatus) -> &[LiveTrack] {
    match status {
        RecordingStatus::Recording { tracks, .. } => tracks,
        other => panic!("expected a recording, got {other:?}"),
    }
}

#[test]
fn a_recording_runs_from_start_to_a_finalised_pair_of_tracks() {
    let bench = bench();
    let (meeting, args) = bench.meeting("Weekly sync");
    assert_eq!(bench.recording.status(), RecordingStatus::Idle);
    assert!(!bench.recording.is_live());

    let started = start(&bench.recording, &args).expect("start");

    let tracks = live_tracks(&started);
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].source, TrackSource::Microphone);
    assert_eq!(tracks[1].source, TrackSource::System);
    assert!(tracks.iter().all(|track| track.capturing));
    assert!(matches!(
        &started,
        RecordingStatus::Recording {
            synthetic: true,
            ..
        }
    ));
    assert!(bench.recording.is_live());
    let during = bench.detail(&args);
    assert_eq!(
        during.meeting.recording_state,
        states::RecordingState::Recording
    );
    assert!(during.meeting.recording_started_at_unix_ms.is_some());
    assert!(!during.agenda_editable);
    assert_eq!(during.tracks.len(), 2);
    assert_eq!(
        during.tracks[0].finalisation,
        states::TrackFinalisation::Pending
    );

    // One second on the microphone; system audio plays for 200 ms from 300 ms in.
    bench.script.deliver_run(Source::Microphone, 0, 100);
    bench.script.deliver_run(Source::System, 300_000, 20);
    let live = bench.recording.tick();
    assert_eq!(live_tracks(&live)[0].captured_us, 1_000_000);
    assert_eq!(live_tracks(&live)[1].captured_us, 200_000);
    assert_eq!(status(&bench.recording), Ok(live));

    let finished = stop(&bench.recording, &args).expect("stop");

    assert_eq!(bench.recording.status(), RecordingStatus::Idle);
    assert_eq!(
        finished.meeting.recording_state,
        states::RecordingState::Recorded
    );
    assert!(finished.meeting.recording_duration_us.is_some());
    let [microphone, system] = [&finished.tracks[0], &finished.tracks[1]];
    assert_eq!(microphone.source, TrackSource::Microphone);
    assert_eq!(
        microphone.finalisation,
        states::TrackFinalisation::Finalised
    );
    assert_eq!(microphone.health, states::TrackHealth::Healthy);
    assert_eq!(microphone.problem, None);
    assert_eq!(microphone.audio_us, Some(1_000_000));
    assert_eq!(microphone.start_us, Some(0));
    assert_eq!(microphone.format, "48000 Hz, 1 channel, 32-bit float");
    assert_eq!(system.audio_us, Some(200_000));
    assert_eq!(system.start_us, Some(300_000));
    assert_eq!(system.format, "48000 Hz, 2 channels, 16-bit integer");
    assert!(microphone.device_name.contains("not a microphone"));
    assert_eq!(microphone.sha256.as_ref().map(String::len), Some(64));

    // Each track is its own file under the meeting's audio folder, with its journal and
    // clock map, and nothing was mixed.
    let folder = bench.audio_folder(&meeting);
    for track in [microphone, system] {
        let bytes = fs::read(folder.join(format!("{}.wav", track.id))).expect("track file");
        let info = wav::read_header(&bytes).expect("valid track");
        assert_eq!(Some(info.data_bytes), track.size_bytes);
        assert!(folder.join(format!("{}.journal", track.id)).is_file());
        assert!(folder.join(format!("{}.clock", track.id)).is_file());
    }
    assert_eq!(fs::read_dir(&folder).expect("list").count(), 6);
    assert_eq!(bench.detail(&args), finished);
}

#[test]
fn recording_and_notes_need_no_model_or_network_only_a_library_and_devices() {
    // The service is built from the library and a capture backend alone: there is no model,
    // catalogue or network handle for a recording to depend on.
    let bench = bench();
    let (_, args) = bench.meeting("Offline");

    start(&bench.recording, &args).expect("start");
    bench.script.deliver_run(Source::Microphone, 0, 5);
    let finished = stop(&bench.recording, &args).expect("stop");

    assert_eq!(
        finished.meeting.recording_state,
        states::RecordingState::Recorded
    );
    assert_eq!(
        finished.meeting.processing_state,
        states::ProcessingState::None
    );
}

#[test]
fn preflight_reports_devices_and_disk_room_without_changing_anything() {
    let bench = bench();
    let (meeting, args) = bench.meeting("Weekly sync");

    let report = preflight_command(&bench.recording, &args).expect("preflight");

    assert!(report.synthetic);
    assert_eq!(report.capture_name, "Generated test signal");
    assert_eq!(report.devices.len(), 2);
    assert_eq!(report.devices[0].source, TrackSource::Microphone);
    assert_eq!(
        report.devices[1].format,
        "48000 Hz, 2 channels, 16-bit integer"
    );
    assert_eq!(report.minimum_seconds, 3_600);
    assert!(report.available_seconds >= report.minimum_seconds);
    // Checking is not starting.
    assert!(!bench.audio_folder(&meeting).exists());
    assert_eq!(
        bench.detail(&args).meeting.recording_state,
        states::RecordingState::Setup
    );
    assert!(!bench.script.is_capturing(Source::Microphone));
}

#[test]
fn a_start_is_refused_with_the_reason_and_leaves_the_meeting_as_it_was() {
    let bench = bench();
    let (meeting, args) = bench.meeting("Weekly sync");
    let before = bench.detail(&args);
    let unchanged = |bench: &Bench| {
        assert_eq!(bench.detail(&args), before);
        assert!(!bench.audio_folder(&meeting).exists());
        assert_eq!(bench.recording.status(), RecordingStatus::Idle);
        assert!(!bench.script.is_capturing(Source::Microphone));
    };

    bench
        .script
        .fail_open(Source::System, CaptureErrorKind::NoDevice, "0x80070490");
    let missing = start(&bench.recording, &args).expect_err("no playback device");
    assert_eq!(missing.code, ErrorCode::CaptureDeviceMissing);
    assert!(missing.message.contains("playback device"));
    assert!(missing.retained.contains("Nothing was recorded"));
    unchanged(&bench);
    let checked = preflight_command(&bench.recording, &args).expect_err("same reason");
    assert_eq!(checked.code, ErrorCode::CaptureDeviceMissing);

    bench
        .script
        .fail_open(Source::System, CaptureErrorKind::AccessDenied, "0x80070005");
    let denied = start(&bench.recording, &args).expect_err("denied");
    assert_eq!(denied.code, ErrorCode::CaptureAccessDenied);
    unchanged(&bench);

    // Both devices open, but one then fails to start capturing.
    bench.script.allow_open(Source::System);
    bench
        .script
        .fail_start(Source::System, "the device is in use");
    let busy = start(&bench.recording, &args).expect_err("start failed");
    assert_eq!(busy.code, ErrorCode::CaptureDeviceError);
    unchanged(&bench);
}

fn device_arg(id: &str) -> DeviceChoiceArg {
    DeviceChoiceArg::Device {
        device_id: id.to_owned(),
    }
}

fn set(
    bench: &Bench,
    microphone: DeviceChoiceArg,
    system: DeviceChoiceArg,
    expected_revision: u32,
) -> Result<RecordingDevices, CommandError> {
    set_devices(
        &bench.recording,
        &SetRecordingDevicesArgs {
            microphone,
            system,
            expected_revision,
        },
    )
}

#[test]
fn devices_follow_the_windows_default_until_the_user_chooses() {
    let bench = bench();

    let listed = devices(&bench.recording).expect("devices");

    assert_eq!(listed.revision, 0);
    assert!(listed.synthetic);
    for source in [&listed.microphone, &listed.system] {
        assert_eq!(source.choice, DeviceChoice::Default);
        assert!(source.choice_available);
        assert_eq!(source.unavailable, None);
        assert_eq!(source.available.len(), 2);
        assert!(source.available[0].is_default);
        assert!(source.available[1].is_communications_default);
    }
}

#[test]
fn the_chosen_devices_are_the_ones_checked_and_recorded() {
    let bench = bench();
    let (_, args) = bench.meeting("Weekly sync");
    let listed = devices(&bench.recording).expect("devices");
    let headset = listed.microphone.available[1].clone();

    let saved = set(
        &bench,
        device_arg(&headset.id),
        DeviceChoiceArg::Communications,
        0,
    )
    .expect("save");

    assert_eq!(saved.revision, 1);
    // The name saved is the one the system reports, not anything the frontend sent.
    assert_eq!(
        saved.microphone.choice,
        DeviceChoice::Device {
            id: headset.id.clone(),
            name: headset.name.clone()
        }
    );
    assert_eq!(saved.system.choice, DeviceChoice::Communications);
    assert_eq!(devices(&bench.recording), Ok(saved));

    let report = preflight_command(&bench.recording, &args).expect("preflight");
    assert_eq!(report.devices[0].name, headset.name);
    assert!(matches!(
        report.devices[0].choice,
        DeviceChoice::Device { .. }
    ));
    assert_eq!(report.devices[1].name, listed.system.available[1].name);
    assert_eq!(report.devices[1].choice, DeviceChoice::Communications);

    start(&bench.recording, &args).expect("start");
    let live = bench.recording.tick();
    assert_eq!(live_tracks(&live)[0].device_name, headset.name);
    let finished = stop(&bench.recording, &args).expect("stop");
    assert_eq!(finished.tracks[0].device_name, headset.name);
    assert_eq!(
        finished.tracks[1].device_name,
        listed.system.available[1].name
    );
}

#[test]
fn a_chosen_device_that_is_unplugged_refuses_recording_and_is_never_replaced() {
    let bench = bench();
    let (meeting, args) = bench.meeting("Weekly sync");
    let listed = devices(&bench.recording).expect("devices");
    let speakers = listed.system.available[1].clone();
    set(
        &bench,
        DeviceChoiceArg::Default,
        device_arg(&speakers.id),
        0,
    )
    .expect("save");
    // The chosen playback device goes away. The default one is still connected.
    let remaining: Vec<_> = skribe_audio::synthetic::standard_devices(Source::System)
        .into_iter()
        .take(1)
        .collect();
    bench.script.set_devices(Source::System, remaining);

    let shown = devices(&bench.recording).expect("devices");
    assert!(!shown.system.choice_available);
    // The choice is still shown by name although the device is not in the list.
    assert_eq!(
        shown.system.choice,
        DeviceChoice::Device {
            id: speakers.id.clone(),
            name: speakers.name.clone()
        }
    );
    assert_eq!(shown.system.available.len(), 1);
    assert!(shown.microphone.choice_available);

    for error in [
        start(&bench.recording, &args)
            .map(drop)
            .expect_err("refused"),
        preflight_command(&bench.recording, &args)
            .map(drop)
            .expect_err("refused"),
    ] {
        assert_eq!(error.code, ErrorCode::CaptureDeviceMissing);
        assert!(error.message.contains("chosen in Settings"));
        assert!(error.retained.contains("did not switch to another device"));
    }
    assert_eq!(
        bench.detail(&args).meeting.recording_state,
        states::RecordingState::Setup
    );
    assert!(!bench.audio_folder(&meeting).exists());
    assert!(!bench.script.is_capturing(Source::Microphone));

    // The unplugged device can stay chosen while the other track's device is changed.
    let kept = set(
        &bench,
        DeviceChoiceArg::Communications,
        device_arg(&speakers.id),
        1,
    )
    .expect("save");
    assert_eq!(kept.system.choice, shown.system.choice);
    assert_eq!(kept.microphone.choice, DeviceChoice::Communications);
}

#[test]
fn a_device_choice_is_validated_before_it_is_saved() {
    let bench = bench();
    let rejected = |microphone: DeviceChoiceArg, message: &str| {
        let error = set(&bench, microphone, DeviceChoiceArg::Default, 0).expect_err("rejected");
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert_eq!(error.message, message);
    };

    rejected(device_arg(""), "That is not a device.");
    rejected(device_arg("line\nbreak"), "That is not a device.");
    rejected(device_arg(&"x".repeat(600)), "That is not a device.");
    rejected(
        device_arg("a-device-that-is-not-connected"),
        "That device is no longer connected. Choose a device from the list.",
    );
    // A playback device is not a microphone.
    let speakers = devices(&bench.recording).expect("devices").system.available[0]
        .id
        .clone();
    rejected(
        device_arg(&speakers),
        "That device is no longer connected. Choose a device from the list.",
    );
    assert_eq!(devices(&bench.recording).expect("devices").revision, 0);

    set(
        &bench,
        DeviceChoiceArg::Communications,
        DeviceChoiceArg::Default,
        0,
    )
    .expect("save");
    let stale = set(
        &bench,
        DeviceChoiceArg::Default,
        DeviceChoiceArg::Default,
        0,
    )
    .expect_err("conflict");
    assert_eq!(stale.code, ErrorCode::RevisionConflict);
    assert_eq!(
        devices(&bench.recording)
            .expect("devices")
            .microphone
            .choice,
        DeviceChoice::Communications
    );
}

#[test]
fn a_build_without_capture_lists_no_devices_and_says_why() {
    let dir = tempfile::tempdir().expect("temporary folder");
    let library = library_service(&dir);
    let recording = RecordingService::new(Arc::clone(&library), Arc::new(UnavailableBackend));

    let listed = devices(&recording).expect("devices");

    assert!(!listed.synthetic);
    for source in [&listed.microphone, &listed.system] {
        assert!(source.available.is_empty());
        assert!(!source.choice_available);
        let why = source.unavailable.as_ref().expect("a reason");
        assert_eq!(why.code, ErrorCode::CaptureUnavailable);
    }
    // Following a default needs no list, so it can still be saved.
    let saved = set_devices(
        &recording,
        &SetRecordingDevicesArgs {
            microphone: DeviceChoiceArg::Communications,
            system: DeviceChoiceArg::Communications,
            expected_revision: 0,
        },
    )
    .expect("save");
    assert_eq!(saved.system.choice, DeviceChoice::Communications);
}

#[test]
fn a_build_without_capture_says_so_and_still_prepares_meetings() {
    let dir = tempfile::tempdir().expect("temporary folder");
    let library = library_service(&dir);
    let recording = RecordingService::new(Arc::clone(&library), Arc::new(UnavailableBackend));
    let created = meetings::create(
        &library,
        &MeetingCreateArgs {
            title: "Prepared in WSL".to_owned(),
            planned_start_unix_ms: None,
        },
    )
    .expect("create");
    let args = MeetingIdArgs {
        meeting_id: created.meeting.id.clone(),
    };

    for result in [
        start(&recording, &args).map(drop),
        preflight_command(&recording, &args).map(drop),
    ] {
        let error = result.expect_err("capture is unavailable");
        assert_eq!(error.code, ErrorCode::CaptureUnavailable);
    }

    let prepared = meetings::agenda_add(
        &library,
        &AgendaAddArgs {
            meeting_id: created.meeting.id,
            expected_revision: created.meeting.revision,
            title: "Intro".to_owned(),
        },
    )
    .expect("the agenda is still editable");
    assert_eq!(
        prepared.meeting.recording_state,
        states::RecordingState::Setup
    );
}

#[test]
fn only_one_recording_runs_and_a_live_meeting_is_protected() {
    let bench = bench();
    let (_, first) = bench.meeting("First");
    let (_, second) = bench.meeting("Second");
    start(&bench.recording, &first).expect("start");

    let again = start(&bench.recording, &second).expect_err("already recording");
    assert_eq!(again.code, ErrorCode::RecordingInProgress);
    let check = preflight_command(&bench.recording, &second).expect_err("already recording");
    assert_eq!(check.code, ErrorCode::RecordingInProgress);
    let wrong = stop(&bench.recording, &second).expect_err("not this meeting");
    assert_eq!(wrong.code, ErrorCode::RecordingNotActive);
    let trash = meetings::trash(&bench.library, &first).expect_err("being recorded");
    assert_eq!(trash.code, ErrorCode::MeetingBusy);
    let agenda = meetings::agenda_add(
        &bench.library,
        &AgendaAddArgs {
            meeting_id: first.meeting_id.clone(),
            expected_revision: bench.detail(&first).meeting.revision,
            title: "Too late".to_owned(),
        },
    )
    .expect_err("the agenda is fixed");
    assert_eq!(agenda.code, ErrorCode::PreparationClosed);
    // None of that disturbed the recording.
    assert!(bench.script.is_capturing(Source::Microphone));
    assert!(matches!(
        bench.recording.status(),
        RecordingStatus::Recording { .. }
    ));

    stop(&bench.recording, &first).expect("stop");
    // A recorded meeting is not recorded over.
    let rerun = start(&bench.recording, &first).expect_err("already recorded");
    assert_eq!(rerun.code, ErrorCode::RecordingStateConflict);
    start(&bench.recording, &second).expect("the next meeting can be recorded");
    stop(&bench.recording, &second).expect("stop");
}

#[test]
fn losing_a_device_keeps_the_other_track_and_reports_the_lost_one_truthfully() {
    let bench = bench();
    let (_, args) = bench.meeting("Weekly sync");
    start(&bench.recording, &args).expect("start");
    bench.script.deliver_run(Source::Microphone, 0, 50);
    bench.script.deliver_run(Source::System, 0, 50);

    bench
        .script
        .lose_device(Source::System, "capture failed with 0x88890004");
    let live = bench.recording.tick();

    let tracks = live_tracks(&live);
    assert!(tracks[0].capturing, "the microphone carries on");
    assert_eq!(tracks[0].problem, None);
    assert!(!tracks[1].capturing);
    assert!(
        tracks[1]
            .problem
            .as_deref()
            .is_some_and(|text| text.contains("disconnected or disabled"))
    );
    assert!(tracks[1].stopped_at_us.is_some());
    // The loss is saved while the recording is still running.
    let during = bench.detail(&args);
    let system_id: TrackId = during.tracks[1].id.parse().expect("track id");
    let library = open_library(&bench.library).expect("library");
    let events = library.health_events(system_id).expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, skribe_store::HealthEventKind::DeviceLost);
    assert_eq!(
        events[0].detail.as_deref(),
        Some("capture failed with 0x88890004")
    );

    bench.script.deliver_run(Source::Microphone, 500_000, 50);
    let finished = stop(&bench.recording, &args).expect("stop");

    // The lost track left a valid partial file, so the meeting is recorded, and the track
    // itself says it failed. Nothing was substituted for it.
    assert_eq!(
        finished.meeting.recording_state,
        states::RecordingState::Recorded
    );
    let [microphone, system] = [&finished.tracks[0], &finished.tracks[1]];
    assert_eq!(microphone.health, states::TrackHealth::Healthy);
    assert_eq!(microphone.audio_us, Some(1_000_000));
    assert_eq!(system.health, states::TrackHealth::Failed);
    assert_eq!(system.finalisation, states::TrackFinalisation::Finalised);
    assert_eq!(system.audio_us, Some(500_000));
    assert!(system.stopped_at_us.is_some());
    assert!(
        system
            .problem
            .as_deref()
            .is_some_and(|text| text.contains("up to that point"))
    );
    assert_eq!(
        library.health_events(system_id).expect("events").len(),
        1,
        "the event is saved once"
    );
}

#[test]
fn an_interrupted_recording_is_reported_at_startup_and_recovered_on_request() {
    let dir = tempfile::tempdir().expect("temporary folder");
    let (meeting, args) = {
        let library = library_service(&dir);
        let (backend, script) = SyntheticBackend::scripted();
        let recording = RecordingService::new(Arc::clone(&library), Arc::new(backend));
        let created = meetings::create(
            &library,
            &MeetingCreateArgs {
                title: "Cut short".to_owned(),
                planned_start_unix_ms: None,
            },
        )
        .expect("create");
        let args = MeetingIdArgs {
            meeting_id: created.meeting.id,
        };
        start(&recording, &args).expect("start");
        script.deliver_run(Source::Microphone, 0, 100);
        script.deliver_run(Source::System, 0, 40);
        // The app goes away without stopping: nothing is finalised and the library still
        // says the meeting is recording.
        (meeting_id(&args.meeting_id).expect("id"), args)
    };

    let library = library_service(&dir);
    let (backend, _script) = SyntheticBackend::scripted();
    let recording = RecordingService::new(Arc::clone(&library), Arc::new(backend));
    let Ok(LibraryStatus::Ready { library: info }) = crate::library::status(&library) else {
        panic!("expected the library to open");
    };
    let notice = info
        .open_notices
        .iter()
        .find(|notice| notice.kind == OpenNoticeKind::RecordingsInterrupted)
        .expect("the interruption is reported");
    assert!(notice.message.contains("Its audio was kept"));
    let interrupted = meetings::get(&library, &args).expect("meeting");
    assert_eq!(
        interrupted.meeting.recording_state,
        states::RecordingState::Interrupted
    );
    // Before recovery the files are not valid tracks: their headers claim no audio.
    let folder = library.roots().meeting_audio_dir(&meeting);
    let microphone_file = folder.join(format!("{}.wav", interrupted.tracks[0].id));
    let raw = fs::read(&microphone_file).expect("read");
    assert_eq!(wav::read_header(&raw).expect("header").data_bytes, 0);
    // An interrupted recording cannot be stopped or started; it is recovered.
    let not_live = stop(&recording, &args).expect_err("nothing is live");
    assert_eq!(not_live.code, ErrorCode::RecordingNotActive);
    let not_setup = start(&recording, &args).expect_err("not in setup");
    assert_eq!(not_setup.code, ErrorCode::RecordingStateConflict);

    let recovered = recover(&recording, &args).expect("recover");

    assert_eq!(
        recovered.meeting.recording_state,
        states::RecordingState::Recorded
    );
    assert_eq!(recovered.meeting.recording_duration_us, Some(1_000_000));
    for (track, audio_us) in recovered.tracks.iter().zip([1_000_000, 400_000]) {
        assert_eq!(track.finalisation, states::TrackFinalisation::Recovered);
        assert_eq!(track.health, states::TrackHealth::Degraded);
        assert_eq!(track.audio_us, Some(audio_us));
        assert!(
            track
                .problem
                .as_deref()
                .is_some_and(|text| text.contains("interrupted"))
        );
        assert!(track.sha256.is_some());
    }
    let rebuilt = fs::read(&microphone_file).expect("read");
    assert_eq!(
        wav::read_header(&rebuilt).expect("header").data_bytes,
        100 * 480 * 4
    );
    // Recovery is done once.
    let again = recover(&recording, &args).expect_err("already recovered");
    assert_eq!(again.code, ErrorCode::RecordingStateConflict);
    assert_eq!(meetings::get(&library, &args), Ok(recovered));
}

#[test]
fn a_track_that_cannot_be_rebuilt_fails_the_recording_and_keeps_the_other() {
    let dir = tempfile::tempdir().expect("temporary folder");
    let args = {
        let library = library_service(&dir);
        let (backend, script) = SyntheticBackend::scripted();
        let recording = RecordingService::new(Arc::clone(&library), Arc::new(backend));
        let created = meetings::create(
            &library,
            &MeetingCreateArgs {
                title: "Damaged".to_owned(),
                planned_start_unix_ms: None,
            },
        )
        .expect("create");
        let args = MeetingIdArgs {
            meeting_id: created.meeting.id,
        };
        start(&recording, &args).expect("start");
        script.deliver_run(Source::Microphone, 0, 10);
        script.deliver_run(Source::System, 0, 10);
        args
    };
    let library = library_service(&dir);
    let recording = RecordingService::new(Arc::clone(&library), Arc::new(UnavailableBackend));
    let interrupted = meetings::get(&library, &args).expect("meeting");
    let meeting = meeting_id(&args.meeting_id).expect("id");
    let folder = library.roots().meeting_audio_dir(&meeting);
    let system_journal = folder.join(format!("{}.journal", interrupted.tracks[1].id));
    fs::write(&system_journal, b"damaged").expect("damage the journal");
    let system_audio = folder.join(format!("{}.wav", interrupted.tracks[1].id));
    let untouched = fs::read(&system_audio).expect("read");

    // Recovery needs no capture device, so it works in any build.
    let recovered = recover(&recording, &args).expect("recover");

    assert_eq!(
        recovered.meeting.recording_state,
        states::RecordingState::Failed
    );
    assert_eq!(
        recovered.tracks[0].finalisation,
        states::TrackFinalisation::Recovered
    );
    assert_eq!(recovered.tracks[0].audio_us, Some(100_000));
    assert_eq!(
        recovered.tracks[1].finalisation,
        states::TrackFinalisation::Failed
    );
    assert_eq!(recovered.tracks[1].health, states::TrackHealth::Failed);
    assert!(
        recovered.tracks[1]
            .problem
            .as_deref()
            .is_some_and(|text| text.contains("could not be rebuilt"))
    );
    // The track that could not be rebuilt was left exactly as it was found.
    assert_eq!(fs::read(&system_audio).expect("read"), untouched);
    assert_eq!(fs::read(&system_journal).expect("read"), b"damaged");
}

#[test]
fn recovery_is_only_for_interrupted_recordings() {
    let bench = bench();
    let (_, args) = bench.meeting("Weekly sync");

    let error = recover(&bench.recording, &args).expect_err("not interrupted");

    assert_eq!(error.code, ErrorCode::RecordingStateConflict);
    assert_eq!(
        bench.detail(&args).meeting.recording_state,
        states::RecordingState::Setup
    );
}

#[test]
fn what_the_frontend_sends_is_validated() {
    let bench = bench();
    let bad = MeetingIdArgs {
        meeting_id: "../../etc".to_owned(),
    };
    let unknown = MeetingIdArgs {
        meeting_id: MeetingId::generate().to_string(),
    };

    assert_eq!(
        start(&bench.recording, &bad).expect_err("rejected").code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        recover(&bench.recording, &bad).expect_err("rejected").code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        start(&bench.recording, &unknown).expect_err("unknown").code,
        ErrorCode::MeetingNotFound
    );
    assert_eq!(bench.recording.status(), RecordingStatus::Idle);
    assert_eq!(
        fs::read_dir(bench.dir.path().join("library/audio"))
            .expect("list")
            .count(),
        0
    );
}

#[test]
fn a_permanently_deleted_recording_takes_its_audio_with_it() {
    let bench = bench();
    let (meeting, args) = bench.meeting("Doomed");
    start(&bench.recording, &args).expect("start");
    bench.script.deliver_run(Source::Microphone, 0, 10);
    stop(&bench.recording, &args).expect("stop");
    let folder = bench.audio_folder(&meeting);
    assert!(folder.is_dir());

    meetings::trash(&bench.library, &args).expect("trash");
    // In the trash, the audio is still there.
    assert_eq!(fs::read_dir(&folder).expect("list").count(), 6);
    meetings::delete_permanently(&bench.library, &args).expect("delete");

    assert!(!folder.exists());
}

#[test]
fn recording_status_crosses_the_boundary_tagged_by_phase() {
    let idle = serde_json::to_value(RecordingStatus::Idle).expect("serialise");
    assert_eq!(idle, serde_json::json!({ "phase": "idle" }));
    let finalising = serde_json::to_value(RecordingStatus::Finalising {
        meeting_id: "m".into(),
    })
    .expect("serialise");
    assert_eq!(
        finalising,
        serde_json::json!({ "phase": "finalising", "meetingId": "m" })
    );
}
