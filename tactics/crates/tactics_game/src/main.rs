//! Desktop (Windows / Linux / macOS) and Web entry point.
//!
//! All real setup lives in the library so the Android `cdylib` shares it.

// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    tactics_game::run();
}
