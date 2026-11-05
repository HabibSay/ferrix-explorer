//! Navigation orchestration for Ferrix Explorer Tranche 1 Slice 3.
//!
//! Owns the Tokio runtime, a `CancellationToken` per listing, `NavigationHistory`,
//! and `ViewStateStore`. Drives the Slint UI via `invoke_from_event_loop`.

use fx_domain::{
    history::NavigationHistory,
    location::Location,
};
use fx_fs::listing::{ListingBatch, ListingError, list_directory};
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
                        RawWindowHandle::Win32(w) => Some(windows::Win32::Foundation::HWND(
                            w.hwnd.get() as *mut _,
                        )),
                        _ => None,
                    })
            })
            .unwrap_or(windows::Win32::Foundation::HWND(std::ptr::null_mut()))
    }

    /// Cancel any running listing, restore per-folder view state, clear the model,
    /// and kick off a new listing task.
    fn start_listing(&mut self, location: Location) {
        self.cancel.cancel();
        self.cancel = CancellationToken::new();
        self.generation = self.generation.wrapping_add(1);

        // Restore per-folder view state.
        let folder_key = location.to_string();
        let folder_state = self.view_state.get(&folder_key);
        {
            let mut vm = self.view_model.lock().expect("view model lock");
            vm.clear();
            vm.set_view_mode(folder_state.view_mode);
            vm.set_sort_key(folder_state.sort_key.clone());
        }

        // Update path, breadcrumb, rows in the Slint window.
        let path_str = SharedString::from(location.to_string());
        let breadcrumb: Vec<SharedString> = build_breadcrumb_segments(&location);
        let window_weak = self.window.clone();
        invoke_from_event_loop(move || {
            if let Some(win) = window_weak.upgrade() {
                win.set_current_path(path_str);
                win.set_breadcrumb_segments(ModelRc::new(VecModel::from(breadcrumb)));
                win.set_rows(ModelRc::new(VecModel::from(Vec::<fx_ui::FileRow>::new())));
                win.set_status_message(SharedString::from("Loading…"));
            }
        })
        .ok();

        // Spawn the background listing task.
        let token = self.cancel.clone();
        let generation = self.generation;
        let vm_arc = self.view_model.clone();
        let window_weak2 = self.window.clone();

        self.runtime.spawn(async move {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<ListingBatch>(8);

            let list_fut = list_directory(&location, generation, token, tx);

            let recv_fut = async {
                // Row model shared between event-loop invocations.
                // We rebuild from scratch on every batch to keep things correct.
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
                            vm.to_ui_rows()
                                .into_iter()
                                .map(ui_row_to_slint)
                                .collect()
                        };
                        if let Some(win) = win_weak.upgrade() {
                            win.set_rows(ModelRc::new(VecModel::from(rows)));
                            let count = vm_clone.lock().expect("vm lock").entry_count();
                            win.set_status_message(SharedString::from(format!("{count} items")));
                        }
                        if let Some(err) = batch_error {
                            if let Some(win) = win_weak.upgrade() {
                                win.set_status_message(SharedString::from(err.to_string()));
                            }
                        }
                    })
                    .ok();
                }
            };

            let (list_result, _) = tokio::join!(list_fut, recv_fut);

            if let Err(err) = list_result {
                if !matches!(err, ListingError::Cancelled) {
                    let msg = err.to_string();
                    invoke_from_event_loop(move || {
                        if let Some(win) = window_weak2.upgrade() {
                            win.set_status_message(SharedString::from(msg));
                        }
                    })
                    .ok();
                }
            }
        });
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
