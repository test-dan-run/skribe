//! Recording: the service that owns the one live capture session, and its commands. Which
//! devices it records from is a setting, kept in the `devices` module.
//!
//! The service connects the audio crate, which knows devices and files, to the store, which
//! knows meetings. It holds its lock only to read or swap state, never while opening devices,
//! writing to the database or finalising, so a status request is answered at once even while
//! a recording is being saved.

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use skribe_audio::{
    AudioFormat, CaptureBackend, FinalisedTrack, HealthEventKind, OpenDevices, RecordingSession,
    SessionStatus, Source, StopReason, TrackFiles, TrackOutcome, TrackStop, open_devices,
    preflight,
};
use skribe_domain::{MeetingId, RecordingState, TrackFinalisation, TrackHealth, TrackId};
use skribe_store::{
    HealthEvent, Library, LibraryService, NewTrack, RecordingDeviceSettings, StoreError,
    TrackFormat, TrackRecord, TrackResult,
};

use crate::meetings::{MeetingDetail, MeetingIdArgs, detail, meeting_id};
use crate::states::{self, TrackSource};
use crate::{CommandError, open_library};

/// One of a recording's tracks as stored: what it was recorded from and how it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackSummary {
    pub id: String,
    pub source: TrackSource,
    pub device_name: String,
    /// The device's native format, in which the track is stored, for display.
    pub format: String,
    pub finalisation: states::TrackFinalisation,
    pub health: states::TrackHealth,
    /// Why the track is degraded or failed, for the user. `None` for a healthy track.
    pub problem: Option<String>,
    /// Where on the timeline the track stopped, when it stopped before the recording did.
    pub stopped_at_us: Option<i64>,
    /// Timeline time of the first frame. `None` when the track holds no audio.
    pub start_us: Option<i64>,
    /// Length of the audio in the track, at its nominal sample rate.
    pub audio_us: Option<u64>,
    pub size_bytes: Option<u64>,
    pub dropped_frames: u64,
    /// SHA-256 of the finalised file.
    pub sha256: Option<String>,
}

/// What a start would record from, and how much room there is for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PreflightReport {
    /// What this build captures with.
    pub capture_name: String,
    /// The audio would be a generated test signal, not real devices. Development builds only.
    pub synthetic: bool,
    pub devices: Vec<PreflightDevice>,
    /// How long a recording the free disk space allows at these formats.
    pub available_seconds: u64,
    /// The least room a recording needs before it is allowed to start.
    pub minimum_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PreflightDevice {
    pub source: TrackSource,
    pub name: String,
    pub format: String,
    /// Why it is this device: it is the Windows default, the Windows default communications
    /// device, or the device chosen in Settings.
    pub choice: DeviceChoice,
}

/// A track while it is being recorded.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LiveTrack {
    pub source: TrackSource,
    pub device_name: String,
    pub format: String,
    /// Length of the audio captured so far, at the nominal sample rate.
    pub captured_us: u64,
    /// Frames dropped because the computer could not keep up.
    pub dropped_frames: u64,
    /// Loudest level since the last status, from 0 to 1.
    pub level: f32,
    /// Whether the device is still delivering. A stopped track is not replaced by another
    /// device; the other track carries on.
    pub capturing: bool,
    /// Why the track stopped, for the user.
    pub problem: Option<String>,
    pub stopped_at_us: Option<i64>,
}

/// What the recording service is doing.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(
    tag = "phase",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RecordingStatus {
    Idle,
    /// Devices are being checked and opened.
    Starting {
        meeting_id: String,
    },
    Recording {
        meeting_id: String,
        /// The audio is a generated test signal, not real devices.
        synthetic: bool,
        /// Time since the recording's origin, on its own monotonic clock.
        elapsed_us: u64,
        tracks: Vec<LiveTrack>,
    },
    /// Capture has stopped and the tracks are being completed and checked.
    Finalising {
        meeting_id: String,
    },
}

/// Sent to the window when it was asked to close while a recording is live. The window stays
/// open: closing would end the recording without finalising it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CloseBlocked {
    pub message: String,
}

impl CloseBlocked {
    pub fn recording() -> Self {
        Self {
            message: "Skribe is recording. Stop the recording before closing the window, so \
                      that both tracks are saved completely."
                .to_owned(),
        }
    }
}

/// The name of the event that carries [`RecordingStatus`].
pub const RECORDING_STATUS_EVENT: &str = "recording-status";
/// The name of the event that carries [`CloseBlocked`].
pub const CLOSE_BLOCKED_EVENT: &str = "close-blocked";

fn track_source(source: Source) -> skribe_domain::TrackSource {
    match source {
        Source::Microphone => skribe_domain::TrackSource::Microphone,
        Source::System => skribe_domain::TrackSource::System,
    }
}

fn audio_us(sample_rate: u32, frames: u64) -> u64 {
    let micros = u128::from(frames) * 1_000_000 / u128::from(sample_rate.max(1));
    u64::try_from(micros).unwrap_or(u64::MAX)
}

fn describe_format(format: &TrackFormat) -> String {
    format!(
        "{} Hz, {} channel{}, {}-bit {}",
        format.sample_rate,
        format.channels,
        if format.channels == 1 { "" } else { "s" },
        format.bits_per_sample,
        if format.float { "float" } else { "integer" }
    )
}

fn track_format(format: &AudioFormat) -> TrackFormat {
    TrackFormat {
        sample_rate: format.sample_rate,
        channels: format.channels,
        bits_per_sample: format.bits_per_sample,
        valid_bits: format.valid_bits,
        float: format.kind == skribe_audio::SampleKind::Float,
        channel_mask: format.channel_mask,
    }
}

/// Stable codes for why a track is degraded or failed, stored with the track.
mod problem {
    pub const FRAMES_DROPPED: &str = "frames_dropped";
    pub const DEVICE_LOST: &str = "device_lost";
    pub const CAPTURE_FAILED: &str = "capture_failed";
    pub const WRITE_FAILED: &str = "write_failed";
    pub const FINALISE_FAILED: &str = "finalise_failed";
    pub const INTERRUPTED: &str = "interrupted";
    pub const RECOVERY_FAILED: &str = "recovery_failed";
}

/// What a stored problem code means, for the user.
fn problem_text(code: &str) -> String {
    match code {
        problem::FRAMES_DROPPED => {
            "Some audio was not recorded because the computer could not keep up. The gaps are \
             marked on the track's timeline."
        }
        problem::DEVICE_LOST => {
            "The device was disconnected or disabled during the recording. The track holds the \
             audio up to that point."
        }
        problem::CAPTURE_FAILED => {
            "The device reported an error during the recording. The track holds the audio up \
             to that point."
        }
        problem::WRITE_FAILED => {
            "The track could not be written to disk during the recording. It holds the audio \
             saved before that."
        }
        problem::FINALISE_FAILED => "The track's file could not be completed.",
        problem::INTERRUPTED => {
            "The recording was interrupted. The track was rebuilt from what had been saved; up \
             to the last second before the interruption may be missing."
        }
        problem::RECOVERY_FAILED => "The track could not be rebuilt after the interruption.",
        _ => "The track has a problem this version of Skribe cannot describe.",
    }
    .to_owned()
}

fn stop_code(reason: StopReason) -> &'static str {
    match reason {
        StopReason::DeviceLost => problem::DEVICE_LOST,
        StopReason::CaptureFailed => problem::CAPTURE_FAILED,
        StopReason::WriteFailed => problem::WRITE_FAILED,
    }
}

pub(crate) fn track_summary(track: &TrackRecord) -> TrackSummary {
    let result = &track.result;
    TrackSummary {
        id: result.id.to_string(),
        source: track.source.into(),
        device_name: track.device_name.clone(),
        format: describe_format(&track.format),
        finalisation: result.finalisation.into(),
        health: result.health.into(),
        problem: result.problem.as_deref().map(problem_text),
        stopped_at_us: result.stopped_at_us,
        start_us: result.start_us,
        audio_us: result
            .frame_count
            .map(|frames| audio_us(track.format.sample_rate, frames)),
        size_bytes: result.data_bytes,
        dropped_frames: result.dropped_frames,
        sha256: result.sha256.clone(),
    }
}

/// The result of a track that produced a valid file.
fn valid_result(
    id: TrackId,
    track: &FinalisedTrack,
    finalisation: TrackFinalisation,
    health: TrackHealth,
    problem: Option<&str>,
    stopped: Option<&TrackStop>,
) -> TrackResult {
    TrackResult {
        id,
        finalisation,
        health,
        problem: problem.map(str::to_owned),
        problem_detail: stopped.map(|stop| stop.detail.clone()),
        stopped_at_us: stopped.map(|stop| stop.at_us),
        start_us: track.start_us,
        end_us: track.end_us,
        frame_count: Some(track.frames),
        data_bytes: Some(track.data_bytes),
        dropped_frames: track.dropped_frames,
        sha256: Some(track.sha256.clone()),
    }
}

/// The result of a track with no valid file.
fn failed_result(
    id: TrackId,
    problem: &str,
    detail: String,
    stopped: Option<&TrackStop>,
) -> TrackResult {
    TrackResult {
        id,
        finalisation: TrackFinalisation::Failed,
        health: TrackHealth::Failed,
        problem: Some(problem.to_owned()),
        problem_detail: Some(detail),
        stopped_at_us: stopped.map(|stop| stop.at_us),
        start_us: None,
        end_us: None,
        frame_count: None,
        data_bytes: None,
        dropped_frames: 0,
        sha256: None,
    }
}

/// How a track that was stopped in the ordinary way ended. Its health is reported as it is:
/// a track that stopped early is failed even when it left a valid partial file.
fn stopped_result(id: TrackId, outcome: &TrackOutcome) -> TrackResult {
    let stopped = outcome.stopped.as_ref();
    match &outcome.finalised {
        Ok(track) => {
            let (health, problem) = match stopped {
                Some(stop) => (TrackHealth::Failed, Some(stop_code(stop.reason))),
                None if track.dropped_frames > 0 => {
                    (TrackHealth::Degraded, Some(problem::FRAMES_DROPPED))
                }
                None => (TrackHealth::Healthy, None),
            };
            valid_result(
                id,
                track,
                TrackFinalisation::Finalised,
                health,
                problem,
                stopped,
            )
        }
        Err(error) => failed_result(
            id,
            stopped.map_or(problem::FINALISE_FAILED, |stop| stop_code(stop.reason)),
            error.to_string(),
            stopped,
        ),
    }
}

fn store_event(track: TrackId, event: &skribe_audio::HealthEvent) -> HealthEvent {
    use skribe_store::HealthEventKind as Kind;
    let (kind, frames, until_us, detail) = match &event.kind {
        HealthEventKind::Overflow { frames, until_us } => {
            (Kind::Overflow, Some(*frames), Some(*until_us), None)
        }
        HealthEventKind::DeviceLost { detail } => {
            (Kind::DeviceLost, None, None, Some(detail.clone()))
        }
        HealthEventKind::CaptureFailed { detail } => {
            (Kind::CaptureFailed, None, None, Some(detail.clone()))
        }
        HealthEventKind::WriteFailed { detail } => {
            (Kind::WriteFailed, None, None, Some(detail.clone()))
        }
    };
    HealthEvent {
        track,
        at_us: event.at_us,
        kind,
        frames,
        until_us,
        detail,
    }
}

fn now_unix_ms() -> u64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

fn stored<T>(result: Result<T, StoreError>) -> Result<T, CommandError> {
    result.map_err(|error| CommandError::from(&error))
}

/// `path` relative to the library folder, with forward slashes, as tracks store it.
fn library_relative(library: &Library, path: &Path) -> String {
    path.strip_prefix(library.roots().library())
        .unwrap_or(path)
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

struct ActiveTrack {
    source: Source,
    id: TrackId,
    device_name: String,
    format: AudioFormat,
}

struct Active {
    meeting: MeetingId,
    synthetic: bool,
    tracks: Vec<ActiveTrack>,
    session: RecordingSession,
    /// The status as of the last tick. Reading levels resets them, so only the tick reads
    /// the session and everything else is answered from this copy.
    last: RecordingStatus,
}

impl Active {
    fn id_of(&self, source: Source) -> Option<TrackId> {
        self.tracks
            .iter()
            .find(|track| track.source == source)
            .map(|track| track.id)
    }

    fn describe(&self, status: &SessionStatus) -> RecordingStatus {
        RecordingStatus::Recording {
            meeting_id: self.meeting.to_string(),
            synthetic: self.synthetic,
            elapsed_us: u64::try_from(status.elapsed_us).unwrap_or(0),
            tracks: self
                .tracks
                .iter()
                .zip(&status.tracks)
                .map(|(track, live)| LiveTrack {
                    source: track_source(track.source).into(),
                    device_name: track.device_name.clone(),
                    format: describe_format(&track_format(&track.format)),
                    captured_us: audio_us(track.format.sample_rate, live.frames),
                    dropped_frames: live.dropped_frames,
                    level: live.peak,
                    capturing: live.stopped.is_none(),
                    problem: live
                        .stopped
                        .as_ref()
                        .map(|stop| problem_text(stop_code(stop.reason))),
                    stopped_at_us: live.stopped.as_ref().map(|stop| stop.at_us),
                })
                .collect(),
        }
    }

    /// Health events since the last call, ready to be saved.
    fn take_events(&self) -> Vec<HealthEvent> {
        self.session
            .take_events()
            .iter()
            .filter_map(|(source, event)| Some(store_event(self.id_of(*source)?, event)))
            .collect()
    }
}

enum State {
    Idle,
    Starting(MeetingId),
    Recording(Box<Active>),
    Finalising(MeetingId),
}

/// Owns the one recording an app instance can have in progress.
pub struct RecordingService {
    library: Arc<LibraryService>,
    backend: Arc<dyn CaptureBackend>,
    state: Mutex<State>,
}

impl std::fmt::Debug for RecordingService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingService")
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
}

/// Returns the service to idle if a start or stop ends early, including by panic, so one
/// failure cannot leave the app unable to record.
struct ResetToIdle<'a> {
    service: &'a RecordingService,
    armed: bool,
}

impl Drop for ResetToIdle<'_> {
    fn drop(&mut self) {
        if self.armed {
            *self.service.lock() = State::Idle;
        }
    }
}

impl RecordingService {
    pub fn new(library: Arc<LibraryService>, backend: Arc<dyn CaptureBackend>) -> Self {
        Self {
            library,
            backend,
            state: Mutex::new(State::Idle),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether a recording is starting, running or being saved.
    pub fn is_live(&self) -> bool {
        !matches!(*self.lock(), State::Idle)
    }

    /// The current status, without touching devices or the database.
    pub fn status(&self) -> RecordingStatus {
        match &*self.lock() {
            State::Idle => RecordingStatus::Idle,
            State::Starting(meeting) => RecordingStatus::Starting {
                meeting_id: meeting.to_string(),
            },
            State::Recording(active) => active.last.clone(),
            State::Finalising(meeting) => RecordingStatus::Finalising {
                meeting_id: meeting.to_string(),
            },
        }
    }

    /// Reads fresh track levels and saves any new health events. Called a few times a
    /// second while the app runs; it does nothing when no recording is live.
    pub fn tick(&self) -> RecordingStatus {
        let events = {
            let mut state = self.lock();
            let State::Recording(active) = &mut *state else {
                drop(state);
                return self.status();
            };
            active.last = active.describe(&active.session.status());
            active.take_events()
        };
        if !events.is_empty() {
            // The clock map already marks every gap; a failure here loses only the summary.
            if let Ok(library) = open_library(&self.library) {
                let _ = library.add_health_events(events);
            }
        }
        self.status()
    }

    /// The meeting, checked to be one that can be recorded now.
    fn recordable(&self, meeting: MeetingId) -> Result<Arc<Library>, CommandError> {
        let library = open_library(&self.library)?;
        let found = stored(library.meeting(meeting))?.meeting;
        if found.trashed_at_unix_ms.is_some() {
            return Err(CommandError::from(&StoreError::MeetingInTrash));
        }
        if found.recording_state != RecordingState::Setup {
            return Err(CommandError::from(&StoreError::RecordingStateConflict {
                from: found.recording_state,
                to: RecordingState::Ready,
            }));
        }
        Ok(library)
    }

    /// Opens the chosen devices and checks the disk, without recording or changing anything.
    /// A chosen device that is not connected is an error here: nothing else is opened in
    /// its place.
    fn check(
        &self,
        library: &Library,
        meeting: MeetingId,
    ) -> Result<(OpenDevices, RecordingDeviceSettings, preflight::DiskSpace), CommandError> {
        let (settings, choices) = devices::saved_choices(library)?;
        let devices =
            open_devices(&*self.backend, &choices).map_err(|error| CommandError::from(&error))?;
        // The meeting's own folder is created only when recording starts; its parent is
        // where the space is measured.
        let folder = library.roots().meeting_audio_dir(&meeting);
        let audio_root = folder.parent().unwrap_or(&folder);
        let space = preflight::check_disk_space(audio_root, &devices.formats())
            .map_err(|error| CommandError::from(&error))?;
        Ok((devices, settings, space))
    }

    /// `recording_preflight`: what a start would record from and how much room there is.
    pub fn preflight(&self, meeting: MeetingId) -> Result<PreflightReport, CommandError> {
        if self.is_live() {
            return Err(CommandError::recording_in_progress());
        }
        let library = self.recordable(meeting)?;
        let (devices, settings, space) = self.check(&library, meeting)?;
        let info = self.backend.info();
        Ok(PreflightReport {
            capture_name: info.name.to_owned(),
            synthetic: info.synthetic,
            devices: devices
                .tracks()
                .iter()
                .map(|track| PreflightDevice {
                    source: track_source(track.source).into(),
                    name: track.device.name.clone(),
                    format: describe_format(&track_format(&track.format)),
                    choice: devices::choice(match track.source {
                        Source::Microphone => &settings.microphone,
                        Source::System => &settings.system,
                    }),
                })
                .collect(),
            available_seconds: space.available_duration().as_secs(),
            minimum_seconds: preflight::MINIMUM_RECORDING.as_secs(),
        })
    }

    /// `recording_start`: preflight, then both tracks start against one clock. Refused with
    /// the specific reason, and with nothing recorded, if any check or device fails.
    pub fn start(&self, meeting: MeetingId) -> Result<RecordingStatus, CommandError> {
        {
            let mut state = self.lock();
            if !matches!(*state, State::Idle) {
                return Err(CommandError::recording_in_progress());
            }
            *state = State::Starting(meeting);
        }
        let mut reset = ResetToIdle {
            service: self,
            armed: true,
        };
        let active = self.start_session(meeting)?;
        let status = active.last.clone();
        *self.lock() = State::Recording(Box::new(active));
        reset.armed = false;
        Ok(status)
    }

    fn start_session(&self, meeting: MeetingId) -> Result<Active, CommandError> {
        let library = self.recordable(meeting)?;
        let (devices, _settings, _space) = self.check(&library, meeting)?;
        let folder = library.roots().meeting_audio_dir(&meeting);
        // An empty folder left by a refused start is removed again.
        let remove_empty_folder = || {
            let _ = fs::remove_dir(&folder);
        };
        if let Err(error) = preflight::check_writable(&folder) {
            remove_empty_folder();
            return Err(CommandError::from(&error));
        }

        let tracks: Vec<ActiveTrack> = devices
            .tracks()
            .iter()
            .map(|track| ActiveTrack {
                source: track.source,
                id: TrackId::generate(),
                device_name: track.device.name.clone(),
                format: track.format,
            })
            .collect();
        let files_for = |source: Source| {
            let id = tracks
                .iter()
                .find(|track| track.source == source)
                .map_or_else(TrackId::generate, |track| track.id);
            TrackFiles::in_dir(&folder, &id.to_string())
        };
        let registered = devices
            .tracks()
            .iter()
            .zip(&tracks)
            .map(|(device, track)| NewTrack {
                id: track.id,
                source: track_source(track.source),
                device_id: device.device.id.clone(),
                device_name: device.device.name.clone(),
                format: track_format(&track.format),
                audio_path: library_relative(&library, &files_for(track.source).wav),
            })
            .collect();
        if let Err(error) = library.begin_recording(meeting, registered) {
            remove_empty_folder();
            return Err(CommandError::from(&error));
        }

        let session = match RecordingSession::start(devices, files_for) {
            Ok(session) => session,
            Err(error) => {
                let _ = library.abandon_recording_start(meeting);
                remove_empty_folder();
                return Err(CommandError::from(&error));
            }
        };
        if let Err(error) = library.mark_recording_started(meeting, now_unix_ms()) {
            // Capture had begun, but the library does not know: stop, and remove the few
            // milliseconds this attempt wrote, so the meeting is exactly as it was.
            for track in session.stop().tracks {
                for file in [&track.files.wav, &track.files.journal, &track.files.clock] {
                    let _ = fs::remove_file(file);
                }
            }
            let _ = library.abandon_recording_start(meeting);
            remove_empty_folder();
            return Err(CommandError::from(&error));
        }

        let mut active = Active {
            meeting,
            synthetic: self.backend.info().synthetic,
            tracks,
            session,
            last: RecordingStatus::Idle,
        };
        active.last = active.describe(&active.session.status());
        Ok(active)
    }

    /// `recording_stop`: stops both tracks together, finalises each, and reports exactly how
    /// each one ended. Blocks while the tracks are completed and hashed.
    pub fn stop(&self, meeting: MeetingId) -> Result<MeetingDetail, CommandError> {
        let active = {
            let mut state = self.lock();
            match std::mem::replace(&mut *state, State::Finalising(meeting)) {
                State::Recording(active) if active.meeting == meeting => active,
                other => {
                    *state = other;
                    return Err(CommandError::recording_not_active());
                }
            }
        };
        let _reset = ResetToIdle {
            service: self,
            armed: true,
        };

        // The audio is made safe first, whatever state the library is in.
        let library = open_library(&self.library);
        let marked = library
            .as_ref()
            .is_ok_and(|library| library.mark_finalising(meeting).is_ok());
        let mut events = active.take_events();
        let Active {
            tracks, session, ..
        } = *active;
        let outcome = session.stop();

        let mut results = Vec::with_capacity(outcome.tracks.len());
        for track in &outcome.tracks {
            let Some(id) = tracks
                .iter()
                .find(|known| known.source == track.source)
                .map(|known| known.id)
            else {
                continue;
            };
            events.extend(track.events.iter().map(|event| store_event(id, event)));
            if track.events_discarded > 0 {
                events.push(HealthEvent {
                    track: id,
                    at_us: outcome.duration_us,
                    kind: skribe_store::HealthEventKind::EventsDiscarded,
                    frames: Some(track.events_discarded),
                    until_us: None,
                    detail: None,
                });
            }
            results.push(stopped_result(id, track));
        }

        let library = library.map_err(|error| {
            CommandError::recording_not_registered(error.detail.unwrap_or(error.message))
        })?;
        let saved = (|| {
            if !marked {
                library.mark_finalising(meeting)?;
            }
            library.add_health_events(events)?;
            library.finish_recording(meeting, Some(outcome.duration_us), results)
        })();
        saved
            .map(detail)
            .map_err(CommandError::recording_not_registered)
    }

    /// `recording_recover`: rebuilds each track of an interrupted recording from its journal
    /// and settles the meeting. Nothing is deleted: a track that cannot be rebuilt is left
    /// as it is and reported as failed.
    pub fn recover(&self, meeting: MeetingId) -> Result<MeetingDetail, CommandError> {
        let library = open_library(&self.library)?;
        let found = stored(library.meeting(meeting))?;
        if found.meeting.recording_state != RecordingState::Interrupted {
            return Err(CommandError::from(&StoreError::RecordingStateConflict {
                from: found.meeting.recording_state,
                to: RecordingState::Recorded,
            }));
        }
        // File locations follow from the meeting and track IDs, never from stored text.
        let folder = library.roots().meeting_audio_dir(&meeting);
        let mut duration_us: Option<i64> = None;
        let results = found
            .tracks
            .iter()
            .map(|track| {
                let id = track.result.id;
                let files = TrackFiles::in_dir(&folder, &id.to_string());
                match skribe_audio::recover(&files) {
                    Ok(recovered) => {
                        duration_us = duration_us.max(recovered.track.end_us);
                        if recovered.was_finalised {
                            // Only the library update was missed; the track itself is whole.
                            let dropped = recovered.track.dropped_frames > 0;
                            valid_result(
                                id,
                                &recovered.track,
                                TrackFinalisation::Finalised,
                                if dropped {
                                    TrackHealth::Degraded
                                } else {
                                    TrackHealth::Healthy
                                },
                                dropped.then_some(problem::FRAMES_DROPPED),
                                None,
                            )
                        } else {
                            valid_result(
                                id,
                                &recovered.track,
                                TrackFinalisation::Recovered,
                                TrackHealth::Degraded,
                                Some(problem::INTERRUPTED),
                                None,
                            )
                        }
                    }
                    Err(error) => {
                        failed_result(id, problem::RECOVERY_FAILED, error.to_string(), None)
                    }
                }
            })
            .collect();
        stored(library.finish_recording(meeting, duration_us, results)).map(detail)
    }
}

/// `recording_preflight`
pub fn preflight_command(
    service: &RecordingService,
    args: &MeetingIdArgs,
) -> Result<PreflightReport, CommandError> {
    service.preflight(meeting_id(&args.meeting_id)?)
}

/// `recording_start`
pub fn start(
    service: &RecordingService,
    args: &MeetingIdArgs,
) -> Result<RecordingStatus, CommandError> {
    service.start(meeting_id(&args.meeting_id)?)
}

/// `recording_status`
pub fn status(service: &RecordingService) -> Result<RecordingStatus, CommandError> {
    Ok(service.status())
}

/// `recording_stop`
pub fn stop(
    service: &RecordingService,
    args: &MeetingIdArgs,
) -> Result<MeetingDetail, CommandError> {
    service.stop(meeting_id(&args.meeting_id)?)
}

/// `recording_recover`
pub fn recover(
    service: &RecordingService,
    args: &MeetingIdArgs,
) -> Result<MeetingDetail, CommandError> {
    service.recover(meeting_id(&args.meeting_id)?)
}

mod devices;
pub use devices::{
    AvailableDevice, DeviceChoice, DeviceChoiceArg, RecordingDevices, SetRecordingDevicesArgs,
    SourceDevices, devices, set_devices,
};

#[cfg(test)]
mod tests;
