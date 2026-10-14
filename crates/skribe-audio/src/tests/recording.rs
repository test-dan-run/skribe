//! Recording behaviour end to end, against real files in a temporary folder and the scripted
//! backend in place of devices.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::clock::{ClockMap, Gap, flags};
use crate::live::TrackLive;
use crate::ring::{self, PacketMeta, Pushed};
use crate::synthetic::{MICROPHONE_FORMAT, SYSTEM_FORMAT, Script, SyntheticBackend, signal};
use crate::writer::Writer;
use crate::{
    AudioError, AudioFormat, CaptureBackend, CaptureErrorKind, DeviceChoice, DeviceChoices,
    FLUSH_INTERVAL, HealthEventKind, RecordingSession, SampleKind, SessionClock, SessionOutcome,
    Source, StopReason, TrackFiles, TrackOutcome, UnavailableBackend, journal, open_devices,
    recover, wav,
};

/// Ten milliseconds of audio at 48 kHz.
const PACKET_FRAMES: u32 = 480;

struct Recording {
    dir: TempDir,
    script: Script,
    session: RecordingSession,
}

fn files(dir: &Path, source: Source) -> TrackFiles {
    TrackFiles::in_dir(dir, source.as_str())
}

fn start() -> Recording {
    let dir = tempfile::tempdir().expect("temporary folder");
    let (backend, script) = SyntheticBackend::scripted();
    let devices = open_devices(&backend, &DeviceChoices::default()).expect("open devices");
    let session =
        RecordingSession::start(devices, |source| files(dir.path(), source)).expect("start");
    Recording {
        dir,
        script,
        session,
    }
}

fn track(outcome: &SessionOutcome, source: Source) -> &TrackOutcome {
    outcome
        .tracks
        .iter()
        .find(|track| track.source == source)
        .expect("track")
}

fn wait_for(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Checks a finalised track file against what was delivered and returns its clock map.
fn assert_track_file(outcome: &TrackOutcome, format: AudioFormat, frames: u64) -> ClockMap {
    let finalised = outcome.finalised.as_ref().expect("finalised");
    let bytes = fs::read(&outcome.files.wav).expect("read track");
    let info = wav::read_header(&bytes).expect("valid header");
    let data_bytes = frames * u64::from(format.bytes_per_frame());

    assert_eq!(info.format, format);
    assert_eq!(info.data_bytes, data_bytes);
    assert_eq!(bytes.len() as u64, wav::HEADER_LEN + data_bytes);
    assert_eq!(finalised.frames, frames);
    assert_eq!(finalised.data_bytes, data_bytes);
    assert_eq!(finalised.sha256, sha256_hex(&bytes));
    // The audio is exactly the signal that was delivered, in order.
    let expected = signal(outcome.source, 0, u32::try_from(frames).expect("small"));
    assert!(
        bytes[wav::HEADER_LEN as usize..] == expected[..],
        "audio differs"
    );

    let map = ClockMap::read(&outcome.files.clock).expect("clock map");
    assert_eq!(map.frames(), frames);
    let state = journal::read(&outcome.files.journal).expect("journal");
    assert!(state.finalised);
    assert_eq!(state.format, format);
    assert_eq!(state.committed.data_bytes, data_bytes);
    map
}

#[test]
fn both_tracks_are_written_separately_in_their_native_formats_on_one_timeline() {
    let recording = start();
    let devices: Vec<_> = recording
        .session
        .tracks()
        .map(|(source, device, format)| (source, device.name.clone(), *format))
        .collect();
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0].0, Source::Microphone);
    assert_eq!(devices[0].2, MICROPHONE_FORMAT);
    assert_eq!(devices[1].0, Source::System);
    assert_eq!(devices[1].2, SYSTEM_FORMAT);
    assert!(devices[0].1.contains("not a microphone"));

    // The microphone starts 1 ms after the origin; system audio starts a quarter second in.
    let mic_end = recording.script.deliver_run(Source::Microphone, 1_000, 50);
    let system_end = recording.script.deliver_run(Source::System, 250_000, 20);

    let outcome = recording.session.stop();

    assert!(outcome.duration_us >= 0);
    let mic = track(&outcome, Source::Microphone);
    let system = track(&outcome, Source::System);
    assert!(mic.stopped.is_none() && system.stopped.is_none());
    assert_ne!(mic.files.wav, system.files.wav);

    let mic_map = assert_track_file(mic, MICROPHONE_FORMAT, 50 * 480);
    let system_map = assert_track_file(system, SYSTEM_FORMAT, 20 * 480);
    let mic_track = mic.finalised.as_ref().expect("finalised");
    let system_track = system.finalised.as_ref().expect("finalised");
    assert_eq!(mic_track.start_us, Some(1_000));
    assert_eq!(mic_track.end_us, Some(mic_end));
    assert_eq!(system_track.start_us, Some(250_000));
    assert_eq!(system_track.end_us, Some(system_end));
    assert_eq!(mic_track.dropped_frames, 0);
    assert!(!mic_track.rf64);
    // Every packet's device timestamp is in the clock map.
    assert_eq!(mic_map.entries.len(), 50);
    assert_eq!(mic_map.entries[49].timeline_us, 1_000 + 49 * 10_000);
    assert_eq!(system_map.entries.len(), 20);
    assert_eq!(system_map.start_us(), Some(250_000));
}

#[test]
fn loopback_silence_is_a_gap_on_the_timeline_and_the_tracks_stay_aligned() {
    let recording = start();
    recording.script.deliver_run(Source::Microphone, 0, 30);
    // Something plays for 100 ms, nothing for five seconds, then something again.
    let first_end = recording.script.deliver_run(Source::System, 0, 10);
    recording.script.deliver_run(Source::System, 5_100_000, 10);

    let outcome = recording.session.stop();

    let system = track(&outcome, Source::System);
    let map = assert_track_file(system, SYSTEM_FORMAT, 20 * 480);
    assert_eq!(
        map.gaps(1_000),
        vec![Gap {
            frame: 10 * 480,
            from_us: first_end,
            to_us: 5_100_000
        }]
    );
    // No silence was invented: the file holds only what was delivered.
    assert_eq!(map.time_of_frame(10 * 480 - 1), Some(first_end - 21));
    assert_eq!(map.time_of_frame(10 * 480), Some(5_100_000));
    let finalised = system.finalised.as_ref().expect("finalised");
    assert_eq!(finalised.end_us, Some(5_200_000));
}

#[test]
fn a_system_track_with_nothing_played_is_a_valid_empty_file() {
    let recording = start();
    recording.script.deliver_run(Source::Microphone, 0, 5);

    let outcome = recording.session.stop();

    let system = track(&outcome, Source::System);
    assert_track_file(system, SYSTEM_FORMAT, 0);
    let finalised = system.finalised.as_ref().expect("finalised");
    assert_eq!((finalised.start_us, finalised.end_us), (None, None));
    assert!(system.stopped.is_none(), "silence is not a failure");
}

#[test]
fn a_device_flag_is_kept_with_the_packet_it_applies_to() {
    let recording = start();
    recording
        .script
        .deliver_flagged(Source::Microphone, 0, 480, flags::DISCONTINUITY);
    recording.script.deliver(Source::Microphone, 10_000, 480);

    let outcome = recording.session.stop();

    let map = assert_track_file(track(&outcome, Source::Microphone), MICROPHONE_FORMAT, 960);
    assert_eq!(map.entries[0].flags, flags::DISCONTINUITY);
    assert_eq!(map.entries[1].flags, 0);
}

#[test]
fn losing_one_device_keeps_the_other_recording_and_nothing_takes_its_place() {
    let recording = start();
    recording.script.deliver_run(Source::Microphone, 0, 10);
    recording.script.deliver_run(Source::System, 0, 10);

    recording
        .script
        .lose_device(Source::System, "the device was removed");

    // The lost track reports it; the healthy one does not.
    let status = recording.session.status();
    let system = &status.tracks[1];
    assert_eq!(system.source, Source::System);
    let stop = system.stopped.as_ref().expect("the system track stopped");
    assert_eq!(stop.reason, StopReason::DeviceLost);
    assert_eq!(stop.detail, "the device was removed");
    assert!(status.tracks[0].stopped.is_none());
    // No device is switched in behind the user's back.
    assert!(!recording.script.is_capturing(Source::System));
    assert!(!recording.script.deliver(Source::System, 100_000, 480));

    recording
        .script
        .deliver_run(Source::Microphone, 100_000, 10);
    let events = recording.session.take_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, Source::System);
    assert!(matches!(
        events[0].1.kind,
        HealthEventKind::DeviceLost { .. }
    ));

    let outcome = recording.session.stop();

    let microphone = track(&outcome, Source::Microphone);
    assert!(microphone.stopped.is_none());
    assert_track_file(microphone, MICROPHONE_FORMAT, 20 * 480);
    // The lost track is a valid partial file holding everything up to the loss.
    let system = track(&outcome, Source::System);
    assert_eq!(
        system.stopped.as_ref().map(|stop| stop.reason),
        Some(StopReason::DeviceLost)
    );
    assert_track_file(system, SYSTEM_FORMAT, 10 * 480);
}

#[test]
fn status_reports_each_tracks_frames_and_level() {
    let recording = start();
    recording.script.deliver_run(Source::Microphone, 0, 20);

    let first = recording.session.status();
    assert_eq!(first.tracks[0].frames, 20 * 480);
    assert_eq!(first.tracks[0].dropped_frames, 0);
    assert_eq!(first.tracks[1].frames, 0);
    assert!(first.elapsed_us >= 0);

    // The level comes from the writer thread, so it follows the audio by a moment.
    let mut loudest: f32 = 0.0;
    let deadline = Instant::now() + Duration::from_secs(10);
    while loudest < 0.2 {
        assert!(Instant::now() < deadline, "no level reported");
        loudest = loudest.max(recording.session.status().tracks[0].peak);
        thread::sleep(Duration::from_millis(5));
    }
    assert!((0.2..=0.26).contains(&loudest), "{loudest}");
    // The system track has delivered nothing and shows no level.
    assert_eq!(recording.session.status().tracks[1].peak, 0.0);
    recording.session.stop();
}

#[test]
fn start_is_refused_with_the_reason_when_a_device_is_missing() {
    let (backend, script) = SyntheticBackend::scripted();
    script.fail_open(
        Source::System,
        CaptureErrorKind::NoDevice,
        "no playback device",
    );

    let error = open_devices(&backend, &DeviceChoices::default()).expect_err("refused");

    assert_eq!(error.source_name, Source::System);
    assert_eq!(error.kind, CaptureErrorKind::NoDevice);
    assert_eq!(error.detail, "no playback device");
}

#[test]
fn each_track_is_recorded_from_the_device_chosen_for_it() {
    let dir = tempfile::tempdir().expect("temporary folder");
    let (backend, script) = SyntheticBackend::scripted();
    let listed = backend.devices(Source::System).expect("list devices");
    assert_eq!(listed.len(), 2);
    assert!(listed[0].is_default && listed[1].is_communications_default);
    let choices = DeviceChoices {
        microphone: DeviceChoice::Specific(
            backend.devices(Source::Microphone).expect("list")[1]
                .device
                .id
                .clone(),
        ),
        // Follow the device meeting apps play through, which is not the default one here.
        system: DeviceChoice::Communications,
    };

    let devices = open_devices(&backend, &choices).expect("open");
    let session =
        RecordingSession::start(devices, |source| files(dir.path(), source)).expect("start");
    script.deliver_run(Source::System, 0, 5);
    let outcome = session.stop();

    let microphone = track(&outcome, Source::Microphone);
    let system = track(&outcome, Source::System);
    assert_eq!(microphone.device.id, "synthetic:microphone:second");
    assert_eq!(system.device, listed[1].device);
    assert_eq!(
        system.finalised.as_ref().expect("finalised").frames,
        5 * 480
    );
}

#[test]
fn a_chosen_device_that_is_gone_refuses_the_start_instead_of_recording_another() {
    let (backend, script) = SyntheticBackend::scripted();
    let chosen = backend.devices(Source::System).expect("list")[1]
        .device
        .id
        .clone();
    let choices = DeviceChoices {
        microphone: DeviceChoice::Default,
        system: DeviceChoice::Specific(chosen),
    };
    // The chosen playback device is unplugged; the default one is still there.
    let mut remaining = backend.devices(Source::System).expect("list");
    remaining.truncate(1);
    script.set_devices(Source::System, remaining);

    let error = open_devices(&backend, &choices).expect_err("refused");

    assert_eq!(error.source_name, Source::System);
    assert_eq!(error.kind, CaptureErrorKind::ChosenDeviceMissing);
    assert!(!script.is_capturing(Source::Microphone));
}

#[test]
fn a_device_format_that_cannot_be_recorded_is_refused_before_anything_starts() {
    let (backend, script) = SyntheticBackend::scripted();
    script.set_format(
        Source::Microphone,
        AudioFormat {
            bits_per_sample: 64,
            valid_bits: 64,
            kind: SampleKind::Float,
            ..MICROPHONE_FORMAT
        },
    );

    let error = open_devices(&backend, &DeviceChoices::default()).expect_err("refused");

    assert_eq!(error.source_name, Source::Microphone);
    assert_eq!(error.kind, CaptureErrorKind::FormatUnsupported);
    assert!(error.detail.contains("64-bit float"), "{}", error.detail);
}

#[test]
fn a_start_that_fails_part_way_stops_both_devices_and_leaves_no_files() {
    let dir = tempfile::tempdir().expect("temporary folder");
    let (backend, script) = SyntheticBackend::scripted();
    script.fail_start(Source::System, "the device is in use");
    let devices = open_devices(&backend, &DeviceChoices::default()).expect("both devices open");

    let result = RecordingSession::start(devices, |source| files(dir.path(), source));

    match result {
        Err(AudioError::Capture(error)) => {
            assert_eq!(error.source_name, Source::System);
            assert_eq!(error.detail, "the device is in use");
        }
        other => panic!("expected the start to be refused, got {other:?}"),
    }
    // The microphone had started; it is stopped again, and nothing was recorded.
    assert!(!script.is_capturing(Source::Microphone));
    assert_eq!(fs::read_dir(dir.path()).expect("list").count(), 0);
}

#[test]
fn an_existing_track_file_is_never_overwritten() {
    let dir = tempfile::tempdir().expect("temporary folder");
    let existing = files(dir.path(), Source::System);
    fs::write(&existing.clock, b"someone else's file").expect("write");
    let (backend, script) = SyntheticBackend::scripted();
    let devices = open_devices(&backend, &DeviceChoices::default()).expect("open");

    let result = RecordingSession::start(devices, |source| files(dir.path(), source));

    assert!(matches!(result, Err(AudioError::Io { .. })), "{result:?}");
    assert!(!script.is_capturing(Source::Microphone));
    // The file that was there is untouched; everything this start created is gone.
    assert_eq!(
        fs::read(&existing.clock).expect("read"),
        b"someone else's file"
    );
    assert_eq!(fs::read_dir(dir.path()).expect("list").count(), 1);
}

#[test]
fn a_build_without_a_capture_backend_says_capture_is_unavailable() {
    let error =
        open_devices(&UnavailableBackend, &DeviceChoices::default()).expect_err("unavailable");

    assert_eq!(error.kind, CaptureErrorKind::Unavailable);
    assert!(!UnavailableBackend.info().synthetic);
}

fn copy_track(from: &TrackFiles, to_dir: &Path) -> TrackFiles {
    let to = TrackFiles::in_dir(to_dir, "crashed");
    fs::copy(&from.wav, &to.wav).expect("copy wav");
    fs::copy(&from.journal, &to.journal).expect("copy journal");
    fs::copy(&from.clock, &to.clock).expect("copy clock");
    to
}

#[test]
fn a_crash_mid_recording_loses_no_more_than_the_last_flush_interval() {
    let recording = start();
    let live_files = files(recording.dir.path(), Source::Microphone);

    // One second of audio, then long enough for the writer's periodic flush to commit it.
    let first_end = recording.script.deliver_run(Source::Microphone, 0, 100);
    wait_for("the first second to be committed", || {
        journal::read(&live_files.journal)
            .is_ok_and(|state| state.committed.data_bytes == 100 * 480 * 4)
    });
    // More audio arrives and is written, but the process dies before the next flush.
    recording
        .script
        .deliver_run(Source::Microphone, first_end, 30);
    wait_for("the later audio to reach the file", || {
        fs::metadata(&live_files.wav)
            .is_ok_and(|meta| meta.len() == wav::HEADER_LEN + 130 * 480 * 4)
    });
    let crash = tempfile::tempdir().expect("temporary folder");
    let crashed = copy_track(&live_files, crash.path());
    // What a crash leaves is not a valid track: its header says it holds no audio.
    let left = fs::read(&crashed.wav).expect("read");
    assert_eq!(wav::read_header(&left).expect("header").data_bytes, 0);
    // The journal says how much was confirmed on disk at that instant: the first second, and
    // on a machine that stalled for a whole flush interval just here, the rest as well.
    let committed = journal::read(&crashed.journal)
        .expect("journal")
        .committed
        .data_bytes;
    let committed_frames = committed / 4;
    assert!(
        (100 * 480..=130 * 480).contains(&committed_frames),
        "{committed_frames}"
    );
    assert!(left.len() as u64 >= wav::HEADER_LEN + committed);

    let recovered = recover(&crashed).expect("recover");

    assert!(!recovered.was_finalised);
    assert_eq!(recovered.format, MICROPHONE_FORMAT);
    // Everything committed is kept; only the unconfirmed tail, under a flush interval, goes.
    assert_eq!(recovered.track.frames, committed_frames);
    assert_eq!(recovered.track.start_us, Some(0));
    let unconfirmed = Duration::from_micros((130 * 480 - committed_frames) / 48 * 1_000);
    assert!(unconfirmed < FLUSH_INTERVAL);
    let bytes = fs::read(&crashed.wav).expect("read");
    let info = wav::read_header(&bytes).expect("valid header");
    assert_eq!(info.data_bytes, committed);
    assert_eq!(bytes.len() as u64, wav::HEADER_LEN + committed);
    assert_eq!(recovered.track.sha256, sha256_hex(&bytes));
    let expected = signal(
        Source::Microphone,
        0,
        u32::try_from(committed_frames).expect("small"),
    );
    assert!(bytes[wav::HEADER_LEN as usize..] == expected[..]);
    let map = ClockMap::read(&crashed.clock).expect("clock map");
    assert_eq!(map.frames(), committed_frames);
    assert_eq!(map.end_us(), recovered.track.end_us);
    if committed_frames == 100 * 480 {
        assert_eq!(recovered.track.end_us, Some(first_end));
        assert_eq!(map.entries.len(), 100);
    }

    // Recovering again changes nothing and says the track was already complete.
    let again = recover(&crashed).expect("recover again");
    assert!(again.was_finalised);
    assert_eq!(again.track, recovered.track);
    assert_eq!(fs::read(&crashed.wav).expect("read"), bytes);

    // The live recording was not disturbed and finishes with everything.
    let outcome = recording.session.stop();
    assert_track_file(
        track(&outcome, Source::Microphone),
        MICROPHONE_FORMAT,
        130 * 480,
    );
}

#[test]
fn a_track_interrupted_before_its_first_flush_recovers_as_a_valid_empty_file() {
    // The state a crash leaves in the first second: audio and clock entries written, and a
    // journal that has its header but no record yet. Built by hand so no timing is involved.
    let dir = tempfile::tempdir().expect("temporary folder");
    let crashed = files(dir.path(), Source::System);
    let mut audio = wav::header(&SYSTEM_FORMAT, 0).to_vec();
    audio.extend_from_slice(&signal(Source::System, 0, 3 * 480));
    fs::write(&crashed.wav, &audio).expect("write audio");
    let mut entries = crate::clock::header(&SYSTEM_FORMAT).to_vec();
    crate::clock::ClockEntry {
        frame: 0,
        timeline_us: 0,
        frames: 3 * 480,
        flags: 0,
    }
    .encode(&mut entries);
    fs::write(&crashed.clock, &entries).expect("write clock map");
    drop(crate::journal::Journal::create(&crashed.journal, &SYSTEM_FORMAT).expect("journal"));

    let recovered = recover(&crashed).expect("recover");

    // Nothing was confirmed on disk, so nothing is claimed.
    assert_eq!(recovered.track.frames, 0);
    assert_eq!(recovered.track.start_us, None);
    let bytes = fs::read(&crashed.wav).expect("read");
    assert_eq!(bytes.len() as u64, wav::HEADER_LEN);
    assert_eq!(
        wav::read_header(&bytes).expect("header").format,
        SYSTEM_FORMAT
    );
    assert!(
        ClockMap::read(&crashed.clock)
            .expect("clock map")
            .entries
            .is_empty()
    );
}

#[test]
fn recovery_refuses_a_track_shorter_than_its_journal_says_and_changes_nothing() {
    let recording = start();
    recording.script.deliver_run(Source::Microphone, 0, 10);
    let outcome = recording.session.stop();
    let finished = &track(&outcome, Source::Microphone).files;
    let truncated = fs::read(&finished.wav).expect("read")[..1_000].to_vec();
    fs::write(&finished.wav, &truncated).expect("truncate");

    let result = recover(finished);

    assert!(
        matches!(result, Err(AudioError::WavInvalid { .. })),
        "{result:?}"
    );
    assert_eq!(fs::read(&finished.wav).expect("read"), truncated);
}

#[test]
fn recovery_without_a_usable_journal_is_refused() {
    let dir = tempfile::tempdir().expect("temporary folder");
    let missing = files(dir.path(), Source::Microphone);
    assert!(matches!(recover(&missing), Err(AudioError::Io { .. })));

    fs::write(&missing.journal, b"not a journal").expect("write");
    assert!(matches!(
        recover(&missing),
        Err(AudioError::JournalInvalid { .. })
    ));
}

#[test]
fn a_full_capture_buffer_drops_frames_records_the_gap_and_keeps_capturing() {
    let dir = tempfile::tempdir().expect("temporary folder");
    let track_files = files(dir.path(), Source::Microphone);
    let format = MICROPHONE_FORMAT;
    let live = Arc::new(TrackLive::default());
    let clock = SessionClock::start();
    // Room for exactly two packets. The writer has not started, so the third does not fit.
    let packet_bytes = 480 * 4;
    let (producer, consumer) = ring::ring(2 * packet_bytes, 16);
    let meta = |index: i64| PacketMeta {
        timeline_us: index * 10_000,
        frames: PACKET_FRAMES,
        flags: 0,
    };
    let data = |index: u64| signal(Source::Microphone, index * 480, PACKET_FRAMES);
    assert_eq!(producer.push(meta(0), &data(0), 10_000), Pushed::Stored);
    assert_eq!(producer.push(meta(1), &data(1), 10_000), Pushed::Stored);
    assert_eq!(producer.push(meta(2), &data(2), 10_000), Pushed::Dropped);
    assert_eq!(producer.push(meta(3), &data(3), 10_000), Pushed::Dropped);

    let writer = Writer::start(
        Source::Microphone,
        &track_files,
        format,
        clock,
        consumer,
        Arc::clone(&live),
    )
    .expect("start writer");
    // Once the writer has caught up there is room again and capture simply continues.
    wait_for("the writer to drain the buffer", || {
        fs::metadata(&track_files.wav)
            .is_ok_and(|meta| meta.len() == wav::HEADER_LEN + 2 * packet_bytes as u64)
    });
    assert_eq!(producer.push(meta(4), &data(4), 10_000), Pushed::Stored);
    producer.close();
    let end = writer.finish(Source::Microphone);

    assert!(end.error.is_none(), "{:?}", end.error);
    assert_eq!(end.committed.data_bytes, 3 * packet_bytes as u64);
    assert_eq!(end.committed.dropped_frames, 2 * 480);
    assert_eq!(
        live.take_events()
            .into_iter()
            .map(|event| (event.at_us, event.kind))
            .collect::<Vec<_>>(),
        vec![(
            20_000,
            HealthEventKind::Overflow {
                frames: 960,
                until_us: 40_000
            }
        )]
    );
    // The frames after the drop are contiguous in the file and flagged in the clock map,
    // where the missing 20 ms shows as a gap.
    let map = ClockMap::read(&track_files.clock).expect("clock map");
    assert_eq!(map.entries.len(), 3);
    assert_eq!(map.entries[2].flags, flags::AFTER_DROP);
    assert_eq!(map.entries[2].frame, 960);
    assert_eq!(
        map.gaps(1_000),
        vec![Gap {
            frame: 960,
            from_us: 20_000,
            to_us: 40_000
        }]
    );
}

#[test]
fn the_tone_backend_records_both_tracks_in_real_time() {
    let dir = tempfile::tempdir().expect("temporary folder");
    let backend = SyntheticBackend::tone();
    assert!(backend.info().synthetic);
    let devices = open_devices(&backend, &DeviceChoices::default()).expect("open");
    let session =
        RecordingSession::start(devices, |source| files(dir.path(), source)).expect("start");

    wait_for("both tracks to deliver audio", || {
        session
            .status()
            .tracks
            .iter()
            .all(|track| track.frames >= 4_800)
    });
    let outcome = session.stop();

    for source in Source::BOTH {
        let track = track(&outcome, source);
        let finalised = track.finalised.as_ref().expect("finalised");
        assert!(finalised.frames >= 4_800, "{source}");
        assert!(track.device.name.starts_with("Generated test tone"));
        let bytes = fs::read(&track.files.wav).expect("read");
        assert_eq!(finalised.sha256, sha256_hex(&bytes));
        let map = ClockMap::read(&track.files.clock).expect("clock map");
        assert_eq!(map.frames(), finalised.frames);
        // Real time: timestamps advance by about the length of the audio delivered.
        let span = map.end_us().expect("end") - map.start_us().expect("start");
        let audio = track.format.duration_us(finalised.frames);
        assert!((span - audio).abs() < 50_000, "{span} vs {audio}");
    }
    assert!(outcome.duration_us >= 100_000);
}

/// Resident memory of this process in bytes, where the platform makes it easy to read.
fn resident_bytes() -> Option<u64> {
    let statm = fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4_096)
}

/// A long recording, delivered as fast as the writers accept it rather than in real time.
/// It is not run by default because it writes about 23 MB per simulated minute:
///
/// ```text
/// SKRIBE_SOAK_MINUTES=120 cargo test --release -p skribe-audio -- --ignored long_recording
/// ```
#[test]
#[ignore = "writes gigabytes; run on request with SKRIBE_SOAK_MINUTES"]
fn a_long_recording_stays_within_a_fixed_memory_bound() {
    let minutes: u64 = std::env::var("SKRIBE_SOAK_MINUTES")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(10);
    let recording = start();
    let seconds = minutes * 60;
    let wav_len = |source: Source| {
        fs::metadata(files(recording.dir.path(), source).wav).map_or(0, |meta| meta.len())
    };
    let bytes_per_second = |source: Source| match source {
        Source::Microphone => MICROPHONE_FORMAT.bytes_per_second(),
        Source::System => SYSTEM_FORMAT.bytes_per_second(),
    };

    let mut baseline = None;
    let mut peak = 0;
    for second in 0..seconds {
        let from_us = i64::try_from(second).expect("small") * 1_000_000;
        for source in Source::BOTH {
            recording.script.deliver_run(source, from_us, 100);
        }
        // Wait for the writers, so the test measures steady state rather than a backlog.
        for source in Source::BOTH {
            let expected = wav::HEADER_LEN + (second + 1) * bytes_per_second(source);
            wait_for("the writer to catch up", || wav_len(source) >= expected);
        }
        if let Some(resident) = resident_bytes() {
            // Take the baseline after a minute, once every buffer has reached its full size.
            if second == 60.min(seconds - 1) {
                baseline = Some(resident);
            }
            peak = peak.max(resident);
        }
    }
    let status = recording.session.status();
    assert!(
        status.tracks.iter().all(|track| track.dropped_frames == 0),
        "{status:?}"
    );

    let outcome = recording.session.stop();

    for source in Source::BOTH {
        let finalised = track(&outcome, source)
            .finalised
            .as_ref()
            .expect("finalised");
        assert_eq!(finalised.frames, seconds * 48_000, "{source}");
        assert_eq!(finalised.dropped_frames, 0);
        assert_eq!(
            finalised.end_us,
            Some(i64::try_from(seconds).expect("small") * 1_000_000)
        );
    }
    if let Some(baseline) = baseline {
        let growth = peak.saturating_sub(baseline);
        println!(
            "{minutes} simulated minutes: resident memory {} MiB after the first minute, peak {} \
             MiB, growth {} KiB",
            baseline >> 20,
            peak >> 20,
            growth >> 10
        );
        // Both ring buffers together hold under 1 MiB; anything like this much growth would
        // mean memory proportional to the length of the recording.
        assert!(growth < 8 << 20, "memory grew by {growth} bytes");
    }
}
