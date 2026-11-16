//! `OperationRunner` — the bridge between UI commands and the `fx-ops` engine.
//!
//! All operations run in `tokio::task::spawn_blocking`; no blocking call ever runs on the
//! Slint thread. Progress events are coalesced to ~10 Hz; terminal events are never dropped.
//! One operation runs at a time per `OperationRunner`; new commands are queued.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use fx_ops::{
    Progress,
    checksum::ChecksumAlgorithm,
    conflict::{ConflictAnswer, ConflictChoice, ConflictRequest},
    control::OperationControl,
    delete,
    port::PlatformOps,
    progress::{OperationId, OperationSummary},
};
use tokio::sync::{mpsc, oneshot};

const EVENT_CHANNEL_CAPACITY: usize = 64;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

// ── Public event types ─────────────────────────────────────────────────────

pub enum OpEvent {
    Progress {
        id: OperationId,
        progress: Progress,
    },
    /// Which engine performs this transfer, decided by the route gate once the plan is
    /// known (Tranche 3 slice 5). The activity row uses it to hide Pause: `IFileOperation`
    /// has no pause (spec §6).
    RouteChosen {
        id: OperationId,
        native: bool,
    },
    Conflict {
        id: OperationId,
        request: ConflictRequest,
        answer_tx: oneshot::Sender<Option<ConflictAnswer>>,
    },
    Completed {
        id: OperationId,
        summary: OperationSummary,
    },
    Failed {
        id: OperationId,
        summary: OperationSummary,
    },
    ChecksumResults {
        rows: Vec<ChecksumOutcome>,
    },
    /// Report-first empty-folder scan result, awaiting the user's confirmation.
    EmptyFolderReport {
        id: OperationId,
        root: PathBuf,
        directories: Vec<PathBuf>,
    },
}

/// One selected file with its three digests, for the checksum sheet.
#[derive(Clone, Debug)]
pub struct ChecksumOutcome {
    pub path: PathBuf,
    pub md5: String,
    pub sha256: String,
    pub blake3: String,
}

// ── Internal queued command ────────────────────────────────────────────────

struct QueuedOp {
    id: OperationId,
    cmd: QueuedCommand,
}

enum QueuedCommand {
    PasteFromClipboard {
        destination: PathBuf,
        port: Arc<dyn PlatformOps + Send + Sync>,
    },
    /// Explicit copy/move of a known path set (internal drag-and-drop): no
    /// clipboard involved, the caller already resolved the desired effect.
    Transfer {
        sources: Vec<PathBuf>,
        destination: PathBuf,
        kind: fx_ops::plan::TransferKind,
        port: Arc<dyn PlatformOps + Send + Sync>,
    },
    /// Re-run over the items that failed in a previous transfer; the effect is
    /// re-read from the clipboard (spec: retry is a new operation).
    RetryPaste {
        sources: Vec<PathBuf>,
        destination: PathBuf,
        port: Arc<dyn PlatformOps + Send + Sync>,
    },
    DeleteSelection {
        paths: Vec<PathBuf>,
        permanent: bool,
        port: Arc<dyn PlatformOps + Send + Sync>,
    },
    RenameEntry {
        path: PathBuf,
        new_name: String,
        port: Arc<dyn PlatformOps + Send + Sync>,
    },
    CreateFolder {
        parent: PathBuf,
        name: String,
        port: Arc<dyn PlatformOps + Send + Sync>,
    },
    CreateFile {
        parent: PathBuf,
        name: String,
        port: Arc<dyn PlatformOps + Send + Sync>,
    },
    Undo {
        port: Arc<dyn PlatformOps + Send + Sync>,
    },
    Redo {
        port: Arc<dyn PlatformOps + Send + Sync>,
    },
    ComputeChecksums {
        paths: Vec<PathBuf>,
    },
    /// Report-first half of the empty-folder cleaner (OPS-13): scan only, never delete.
    ScanEmptyFolders {
        root: PathBuf,
    },
    /// Deletion half, reached only after the user confirmed the report.
    CleanEmptyFolders {
        root: PathBuf,
        plan: fx_ops::clean::CleanPlan,
    },
}

// ── UndoStack wrapper ─────────────────────────────────────────────────────

pub type UndoStack = fx_ops::undo::UndoStack;

// ── OperationRunner ────────────────────────────────────────────────────────

pub struct OperationRunner {
    core: Arc<RunnerCore>,
}

/// State shared between the UI thread (which submits commands) and the blocking
/// pool (which drains the queue when an operation finishes).
struct RunnerState {
    /// `(id, control)` of the running operation; `None` when idle.
    current: Option<(OperationId, Arc<OperationControl>)>,
    queue: VecDeque<QueuedOp>,
}

struct RunnerCore {
    runtime: tokio::runtime::Handle,
    undo_stack: Arc<Mutex<UndoStack>>,
    event_tx: mpsc::Sender<OpEvent>,
    state: Mutex<RunnerState>,
    /// Tranche 3 slice 5: everything the native route needs.
    native: NativeContext,
}

/// What the native route needs: the capability probed once at startup, the window the
/// Shell's dialog parents to, and whether the product shows that dialog.
///
/// The owner is stored as an address because `HWND` is not `Send` and the transfer runs on
/// a worker thread; `context_menu.rs` carries its owner the same way.
pub struct NativeContext {
    /// The `IFileOperation` capability, probed once by the orchestrator.
    capability: fx_ops::routing::NativeCapability,
    owner: Mutex<Option<isize>>,
    show_progress: bool,
}

impl NativeContext {
    /// A context whose transfers show the Shell's progress dialog — the product mode.
    pub fn new(capability: fx_ops::routing::NativeCapability) -> Self {
        Self {
            capability,
            owner: Mutex::new(None),
            show_progress: true,
        }
    }

    /// Records the window the native dialog parents to. Called on the UI thread once Slint
    /// has materialised the HWND; nothing reads it until a transfer takes the native route.
    pub fn set_owner_window(&self, owner: Option<isize>) {
        *self.owner.lock().expect("native owner") = owner;
    }

    fn owner_window(&self) -> Option<isize> {
        *self.owner.lock().expect("native owner")
    }

    fn show_progress(&self) -> bool {
        self.show_progress
    }
}

impl RunnerCore {
    /// Starts `op` immediately when idle, queues it otherwise (decision 6: one
    /// operation at a time, later commands FIFO).
    fn submit(self: &Arc<Self>, op: QueuedOp) {
        let mut state = self.state.lock().expect("runner state");
        if state.current.is_some() {
            state.queue.push_back(op);
        } else {
            self.start_locked(&mut state, op);
        }
    }

    fn start_locked(self: &Arc<Self>, state: &mut RunnerState, op: QueuedOp) {
        let control = Arc::new(OperationControl::new());
        state.current = Some((op.id, control.clone()));
        let core = self.clone();
        self.runtime.spawn_blocking(move || {
            run_command(op, &control, &core.event_tx, &core.undo_stack, &core.native);
            // Drain under a single lock: pop-and-start must not interleave with a
            // UI submit, or two operations would run at once (decision 6).
            let mut state = core.state.lock().expect("runner state");
            state.current = None;
            if let Some(next) = state.queue.pop_front() {
                core.start_locked(&mut state, next);
            }
        });
    }
}

impl OperationRunner {
    pub fn new(
        runtime: tokio::runtime::Handle,
        native: NativeContext,
    ) -> (Self, mpsc::Receiver<OpEvent>) {
        let (tx, rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
        let core = Arc::new(RunnerCore {
            runtime,
            undo_stack: Arc::new(Mutex::new(UndoStack::new())),
            event_tx: tx,
            state: Mutex::new(RunnerState {
                current: None,
                queue: VecDeque::new(),
            }),
            native,
        });
        (Self { core }, rx)
    }

    /// The window the Shell's progress dialog parents to (Tranche 3 slice 5).
    pub fn set_owner_window(&self, owner: Option<isize>) {
        self.core.native.set_owner_window(owner);
    }

    // ── Control ────────────────────────────────────────────────────────────

    /// Cancels by id. Returns true when the command was still queued and got
    /// dropped before ever running — the caller must then close its activity row,
    /// because no terminal event will arrive for it.
    pub fn cancel_op(&self, id: OperationId) -> bool {
        let mut state = self.core.state.lock().expect("runner state");
        if let Some((cur_id, ctrl)) = &state.current
            && *cur_id == id
        {
            ctrl.cancel();
            return false;
        }
        let before = state.queue.len();
        state.queue.retain(|op| op.id != id);
        before != state.queue.len()
    }

    pub fn pause_op(&self, id: OperationId) {
        let state = self.core.state.lock().expect("runner state");
        if let Some((cur_id, ctrl)) = &state.current
            && *cur_id == id
        {
            ctrl.pause();
        }
    }

    pub fn resume_op(&self, id: OperationId) {
        let state = self.core.state.lock().expect("runner state");
        if let Some((cur_id, ctrl)) = &state.current
            && *cur_id == id
        {
            ctrl.resume();
        }
    }

    // ── Public command methods ─────────────────────────────────────────────
    // Each returns the pre-allocated id so the caller can key its activity row
    // before the worker ever starts.

    /// Reads the clipboard inside the worker and runs a transfer if paths are present.
    pub fn paste_from_clipboard(
        &mut self,
        destination: PathBuf,
        port: Arc<dyn PlatformOps + Send + Sync>,
    ) -> OperationId {
        self.submit(QueuedCommand::PasteFromClipboard { destination, port })
    }

    /// Retry of a failed transfer, over exactly the sources that failed.
    pub fn retry_paste(
        &mut self,
        sources: Vec<PathBuf>,
        destination: PathBuf,
        port: Arc<dyn PlatformOps + Send + Sync>,
    ) -> OperationId {
        self.submit(QueuedCommand::RetryPaste {
            sources,
            destination,
            port,
        })
    }

    pub fn delete_selection(
        &mut self,
        paths: Vec<PathBuf>,
        permanent: bool,
        port: Arc<dyn PlatformOps + Send + Sync>,
    ) -> OperationId {
        self.submit(QueuedCommand::DeleteSelection {
            paths,
            permanent,
            port,
        })
    }

    pub fn rename_entry(
        &mut self,
        path: PathBuf,
        new_name: String,
        port: Arc<dyn PlatformOps + Send + Sync>,
    ) -> OperationId {
        self.submit(QueuedCommand::RenameEntry {
            path,
            new_name,
            port,
        })
    }

    pub fn create_folder(
        &mut self,
        parent: PathBuf,
        name: String,
        port: Arc<dyn PlatformOps + Send + Sync>,
    ) -> OperationId {
        self.submit(QueuedCommand::CreateFolder { parent, name, port })
    }

    pub fn create_file(
        &mut self,
        parent: PathBuf,
        name: String,
        port: Arc<dyn PlatformOps + Send + Sync>,
    ) -> OperationId {
        self.submit(QueuedCommand::CreateFile { parent, name, port })
    }

    pub fn undo(&mut self, port: Arc<dyn PlatformOps + Send + Sync>) -> OperationId {
        self.submit(QueuedCommand::Undo { port })
    }

    pub fn redo(&mut self, port: Arc<dyn PlatformOps + Send + Sync>) -> OperationId {
        self.submit(QueuedCommand::Redo { port })
    }

    pub fn compute_checksums(&mut self, paths: Vec<PathBuf>) -> OperationId {
        self.submit(QueuedCommand::ComputeChecksums { paths })
    }

    /// Explicit DnD-style transfer with the effect already resolved by the caller.
    pub fn transfer_paths(
        &mut self,
        sources: Vec<PathBuf>,
        destination: PathBuf,
        kind: fx_ops::plan::TransferKind,
        port: Arc<dyn PlatformOps + Send + Sync>,
    ) -> OperationId {
        self.submit(QueuedCommand::Transfer {
            sources,
            destination,
            kind,
            port,
        })
    }

    /// Scan-only half of the cleaner; its report gates the confirmation.
    pub fn scan_empty_folders(&mut self, root: PathBuf) -> OperationId {
        self.submit(QueuedCommand::ScanEmptyFolders { root })
    }

    pub fn clean_empty_folders(
        &mut self,
        root: PathBuf,
        plan: fx_ops::clean::CleanPlan,
    ) -> OperationId {
        self.submit(QueuedCommand::CleanEmptyFolders { root, plan })
    }

    // ── Internal dispatch ──────────────────────────────────────────────────

    fn submit(&mut self, cmd: QueuedCommand) -> OperationId {
        let id = OperationId::next();
        self.core.submit(QueuedOp { id, cmd });
        id
    }
}

fn run_command(
    op: QueuedOp,
    control: &OperationControl,
    tx: &mpsc::Sender<OpEvent>,
    undo_stack: &Mutex<UndoStack>,
    native: &NativeContext,
) {
    let id = op.id;
    match op.cmd {
        QueuedCommand::PasteFromClipboard { destination, port } => {
            let ctx = RunCtx {
                port: port.as_ref(),
                control,
                tx,
                undo_stack,
                native,
            };
            run_paste(id, destination, &ctx);
        }
        QueuedCommand::Transfer {
            sources,
            destination,
            kind,
            port,
        } => {
            run_transfer_kind(
                id,
                kind,
                &sources,
                &destination,
                &RunCtx {
                    port: port.as_ref(),
                    control,
                    tx,
                    undo_stack,
                    native,
                },
            );
        }
        QueuedCommand::RetryPaste {
            sources,
            destination,
            port,
        } => {
            let ctx = RunCtx {
                port: port.as_ref(),
                control,
                tx,
                undo_stack,
                native,
            };
            run_retry_paste(id, sources, destination, &ctx);
        }
        QueuedCommand::DeleteSelection {
            paths,
            permanent,
            port,
        } => {
            run_delete(id, paths, permanent, port.as_ref(), control, tx, undo_stack);
        }
        QueuedCommand::RenameEntry {
            path,
            new_name,
            port,
        } => {
            run_rename(id, path, new_name, port.as_ref(), control, tx, undo_stack);
        }
        QueuedCommand::CreateFolder { parent, name, port } => {
            run_create_folder(id, parent, name, port.as_ref(), control, tx, undo_stack);
        }
        QueuedCommand::CreateFile { parent, name, port } => {
            run_create_file(id, parent, name, port.as_ref(), control, tx, undo_stack);
        }
        QueuedCommand::Undo { port } => {
            run_undo(id, port.as_ref(), control, tx, undo_stack);
        }
        QueuedCommand::Redo { port } => {
            run_redo(id, port.as_ref(), control, tx, undo_stack);
        }
        QueuedCommand::ComputeChecksums { paths } => {
            run_checksums(id, paths, control, tx);
        }
        QueuedCommand::ScanEmptyFolders { root } => {
            run_scan(id, root, control, tx);
        }
        QueuedCommand::CleanEmptyFolders { root, plan } => {
            run_clean(id, root, &plan, control, tx);
        }
    }
}

// ── Individual operation implementations ──────────────────────────────────

fn run_paste(id: OperationId, destination: PathBuf, ctx: &RunCtx<'_>) {
    // Read the clipboard inside the worker, not on the Slint thread.
    let clipboard_data = match fx_shell::clipboard::read_clipboard() {
        Ok(Some(data)) => data,
        Ok(None) | Err(_) => {
            let summary =
                fx_ops::progress::OperationSummary::new(id, fx_ops::progress::OperationKind::Copy)
                    .fail_with(fx_ops::error::OperationError::NotFound);
            let _ = ctx.tx.blocking_send(OpEvent::Failed { id, summary });
            return;
        }
    };

    let (paths, effect) = clipboard_data;
    run_transfer(id, effect, &paths, &destination, ctx);
}

/// Retry of a failed transfer over exactly the sources that failed (spec: retry
/// starts a new operation). The effect is re-read from the clipboard; if the
/// clipboard no longer carries files, the retry runs as a plain copy.
fn run_retry_paste(id: OperationId, sources: Vec<PathBuf>, destination: PathBuf, ctx: &RunCtx<'_>) {
    let effect = fx_shell::clipboard::read_clipboard()
        .ok()
        .flatten()
        .map(|(_, effect)| effect)
        .unwrap_or(fx_shell::clipboard::ClipboardEffect::Copy);
    run_transfer(id, effect, &sources, &destination, ctx);
}

/// The services every worker function shares; bundling them keeps signatures short.
struct RunCtx<'a> {
    port: &'a dyn PlatformOps,
    control: &'a OperationControl,
    tx: &'a mpsc::Sender<OpEvent>,
    undo_stack: &'a Mutex<UndoStack>,
    native: &'a NativeContext,
}

fn run_transfer(
    id: OperationId,
    effect: fx_shell::clipboard::ClipboardEffect,
    paths: &[PathBuf],
    destination: &Path,
    ctx: &RunCtx<'_>,
) {
    let kind = match effect {
        fx_shell::clipboard::ClipboardEffect::Copy => fx_ops::plan::TransferKind::Copy,
        fx_shell::clipboard::ClipboardEffect::Move => fx_ops::plan::TransferKind::Move,
    };
    let summary = run_transfer_kind(id, kind, paths, destination, ctx);
    if should_clear_clipboard(kind, &summary)
        && let Err(err) = fx_shell::clipboard::clear_clipboard()
    {
        tracing::warn!(%err, "could not clear the clipboard after a move-paste");
    }
}

/// A move-paste consumes the cut only when every item landed.
fn should_clear_clipboard(kind: fx_ops::plan::TransferKind, summary: &OperationSummary) -> bool {
    kind == fx_ops::plan::TransferKind::Move
        && summary.state == fx_ops::progress::OperationState::Completed
}

fn run_transfer_kind(
    id: OperationId,
    kind: fx_ops::plan::TransferKind,
    paths: &[PathBuf],
    destination: &Path,
    ctx: &RunCtx<'_>,
) -> OperationSummary {
    let plan = match fx_ops::plan::plan_transfer(kind, paths, destination) {
        Ok(p) => p,
        Err(err) => {
            let summary =
                fx_ops::progress::OperationSummary::new(id, fx_ops::progress::OperationKind::Copy)
                    .fail_with(err);
            let _ = ctx.tx.blocking_send(OpEvent::Failed {
                id,
                summary: summary.clone(),
            });
            return summary;
        }
    };

    // Decision 1: the gate runs on the plan, before anything touches the disk. It is
    // reported so the activity row can hide Pause, which the native route cannot honour.
    let route = route_for(&plan, ctx.native.capability);
    let _ = ctx.tx.blocking_send(OpEvent::RouteChosen {
        id,
        native: route == fx_ops::routing::TransferRoute::Native,
    });
    tracing::info!(
        target: "transfer",
        operation = id.0,
        route = route_label(route),
        kind = ?plan.kind,
        items = plan.items.len(),
        "transfer route"
    );

    match route {
        fx_ops::routing::TransferRoute::Native => run_native_transfer(id, &plan, ctx),
        fx_ops::routing::TransferRoute::BuiltIn => run_builtin_transfer(id, kind, &plan, ctx),
    }
}

/// The route gate: native only for a clean copy or move, and only when the Shell's
/// operation object exists here.
///
/// A destination probe that fails (a permission error on the destination folder, say) is
/// not a reason to fail the operation: it is a reason to run the engine that handles its
/// own conflicts, which is the fallback `PRD.md:112` asks for.
fn route_for(
    plan: &fx_ops::plan::OperationPlan,
    capability: fx_ops::routing::NativeCapability,
) -> fx_ops::routing::TransferRoute {
    match fx_ops::routing::scan_destination(plan) {
        Ok(scan) => fx_ops::routing::choose_route(&scan, plan.kind, capability),
        Err(error) => {
            tracing::warn!(%error, "destination probe failed; using the built-in engine");
            fx_ops::routing::TransferRoute::BuiltIn
        }
    }
}

/// The route name the log carries (spec §8.3).
fn route_label(route: fx_ops::routing::TransferRoute) -> &'static str {
    match route {
        fx_ops::routing::TransferRoute::Native => "native",
        fx_ops::routing::TransferRoute::BuiltIn => "built_in",
    }
}

/// The Tranche 2 engine: conflict dialog, four choices, pause, checksum-free copy.
fn run_builtin_transfer(
    id: OperationId,
    kind: fx_ops::plan::TransferKind,
    plan: &fx_ops::plan::OperationPlan,
    ctx: &RunCtx<'_>,
) -> OperationSummary {
    let mut last_emit = Instant::now();
    let tx_progress = ctx.tx.clone();
    let mut on_progress = move |p: &fx_ops::Progress| {
        if last_emit.elapsed() >= PROGRESS_INTERVAL {
            last_emit = Instant::now();
            let _ = tx_progress.blocking_send(OpEvent::Progress {
                id,
                progress: p.clone(),
            });
        }
    };

    // Build a conflict resolver that sends conflict events and waits for answers.
    let tx_conflict = ctx.tx.clone();
    let mut resolver = ChannelConflictResolver {
        id,
        tx: tx_conflict,
    };

    let mut transfer = fx_ops::transfer::Transfer {
        id,
        port: ctx.port,
        control: ctx.control,
        resolver: &mut resolver,
        on_progress: &mut on_progress,
    };

    let summary = transfer.run(plan);
    let is_failed = summary.state == fx_ops::progress::OperationState::Failed;

    // Record undo entry for moves, through the derivation both routes share (decision 3).
    if kind == fx_ops::plan::TransferKind::Move && !is_failed {
        let pairs: Vec<fx_ops::plan::PathPair> = plan
            .items
            .iter()
            .map(|item| fx_ops::plan::PathPair {
                from: item.source.clone(),
                to: item.destination.clone(),
            })
            .collect();
        ctx.undo_stack
            .lock()
            .expect("undo lock")
            .record(fx_ops::undo::move_entry(id, pairs));
    }

    let event = if is_failed {
        OpEvent::Failed {
            id,
            summary: summary.clone(),
        }
    } else {
        OpEvent::Completed {
            id,
            summary: summary.clone(),
        }
    };
    let _ = ctx.tx.blocking_send(event);
    summary
}

/// The Shell's own route: `IFileOperation` with its progress dialog (slice 5).
///
/// The worker thread owns COM; this function owns the bookkeeping. Every state shown to the
/// user is derived here from three facts: the events the sink reported, the on-disk
/// post-condition (rule 13), and whether the user cancelled — never from the Shell's own
/// "it worked" verdict.
fn run_native_transfer(
    id: OperationId,
    plan: &fx_ops::plan::OperationPlan,
    ctx: &RunCtx<'_>,
) -> OperationSummary {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::RecvTimeoutError;

    let request = fx_shell::NativeTransferRequest {
        kind: match plan.kind {
            fx_ops::progress::OperationKind::Move => fx_shell::NativeKind::Move,
            _ => fx_shell::NativeKind::Copy,
        },
        sources: plan.items.iter().map(|item| item.source.clone()).collect(),
        destination: plan.destination_root.clone(),
        owner: ctx.native.owner_window(),
        show_progress: ctx.native.show_progress(),
    };

    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let handle = fx_shell::run_native_transfer(request, event_tx, std::sync::Arc::clone(&cancel));

    let mut pairs: Vec<fx_ops::plan::PathPair> = Vec::new();
    let mut failures: Vec<fx_ops::error::ItemFailure> = Vec::new();
    let mut shell_files_done: u64 = 0;
    let mut bytes_done: u64 = 0;
    let mut current: Option<PathBuf> = None;
    let mut last_emit = Instant::now();
    let mut terminal: Option<(bool, Option<fx_shell::NativeError>)> = None;

    loop {
        // The UI thread cancels by flipping the operation's token; the sink learns about it
        // through this flag, because it cannot see the token from fx-shell.
        if ctx.control.is_cancelled() {
            cancel.store(true, Ordering::Relaxed);
        }
        match event_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(fx_shell::NativeEvent::Started) => {}
            Ok(fx_shell::NativeEvent::Progress {
                files_done,
                bytes_done: bytes,
                current: path,
            }) => {
                if files_done > 0 {
                    shell_files_done = shell_files_done.max(files_done);
                }
                bytes_done = bytes_done.max(bytes);
                if path.is_some() {
                    current = path;
                }
                if last_emit.elapsed() >= PROGRESS_INTERVAL {
                    last_emit = Instant::now();
                    let progress = fx_ops::Progress {
                        files_total: plan.files_total,
                        files_done: shell_files_done.max(pairs.len() as u64),
                        bytes_total: plan.bytes_total,
                        bytes_done,
                        current: current.clone(),
                        ..Default::default()
                    };
                    let _ = ctx.tx.blocking_send(OpEvent::Progress { id, progress });
                }
            }
            Ok(fx_shell::NativeEvent::ItemDone {
                source,
                destination,
            }) => pairs.push(fx_ops::plan::PathPair {
                from: source,
                to: destination,
            }),
            Ok(fx_shell::NativeEvent::ItemFailed { path, code }) => {
                failures.push(fx_ops::error::ItemFailure::new(path, native_failure(code)))
            }
            Ok(fx_shell::NativeEvent::Finished { aborted, error }) => {
                terminal = Some((aborted, error));
                break;
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    handle.join();

    // A worker that vanished without a terminal event is a failure of the route, not a
    // silent success.
    let (aborted, error) = terminal.unwrap_or_else(|| {
        (
            ctx.control.is_cancelled(),
            Some(fx_shell::NativeError {
                code: 0,
                message: "the native transfer ended without reporting a result".to_owned(),
            }),
        )
    });
    // Whether this counts as a cancellation is derived, not copied from the Shell's flag:
    // it sets that flag when it skips a failed item too (measured on this host — a locked
    // source aborts while reporting the item failure).
    let failed_items = failures.len();
    let cancelled = native_was_cancelled(
        ctx.control.is_cancelled(),
        aborted,
        failed_items,
        error.is_some(),
    );

    let mut summary = fx_ops::progress::OperationSummary::new(id, plan.kind);
    summary.files_done = pairs.len() as u64;
    for failure in failures {
        summary.record_failure(failure);
    }

    if !cancelled {
        if let Some(error) = &error {
            tracing::warn!(%error, "native transfer failed");
            summary = summary.fail_with(fx_ops::error::OperationError::Io {
                kind: std::io::ErrorKind::Other,
                message: error.to_string(),
            });
        }
        // Rule 13: what is on disk decides. A destination the Shell claimed and did not
        // produce is a warning, never a success.
        for path in fx_ops::routing::verify_destinations(plan.kind, plan) {
            summary.record_failure(fx_ops::error::ItemFailure::new(
                path,
                fx_ops::error::OperationError::VerificationFailed,
            ));
        }
        // Decision 9: the metadata post-pass keeps the Tranche 2 parity guarantee without
        // assuming what the Shell preserves. Copies only — a move's source is gone.
        for path in fx_ops::routing::apply_native_metadata(ctx.port, plan.kind, &pairs) {
            tracing::warn!(
                path = %path.display(),
                "metadata post-pass failed after a native copy"
            );
        }
    }

    // Decision 3: undo is derived from the pairs the sink observed, through the same helper
    // the built-in engine uses.
    if plan.kind == fx_ops::progress::OperationKind::Move
        && !cancelled
        && summary.error.is_none()
        && !pairs.is_empty()
    {
        ctx.undo_stack
            .lock()
            .expect("undo lock")
            .record(fx_ops::undo::move_entry(id, pairs));
    }

    let summary = summary.finish(cancelled);
    let is_failed = summary.state == fx_ops::progress::OperationState::Failed;
    let event = if is_failed {
        OpEvent::Failed {
            id,
            summary: summary.clone(),
        }
    } else {
        OpEvent::Completed {
            id,
            summary: summary.clone(),
        }
    };
    let _ = ctx.tx.blocking_send(event);
    summary
}

/// Whether a finished native transfer is a cancellation rather than a result.
///
/// A cancel the user requested always wins. Without one, the Shell's abort flag only means
/// "the operation stopped early": when items failed, the run is a partial result that must
/// be reported as such, not hidden behind `Cancelled`.
fn native_was_cancelled(requested: bool, aborted: bool, failed_items: usize, failed: bool) -> bool {
    requested || (aborted && failed_items == 0 && !failed)
}

/// Maps the Shell's HRESULT for one item to the typed failure the activity center shows.
///
/// The win32 codes are the ones a copy actually produces; anything else keeps its HRESULT in
/// the message instead of being flattened into a vague "I/O error".
fn native_failure(code: i32) -> fx_ops::error::OperationError {
    use fx_ops::error::OperationError;
    match code as u32 {
        0x8007_0002 | 0x8007_0003 => OperationError::NotFound,
        0x8007_0005 => OperationError::AccessDenied,
        0x8007_0020 | 0x8007_0021 => OperationError::Locked,
        0x8007_0050 => OperationError::AlreadyExists,
        0x8007_00CE => OperationError::PathTooLong,
        other => OperationError::Io {
            kind: std::io::ErrorKind::Other,
            message: format!("HRESULT 0x{other:08X}"),
        },
    }
}

fn run_delete(
    id: OperationId,
    paths: Vec<PathBuf>,
    permanent: bool,
    port: &dyn PlatformOps,
    control: &OperationControl,
    tx: &mpsc::Sender<OpEvent>,
    undo_stack: &Mutex<UndoStack>,
) {
    if permanent {
        let summary = delete::delete_permanent(id, &paths, control);
        let is_failed = summary.state == fx_ops::progress::OperationState::Failed;
        let event = if is_failed {
            OpEvent::Failed { id, summary }
        } else {
            OpEvent::Completed { id, summary }
        };
        let _ = tx.blocking_send(event);
    } else {
        match delete::delete_to_recycle(&paths, port) {
            Ok(recycled) => {
                // Record undo entry.
                let items: Vec<fx_ops::port::RecycledItem> = recycled
                    .into_iter()
                    .map(|item| fx_ops::port::RecycledItem {
                        original: item.original,
                        recycled_name: item.recycled_name,
                        verified: item.verified,
                    })
                    .collect();
                undo_stack
                    .lock()
                    .expect("undo lock")
                    .record(fx_ops::undo::UndoEntry {
                        operation: id,
                        kind: fx_ops::progress::OperationKind::Recycle,
                        action: fx_ops::undo::ReversibleAction::Recycle { items },
                    });

                let mut summary = fx_ops::progress::OperationSummary::new(
                    id,
                    fx_ops::progress::OperationKind::Recycle,
                );
                summary.files_done = paths.len() as u64;
                let summary = summary.finish(false);
                let _ = tx.blocking_send(OpEvent::Completed { id, summary });
            }
            Err(err) => {
                tracing::warn!("recycle failed: {err}");
                let summary = fx_ops::progress::OperationSummary::new(
                    id,
                    fx_ops::progress::OperationKind::Recycle,
                )
                .fail_with(err);
                let _ = tx.blocking_send(OpEvent::Failed { id, summary });
            }
        }
    }
}

fn run_rename(
    id: OperationId,
    path: PathBuf,
    new_name: String,
    _port: &dyn PlatformOps,
    _control: &OperationControl,
    tx: &mpsc::Sender<OpEvent>,
    undo_stack: &Mutex<UndoStack>,
) {
    let original = path.clone();

    match fx_ops::rename::rename_path(&path, &new_name) {
        Ok(new_path) => {
            undo_stack
                .lock()
                .expect("undo lock")
                .record(fx_ops::undo::UndoEntry {
                    operation: id,
                    kind: fx_ops::progress::OperationKind::Rename,
                    action: fx_ops::undo::ReversibleAction::Rename {
                        pairs: vec![fx_ops::plan::PathPair {
                            from: original,
                            to: new_path,
                        }],
                    },
                });
            let mut summary = fx_ops::progress::OperationSummary::new(
                id,
                fx_ops::progress::OperationKind::Rename,
            );
            summary.files_done = 1;
            let summary = summary.finish(false);
            let _ = tx.blocking_send(OpEvent::Completed { id, summary });
        }
        Err(err) => {
            let summary = fx_ops::progress::OperationSummary::new(
                id,
                fx_ops::progress::OperationKind::Rename,
            )
            .fail_with(err);
            let _ = tx.blocking_send(OpEvent::Failed { id, summary });
        }
    }
}

fn run_create_folder(
    id: OperationId,
    parent: PathBuf,
    name: String,
    _port: &dyn PlatformOps,
    _control: &OperationControl,
    tx: &mpsc::Sender<OpEvent>,
    undo_stack: &Mutex<UndoStack>,
) {
    match fx_ops::create::create_directory(&parent, &name) {
        Ok(created) => {
            undo_stack
                .lock()
                .expect("undo lock")
                .record(fx_ops::undo::UndoEntry {
                    operation: id,
                    kind: fx_ops::progress::OperationKind::CreateDirectory,
                    action: fx_ops::undo::ReversibleAction::CreateDirectory {
                        paths: vec![created],
                    },
                });
            let mut summary = fx_ops::progress::OperationSummary::new(
                id,
                fx_ops::progress::OperationKind::CreateDirectory,
            );
            summary.files_done = 1;
            let summary = summary.finish(false);
            let _ = tx.blocking_send(OpEvent::Completed { id, summary });
        }
        Err(err) => {
            let summary = fx_ops::progress::OperationSummary::new(
                id,
                fx_ops::progress::OperationKind::CreateDirectory,
            )
            .fail_with(err);
            let _ = tx.blocking_send(OpEvent::Failed { id, summary });
        }
    }
}

fn run_create_file(
    id: OperationId,
    parent: PathBuf,
    name: String,
    _port: &dyn PlatformOps,
    _control: &OperationControl,
    tx: &mpsc::Sender<OpEvent>,
    undo_stack: &Mutex<UndoStack>,
) {
    match fx_ops::create::create_file(&parent, &name) {
        Ok(created) => {
            undo_stack
                .lock()
                .expect("undo lock")
                .record(fx_ops::undo::UndoEntry {
                    operation: id,
                    kind: fx_ops::progress::OperationKind::CreateFile,
                    action: fx_ops::undo::ReversibleAction::CreateFile {
                        paths: vec![created],
                    },
                });
            let mut summary = fx_ops::progress::OperationSummary::new(
                id,
                fx_ops::progress::OperationKind::CreateFile,
            );
            summary.files_done = 1;
            let summary = summary.finish(false);
            let _ = tx.blocking_send(OpEvent::Completed { id, summary });
        }
        Err(err) => {
            let summary = fx_ops::progress::OperationSummary::new(
                id,
                fx_ops::progress::OperationKind::CreateFile,
            )
            .fail_with(err);
            let _ = tx.blocking_send(OpEvent::Failed { id, summary });
        }
    }
}

fn run_undo(
    id: OperationId,
    port: &dyn PlatformOps,
    control: &OperationControl,
    tx: &mpsc::Sender<OpEvent>,
    undo_stack: &Mutex<UndoStack>,
) {
    let result = undo_stack.lock().expect("undo lock").undo(port, control);
    match result {
        Ok(entry) => {
            let mut summary = fx_ops::progress::OperationSummary::new(id, entry.kind);
            summary.files_done = 1;
            let summary = summary.finish(false);
            let _ = tx.blocking_send(OpEvent::Completed { id, summary });
        }
        Err(err) => {
            tracing::warn!("undo failed: {err:?}");
            let summary = fx_ops::progress::OperationSummary::new(
                id,
                fx_ops::progress::OperationKind::Rename,
            )
            .fail_with(fx_ops::error::OperationError::Io {
                kind: std::io::ErrorKind::Other,
                message: format!("{err:?}"),
            });
            let _ = tx.blocking_send(OpEvent::Failed { id, summary });
        }
    }
}

fn run_redo(
    id: OperationId,
    port: &dyn PlatformOps,
    control: &OperationControl,
    tx: &mpsc::Sender<OpEvent>,
    undo_stack: &Mutex<UndoStack>,
) {
    let result = undo_stack.lock().expect("undo lock").redo(port, control);
    match result {
        Ok(entry) => {
            let mut summary = fx_ops::progress::OperationSummary::new(id, entry.kind);
            summary.files_done = 1;
            let summary = summary.finish(false);
            let _ = tx.blocking_send(OpEvent::Completed { id, summary });
        }
        Err(err) => {
            tracing::warn!("redo failed: {err:?}");
            let summary = fx_ops::progress::OperationSummary::new(
                id,
                fx_ops::progress::OperationKind::Rename,
            )
            .fail_with(fx_ops::error::OperationError::Io {
                kind: std::io::ErrorKind::Other,
                message: format!("{err:?}"),
            });
            let _ = tx.blocking_send(OpEvent::Failed { id, summary });
        }
    }
}

fn run_checksums(
    id: OperationId,
    paths: Vec<PathBuf>,
    control: &OperationControl,
    tx: &mpsc::Sender<OpEvent>,
) {
    let mut summary =
        fx_ops::progress::OperationSummary::new(id, fx_ops::progress::OperationKind::Checksum);
    let total = paths.len() as u64;

    let mut outcomes: Vec<ChecksumOutcome> = Vec::with_capacity(paths.len());
    let mut last_emit = Instant::now();
    let mut cancelled = false;

    for path in &paths {
        // One checkpoint per file: cancellation is honored between files; the
        // per-chunk checks happen inside `hash_file`.
        if control.checkpoint().is_err() {
            cancelled = true;
            break;
        }
        let mut digests: [String; 3] = Default::default();
        let mut failure: Option<fx_ops::error::OperationError> = None;
        for (i, algorithm) in ChecksumAlgorithm::all().iter().enumerate() {
            match fx_ops::checksum::hash_file(path, *algorithm, control, |_| {}) {
                Ok(result) => digests[i] = result.hex,
                Err(err) => {
                    failure = Some(err);
                    break;
                }
            }
        }
        match failure {
            None => {
                outcomes.push(ChecksumOutcome {
                    path: path.clone(),
                    md5: std::mem::take(&mut digests[0]),
                    sha256: std::mem::take(&mut digests[1]),
                    blake3: std::mem::take(&mut digests[2]),
                });
                summary.files_done += 1;
            }
            Some(err) => summary.record_failure(fx_ops::error::ItemFailure::new(path.clone(), err)),
        }
        if last_emit.elapsed() >= PROGRESS_INTERVAL {
            last_emit = Instant::now();
            let progress = fx_ops::Progress {
                files_total: total,
                files_done: summary.files_done,
                ..Default::default()
            };
            let _ = tx.blocking_send(OpEvent::Progress { id, progress });
        }
    }

    let _ = tx.blocking_send(OpEvent::ChecksumResults {
        rows: std::mem::take(&mut outcomes),
    });
    let summary = summary.finish(cancelled);
    let is_failed = summary.state == fx_ops::progress::OperationState::Failed;
    let event = if is_failed {
        OpEvent::Failed { id, summary }
    } else {
        OpEvent::Completed { id, summary }
    };
    let _ = tx.blocking_send(event);
}

/// Scan-only half of the cleaner: emits the report and a completed summary.
/// Nothing is deleted here; the user's confirmation starts the clean command.
fn run_scan(
    id: OperationId,
    root: PathBuf,
    control: &OperationControl,
    tx: &mpsc::Sender<OpEvent>,
) {
    let plan = match fx_ops::clean::scan_empty_dirs(&root, control) {
        Ok(p) => p,
        Err(err) => {
            let summary = fx_ops::progress::OperationSummary::new(
                id,
                fx_ops::progress::OperationKind::CleanEmptyFolders,
            )
            .fail_with(err);
            let _ = tx.blocking_send(OpEvent::Failed { id, summary });
            return;
        }
    };
    let directories = plan.directories.clone();
    let _ = tx.blocking_send(OpEvent::EmptyFolderReport {
        id,
        root: root.clone(),
        directories,
    });
    let mut summary = fx_ops::progress::OperationSummary::new(
        id,
        fx_ops::progress::OperationKind::CleanEmptyFolders,
    );
    summary.files_done = plan.directories.len() as u64;
    let summary = summary.finish(control.is_cancelled());
    let _ = tx.blocking_send(OpEvent::Completed { id, summary });
}

/// Deletion half; runs only on a plan the user already confirmed. `fx_ops::clean`
/// re-checks emptiness item by item and never touches the root.
fn run_clean(
    id: OperationId,
    root: PathBuf,
    plan: &fx_ops::clean::CleanPlan,
    control: &OperationControl,
    tx: &mpsc::Sender<OpEvent>,
) {
    let summary = fx_ops::clean::clean(id, &root, plan, control);
    let is_failed = summary.state == fx_ops::progress::OperationState::Failed;
    let event = if is_failed {
        OpEvent::Failed { id, summary }
    } else {
        OpEvent::Completed { id, summary }
    };
    let _ = tx.blocking_send(event);
}

// ── Conflict resolver that sends channel events ────────────────────────────

struct ChannelConflictResolver {
    id: OperationId,
    tx: mpsc::Sender<OpEvent>,
}

impl fx_ops::conflict::ConflictResolver for ChannelConflictResolver {
    fn resolve(&mut self, request: &ConflictRequest) -> Option<ConflictAnswer> {
        let (answer_tx, answer_rx) = oneshot::channel();
        let event = OpEvent::Conflict {
            id: self.id,
            request: request.clone(),
            answer_tx,
        };
        // Send the conflict event; if the channel is full, skip (auto-answer Skip).
        if self.tx.blocking_send(event).is_err() {
            return Some(ConflictAnswer::once(ConflictChoice::Skip));
        }
        // Block until the UI answers; a dropped answer means the dialog went
        // away, which the engine reads as cancel.
        answer_rx.blocking_recv().unwrap_or_default()
    }
}

#[cfg(test)]
mod conflict_protocol_tests {
    use super::*;
    use fx_ops::conflict::{ConflictClass, ConflictResolver};
    use std::thread;

    fn sample_request() -> ConflictRequest {
        ConflictRequest {
            source: std::path::PathBuf::from(r"C:\src\a.txt"),
            destination: std::path::PathBuf::from(r"C:\dst\a.txt"),
            class: ConflictClass::FileFile,
            source_len: Some(1),
            target_len: Some(1),
            source_modified: None,
            target_modified: None,
            remaining: 1,
        }
    }

    #[test]
    fn resolve_blocks_until_the_ui_answers() {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<OpEvent>(4);
        let responder = thread::spawn(move || {
            let event = event_rx.blocking_recv().expect("conflict event");
            if let OpEvent::Conflict { answer_tx, .. } = event {
                answer_tx
                    .send(Some(ConflictAnswer::once(ConflictChoice::KeepBoth)))
                    .expect("resolver still waiting");
            }
        });
        let mut resolver = ChannelConflictResolver {
            id: OperationId::next(),
            tx: event_tx,
        };
        let answer = resolver.resolve(&sample_request());
        responder.join().unwrap();
        assert_eq!(answer, Some(ConflictAnswer::once(ConflictChoice::KeepBoth)));
    }

    #[test]
    fn a_closed_answer_channel_means_cancel() {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<OpEvent>(4);
        let responder = thread::spawn(move || {
            let event = event_rx.blocking_recv().expect("conflict event");
            if let OpEvent::Conflict { answer_tx, .. } = event {
                drop(answer_tx); // the dialog went away without answering
            }
        });
        let mut resolver = ChannelConflictResolver {
            id: OperationId::next(),
            tx: event_tx,
        };
        assert_eq!(resolver.resolve(&sample_request()), None);
        responder.join().unwrap();
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;
    use fx_ops::port::{RecycleError, RecyclePrecheck, RecycledItem, RestoreError, VolumeKind};
    use std::io;

    struct DummyPort;

    impl PlatformOps for DummyPort {
        fn volume_id(&self, _path: &std::path::Path) -> io::Result<u64> {
            Ok(0)
        }
        fn volume_kind(&self, _path: &std::path::Path) -> io::Result<VolumeKind> {
            Ok(VolumeKind::Fixed)
        }
        fn free_space(&self, _path: &std::path::Path) -> io::Result<u64> {
            Ok(u64::MAX)
        }
        fn replace_file(&self, _r: &std::path::Path, _d: &std::path::Path) -> io::Result<()> {
            Err(io::Error::other("not used"))
        }
        fn can_recycle(&self, _p: &[PathBuf]) -> Result<RecyclePrecheck, RecycleError> {
            Ok(RecyclePrecheck::Possible)
        }
        fn recycle(&self, _p: &[PathBuf]) -> Result<Vec<RecycledItem>, RecycleError> {
            Err(RecycleError {
                code: 1,
                message: "not used".into(),
            })
        }
        fn restore_recycled(&self, _i: &RecycledItem) -> Result<(), RestoreError> {
            Err(RestoreError {
                message: "not used".into(),
            })
        }
    }

    /// Decision 6: back-to-back commands are queued, not parallel — and the queue
    /// must actually drain when the running operation finishes.
    #[test]
    fn queued_commands_run_after_the_current_one_finishes() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .build()
            .expect("runtime");
        let (mut runner, mut rx) = OperationRunner::new(
            rt.handle().clone(),
            NativeContext::new(fx_ops::routing::NativeCapability::default()),
        );
        let port: Arc<dyn PlatformOps + Send + Sync> = Arc::new(DummyPort);

        // Two undo requests, back to back: the first starts, the second is queued.
        runner.undo(port.clone());
        runner.undo(port);

        let mut finished = 0;
        let mut ids = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while finished < 2 && Instant::now() < deadline {
            match rx.try_recv() {
                Ok(OpEvent::Failed { id, .. }) => {
                    finished += 1;
                    ids.push(id.0);
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        assert_eq!(finished, 2, "the second queued command never ran");
        assert_ne!(ids[0], ids[1], "distinct ids for distinct operations");
        drop(rx);
        drop(runner);
        drop(rt);
    }
}

/// Explorer's cut tracking: a move that lands completely empties the clipboard so the
/// stale entry cannot be pasted a second time. Anything less than a clean completion
/// leaves the cut in place for a retry.
#[cfg(test)]
mod native_route_tests {
    use super::*;
    use fx_ops::error::OperationError;

    /// A requested cancel always wins, whatever the item counts say.
    #[test]
    fn a_requested_cancel_is_a_cancellation() {
        assert!(native_was_cancelled(true, true, 0, false));
        assert!(native_was_cancelled(true, false, 3, false));
    }

    /// The Shell's abort flag without a request and without failures is the user cancelling
    /// the Shell's own dialog: a cancellation.
    #[test]
    fn an_abort_without_failures_is_a_cancellation() {
        assert!(native_was_cancelled(false, true, 0, false));
    }

    /// An abort with failures is a partial result, not a cancellation — the failures must
    /// reach the activity center (rule 13).
    #[test]
    fn an_abort_after_a_failed_item_is_not_a_cancellation() {
        assert!(!native_was_cancelled(false, true, 1, false));
        assert!(!native_was_cancelled(false, true, 0, true));
        assert!(!native_was_cancelled(false, false, 0, false));
    }

    /// Win32 codes reach the activity center as typed failures, not as one vague string.
    #[test]
    fn win32_codes_map_to_typed_failures() {
        assert_eq!(
            native_failure(0x8007_0002u32 as i32),
            OperationError::NotFound
        );
        assert_eq!(
            native_failure(0x8007_0005u32 as i32),
            OperationError::AccessDenied
        );
        assert_eq!(
            native_failure(0x8007_0020u32 as i32),
            OperationError::Locked
        );
        assert_eq!(
            native_failure(0x8007_0050u32 as i32),
            OperationError::AlreadyExists
        );
        assert_eq!(
            native_failure(0x8007_00CEu32 as i32),
            OperationError::PathTooLong
        );
    }

    /// An unmapped HRESULT keeps its code, so a report says what actually happened.
    #[test]
    fn an_unmapped_hresult_keeps_its_code() {
        match native_failure(0x8000_4005u32 as i32) {
            OperationError::Io { message, .. } => {
                assert!(
                    message.contains("0x80004005"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("unexpected mapping: {other:?}"),
        }
    }

    #[test]
    fn routes_have_stable_log_labels() {
        assert_eq!(
            route_label(fx_ops::routing::TransferRoute::Native),
            "native"
        );
        assert_eq!(
            route_label(fx_ops::routing::TransferRoute::BuiltIn),
            "built_in"
        );
    }

    /// The owner window is stored as an address so the worker thread can use it, and the
    /// product mode keeps the Shell's dialog.
    #[test]
    fn the_native_context_keeps_the_owner_window() {
        let context = NativeContext::new(fx_ops::routing::NativeCapability {
            file_operation: true,
        });
        assert_eq!(context.owner_window(), None);
        context.set_owner_window(Some(0x1234));
        assert_eq!(context.owner_window(), Some(0x1234));
        assert!(context.show_progress());
    }
}

#[cfg(test)]
mod clipboard_clear_rule_tests {
    use super::should_clear_clipboard;
    use fx_ops::{
        plan::TransferKind,
        progress::{OperationId, OperationKind, OperationState, OperationSummary},
    };

    fn summary(kind: OperationKind, state: OperationState) -> OperationSummary {
        let mut summary = OperationSummary::new(OperationId::next(), kind);
        summary.state = state;
        summary
    }

    #[test]
    fn a_completed_move_clears_the_clipboard() {
        assert!(should_clear_clipboard(
            TransferKind::Move,
            &summary(OperationKind::Move, OperationState::Completed)
        ));
    }

    #[test]
    fn a_copy_never_clears_the_clipboard() {
        assert!(!should_clear_clipboard(
            TransferKind::Copy,
            &summary(OperationKind::Copy, OperationState::Completed)
        ));
    }

    #[test]
    fn a_cancelled_or_partial_move_keeps_the_cut() {
        for state in [
            OperationState::Cancelled,
            OperationState::CompletedWithWarnings,
            OperationState::Failed,
        ] {
            assert!(
                !should_clear_clipboard(TransferKind::Move, &summary(OperationKind::Move, state)),
                "{state:?} must leave the clipboard intact"
            );
        }
    }
}
