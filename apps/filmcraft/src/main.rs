//! FilmCraft desktop binary: a thin wrapper over the app library (the same code Android loads
//! as a cdylib, entering through `android_main` in `src/lib.rs`).
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(target_os = "android"))]
fn main() -> eframe::Result {
    filmcraft::run_desktop()
}

/// The desktop build never packages the binary for Android; this keeps the target linkable
/// if it is ever cross-built without the library form driving it.
#[cfg(target_os = "android")]
fn main() {}
