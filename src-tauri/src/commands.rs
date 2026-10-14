//! Tauri command adapters. Each moves the call off the async runtime onto a blocking thread
//! (never the UI thread) and delegates to `skribe_ipc`. Nothing is validated or decided here.

use std::sync::Arc;

use skribe_ipc::library::{LibraryStatus, RestoreSnapshotArgs};
use skribe_ipc::meetings::{
    AgendaAddArgs, AgendaEditArgs, AgendaRemoveArgs, AgendaReorderArgs, MeetingCreateArgs,
    MeetingDetail, MeetingIdArgs, MeetingList, MeetingUpdateArgs,
};
use skribe_ipc::recording::{
    PreflightReport, RecordingDevices, RecordingStatus, SetRecordingDevicesArgs,
};
use skribe_ipc::settings::{SetSectionShortcutArgs, Settings};
use skribe_ipc::{CommandError, library, meetings, recording, settings};
use tauri::State;

use crate::host::{Host, Services};

async fn blocking<T, F>(services: &Arc<Services>, work: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&Services) -> Result<T, CommandError> + Send + 'static,
{
    let services = Arc::clone(services);
    tauri::async_runtime::spawn_blocking(move || work(&services))
        .await
        .map_err(CommandError::internal)?
}

/// Runs `work` against the services, or reports why they could not be set up.
async fn run<T, F>(host: &Host, work: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&Services) -> Result<T, CommandError> + Send + 'static,
{
    match host {
        Host::Ready(services) => blocking(services, work).await,
        Host::SetupFailed(error) => Err(error.clone()),
    }
}

fn setup_failed(error: &CommandError) -> LibraryStatus {
    LibraryStatus::Unavailable {
        error: error.clone(),
        snapshots: Vec::new(),
    }
}

#[tauri::command]
pub(crate) async fn library_status(host: State<'_, Host>) -> Result<LibraryStatus, CommandError> {
    match host.inner() {
        Host::Ready(services) => blocking(services, |s| library::status(&s.library)).await,
        Host::SetupFailed(error) => Ok(setup_failed(error)),
    }
}

#[tauri::command]
pub(crate) async fn library_retry_open(
    host: State<'_, Host>,
) -> Result<LibraryStatus, CommandError> {
    match host.inner() {
        Host::Ready(services) => blocking(services, |s| library::retry_open(&s.library)).await,
        Host::SetupFailed(error) => Ok(setup_failed(error)),
    }
}

#[tauri::command]
pub(crate) async fn library_restore_snapshot(
    host: State<'_, Host>,
    args: RestoreSnapshotArgs,
) -> Result<LibraryStatus, CommandError> {
    run(&host, move |s| library::restore_snapshot(&s.library, &args)).await
}

#[tauri::command]
pub(crate) async fn meetings_list(host: State<'_, Host>) -> Result<MeetingList, CommandError> {
    run(&host, |s| meetings::list(&s.library)).await
}

#[tauri::command]
pub(crate) async fn meeting_create(
    host: State<'_, Host>,
    args: MeetingCreateArgs,
) -> Result<MeetingDetail, CommandError> {
    run(&host, move |s| meetings::create(&s.library, &args)).await
}

#[tauri::command]
pub(crate) async fn meeting_get(
    host: State<'_, Host>,
    args: MeetingIdArgs,
) -> Result<MeetingDetail, CommandError> {
    run(&host, move |s| meetings::get(&s.library, &args)).await
}

#[tauri::command]
pub(crate) async fn meeting_update(
    host: State<'_, Host>,
    args: MeetingUpdateArgs,
) -> Result<MeetingDetail, CommandError> {
    run(&host, move |s| meetings::update(&s.library, &args)).await
}

#[tauri::command]
pub(crate) async fn agenda_add(
    host: State<'_, Host>,
    args: AgendaAddArgs,
) -> Result<MeetingDetail, CommandError> {
    run(&host, move |s| meetings::agenda_add(&s.library, &args)).await
}

#[tauri::command]
pub(crate) async fn agenda_edit(
    host: State<'_, Host>,
    args: AgendaEditArgs,
) -> Result<MeetingDetail, CommandError> {
    run(&host, move |s| meetings::agenda_edit(&s.library, &args)).await
}

#[tauri::command]
pub(crate) async fn agenda_remove(
    host: State<'_, Host>,
    args: AgendaRemoveArgs,
) -> Result<MeetingDetail, CommandError> {
    run(&host, move |s| meetings::agenda_remove(&s.library, &args)).await
}

#[tauri::command]
pub(crate) async fn agenda_reorder(
    host: State<'_, Host>,
    args: AgendaReorderArgs,
) -> Result<MeetingDetail, CommandError> {
    run(&host, move |s| meetings::agenda_reorder(&s.library, &args)).await
}

#[tauri::command]
pub(crate) async fn meeting_trash(
    host: State<'_, Host>,
    args: MeetingIdArgs,
) -> Result<MeetingList, CommandError> {
    run(&host, move |s| meetings::trash(&s.library, &args)).await
}

#[tauri::command]
pub(crate) async fn meeting_restore(
    host: State<'_, Host>,
    args: MeetingIdArgs,
) -> Result<MeetingList, CommandError> {
    run(&host, move |s| meetings::restore(&s.library, &args)).await
}

#[tauri::command]
pub(crate) async fn meeting_delete_permanently(
    host: State<'_, Host>,
    args: MeetingIdArgs,
) -> Result<MeetingList, CommandError> {
    run(&host, move |s| {
        meetings::delete_permanently(&s.library, &args)
    })
    .await
}

#[tauri::command]
pub(crate) async fn settings_get(host: State<'_, Host>) -> Result<Settings, CommandError> {
    run(&host, |s| settings::get(&s.library)).await
}

#[tauri::command]
pub(crate) async fn settings_set_section_shortcut(
    host: State<'_, Host>,
    args: SetSectionShortcutArgs,
) -> Result<Settings, CommandError> {
    run(&host, move |s| {
        settings::set_section_shortcut(&s.library, &args)
    })
    .await
}

#[tauri::command]
pub(crate) async fn recording_preflight(
    host: State<'_, Host>,
    args: MeetingIdArgs,
) -> Result<PreflightReport, CommandError> {
    run(&host, move |s| {
        recording::preflight_command(&s.recording, &args)
    })
    .await
}

#[tauri::command]
pub(crate) async fn recording_start(
    host: State<'_, Host>,
    args: MeetingIdArgs,
) -> Result<RecordingStatus, CommandError> {
    run(&host, move |s| recording::start(&s.recording, &args)).await
}

#[tauri::command]
pub(crate) async fn recording_status(
    host: State<'_, Host>,
) -> Result<RecordingStatus, CommandError> {
    run(&host, |s| recording::status(&s.recording)).await
}

#[tauri::command]
pub(crate) async fn recording_stop(
    host: State<'_, Host>,
    args: MeetingIdArgs,
) -> Result<MeetingDetail, CommandError> {
    run(&host, move |s| recording::stop(&s.recording, &args)).await
}

#[tauri::command]
pub(crate) async fn recording_recover(
    host: State<'_, Host>,
    args: MeetingIdArgs,
) -> Result<MeetingDetail, CommandError> {
    run(&host, move |s| recording::recover(&s.recording, &args)).await
}

#[tauri::command]
pub(crate) async fn recording_devices(
    host: State<'_, Host>,
) -> Result<RecordingDevices, CommandError> {
    run(&host, |s| recording::devices(&s.recording)).await
}

#[tauri::command]
pub(crate) async fn recording_set_devices(
    host: State<'_, Host>,
    args: SetRecordingDevicesArgs,
) -> Result<RecordingDevices, CommandError> {
    run(&host, move |s| recording::set_devices(&s.recording, &args)).await
}
