// Release builds on Windows are GUI programs: no console window opens behind the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    skribe_app::run();
}
