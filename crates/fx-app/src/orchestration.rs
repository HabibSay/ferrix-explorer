//! Navigation orchestration for Ferrix Explorer Tranche 1 Slice 4 + Tranche 2.
//!
//! Owns the Tokio runtime, a `CancellationToken` per listing, `NavigationHistory`,
//! `ViewStateStore`, and a `NotifyWatcher` for the current directory.
//! Tranche 2 additions: `OperationRunner`, `WindowsPlatformOps`, status bar updates.
//! Drives the Slint UI via `invoke_from_event_loop`.

use fx_domain::{
    history::NavigationHistory, keys::ShortcutAction, location::Location, view::ColumnLayout,
};
use fx_fs::listing::{ListingBatch, ListingError, list_directory};
use fx_fs::watch::{LocationWatcher, NotifyWatcher, WatchEvent};
use fx_settings::view_state::ViewStateStore;
use fx_shell::clipboard::ClipboardEffect;
use fx_ui::ApplicationWindow;
use fx_ui::viewmodel::FileListViewModel;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, invoke_from_event_loop};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

use crate::operations::{OpEvent, OperationRunner};
use crate::platform_ops::WindowsPlatformOps;
use crate::strings;

// ── Orchestrator ──────────────────────────────────────────────────────────────

/// Drives navigation and file operations for a single `ApplicationWindow`.
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
    /// Tranche 2: operation runner.
    operations: OperationRunner,
    /// Tranche 2: Windows platform adapter.
    port: Arc<WindowsPlatformOps>,
    /// Paths snapshotted when permanent-delete confirmation was requested; the
    /// confirmation deletes exactly these, even if the selection changed meanwhile.
    pending_permanent_delete: Vec<PathBuf>,
    /// Conflict-dialog answers forwarded to the waiting dispatcher task; cleared on
    /// window close so a pending conflict resolves as cancel instead of hanging shutdown.
    conflict_ui_tx: Option<tokio::sync::mpsc::UnboundedSender<(String, bool)>>,
    /// Moved into the dispatch task by `spawn_event_dispatch`.
    event_rx: Option<tokio::sync::mpsc::Receiver<OpEvent>>,
    /// Moved into the dispatch task by `spawn_event_dispatch`.
    conflict_ui_rx: Option<tokio::sync::mpsc::UnboundedReceiver<(String, bool)>>,
    /// Yes/no answer for the empty-folder-cleaner confirmation.
    clean_ui_tx: Option<tokio::sync::mpsc::UnboundedSender<bool>>,
    /// Moved into the dispatch task by `spawn_event_dispatch`.
    clean_ui_rx: Option<tokio::sync::mpsc::UnboundedReceiver<bool>>,
    /// Activity rows, shared with the dispatch task (cap 50, PRD 13.4).
    activity: Arc<Mutex<crate::activity::ActivityStore>>,
    /// Folder the last submitted command modifies; the dispatcher refreshes it on
    /// completion when it is still the visible folder (spec §6).
    refresh_target: Arc<Mutex<Option<PathBuf>>>,
}

impl Orchestrator {
    /// Create a new orchestrator and immediately begin listing `initial_location`.
    pub fn new(window: slint::Weak<ApplicationWindow>, initial_location: Location) -> Self {
        let runtime = Runtime::new().expect("Tokio runtime");
        let view_state = ViewStateStore::load().unwrap_or_else(|err| {
            tracing::warn!(%err, "Could not load view state; using defaults");
            ViewStateStore::new_empty()
        });

        let port = Arc::new(WindowsPlatformOps);
        let (operations, event_rx) = OperationRunner::new(runtime.handle().clone());
        let (conflict_ui_tx, conflict_ui_rx) =
            tokio::sync::mpsc::unbounded_channel::<(String, bool)>();
        let (clean_ui_tx, clean_ui_rx) = tokio::sync::mpsc::unbounded_channel::<bool>();

        let mut orch = Orchestrator {
            view_model: Arc::new(Mutex::new(FileListViewModel::new())),
            history: NavigationHistory::new(initial_location.clone()),
            view_state,
            runtime,
            cancel: CancellationToken::new(),
            generation: 0,
            window: window.clone(),
            watcher: NotifyWatcher::new(),
            watch_rx: None,
            operations,
            port,
            pending_permanent_delete: Vec::new(),
            conflict_ui_tx: Some(conflict_ui_tx),
            event_rx: Some(event_rx),
            conflict_ui_rx: Some(conflict_ui_rx),
            clean_ui_tx: Some(clean_ui_tx),
            clean_ui_rx: Some(clean_ui_rx),
            activity: Arc::new(Mutex::new(crate::activity::ActivityStore::new())),
            refresh_target: Arc::new(Mutex::new(None)),
        };

        orch.start_listing(initial_location);
        orch
    }

    /// Moves the event and conflict receivers into the dispatch task. Called by
    /// `main` once the `Arc<Mutex<Orchestrator>>` exists, because the dispatcher
    /// needs that handle to update rows, refresh folders, and reach the runner.
    pub fn spawn_event_dispatch(orch: Arc<Mutex<Orchestrator>>) {
        let (runtime, event_rx, conflict_rx, clean_rx) = {
            let mut guard = orch.lock().expect("orchestrator");
            let runtime = guard.runtime.handle().clone();
            (
                runtime,
                guard.event_rx.take().expect("dispatch already started"),
                guard
                    .conflict_ui_rx
                    .take()
                    .expect("dispatch already started"),
                guard.clean_ui_rx.take().expect("dispatch already started"),
            )
        };
        runtime.spawn(async move {
            dispatch_op_events(orch, event_rx, conflict_rx, clean_rx).await;
        });
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
            let is_dir = vm.entry_is_dir(entry_id);
            path.zip(is_dir)
        };
        let Some((path, is_dir)) = info else { return };

        if is_dir {
            self.navigate_to(Location::Local(path));
        } else {
            self.open_file(path);
        }
    }

    /// Called when the user drags a column resize handle.
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

    /// Called when the user changes the row selection.
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
        self.push_status_to_window();
    }

    // ── Tranche 2: file operation commands ─────────────────────────────────

    /// Copy selected entries to the clipboard.
    pub fn on_copy(&mut self) {
        self.write_to_clipboard(ClipboardEffect::Copy);
    }

    /// Cut selected entries to the clipboard.
    pub fn on_cut(&mut self) {
        self.write_to_clipboard(ClipboardEffect::Move);
    }

    /// Clipboard writes contend for a process-global lock and retry with sleeps, so they
    /// run on the blocking pool; the Slint thread must never wait on them.
    fn write_to_clipboard(&self, effect: ClipboardEffect) {
        let paths = self.view_model.lock().expect("vm lock").selected_paths();
        if paths.is_empty() {
            return;
        }
        let handle = self.runtime.handle().clone();
        drop(handle.spawn_blocking(move || {
            if let Err(err) = fx_shell::clipboard::write_clipboard(&paths, effect) {
                tracing::warn!(%err, "clipboard write failed");
            }
        }));
    }

    /// Paste from the clipboard into the current folder.
    pub fn on_paste(&mut self) {
        let Some(destination) = self.local_folder() else {
            self.announce_unsupported_location();
            return;
        };
        let id = self
            .operations
            .paste_from_clipboard(destination.clone(), self.port.clone());
        self.begin_row(
            id,
            strings::ACTIVITY_PASTE,
            crate::activity::RetrySpec::Paste {
                destination: destination.clone(),
            },
            Some(destination),
        );
    }

    /// Delete selected entries to the Recycle Bin.
    pub fn on_delete(&mut self) {
        let paths = self.view_model.lock().expect("vm lock").selected_paths();
        if paths.is_empty() {
            return;
        }
        let touched = Self::touched_parent(&paths);
        let id = self
            .operations
            .delete_selection(paths, false, self.port.clone());
        self.begin_row(
            id,
            strings::ACTIVITY_RECYCLE,
            crate::activity::RetrySpec::Delete { permanent: false },
            touched,
        );
    }

    /// Snapshot the selection and show the confirmation gate (Shift+Del).
    /// Nothing is deleted until `confirm_permanent_delete` runs on that snapshot.
    pub fn request_permanent_delete(&mut self) {
        let paths = self.view_model.lock().expect("vm lock").selected_paths();
        if paths.is_empty() {
            return;
        }
        let count = paths.len();
        self.pending_permanent_delete = paths;
        let window_weak = self.window.clone();
        invoke_from_event_loop(move || {
            if let Some(win) = window_weak.upgrade() {
                win.set_permanent_delete_count_text(SharedString::from(format!(
                    "{count} {}",
                    strings::STATUS_ITEMS_COUNT
                )));
                win.set_confirm_permanent_delete(true);
            }
        })
        .ok();
    }

    /// Confirmation accepted: permanently delete exactly the snapshotted paths.
    pub fn confirm_permanent_delete(&mut self) {
        let paths = std::mem::take(&mut self.pending_permanent_delete);
        if paths.is_empty() {
            return;
        }
        let touched = Self::touched_parent(&paths);
        let id = self
            .operations
            .delete_selection(paths, true, self.port.clone());
        self.begin_row(
            id,
            strings::ACTIVITY_DELETE_PERMANENTLY,
            crate::activity::RetrySpec::Delete { permanent: true },
            touched,
        );
    }

    /// Confirmation dismissed: forget the snapshot, delete nothing.
    pub fn cancel_permanent_delete(&mut self) {
        self.pending_permanent_delete.clear();
    }

    /// The conflict dialog answered (or was escaped): hand the choice to the dispatcher
    /// task that holds the paused worker's answer channel open.
    pub fn submit_conflict_answer(&mut self, choice: String, apply_to_all: bool) {
        if let Some(tx) = &self.conflict_ui_tx {
            tx.send((choice, apply_to_all)).ok();
        }
    }

    /// Window closing: drop the sender so a pending conflict resolves as cancel and
    /// the paused worker unwinds before runtime shutdown.
    pub fn close_conflict_channel(&mut self) {
        self.conflict_ui_tx = None;
    }

    // ── Tranche 2: activity rows, retry, and the selection menu ────────────

    fn begin_row(
        &mut self,
        id: fx_ops::progress::OperationId,
        label: &str,
        retry: crate::activity::RetrySpec,
        touched: Option<PathBuf>,
    ) {
        self.activity.lock().expect("activity").push(
            id,
            crate::activity::RowMeta {
                label: label.to_string(),
                retry,
            },
        );
        *self.refresh_target.lock().expect("refresh slot") = touched;
        self.sync_activity();
    }

    fn sync_activity(&self) {
        let rows: Vec<fx_ui::ActivityRow> = self
            .activity
            .lock()
            .expect("activity")
            .rows
            .iter()
            .map(|row| fx_ui::ActivityRow {
                op_kind: SharedString::from(row.label.clone()),
                current_file: SharedString::from(row.current.clone()),
                files_done: row.files_done.min(u32::MAX as u64) as i32,
                files_total: row.files_total.min(u32::MAX as u64) as i32,
                bytes_done: row.bytes_done as f32,
                bytes_total: row.bytes_total as f32,
                speed_text: SharedString::from(row.speed_text.clone()),
                eta_text: SharedString::from(row.eta_text.clone()),
                state: SharedString::from(row.state.label()),
                is_paused: row.state == crate::activity::RowState::Paused,
                error_text: SharedString::from(row.error_text.clone()),
                progress_text: SharedString::from(progress_text_of(row)),
            })
            .collect();
        let window_weak = self.window.clone();
        invoke_from_event_loop(move || {
            if let Some(win) = window_weak.upgrade() {
                win.set_activity_rows(ModelRc::new(VecModel::from(rows)));
            }
        })
        .ok();
    }

    fn row_id(&self, index: usize) -> Option<fx_ops::progress::OperationId> {
        self.activity.lock().expect("activity").row_id(index)
    }

    pub fn activity_pause(&mut self, index: usize) {
        let Some(id) = self.row_id(index) else { return };
        self.operations.pause_op(id);
        self.activity
            .lock()
            .expect("activity")
            .note_paused(id, true);
        self.sync_activity();
    }

    pub fn activity_resume(&mut self, index: usize) {
        let Some(id) = self.row_id(index) else { return };
        self.operations.resume_op(id);
        self.activity
            .lock()
            .expect("activity")
            .note_paused(id, false);
        self.sync_activity();
    }

    pub fn activity_cancel(&mut self, index: usize) {
        let Some(id) = self.row_id(index) else { return };
        // A command dropped from the queue produces no terminal event; close its
        // row here so the pane never shows a phantom pending operation.
        if self.operations.cancel_op(id) {
            self.activity.lock().expect("activity").note_terminal(
                id,
                crate::activity::RowState::Cancelled,
                Vec::new(),
                Vec::new(),
            );
            self.sync_activity();
        }
    }

    /// Retry starts a *new* operation over exactly the items that failed (spec
    /// §9), recorded under its own row.
    pub fn activity_retry(&mut self, index: usize) {
        use crate::activity::RetrySpec;
        let Some(request) = self.activity.lock().expect("activity").retry_request(index) else {
            return;
        };
        let old_label = self
            .activity
            .lock()
            .expect("activity")
            .rows
            .get(index)
            .map(|row| row.label.clone())
            .unwrap_or_default();
        let label = format!("{}{old_label}", strings::ACTIVITY_RETRY_PREFIX);
        let port = self.port.clone();
        let (id, retry, touched) = match request.spec {
            RetrySpec::Paste { destination } => {
                let touched = Some(destination.clone());
                let id = self
                    .operations
                    .retry_paste(request.sources, destination.clone(), port);
                (id, RetrySpec::Paste { destination }, touched)
            }
            RetrySpec::Delete { permanent } => {
                let touched = request
                    .sources
                    .first()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()));
                let id = self
                    .operations
                    .delete_selection(request.sources, permanent, port);
                (id, RetrySpec::Delete { permanent }, touched)
            }
            RetrySpec::Rename { path, new_name } => {
                let touched = path.parent().map(|d| d.to_path_buf());
                let id = self
                    .operations
                    .rename_entry(path.clone(), new_name.clone(), port);
                (id, RetrySpec::Rename { path, new_name }, touched)
            }
            RetrySpec::Create { parent, name, dir } => {
                let touched = Some(parent.clone());
                let id = if dir {
                    self.operations
                        .create_folder(parent.clone(), name.clone(), port)
                } else {
                    self.operations
                        .create_file(parent.clone(), name.clone(), port)
                };
                (id, RetrySpec::Create { parent, name, dir }, touched)
            }
            RetrySpec::Checksum { paths } => {
                let id = self.operations.compute_checksums(paths.clone());
                (id, RetrySpec::Checksum { paths }, None)
            }
            RetrySpec::Clean { root } => {
                let id = self.operations.scan_empty_folders(root.clone());
                (id, RetrySpec::Clean { root }, None)
            }
            RetrySpec::Undo => {
                let touched = self.local_folder();
                (
                    self.operations.undo(self.port.clone()),
                    RetrySpec::Undo,
                    touched,
                )
            }
            RetrySpec::Redo => {
                let touched = self.local_folder();
                (
                    self.operations.redo(self.port.clone()),
                    RetrySpec::Redo,
                    touched,
                )
            }
        };
        self.begin_row(id, &label, retry, touched);
    }

    /// Right-click / Shift+F10 over the selection: the app-local menu carrying the
    /// menu-driven Tranche 2 commands (PRD 13.4: checksums; OPS-11 pattern; OPS-13).
    pub fn on_context_menu(&mut self) {
        let has_selection = {
            let vm = self.view_model.lock().expect("vm lock");
            !vm.selected_paths().is_empty()
        };
        // The label is presentation; the command travels beside it so a translation can
        // never change what a menu click does.
        #[derive(Clone, Copy)]
        enum Command {
            Checksums,
            Pattern,
            Invert,
            NewFile,
            Clean,
        }
        let items: Vec<(&str, Command)> = if has_selection {
            vec![
                (strings::ACTIVITY_CHECKSUMS, Command::Checksums),
                (strings::MENU_SELECT_BY_PATTERN, Command::Pattern),
                (strings::MENU_INVERT_SELECTION, Command::Invert),
                (strings::MENU_NEW_FILE, Command::NewFile),
                (strings::ACTIVITY_CLEAN, Command::Clean),
            ]
        } else {
            vec![
                (strings::MENU_SELECT_BY_PATTERN, Command::Pattern),
                (strings::MENU_INVERT_SELECTION, Command::Invert),
                (strings::MENU_NEW_FILE, Command::NewFile),
                (strings::ACTIVITY_CLEAN, Command::Clean),
            ]
        };
        let labels: Vec<&str> = items.iter().map(|(label, _)| *label).collect();
        let hwnd = self.get_hwnd();
        let (x, y) = fx_shell::cursor_pos();
        let chosen = match fx_shell::show_popup_menu(hwnd, x, y, &labels) {
            Ok(Some(index)) => index,
            _ => return,
        };
        match items.get(chosen).map(|(_, command)| *command) {
            Some(Command::Checksums) => self.compute_checksums_for_selection(),
            Some(Command::Pattern) => {
                if let Some(win) = self.window.upgrade() {
                    win.set_show_pattern_dialog(true);
                }
            }
            Some(Command::Invert) => self.on_invert_selection(),
            Some(Command::NewFile) => self.on_create_file(),
            Some(Command::Clean) => self.start_clean_scan(),
            None => {}
        }
    }

    pub fn compute_checksums_for_selection(&mut self) {
        let paths = self.view_model.lock().expect("vm lock").selected_paths();
        if paths.is_empty() {
            return;
        }
        let id = self.operations.compute_checksums(paths.clone());
        self.begin_row(
            id,
            strings::ACTIVITY_CHECKSUMS,
            crate::activity::RetrySpec::Checksum { paths },
            None,
        );
    }

    // ── Tranche 2: cleaner report-first, pattern selection, internal DnD ───

    pub fn start_clean_scan(&mut self) {
        let Some(root) = self.local_folder() else {
            self.announce_unsupported_location();
            return;
        };
        let id = self.operations.scan_empty_folders(root.clone());
        self.begin_row(
            id,
            strings::ACTIVITY_CLEAN_SCAN,
            crate::activity::RetrySpec::Clean { root },
            None,
        );
    }

    /// Called only after the user confirmed the scan report (OPS-13).
    pub fn start_clean_from_report(&mut self, root: PathBuf, directories: Vec<PathBuf>) {
        let plan = fx_ops::clean::CleanPlan { directories };
        let id = self.operations.clean_empty_folders(root.clone(), plan);
        self.begin_row(
            id,
            strings::ACTIVITY_CLEAN,
            crate::activity::RetrySpec::Clean { root: root.clone() },
            Some(root),
        );
    }

    pub fn submit_clean_answer(&mut self, accepted: bool) {
        if let Some(tx) = &self.clean_ui_tx {
            tx.send(accepted).ok();
        }
    }

    pub fn on_pattern_submitted(&mut self, pattern: String) {
        let matched = self
            .view_model
            .lock()
            .expect("vm lock")
            .select_by_pattern(&pattern);
        self.push_rows_to_window();
        self.push_status_to_window();
        let msg = format!("{matched} {}", strings::ENTRIES_MATCHED);
        let window_weak = self.window.clone();
        invoke_from_event_loop(move || {
            if let Some(win) = window_weak.upgrade() {
                win.set_status_message(SharedString::from(msg));
            }
        })
        .ok();
    }

    pub fn on_invert_selection(&mut self) {
        self.view_model.lock().expect("vm lock").invert_selection();
        self.push_rows_to_window();
        self.push_status_to_window();
    }

    /// A row press arms a drag: the payload is the selection when the pressed row
    /// is part of it, otherwise just that row (Explorer rule).
    pub fn on_drag_pressed(&mut self, entry_id: u64) -> slint::DataTransfer {
        let paths = {
            let vm = self.view_model.lock().expect("vm lock");
            if vm.is_selected(entry_id) {
                vm.selected_paths()
            } else {
                vm.entry_path(entry_id).into_iter().collect::<Vec<_>>()
            }
        };
        let mut transfer = slint::DataTransfer::default();
        transfer.set_file_paths(paths);
        transfer
    }

    /// `can-drop` already rejected wrong targets; this is the final negotiation
    /// and enqueue. Codes: 0 none, 1 copy, 2 move (mirrors `DragAction`).
    pub fn on_drop_requested(
        &mut self,
        data: slint::DataTransfer,
        proposed: i32,
        target_id: i32,
    ) -> i32 {
        let Ok(iter) = data.file_paths() else {
            return 0;
        };
        let paths: Vec<PathBuf> = iter.map(|p| p.to_path_buf()).collect();
        if paths.is_empty() {
            return 0;
        }
        let destination = if target_id < 0 {
            let Some(folder) = self.local_folder() else {
                self.announce_unsupported_location();
                return 0;
            };
            folder
        } else {
            let Some(p) = self
                .view_model
                .lock()
                .expect("vm lock")
                .entry_path(target_id as u64)
            else {
                return 0;
            };
            p
        };
        // Rule 15: dropping rows back onto their own folder is a no-op with a message.
        if paths
            .first()
            .and_then(|p| p.parent())
            .is_some_and(|par| par == destination.as_path())
        {
            let window_weak = self.window.clone();
            invoke_from_event_loop(move || {
                if let Some(win) = window_weak.upgrade() {
                    win.set_status_message(SharedString::from(strings::STATUS_DROP_IN_SOURCE));
                }
            })
            .ok();
            return 0;
        }
        use fx_ops::port::PlatformOps;
        let wanted_copy = proposed == 1;
        let same_volume = match (
            self.port.volume_id(&paths[0]),
            self.port.volume_id(&destination),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => true,
        };
        // Explorer default: no modifier on a cross-volume drag is a copy, never a move.
        let kind = if wanted_copy || !same_volume {
            fx_ops::plan::TransferKind::Copy
        } else {
            fx_ops::plan::TransferKind::Move
        };
        let is_copy = kind == fx_ops::plan::TransferKind::Copy;
        let id =
            self.operations
                .transfer_paths(paths, destination.clone(), kind, self.port.clone());
        self.begin_row(
            id,
            if is_copy {
                strings::DRAG_COPY
            } else {
                strings::DRAG_MOVE
            },
            crate::activity::RetrySpec::Paste {
                destination: destination.clone(),
            },
            Some(destination),
        );
        if is_copy { 1 } else { 2 }
    }

    fn on_create_file(&mut self) {
        let Some(parent) = self.local_folder() else {
            self.announce_unsupported_location();
            return;
        };
        let name = strings::NEW_FILE_NAME.to_string();
        let id = self
            .operations
            .create_file(parent.clone(), name.clone(), self.port.clone());
        self.begin_row(
            id,
            strings::ACTIVITY_NEW_FILE,
            crate::activity::RetrySpec::Create {
                parent: parent.clone(),
                name,
                dir: false,
            },
            Some(parent),
        );
    }

    fn touched_parent(paths: &[PathBuf]) -> Option<PathBuf> {
        paths
            .first()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    }

    /// Begin an inline rename for the entry with `entry_id`.
    pub fn on_rename(&mut self, entry_id: u64) {
        let seed = {
            let mut vm = self.view_model.lock().expect("vm lock");
            vm.set_rename_target(Some(entry_id));
            vm.rename_seed(entry_id)
        };
        let Some((name, is_folder)) = seed else {
            return;
        };
        // Explorer pre-selects the base name so typing replaces it and the extension survives.
        let selection = fx_ui::viewmodel::rename_selection(&name, is_folder) as i32;
        let window_weak = self.window.clone();
        invoke_from_event_loop(move || {
            if let Some(win) = window_weak.upgrade() {
                // The seed must be set before the target id: the editor reads it when it is
                // created, which is exactly when the id stops being -1.
                win.set_rename_seed_text(SharedString::from(name));
                win.set_rename_seed_selection(selection);
                win.set_status_rename_target_id(entry_id as i32);
            }
        })
        .ok();
    }

    /// Live inline-rename feedback: the reason the name cannot be used, or empty when it can.
    pub fn on_rename_validate(&mut self, name: String) -> String {
        match fx_ops::rename::validate_name(&name) {
            Ok(()) => String::new(),
            Err(err) => err.to_string(),
        }
    }

    /// The inline rename was committed; apply it.
    pub fn on_rename_committed(&mut self, entry_id: u64, new_name: String) {
        if let Err(err) = fx_ops::rename::validate_name(&new_name) {
            // The editor stays open with its current text: refusing the commit is the
            // difference between a rejected rename and a silently lost one.
            self.set_status(format!("{}{err}", strings::STATUS_RENAME_REFUSED));
            return;
        }
        let path = {
            let mut vm = self.view_model.lock().expect("vm lock");
            vm.set_rename_target(None);
            vm.entry_path(entry_id)
        };
        let window_weak = self.window.clone();
        invoke_from_event_loop(move || {
            if let Some(win) = window_weak.upgrade() {
                win.set_status_rename_target_id(-1);
            }
        })
        .ok();

        let Some(path) = path else { return };
        let touched = path.parent().map(|d| d.to_path_buf());
        let id = self
            .operations
            .rename_entry(path.clone(), new_name.clone(), self.port.clone());
        self.begin_row(
            id,
            strings::ACTIVITY_RENAME,
            crate::activity::RetrySpec::Rename { path, new_name },
            touched,
        );
    }

    /// Create a new folder in the current directory.
    pub fn on_create_folder(&mut self) {
        let Some(parent) = self.local_folder() else {
            self.announce_unsupported_location();
            return;
        };
        let name = strings::ACTIVITY_NEW_FOLDER.to_string();
        let id = self
            .operations
            .create_folder(parent.clone(), name.clone(), self.port.clone());
        self.begin_row(
            id,
            strings::ACTIVITY_NEW_FOLDER,
            crate::activity::RetrySpec::Create {
                parent: parent.clone(),
                name,
                dir: true,
            },
            Some(parent),
        );
    }

    /// Undo the last reversible operation.
    pub fn on_undo(&mut self) {
        let touched = self.local_folder();
        let id = self.operations.undo(self.port.clone());
        self.begin_row(
            id,
            strings::ACTIVITY_UNDO,
            crate::activity::RetrySpec::Undo,
            touched,
        );
    }

    /// Redo the previously undone operation.
    pub fn on_redo(&mut self) {
        let touched = self.local_folder();
        let id = self.operations.redo(self.port.clone());
        self.begin_row(
            id,
            strings::ACTIVITY_REDO,
            crate::activity::RetrySpec::Redo,
            touched,
        );
    }

    /// Route a domain shortcut action to its command; true when dispatched.
    pub fn dispatch_action(&mut self, action: ShortcutAction) -> bool {
        match action {
            ShortcutAction::Copy => self.on_copy(),
            ShortcutAction::Cut => self.on_cut(),
            ShortcutAction::Delete => self.on_delete(),
            ShortcutAction::PermanentDelete => self.request_permanent_delete(),
            ShortcutAction::Paste => self.on_paste(),
            ShortcutAction::Undo => self.on_undo(),
            ShortcutAction::Redo => self.on_redo(),
            ShortcutAction::CreateFolder => self.on_create_folder(),
            ShortcutAction::Rename => self.on_rename_selected(),
            ShortcutAction::SelectAll => self.on_select_all(),
            ShortcutAction::NavigateBack => self.navigate_back(),
            ShortcutAction::NavigateForward => self.navigate_forward(),
            ShortcutAction::NavigateUp => self.navigate_up(),
            ShortcutAction::Refresh => self.refresh(),
            ShortcutAction::ContextMenu => self.on_context_menu(),
            // Properties / FocusAddressBar have no command path yet.
            _ => return false,
        }
        true
    }

    /// Begin an inline rename of the first selected entry (F2).
    pub fn on_rename_selected(&mut self) {
        let id = self.view_model.lock().expect("vm lock").first_selected_id();
        if let Some(id) = id {
            self.on_rename(id);
        }
    }

    /// The inline rename was dismissed; clear the editor without touching the file.
    pub fn on_rename_cancelled(&mut self) {
        self.view_model
            .lock()
            .expect("vm lock")
            .set_rename_target(None);
        let window_weak = self.window.clone();
        invoke_from_event_loop(move || {
            if let Some(win) = window_weak.upgrade() {
                win.set_status_rename_target_id(-1);
            }
        })
        .ok();
    }

    /// Select every visible entry (Ctrl+A).
    pub fn on_select_all(&mut self) {
        self.view_model.lock().expect("vm lock").select_all();
        self.push_rows_to_window();
        self.push_status_to_window();
    }

    // ── Internal ──────────────────────────────────────────────────────────

    /// The current folder when the view is a local path. Non-local locations do not
    /// support Tranche 2 operations (spec §1: refuse with a message, never redirect).
    fn local_folder(&self) -> Option<PathBuf> {
        match &self.history.current().location {
            Location::Local(p) => Some(p.clone()),
            _ => None,
        }
    }

    fn announce_unsupported_location(&self) {
        self.set_status(strings::STATUS_NEEDS_LOCAL_FOLDER.to_string());
    }

    /// Show a message in the status bar.
    fn set_status(&self, message: String) {
        let window_weak = self.window.clone();
        invoke_from_event_loop(move || {
            if let Some(win) = window_weak.upgrade() {
                win.set_status_message(SharedString::from(message));
            }
        })
        .ok();
    }

    /// Open a file with `ShellExecuteW`.
    fn open_file(&self, path: PathBuf) {
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
                win.set_status_message(SharedString::from(strings::STATUS_LOADING));
                win.set_status_item_count(SharedString::default());
                win.set_status_selected_count(SharedString::default());
                win.set_status_selected_bytes(SharedString::default());
            }
        })
        .ok();

        // Start filesystem watcher for this location.
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
                                let (count, sel_count) = {
                                    let vm = vm_c.lock().expect("vm lock");
                                    (vm.entry_count(), vm.selected_count())
                                };
                                win.set_status_item_count(SharedString::from(format!(
                                    "{count} items"
                                )));
                                win.set_status_selected_count(SharedString::from(format!(
                                    "{sel_count} {}",
                                    strings::SELECTED_COUNT_FMT
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
        let port_clone = self.port.clone();
        let location_str = location.to_string();

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
                            let (count, sel_count) = {
                                let vm = vm_clone.lock().expect("vm lock");
                                (vm.entry_count(), vm.selected_count())
                            };
                            win.set_status_item_count(SharedString::from(format!(
                                "{count} {}",
                                strings::STATUS_ITEMS_COUNT
                            )));
                            win.set_status_selected_count(SharedString::from(format!(
                                "{sel_count} {}",
                                strings::SELECTED_COUNT_FMT
                            )));
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
                let ww = window_weak2.clone();
                invoke_from_event_loop(move || {
                    if let Some(win) = ww.upgrade() {
                        win.set_status_message(SharedString::from(msg));
                    }
                })
                .ok();
            }

            // After listing completes, update free space asynchronously.
            let location_path = PathBuf::from(&location_str);
            let free_str_result = tokio::task::spawn_blocking(move || {
                use fx_ops::port::PlatformOps;
                port_clone.free_space(&location_path).ok()
            })
            .await
            .ok()
            .flatten();

            if let Some(bytes) = free_str_result {
                let free_str = format_bytes(bytes) + " free";
                invoke_from_event_loop(move || {
                    if let Some(win) = window_weak2.upgrade() {
                        win.set_status_free_space(SharedString::from(free_str));
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
                win.set_status_item_count(SharedString::from(format!(
                    "{count} {}",
                    strings::STATUS_ITEMS_COUNT
                )));
            }
        })
        .ok();
    }

    /// Push enriched status bar data (selected count, selected bytes) to the window.
    fn push_status_to_window(&self) {
        let (sel_count, sel_bytes) = {
            let vm = self.view_model.lock().expect("view model lock");
            (vm.selected_count(), vm.selected_bytes())
        };
        let window_weak = self.window.clone();
        invoke_from_event_loop(move || {
            if let Some(win) = window_weak.upgrade() {
                win.set_status_selected_count(SharedString::from(format!(
                    "{sel_count} {}",
                    strings::SELECTED_COUNT_FMT
                )));
                win.set_status_selected_bytes(SharedString::from(format_bytes(sel_bytes)));
            }
        })
        .ok();
    }
}

// ── OpEvent dispatcher ────────────────────────────────────────────────────────

async fn dispatch_op_events(
    orch: Arc<Mutex<Orchestrator>>,
    mut event_rx: tokio::sync::mpsc::Receiver<OpEvent>,
    mut conflict_ui_rx: tokio::sync::mpsc::UnboundedReceiver<(String, bool)>,
    mut clean_ui_rx: tokio::sync::mpsc::UnboundedReceiver<bool>,
) {
    while let Some(event) = event_rx.recv().await {
        let window = orch.lock().expect("orchestrator").window.clone();
        match event {
            OpEvent::Progress { id, progress } => {
                let msg = format!(
                    "{}/{} {}, {}",
                    progress.files_done,
                    progress.files_total,
                    strings::PROGRESS_FILES,
                    progress
                        .current
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                );
                {
                    let guard = orch.lock().expect("orchestrator");
                    guard
                        .activity
                        .lock()
                        .expect("activity")
                        .note_progress(id, &progress);
                    guard.sync_activity();
                }
                invoke_from_event_loop(move || {
                    if let Some(w) = window.upgrade() {
                        w.set_status_message(SharedString::from(msg));
                    }
                })
                .ok();
            }
            OpEvent::Completed { id, summary } => {
                tracing::info!(id = id.0, "operation completed");
                let (errors, failed) = failure_lists(&summary);
                {
                    let guard = orch.lock().expect("orchestrator");
                    guard.activity.lock().expect("activity").note_terminal(
                        id,
                        row_state_of(&summary),
                        errors,
                        failed,
                    );
                    guard.sync_activity();
                }
                finish_refresh(&orch);
                invoke_from_event_loop(move || {
                    if let Some(w) = window.upgrade() {
                        w.set_status_message(SharedString::from(strings::STATUS_DONE));
                    }
                })
                .ok();
            }
            OpEvent::Failed { id, summary } => {
                let msg = summary
                    .error
                    .as_ref()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| strings::STATUS_OPERATION_FAILED.to_string());
                let (errors, failed) = failure_lists(&summary);
                tracing::warn!(id = id.0, "operation failed: {msg}");
                {
                    let guard = orch.lock().expect("orchestrator");
                    guard.activity.lock().expect("activity").note_terminal(
                        id,
                        row_state_of(&summary),
                        errors,
                        failed,
                    );
                    guard.sync_activity();
                }
                finish_refresh(&orch);
                invoke_from_event_loop(move || {
                    if let Some(w) = window.upgrade() {
                        w.set_status_message(SharedString::from(msg));
                    }
                })
                .ok();
            }
            OpEvent::ChecksumResults { rows, .. } => {
                let slint_rows: Vec<fx_ui::ChecksumRow> = rows
                    .into_iter()
                    .map(|r| fx_ui::ChecksumRow {
                        path: SharedString::from(r.path.display().to_string()),
                        md5: SharedString::from(r.md5),
                        sha256: SharedString::from(r.sha256),
                        blake3: SharedString::from(r.blake3),
                    })
                    .collect();
                invoke_from_event_loop(move || {
                    if let Some(w) = window.upgrade() {
                        w.set_checksum_rows(ModelRc::new(VecModel::from(slint_rows)));
                        w.set_show_checksum_sheet(true);
                    }
                })
                .ok();
            }
            OpEvent::EmptyFolderReport {
                id,
                root,
                directories,
            } => {
                let count = directories.len();
                tracing::info!(id = id.0, count, "cleaner scan report ready");
                if count == 0 {
                    invoke_from_event_loop(move || {
                        if let Some(w) = window.upgrade() {
                            w.set_status_message(SharedString::from(
                                strings::STATUS_NO_EMPTY_FOLDERS,
                            ));
                        }
                    })
                    .ok();
                    continue;
                }
                invoke_from_event_loop(move || {
                    if let Some(w) = window.upgrade() {
                        w.set_empty_clean_count_text(SharedString::from(format!(
                            "{count} {}",
                            strings::EMPTY_FOLDERS
                        )));
                        w.set_confirm_empty_clean(true);
                    }
                })
                .ok();
                // The scan summary has already terminal-ed for this row; the actual
                // removal is a new operation, started only on an explicit yes.
                if clean_ui_rx.recv().await.unwrap_or(false) {
                    orch.lock()
                        .expect("orchestrator")
                        .start_clean_from_report(root, directories);
                }
            }
            OpEvent::Conflict {
                id,
                request,
                answer_tx,
            } => {
                tracing::info!(
                    id = id.0,
                    source = %request.source.display(),
                    destination = %request.destination.display(),
                    "conflict: showing resolution dialog"
                );
                let state = crate::conflict_dialog::dialog_state_for(&request);
                invoke_from_event_loop(move || {
                    if let Some(w) = window.upgrade() {
                        w.set_conflict_file_name(SharedString::from(state.file_name));
                        w.set_conflict_source_path(SharedString::from(state.source_path));
                        w.set_conflict_source_size(SharedString::from(state.source_size));
                        w.set_conflict_source_modified(SharedString::from(state.source_modified));
                        w.set_conflict_destination_path(SharedString::from(state.destination_path));
                        w.set_conflict_destination_size(SharedString::from(state.destination_size));
                        w.set_conflict_destination_modified(SharedString::from(
                            state.destination_modified,
                        ));
                        w.set_show_conflict_dialog(true);
                    }
                })
                .ok();
                // The worker stays paused in the resolver until the dialog answers.
                // A closed channel (window gone) answers as cancel, never a default.
                let answer = match conflict_ui_rx.recv().await {
                    Some((choice, apply_to_all)) => {
                        crate::conflict_dialog::choice_from(&choice, apply_to_all)
                    }
                    None => None,
                };
                let _ = answer_tx.send(answer);
            }
        }
    }
}

/// Terminal row state from the engine summary; per-item failures surface as
/// warnings even when the engine's own state stayed "completed".
fn row_state_of(summary: &fx_ops::progress::OperationSummary) -> crate::activity::RowState {
    use crate::activity::RowState;
    use fx_ops::progress::OperationState as S;
    match summary.state {
        S::Cancelled => RowState::Cancelled,
        S::Failed => RowState::Failed,
        S::CompletedWithWarnings => RowState::CompletedWithWarnings,
        _ if !summary.failures.is_empty() => RowState::CompletedWithWarnings,
        _ => RowState::Completed,
    }
}

fn failure_lists(summary: &fx_ops::progress::OperationSummary) -> (Vec<String>, Vec<PathBuf>) {
    let mut errors: Vec<String> = summary
        .failures
        .iter()
        .map(|f| format!("{}: {}", f.path.display(), f.error))
        .collect();
    if let Some(err) = &summary.error {
        errors.insert(0, err.to_string());
    }
    let failed = summary.failures.iter().map(|f| f.path.clone()).collect();
    (errors, failed)
}

/// Accessible stand-in for the progress bar (spec §9): Slint 1.18 exposes no
/// progress-bar role, so the row carries "42 percent, 3 of 7 files" as text.
fn progress_text_of(row: &crate::activity::ActivityRow) -> String {
    let pct = if row.bytes_total > 0 {
        ((row.bytes_done as f64 * 100.0) / row.bytes_total as f64).round() as u64
    } else if row.files_total > 0 {
        (row.files_done * 100 / row.files_total).min(100)
    } else {
        0
    };
    format!(
        "{pct} {}, {} of {} {}",
        strings::PROGRESS_PERCENT,
        row.files_done,
        row.files_total,
        strings::PROGRESS_FILES
    )
}

/// Spec §6: after completion, refresh the current folder when it is still the
/// folder the operation touched.
fn finish_refresh(orch: &Arc<Mutex<Orchestrator>>) {
    let target = orch
        .lock()
        .expect("orchestrator")
        .refresh_target
        .lock()
        .expect("refresh slot")
        .take();
    let Some(dir) = target else { return };
    let same = {
        let guard = orch.lock().expect("orchestrator");
        matches!(&guard.history.current().location, Location::Local(p) if *p == dir)
    };
    if same {
        orch.lock().expect("orchestrator").refresh();
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn ui_row_to_slint(row: fx_ui::viewmodel::UiFileRow) -> fx_ui::FileRow {
    fx_ui::FileRow {
        id: row.id as i32,
        display_name: SharedString::from(row.display_name),
        kind: SharedString::from(row.kind),
        is_folder: row.is_folder,
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

/// Human-readable byte count: "1.2 GB", "345 MB", etc.
pub(crate) fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}
