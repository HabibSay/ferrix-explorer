#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Ferrix Explorer — Tranche 1 host.
//!
//! Bootstraps the Slint event loop, obtains the Win32 HWND from the generated window,
//! registers an OLE drop target on it, and persists the window size on close.

#[path = "../../../resources/en-US.rs"]
#[allow(dead_code)]
mod strings;

mod activity;
mod conflict_dialog;
mod context_menu;
mod drag_drop;
mod operations;
mod orchestration;
mod platform_ops;
mod properties;
mod shortcuts;
mod thumbnails;

use std::sync::{Arc, Mutex};

use fx_domain::location::Location;
use fx_settings::{WindowSize, load_window_size, save_window_size};
use fx_shell::OleDropRegistration;
use fx_ui::ApplicationWindow;
use orchestration::Orchestrator;
use slint::{ComponentHandle, Model};
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use windows::Win32::Foundation::HWND;

use crate::drag_drop::{DropContext, DropHost, FerrixDropSink, ListGeometry, ResolvedDrop};
use crate::platform_ops::WindowsPlatformOps;

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
    window.window().set_size(slint::PhysicalSize::new(
        size.width as u32,
        size.height as u32,
    ));

    // Determine initial location: user's home directory, falling back to C:\.
    let initial_location = {
        let home = directories::BaseDirs::new()
            .map(|b| b.home_dir().to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from(r"C:\"));
        Location::Local(home)
    };

    // Tranche 3 slice 2: the shared drop state. The UI publishes the list geometry
    // and the rows into it; the OLE callbacks only ever read it, because they run
    // inside the shell's modal drag loop where the window must not be queried.
    let drop_context = Arc::new(Mutex::new(DropContext::default()));

    // The drop channel: the sink hands a resolved drop over without blocking, and
    // the dispatcher task performs the transfer (rule 4).
    let (drop_tx, drop_rx) = tokio::sync::mpsc::unbounded_channel::<ResolvedDrop>();

    // Create the orchestrator and wire all window callbacks.
    let orchestrator = Orchestrator::new(window.as_weak(), initial_location, drop_context.clone());
    let orch = Arc::new(Mutex::new(orchestrator));
    let orch_close = orch.clone();
    {
        let win = &window;

        let orch_back = orch.clone();
        win.on_navigate_back(move || {
            orch_back.lock().unwrap().navigate_back();
        });

        let orch_fwd = orch.clone();
        win.on_navigate_forward(move || {
            orch_fwd.lock().unwrap().navigate_forward();
        });

        let orch_up = orch.clone();
        win.on_navigate_up(move || {
            orch_up.lock().unwrap().navigate_up();
        });

        let orch_refresh = orch.clone();
        win.on_navigate_refresh(move || {
            orch_refresh.lock().unwrap().refresh();
        });

        let orch_addr = orch.clone();
        win.on_address_bar_confirmed(move |text| {
            orch_addr.lock().unwrap().navigate_to_path(&text);
        });

        let orch_act = orch.clone();
        win.on_row_activated(move |id| {
            orch_act.lock().unwrap().on_row_activated(id as u64);
        });

        // Breadcrumb segment click: navigate to the partial path formed by segments 0..=i.
        let win_weak_bc = window.as_weak();
        let orch_bc = orch.clone();
        win.on_breadcrumb_segment_clicked(move |idx| {
            if let Some(win) = win_weak_bc.upgrade() {
                let segments = win.get_breadcrumb_segments();
                let count = (idx + 1).min(segments.row_count() as i32) as usize;
                let mut path = std::path::PathBuf::new();
                for i in 0..count {
                    let seg = segments.row_data(i).unwrap_or_default();
                    path.push(seg.as_str());
                }
                orch_bc.lock().unwrap().navigate_to(Location::Local(path));
            }
        });

        // Subfolder item selected: navigate to the path.
        let orch_sub = orch.clone();
        win.on_subfolder_item_selected(move |item| {
            orch_sub.lock().unwrap().navigate_to_path(&item);
        });

        // Row selection (single, ctrl, shift).
        let orch_sel = orch.clone();
        win.on_row_selected(move |id, ctrl, shift| {
            orch_sel
                .lock()
                .unwrap()
                .on_row_selected(id as u64, ctrl, shift);
        });

        // Column resize callback.
        let orch_resize = orch.clone();
        win.on_column_resize_requested(move |col_index, new_width| {
            orch_resize
                .lock()
                .unwrap()
                .on_column_resize(col_index, new_width);
        });

        // Column reorder callback.
        let orch_reorder = orch.clone();
        win.on_column_reorder_requested(move |from_index, to_index| {
            orch_reorder
                .lock()
                .unwrap()
                .on_column_reorder(from_index, to_index);
        });

        // focus-address-bar callback: handled in Slint (calls focus-input() directly).
        win.on_focus_address_bar(|| {});

        // Tranche 2: keyboard shortcuts → domain action → orchestrator command.
        let orch_keys = orch.clone();
        win.on_shortcut_key(move |text, ctrl, shift, alt| {
            match shortcuts::action_for(&text, ctrl, shift, alt) {
                Some(action) => orch_keys.lock().unwrap().dispatch_action(action),
                None => false,
            }
        });

        // Tranche 2: inline rename commit / dismiss.
        let orch_ren = orch.clone();
        win.on_rename_committed(move |id, name| {
            orch_ren
                .lock()
                .unwrap()
                .on_rename_committed(id as u64, name.to_string());
        });

        let orch_ren_cancel = orch.clone();
        win.on_rename_cancelled(move |_| {
            orch_ren_cancel.lock().unwrap().on_rename_cancelled();
        });

        // Live feedback while the inline rename editor is open.
        let orch_ren_validate = orch.clone();
        win.on_rename_validate(move |name| {
            orch_ren_validate
                .lock()
                .unwrap()
                .on_rename_validate(name.to_string())
                .into()
        });

        // Tranche 2: permanent-delete confirmation gate.
        let orch_del_yes = orch.clone();
        win.on_permanent_delete_confirmed(move || {
            orch_del_yes.lock().unwrap().confirm_permanent_delete();
        });

        let orch_del_no = orch.clone();
        win.on_permanent_delete_cancelled(move || {
            orch_del_no.lock().unwrap().cancel_permanent_delete();
        });

        // Tranche 2: conflict dialog answer → the dispatcher task holding the
        // paused worker's answer channel.
        let orch_conflict = orch.clone();
        win.on_conflict_resolved(move |choice, apply_to_all| {
            orch_conflict
                .lock()
                .unwrap()
                .submit_conflict_answer(choice.to_string(), apply_to_all);
        });

        // Tranche 2: activity center buttons (per row index).
        let orch_apause = orch.clone();
        win.on_activity_pause_op(move |i| {
            orch_apause
                .lock()
                .unwrap()
                .activity_pause(i.max(0) as usize);
        });

        let orch_aresume = orch.clone();
        win.on_activity_resume_op(move |i| {
            orch_aresume
                .lock()
                .unwrap()
                .activity_resume(i.max(0) as usize);
        });

        let orch_acancel = orch.clone();
        win.on_activity_cancel_op(move |i| {
            orch_acancel
                .lock()
                .unwrap()
                .activity_cancel(i.max(0) as usize);
        });

        let orch_aretry = orch.clone();
        win.on_activity_retry_op(move |i| {
            orch_aretry
                .lock()
                .unwrap()
                .activity_retry(i.max(0) as usize);
        });

        // Tranche 3: right-click over a row → the native Windows menu (Shell verbs plus
        // the Ferrix section), falling back to the minimal Ferrix menu.
        let orch_menu = orch.clone();
        win.on_context_menu_requested(move |id, _x, _y| {
            let row = u64::try_from(id).ok();
            orch_menu.lock().unwrap().on_context_menu(row);
        });

        // Tranche 3 slice 2: a row drag runs an OLE drag source. `DoDragDrop` is a
        // modal pump on the UI thread — the same shape Explorer's own drag has
        // (spec §6). The payload is built before the drag starts, so the
        // orchestrator's lock is not held across the modal loop.
        let orch_drag = orch.clone();
        win.on_row_drag_started(move |id| {
            let request = {
                let orch = orch_drag.lock().unwrap();
                orch.drag_request(id as u64)
            };
            let Some(request) = request else {
                return;
            };
            match fx_shell::run_drag(&request) {
                Ok(fx_shell::DragOutcome::Dropped(effect)) => {
                    tracing::debug!(?effect, files = request.paths.len(), "OLE drag finished");
                }
                Ok(fx_shell::DragOutcome::Cancelled) => {
                    tracing::debug!("OLE drag cancelled");
                }
                Err(err) => tracing::warn!(%err, "OLE drag failed"),
            }
        });

        // Tranche 3 slice 2: the list publishes the geometry the OLE drop hit-test
        // needs. The shell reads it from the shared context; it never asks the
        // window from inside its own drag loop.
        let drop_context_geometry = drop_context.clone();
        let window_weak_geometry = window.as_weak();
        win.on_list_geometry_changed(move |top, height, row_height, scroll| {
            let scale_factor = window_weak_geometry
                .upgrade()
                .map(|win| win.window().scale_factor())
                .unwrap_or(1.0);
            let geometry = ListGeometry {
                viewport_top: top,
                viewport_height: height,
                row_height,
                scroll_offset: scroll,
                row_count: 0,
            };
            if let Ok(mut context) = drop_context_geometry.lock() {
                context.set_geometry(geometry, scale_factor);
            }
        });

        // Tranche 2: selection by pattern (OPS-11).
        let orch_pat = orch.clone();
        win.on_pattern_submitted(move |pattern| {
            orch_pat
                .lock()
                .unwrap()
                .on_pattern_submitted(pattern.to_string());
        });

        // Tranche 2: cleaner confirmation answer (OPS-13).
        let orch_clean = orch.clone();
        win.on_empty_clean_answered(move |accepted| {
            orch_clean.lock().unwrap().submit_clean_answer(accepted);
        });
    }

    // Start the event dispatcher now that the orchestrator is addressable.
    Orchestrator::spawn_event_dispatch(orch.clone());

    // Tranche 3 slice 2: the drop dispatcher. A resolved drop is produced inside the
    // shell's modal drag loop, so the work is handed to a task of its own rather
    // than taken on the orchestrator's lock there (rule 4).
    Orchestrator::spawn_drop_dispatch(orch.clone(), drop_rx);

    // Tranche 3 slice 4: thumbnails arrive on the blocking pool and are applied here.
    Orchestrator::spawn_thumbnail_dispatch(orch.clone());

    // The window can move to a monitor with another scale factor, and the list
    // geometry does not change when it does — yet the shell hands the drop point in
    // physical pixels. The factor is therefore refreshed while the app runs.
    let drop_context_scale = drop_context.clone();
    let window_weak_scale = window.as_weak();
    let scale_timer = slint::Timer::default();
    scale_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_secs(1),
        move || {
            let Some(win) = window_weak_scale.upgrade() else {
                return;
            };
            let scale = win.window().scale_factor();
            if let Ok(mut context) = drop_context_scale.lock() {
                context.set_scale_factor(scale);
            }
        },
    );

    // Persist window size when the user closes the window.
    let window_weak = window.as_weak();
    window.window().on_close_requested(move || {
        // Release any pending conflict first: answered-cancel lets the paused worker
        // finish before the runtime is dropped.
        orch_close.lock().unwrap().close_conflict_channel();
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

    // The OLE drop registration needs the HWND, which Slint only materialises after
    // the first event-loop tick.  We schedule it via a single-shot Timer that fires
    // on the UI/STA thread, where both OleInitialize and RegisterDragDrop must run.
    // The registration is kept alive for the run loop lifetime in a thread_local so
    // it is dropped on the same STA thread without crossing any Send boundary.
    //
    // SAFETY: Timer callbacks run on the Slint UI thread (the STA thread on Windows).
    // OleDropRegistration is !Send and is never moved off that thread.
    thread_local! {
        static DROP_REG: std::cell::RefCell<Option<OleDropRegistration>> =
            const { std::cell::RefCell::new(None) };
    }

    let window_weak_ole = window.as_weak();
    let drop_context_ole = drop_context.clone();
    let ole_timer = slint::Timer::default();
    ole_timer.start(
        slint::TimerMode::SingleShot,
        std::time::Duration::ZERO,
        move || {
            let Some(win) = window_weak_ole.upgrade() else {
                return;
            };
            // Tranche 3 slice 2: the drop target is the application sink, which owns
            // the policy (modifiers, volume, no-op rule) and hands the work over.
            let host = WindowDropHost {
                sender: drop_tx.clone(),
                window: win.as_weak(),
            };
            let sink = FerrixDropSink::new(
                drop_context_ole.clone(),
                Arc::new(WindowsPlatformOps),
                Arc::new(host),
                crate::drag_drop::shell_modifiers,
            );
            match get_hwnd(&win) {
                Ok(hwnd) => match OleDropRegistration::register(hwnd, Arc::new(sink)) {
                    Ok(reg) => {
                        DROP_REG.with(|cell| *cell.borrow_mut() = Some(reg));
                        tracing::info!("OLE drop target registered");
                    }
                    Err(err) => tracing::warn!(%err, "OLE drop target registration failed"),
                },
                Err(err) => tracing::warn!(%err, "Could not obtain HWND for OLE registration"),
            }
        },
    );

    // Show the window and hand control to the Slint event loop.
    window.window().show()?;
    window.run()?;

    // Revoke OLE drop target before the HWND is destroyed (still on the UI thread
    // because run() returns on the UI thread after the event loop exits).
    DROP_REG.with(|cell| {
        if let Some(mut reg) = cell.borrow_mut().take() {
            reg.revoke().ok();
        }
    });

    // ole_timer is dropped here; the single-shot has already fired so this is a no-op.
    drop(ole_timer);

    // The scale-factor timer stops with the window.
    drop(scale_timer);

    tracing::info!("Ferrix Explorer stopped");
    Ok(())
}

/// Extract the Win32 HWND from the Slint `ApplicationWindow`.
///
/// Requires the `raw-window-handle-06` Slint feature and `raw-window-handle = "0.6"`.
/// The window must have been shown before calling this function.
pub(crate) fn get_hwnd(window: &ApplicationWindow) -> Result<HWND, Box<dyn std::error::Error>> {
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

// ── Drop host ──────────────────────────────────────────────────────────────────

/// The application's side of an OLE drop.
///
/// The sink calls this from inside the shell's modal drag loop, so neither method may
/// take the orchestrator's lock: `submit` only pushes the resolved drop onto the
/// unbounded channel the dispatcher task drains, and `announce` only queues a status
/// message for the UI thread.
struct WindowDropHost {
    sender: tokio::sync::mpsc::UnboundedSender<ResolvedDrop>,
    window: slint::Weak<ApplicationWindow>,
}

impl DropHost for WindowDropHost {
    fn submit(&self, drop: &ResolvedDrop) -> bool {
        // An unbounded send never blocks, so the shell gets its answer immediately.
        self.sender.send(drop.clone()).is_ok()
    }

    fn announce(&self, message: &'static str) {
        let window = self.window.clone();
        slint::invoke_from_event_loop(move || {
            if let Some(win) = window.upgrade() {
                win.set_status_message(slint::SharedString::from(message));
            }
        })
        .ok();
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

    /// Tranche 2 guard (spec §10): the dialog and activity sources must carry no literal
    /// user-visible text — every `text:`/`accessible-label:` comes from `resources/en-US.slint`.
    #[test]
    fn tranche_2_dialog_components_take_their_text_from_resources() {
        let dialogs = include_str!("../../fx-ui/src/dialogs.slint");
        let activity = include_str!("../../fx-ui/src/activity.slint");
        for (name, source) in [("dialogs.slint", dialogs), ("activity.slint", activity)] {
            for marker in ["text: \"", "accessible-label: \""] {
                assert!(
                    !source.contains(marker),
                    "{name} still hard-codes text after `{marker}`"
                );
            }
        }
    }

    /// The Rust and Slint resource files must not drift apart for the wording they share.
    #[test]
    fn rust_and_slint_resources_agree_on_the_shared_labels() {
        let slint = include_str!("../../../resources/en-US.slint");
        for value in [
            strings::STATUS_ITEMS_COUNT,
            strings::KIND_FOLDER,
            strings::KIND_FILE,
            strings::KIND_SYMLINK,
            strings::KIND_REPARSE_POINT,
        ] {
            assert!(slint.contains(value), "missing from en-US.slint: {value}");
        }
    }

    /// Tranche 3 guard (spec §11): every new user-visible string lives in
    /// `resources/en-US.rs`, and the native menu module spells none of them out.
    #[test]
    fn tranche_3_native_menu_takes_its_text_from_resources() {
        let source = include_str!("context_menu.rs");
        for label in [
            strings::ACTIVITY_CHECKSUMS,
            strings::MENU_SELECT_BY_PATTERN,
            strings::MENU_INVERT_SELECTION,
            strings::MENU_NEW_FILE,
            strings::ACTIVITY_CLEAN,
            strings::MENU_COPY_PATH,
            strings::MENU_COPY_NAME,
            strings::MENU_PROPERTIES,
            strings::MENU_OPEN_WITH,
            strings::STATUS_MENU_FALLBACK,
            strings::STATUS_MENU_EXTENSION_BLOCKED,
        ] {
            assert!(!label.is_empty(), "empty label in resources/en-US.rs");
            // A hard-coded label would appear as a quoted literal; the module only
            // names the `strings::` constants.
            let literal = format!("\"{label}\"");
            assert!(
                !source.contains(&literal),
                "the native menu hard-codes the label {label}"
            );
        }
    }

    /// Tranche 3 slice 2 — rule 8 evidence: does Ferrix really own the OLE drop
    /// registration on its own window?
    ///
    /// winit 0.30 registers its own `IDropTarget` on every window it creates
    /// (`platform_specific.drag_and_drop` defaults to true) and Slint's winit backend
    /// never relays winit's `WindowEvent::DragEntered`/`DragDropped` to Slint code. If
    /// winit owns the registration, our own `RegisterDragDrop` on the same HWND must
    /// fail, which would make the OLE drop target of slice 2 dead on arrival.
    ///
    /// `OleDropRegistration::register` therefore revokes a foreign registration and
    /// retries. The probe that proves the three states of the window's registration
    /// runs as an example binary, because a native window needs the main thread and the
    /// test harness runs its tests on threads of their own — and because raw Win32 code
    /// belongs in `fx-shell` only (rule 3):
    /// `cargo run -p fx-shell --example ole_registration_probe`.
    #[test]
    fn the_ole_registration_probe_lives_in_an_example_binary() {
        // The probe needs a native window, and a native window needs the main thread.
        // Keeping the reason here stops the next reader from moving it back into a
        // test, where it silently finds no window at all.
        let probe = include_str!("../../fx-shell/examples/ole_registration_probe.rs");
        assert!(
            probe.contains("RegisterDragDrop"),
            "the probe must exercise the drop registration"
        );
        assert!(
            probe.contains("winit"),
            "the probe must name the registration it takes over"
        );
    }
}
