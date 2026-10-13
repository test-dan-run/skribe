/// Every command the frontend may invoke. This file is also included by `src-tauri/build.rs`,
/// which turns each name into a permission, so it must stay a single self-contained item.
pub const COMMAND_NAMES: &[&str] = &[
    "library_status",
    "library_retry_open",
    "library_restore_snapshot",
];
