//! Skribe audio capture.
//!
//! This is the only crate that touches audio devices. It records a chosen microphone and the
//! loopback of a chosen playback device as two separate original tracks on one monotonic timeline, each in its
//! device's native format, writing incrementally with bounded memory and keeping a journal
//! from which an interrupted track can be rebuilt. It knows nothing about meetings or the
//! database: callers give it file paths and receive typed results.
//!
//! The file formats are described in `docs/recording.md`.

#[cfg(all(feature = "synthetic", not(debug_assertions)))]
compile_error!(
    "the synthetic capture backend is a test fixture and must never be compiled into a release \
     build; build without the `synthetic` feature"
);

mod capture;
pub mod clock;
mod error;
mod finalise;
mod format;
pub mod journal;
mod level;
mod live;
pub mod preflight;
mod ring;
mod session;
#[cfg(any(test, feature = "synthetic"))]
pub mod synthetic;
mod unavailable;
#[cfg(windows)]
mod wasapi;
pub mod wav;
mod writer;

pub use capture::{
    BackendInfo, CaptureBackend, CaptureSink, CaptureStream, DeviceChoice, DeviceChoices,
    DeviceInfo, ListedDevice, choose,
};
pub use clock::SessionClock;
pub use error::{AudioError, CaptureError, CaptureErrorKind};
pub use finalise::{FinalisedTrack, RecoveredTrack, recover};
pub use format::{AudioFormat, SampleKind, Source, UnsupportedFormat};
pub use live::{HealthEvent, HealthEventKind, StopReason, TrackStop};
pub use session::{
    OpenDevices, OpenTrack, RecordingSession, SessionOutcome, SessionStatus, TrackOutcome,
    TrackStatus, open_devices,
};
pub use unavailable::UnavailableBackend;
pub use writer::{FLUSH_INTERVAL, TrackFiles};

/// The capture backend for the platform this build runs on: Windows audio on Windows, and
/// everywhere else a backend that says plainly that capture is unavailable.
pub fn platform_backend() -> std::sync::Arc<dyn CaptureBackend> {
    #[cfg(windows)]
    {
        std::sync::Arc::new(wasapi::WasapiBackend)
    }
    #[cfg(not(windows))]
    {
        std::sync::Arc::new(UnavailableBackend)
    }
}

#[cfg(test)]
mod tests;
