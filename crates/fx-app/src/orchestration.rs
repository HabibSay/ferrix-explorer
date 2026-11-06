//! Navigation orchestration for Ferrix Explorer Tranche 1 Slice 4.
//!
//! Owns the Tokio runtime, a `CancellationToken` per listing, `NavigationHistory`,
//! `ViewStateStore`, and a `NotifyWatcher` for the current directory.
//! Drives the Slint UI via `invoke_from_event_loop`.

use fx_domain::{history::NavigationHistory, location::Location, view::ColumnLayout};
use fx_fs::listing::{ListingBatch, ListingError, list_directory};
use fx_fs::watch::{LocationWatcher, NotifyWatcher, WatchEvent};
use fx_settings::view_state::ViewStateStore;
use fx_ui::ApplicationWindow;
use fx_ui::viewmodel::FileListViewModel;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, invoke_from_event_loop};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

// ── Orchestrator ──────────────────────────────────────────────────────────────

/// Drives navigation for a single `ApplicationWindow`.
///
/// Constructed on the UI thread; the Tokio runtime is used only for spawning
/// background listing tasks — all Slint mutations go through `invoke_from_event_loop`.
pub struct Orchestrator {
    /// Shared view model; mutated only from the Slint event loop.
    view_model: Arc<Mutex<FileListViewModel>>,
    /// Navigation history.
    history: NavigationHistory,
    /// Per-folder view state store.
    view_state: ViewStateStore,
    /// Tokio runtime for listing tasks.
    runtime: Runtime,
    /// Token for the most-recent listing; cancelled on every new navigation.
    cancel: CancellationToken,
    /// Monotonically increasing generation counter.
    generation: u64,
    /// Weak reference to the Slint window.
    window: slint::Weak<ApplicationWindow>,
    /// Filesystem watcher for the current directory.
    watcher: NotifyWatcher,
    /// Channel receiver for watch events — kept alive so the watcher task can consume it.
    #[allow(dead_code)]
    watch_rx: Option<tokio::sync::mpsc::UnboundedReceiver<WatchEvent>>,
}

impl Orchestrator {
    /// Create a new orchestrator and immediately begin listing `initial_location`.
    pub fn new(window: slint::Weak<ApplicationWindow>, initial_location: Location) -> Self {
        let runtime = Runtime::new().expect("Tokio runtime");
        let view_state = ViewStateStore::load().unwrap_or_else(|err| {
            tracing::warn!(%err, "Could not load view state; using defaults");
            ViewStateStore::new_empty()
        });

        let mut orch = Orchestrator {
            view_model: Arc::new(Mutex::new(FileListViewModel::new())),
            history: NavigationHistory::new(initial_location.clone()),
            view_state,
            runtime,
            cancel: CancellationToken::new(),
            generation: 0,
            window,
            watcher: NotifyWatcher::new(),
            watch_rx: None,
        };

        orch.start_listing(initial_location);
        orch
    }

    // ── Public navigation commands ─────────────────────────────────────────

    /// Navigate to an arbitrary location (e.g. from the address bar).
    pub fn navigate_to(&mut self, location: Location) {
        self.history.navigate(location.clone());
        self.start_listing(location);
    }

    /// Navigate to the address-bar text (parsed as a `Location`).
    pub fn navigate_to_path(&mut self, path: &str) {
        self.navigate_to(Location::parse(path));
    }

    /// Navigate back in history.
    pub fn navigate_back(&mut self) {
        if let Some(state) = self.history.back() {
            let loc = state.location.clone();
            self.start_listing(loc);
        }
    }

    /// Navigate forward in history.
    pub fn navigate_forward(&mut self) {
        if let Some(state) = self.history.forward() {
            let loc = state.location.clone();
            self.start_listing(loc);
        }
    }

    /// Navigate to the parent directory.
    pub fn navigate_up(&mut self) {
        if let Some(parent) = self.history.up() {
            self.navigate_to(parent);
        }
    }

    /// Refresh the current location (re-list without modifying history).
    pub fn refresh(&mut self) {
        let loc = self.history.refresh().location.clone();
        self.start_listing(loc);
    }

    /// Called when the user double-clicks a row with the given entry id.
    ///
    /// - Directory → navigate into it.
    /// - File → `ShellExecuteW` with the default verb.
    pub fn on_row_activated(&mut self, entry_id: u64) {
        let info = {
            let vm = self.view_model.lock().expect("view model lock");
            let path = vm.entry_path(entry_id);
            let kind = vm.entry_kind_str(entry_id);
            path.zip(kind)
        };
        let Some((path, kind)) = info else { return };

        if kind == "Folder" {
            self.navigate_to(Location::Local(path));
        } else {
            self.open_file(path);
        }
    }

    /// Called when the user drags a column resize handle.
    ///
    /// Clamps the new width, updates the view model, persists via `ViewStateStore`,
    /// and pushes updated columns back to the Slint window.
    pub fn on_column_resize(&mut self, col_index: i32, new_width_px: f32) {
        let folder_key = self.history.current().location.to_string();
        {
            let mut vm = self.view_model.lock().expect("view model lock");
            let mut layout = vm.column_layout().clone();
            let idx = col_index as usize;
            if idx < layout.columns.len() {
                let clamped = (new_width_px as u32).clamp(40, 600);
                layout.columns[idx].width_px = clamped;
            }
            vm.set_column_layout(layout.clone());
            let mut state = self.view_state.get(&folder_key);
            state.column_layout = layout;
            self.view_state.set(&folder_key, state);
        }
        if let Err(err) = self.view_state.save() {
            tracing::warn!(%err, "Could not save view state after column resize");
        }
        self.push_columns_to_window();
    }

    /// Called when the user drags a column header to reorder columns.
    ///
    /// Moves the column in the layout, persists via `ViewStateStore`,
    /// and pushes updated columns back to the Slint window.
    pub fn on_column_reorder(&mut self, from_index: i32, to_index: i32) {
        let folder_key = self.history.current().location.to_string();
        {
            let mut vm = self.view_model.lock().expect("view model lock");
            let mut layout = vm.column_layout().clone();
            let from = from_index as usize;
            let to = to_index as usize;
            if from < layout.columns.len() && to < layout.columns.len() && from != to {
                let col = layout.columns.remove(from);
                layout.columns.insert(to, col);
            }
            vm.set_column_layout(layout.clone());
            let mut state = self.view_state.get(&folder_key);
            state.column_layout = layout;
            self.view_state.set(&folder_key, state);
        }
        if let Err(err) = self.view_state.save() {
            tracing::warn!(%err, "Could not save view state after column reorder");
        }
        self.push_columns_to_window();
    }

    /// Called when the user changes the row selection (single, Ctrl, or Shift click).
    pub fn on_row_selected(&mut self, entry_id: u64, ctrl: bool, shift: bool) {
        {
            let mut vm = self.view_model.lock().expect("view model lock");
            if ctrl {
                vm.toggle_select(entry_id);
            } else if shift {
                vm.range_select(entry_id);
            } else {
                vm.select_single(entry_id);
            }
        }
        self.push_rows_to_window();
    }

    // ── Internal ──────────────────────────────────────────────────────────

    /// Open a file with `ShellExecuteW`.
    fn open_file(&self, path: std::path::PathBuf) {
        let hwnd = self.get_hwnd();
        if let Err(err) = fx_shell::execute::shell_open(hwnd, &path) {
            tracing::warn!(%err, path = %path.display(), "ShellExecuteW failed");
        }
    }

    /// Extract the HWND from the window handle (returns NULL on failure).
    fn get_hwnd(&self) -> windows::Win32::Foundation::HWND {
        self.window
            .upgrade()
            .and_then(|win| {
                let slint_handle = win.window().window_handle();
                slint_handle
                    .window_handle()
                    .ok()
                    .and_then(|h| match h.as_raw() {
                        RawWindowHandle::Win32(w) => {
                            Some(windows::Win32::Foundation::HWND(w.hwnd.get() as *mut _))
                        }
                        _ => None,
                    })
            })
            .unwrap_or(windows::Win32::Foundation::HWND(std::ptr::null_mut()))
    }

    /// Cancel any running listing, restore per-folder view state, clear the model,
    /// start a new listing task, and start a watcher for the location.
    fn start_listing(&mut self, location: Location) {
        self.cancel.cancel();
        self.cancel = CancellationToken::new();
        self.generation = self.generation.wrapping_add(1);

        // Stop the previous watcher.
        self.watcher.stop();
        self.watch_rx = None;

        // Restore per-folder view state.
        let folder_key = location.to_string();
        let folder_state = self.view_state.get(&folder_key);
        {
            let mut vm = self.view_model.lock().expect("view model lock");
            vm.clear();
            vm.set_view_mode(folder_state.view_mode);
            vm.set_sort_key(folder_state.sort_key.clone());
            vm.set_column_layout(folder_state.column_layout.clone());
        }

        // Update path, breadcrumb, columns, and rows in the Slint window.
        let path_str = SharedString::from(location.to_string());
        let breadcrumb: Vec<SharedString> = build_breadcrumb_segments(&location);
        let columns = build_column_specs(&folder_state.column_layout, &folder_state.sort_key);
        let window_weak = self.window.clone();
        invoke_from_event_loop(move || {
            if let Some(win) = window_weak.upgrade() {
                win.set_current_path(path_str);
                win.set_breadcrumb_segments(ModelRc::new(VecModel::from(breadcrumb)));
                win.set_rows(ModelRc::new(VecModel::from(Vec::<fx_ui::FileRow>::new())));
                win.set_columns(ModelRc::new(VecModel::from(columns)));
                win.set_status_message(SharedString::from("Loading\u{2026}"));
            }
        })
        .ok();

        // Start filesystem watcher for this location; the watch task owns the receiver.
        // `NotifyWatcher::watch` calls `tokio::spawn` internally, which requires an
        // active Tokio context.  We enter the runtime for the duration of the call so
        // that `tokio::spawn` finds the runtime handle even though we are on the
        // synchronous Slint UI thread.
        let (watch_tx, mut watch_rx) = tokio::sync::mpsc::unbounded_channel::<WatchEvent>();
        {
            let _guard = self.runtime.enter();
            if let Err(err) = self.watcher.watch(&location, watch_tx) {
                tracing::warn!(%err, "Could not start watcher for {location}");
            }
        }

        // Spawn a task that refreshes the listing whenever the watcher fires.
        let vm_arc_watch = self.view_model.clone();
        let window_weak_watch = self.window.clone();
        let location_watch = location.clone();
        let generation_watch = self.generation;
        let cancel_watch = self.cancel.clone();
        self.runtime.spawn(async move {
            while let Some(_event) = watch_rx.recv().await {
                if cancel_watch.is_cancelled() {
                    break;
                }
                // Clear model, then re-list.
                let vm_clear = vm_arc_watch.clone();
                invoke_from_event_loop(move || {
                    vm_clear.lock().expect("vm lock").clear();
                })
                .ok();

                let (tx, mut rx) = tokio::sync::mpsc::channel::<ListingBatch>(8);
                let loc2 = location_watch.clone();
                let tok2 = cancel_watch.clone();
                let list_fut = list_directory(&loc2, generation_watch, tok2, tx);

                let vm_recv = vm_arc_watch.clone();
                let win_recv = window_weak_watch.clone();
                let recv_fut = async move {
                    while let Some(batch) = rx.recv().await {
                        if batch.generation != generation_watch {
                            continue;
                        }
                        let entries = batch.entries.clone();
                        let vm_c = vm_recv.clone();
                        let ww = win_recv.clone();
                        invoke_from_event_loop(move || {
                            let rows: Vec<fx_ui::FileRow> = {
                                let mut vm = vm_c.lock().expect("vm lock");
                                vm.append_batch(entries);
                                vm.to_ui_rows().into_iter().map(ui_row_to_slint).collect()
                            };
                            if let Some(win) = ww.upgrade() {
                                win.set_rows(ModelRc::new(VecModel::from(rows)));
                                let count = vm_c.lock().expect("vm lock").entry_count();
                                win.set_status_message(SharedString::from(format!(
                                    "{count} items"
                                )));
                            }
                        })
                        .ok();
                    }
                };
                let _ = tokio::join!(list_fut, recv_fut);
            }
        });

        // Spawn the primary listing task.
        let token = self.cancel.clone();
        let generation = self.generation;
        let vm_arc = self.view_model.clone();
        let window_weak2 = self.window.clone();

        self.runtime.spawn(async move {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<ListingBatch>(8);

            let list_fut = list_directory(&location, generation, token, tx);

            let recv_fut = async {
                while let Some(batch) = rx.recv().await {
                    if batch.generation != generation {
                        continue;
                    }
                    let entries = batch.entries.clone();
                    let batch_error = batch.error.clone();
                    let vm_clone = vm_arc.clone();
                    let win_weak = window_weak2.clone();

                    invoke_from_event_loop(move || {
                        let rows: Vec<fx_ui::FileRow> = {
                            let mut vm = vm_clone.lock().expect("vm lock");
                            vm.append_batch(entries);
                            vm.to_ui_rows().into_iter().map(ui_row_to_slint).collect()
                        };
                        if let Some(win) = win_weak.upgrade() {
                            win.set_rows(ModelRc::new(VecModel::from(rows)));
                            let count = vm_clone.lock().expect("vm lock").entry_count();
                            win.set_status_message(SharedString::from(format!("{count} items")));
                        }
                        if let Some(err) = batch_error
                            && let Some(win) = win_weak.upgrade()
                        {
                            win.set_status_message(SharedString::from(err.to_string()));
                        }
                    })
                    .ok();
                }
            };

            let (list_result, _) = tokio::join!(list_fut, recv_fut);

            if let Err(err) = list_result
                && !matches!(err, ListingError::Cancelled)
            {
                let msg = err.to_string();
                invoke_from_event_loop(move || {
                    if let Some(win) = window_weak2.upgrade() {
                        win.set_status_message(SharedString::from(msg));
                    }
                })
                .ok();
            }
        });
    }

    /// Push the current column layout from the view model to the Slint window.
    fn push_columns_to_window(&self) {
        let (layout, sort_key) = {
            let vm = self.view_model.lock().expect("view model lock");
            (vm.column_layout().clone(), vm.sort_key().clone())
        };
        let columns = build_column_specs(&layout, &sort_key);
        let window_weak = self.window.clone();
        invoke_from_event_loop(move || {
            if let Some(win) = window_weak.upgrade() {
                win.set_columns(ModelRc::new(VecModel::from(columns)));
            }
        })
        .ok();
    }

    /// Push the current row list from the view model to the Slint window.
    fn push_rows_to_window(&self) {
        let vm = self.view_model.lock().expect("view model lock");
        let rows: Vec<fx_ui::FileRow> = vm.to_ui_rows().into_iter().map(ui_row_to_slint).collect();
        let count = rows.len();
        let window_weak = self.window.clone();
        invoke_from_event_loop(move || {
            if let Some(win) = window_weak.upgrade() {
                win.set_rows(ModelRc::new(VecModel::from(rows)));
                win.set_status_message(SharedString::from(format!("{count} items")));
            }
        })
        .ok();
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn ui_row_to_slint(row: fx_ui::viewmodel::UiFileRow) -> fx_ui::FileRow {
    fx_ui::FileRow {
        id: row.id as i32,
        display_name: SharedString::from(row.display_name),
        kind: SharedString::from(row.kind),
        size_text: SharedString::from(row.size_text),
        modified_text: SharedString::from(row.modified_text),
        type_text: SharedString::from(row.type_text),
        attributes_text: SharedString::default(),
        selected: row.selected,
        icon_key: row.icon_key as i32,
    }
}

fn build_breadcrumb_segments(location: &Location) -> Vec<SharedString> {
    match location {
        Location::Local(path) => {
            let mut segs = Vec::new();
            for component in path.components() {
                use std::path::Component;
                let s = match component {
                    Component::Prefix(p) => p.as_os_str().to_string_lossy().into_owned(),
                    Component::RootDir => continue,
                    Component::Normal(n) => n.to_string_lossy().into_owned(),
                    Component::CurDir | Component::ParentDir => continue,
                };
                segs.push(SharedString::from(s));
            }
            segs
        }
        other => vec![SharedString::from(other.to_string())],
    }
}

/// Build Slint `ColumnSpec` values from a domain `ColumnLayout` + `SortKey`.
fn build_column_specs(
    layout: &ColumnLayout,
    sort_key: &fx_domain::view::SortKey,
) -> Vec<fx_ui::ColumnSpec> {
    layout
        .columns
        .iter()
        .map(|cs| {
            let col_name = cs.column.to_string();
            let sort_active = sort_key.primary.to_string() == col_name;
            let sort_ascending = matches!(
                sort_key.direction,
                fx_domain::view::SortDirection::Ascending
            );
            let sort_direction = if sort_active {
                if sort_ascending {
                    SharedString::from("ascending")
                } else {
                    SharedString::from("descending")
                }
            } else {
                SharedString::from("none")
            };
            fx_ui::ColumnSpec {
                title: SharedString::from(col_name),
                width: cs.width_px as f32,
                visible: cs.visible,
                sort_ascending: sort_ascending && sort_active,
                sort_active,
                sort_direction,
            }
        })
        .collect()
}
