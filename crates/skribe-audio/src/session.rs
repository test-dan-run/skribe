//! One recording: both tracks opened together, started against one clock, stopped together
//! and finalised. Each track has its own device, buffer, writer and files, so trouble with
//! one never reaches the other.

use std::fs;
use std::sync::Arc;
use std::thread;

use crate::capture::{CaptureBackend, CaptureSink, CaptureStream, DeviceChoices, DeviceInfo};
use crate::finalise::{self, FinalisedTrack};
use crate::live::{HealthEvent, TrackLive, TrackStop};
use crate::ring;
use crate::writer::Writer;
use crate::{
    AudioError, AudioFormat, CaptureError, CaptureErrorKind, SessionClock, Source, TrackFiles,
};

/// Seconds of audio a track's capture buffer holds before packets are dropped.
const BUFFER_SECONDS: u64 = 2;
/// Packets a track's capture buffer holds; at ten milliseconds each, well over the above.
const BUFFER_PACKETS: usize = 1_024;

/// A device opened and validated for one source, not yet capturing.
pub struct OpenTrack {
    pub source: Source,
    pub device: DeviceInfo,
    pub format: AudioFormat,
    stream: Box<dyn CaptureStream>,
}

impl std::fmt::Debug for OpenTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenTrack")
            .field("source", &self.source)
            .field("device", &self.device)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

/// Both devices, opened. Dropping it closes them without recording anything.
#[derive(Debug)]
pub struct OpenDevices {
    tracks: Vec<OpenTrack>,
}

impl OpenDevices {
    /// The microphone and the system track, in that order.
    pub fn tracks(&self) -> &[OpenTrack] {
        &self.tracks
    }

    pub fn formats(&self) -> Vec<AudioFormat> {
        self.tracks.iter().map(|track| track.format).collect()
    }
}

/// The device half of preflight: opens the chosen microphone and the chosen playback device
/// for loopback, and checks each one's native format can be recorded. Fails naming the source
/// and the reason; when it fails, neither device stays open.
pub fn open_devices(
    backend: &dyn CaptureBackend,
    choices: &DeviceChoices,
) -> Result<OpenDevices, CaptureError> {
    let mut tracks = Vec::with_capacity(Source::BOTH.len());
    for source in Source::BOTH {
        let stream = backend.open(source, choices.of(source))?;
        let format = stream.format();
        format.validate().map_err(|unsupported| {
            CaptureError::new(
                source,
                CaptureErrorKind::FormatUnsupported,
                unsupported.to_string(),
            )
        })?;
        tracks.push(OpenTrack {
            source,
            device: stream.device().clone(),
            format,
            stream,
        });
    }
    Ok(OpenDevices { tracks })
}

struct ActiveTrack {
    source: Source,
    device: DeviceInfo,
    format: AudioFormat,
    files: TrackFiles,
    stream: Box<dyn CaptureStream>,
    writer: Writer,
    live: Arc<TrackLive>,
}

/// How a track is doing right now.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackStatus {
    pub source: Source,
    /// Frames the device has delivered that were accepted for writing.
    pub frames: u64,
    /// Frames dropped because the capture buffer was full.
    pub dropped_frames: u64,
    /// Loudest level since the last status, from 0.0 to 1.0.
    pub peak: f32,
    /// Set once the track has stopped before the recording: its device was lost, or it could
    /// not be captured or written. The other track carries on.
    pub stopped: Option<TrackStop>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionStatus {
    /// Time since the recording's origin.
    pub elapsed_us: i64,
    pub tracks: Vec<TrackStatus>,
}

/// How one track ended.
#[derive(Debug)]
pub struct TrackOutcome {
    pub source: Source,
    pub device: DeviceInfo,
    pub format: AudioFormat,
    pub files: TrackFiles,
    /// The finalised file, or why there is no valid one.
    pub finalised: Result<FinalisedTrack, AudioError>,
    /// Why the track stopped before the recording did, if it did. A track can have stopped
    /// early and still have finalised a valid partial file.
    pub stopped: Option<TrackStop>,
    /// Health events not yet collected with [`RecordingSession::take_events`].
    pub events: Vec<HealthEvent>,
    /// Health events that could not be kept because too many arrived at once.
    pub events_discarded: u64,
}

#[derive(Debug)]
pub struct SessionOutcome {
    /// The recording's length on its own clock, from origin to stop.
    pub duration_us: i64,
    pub tracks: Vec<TrackOutcome>,
}

/// A recording in progress.
pub struct RecordingSession {
    clock: SessionClock,
    tracks: Vec<ActiveTrack>,
}

impl std::fmt::Debug for RecordingSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingSession")
            .field("clock", &self.clock)
            .field("tracks", &self.tracks.len())
            .finish()
    }
}

impl RecordingSession {
    /// Creates each track's files and starts both devices against one clock.
    ///
    /// If any step fails nothing has been recorded: every device is stopped, the files this
    /// call created are removed, and the error says which step failed.
    pub fn start(
        devices: OpenDevices,
        files_for: impl Fn(Source) -> TrackFiles,
    ) -> Result<Self, AudioError> {
        let clock = SessionClock::start();
        let mut session = Self {
            clock,
            tracks: Vec::with_capacity(devices.tracks.len()),
        };
        let mut sinks = Vec::with_capacity(devices.tracks.len());
        let mut failure = None;

        for track in devices.tracks {
            let files = files_for(track.source);
            let live = Arc::new(TrackLive::default());
            let capacity = usize::try_from(track.format.bytes_per_second() * BUFFER_SECONDS)
                .unwrap_or(usize::MAX);
            let (producer, consumer) = ring::ring(capacity, BUFFER_PACKETS);
            match Writer::start(
                track.source,
                &files,
                track.format,
                clock,
                consumer,
                Arc::clone(&live),
            ) {
                Ok(writer) => {
                    sinks.push(CaptureSink::new(
                        track.source,
                        track.format,
                        clock,
                        producer,
                        Arc::clone(&live),
                    ));
                    session.tracks.push(ActiveTrack {
                        source: track.source,
                        device: track.device,
                        format: track.format,
                        files,
                        stream: track.stream,
                        writer,
                        live,
                    });
                }
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }

        if failure.is_none() {
            // Both writers are ready, so the two devices start as close together as possible.
            for (track, sink) in session.tracks.iter_mut().zip(sinks.drain(..)) {
                if let Err(error) = track.stream.start(sink) {
                    failure = Some(AudioError::Capture(error));
                    break;
                }
            }
        }

        match failure {
            None => Ok(session),
            Some(error) => {
                drop(sinks);
                session.abandon();
                Err(error)
            }
        }
    }

    /// Stops everything and removes the files of a recording that never started.
    fn abandon(mut self) {
        for mut track in self.tracks.drain(..) {
            track.stream.stop();
            drop(track.stream);
            let _ = track.writer.finish(track.source);
            for file in track.files.all() {
                let _ = fs::remove_file(file);
            }
        }
    }

    /// The recording's clock. Notes and section transitions are stamped against it.
    pub fn clock(&self) -> SessionClock {
        self.clock
    }

    /// Each track's device and format, in the order the tracks are reported.
    pub fn tracks(&self) -> impl Iterator<Item = (Source, &DeviceInfo, &AudioFormat)> {
        self.tracks
            .iter()
            .map(|track| (track.source, &track.device, &track.format))
    }

    pub fn status(&self) -> SessionStatus {
        SessionStatus {
            elapsed_us: self.clock.now_us(),
            tracks: self
                .tracks
                .iter()
                .map(|track| TrackStatus {
                    source: track.source,
                    frames: track.live.frames(),
                    dropped_frames: track.live.dropped_frames(),
                    peak: track.live.take_peak(),
                    stopped: track.live.stopped(),
                })
                .collect(),
        }
    }

    /// Health events since the last call, for saving while the recording runs.
    pub fn take_events(&self) -> Vec<(Source, HealthEvent)> {
        self.tracks
            .iter()
            .flat_map(|track| {
                track
                    .live
                    .take_events()
                    .into_iter()
                    .map(|event| (track.source, event))
            })
            .collect()
    }

    /// Stops both devices together, lets each writer make the last of its audio durable, and
    /// finalises each track: true header, flush, SHA-256. Blocks until all of that is done.
    /// Each track's result is reported separately; one failing does not fail the other.
    pub fn stop(mut self) -> SessionOutcome {
        thread::scope(|scope| {
            for track in &mut self.tracks {
                let stream = &mut track.stream;
                scope.spawn(move || stream.stop());
            }
        });
        let duration_us = self.clock.now_us();

        let tracks = std::mem::take(&mut self.tracks)
            .into_iter()
            .map(|track| {
                // The stream owns the sink; dropping it closes the buffer for the writer.
                drop(track.stream);
                let end = track.writer.finish(track.source);
                let finalised = match end.error {
                    // The writer died without saying what it had made durable, so ask the
                    // journal, exactly as recovery after a crash would.
                    Some(AudioError::WriterStopped(_)) => {
                        finalise::recover(&track.files).map(|recovered| recovered.track)
                    }
                    // After a write failure, what was durable before it is still a valid
                    // partial track.
                    _ => finalise::finalise(&track.files, &track.format, &end.committed),
                };
                TrackOutcome {
                    source: track.source,
                    device: track.device,
                    format: track.format,
                    files: track.files,
                    finalised,
                    stopped: track.live.stopped(),
                    events: track.live.take_events(),
                    events_discarded: track.live.events_discarded(),
                }
            })
            .collect();
        SessionOutcome {
            duration_us,
            tracks,
        }
    }
}

impl Drop for RecordingSession {
    /// A session dropped without [`RecordingSession::stop`] still stops its devices and lets
    /// each writer make what it holds durable, so the journals describe everything captured.
    /// The tracks are left unfinalised, exactly as an interruption leaves them, and are
    /// completed by [`crate::recover`].
    fn drop(&mut self) {
        for mut track in self.tracks.drain(..) {
            track.stream.stop();
            drop(track.stream);
            let _ = track.writer.finish(track.source);
        }
    }
}
