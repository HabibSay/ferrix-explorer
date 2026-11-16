//! The OLE drop target: receives file drops from Explorer, other OLE applications and
//! Ferrix itself, and hands them to the host.
//!
//! Tranche 3, slice 2 (spec §7). The `lib.rs` no-op `FileDropTarget` is replaced by this
//! real implementation:
//!
//! * `DragEnter` reads `CF_HDROP` **once** and caches the paths plus the source's
//!   `Preferred DropEffect`.
//! * `DragOver` asks the host what effect the drop would have at the cursor, so the shell
//!   shows the right cursor (spec decision 8).
//! * `DragLeave` clears the cache.
//! * `Drop` snapshots the paths and hands them to the [`DropSink`]; the host owns the
//!   destination and the operation, so no file I/O happens on the UI thread (rule 4).
//!
//! A drop that carries no file payload is refused with `DROPEFFECT_NONE`, which is what
//! makes the shell show the standard "no" cursor instead of silently doing nothing.
//!
//! The shell layer stays free of *policy*: the rules that pick an effect live in `fx-ops`,
//! which this crate must not depend on (spec §4). What lives here is the shell's own
//! vocabulary — the `DROPEFFECT_*` mapping and the reading of the modifier keys — and the
//! translation of the cursor position into the window's client coordinates.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{DRAGDROP_E_ALREADYREGISTERED, HGLOBAL, POINT, POINTL};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{
    DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE,
    IDropTarget, IDropTarget_Impl, ReleaseStgMedium,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::core::{PCWSTR, Ref};
use windows_implement::implement;

/// The effect a drop performs.
///
/// Maps onto the shell's `DROPEFFECT_*` values; the conversion lives in this crate
/// because the mapping is a shell detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragEffect {
    /// The files are copied to the destination.
    Copy,
    /// The files are moved to the destination.
    Move,
    /// A shortcut to the files is created in the destination.
    Shortcut,
}

impl DragEffect {
    /// The shell effect value for this effect.
    pub fn to_dropeffect(self) -> DROPEFFECT {
        match self {
            Self::Copy => DROPEFFECT_COPY,
            Self::Move => DROPEFFECT_MOVE,
            Self::Shortcut => DROPEFFECT_LINK,
        }
    }

    /// The effect a shell effect value stands for, if any.
    ///
    /// `DROPEFFECT_NONE` and the scroll pseudo-effect have no counterpart here.
    pub fn from_dropeffect(value: DROPEFFECT) -> Option<Self> {
        match value {
            DROPEFFECT_COPY => Some(Self::Copy),
            DROPEFFECT_MOVE => Some(Self::Move),
            DROPEFFECT_LINK => Some(Self::Shortcut),
            _ => None,
        }
    }
}

/// The modifier keys held while a drag is in progress.
///
/// This is the shell's reading of the keyboard; the *meaning* of those keys lives in
/// `fx-ops`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropModifiers {
    /// The control key is held.
    pub control: bool,
    /// The shift key is held.
    pub shift: bool,
}

impl DropModifiers {
    /// Reads the modifier keys as they are right now.
    ///
    /// The shell's `grfKeyState` is only reliable for the drag's first moment, so the
    /// live keys are read instead (spec §7).
    pub fn current() -> Self {
        Self {
            control: key_is_down(VK_CONTROL.0 as i32),
            shift: key_is_down(VK_SHIFT.0 as i32),
        }
    }
}

/// Whether a virtual key is currently held down.
fn key_is_down(virtual_key: i32) -> bool {
    // SAFETY: `GetAsyncKeyState` only reads the calling thread's key state; the high bit
    // of the result is set while the key is down.
    unsafe { GetAsyncKeyState(virtual_key) < 0 }
}

/// The files a drag carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DragPayload {
    /// The dragged paths, in the order the source listed them.
    pub paths: Vec<PathBuf>,
    /// The effect the source advertised as its preference.
    ///
    /// Explorer publishes this in the `Preferred DropEffect` format; it says what the drag
    /// would do without a modifier key.
    pub preferred: DragEffect,
}

/// What the host did with a drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropOutcome {
    /// The drop was carried out with this effect.
    Executed { effect: DragEffect },
    /// The drop was cancelled by the user or by the host.
    Cancelled,
    /// The drop was refused; the shell showed its "no" cursor.
    Rejected,
}

/// The host's side of a drop.
///
/// The shell target calls the sink on the thread that registered the drop target, which is
/// the UI thread. The implementation must therefore not do file I/O: it hands the work to
/// the operation runner instead (rule 4).
///
/// The effect is the host's decision: the shell only knows the shell's vocabulary
/// (`DROPEFFECT_*`), while the *rules* that pick an effect live in `fx-ops`, which
/// `fx-shell` must not depend on. The host therefore answers both questions.
pub trait DropSink: Send + Sync {
    /// The effect a drop at `point` would perform, or `None` to refuse it.
    ///
    /// This drives the cursor the shell shows while the drag hovers the window, so it is
    /// called often and must stay cheap: no file I/O.
    ///
    /// `point` is in the client coordinates of the window that registered the drop target.
    fn effect_at(&self, payload: &DragPayload, point: (i32, i32)) -> Option<DragEffect>;

    /// Handles a completed drop.
    ///
    /// `point` is in the client coordinates of the window that registered the drop target.
    /// The call happens inside the shell's modal drag loop, so it must return promptly.
    fn on_drop(&self, payload: DragPayload, point: (i32, i32)) -> DropOutcome;
}

/// The payload cached while a drag hovers the window.
type CachedDrag = DragPayload;

/// The real `IDropTarget` registered on Ferrix's window.
#[implement(IDropTarget)]
struct FileDropTarget {
    sink: Arc<dyn DropSink>,
    hwnd: windows::Win32::Foundation::HWND,
    cached: Mutex<Option<CachedDrag>>,
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for FileDropTarget_Impl {
    fn DragEnter(
        &self,
        data: Ref<'_, IDataObject>,
        _key_state: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let cached = read_payload(&data);
        let client = self.to_client(point);
        let chosen = cached.as_ref().and_then(|payload| {
            self.sink
                .effect_at(payload, client)
                .map(|effect| effect.to_dropeffect())
        });
        if let Ok(mut slot) = self.cached.lock() {
            *slot = cached;
        }
        write_drop_effect(effect, chosen);
        Ok(())
    }

    fn DragOver(
        &self,
        _key_state: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let client = self.to_client(point);
        let chosen = {
            let payload: Option<DragPayload> = match self.cached.lock().ok() {
                Some(guard) => guard.clone(),
                None => None,
            };
            payload.and_then(|payload| {
                self.sink
                    .effect_at(&payload, client)
                    .map(|effect| effect.to_dropeffect())
            })
        };
        write_drop_effect(effect, chosen);
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        if let Ok(mut slot) = self.cached.lock() {
            *slot = None;
        }
        Ok(())
    }

    fn Drop(
        &self,
        _data: Ref<'_, IDataObject>,
        _key_state: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let cached = self.cached.lock().ok().and_then(|mut slot| slot.take());
        let Some(payload) = cached else {
            write_drop_effect(effect, None);
            return Ok(());
        };
        let client = self.to_client(point);
        let outcome = self.sink.on_drop(payload, client);
        let chosen = match outcome {
            DropOutcome::Executed { effect } => Some(effect.to_dropeffect()),
            DropOutcome::Cancelled | DropOutcome::Rejected => None,
        };
        write_drop_effect(effect, chosen);
        Ok(())
    }
}

impl FileDropTarget_Impl {
    /// Converts a screen position into the window's client coordinates.
    ///
    /// The shell hands the cursor position in screen coordinates; the host works in client
    /// coordinates, which is what the window's own layout uses.
    fn to_client(&self, point: &POINTL) -> (i32, i32) {
        let mut point = POINT {
            x: point.x,
            y: point.y,
        };
        // SAFETY: `ScreenToClient` only writes the converted coordinates into the point
        // it is given; the window handle is the one this target is registered on.
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut point);
        }
        (point.x, point.y)
    }
}

/// Writes the effect the shell should act on; `None` means "no drop".
fn write_drop_effect(slot: *mut DROPEFFECT, effect: Option<DROPEFFECT>) {
    if slot.is_null() {
        return;
    }
    let value = effect.unwrap_or(DROPEFFECT_NONE);
    // SAFETY: the shell passes a writable output pointer; the null case is rejected above.
    unsafe { *slot = value };
}

/// Reads the file payload of a drag, if it carries one.
///
/// `CF_HDROP` is read once; the source's `Preferred DropEffect` says what the source would
/// do without a modifier key, which is the base the modifier keys then adjust.
fn read_payload(data: &Ref<'_, IDataObject>) -> Option<CachedDrag> {
    let paths = read_hdrop(data)?;
    let preferred = read_preferred_effect(data).unwrap_or(DragEffect::Copy);
    Some(DragPayload { paths, preferred })
}

/// Reads the file list of a drag's `CF_HDROP` format.
fn read_hdrop(data: &Ref<'_, IDataObject>) -> Option<Vec<PathBuf>> {
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    // SAFETY: `format` is a fully initialised FORMATETC that lives until the call
    // returns, and the data object is a live COM interface for the duration.
    let mut medium = unsafe { data.as_ref().and_then(|d| d.GetData(&format).ok()) }?;
    let paths = read_hdrop_paths(&medium);
    // SAFETY: the medium was filled by `GetData` and is released exactly once here.
    unsafe { ReleaseStgMedium(&mut medium) };
    paths
}

/// Extracts the paths of a `CF_HDROP` medium.
fn read_hdrop_paths(medium: &STGMEDIUM) -> Option<Vec<PathBuf>> {
    // SAFETY: the union member is the one `GetData` filled for a `CF_HDROP` request,
    // which is an HGLOBAL holding a DROPFILES block.
    let handle = unsafe { medium.u.hGlobal };
    if handle.is_invalid() {
        return None;
    }
    let drop = HDROP(handle.0);
    // SAFETY: the handle came from a `CF_HDROP` medium and stays valid until the
    // medium is released, which happens after this function returns.
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
    if count == 0 {
        return None;
    }
    let mut paths = Vec::with_capacity(count as usize);
    for index in 0..count {
        // SAFETY: the query with a null buffer reports the length the file name needs.
        let length = unsafe { DragQueryFileW(drop, index, None) } as usize;
        if length == 0 {
            continue;
        }
        let mut buffer = vec![0u16; length + 1];
        // SAFETY: the buffer holds `length + 1` UTF-16 units and the call writes at most
        // `length` units into it, leaving the terminator in place.
        let written = unsafe { DragQueryFileW(drop, index, Some(&mut buffer)) } as usize;
        if written == 0 {
            continue;
        }
        buffer.truncate(written);
        paths.push(PathBuf::from(OsString::from_wide(&buffer)));
    }
    if paths.is_empty() { None } else { Some(paths) }
}

/// Reads the source's `Preferred DropEffect` format, if the source publishes one.
fn read_preferred_effect(data: &Ref<'_, IDataObject>) -> Option<DragEffect> {
    let name: Vec<u16> = "Preferred DropEffect\0".encode_utf16().collect();
    // SAFETY: `name` is a NUL-terminated wide string that outlives the call.
    let format_id = unsafe { RegisterClipboardFormatW(PCWSTR(name.as_ptr())) };
    if format_id == 0 {
        return None;
    }
    let format = FORMATETC {
        cfFormat: format_id as u16,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    // SAFETY: `format` is fully initialised and lives until the call returns.
    let mut medium = unsafe { data.as_ref().and_then(|d| d.GetData(&format).ok()) }?;
    // SAFETY: the medium was filled by `GetData`; the pointer is read before the
    // medium is released.
    let value = unsafe { read_dword_medium(&medium) };
    // SAFETY: the medium was filled by `GetData` and is released exactly once here.
    unsafe { ReleaseStgMedium(&mut medium) };
    value
}

/// Reads the `DWORD` a `Preferred DropEffect` medium carries.
unsafe fn read_dword_medium(medium: &STGMEDIUM) -> Option<DragEffect> {
    // SAFETY: the union member is the one `GetData` filled for this format, an HGLOBAL
    // holding a single DWORD.
    let handle: HGLOBAL = unsafe { medium.u.hGlobal };
    if handle.is_invalid() {
        return None;
    }
    // SAFETY: the handle is a live HGLOBAL owned by the medium; locking it for reading is
    // valid until the medium is released.
    let pointer = unsafe { GlobalLock(handle) };
    if pointer.is_null() {
        return None;
    }
    // SAFETY: the lock succeeded, so the first DWORD of the block is readable.
    let value = unsafe { std::ptr::read(pointer.cast::<u32>()) };
    // SAFETY: the handle was locked above and is unlocked exactly once here.
    unsafe {
        let _ = GlobalUnlock(handle);
    }
    DragEffect::from_dropeffect(DROPEFFECT(value))
}

/// Reports whether the window already carries a drop target (rule 8 evidence).
///
/// `RegisterDragDrop` refuses a second registration with `DRAGDROP_E_ALREADYREGISTERED`,
/// so this probe answers "is a target on this window?" without owning one: when the
/// window carries none, the probe registers a throw-away target and revokes it again,
/// leaving the window as it was.
pub fn is_drop_target_registered(
    hwnd: windows::Win32::Foundation::HWND,
    sink: Arc<dyn DropSink>,
) -> bool {
    let target: IDropTarget = FileDropTarget {
        sink,
        hwnd,
        cached: Mutex::new(None),
    }
    .into();
    // SAFETY: the window is alive and owned by the calling thread; the probe's own
    // registration is revoked again before this function returns.
    match unsafe { windows::Win32::System::Ole::RegisterDragDrop(hwnd, &target) } {
        Ok(()) => {
            // SAFETY: the probe's registration is the one on the window, and it is
            // revoked here, so the window is left the way it was found.
            unsafe {
                let _ = windows::Win32::System::Ole::RevokeDragDrop(hwnd);
            }
            false
        }
        Err(error) => error.code() == DRAGDROP_E_ALREADYREGISTERED,
    }
}

/// Registers the drop target on a window.
///
/// Must run on the thread that owns the window, on a thread with an OLE apartment
/// initialized. The window's drop target stays registered until the returned guard is
/// dropped.
pub struct OleDropRegistration {
    hwnd: windows::Win32::Foundation::HWND,
    initialized: bool,
    registered: bool,
    _target: IDropTarget,
    _ui_thread_only: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl OleDropRegistration {
    /// Initializes OLE and registers the drop target for `hwnd`.
    ///
    /// The apartment initialization is this function's responsibility: it is done once
    /// here and undone when the guard is dropped.
    pub fn register(
        hwnd: windows::Win32::Foundation::HWND,
        sink: Arc<dyn DropSink>,
    ) -> windows::core::Result<Self> {
        // SAFETY: OLE apartment initialization and drop-target registration run on the
        // caller's UI thread; the `Rc` marker keeps this guard from being moved to
        // another thread.
        unsafe {
            windows::Win32::System::Ole::OleInitialize(None)?;
            let target = FileDropTarget {
                sink,
                hwnd,
                cached: Mutex::new(None),
            }
            .into();
            if let Err(error) = windows::Win32::System::Ole::RegisterDragDrop(hwnd, &target) {
                // winit registers its own `IDropTarget` on every window it creates —
                // `platform_specific.drag_and_drop` defaults to `true` and winit asserts
                // `S_OK` — while Slint's winit backend never relays winit's
                // drag-and-drop events to Slint code. That target is unreachable *and* in
                // the way: revoke it and take the registration over. winit's own
                // `RevokeDragDrop` on `WM_DESTROY` ignores its result, so revoking an
                // already-revoked window later is harmless.
                //
                // Observed with the `ole_registration_probe` integration test: without
                // this branch the registration fails, with it the window carries our
                // target (rule 8 evidence).
                if error.code() == DRAGDROP_E_ALREADYREGISTERED {
                    // SAFETY: the window is alive and owned by this thread; revoking
                    // unregisters whichever target sits on it, ours not being one yet.
                    let _ = windows::Win32::System::Ole::RevokeDragDrop(hwnd);
                }
                if let Err(error) = windows::Win32::System::Ole::RegisterDragDrop(hwnd, &target) {
                    windows::Win32::System::Ole::OleUninitialize();
                    return Err(error);
                }
            }
            Ok(Self {
                hwnd,
                initialized: true,
                registered: true,
                _target: target,
                _ui_thread_only: std::marker::PhantomData,
            })
        }
    }

    /// Revokes the OLE drop target. The window must still be valid.
    pub fn revoke(&mut self) -> windows::core::Result<()> {
        // SAFETY: registration is revoked on the same thread, before the window is
        // destroyed.
        unsafe {
            if self.registered {
                windows::Win32::System::Ole::RevokeDragDrop(self.hwnd)?;
                self.registered = false;
            }
        }
        self.uninitialize();
        Ok(())
    }

    fn uninitialize(&mut self) {
        // SAFETY: each successful `OleInitialize` on this thread is matched exactly once.
        unsafe {
            if self.initialized {
                windows::Win32::System::Ole::OleUninitialize();
                self.initialized = false;
            }
        }
    }
}

impl Drop for OleDropRegistration {
    fn drop(&mut self) {
        // SAFETY: best-effort cleanup for resources created by `register`, on the
        // initializing UI thread. The owner must destroy the window after this guard is
        // dropped.
        unsafe {
            if self.registered {
                let _ = windows::Win32::System::Ole::RevokeDragDrop(self.hwnd);
                self.registered = false;
            }
        }
        self.uninitialize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_map_onto_the_shell_values() {
        assert_eq!(DragEffect::Copy.to_dropeffect(), DROPEFFECT_COPY);
        assert_eq!(DragEffect::Move.to_dropeffect(), DROPEFFECT_MOVE);
        assert_eq!(DragEffect::Shortcut.to_dropeffect(), DROPEFFECT_LINK);
        assert_eq!(
            DragEffect::from_dropeffect(DROPEFFECT_LINK),
            Some(DragEffect::Shortcut)
        );
        assert_eq!(DragEffect::from_dropeffect(DROPEFFECT_NONE), None);
    }
}
