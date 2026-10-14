use crate::capture::{BackendInfo, CaptureBackend, CaptureStream, DeviceChoice, ListedDevice};
use crate::{CaptureError, CaptureErrorKind, Source};

/// The backend of a build that cannot capture audio. Recording needs Windows; a build for
/// another platform says so instead of recording something else.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableBackend;

impl CaptureBackend for UnavailableBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "No audio capture on this platform",
            synthetic: false,
        }
    }

    fn devices(&self, source: Source) -> Result<Vec<ListedDevice>, CaptureError> {
        Err(unavailable(source))
    }

    fn open(
        &self,
        source: Source,
        _choice: &DeviceChoice,
    ) -> Result<Box<dyn CaptureStream>, CaptureError> {
        Err(unavailable(source))
    }
}

fn unavailable(source: Source) -> CaptureError {
    CaptureError::new(
        source,
        CaptureErrorKind::Unavailable,
        "this build has no capture backend for this platform",
    )
}
