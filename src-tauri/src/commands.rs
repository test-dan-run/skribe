//! Tauri command adapters. Each moves the call off the async runtime onto a blocking thread
//! (never the UI thread) and delegates to `skribe_ipc`.

use std::sync::Arc;

use skribe_ipc::CommandError;
use skribe_ipc::library::{self, LibraryStatus, RestoreSnapshotArgs};
use skribe_store::LibraryService;
use tauri::State;

use crate::host::LibraryHost;

async fn blocking<T, F>(service: &Arc<LibraryService>, work: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&LibraryService) -> Result<T, CommandError> + Send + 'static,
{
    let service = Arc::clone(service);
    tauri::async_runtime::spawn_blocking(move || work(&service))
        .await
        .map_err(CommandError::internal)?
}

fn setup_failed(error: &CommandError) -> LibraryStatus {
    LibraryStatus::Unavailable {
        error: error.clone(),
        snapshots: Vec::new(),
    }
}

#[tauri::command]
pub(crate) async fn library_status(
    host: State<'_, LibraryHost>,
) -> Result<LibraryStatus, CommandError> {
    match host.inner() {
        LibraryHost::Service(service) => blocking(service, library::status).await,
        LibraryHost::SetupFailed(error) => Ok(setup_failed(error)),
    }
}

#[tauri::command]
pub(crate) async fn library_retry_open(
    host: State<'_, LibraryHost>,
) -> Result<LibraryStatus, CommandError> {
    match host.inner() {
        LibraryHost::Service(service) => blocking(service, library::retry_open).await,
        LibraryHost::SetupFailed(error) => Ok(setup_failed(error)),
    }
}

#[tauri::command]
pub(crate) async fn library_restore_snapshot(
    host: State<'_, LibraryHost>,
    args: RestoreSnapshotArgs,
) -> Result<LibraryStatus, CommandError> {
    match host.inner() {
        LibraryHost::Service(service) => {
            blocking(service, move |service| {
                library::restore_snapshot(service, &args)
            })
            .await
        }
        LibraryHost::SetupFailed(error) => Err(error.clone()),
    }
}
