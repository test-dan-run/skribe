use std::io;
use std::path::{Path, PathBuf};

use crate::Source;

/// Why a capture device could not be opened or started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureErrorKind {
    /// This build has no way to capture audio: it is not running on Windows.
    Unavailable,
    /// There is no default device, or no default communications device, for this source.
    NoDevice,
    /// The specific device chosen for this source is not connected or is disabled.
    ChosenDeviceMissing,
    /// Windows refused access, for example through its microphone privacy setting.
    AccessDenied,
    /// The device's native format is not one Skribe can record.
    FormatUnsupported,
    /// The device reported an error.
    Device,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{source_name} capture failed ({kind:?}): {detail}")]
pub struct CaptureError {
    pub source_name: Source,
    pub kind: CaptureErrorKind,
    /// Technical detail such as a device error code. Never audio content.
    pub detail: String,
}

impl CaptureError {
    pub fn new(source: Source, kind: CaptureErrorKind, detail: impl Into<String>) -> Self {
        Self {
            source_name: source,
            kind,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error(transparent)]
    Capture(#[from] CaptureError),

    #[error("could not {action} {}: {source}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("{} is not a usable track journal: {reason}", path.display())]
    JournalInvalid { path: PathBuf, reason: String },

    #[error("{} is not a usable clock map: {reason}", path.display())]
    ClockInvalid { path: PathBuf, reason: String },

    #[error("{} is not a track file Skribe wrote: {reason}", path.display())]
    WavInvalid { path: PathBuf, reason: String },

    #[error(
        "{} has {available_bytes} bytes free but recording needs {required_bytes}",
        path.display()
    )]
    DiskSpaceLow {
        path: PathBuf,
        available_bytes: u64,
        required_bytes: u64,
    },

    #[error("the {0} track's writer stopped unexpectedly")]
    WriterStopped(Source),
}

impl AudioError {
    pub(crate) fn io(action: &'static str, path: &Path) -> impl FnOnce(io::Error) -> Self {
        let path = path.to_owned();
        move |source| Self::Io {
            action,
            path,
            source,
        }
    }
}
