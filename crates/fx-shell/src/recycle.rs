//! Recycle Bin interop: SHFileOperationW(FOF_ALLOWUNDO) + Shell folder restore.
//!
//! Design note (spec §8): SHFileOperationW does not accept `\\?\` prefixes; paths are
//! passed in display form. Long-path failures return `LongPath`. The before/after diff of
//! the Recycle Bin folder is used to derive the recycled identity; an ambiguous diff
//! sets `verified = false` so undo refuses to act.
//!
//! Restore follows the decided approach (decision 3): enumerate the Recycle Bin's shell
//! folder and invoke the item's own Restore verb, matched by its **language-independent
//! canonical verb token** (`GCS_VERBA` = `"restore"`), never by localized menu text.
//! `$I`/`$R` records are read only for the identity diff and never written or moved here.
//! PRD 10.5: the COM calls run on a worker thread that explicitly initializes an STA.

use std::{
    ffi::c_void,
    path::{Path, PathBuf},
    ptr,
    time::SystemTime,
};

use windows::{
    Win32::{
        Foundation::HWND,
        System::Com::{
            COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoTaskMemFree,
            CoUninitialize,
        },
        UI::{
            Shell::Common::{ITEMIDLIST, STRRET},
            Shell::{
                CMF_NORMAL, CSIDL_BITBUCKET, GCS_VERBA, IContextMenu, IEnumIDList, ILFree,
                IShellFolder, SHCONTF_INCLUDEHIDDEN, SHCONTF_NONFOLDERS, SHGDNF,
                SHGetDesktopFolder, SHGetFolderLocation, StrRetToBufW,
            },
            WindowsAndMessaging::{CreatePopupMenu, DestroyMenu, GetMenuItemCount, HMENU},
        },
    },
    core::{PCSTR, PCWSTR},
};
use windows_core::BOOL;

/// One item the Shell sent to the Recycle Bin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellRecycledItem {
    /// Where the item lived before deletion.
    pub original: PathBuf,
    /// The `$R…` display name assigned by the Recycle Bin (derived from before/after diff).
    pub recycled_name: String,
    /// When the item was deleted.
    pub deletion_time: SystemTime,
}

/// Errors from Recycle Bin operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellRecycleError {
    /// The user cancelled the Shell dialog (e.g. responded "No" to the nuke warning).
    Aborted,
    /// A Win32 / Shell error code was returned.
    Failed(u32),
    /// The path is too long for `SHFileOperationW` (>= 260 characters).
    LongPath,
    /// A required COM or Shell interface was unavailable.
    NotAvailable(String),
}

impl std::fmt::Display for ShellRecycleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Aborted => f.write_str("operation was aborted"),
            Self::Failed(code) => write!(f, "Shell error {code:#x}"),
            Self::LongPath => f.write_str("path is too long for SHFileOperationW"),
            Self::NotAvailable(msg) => write!(f, "Shell interface unavailable: {msg}"),
        }
    }
}

impl std::error::Error for ShellRecycleError {}

/// Sends `paths` to the Recycle Bin using `SHFileOperationW`.
///
/// Uses `FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_WANTNUKEWARNING`.
/// `FOF_WANTNUKEWARNING` is kept deliberately: when Windows cannot recycle an item it shows
/// its own warning dialog rather than silently deleting it (spec §8 deviation).
///
/// The before/after diff of the Recycle Bin folder is used to derive `ShellRecycledItem`s.
/// When the diff is ambiguous the item has an empty `recycled_name` and callers should mark
/// it as unverified.
pub fn recycle_paths(
    paths: &[PathBuf],
    hwnd: HWND,
) -> Result<Vec<ShellRecycledItem>, ShellRecycleError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    // Validate: SHFileOperationW does not accept \\?\ prefixes.
    for path in paths {
        let s = path.to_string_lossy();
        if s.starts_with(r"\\?\") {
            return Err(ShellRecycleError::LongPath);
        }
        if s.len() >= 260 {
            return Err(ShellRecycleError::LongPath);
        }
    }

    // Build the double-NUL-terminated UTF-16 source list.
    let mut from_wide: Vec<u16> = Vec::new();
    for path in paths {
        from_wide.extend(path.to_string_lossy().encode_utf16());
        from_wide.push(0);
    }
    from_wide.push(0); // double-NUL terminator

    // Snapshot the Recycle Bin before the operation.
    let before = snapshot_recycle_bin();
    let deletion_time = SystemTime::now();

    // SAFETY: SHFILEOPSTRUCTW is fully initialised; pFrom is a double-NUL-terminated
    // UTF-16 string that lives for the duration of this call in `from_wide`.
    // We check both the return code and fAnyOperationsAborted.
    let (return_code, any_aborted) = unsafe {
        let mut op = windows::Win32::UI::Shell::SHFILEOPSTRUCTW {
            hwnd,
            wFunc: windows::Win32::UI::Shell::FO_DELETE,
            pFrom: PCWSTR(from_wide.as_ptr()),
            pTo: PCWSTR(ptr::null()),
            fFlags: (windows::Win32::UI::Shell::FOF_ALLOWUNDO
                | windows::Win32::UI::Shell::FOF_NOCONFIRMATION
                | windows::Win32::UI::Shell::FOF_NOERRORUI
                | windows::Win32::UI::Shell::FOF_WANTNUKEWARNING)
                .0 as u16,
            fAnyOperationsAborted: BOOL(0),
            hNameMappings: ptr::null_mut(),
            lpszProgressTitle: PCWSTR(ptr::null()),
        };
        let rc = windows::Win32::UI::Shell::SHFileOperationW(&mut op);
        (rc, op.fAnyOperationsAborted.as_bool())
    };

    if any_aborted {
        return Err(ShellRecycleError::Aborted);
    }
    if return_code != 0 {
        return Err(ShellRecycleError::Failed(return_code as u32));
    }

    // Snapshot after the operation and compute the diff.
    let after = snapshot_recycle_bin();
    let new_items: Vec<String> = after
        .into_iter()
        .filter(|item| !before.contains(item))
        .collect();

    let mut recycled = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        let recycled_name = new_items.get(i).cloned().unwrap_or_default();
        recycled.push(ShellRecycledItem {
            original: path.clone(),
            recycled_name,
            deletion_time,
        });
    }

    Ok(recycled)
}

/// Returns a list of `$R…` item names currently in the Recycle Bin.
///
/// Uses a best-effort listing of the standard Recycle Bin paths (`$Recycle.Bin` on NTFS).
/// This is a lightweight heuristic used only for the before/after identity diff.
fn snapshot_recycle_bin() -> Vec<String> {
    let mut items = Vec::new();
    // Try each drive letter A–Z.
    for letter in b'A'..=b'Z' {
        let recycle_path = format!("{}:\\$Recycle.Bin", letter as char);
        let recycle = Path::new(&recycle_path);
        if !recycle.exists() {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(recycle) {
            for entry in rd.flatten() {
                // Each SID subfolder contains $R… and $I… pairs.
                let sid_path = entry.path();
                if let Ok(rd2) = std::fs::read_dir(&sid_path) {
                    for item in rd2.flatten() {
                        let name = item.file_name().to_string_lossy().into_owned();
                        if name.starts_with("$R") {
                            items.push(name);
                        }
                    }
                }
            }
        }
    }
    items
}

/// STA guard for a worker thread: COM calls below need an apartment, and PRD 10.5
/// requires explicit initialization on non-UI threads.
struct Apartment;

impl Apartment {
    fn new() -> Result<Self, ShellRecycleError> {
        // SAFETY: CoInitializeEx on a thread with no prior COM mode always pairs with
        // exactly one CoUninitialize in this guard's Drop; the guard never outlives
        // the calling thread.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if hr.is_ok() {
            Ok(Self)
        } else {
            Err(ShellRecycleError::NotAvailable(format!(
                "STA init failed: {:#010x}",
                hr.0 as u32
            )))
        }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: paired 1:1 with the successful CoInitializeEx in `Apartment::new`.
        unsafe { CoUninitialize() };
    }
}

/// Restores a recycled item by invoking the Recycle Bin shell item's own "restore"
/// verb (decision 3). The item is matched by display name (which the Recycle Bin
/// shows as the original file name); two items sharing that name are treated as
/// ambiguous and refused, mirroring the spec's identity rule.
pub fn restore_recycled_item(original: &Path) -> Result<(), ShellRecycleError> {
    if original.to_string_lossy().len() >= 260 {
        return Err(ShellRecycleError::LongPath);
    }
    let wanted = original
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| ShellRecycleError::NotAvailable("path has no file name".into()))?;

    let _apartment = Apartment::new()?;

    unsafe {
        // The Recycle Bin as a shell folder, via the documented CSIDL.
        let bin_pidl = SHGetFolderLocation(None, CSIDL_BITBUCKET as i32, None, 0)
            .map_err(|e| ShellRecycleError::NotAvailable(format!("SHGetFolderLocation: {e}")))?;
        if bin_pidl.is_null() {
            return Err(ShellRecycleError::NotAvailable(
                "recycle bin pidl is null".into(),
            ));
        }

        let desktop = SHGetDesktopFolder()
            .map_err(|e| ShellRecycleError::NotAvailable(format!("desktop: {e}")))?;
        let bin: IShellFolder = desktop
            .BindToObject(bin_pidl, None)
            .map_err(|e| ShellRecycleError::NotAvailable(format!("bind bin: {e}")))?;
        ILFree(Some(bin_pidl as *const ITEMIDLIST));

        find_and_restore(&bin, &wanted)?;
    }

    // Post-condition (rule 13): the verb must actually have put the file back.
    if original.exists() {
        Ok(())
    } else {
        Err(ShellRecycleError::NotAvailable(
            "restore verb reported success but the file did not return".into(),
        ))
    }
}

/// Enumerates the bin folder, matches the item by display name and invokes its
/// canonical `restore` verb. Refuses on zero or multiple name matches.
fn find_and_restore(bin: &IShellFolder, wanted: &str) -> Result<(), ShellRecycleError> {
    unsafe {
        let mut enum_opt: Option<IEnumIDList> = None;
        bin.EnumObjects(
            HWND(ptr::null_mut()),
            (SHCONTF_NONFOLDERS.0 | SHCONTF_INCLUDEHIDDEN.0) as u32,
            &raw mut enum_opt,
        )
        .ok()
        .map_err(|e| ShellRecycleError::NotAvailable(format!("EnumObjects: {e}")))?;
        let enumerator = enum_opt
            .ok_or_else(|| ShellRecycleError::NotAvailable("enumerator was null".into()))?;

        let mut matched: Option<*mut ITEMIDLIST> = None;
        let mut display_buf = [0u16; 512];

        loop {
            let mut slot: [*mut ITEMIDLIST; 1] = [ptr::null_mut()];
            let mut fetched = 0u32;
            let hr = enumerator.Next(&mut slot, Some(&raw mut fetched));
            if hr.0 < 0 || fetched == 0 {
                break;
            }
            let item = slot[0];
            if item.is_null() {
                continue;
            }

            let mut name_ok = false;
            let mut strret = STRRET::default();
            if bin
                .GetDisplayNameOf(item, SHGDNF(0), &raw mut strret)
                .is_ok()
                && StrRetToBufW(&raw mut strret, Some(item), &mut display_buf).is_ok()
            {
                let end = display_buf
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(display_buf.len());
                let shown = String::from_utf16_lossy(&display_buf[..end]);
                name_ok = shown == wanted;
            }

            if name_ok {
                if matched.is_some() {
                    // Two items now claim the same name: undo cannot pick safely.
                    CoTaskMemFree(Some(item as *const c_void));
                    if let Some(first) = matched {
                        CoTaskMemFree(Some(first as *const c_void));
                    }
                    return Err(ShellRecycleError::NotAvailable(format!(
                        "ambiguous: two Recycle Bin items named '{wanted}'"
                    )));
                }
                matched = Some(item);
            } else {
                CoTaskMemFree(Some(item as *const c_void));
            }
        }

        let target = matched.ok_or_else(|| {
            ShellRecycleError::NotAvailable(format!("no Recycle Bin item named '{wanted}'"))
        })?;

        let result = invoke_restore_verb(bin, target);
        CoTaskMemFree(Some(target as *const c_void));
        result
    }
}

fn invoke_restore_verb(bin: &IShellFolder, item: *mut ITEMIDLIST) -> Result<(), ShellRecycleError> {
    unsafe {
        let apidl: [*const ITEMIDLIST; 1] = [item as *const ITEMIDLIST];
        let ctx: IContextMenu = bin
            .GetUIObjectOf(HWND(ptr::null_mut()), &apidl, None)
            .map_err(|e| ShellRecycleError::NotAvailable(format!("GetUIObjectOf: {e}")))?;

        let menu: HMENU = CreatePopupMenu()
            .map_err(|e| ShellRecycleError::NotAvailable(format!("CreatePopupMenu: {e}")))?;
        let query = ctx.QueryContextMenu(menu, 0, 1, 100, CMF_NORMAL);
        if query.0 < 0 {
            let _ = DestroyMenu(menu);
            return Err(ShellRecycleError::NotAvailable(format!(
                "QueryContextMenu: {:#010x}",
                query.0 as u32
            )));
        }

        // Find the menu position whose canonical verb token is "restore". The token
        // is language-independent, unlike the displayed menu text.
        let count = GetMenuItemCount(Some(menu));
        let mut found: Option<usize> = None;
        for pos in 0..count.max(0) as usize {
            let mut verb = [0u8; 64];
            // QueryContextMenu used idCmdFirst = 1, so command id = pos + 1 and the
            // verb offset for InvokeCommand is pos.
            if ctx
                .GetCommandString(
                    pos + 1,
                    GCS_VERBA,
                    None,
                    windows::core::PSTR(verb.as_mut_ptr()),
                    verb.len() as u32,
                )
                .is_ok()
            {
                let end = verb.iter().position(|b| *b == 0).unwrap_or(verb.len());
                if &verb[..end] == b"restore" {
                    found = Some(pos);
                    break;
                }
            }
        }
        let _ = DestroyMenu(menu);

        let verb_offset = found.ok_or_else(|| {
            ShellRecycleError::NotAvailable("item exposes no restore verb".into())
        })?;

        let info = windows::Win32::UI::Shell::CMINVOKECOMMANDINFO {
            cbSize: std::mem::size_of::<windows::Win32::UI::Shell::CMINVOKECOMMANDINFO>() as u32,
            hwnd: HWND(ptr::null_mut()),
            // MAKEINTRESOURCEA(offset): a value with the high word zero is a verb
            // offset, not a string.
            lpVerb: PCSTR(verb_offset as *const u8),
            lpParameters: PCSTR(ptr::null()),
            lpDirectory: PCSTR(ptr::null()),
            nShow: 1, // SW_SHOWNORMAL
            // CMIC_MASK_FLAG_NO_UI: never raise Shell UI from an undo path.
            fMask: 0x0000_0400,
            ..Default::default()
        };
        ctx.InvokeCommand(&info)
            .map_err(|e| ShellRecycleError::NotAvailable(format!("InvokeCommand: {e}")))?;
        Ok(())
    }
}
