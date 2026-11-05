//! Windows Shell/OLE interop spikes for Tranche 0.
//!
//! This crate owns the raw COM and Win32 code used by the host: a no-op `IDropTarget` that proves
//! drop registration works, and an app-local `IContextMenu` that exercises `QueryContextMenu`,
//! `InvokeCommand`, and a native popup menu. Neither registers a Shell extension nor touches the
//! registry. All user-visible text is supplied by the caller.

pub mod execute;

use windows::{
    Win32::{
        Foundation::{E_NOTIMPL, E_POINTER, HWND, POINTL},
        System::{
            Com::IDataObject,
            Ole::{
                DROPEFFECT, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl, OleInitialize,
                OleUninitialize, RegisterDragDrop, RevokeDragDrop,
            },
            SystemServices::MODIFIERKEYS_FLAGS,
        },
        UI::{
            Shell::{CMF_DEFAULTONLY, CMINVOKECOMMANDINFO, IContextMenu, IContextMenu_Impl},
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, DestroyMenu, HMENU, MF_STRING, TPM_RETURNCMD,
                TrackPopupMenu,
            },
        },
    },
    core::{HRESULT, PCSTR, PCWSTR, Ref, Result},
};
use windows_implement::implement;

/// Owns the OLE apartment and the drop-target registration for one UI-thread window.
///
/// Construct and drop this value on the same STA thread. Call [`Self::revoke`] before destroying
/// the window; `Drop` is a best-effort safety net for early returns.
pub struct OleDropRegistration {
    hwnd: HWND,
    initialized: bool,
    registered: bool,
    _target: IDropTarget,
    _ui_thread_only: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl OleDropRegistration {
    /// Initialize OLE and register a minimal `IDropTarget` for `hwnd`.
    ///
    /// The target reports "no effect" for every operation. Extracting and transferring Shell data
    /// belongs to a later tranche and is intentionally not implemented here.
    pub fn register(hwnd: HWND) -> Result<Self> {
        // SAFETY: OLE apartment initialization and drop-target registration run on the caller's UI
        // thread; the `Rc` marker keeps this guard from being moved to another thread.
        unsafe {
            OleInitialize(None)?;
            let target = FileDropTarget.into();
            if let Err(error) = RegisterDragDrop(hwnd, &target) {
                OleUninitialize();
                return Err(error);
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

    /// Revoke the OLE drop target. The HWND must still be valid.
    pub fn revoke(&mut self) -> Result<()> {
        // SAFETY: Registration is revoked on the same thread, before the HWND is destroyed.
        unsafe {
            if self.registered {
                RevokeDragDrop(self.hwnd)?;
                self.registered = false;
            }
        }
        self.uninitialize();
        Ok(())
    }

    fn uninitialize(&mut self) {
        // SAFETY: Each successful `OleInitialize` on this thread is matched exactly once.
        unsafe {
            if self.initialized {
                OleUninitialize();
                self.initialized = false;
            }
        }
    }
}

impl Drop for OleDropRegistration {
    fn drop(&mut self) {
        // SAFETY: Best-effort cleanup for resources created by `register`, on the initializing UI
        // thread. The owner must destroy the HWND after this guard is dropped.
        unsafe {
            if self.registered {
                let _ = RevokeDragDrop(self.hwnd);
                self.registered = false;
            }
        }
        self.uninitialize();
    }
}

#[implement(IDropTarget)]
struct FileDropTarget;

#[allow(non_snake_case)]
impl IDropTarget_Impl for FileDropTarget_Impl {
    fn DragEnter(
        &self,
        _data: Ref<'_, IDataObject>,
        _key_state: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        write_drop_effect(effect)
    }

    fn DragOver(
        &self,
        _key_state: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        write_drop_effect(effect)
    }

    fn DragLeave(&self) -> Result<()> {
        Ok(())
    }

    fn Drop(
        &self,
        _data: Ref<'_, IDataObject>,
        _key_state: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        write_drop_effect(effect)
    }
}

fn write_drop_effect(effect: *mut DROPEFFECT) -> Result<()> {
    if effect.is_null() {
        return Err(E_POINTER.into());
    }
    // SAFETY: COM supplies a writable output pointer; the null case is rejected above.
    unsafe { *effect = DROPEFFECT_NONE };
    Ok(())
}

/// Absolute command ID used for the single app-local menu entry.
const IDM_FERRIX_ABOUT: u32 = 1;

/// `IContextMenu` verb offset of [`IDM_FERRIX_ABOUT`] relative to `idCmdFirst`.
const IDM_FERRIX_ABOUT_VERB: usize = 0;

/// Which app-local command the user chose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextMenuSelection {
    /// The menu was dismissed without choosing a command.
    Dismissed,
    /// The "About" spike command was invoked through `IContextMenu::InvokeCommand`.
    About,
}

#[implement(IContextMenu)]
struct AppContextMenu {
    label: Vec<u16>,
}

#[allow(non_snake_case)]
impl IContextMenu_Impl for AppContextMenu_Impl {
    fn QueryContextMenu(
        &self,
        menu: HMENU,
        _index_menu: u32,
        id_cmd_first: u32,
        id_cmd_last: u32,
        flags: u32,
    ) -> HRESULT {
        if flags & CMF_DEFAULTONLY != 0 || id_cmd_first > id_cmd_last {
            return HRESULT(0);
        }
        // SAFETY: The host supplies a valid HMENU for the duration of this call.
        let appended = unsafe {
            AppendMenuW(
                menu,
                MF_STRING,
                id_cmd_first as usize,
                PCWSTR(self.label.as_ptr()),
            )
        };
        // Low word: number of command IDs consumed (largest offset + one).
        match appended {
            Ok(()) => HRESULT(1),
            Err(error) => error.code(),
        }
    }

    fn InvokeCommand(&self, command: *const CMINVOKECOMMANDINFO) -> Result<()> {
        if command.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: The COM caller provides a readable CMINVOKECOMMANDINFO.
        let verb = unsafe { (*command).lpVerb.0 as usize };
        // Only this menu's numeric ordinal is accepted; raw verb strings are never interpreted.
        if verb == IDM_FERRIX_ABOUT_VERB {
            Ok(())
        } else {
            Err(windows::Win32::Foundation::E_INVALIDARG.into())
        }
    }

    fn GetCommandString(
        &self,
        _id_cmd: usize,
        _command_type: u32,
        _reserved: *const u32,
        _name: windows::core::PSTR,
        _max_chars: u32,
    ) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
}

/// Show an app-local COM context menu at screen coordinates `(x, y)`.
///
/// The popup is built by an in-process `IContextMenu` and displayed with `TPM_RETURNCMD`, which
/// keeps selection handling on this thread. No Shell extension or registry entry is involved.
pub fn show_context_menu(hwnd: HWND, x: i32, y: i32, label: &str) -> Result<ContextMenuSelection> {
    let label: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
    let context_menu: IContextMenu = AppContextMenu { label }.into();

    // SAFETY: The caller owns a valid UI-thread window; menu creation, query, and teardown all run
    // on that thread before the function returns.
    unsafe {
        let menu = CreatePopupMenu()?;
        let query = context_menu.QueryContextMenu(menu, 0, IDM_FERRIX_ABOUT, IDM_FERRIX_ABOUT, 0);
        if query.is_err() {
            let _ = DestroyMenu(menu);
            return Err(query.into());
        }
        if query.0 <= 0 {
            let _ = DestroyMenu(menu);
            return Ok(ContextMenuSelection::Dismissed);
        }

        let selected = TrackPopupMenu(menu, TPM_RETURNCMD, x, y, None, hwnd, None);
        let _ = DestroyMenu(menu);
        if selected.0 as u32 != IDM_FERRIX_ABOUT {
            return Ok(ContextMenuSelection::Dismissed);
        }

        let info = CMINVOKECOMMANDINFO {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
            lpVerb: PCSTR(std::ptr::null()),
            ..Default::default()
        };
        context_menu.InvokeCommand(&info)?;
        Ok(ContextMenuSelection::About)
    }
}
