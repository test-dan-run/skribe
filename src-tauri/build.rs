// The command list is shared with skribe-ipc as source so the two cannot drift.
include!("../crates/skribe-ipc/src/command_names.rs");

fn main() {
    println!("cargo:rerun-if-changed=../crates/skribe-ipc/src/command_names.rs");

    // Declaring the app's commands makes each one require an explicit permission in
    // `capabilities/`, instead of every command being callable from any window.
    let manifest = tauri_build::AppManifest::new().commands(COMMAND_NAMES);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("failed to run tauri-build");
}
