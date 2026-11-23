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
use fx_domain::tabs::TabStore;
use fx_settings::sidebar::{PinnedStore, RecentStore};
use fx_settings::{WindowSize, load_window_size, save_window_size};
use fx_shell::OleDropRegistration;
use fx_ui::ApplicationWindow;
use orchestration::Orchestrator;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use windows::Win32::Foundation::HWND;

use crate::drag_drop::{DropContext, DropHost, FerrixDropSink, ListGeometry, ResolvedDrop};
use crate::platform_ops::WindowsPlatformOps;

fn main() {
    // Before any window exists: a DPI-unaware process is laid out at a different width
    // from the window Windows gives it, which leaves the UI short of the right edge.
    fx_shell::enable_per_monitor_dpi_awareness();
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

// ── Sidebar model ────────────────────────────────────────────────────────────
//
// The sidebar is one flat row model. Grouping is expressed by a `show-header` flag on
// the first row of each section, so the section order and the header placement are
// decided here, where they are tested, and the Slint component only paints them.

// Glyph kinds understood by `icons.slint`.
const GLYPH_HOME: i32 = 0;
const GLYPH_FOLDER: i32 = 1;
const GLYPH_DRIVE: i32 = 2;
const GLYPH_NETWORK: i32 = 3;
const GLYPH_RECENT: i32 = 4;

/// One sidebar row, before its section header and current-folder state are known.
fn sidebar_row(id: &str, label: &str, section: &str, glyph: i32) -> fx_ui::SidebarEntry {
    fx_ui::SidebarEntry {
        id: SharedString::from(id),
        label: SharedString::from(label),
        section: SharedString::from(section),
        detail: SharedString::default(),
        glyph,
        unavailable: false,
        gauge_percent: 0,
        show_header: false,
        active: false,
    }
}

/// Appends a row, deciding whether it opens a new section and whether it is the
/// folder the user is looking at.
fn append_sidebar_entry(
    entries: &mut Vec<fx_ui::SidebarEntry>,
    mut entry: fx_ui::SidebarEntry,
    current: Option<&str>,
) {
    entry.show_header = entries
        .last()
        .map(|last| last.section != entry.section)
        .unwrap_or(true);
    entry.active = current.is_some_and(|current| same_path(entry.id.as_str(), current));
    entries.push(entry);
}

/// Whether two spellings name the same folder.
///
/// Windows paths are case-insensitive and accept either separator, and a trailing
/// separator does not change the folder, so `C:\\` and `c:` are the same location.
fn same_path(candidate: &str, current: &str) -> bool {
    fn normalized(value: &str) -> String {
        value
            .trim_end_matches(['\\', '/'])
            .replace('/', "\\")
            .to_ascii_lowercase()
    }
    !candidate.is_empty() && normalized(candidate) == normalized(current)
}

/// A size in GB, one decimal below 100 GB and whole numbers above it.
fn gigabytes(bytes: u64) -> String {
    let gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    if gb >= 100.0 {
        format!("{gb:.0} GB")
    } else {
        format!("{gb:.1} GB")
    }
}

/// The volume caption under a drive name, empty when the space is unknown.
fn volume_detail(free: Option<u64>, total: Option<u64>) -> String {
    match (free, total) {
        (Some(free), Some(total)) => crate::strings::SIDEBAR_VOLUME_DETAIL_FMT
            .replace("{free}", &gigabytes(free))
            .replace("{total}", &gigabytes(total)),
        _ => String::new(),
    }
}

fn build_sidebar_entries(current: Option<&str>) -> Vec<fx_ui::SidebarEntry> {
    let mut entries = Vec::<fx_ui::SidebarEntry>::new();
    let quick = crate::strings::SIDEBAR_SECTION_QUICK_ACCESS;

    if let Some(base_dirs) = directories::BaseDirs::new() {
        let home = base_dirs.home_dir().to_path_buf();
        if home.exists() {
            append_sidebar_entry(
                &mut entries,
                sidebar_row(
                    &home.to_string_lossy(),
                    crate::strings::SIDEBAR_HOME,
                    quick,
                    GLYPH_HOME,
                ),
                current,
            );
        }
    }

    if let Some(user_dirs) = directories::UserDirs::new() {
        let mut push_known_folder = |label: &str, path: Option<&std::path::Path>| {
            if let Some(path) = path
                && path.exists()
            {
                append_sidebar_entry(
                    &mut entries,
                    sidebar_row(&path.to_string_lossy(), label, quick, GLYPH_FOLDER),
                    current,
                );
            }
        };
        push_known_folder(crate::strings::SIDEBAR_DESKTOP, user_dirs.desktop_dir());
        push_known_folder(crate::strings::SIDEBAR_DOCUMENTS, user_dirs.document_dir());
        push_known_folder(crate::strings::SIDEBAR_DOWNLOADS, user_dirs.download_dir());
        push_known_folder(crate::strings::SIDEBAR_PICTURES, user_dirs.picture_dir());
        push_known_folder(crate::strings::SIDEBAR_VIDEOS, user_dirs.video_dir());
        push_known_folder(crate::strings::SIDEBAR_MUSIC, user_dirs.audio_dir());
    }

    let pinned = PinnedStore::load().unwrap_or_else(|error| {
        tracing::warn!(%error, "Could not load pinned sidebar entries");
        PinnedStore::new_empty()
    });
    for path in pinned.locations() {
        if path.exists() {
            let label = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| crate::strings::SIDEBAR_PINNED.to_owned());
            append_sidebar_entry(
                &mut entries,
                sidebar_row(
                    &path.to_string_lossy(),
                    &label,
                    crate::strings::SIDEBAR_PINNED,
                    GLYPH_FOLDER,
                ),
                current,
            );
        }
    }

    let recent = RecentStore::load().unwrap_or_else(|error| {
        tracing::warn!(%error, "Could not load recent sidebar entries");
        RecentStore::new_empty()
    });
    for path in recent.locations() {
        if path.exists() {
            let label = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            append_sidebar_entry(
                &mut entries,
                sidebar_row(
                    &path.to_string_lossy(),
                    &label,
                    crate::strings::SIDEBAR_RECENT,
                    GLYPH_RECENT,
                ),
                current,
            );
        }
    }

    for root in fx_shell::volume::logical_drive_roots() {
        let total = fx_shell::volume::get_total_space(&root).ok();
        let free = fx_shell::volume::get_free_space(&root).ok();
        let used = match (total, free) {
            (Some(total), Some(free)) => Some(total.saturating_sub(free)),
            _ => None,
        };
        let gauge_percent = match (used, total) {
            (Some(used), Some(total)) if total > 0 => used
                .saturating_mul(100)
                .checked_div(total)
                .unwrap_or(0)
                .min(100) as i32,
            _ => 0,
        };
        let unavailable = total.is_none() || free.is_none();
        let mut label = root.to_string_lossy().into_owned();
        if fx_shell::volume::is_dev_drive(&root) {
            label = format!("Dev Drive ({label})");
        }
        let mut row = sidebar_row(
            &label,
            &label,
            crate::strings::SIDEBAR_SECTION_THIS_PC,
            GLYPH_DRIVE,
        );
        row.detail = SharedString::from(volume_detail(free, total));
        row.unavailable = unavailable;
        row.gauge_percent = gauge_percent;
        append_sidebar_entry(&mut entries, row, current);
    }

    entries
}

/// Pushes a new sidebar model only when it actually changed.
///
/// The removable-drive refresh runs on a timer; republishing an identical model every
/// second would reset the sidebar's hover and scroll position under the pointer.
fn set_sidebar_if_changed(window: &ApplicationWindow, entries: Vec<fx_ui::SidebarEntry>) {
    let model = window.get_sidebar_entries();
    let unchanged = model.row_count() == entries.len()
        && entries.iter().enumerate().all(|(index, entry)| {
            model
                .row_data(index)
                .is_some_and(|row| same_sidebar_row(&row, entry))
        });
    if !unchanged {
        window.set_sidebar_entries(ModelRc::new(VecModel::from(entries)));
    }
}

fn same_sidebar_row(left: &fx_ui::SidebarEntry, right: &fx_ui::SidebarEntry) -> bool {
    left.id == right.id
        && left.label == right.label
        && left.section == right.section
        && left.detail == right.detail
        && left.glyph == right.glyph
        && left.unavailable == right.unavailable
        && left.gauge_percent == right.gauge_percent
        && left.show_header == right.show_header
        && left.active == right.active
}

/// The folder the window is showing, as a tab path; the drive root when it has none.
fn current_folder(window: &ApplicationWindow) -> std::path::PathBuf {
    let current = window.get_current_path();
    if current.is_empty() {
        std::path::PathBuf::from(r"C:\")
    } else {
        std::path::PathBuf::from(current.as_str())
    }
}

/// The caption of one tab: the folder name, or the path itself for a drive root.
fn tab_title(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn tab_items(store: &TabStore) -> Vec<fx_ui::TabItem> {
    store
        .paths()
        .iter()
        .enumerate()
        .map(|(index, path)| fx_ui::TabItem {
            id: index as i32,
            title: slint::SharedString::from(tab_title(path)),
            active: index == store.active_index(),
        })
        .collect()
}

/// Store the window size the user left behind. Shared by the native close path and the
/// app's own close button, so a window closed either way reopens the same size.
fn persist_window_size(window: &ApplicationWindow) {
    let physical = window.window().size();
    let saved = WindowSize {
        width: physical.width as i32,
        height: physical.height as i32,
    };
    if let Err(error) = save_window_size(saved) {
        tracing::warn!(%error, "Could not save window size");
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    // A log that cannot be created must not keep the explorer from opening: measured on this
    // host, a launch refused here left the previous window on screen and made the new build
    // look like it had not changed at all.
    let _log_guard = init_logging();

    tracing::info!("Ferrix Explorer starting");

    let size = load_window_size().unwrap_or_else(|error| {
        tracing::warn!(%error, "Could not load saved window size; using defaults");
        WindowSize::default()
    });

    let window = ApplicationWindow::new()?;

    // Restore the saved window size before showing.
    window.window().set_size(slint::PhysicalSize::new(
        size.width as u32,
        size.height as u32,
    ));

    // Determine initial location: user's home directory, falling back to C:\\.
    let initial_location = {
        let home = directories::BaseDirs::new()
            .map(|b| b.home_dir().to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from(r"C:\"));
        Location::Local(home)
    };

    let initial_current = match &initial_location {
        Location::Local(path) => path.to_string_lossy().into_owned(),
        _ => String::new(),
    };
    let sidebar_entries = build_sidebar_entries(Some(initial_current.as_str()));
    window.set_sidebar_entries(ModelRc::new(VecModel::from(sidebar_entries)));

    let discovery_window = window.as_weak();
    std::thread::spawn(move || {
        let mut discovered = fx_network::discover_mapped_drives();
        discovered.extend(fx_network::discover_wsl());
        let _ = slint::invoke_from_event_loop(move || {
            let Some(window) = discovery_window.upgrade() else {
                return;
            };
            let current = window.get_current_path();
            let mut entries = build_sidebar_entries(Some(current.as_str()));
            for location in discovered {
                append_sidebar_entry(
                    &mut entries,
                    sidebar_row(
                        &location.path.to_string_lossy(),
                        &location.label,
                        crate::strings::SIDEBAR_SECTION_NETWORK,
                        GLYPH_NETWORK,
                    ),
                    Some(current.as_str()),
                );
            }
            set_sidebar_if_changed(&window, entries);
        });
    });

    let mut tab_store = TabStore::new_empty();
    let initial_tab_path = match &initial_location {
        Location::Local(path) => path.clone(),
        _ => std::path::PathBuf::from(r"C:\"),
    };
    tab_store.open(initial_tab_path);
    let initial_tab_items = tab_items(&tab_store);
    window.set_tab_items(ModelRc::new(VecModel::from(initial_tab_items)));

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

        let orch_sidebar = orch.clone();
        win.on_sidebar_entry_selected(move |id| {
            orch_sidebar
                .lock()
                .unwrap()
                .navigate_to(Location::Local(std::path::PathBuf::from(id.as_str())));
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
        let tab_store = std::rc::Rc::new(std::cell::RefCell::new(tab_store));

        let tab_store_new = tab_store.clone();
        let win_tab_new = window.as_weak();
        let orch_tab_new = orch.clone();
        win.on_tab_new(move || {
            let Some(win) = win_tab_new.upgrade() else {
                return;
            };
            // A new tab opens where the user already is, exactly like Ctrl+T.
            let path = current_folder(&win);
            tab_store_new.borrow_mut().open(path.clone());
            {
                let tabs = tab_store_new.borrow();
                win.set_tab_items(ModelRc::new(VecModel::from(tab_items(&tabs))));
            }
            orch_tab_new
                .lock()
                .unwrap()
                .navigate_to(Location::Local(path));
        });

        let tab_store_close = tab_store.clone();
        let win_tab_close = window.as_weak();
        let orch_tab_close = orch.clone();
        win.on_tab_close(move |id| {
            let index = id as usize;
            let mut tabs = tab_store_close.borrow_mut();
            tabs.activate(index);
            tabs.close_active();
            if let Some(win) = win_tab_close.upgrade() {
                win.set_tab_items(ModelRc::new(VecModel::from(tab_items(&tabs))));
                if let Some(path) = tabs.active_path() {
                    let path = path.clone();
                    drop(tabs);
                    orch_tab_close
                        .lock()
                        .unwrap()
                        .navigate_to(Location::Local(path));
                }
            }
        });

        let tab_store_select = tab_store.clone();
        let win_tab_select = window.as_weak();
        let orch_tab_select = orch.clone();
        win.on_tab_selected(move |id| {
            let index = id as usize;
            tab_store_select.borrow_mut().activate(index);
            if let Some(win) = win_tab_select.upgrade() {
                let tabs = tab_store_select.borrow();
                win.set_tab_items(ModelRc::new(VecModel::from(tab_items(&tabs))));
                if let Some(path) = tabs.active_path() {
                    let path = path.clone();
                    drop(tabs);
                    orch_tab_select
                        .lock()
                        .unwrap()
                        .navigate_to(Location::Local(path));
                }
            }
        });

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

        // Tranche 2/5: keyboard shortcuts → domain action/tab state → orchestrator command.
        let orch_keys = orch.clone();
        let tab_store_shortcuts = tab_store.clone();
        let win_tab_shortcuts = window.as_weak();
        let orch_tab_shortcuts = orch.clone();
        win.on_shortcut_key(move |text, ctrl, shift, alt| {
            if ctrl && text.eq_ignore_ascii_case("t") {
                let path = win_tab_shortcuts
                    .upgrade()
                    .map(|win| current_folder(&win))
                    .unwrap_or_else(|| std::path::PathBuf::from(r"C:\"));
                tab_store_shortcuts.borrow_mut().open(path.clone());
                if let Some(win) = win_tab_shortcuts.upgrade() {
                    let tabs = tab_store_shortcuts.borrow();
                    win.set_tab_items(ModelRc::new(VecModel::from(tab_items(&tabs))));
                }
                orch_tab_shortcuts
                    .lock()
                    .unwrap()
                    .navigate_to(Location::Local(path));
                return true;
            }
            if ctrl && text.eq_ignore_ascii_case("w") {
                tab_store_shortcuts.borrow_mut().close_active();
                if let Some(win) = win_tab_shortcuts.upgrade() {
                    let tabs = tab_store_shortcuts.borrow();
                    win.set_tab_items(ModelRc::new(VecModel::from(tab_items(&tabs))));
                    if let Some(path) = tabs.active_path() {
                        let path = path.clone();
                        drop(tabs);
                        orch_tab_shortcuts
                            .lock()
                            .unwrap()
                            .navigate_to(Location::Local(path));
                    }
                }
                return true;
            }
            if ctrl && (text == "\t" || text.eq_ignore_ascii_case("tab")) {
                tab_store_shortcuts.borrow_mut().cycle_next();
                if let Some(win) = win_tab_shortcuts.upgrade() {
                    let tabs = tab_store_shortcuts.borrow();
                    win.set_tab_items(ModelRc::new(VecModel::from(tab_items(&tabs))));
                    if let Some(path) = tabs.active_path() {
                        let path = path.clone();
                        drop(tabs);
                        orch_tab_shortcuts
                            .lock()
                            .unwrap()
                            .navigate_to(Location::Local(path));
                    }
                }
                return true;
            }
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
    let last_size = std::rc::Rc::new(std::cell::Cell::new((0i32, 0i32, 0u32, 0u32)));
    let orch_columns = orch.clone();
    let sidebar_window = window.as_weak();
    let sidebar_timer = slint::Timer::default();
    sidebar_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_secs(1),
        move || {
            let Some(window) = sidebar_window.upgrade() else {
                return;
            };
            let current = window.get_current_path();
            let entries = build_sidebar_entries(Some(current.as_str()));
            set_sidebar_if_changed(&window, entries);
        },
    );

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
            // Layout diagnostic: Win32's client size and the size Slint lays out can drift
            // apart, and the UI then stops short of the window's right edge.
            if let Ok(hwnd) = get_hwnd(&win)
                && let Some((client_width, client_height)) = fx_shell::window_client_size(hwnd)
            {
                let slint = win.window().size();
                let observed = (client_width, client_height, slint.width, slint.height);
                if last_size.replace(observed) != observed {
                    // The detail columns are fitted to the window width, so a resize (a
                    // maximise in particular) has to re-publish them.
                    match orch_columns.lock() {
                        Ok(orch) => orch.push_columns_to_window(),
                        Err(err) => tracing::warn!("column refit skipped: {err}"),
                    }
                    tracing::info!(
                        win32_client = ?(client_width, client_height),
                        slint_physical = ?(slint.width, slint.height),
                        slint_logical = ?(
                            slint.width as f32 / scale,
                            slint.height as f32 / scale
                        ),
                        scale,
                        "window size"
                    );
                }
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
            persist_window_size(&w);
        }
        slint::CloseRequestResponse::HideWindow
    });

    // Caption-bar buttons. Hover and clicks stay in Slint (so they behave like every
    // other button); the WM_NCHITTEST chrome keeps the rest of the band draggable.
    let weak = window.as_weak();
    window.on_window_minimize(move || {
        if let Some(w) = weak.upgrade() {
            w.window().set_minimized(true);
        }
    });

    let weak = window.as_weak();
    window.on_window_toggle_maximize(move || {
        if let Some(w) = weak.upgrade() {
            let next = !w.window().is_maximized();
            w.window().set_maximized(next);
            w.set_window_maximized(next);
        }
    });

    let weak = window.as_weak();
    window.on_window_close(move || {
        if let Some(w) = weak.upgrade() {
            persist_window_size(&w);
            if let Err(error) = w.window().hide() {
                tracing::warn!(%error, "Could not hide the window");
            }
        }
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
        /// Owned for the run loop: dropping it would uninstall the window subclass.
        static CHROME: std::cell::RefCell<Option<fx_shell::WindowChrome>> =
            const { std::cell::RefCell::new(None) };
    }

    let window_weak_ole = window.as_weak();
    let drop_context_ole = drop_context.clone();
    // Tranche 3 slice 5: the tick that registers OLE is the first moment the HWND exists,
    // which is when the native transfer dialog can be given an owner.
    //
    // One zero-delay tick is not enough: on this host it measured
    // "Failed to get raw window handle: NotSupported" (log 2026-11-16 17:40), because Slint
    // materialises the native window inside the event loop. The drop target and the dialog
    // owner were silently dead while the app looked healthy. Retry briefly instead, then give
    // up with a warning: a window without a drop target is still usable.
    let orch_owner = orch.clone();
    // `slint::Timer` is not `Clone`, so the callback gets its own handle through an `Rc` to
    // stop itself once the window exists. That leaves one stopped timer alive until the
    // process exits, which is the price of not waking the event loop every 50 ms forever.
    let ole_timer = std::rc::Rc::new(slint::Timer::default());
    let ole_retry = std::rc::Rc::clone(&ole_timer);
    let ole_attempts = std::cell::Cell::new(0u32);
    ole_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(50),
        move || {
            let Some(win) = window_weak_ole.upgrade() else {
                ole_retry.stop();
                return;
            };
            let attempts = ole_attempts.get() + 1;
            ole_attempts.set(attempts);
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
                Ok(hwnd) => {
                    ole_retry.stop();
                    orch_owner
                        .lock()
                        .expect("orchestrator")
                        .set_owner_window(hwnd);
                    match OleDropRegistration::register(hwnd, Arc::new(sink)) {
                        Ok(reg) => {
                            DROP_REG.with(|cell| *cell.borrow_mut() = Some(reg));
                            tracing::info!(attempts, "OLE drop target registered");
                        }
                        Err(err) => tracing::warn!(%err, "OLE drop target registration failed"),
                    }
                }
                Err(err) if attempts >= 100 => {
                    ole_retry.stop();
                    tracing::warn!(
                        %err,
                        attempts,
                        "Could not obtain HWND for OLE registration; drops stay disabled"
                    );
                }
                Err(_) => {}
            }
        },
    );

    // The window is frameless in Slint, so the shell supplies what the native frame used to
    // do: dragging the caption band, the resize edges, double-click to maximise and the
    // maximise state. That needs the HWND, and Slint materialises the native window only on
    // the first event-loop tick (the same reason the OLE registration retries), so retry
    // briefly instead of reading the handle once, too early.
    let chrome_timer = std::rc::Rc::new(slint::Timer::default());
    let chrome_retry = std::rc::Rc::clone(&chrome_timer);
    let chrome_window = window.as_weak();
    let chrome_attempts = std::cell::Cell::new(0u32);
    let saved_width = size.width;
    let saved_height = size.height;
    chrome_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(50),
        move || {
            let Some(win) = chrome_window.upgrade() else {
                return;
            };
            let attempts = chrome_attempts.get() + 1;
            chrome_attempts.set(attempts);
            match get_hwnd(&win) {
                Ok(hwnd) => {
                    chrome_retry.stop();
                    let weak = win.as_weak();
                    match fx_shell::WindowChrome::install(
                        hwnd,
                        fx_shell::ChromeMetrics::default(),
                        move |maximized| {
                            if let Some(w) = weak.upgrade() {
                                w.set_window_maximized(maximized);
                            }
                        },
                    ) {
                        Ok(chrome) => {
                            CHROME.with(|slot| *slot.borrow_mut() = Some(chrome));
                            // Keep the restored window on screen: a size saved on a bigger
                            // monitor would open with the caption buttons past the right and
                            // bottom edges, and a frameless window that cannot be dragged
                            // leaves no way to reach them again.
                            if let Some((work_width, work_height)) = fx_shell::work_area_size(hwnd)
                            {
                                // The size set before `show()` does not survive the window
                                // creation, so restore it here, clamped to the work area.
                                win.window().set_size(slint::PhysicalSize::new(
                                    (saved_width as u32).min(work_width).max(400),
                                    (saved_height as u32).min(work_height).max(300),
                                ));
                            }
                            tracing::info!(
                                attempts,
                                win32_scale = fx_shell::window_scale(hwnd),
                                slint_scale = win.window().scale_factor(),
                                size = ?win.window().size(),
                                "Custom window chrome installed"
                            );
                        }
                        Err(error) => {
                            tracing::warn!(%error, "Custom window chrome unavailable");
                        }
                    }
                }
                Err(error) if attempts >= 100 => {
                    chrome_retry.stop();
                    tracing::warn!(
                        %error,
                        attempts,
                        "Could not obtain HWND for the custom window chrome"
                    );
                }
                Err(_) => {}
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
    drop(chrome_timer);

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

/// Starts logging, preferring the log file and falling back to stderr.
///
/// Returns the guard that flushes the file writer; `None` means "stderr only".
fn init_logging() -> Option<WorkerGuard> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let file = directories::BaseDirs::new()
        .map(|base_dirs| {
            base_dirs
                .data_local_dir()
                .join("FerrixExplorer")
                .join("logs")
        })
        .ok_or_else(|| std::io::Error::other("LocalAppData could not be resolved"))
        .and_then(|log_dir| {
            std::fs::create_dir_all(&log_dir)?;
            RollingFileAppender::builder()
                .rotation(Rotation::DAILY)
                .filename_prefix("ferrix-explorer")
                .filename_suffix("log")
                .max_log_files(8)
                .build(log_dir)
                .map_err(|error| {
                    std::io::Error::other(format!("failed to create log file: {error}"))
                })
        });

    match file {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            match tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(writer)
                .with_ansi(false)
                .try_init()
            {
                Ok(()) => Some(guard),
                Err(error) => {
                    eprintln!("Ferrix Explorer could not install the log subscriber: {error}");
                    None
                }
            }
        }
        Err(error) => {
            eprintln!("{}{error}", strings::LOGGING_UNAVAILABLE);
            let _ = tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(std::io::stderr)
                .try_init();
            None
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use fx_a11y::root_accessibility;

    /// Sidebar grouping: the first row of a section opens it, and only the folder the
    /// user is looking at is marked active.
    #[test]
    fn sidebar_rows_open_a_section_once_and_mark_the_current_folder() {
        let mut entries = Vec::new();
        append_sidebar_entry(
            &mut entries,
            sidebar_row("C:\\Users", "Users", "Quick access", GLYPH_HOME),
            Some("c:\\users"),
        );
        append_sidebar_entry(
            &mut entries,
            sidebar_row("C:\\Work", "Work", "Quick access", GLYPH_FOLDER),
            Some("c:\\users"),
        );
        append_sidebar_entry(
            &mut entries,
            sidebar_row("D:\\", "D:\\", "This PC", GLYPH_DRIVE),
            Some("c:\\users"),
        );

        assert!(entries[0].show_header, "the first section starts here");
        assert!(!entries[1].show_header, "the same section is not reopened");
        assert!(entries[2].show_header, "a new section starts here");
        assert!(entries[0].active, "the home row is the current folder");
        assert!(!entries[1].active);
        assert!(!entries[2].active);
    }

    #[test]
    fn same_path_ignores_case_separators_and_a_trailing_separator() {
        assert!(same_path("C:\\Users\\", "c:/users"));
        assert!(!same_path("C:\\Users", "C:\\Windows"));
        assert!(!same_path("", ""), "an empty location is never current");
    }

    #[test]
    fn volume_detail_names_free_and_total_space() {
        let gb = 1024u64 * 1024 * 1024;
        assert_eq!(
            volume_detail(Some(120 * gb), Some(500 * gb)),
            "120 GB free of 500 GB"
        );
        assert_eq!(
            volume_detail(Some(gb / 2), Some(gb)),
            "0.5 GB free of 1.0 GB"
        );
        assert_eq!(
            volume_detail(None, Some(gb)),
            "",
            "an unknown volume has no caption"
        );
    }

    #[test]
    fn a_tab_is_captioned_by_its_folder_name_and_a_root_by_its_path() {
        assert_eq!(tab_title(std::path::Path::new("C:\\Users\\ana")), "ana");
        assert_eq!(tab_title(std::path::Path::new("C:\\")), "C:\\");
    }

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
