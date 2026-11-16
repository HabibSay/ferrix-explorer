//! Windows Shell/OLE interop spikes for Tranche 0.
//!
//! This crate owns the raw COM and Win32 code used by the host: a no-op `IDropTarget` that proves
//! drop registration works, and an app-local `IContextMenu` that exercises `QueryContextMenu`,
//! `InvokeCommand`, and a native popup menu. Neither registers a Shell extension nor touches the
//! registry. All user-visible text is supplied by the caller.

pub mod clipboard;
pub mod context_menu;
pub mod drop_source;
pub mod drop_target;
pub mod execute;
pub mod file_time;
pub mod icons;
pub mod recycle;
pub mod recycle_quota;
pub mod symlink;
pub mod volume;

pub use drop_source::{
    DragOutcome, DragSourceRequest, create_shortcut, run_drag, shortcut_link_path,
};
pub use drop_target::{
    DragEffect, DragPayload, DropModifiers, DropOutcome, DropSink, OleDropRegistration,
};

use windows::{
    Win32::{
        Foundation::{E_NOTIMPL, E_POINTER, HWND, POINT},
        UI::{
            Shell::{CMF_DEFAULTONLY, CMINVOKECOMMANDINFO, IContextMenu, IContextMenu_Impl},
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, HMENU, MF_STRING,
                TPM_RETURNCMD, TrackPopupMenu,
            },
        },
    },
    core::{HRESULT, PCSTR, PCWSTR, Result},
};
use windows_implement::implement;

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

/// Screen position of the cursor, used to anchor app-local popups.
pub fn cursor_pos() -> (i32, i32) {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` is a valid writable struct on this stack; GetCursorPos fills it
    // or leaves it untouched on failure.
    let ok = unsafe { GetCursorPos(&mut point) };
    if ok.is_err() {
        (0, 0)
    } else {
        (point.x, point.y)
    }
}

/// Show a plain (non-COM) popup menu with the given labels at screen `(x, y)` and
/// return the index of the clicked item, or `None` when dismissed.
///
/// Must be called on the UI thread with a valid `hwnd`; `TPM_RETURNCMD` keeps the
/// call synchronous, so no message-loop plumbing is needed.
pub fn show_popup_menu(hwnd: HWND, x: i32, y: i32, items: &[&str]) -> Result<Option<usize>> {
    const ITEM_BASE: u32 = 0x5000;
    // SAFETY: The menu is destroyed on every path before returning; TrackPopupMenu
    // runs modally on this UI thread and the returned command id is within the
    // range [ITEM_BASE, ITEM_BASE + items.len()) we just added.
    unsafe {
        let menu = CreatePopupMenu()?;
        for (index, label) in items.iter().enumerate() {
            let wide: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                (ITEM_BASE + index as u32) as usize,
                PCWSTR(wide.as_ptr()),
            );
        }
        let selected = TrackPopupMenu(menu, TPM_RETURNCMD, x, y, None, hwnd, None);
        let _ = DestroyMenu(menu);
        if selected.0 == 0 {
            return Ok(None);
        }
        let offset = selected.0 as u32 - ITEM_BASE;
        if (offset as usize) < items.len() {
            Ok(Some(offset as usize))
        } else {
            Ok(None)
        }
    }
}
