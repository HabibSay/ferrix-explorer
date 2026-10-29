#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Ferrix Explorer — Tranche 1 host.
//!
//! Bootstraps the Slint event loop, obtains the Win32 HWND from the generated window,
//! registers an OLE drop target on it, and persists the window size on close.

#[path = "../../../resources/en-US.rs"]
mod strings;

mod orchestration;

use fx_settings::{WindowSize, load_window_size, save_window_size};
use fx_shell::OleDropRegistration;
use fx_ui::ApplicationWindow;
use slint::ComponentHandle;
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use windows::Win32::Foundation::HWND;

fn main() {
    if let Err(error) = run() {
        let message = format!("Ferrix Explorer failed to start: {error}");
        if let Some(base_dirs) = directories::BaseDirs::new() {
            let log_dir = base_dirs
                .data_local_dir()
                .join("FerrixExplorer")
                .join("logs");
            let _ = std::fs::create_dir_all(&log_dir);
            let _ = std::fs::write(log_dir.join("startup-error.log"), &message);
        }
        eprintln!("{message}");
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let _log_guard = init_logging().map_err(|error| {
        eprintln!("{}{error}", strings::LOGGING_UNAVAILABLE);
        error
    })?;

    tracing::info!("Ferrix Explorer starting");

    let size = load_window_size().unwrap_or_else(|error| {
        tracing::warn!(%error, "Could not load saved window size; using defaults");
        WindowSize::default()
    });

    let window = ApplicationWindow::new()?;

    // Restore saved window size before showing.
    window
        .window()
        .set_size(slint::PhysicalSize::new(size.width as u32, size.height as u32));

    // The window must be shown before we can retrieve the native handle.
    window.window().show()?;

    // Extract the Win32 HWND so we can register the OLE drop target.
    let hwnd = get_hwnd(&window)?;

    // Register OLE + drop target on the Slint window's HWND.
    let mut drop_reg = OleDropRegistration::register(hwnd)?;

    // Persist window size when the user closes the window.
    let window_weak = window.as_weak();
    window.window().on_close_requested(move || {
        if let Some(w) = window_weak.upgrade() {
            let phys = w.window().size();
            let saved = WindowSize {
                width: phys.width as i32,
                height: phys.height as i32,
            };
            if let Err(error) = save_window_size(saved) {
                tracing::warn!(%error, "Could not save window size");
            }
        }
        slint::CloseRequestResponse::HideWindow
    });

    // Hand control to the Slint event loop. Returns when the last window is hidden.
    window.run()?;

    // Revoke OLE drop target before the HWND is destroyed.
    drop_reg.revoke().ok();

    tracing::info!("Ferrix Explorer stopped");
    Ok(())
}

/// Extract the Win32 HWND from the Slint `ApplicationWindow`.
///
/// Requires the `raw-window-handle-06` Slint feature and `raw-window-handle = "0.6"`.
/// The window must have been shown before calling this function.
fn get_hwnd(window: &ApplicationWindow) -> Result<HWND, Box<dyn std::error::Error>> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // `slint::Window::window_handle()` returns a `slint::WindowHandle` (not a Result).
    // That type implements `raw_window_handle::HasWindowHandle`, whose `window_handle()`
    // method returns `Result<raw_window_handle::WindowHandle<'_>, HandleError>`.
    let slint_handle = window.window().window_handle();
    let raw = slint_handle
        .window_handle()
        .map_err(|e| format!("Failed to get raw window handle: {e:?}"))?;

    match raw.as_raw() {
        RawWindowHandle::Win32(h) => {
            let hwnd_isize = h.hwnd.get();
            Ok(HWND(hwnd_isize as *mut _))
        }
        other => Err(format!("Unexpected window handle type: {other:?}").into()),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use fx_a11y::root_accessibility;

    #[test]
    fn native_and_slint_resources_share_the_window_title() {
        let slint = include_str!("../../../resources/en-US.slint");
        assert!(slint.contains(strings::APP_TITLE));
        assert_eq!(
            root_accessibility(strings::APP_TITLE).name,
            strings::APP_TITLE
        );
    }
}

// ── Logging ────────────────────────────────────────────────────────────────────

fn init_logging() -> std::io::Result<WorkerGuard> {
    let base_dirs = directories::BaseDirs::new()
        .ok_or_else(|| std::io::Error::other("LocalAppData could not be resolved"))?;
    let log_dir = base_dirs
        .data_local_dir()
        .join("FerrixExplorer")
        .join("logs");
    std::fs::create_dir_all(&log_dir)?;

    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("ferrix-explorer")
        .filename_suffix("log")
        .max_log_files(8)
        .build(log_dir)
        .map_err(std::io::Error::other)?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .try_init()
        .map_err(std::io::Error::other)?;
    Ok(guard)
}
