//! The native Windows context menu: the Shell's verbs merged with the Ferrix section.
//!
//! Tranche 3, slice 1 (spec §4, §5, §7). The Shell's `IContextMenu` for the current
//! selection — or the folder's background menu — is queried with
//! `IContextMenu::QueryContextMenu`, the Ferrix section is appended with command ids
//! reserved *outside* the Shell's id range, and the merged popup is shown with
//! `TrackPopupMenuEx`.
//!
//! Threading (spec decision 4): the Shell objects are apartment-bound and a popup menu
//! belongs to the thread that created it, so the whole interaction runs on one thread.
//! [`MenuHost::CallerThread`] runs it on the calling (UI) thread, the same way the
//! app-local menu already does; [`MenuHost::HelperThread`] runs it on a dedicated STA
//! helper thread while the caller watches a watchdog and falls back to the minimal
//! Ferrix menu when the Shell takes too long. Which one is used is the caller's policy;
//! see `docs/tranche-3-validation.md` for the state of the slice-0 spike that was
//! meant to settle the cross-thread popup question.
//!
//! Every `unsafe` block below carries the invariant it relies on (CLAUDE.md rule 3).

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{GetLastError, HWND};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    CMF_NORMAL, CMINVOKECOMMANDINFO, Common::ITEMIDLIST, GCS_VERBA, IContextMenu, IShellFolder,
    SHGetDesktopFolder,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetMenuItemCount, IsWindow, MF_SEPARATOR, MF_STRING,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx,
};
use windows::core::{HRESULT, PCSTR, PCWSTR, PSTR};

/// First command id the Shell may use; verb offsets are relative to it.
const SHELL_ID_FIRST: u32 = 1;

/// Last command id the Shell may use.
///
/// Generous on purpose: the Recycle Bin handler assigns internal fixed ids and silently
/// drops any command whose id exceeds `idCmdLast` (validated on Win11 build 22631 in
/// `fx-shell/src/recycle.rs`).
const SHELL_ID_LAST: u32 = 0x7FFF;

/// First command id reserved for the Ferrix section.
///
/// Outside `[SHELL_ID_FIRST, SHELL_ID_LAST]`, so a Ferrix command can never collide
/// with a Shell verb, whatever the label says.
const FERRIX_ID_BASE: u32 = 0x8000;

/// A command the Ferrix section of the menu can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FerrixCommand {
    /// Compute checksums for the selection (PRD 13.4).
    Checksums,
    /// Select the rows matching a pattern (OPS-11).
    SelectByPattern,
    /// Invert the selection (OPS-13).
    InvertSelection,
    /// Create a new file in the folder.
    NewFile,
    /// Scan for empty folders (OPS-13).
    CleanEmptyFolders,
    /// Copy the full path of the selection to the clipboard (PRD 13.4).
    CopyPath,
    /// Copy the name of the selection to the clipboard (PRD 13.4).
    CopyName,
}

/// One entry of the Ferrix section: the command and the label shown for it.
///
/// The label comes from the caller because user-visible text lives in
/// `resources/en-US.rs`, not in this crate.
#[derive(Clone, Debug)]
pub struct FerrixMenuItem {
    /// The command to run when the entry is chosen.
    pub command: FerrixCommand,
    /// The label shown in the menu.
    pub label: String,
}

/// What the caller asks the native menu to show.
pub struct MenuRequest {
    /// The window that owns the popup.
    pub owner: HWND,
    /// Where the popup opens, in screen coordinates.
    pub cursor: (i32, i32),
    /// The selected items. Empty means the folder background.
    pub selection: Vec<PathBuf>,
    /// The folder whose background was clicked; `None` when items are selected.
    pub background: Option<PathBuf>,
    /// The Ferrix section, in display order.
    pub ferrix_items: Vec<FerrixMenuItem>,
    /// Canonical verbs that must not be executed (spec §8.4).
    pub blocked_verbs: Vec<String>,
}

/// A menu request moved to the helper thread.
///
/// The owner window is carried as its address rather than as an `HWND`, because
/// `HWND` wraps a raw pointer and is therefore not `Send`. The address is turned back
/// into an `HWND` on the helper thread, where it is only passed to the Shell as the
/// popup's owner and never dereferenced. The window belongs to the UI thread and
/// outlives the helper, which is joined or abandoned before the window is destroyed.
struct SendRequest {
    /// The address of the owner window.
    owner: usize,
    /// Where the popup opens, in screen coordinates.
    cursor: (i32, i32),
    /// The selected items. Empty means the folder background.
    selection: Vec<PathBuf>,
    /// The folder whose background was clicked; `None` when items are selected.
    background: Option<PathBuf>,
    /// The Ferrix section, in display order.
    ferrix_items: Vec<FerrixMenuItem>,
    /// Canonical verbs that must not be executed (spec §8.4).
    blocked_verbs: Vec<String>,
}

// SAFETY: the owner window is an opaque handle of this process; see the type's comment.
unsafe impl Send for SendRequest {}

impl SendRequest {
    fn from_request(request: MenuRequest) -> Self {
        Self {
            owner: request.owner.0 as usize,
            cursor: request.cursor,
            selection: request.selection,
            background: request.background,
            ferrix_items: request.ferrix_items,
            blocked_verbs: request.blocked_verbs,
        }
    }

    /// The request, back on the helper thread.
    fn into_request(self) -> MenuRequest {
        MenuRequest {
            owner: HWND(self.owner as *mut std::ffi::c_void),
            cursor: self.cursor,
            selection: self.selection,
            background: self.background,
            ferrix_items: self.ferrix_items,
            blocked_verbs: self.blocked_verbs,
        }
    }
}

/// Which thread runs the Shell interaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuHost {
    /// The calling thread runs everything, as the app-local menu already does.
    CallerThread,
    /// A dedicated STA helper thread runs everything; the caller watches a watchdog.
    HelperThread,
}

/// What the user chose in the native menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuOutcome {
    /// A Shell verb ran; the verb is kept for logging.
    Shell { verb: String },
    /// A Ferrix command was chosen and is left to the caller to run.
    Ferrix { command: FerrixCommand },
    /// The menu was dismissed without a choice.
    Cancelled,
    /// The chosen extension is on the local blocklist and was not run.
    Blocked { verb: String },
}

/// How long the native menu took.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuTimings {
    /// Time spent querying the Shell and building the Ferrix section.
    pub query: Duration,
    /// Time the popup stayed on screen.
    pub visible: Duration,
    /// Number of entries the menu offered.
    pub items: u32,
}

/// Why the native menu could not be shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuError {
    /// The query took longer than the caller's budget; the menu was not shown.
    Timeout,
    /// There is no usable owner window, or nothing to show.
    HwndUnavailable,
    /// A windowing call failed with this Win32 error code.
    Hwnd(u32),
    /// A Shell call failed with this HRESULT.
    QueryFailed(HRESULT),
    /// The helper was abandoned before it could answer.
    Abandoned,
}

/// Shows the native context menu and reports what the user chose.
///
/// `timeout` is the budget for the *query* phase — how long the Shell may take before
/// the menu is given up on. An open menu is unbounded by design (spec §6): the user may
/// take as long as they like over it.
///
/// The caller is expected to show the minimal Ferrix menu when this returns an error
/// (spec §8.7): the user never ends up without a menu.
pub fn show_native_menu(
    request: MenuRequest,
    host: MenuHost,
    timeout: Duration,
) -> Result<(MenuOutcome, MenuTimings), MenuError> {
    match host {
        MenuHost::CallerThread => run_menu(request, timeout, None, None),
        MenuHost::HelperThread => run_menu_on_helper(request, timeout),
    }
}

/// Runs the menu on a helper STA thread while the caller watches the watchdog.
fn run_menu_on_helper(
    request: MenuRequest,
    timeout: Duration,
) -> Result<(MenuOutcome, MenuTimings), MenuError> {
    let (ready_tx, ready_rx) = channel::<()>();
    let (result_tx, result_rx) = channel::<Result<(MenuOutcome, MenuTimings), MenuError>>();
    let abandoned = Arc::new(AtomicBool::new(false));

    let thread_request = SendRequest::from_request(request);
    let thread_abandoned = Arc::clone(&abandoned);

    thread::Builder::new()
        .name("fx-menu-helper".to_string())
        .spawn(move || {
            let result = run_menu(
                thread_request.into_request(),
                timeout,
                Some(ready_tx),
                Some(thread_abandoned),
            );
            let _ = result_tx.send(result);
        })
        .map_err(|_| MenuError::Abandoned)?;

    // The watchdog: the menu must appear within the budget, or the caller gives up and
    // shows the minimal Ferrix menu instead (spec §8.2).
    match ready_rx.recv_timeout(timeout) {
        Ok(()) => match result_rx.recv() {
            Ok(result) => result,
            Err(_) => Err(MenuError::Abandoned),
        },
        Err(RecvTimeoutError::Timeout) => {
            // The helper is stuck inside third-party code. It is abandoned, not killed
            // (spec §6): it keeps its thread and COM state until the process exits.
            abandoned.store(true, Ordering::SeqCst);
            Err(MenuError::Timeout)
        }
        Err(RecvTimeoutError::Disconnected) => Err(MenuError::Abandoned),
    }
}

/// Runs the whole Shell interaction on the current thread.
fn run_menu(
    request: MenuRequest,
    timeout: Duration,
    ready: Option<Sender<()>>,
    abandoned: Option<Arc<AtomicBool>>,
) -> Result<(MenuOutcome, MenuTimings), MenuError> {
    // SAFETY: `IsWindow` only reads the handle value; the window itself is not touched.
    if !unsafe { IsWindow(Some(request.owner)) }.as_bool() {
        return Err(MenuError::HwndUnavailable);
    }

    // The Shell objects below are apartment-bound, so the apartment is set up here and
    // torn down before this function returns.
    let _apartment = Apartment::new()?;

    let shell_menu = if !request.selection.is_empty() {
        // SAFETY: the apartment is initialized above and every PIDL handed to the Shell
        // is caller-owned memory that outlives each call.
        unsafe { context_menu_for_selection(request.owner, &request.selection)? }
    } else if let Some(folder) = request.background.as_deref() {
        // SAFETY: as above.
        unsafe { context_menu_for_background(request.owner, folder)? }
    } else {
        return Err(MenuError::HwndUnavailable);
    };

    // SAFETY: the popup menu is created here and destroyed on every path below.
    let menu = match unsafe { CreatePopupMenu() } {
        Ok(menu) => menu,
        Err(_) => return Err(MenuError::Hwnd(last_error())),
    };

    let query_started = Instant::now();
    // SAFETY: `menu` is a live popup menu owned by this thread and `shell_menu` is a
    // live Shell object; the Shell fills `menu` and the HRESULT reports its outcome.
    let hr =
        unsafe { shell_menu.QueryContextMenu(menu, 0, SHELL_ID_FIRST, SHELL_ID_LAST, CMF_NORMAL) };
    let query_time = query_started.elapsed();
    if hr.0 < 0 {
        // SAFETY: `menu` was created above and is not used again.
        unsafe {
            let _ = DestroyMenu(menu);
        };
        return Err(MenuError::QueryFailed(hr));
    }

    // The Ferrix section is appended after the Shell's own entries, separated from
    // them. Its ids live outside the Shell's range, so no label can collide with a verb.
    if !request.ferrix_items.is_empty() {
        // SAFETY: `menu` is a live popup menu owned by this thread.
        if unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) }.is_err() {
            unsafe {
                let _ = DestroyMenu(menu);
            };
            return Err(MenuError::Hwnd(last_error()));
        }
        for (index, item) in request.ferrix_items.iter().enumerate() {
            let label = wide(OsStr::new(&item.label));
            let id = FERRIX_ID_BASE + index as u32;
            // SAFETY: `menu` is live and owned by this thread; `label` is a
            // NUL-terminated wide string that outlives the call.
            if unsafe { AppendMenuW(menu, MF_STRING, id as usize, PCWSTR(label.as_ptr())) }.is_err()
            {
                unsafe {
                    let _ = DestroyMenu(menu);
                };
                return Err(MenuError::Hwnd(last_error()));
            }
        }
    }

    // SAFETY: `menu` is live and owned by this thread.
    let items = unsafe { GetMenuItemCount(Some(menu)) }.max(0) as u32;

    // The budget covers the query phase: a Shell that took too long is not worth
    // showing (spec §8.2). On the caller thread the budget is checked here, because the
    // query has already returned; the helper-thread host enforces the same budget with
    // its watchdog, because it cannot interrupt a running query.
    if query_time > timeout && ready.is_none() {
        // SAFETY: `menu` was created above and is not used again.
        unsafe {
            let _ = DestroyMenu(menu);
        };
        return Err(MenuError::Timeout);
    }

    // Tell the caller the menu is about to appear, so its watchdog can stop.
    if let Some(ready) = ready
        && ready.send(()).is_err()
    {
        // SAFETY: `menu` was created above and is not used again.
        unsafe {
            let _ = DestroyMenu(menu);
        };
        return Err(MenuError::Abandoned);
    }

    // If the caller gave up while the query was running, do not put a stray popup on
    // screen: the helper is abandoned, not killed (spec §6).
    if abandoned
        .as_ref()
        .is_some_and(|flag| flag.load(Ordering::SeqCst))
    {
        // SAFETY: `menu` was created above and is not used again.
        unsafe {
            let _ = DestroyMenu(menu);
        };
        return Err(MenuError::Abandoned);
    }

    let visible_started = Instant::now();
    // SAFETY: `menu` is live and owned by this thread; `owner` is a window of this
    // process, and the call runs its own modal loop until the user dismisses the menu.
    let chosen = unsafe {
        TrackPopupMenuEx(
            menu,
            TPM_RETURNCMD.0 | TPM_RIGHTBUTTON.0,
            request.cursor.0,
            request.cursor.1,
            request.owner,
            None,
        )
    }
    .0 as u32;
    let visible_time = visible_started.elapsed();

    // SAFETY: the popup is closed and `menu` is not used again.
    unsafe {
        let _ = DestroyMenu(menu);
    };

    let outcome = resolve(chosen, &request, &shell_menu)?;
    Ok((
        outcome,
        MenuTimings {
            query: query_time,
            visible: visible_time,
            items,
        },
    ))
}

/// Turns the id TrackPopupMenuEx returned into an outcome.
fn resolve(
    chosen: u32,
    request: &MenuRequest,
    shell_menu: &IContextMenu,
) -> Result<MenuOutcome, MenuError> {
    if chosen == 0 {
        return Ok(MenuOutcome::Cancelled);
    }

    if chosen >= FERRIX_ID_BASE {
        let index = (chosen - FERRIX_ID_BASE) as usize;
        return match request.ferrix_items.get(index) {
            Some(item) => Ok(MenuOutcome::Ferrix {
                command: item.command,
            }),
            None => Err(MenuError::Abandoned),
        };
    }

    // Shell verbs are addressed by their offset from `idCmdFirst`, exactly as
    // `fx-shell/src/recycle.rs` does (validated on Win11 build 22631).
    let offset = (chosen - SHELL_ID_FIRST) as usize;
    // SAFETY: `shell_menu` is live on this thread and `offset` is below the id range
    // handed to QueryContextMenu.
    let verb = unsafe { command_verb(shell_menu, offset) };

    if request
        .blocked_verbs
        .iter()
        .any(|blocked| blocked.eq_ignore_ascii_case(&verb))
    {
        return Ok(MenuOutcome::Blocked { verb });
    }

    // SAFETY: `shell_menu` is live on this thread and the verb offset is one the Shell
    // itself assigned during QueryContextMenu.
    unsafe { invoke_verb(shell_menu, offset, request.owner)? };
    Ok(MenuOutcome::Shell { verb })
}

/// Reads the canonical verb token of a menu entry, for logging and for the blocklist.
unsafe fn command_verb(menu: &IContextMenu, offset: usize) -> String {
    let mut buffer = [0u8; 256];
    // SAFETY: `buffer` is writable for `buffer.len()` bytes and the Shell writes at most
    // that many bytes into it.
    let ok = unsafe {
        menu.GetCommandString(
            offset,
            GCS_VERBA,
            None,
            PSTR(buffer.as_mut_ptr()),
            buffer.len() as u32,
        )
    };
    if ok.is_err() {
        return String::new();
    }
    let end = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[..end]).into_owned()
}

/// Runs a Shell verb by its offset from `idCmdFirst`.
unsafe fn invoke_verb(menu: &IContextMenu, offset: usize, owner: HWND) -> Result<(), MenuError> {
    let info = CMINVOKECOMMANDINFO {
        cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
        fMask: 0,
        hwnd: owner,
        // MAKEINTRESOURCEA(offset): a value whose high word is zero is a verb offset,
        // not a string (same encoding as `fx-shell/src/recycle.rs`).
        lpVerb: PCSTR(offset as *const u8),
        lpParameters: PCSTR::null(),
        lpDirectory: PCSTR::null(),
        nShow: 1, // SW_SHOWNORMAL
        ..Default::default()
    };
    // SAFETY: `menu` is live on this thread and `info` is a fully initialised
    // CMINVOKECOMMANDINFO whose pointers are null or owner-relative.
    unsafe { menu.InvokeCommand(&info) }.map_err(|error| MenuError::QueryFailed(error.code()))
}

/// The Shell's context menu for the selected items.
///
/// The Shell builds a multi-selection menu from children of one folder, so the items
/// must share a parent; an item from another folder is left out rather than resolved
/// against the wrong folder, where it would hit a same-named file.
unsafe fn context_menu_for_selection(
    owner: HWND,
    selection: &[PathBuf],
) -> Result<IContextMenu, MenuError> {
    let Some(first) = selection.first() else {
        return Err(MenuError::HwndUnavailable);
    };
    let Some(parent_path) = first.parent() else {
        return Err(MenuError::HwndUnavailable);
    };

    // SAFETY: the apartment is initialized by the caller and every PIDL below is freed
    // before this function returns.
    unsafe {
        let desktop = SHGetDesktopFolder().map_err(|error| MenuError::QueryFailed(error.code()))?;
        let folder = bind_folder(&desktop, parent_path)?;

        let mut children: Vec<*mut ITEMIDLIST> = Vec::with_capacity(selection.len());
        for item in selection {
            if item.parent() != Some(parent_path) {
                continue;
            }
            let Some(name) = item.file_name() else {
                continue;
            };
            match parse_child(&folder, name) {
                Ok(child) => children.push(child),
                Err(error) => {
                    // The PIDLs already parsed must not leak.
                    for child in children {
                        CoTaskMemFree(Some(child.cast()));
                    }
                    return Err(error);
                }
            }
        }
        if children.is_empty() {
            return Err(MenuError::HwndUnavailable);
        }

        let pointers: Vec<*const ITEMIDLIST> = children
            .iter()
            .map(|child| *child as *const ITEMIDLIST)
            .collect();
        let menu = folder.GetUIObjectOf(owner, &pointers, None);
        for child in children {
            CoTaskMemFree(Some(child.cast()));
        }
        menu.map_err(|error| MenuError::QueryFailed(error.code()))
    }
}

/// The Shell's background context menu of a folder.
unsafe fn context_menu_for_background(
    owner: HWND,
    folder_path: &Path,
) -> Result<IContextMenu, MenuError> {
    // SAFETY: the apartment is initialized by the caller and the PIDL below is freed
    // before this function returns.
    unsafe {
        let desktop = SHGetDesktopFolder().map_err(|error| MenuError::QueryFailed(error.code()))?;
        let folder = bind_folder(&desktop, folder_path)?;
        // An empty PIDL array asks the folder for its background menu.
        folder
            .GetUIObjectOf(owner, &[], None)
            .map_err(|error| MenuError::QueryFailed(error.code()))
    }
}

/// Binds the Shell folder that lives at `path`.
unsafe fn bind_folder(desktop: &IShellFolder, path: &Path) -> Result<IShellFolder, MenuError> {
    let name = wide(path.as_os_str());
    let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
    // SAFETY: `name` is NUL-terminated and alive for the call; the Shell writes a
    // freshly allocated PIDL into `pidl`, which is freed below.
    unsafe {
        desktop
            .ParseDisplayName(
                HWND::default(),
                None,
                PCWSTR(name.as_ptr()),
                None,
                &mut pidl,
                std::ptr::null_mut(),
            )
            .map_err(|error| MenuError::QueryFailed(error.code()))?;
        if pidl.is_null() {
            return Err(MenuError::QueryFailed(HRESULT(0x80004005u32 as i32)));
        }
        let folder = desktop
            .BindToObject(pidl, None)
            .map_err(|error| MenuError::QueryFailed(error.code()))?;
        CoTaskMemFree(Some(pidl.cast()));
        Ok(folder)
    }
}

/// Resolves one child of `folder` by name, into a PIDL relative to that folder.
unsafe fn parse_child(folder: &IShellFolder, name: &OsStr) -> Result<*mut ITEMIDLIST, MenuError> {
    let wide_name = wide(name);
    let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
    // SAFETY: `wide_name` is NUL-terminated and alive for the call; the Shell writes a
    // freshly allocated PIDL into `pidl`, whose ownership passes to the caller.
    unsafe {
        folder
            .ParseDisplayName(
                HWND::default(),
                None,
                PCWSTR(wide_name.as_ptr()),
                None,
                &mut pidl,
                std::ptr::null_mut(),
            )
            .map_err(|error| MenuError::QueryFailed(error.code()))?;
    }
    if pidl.is_null() {
        return Err(MenuError::QueryFailed(HRESULT(0x80004005u32 as i32)));
    }
    Ok(pidl)
}

/// NUL-terminated wide form of a string.
fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

/// The last Win32 error code, for the typed menu error.
fn last_error() -> u32 {
    // SAFETY: GetLastError only reads the calling thread's error slot.
    unsafe { GetLastError().0 }
}

/// STA guard for the Shell objects created while a menu is built.
struct Apartment;

impl Apartment {
    fn new() -> Result<Self, MenuError> {
        // SAFETY: one COM initialization for this thread, paired with exactly one
        // CoUninitialize in `Drop`; the guard never leaves the calling thread.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr.is_ok() {
            Ok(Self)
        } else {
            Err(MenuError::QueryFailed(hr))
        }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: paired 1:1 with the successful CoInitializeEx in `Apartment::new`.
        unsafe { CoUninitialize() };
    }
}
