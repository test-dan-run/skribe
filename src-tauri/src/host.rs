use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use skribe_ipc::CommandError;
use skribe_store::{LibraryService, StorageRoots};
use tauri::{AppHandle, Manager};

/// The library service for this app instance, or why it could not be set up at all.
pub(crate) enum LibraryHost {
    Service(Arc<LibraryService>),
    SetupFailed(CommandError),
}

impl LibraryHost {
    /// Resolves the storage roots and starts opening the library on its own thread, so the
    /// window appears while integrity checks and migrations run.
    pub(crate) fn start(app: &AppHandle) -> Self {
        let service = match build_service(app) {
            Ok(service) => Arc::new(service),
            Err(error) => return Self::SetupFailed(error),
        };
        let opener = Arc::clone(&service);
        // If the thread cannot start, the first command opens the library instead.
        let _ = thread::Builder::new()
            .name("skribe-library-open".to_owned())
            .spawn(move || {
                opener.availability();
            });
        Self::Service(service)
    }
}

fn build_service(app: &AppHandle) -> Result<LibraryService, CommandError> {
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
