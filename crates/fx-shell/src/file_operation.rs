//! `IFileOperation`: the Shell's own copy/move route, with its progress dialog — Tranche 3
//! slice 5.
//!
//! Decision 1 routes here only for a clean copy or move; everything else stays on the
//! Tranche 2 engine. What this module adds is the thing the tranche exists for: the native
//! progress dialog (`PRD.md:961`, `PRD.md:963`), with the owner window so it parents to
//! Ferrix.
//!
//! Threading (spec §6): `PerformOperations` runs on a dedicated STA worker with an explicit
//! `CoInitializeEx` guard. The sink callbacks translate to [`NativeEvent`] messages and
//! return immediately, so the Shell's own dialog pump is never blocked by the application.
//! Nothing here touches the Slint thread.
//!
//! Cancellation is the sink-side abort the slice-0 spike exercised: a callback that returns
//! `E_ABORT` stops the operation. That path has **not** been validated on real Windows (the
//! spike was blocked by this workstation's security layer, see
//! `docs/tranche-3-validation.md`), so [`run_native_transfer`] also accepts a flag the
//! caller can raise; until the manual row runs, cancellation on the native route is
//! recorded as unvalidated rather than claimed (CLAUDE.md rule 8).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::Sender,
    },
    thread::JoinHandle,
};

use windows::{
    Win32::{
        Foundation::{E_ABORT, HWND},
        System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree},
        UI::Shell::{
            FILEOPERATION_FLAGS, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, IFileOperation,
            IFileOperationProgressSink, IFileOperationProgressSink_Impl, IShellItem,
            SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
        },
    },
    core::{GUID, HRESULT, PCWSTR, Ref, Result as WinResult, implement},
};

use crate::drop_source::Apartment;

/// `CLSID_FileOperation` from `shobjidl.h`; the windows crate does not export it
/// (`docs/tranche-3-validation.md`, spike findings).
const CLSID_FILE_OPERATION: GUID = GUID::from_values(
    0x3AD05575,
    0x8857,
    0x4850,
    [0x92, 0x77, 0x11, 0xB8, 0x5B, 0xDB, 0x8E, 0x09],
);

/// Which way the Shell moves the sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeKind {
    Copy,
    Move,
}

/// One native transfer, as the route gate approved it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeTransferRequest {
    /// Copy or move; the gate admits nothing else.
    pub kind: NativeKind,
    /// The planned sources, top level only. Undo pairs are reported for these.
    pub sources: Vec<PathBuf>,
    /// The folder the sources land in; it must already exist (`plan_transfer` checked).
    pub destination: PathBuf,
    /// The owner window the Shell parents its dialog to, carried as an address because
    /// `HWND` is not `Send` (the same shape `context_menu.rs` uses). `None` for a headless
    /// run, which is what the conformance tests use.
    pub owner: Option<isize>,
    /// Show the Shell's own progress dialog and its mid-run confirmations.
    ///
    /// The product sets this; a test sets it off so a runner without an interactive desktop
    /// can never block on a modal window. It maps to `FOF_SILENT | FOF_NOCONFIRMATION`,
    /// which is the only combination the conformance suite exercises.
    pub show_progress: bool,
}

/// What the worker reports while the Shell works.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeEvent {
    /// The operation started.
    Started,
    /// Progress from the sink. `current` is `None` when the Shell only reports counts.
    Progress {
        files_done: u64,
        bytes_done: u64,
        current: Option<PathBuf>,
    },
    /// One planned source landed, with the endpoint the Shell actually produced.
    ///
    /// The pair is what undo needs, and it is why the sink is told which sources are top
    /// level: a copied folder reports each descendant too, and only the top-level pair
    /// belongs in the journal (decision 3).
    ItemDone {
        source: PathBuf,
        destination: PathBuf,
    },
    /// One item failed. `code` is the HRESULT the Shell returned for it.
    ItemFailed { path: PathBuf, code: i32 },
    /// Terminal event, always sent: `aborted` is the Shell's own abort flag, `error` the
    /// operation-level failure, if any. The application derives the state from these facts
    /// and from its post-condition check; this crate never asserts a summary (rule 13).
    Finished {
        aborted: bool,
        error: Option<NativeError>,
    },
}

/// A typed native failure: the HRESULT plus the message Windows produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeError {
    pub code: i32,
    pub message: String,
}

impl NativeError {
    fn from_windows(error: &windows::core::Error) -> Self {
        Self {
            code: error.code().0,
            message: error.message().to_string(),
        }
    }
}

impl std::fmt::Display for NativeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HRESULT 0x{:08X}: {}", self.code as u32, self.message)
    }
}

/// Handle on the STA worker, so the caller can wait for it before reading its events.
pub struct NativeHandle {
    worker: Option<JoinHandle<()>>,
}

impl NativeHandle {
    /// Waits for the worker thread to finish. A worker that never started has nothing to
    /// wait for, which is not an error: its `Finished` event already carried the reason.
    pub fn join(self) {
        if let Some(worker) = self.worker {
            let _ = worker.join();
        }
    }
}

/// Runs one native transfer on a dedicated STA worker.
///
/// Returns immediately: every outcome, including a failure to start the worker, arrives on
/// `events` as [`NativeEvent::Finished`], so the caller has exactly one terminal path.
pub fn run_native_transfer(
    request: NativeTransferRequest,
    events: Sender<NativeEvent>,
    cancel: Arc<AtomicBool>,
) -> NativeHandle {
    let worker_events = events.clone();
    let worker = std::thread::Builder::new()
        .name("ferrix-native-transfer".to_owned())
        .spawn(move || {
            let outcome = perform(&request, &worker_events, &cancel);
            let _ = worker_events.send(NativeEvent::Finished {
                aborted: outcome.aborted,
                error: outcome.error,
            });
        });
    match worker {
        Ok(worker) => NativeHandle {
            worker: Some(worker),
        },
        Err(error) => {
            // Reported the same way a COM failure is: one terminal event, no panic on a
            // user path.
            let _ = events.send(NativeEvent::Finished {
                aborted: false,
                error: Some(NativeError {
                    code: 0,
                    message: format!("the transfer worker could not start: {error}"),
                }),
            });
            NativeHandle { worker: None }
        }
    }
}

/// Whether this machine can co-create `IFileOperation`. Probed once, cached.
///
/// This is the boolean `fx-app` feeds to `fx_ops::routing::NativeCapability`; a `false`
/// here keeps every transfer on the Tranche 2 engine (`PRD.md:112`).
pub fn probe_file_operation() -> bool {
    static PROBED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PROBED.get_or_init(|| {
        std::thread::Builder::new()
            .name("ferrix-native-probe".to_owned())
            .spawn(|| {
                let Ok(_apartment) = Apartment::new() else {
                    return false;
                };
                // SAFETY: the CLSID is a compile-time constant from shobjidl.h, this
                // thread has an apartment of its own, and the object is released when it
                // goes out of scope here.
                let created: WinResult<IFileOperation> =
                    unsafe { CoCreateInstance(&CLSID_FILE_OPERATION, None, CLSCTX_ALL) };
                created.is_ok()
            })
            .ok()
            .and_then(|worker| worker.join().ok())
            .unwrap_or(false)
    })
}

// ── Worker ────────────────────────────────────────────────────────────────────

/// What the worker learned, before it becomes a `Finished` event.
struct WorkerOutcome {
    aborted: bool,
    error: Option<NativeError>,
}

impl WorkerOutcome {
    fn ok(aborted: bool) -> Self {
        Self {
            aborted,
            error: None,
        }
    }

    fn failed(error: NativeError) -> Self {
        Self {
            aborted: false,
            error: Some(error),
        }
    }
}

/// The COM half, on the worker thread. Never panics: every failure becomes an outcome.
fn perform(
    request: &NativeTransferRequest,
    events: &Sender<NativeEvent>,
    cancel: &Arc<AtomicBool>,
) -> WorkerOutcome {
    if cancel.load(Ordering::Relaxed) {
        return WorkerOutcome::ok(true);
    }
    let Ok(_apartment) = Apartment::new() else {
        return WorkerOutcome::failed(NativeError {
            code: 0,
            message: "COM could not be initialized on the transfer thread".to_owned(),
        });
    };
    let _ = events.send(NativeEvent::Started);
    match perform_in_apartment(request, events, cancel) {
        Ok(aborted) => WorkerOutcome::ok(aborted),
        Err(error) => WorkerOutcome::failed(NativeError::from_windows(&error)),
    }
}

/// The operation itself. Runs inside the worker's apartment.
fn perform_in_apartment(
    request: &NativeTransferRequest,
    events: &Sender<NativeEvent>,
    cancel: &Arc<AtomicBool>,
) -> WinResult<bool> {
    // SAFETY: the CLSID is a constant, the apartment is live for the whole call, and the
    // object is local to this function.
    let operation: IFileOperation =
        unsafe { CoCreateInstance(&CLSID_FILE_OPERATION, None, CLSCTX_ALL)? };

    // SAFETY: plain flag value; FOF_ADDUNDORECORD is deliberately absent (decision 3).
    unsafe { operation.SetOperationFlags(operation_flags(request.show_progress))? };

    if let Some(owner) = request.owner {
        // SAFETY: the address came from a live window on the UI thread and is only handed
        // back to the Shell as the dialog's parent.
        unsafe { operation.SetOwnerWindow(HWND(owner as *mut std::ffi::c_void))? };
    }

    let parents = Mutex::new(HashMap::new());
    let planned = request
        .sources
        .iter()
        .map(|source| (comparison_key(source, &parents), source.clone()))
        .collect();
    let sink = TransferSink {
        events: events.clone(),
        cancel: Arc::clone(cancel),
        planned,
        parents,
        bytes_done: AtomicU64::new(0),
    };
    let sink_interface: IFileOperationProgressSink = sink.into();
    // SAFETY: the sink is a live interface registered on this operation thread and
    // unadvised on every path below.
    let cookie = unsafe { operation.Advise(&sink_interface)? };

    let destination = shell_item(&request.destination);
    let queue = match destination {
        Ok(destination) => queue_items(&operation, request, &destination, cancel),
        Err(error) => Err(error),
    };
    if let Err(error) = queue {
        // SAFETY: the cookie came from `Advise` on this object.
        unsafe { operation.Unadvise(cookie).ok() };
        return Err(error);
    }

    // SAFETY: the queued items and the sink are live; `PerformOperations` runs the whole
    // operation, pumping the Shell's dialog on this thread when `show_progress` is set.
    let performed = unsafe { operation.PerformOperations() };
    // SAFETY: same object, after the operation returned.
    let aborted = unsafe { operation.GetAnyOperationsAborted() }
        .map(|flag| flag.as_bool())
        .unwrap_or(false);
    // SAFETY: the cookie came from `Advise` on this object.
    unsafe { operation.Unadvise(cookie).ok() };

    match performed {
        Ok(()) => Ok(aborted),
        // An abort surfaces as a failed `PerformOperations`; the flag above says whether
        // the user or our own cancel asked for it, and the caller decides the state.
        Err(_) if aborted => Ok(true),
        Err(error) => Err(error),
    }
}

/// Queues every planned source. Reports the first queueing error, which is an
/// operation-level failure: nothing has started at that point.
fn queue_items(
    operation: &IFileOperation,
    request: &NativeTransferRequest,
    destination: &IShellItem,
    cancel: &Arc<AtomicBool>,
) -> WinResult<()> {
    for source in &request.sources {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let source_item = shell_item(source)?;
        // SAFETY: both shell items are live for the call; a null new name keeps the
        // source's own name, which is what `plan_transfer` computed the destination from.
        unsafe {
            match request.kind {
                NativeKind::Copy => {
                    operation.CopyItem(&source_item, destination, PCWSTR::null(), None)?
                }
                NativeKind::Move => {
                    operation.MoveItem(&source_item, destination, PCWSTR::null(), None)?
                }
            }
        }
    }
    Ok(())
}

// ── The progress sink ─────────────────────────────────────────────────────────

/// A path reduced to a key that tolerates an 8.3 short name anywhere in it.
///
/// The Shell reports long names while the caller can hold a short one:
/// `std::env::temp_dir()` answers `C:\Users\RUNNER~1\...` on the CI runner and the Shell
/// answers `C:\Users\runneradmin\...`, so comparing raw strings lost every undo pair there
/// (measured, CI run 43917693685). Windows paths are case-insensitive, so the key is
/// lowercased, and the parent folder is resolved to its long form because it still exists
/// after a move has taken the item itself away. A folder that cannot be resolved — a
/// deleted source tree — falls back to the path as written, which is still case-insensitive.
fn comparison_key(path: &Path, parents: &Mutex<HashMap<PathBuf, PathBuf>>) -> String {
    let leaf = path
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let parent = path.parent().unwrap_or(path).to_path_buf();
    let long = {
        let mut cache = parents
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache
            .entry(parent.clone())
            .or_insert_with(|| std::fs::canonicalize(&parent).unwrap_or_else(|_| parent.clone()))
            .clone()
    };
    format!("{}\\{leaf}", long.to_string_lossy().to_lowercase())
}

/// Translates the Shell's callbacks into [`NativeEvent`] messages.
///
/// Every callback returns immediately except for one decision: whether the caller asked to
/// cancel, which is answered with `E_ABORT` (the spike's mechanism, unvalidated on a real
/// Windows session — see the module note).
#[implement(IFileOperationProgressSink)]
struct TransferSink {
    events: Sender<NativeEvent>,
    cancel: Arc<AtomicBool>,
    /// The planned top-level sources, keyed by [`comparison_key`]: only these produce an
    /// undo pair. The value is the plan's own path, so an undo entry built from a native
    /// transfer carries exactly the path a built-in transfer would have carried.
    planned: HashMap<String, PathBuf>,
    /// Resolved parent folders, memoised: a folder with a thousand files resolves once.
    parents: Mutex<HashMap<PathBuf, PathBuf>>,
    bytes_done: AtomicU64,
}

impl TransferSink {
    /// Reports one finished item, if it is a planned top-level one, and checks for cancel.
    fn item_finished(
        &self,
        item: &Ref<'_, IShellItem>,
        newly: &Ref<'_, IShellItem>,
        hr: HRESULT,
    ) -> WinResult<()> {
        let source = item_fs_path(item);
        if hr.is_err() {
            if let Some(path) = source {
                let _ = self
                    .events
                    .send(NativeEvent::ItemFailed { path, code: hr.0 });
            }
            return self.check_cancel();
        }
        let destination = item_fs_path(newly);
        // Bytes are counted for every item, nested ones included: the whole tree is what
        // the progress bar measures.
        let bytes = destination
            .as_deref()
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let total_bytes = self.bytes_done.fetch_add(bytes, Ordering::Relaxed) + bytes;

        if let (Some(source), Some(destination)) = (source, destination) {
            if let Some(planned) = self.planned_source(&source) {
                let _ = self.events.send(NativeEvent::ItemDone {
                    source: planned,
                    destination: destination.clone(),
                });
            }
            let _ = self.events.send(NativeEvent::Progress {
                files_done: 0,
                bytes_done: total_bytes,
                current: Some(destination),
            });
        }
        self.check_cancel()
    }

    /// The planned top-level source `path` refers to, if it is one.
    fn planned_source(&self, path: &Path) -> Option<PathBuf> {
        let key = comparison_key(path, &self.parents);
        self.planned.get(&key).cloned()
    }

    /// Stops the operation when the caller cancelled, without touching shared state.
    fn check_cancel(&self) -> WinResult<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(E_ABORT.into())
        } else {
            Ok(())
        }
    }

    fn report_pre(&self, item: &Ref<'_, IShellItem>) -> WinResult<()> {
        if let Some(path) = item_fs_path(item) {
            let _ = self.events.send(NativeEvent::Progress {
                files_done: 0,
                bytes_done: self.bytes_done.load(Ordering::Relaxed),
                current: Some(path),
            });
        }
        self.check_cancel()
    }
}

#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for TransferSink_Impl {
    fn StartOperations(&self) -> WinResult<()> {
        let _ = self.events.send(NativeEvent::Started);
        Ok(())
    }

    fn FinishOperations(&self, _hrresult: HRESULT) -> WinResult<()> {
        // The worker reads the operation's own abort flag and error after
        // `PerformOperations`; this crate holds no logger, so the verdict travels to the
        // application through the terminal [`NativeEvent`] only.
        Ok(())
    }

    fn PreRenameItem(
        &self,
        _flags: u32,
        item: Ref<'_, IShellItem>,
        _new_name: &PCWSTR,
    ) -> WinResult<()> {
        self.report_pre(&item)
    }

    fn PostRenameItem(
        &self,
        _flags: u32,
        item: Ref<'_, IShellItem>,
        _new_name: &PCWSTR,
        hr: HRESULT,
        newly: Ref<'_, IShellItem>,
    ) -> WinResult<()> {
        self.item_finished(&item, &newly, hr)
    }

    fn PreMoveItem(
        &self,
        _flags: u32,
        item: Ref<'_, IShellItem>,
        _dest: Ref<'_, IShellItem>,
        _new_name: &PCWSTR,
    ) -> WinResult<()> {
        self.report_pre(&item)
    }

    fn PostMoveItem(
        &self,
        _flags: u32,
        item: Ref<'_, IShellItem>,
        _dest: Ref<'_, IShellItem>,
        _new_name: &PCWSTR,
        hr: HRESULT,
        newly: Ref<'_, IShellItem>,
    ) -> WinResult<()> {
        self.item_finished(&item, &newly, hr)
    }

    fn PreCopyItem(
        &self,
        _flags: u32,
        item: Ref<'_, IShellItem>,
        _dest: Ref<'_, IShellItem>,
        _new_name: &PCWSTR,
    ) -> WinResult<()> {
        self.report_pre(&item)
    }

    fn PostCopyItem(
        &self,
        _flags: u32,
        item: Ref<'_, IShellItem>,
        _dest: Ref<'_, IShellItem>,
        _new_name: &PCWSTR,
        hr: HRESULT,
        newly: Ref<'_, IShellItem>,
    ) -> WinResult<()> {
        self.item_finished(&item, &newly, hr)
    }

    fn UpdateProgress(&self, work_total: u32, work_so_far: u32) -> WinResult<()> {
        let _ = work_total;
        let _ = self.events.send(NativeEvent::Progress {
            files_done: work_so_far as u64,
            bytes_done: self.bytes_done.load(Ordering::Relaxed),
            current: None,
        });
        self.check_cancel()
    }

    fn PreDeleteItem(&self, _flags: u32, _item: Ref<'_, IShellItem>) -> WinResult<()> {
        // This route only copies and moves; a delete callback would mean the gate failed.
        Ok(())
    }

    fn PostDeleteItem(
        &self,
        _flags: u32,
        item: Ref<'_, IShellItem>,
        hr: HRESULT,
        _newly: Ref<'_, IShellItem>,
    ) -> WinResult<()> {
        if hr.is_err()
            && let Some(path) = item_fs_path(&item)
        {
            let _ = self
                .events
                .send(NativeEvent::ItemFailed { path, code: hr.0 });
        }
        Ok(())
    }

    fn PreNewItem(
        &self,
        _flags: u32,
        _dest: Ref<'_, IShellItem>,
        _new_name: &PCWSTR,
    ) -> WinResult<()> {
        Ok(())
    }

    fn PostNewItem(
        &self,
        _flags: u32,
        _dest: Ref<'_, IShellItem>,
        _new_name: &PCWSTR,
        _template: &PCWSTR,
        _attrs: u32,
        _hr: HRESULT,
        _newly: Ref<'_, IShellItem>,
    ) -> WinResult<()> {
        Ok(())
    }

    fn ResetTimer(&self) -> WinResult<()> {
        Ok(())
    }

    fn PauseTimer(&self) -> WinResult<()> {
        Ok(())
    }

    fn ResumeTimer(&self) -> WinResult<()> {
        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Opens `path` with no sharing allowed, which is the locked-file fixture the transfer
/// conformance suite needs.
///
/// A locked file is how the suite proves failure parity between the two routes; producing
/// one needs `CreateFileW` with a zero share mode, and raw Win32 may only live in an
/// interop crate (CLAUDE.md rule 3). The handle closes with the `File`, unlocking the path.
///
/// Not part of the product path: nothing in the application calls this.
pub fn open_locked(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::io::FromRawHandle;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_MODE,
        OPEN_EXISTING,
    };

    let wide = wide(path);
    // SAFETY: `wide` is a live NUL-terminated buffer for the call, the security attributes
    // are optional and absent, and the returned handle is owned by the `File` built from it
    // (which closes it exactly once on drop).
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .map_err(std::io::Error::from)?;
    // SAFETY: the handle came from `CreateFileW` above and is transferred to the `File`
    // exactly once; nothing else in this function reads or closes it.
    Ok(unsafe { std::fs::File::from_raw_handle(handle.0) })
}

/// The flags the operation runs with.
///
/// The product keeps the Shell's progress dialog and, per decision 2, its three-choice
/// confirmation if a destination appears after the route gate ran. The headless mode
/// suppresses both so a test can never block on a modal window.
///
/// The error UI is off in both modes: a failed item is reported through the sink and shown
/// in the activity center, never as one modal box per file (spec §7). Measured on this host
/// in the slice-5 conformance run, where a locked source with the error UI on left the
/// operation waiting for a dialog nobody could dismiss.
fn operation_flags(show_progress: bool) -> FILEOPERATION_FLAGS {
    let base = FOF_NOERRORUI;
    if show_progress {
        base
    } else {
        base | FOF_SILENT | FOF_NOCONFIRMATION
    }
}

/// `path` without its verbatim prefix, which is the form the Shell accepts.
///
/// Lossless: the prefix is stripped from the wide string, so a name that is not valid UTF-8
/// survives unchanged. `\\?\UNC\server\share` becomes `\\server\share`, and a path that
/// carries no prefix is returned as it is.
fn display_form(path: &Path) -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    const VERBATIM: [u16; 4] = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const UNC: [u16; 4] = [b'U' as u16, b'N' as u16, b'C' as u16, b'\\' as u16];

    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.len() < VERBATIM.len() || wide[..VERBATIM.len()] != VERBATIM {
        return path.to_path_buf();
    }
    let rest = if wide.len() >= VERBATIM.len() + UNC.len()
        && wide[VERBATIM.len()..VERBATIM.len() + UNC.len()] == UNC
    {
        let mut unc = [b'\\' as u16, b'\\' as u16].to_vec();
        unc.extend_from_slice(&wide[VERBATIM.len() + UNC.len()..]);
        unc
    } else {
        wide[VERBATIM.len()..].to_vec()
    };
    PathBuf::from(std::ffi::OsString::from_wide(&rest))
}

fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// The Shell item for a filesystem path.
///
/// The Shell parses the display form: a `\\?\` verbatim path, which `std` needs to reach
/// past `MAX_PATH`, is refused with `E_INVALIDARG` (measured in the slice-5 conformance run).
fn shell_item(path: &Path) -> WinResult<IShellItem> {
    let wide = wide(&display_form(path));
    // SAFETY: `wide` is a live NUL-terminated buffer for the call, and the Shell copies
    // what it needs before returning.
    unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }
}

/// The filesystem path a shell item names, or `None` when it is not a filesystem item.
fn item_fs_path(item: &Ref<'_, IShellItem>) -> Option<PathBuf> {
    let item = item.as_ref()?;
    // SAFETY: `item` is a live interface reference and the returned string is freed on
    // both paths below.
    unsafe {
        match item.GetDisplayName(SIGDN_FILESYSPATH) {
            Ok(path) => {
                let text = PCWSTR(path.0).to_string().ok();
                CoTaskMemFree(Some(path.0.cast()));
                text.map(PathBuf::from)
            }
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe answers, and a second call answers the same thing from the cache: the
    /// route gate reads this boolean on every transfer.
    #[test]
    fn the_capability_probe_is_cached_and_stable() {
        let first = probe_file_operation();
        let second = probe_file_operation();
        assert_eq!(first, second);
    }

    /// The silent flag combination is what the conformance suite relies on: a native run in
    /// a test must never open a dialog. The product keeps the progress dialog (decision 2).
    #[test]
    fn the_dialog_and_silent_flag_modes_are_distinct() {
        assert_eq!(operation_flags(true), FOF_NOERRORUI);
        assert_eq!(
            operation_flags(false),
            FOF_NOERRORUI | FOF_SILENT | FOF_NOCONFIRMATION,
            "a headless run must suppress every Shell prompt"
        );
    }

    /// A short-name ancestor and its long form reach the same key, and the case does not
    /// matter: this is the comparison that lost every undo pair on the CI runner.
    #[test]
    fn a_short_name_ancestor_reaches_the_same_key() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        use windows::Win32::Storage::FileSystem::GetShortPathNameW;
        use windows::core::PCWSTR;

        let folder = std::env::temp_dir().join(format!("fx-key-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("create the fixture folder");
        let long_folder = std::fs::canonicalize(&folder).expect("canonicalise the fixture");
        std::fs::write(long_folder.join("a.txt"), b"").expect("write the fixture");

        let wide: Vec<u16> = long_folder
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut short = vec![0u16; 1024];
        // SAFETY: `wide` is a NUL-terminated wide string that outlives the call, and
        // `short` is a live buffer whose length is passed to the API.
        let written = unsafe { GetShortPathNameW(PCWSTR(wide.as_ptr()), Some(&mut short)) };
        let short_folder = if written == 0 || written as usize >= short.len() {
            long_folder.clone()
        } else {
            PathBuf::from(std::ffi::OsString::from_wide(&short[..written as usize]))
        };

        let parents = Mutex::new(HashMap::new());
        assert_eq!(
            comparison_key(&long_folder.join("a.txt"), &parents),
            comparison_key(&short_folder.join("A.TXT"), &parents),
            "a short name, a long name and a case change must agree"
        );
        assert_ne!(
            comparison_key(&short_folder.join("a.txt"), &parents),
            comparison_key(&short_folder.join("b.txt"), &parents),
            "two files in one folder must not share a key"
        );

        let _ = std::fs::remove_dir_all(&long_folder);
    }

    /// A path whose folder is gone still yields a key, so a deleted source tree cannot make
    /// the sink panic mid-operation.
    #[test]
    fn a_missing_folder_still_yields_a_key() {
        let parents = Mutex::new(HashMap::new());
        let key = comparison_key(Path::new(r"C:\no-such-fx-folder\a.txt"), &parents);
        assert!(key.ends_with("a.txt"), "unexpected key: {key}");
    }

    /// The Shell rejects a verbatim path, so the prefix is removed for its call only — and
    /// the conversion is lossless.
    #[test]
    fn verbatim_paths_are_stripped_for_the_shell() {
        assert_eq!(
            display_form(Path::new(r"\\?\C:\Temp\a.txt")),
            PathBuf::from(r"C:\Temp\a.txt")
        );
        assert_eq!(
            display_form(Path::new(r"\\?\UNC\server\share\a.txt")),
            PathBuf::from(r"\\server\share\a.txt")
        );
        assert_eq!(
            display_form(Path::new(r"C:\Temp\a.txt")),
            PathBuf::from(r"C:\Temp\a.txt"),
            "an ordinary path is untouched"
        );
    }

    /// An already-cancelled request reports one finished event and touches no COM: a cancel
    /// that arrives before the queue cannot leave a half-run operation behind.
    #[test]
    fn a_cancelled_request_never_starts_the_operation() {
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(true));
        let request = NativeTransferRequest {
            kind: NativeKind::Copy,
            sources: Vec::new(),
            destination: std::env::temp_dir(),
            owner: None,
            show_progress: false,
        };
        run_native_transfer(request, tx, cancel).join();

        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        assert_eq!(
            events,
            vec![NativeEvent::Finished {
                aborted: true,
                error: None
            }],
            "a pre-cancelled transfer reports exactly one terminal event"
        );
    }
}
