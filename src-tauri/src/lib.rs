//! The Skribe desktop app: wires Tauri to the services in the library crates.
//!
//! This crate holds no behaviour of its own. Commands validate nothing and decide nothing
//! here; they hand off to `skribe-ipc`, which is tested without a webview.

mod commands;
mod host;

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use skribe_ipc::recording::{
    CLOSE_BLOCKED_EVENT, CloseBlocked, RECORDING_STATUS_EVENT, RecordingService, RecordingStatus,
};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};

use crate::host::Host;

#[cfg(all(feature = "synthetic-capture", not(debug_assertions)))]
compile_error!(
    "the synthetic capture backend is for development builds only and must never be part of \
     a release build"
);

/// The label of the app's one window, as in `tauri.conf.json` and the capability file.
const MAIN_WINDOW: &str = "main";
/// How often track levels are read and sent to the window while recording.
const STATUS_INTERVAL: Duration = Duration::from_millis(250);

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let host = Host::start(app.handle());
            if let Some(recording) = host.recording() {
                push_recording_status(app.handle().clone(), Arc::clone(recording));
            }
            app.manage(host);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window ends the process without running destructors, which would
            // cut a recording off unfinalised. So the window stays open while one is live and
            // says why; the recording is stopped, and its tracks finalised, by the user.
            if let WindowEvent::CloseRequested { api, .. } = event {
                let live = window
                    .try_state::<Host>()
                    .and_then(|host| host.recording().map(|recording| recording.is_live()))
                    .unwrap_or(false);
                if live {
                    api.prevent_close();
                    let _ = window.emit_to(
                        window.label(),
                        CLOSE_BLOCKED_EVENT,
                        CloseBlocked::recording(),
                    );
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::library_status,
            commands::library_retry_open,
            commands::library_restore_snapshot,
            commands::meetings_list,
            commands::meeting_create,
            commands::meeting_get,
            commands::meeting_update,
            commands::agenda_add,
            commands::agenda_edit,
            commands::agenda_remove,
            commands::agenda_reorder,
            commands::meeting_trash,
            commands::meeting_restore,
            commands::meeting_delete_permanently,
            commands::settings_get,
            commands::settings_set_section_shortcut,
            commands::recording_preflight,
            commands::recording_start,
            commands::recording_status,
            commands::recording_stop,
            commands::recording_recover,
            commands::recording_devices,
            commands::recording_set_devices,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Skribe");
}

/// Sends the window each change in the recording's status and, while a recording runs, its
/// track levels a few times a second. The same tick saves new track health events.
fn push_recording_status(app: AppHandle, recording: Arc<RecordingService>) {
    // If the thread cannot start, the window still reads the status when it asks for it.
    let _ = thread::Builder::new()
        .name("skribe-recording-status".to_owned())
        .spawn(move || {
            let mut last = RecordingStatus::Idle;
            loop {
                thread::sleep(STATUS_INTERVAL);
                let status = recording.tick();
                let changing = matches!(status, RecordingStatus::Recording { .. });
                if changing || status != last {
                    let _ = app.emit_to(MAIN_WINDOW, RECORDING_STATUS_EVENT, status.clone());
                    last = status;
                }
            }
        });
}
