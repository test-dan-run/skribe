//! The Skribe desktop app: wires Tauri to the services in the library crates.
//!
//! This crate holds no behaviour of its own. Commands validate nothing and decide nothing
//! here; they hand off to `skribe-ipc`, which is tested without a webview.

mod commands;
mod host;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let host = host::LibraryHost::start(app.handle());
            app.manage(host);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::library_status,
            commands::library_retry_open,
            commands::library_restore_snapshot,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Skribe");
}
