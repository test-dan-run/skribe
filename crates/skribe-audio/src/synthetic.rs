//! A capture backend that generates its own audio, for tests and development builds.
//!
//! It is compiled only for this crate's tests or with the `synthetic` feature, which no
//! release build enables. Its devices name themselves as generated test signals so that
//! nothing it records can be mistaken for a meeting.
//!
//! [`SyntheticBackend::scripted`] delivers exactly the packets a test asks for, with the
//! timestamps the test chooses. [`SyntheticBackend::tone`] runs in real time for a
//! development build: a steady tone on the microphone track and an intermittent one on the
//! system track, which pauses the way loopback does when nothing is played.

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::capture::{
    BackendInfo, CaptureBackend, CaptureSink, CaptureStream, DeviceChoice, DeviceInfo,
    ListedDevice, choose,
};
use crate::{AudioFormat, CaptureError, CaptureErrorKind, SampleKind, Source};

/// The synthetic microphone: 48 kHz mono float.
pub const MICROPHONE_FORMAT: AudioFormat = AudioFormat {
    sample_rate: 48_000,
    channels: 1,
    bits_per_sample: 32,
    valid_bits: 32,
    kind: SampleKind::Float,
    channel_mask: 0x4,
};

/// The synthetic system track: 48 kHz stereo 16-bit, so the two tracks differ in format as
/// real devices usually do.
pub const SYSTEM_FORMAT: AudioFormat = AudioFormat {
    sample_rate: 48_000,
    channels: 2,
    bits_per_sample: 16,
    valid_bits: 16,
    kind: SampleKind::Int,
    channel_mask: 0x3,
};

fn format_of(source: Source) -> AudioFormat {
    match source {
        Source::Microphone => MICROPHONE_FORMAT,
        Source::System => SYSTEM_FORMAT,
    }
}

/// The devices a synthetic backend lists when a test has not said otherwise: two per source,
/// the first the default and the second the default communications device, so choosing
/// between them can be exercised.
pub fn standard_devices(source: Source) -> Vec<ListedDevice> {
    let second = DeviceInfo {
        id: format!("synthetic:{source}:second"),
        name: match source {
            Source::Microphone => "Second generated test tone (not a microphone)".to_owned(),
            Source::System => "Second generated test tone (not system audio)".to_owned(),
        },
    };
    vec![
        ListedDevice {
            device: device_of(source),
            is_default: true,
            is_communications_default: false,
        },
        ListedDevice {
            device: second,
            is_default: false,
            is_communications_default: true,
        },
    ]
}

fn device_of(source: Source) -> DeviceInfo {
    DeviceInfo {
        id: format!("synthetic:{source}"),
        name: match source {
            Source::Microphone => "Generated test tone (not a microphone)".to_owned(),
            Source::System => "Generated test tone (not system audio)".to_owned(),
        },
    }
}

/// The bytes of `frames` frames of the synthetic signal for `source`, starting at
/// `first_frame`: a quiet 440 Hz tone, the same on every channel. Tests compare recorded
/// files against it.
pub fn signal(source: Source, first_frame: u64, frames: u32) -> Vec<u8> {
    let format = format_of(source);
    let mut out = Vec::with_capacity(frames as usize * format.bytes_per_frame() as usize);
    for index in 0..u64::from(frames) {
        let position = (first_frame + index) % u64::from(format.sample_rate);
        let phase = position as f32 / format.sample_rate as f32;
        let value = 0.25 * (TAU * 440.0 * phase).sin();
        for _ in 0..format.channels {
            match format.kind {
                SampleKind::Float => out.extend_from_slice(&value.to_le_bytes()),
                SampleKind::Int => {
                    out.extend_from_slice(&((value * 32_767.0) as i16).to_le_bytes());
                }
            }
        }
    }
    out
}

#[derive(Default)]
struct ScriptState {
    sinks: HashMap<Source, CaptureSink>,
    /// Frames delivered so far per source, so the signal continues where it left off.
    delivered: HashMap<Source, u64>,
    open_failures: HashMap<Source, (CaptureErrorKind, String)>,
    start_failures: HashMap<Source, String>,
    formats: HashMap<Source, AudioFormat>,
    /// The devices listed per source, when a test has replaced the standard ones.
    devices: HashMap<Source, Vec<ListedDevice>>,
}

impl ScriptState {
    fn listed(&self, source: Source) -> Vec<ListedDevice> {
        self.devices
            .get(&source)
            .cloned()
            .unwrap_or_else(|| standard_devices(source))
    }
}

#[derive(Default)]
struct Shared {
    state: Mutex<ScriptState>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, ScriptState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A test's control over the scripted backend's devices.
#[derive(Clone)]
pub struct Script {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Script {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Script").finish_non_exhaustive()
    }
}

impl Script {
    /// Delivers one packet of the synthetic signal stamped `timeline_us`. Returns `false`
    /// when the source is not capturing.
    pub fn deliver(&self, source: Source, timeline_us: i64, frames: u32) -> bool {
        self.deliver_flagged(source, timeline_us, frames, 0)
    }

    pub fn deliver_flagged(
        &self,
        source: Source,
        timeline_us: i64,
        frames: u32,
        flags: u32,
    ) -> bool {
        let mut state = self.shared.lock();
        let first_frame = state.delivered.get(&source).copied().unwrap_or(0);
        let Some(sink) = state.sinks.get(&source) else {
            return false;
        };
        sink.packet(
            timeline_us,
            frames,
            flags,
            &signal(source, first_frame, frames),
        );
        state
            .delivered
            .insert(source, first_frame + u64::from(frames));
        true
    }

    /// Delivers `packets` back-to-back ten-millisecond packets starting at `from_us`, and
    /// returns the timeline time just after the last one.
    pub fn deliver_run(&self, source: Source, from_us: i64, packets: u32) -> i64 {
        let frames = format_of(source).sample_rate / 100;
        let mut at = from_us;
        for _ in 0..packets {
            self.deliver(source, at, frames);
            at += 10_000;
        }
        at
    }

    /// The device goes away, as if unplugged: the track is told and capture ends.
    pub fn lose_device(&self, source: Source, detail: &str) {
        if let Some(sink) = self.shared.lock().sinks.remove(&source) {
            sink.device_lost(detail);
        }
    }

    /// Makes the next attempts to open `source` fail.
    pub fn fail_open(&self, source: Source, kind: CaptureErrorKind, detail: &str) {
        self.shared
            .lock()
            .open_failures
            .insert(source, (kind, detail.to_owned()));
    }

    /// Makes opening `source` succeed again.
    pub fn allow_open(&self, source: Source) {
        self.shared.lock().open_failures.remove(&source);
    }

    /// Makes `source` open but then fail to start.
    pub fn fail_start(&self, source: Source, detail: &str) {
        self.shared
            .lock()
            .start_failures
            .insert(source, detail.to_owned());
    }

    /// Makes `source` report `format` as its native format.
    pub fn set_format(&self, source: Source, format: AudioFormat) {
        self.shared.lock().formats.insert(source, format);
    }

    /// Replaces the devices listed for `source`, as if devices were plugged in or removed.
    pub fn set_devices(&self, source: Source, devices: Vec<ListedDevice>) {
        self.shared.lock().devices.insert(source, devices);
    }

    pub fn is_capturing(&self, source: Source) -> bool {
        self.shared.lock().sinks.contains_key(&source)
    }
}

#[derive(Clone)]
enum Mode {
    Scripted(Arc<Shared>),
    Tone,
}

/// See the module documentation.
#[derive(Clone)]
pub struct SyntheticBackend {
    mode: Mode,
}

impl std::fmt::Debug for SyntheticBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.mode {
            Mode::Scripted(_) => "SyntheticBackend(scripted)",
            Mode::Tone => "SyntheticBackend(tone)",
        })
    }
}

impl SyntheticBackend {
    /// A backend whose devices deliver only what the returned [`Script`] tells them to.
    pub fn scripted() -> (Self, Script) {
        let shared = Arc::new(Shared::default());
        (
            Self {
                mode: Mode::Scripted(Arc::clone(&shared)),
            },
            Script { shared },
        )
    }

    /// A backend that generates tones in real time, for development builds.
    pub fn tone() -> Self {
        Self { mode: Mode::Tone }
    }
}

impl CaptureBackend for SyntheticBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Generated test signal",
            synthetic: true,
        }
    }

    fn devices(&self, source: Source) -> Result<Vec<ListedDevice>, CaptureError> {
        match &self.mode {
            Mode::Scripted(shared) => {
                let state = shared.lock();
                if let Some((kind, detail)) = state.open_failures.get(&source) {
                    return Err(CaptureError::new(source, kind.clone(), detail.clone()));
                }
                Ok(state.listed(source))
            }
            Mode::Tone => Ok(standard_devices(source)),
        }
    }

    fn open(
        &self,
        source: Source,
        choice: &DeviceChoice,
    ) -> Result<Box<dyn CaptureStream>, CaptureError> {
        let listed = self.devices(source)?;
        let device = listed[choose(source, &listed, choice)?].device.clone();
        match &self.mode {
            Mode::Scripted(shared) => {
                let format = shared
                    .lock()
                    .formats
                    .get(&source)
                    .copied()
                    .unwrap_or_else(|| format_of(source));
                Ok(Box::new(ScriptedStream {
                    source,
                    device,
                    format,
                    shared: Arc::clone(shared),
                }))
            }
            Mode::Tone => Ok(Box::new(ToneStream {
                source,
                device,
                stop: Arc::new(AtomicBool::new(false)),
                thread: None,
            })),
        }
    }
}

struct ScriptedStream {
    source: Source,
    device: DeviceInfo,
    format: AudioFormat,
    shared: Arc<Shared>,
}

impl std::fmt::Debug for ScriptedStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ScriptedStream({})", self.source)
    }
}

impl CaptureStream for ScriptedStream {
    fn device(&self) -> &DeviceInfo {
        &self.device
    }

    fn format(&self) -> AudioFormat {
        self.format
    }

    fn start(&mut self, sink: CaptureSink) -> Result<(), CaptureError> {
        let mut state = self.shared.lock();
        if let Some(detail) = state.start_failures.get(&self.source) {
            return Err(CaptureError::new(
                self.source,
                CaptureErrorKind::Device,
                detail.clone(),
            ));
        }
        state.delivered.insert(self.source, 0);
        state.sinks.insert(self.source, sink);
        Ok(())
    }

    fn stop(&mut self) {
        self.shared.lock().sinks.remove(&self.source);
    }
}

impl Drop for ScriptedStream {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Ten-millisecond packets, as shared-mode devices commonly deliver.
const TONE_PACKET: Duration = Duration::from_millis(10);
/// The system tone plays for four seconds, then delivers nothing for two.
const SYSTEM_ON_PACKETS: u64 = 400;
const SYSTEM_CYCLE_PACKETS: u64 = 600;

struct ToneStream {
    source: Source,
    device: DeviceInfo,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for ToneStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ToneStream({})", self.source)
    }
}

impl CaptureStream for ToneStream {
    fn device(&self) -> &DeviceInfo {
        &self.device
    }

    fn format(&self) -> AudioFormat {
        format_of(self.source)
    }

    fn start(&mut self, sink: CaptureSink) -> Result<(), CaptureError> {
        let source = self.source;
        let stop = Arc::clone(&self.stop);
        let frames = format_of(source).sample_rate / 100;
        let thread = thread::Builder::new()
            .name(format!("skribe-tone-{source}"))
            .spawn(move || {
                let clock = sink.clock();
                let began = Instant::now();
                let mut frame = 0u64;
                let mut packet = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    // Pace against the start time so sleep jitter does not accumulate.
                    let due = began + TONE_PACKET * u32::try_from(packet).unwrap_or(u32::MAX);
                    thread::sleep(due.saturating_duration_since(Instant::now()));
                    let playing = source == Source::Microphone
                        || packet % SYSTEM_CYCLE_PACKETS < SYSTEM_ON_PACKETS;
                    if playing {
                        sink.packet(clock.at(due), frames, 0, &signal(source, frame, frames));
                        frame += u64::from(frames);
                    }
                    packet += 1;
                }
            })
            .map_err(|error| {
                CaptureError::new(source, CaptureErrorKind::Device, error.to_string())
            })?;
        self.thread = Some(thread);
        Ok(())
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ToneStream {
    fn drop(&mut self) {
        self.stop();
    }
}
