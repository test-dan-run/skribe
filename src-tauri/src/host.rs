use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use skribe_audio::CaptureBackend;
use skribe_ipc::CommandError;
use skribe_ipc::recording::RecordingService;
use skribe_store::{LibraryService, StorageRoots};
use tauri::{AppHandle, Manager};

/// The services of this app instance.
pub(crate) struct Services {
    pub(crate) library: Arc<LibraryService>,
    pub(crate) recording: Arc<RecordingService>,
}

/// The app's services, or why they could not be set up at all.
pub(crate) enum Host {
    Ready(Arc<Services>),
    SetupFailed(CommandError),
}

impl Host {
    /// Resolves the storage roots and starts opening the library on its own thread, so the
    /// window appears while integrity checks and migrations run.
    pub(crate) fn start(app: &AppHandle) -> Self {
        let library = match build_library(app) {
            Ok(service) => Arc::new(service),
            Err(error) => return Self::SetupFailed(error),
        };
        let opener = Arc::clone(&library);
        // If the thread cannot start, the first command opens the library instead.
        let _ = thread::Builder::new()
            .name("skribe-library-open".to_owned())
            .spawn(move || {
                opener.availability();
            });
        let recording = Arc::new(RecordingService::new(
            Arc::clone(&library),
            capture_backend(),
        ));
        Self::Ready(Arc::new(Services { library, recording }))
    }

    pub(crate) fn recording(&self) -> Option<&Arc<RecordingService>> {
        match self {
            Self::Ready(services) => Some(&services.recording),
            Self::SetupFailed(_) => None,
        }
    }
}

/// Real devices, unless this development build was asked for the generated test signal.
fn capture_backend() -> Arc<dyn CaptureBackend> {
    #[cfg(feature = "synthetic-capture")]
    {
        Arc::new(skribe_audio::synthetic::SyntheticBackend::tone())
    }
    #[cfg(not(feature = "synthetic-capture"))]
    {
        skribe_audio::platform_backend()
    }
}

fn build_library(app: &AppHandle) -> Result<LibraryService, CommandError> {
    let base = app
        .path()
        .app_local_data_dir()
        .map_err(CommandError::storage_location_unresolved)?;
    let roots = StorageRoots::under(&base).map_err(|error| CommandError::from(&error))?;
    if let Some(install_dir) = install_dir() {
        roots
            .ensure_outside(&install_dir)
            .map_err(|error| CommandError::from(&error))?;
    }
    Ok(LibraryService::new(
        roots,
        app.package_info().version.to_string(),
    ))
}

fn install_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(PathBuf::from)
}
