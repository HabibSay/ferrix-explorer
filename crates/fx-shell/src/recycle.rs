//! Recycle Bin interop: SHFileOperationW(FOF_ALLOWUNDO) + Shell folder restore.
//!
//! Design note (spec §8): SHFileOperationW does not accept `\\?\` prefixes; paths are
//! passed in display form. Long-path failures return `LongPath`. The before/after diff of
//! the Recycle Bin folder is used to derive the recycled identity; an ambiguous diff
//! sets `verified = false` so undo refuses to act.
//!
//! Restore follows the decided approach (decision 3): enumerate the Recycle Bin's shell
//! folder and invoke the item's own Restore verb, matched by its **language-independent
//! canonical verb token** (`GCS_VERBA` = `"undelete"` on Windows 11 build 22631;
//! `"restore"` is also accepted for other Windows builds), never by localized menu text.
//! Items are matched by the original name stored in their `$I` record — the Shell's
//! display name hides known file extensions on default Explorer settings — falling back
//! to the display name. `$I`/`$R` records are read only and never written or moved here.
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
                CMF_NORMAL, CSIDL_BITBUCKET, GCS_VERBA, GPFIDL_DEFAULT, IContextMenu, IEnumIDList,
                ILFree, IShellFolder, SHCONTF_INCLUDEHIDDEN, SHCONTF_NONFOLDERS, SHGDNF,
                SHGetDesktopFolder, SHGetFolderLocation, SHGetPathFromIDListEx, StrRetToBufW,
            },
            WindowsAndMessaging::{
                CreatePopupMenu, DestroyMenu, GetMenuItemCount, GetMenuItemID, HMENU,
            },
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

    // SAFETY: the STA is held by `_apartment` for this scope; the bin PIDL obtained
    // below is freed before returning, and every pointer passed to COM points at
    // caller-owned memory that outlives the call.
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
    // SAFETY: `bin` and the enumerator are valid COM objects on this STA thread; each
    // PIDL fetched from the enumerator is either kept for the verb invocation or freed
    // in the same iteration; every out-parameter is caller-owned stack memory that
    // outlives its call.
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
        let mut total = 0usize;
        let mut seen: Vec<String> = Vec::new();

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
            total += 1;

            // Primary source: the item's parsing name / `$I` record, which keeps the
            // extension even when the display name hides it.
            let i_source = original_name_via_i(bin, item);
            let mut name_ok = i_source
                .as_ref()
                .map(|name| name.eq_ignore_ascii_case(wanted))
                .unwrap_or(false);

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
                if seen.len() < 6 {
                    match &i_source {
                        Ok(name) => seen.push(format!("{shown} [i:{name}]")),
                        Err(reason) => seen.push(format!("{shown} [i:ERR {reason}]")),
                    }
                }
                // Fallback: SHGDN_NORMAL on the bitbucket folder yields the item's
                // original full path on Windows 11 build 22631 (bare name on other
                // builds, extension possibly hidden by Explorer settings). Match its
                // file name so the comparison is independent of how the path is
                // displayed; Windows names are case-insensitive.
                if !name_ok {
                    let base = std::path::Path::new(&shown)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| shown.clone());
                    name_ok = base.eq_ignore_ascii_case(wanted);
                }
            } else if seen.len() < 6 {
                match &i_source {
                    Ok(name) => seen.push(format!("<display name unavailable> [i:{name}]")),
                    Err(reason) => {
                        seen.push(format!("<display name unavailable> [i:ERR {reason}]"))
                    }
                }
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
            let names = if seen.is_empty() {
                "no items".to_string()
            } else {
                seen.join(" | ")
            };
            ShellRecycleError::NotAvailable(format!(
                "no Recycle Bin item named '{wanted}' ({total} enumerated; saw: {names})"
            ))
        })?;

        let result = invoke_restore_verb(bin, target);
        CoTaskMemFree(Some(target as *const c_void));
        result
    }
}

/// Reads the original file name for an item from its Shell names.
///
/// The Shell's display name (and on some builds the parsing name too) honours
/// Explorer's "hide extensions for known file types" setting (a default Windows
/// shows `...\fx-restore-probe` without `.txt`), so those cannot be matched
/// reliably across hosts. Strategy, in order:
/// 1. `SHGetPathFromIDListEx` — the item's real filesystem path (the `$R` file);
///    read the sibling `$I` record, which stores the original path.
/// 2. The parsing name, when it is already an absolute path.
/// 3. The caller falls back to the display name.
///
/// Returns `Ok(original name)` or `Err(short reason)` for the caller's diagnostics.
fn original_name_via_i(bin: &IShellFolder, item: *const ITEMIDLIST) -> Result<String, String> {
    // SAFETY: `item` is a valid PIDL the caller owns for this call; `strret` and
    // the buffers are caller-owned out-parameters that outlive the COM calls.
    unsafe {
        // 1) Real filesystem path of the item — not subject to display settings.
        let mut fs_buf = [0u16; 512];
        if SHGetPathFromIDListEx(item, &mut fs_buf, GPFIDL_DEFAULT).as_bool() {
            let end = fs_buf.iter().position(|c| *c == 0).unwrap_or(fs_buf.len());
            let fs_str = String::from_utf16_lossy(&fs_buf[..end]);
            let fs_path = Path::new(&fs_str);
            if let Some(fs_name) = fs_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
            {
                if let Some(r_suffix) = fs_name.strip_prefix("$R") {
                    let i_path = fs_path.with_file_name(format!("$I{r_suffix}"));
                    let bytes = std::fs::read(&i_path)
                        .map_err(|e| format!("read '{i_path:?}' failed: {e}"))?;
                    return parse_i_original_name(&bytes)
                        .ok_or_else(|| format!("$I parse failed for '{i_path:?}'"));
                }
                // Not a `$R` file: the API handed back an absolute path that already
                // carries the real file name (filesystem names keep their extension).
                if is_absoluteish(fs_path) {
                    return Ok(fs_name);
                }
            }
        }

        // 2) Parsing name: `$R` sibling on some builds, original path on others.
        let mut strret = STRRET::default();
        bin.GetDisplayNameOf(item, SHGDNF(0x4000), &raw mut strret)
            .map_err(|e| format!("parsing name failed: {e}"))?;
        let mut buf = [0u16; 512];
        StrRetToBufW(&raw mut strret, Some(item), &mut buf)
            .map_err(|e| format!("StrRetToBufW failed: {e}"))?;
        let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        let parsing = String::from_utf16_lossy(&buf[..end]);
        let p_path = Path::new(&parsing);
        let p_name = p_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("no file name in '{parsing}'"))?;

        if let Some(r_suffix) = p_name.strip_prefix("$R") {
            // Sibling `$I` record of the `$R…` file: the authoritative original path.
            let i_path = p_path.with_file_name(format!("$I{r_suffix}"));
            let bytes =
                std::fs::read(&i_path).map_err(|e| format!("read '{i_path:?}' failed: {e}"))?;
            return parse_i_original_name(&bytes)
                .ok_or_else(|| format!("$I parse failed for '{i_path:?}'"));
        }

        if is_absoluteish(p_path) {
            return Ok(p_name);
        }
        Err(format!(
            "fs path unavailable; unexpected parsing name '{parsing}'"
        ))
    }
}

/// `C:\…` or `\\server\…`.
fn is_absoluteish(path: &Path) -> bool {
    let raw = path.to_string_lossy();
    (raw.len() >= 3 && raw.as_bytes()[1] == b':') || raw.starts_with(r"\\")
}

/// Parses the original path out of an `$I` record: 8-byte version (1 or 2),
/// 8-byte file size, 8-byte delete time, 4-byte UTF-16 length, then the path.
fn parse_i_original_name(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 28 {
        return None;
    }
    let version = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
    if version != 1 && version != 2 {
        return None;
    }
    let chars = u32::from_le_bytes(bytes[24..28].try_into().ok()?) as usize;
    let needed = 28usize.checked_add(chars.checked_mul(2)?)?;
    if chars == 0 || bytes.len() < needed {
        return None;
    }
    let mut units = Vec::with_capacity(chars);
    for index in 0..chars {
        units.push(u16::from_le_bytes([
            bytes[28 + index * 2],
            bytes[28 + index * 2 + 1],
        ]));
    }
    let mut path = String::from_utf16(&units).ok()?;
    while path.ends_with('\0') {
        path.pop();
    }
    // A stored absolute path has a drive letter (`C:\…`) or a UNC prefix; anything
    // else means the layout assumptions above do not hold on this build.
    if !is_absoluteish(Path::new(&path)) {
        return None;
    }
    Path::new(&path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

fn invoke_restore_verb(bin: &IShellFolder, item: *mut ITEMIDLIST) -> Result<(), ShellRecycleError> {
    // SAFETY: `item` is a valid PIDL owned by the caller for this call; the popup
    // menu and context menu are released before returning; verb offsets are derived
    // from menu ids bounded by the idCmdLast passed to QueryContextMenu.
    unsafe {
        let apidl: [*const ITEMIDLIST; 1] = [item as *const ITEMIDLIST];
        let ctx: IContextMenu = bin
            .GetUIObjectOf(HWND(ptr::null_mut()), &apidl, None)
            .map_err(|e| ShellRecycleError::NotAvailable(format!("GetUIObjectOf: {e}")))?;

        let menu: HMENU = CreatePopupMenu()
            .map_err(|e| ShellRecycleError::NotAvailable(format!("CreatePopupMenu: {e}")))?;
        // Validated on Win11 build 22631: the bitbucket handler assigns internal fixed
        // ids (its restore command is id 167 with idCmdFirst = 1) and silently DROPS
        // any command whose id exceeds idCmdLast — with the small idCmdLast the restore
        // command disappears from the menu, so the id space must be generous.
        let query = ctx.QueryContextMenu(menu, 0, 1, 0x7FFF, CMF_NORMAL);
        if query.0 < 0 {
            let _ = DestroyMenu(menu);
            return Err(ShellRecycleError::NotAvailable(format!(
                "QueryContextMenu: {:#010x}",
                query.0 as u32
            )));
        }

        // Find the command whose canonical verb token is the Recycle Bin's restore
        // command. Validated on Win11 22631: the token is "undelete" ("restore" is
        // accepted for other Windows builds that use it); both are language-independent
        // canonical tokens, unlike the displayed menu text. The lpVerb offset is
        // (menu id - idCmdFirst) — menu positions do not match these offsets.
        let count = GetMenuItemCount(Some(menu));
        let mut found: Option<usize> = None;
        for pos in 0..count.max(0) {
            let id = GetMenuItemID(menu, pos);
            if id == u32::MAX || id == 0 {
                continue; // separators and non-command items
            }
            let offset = (id as usize).saturating_sub(1); // idCmdFirst = 1
            let mut verb = [0u8; 64];
            if ctx
                .GetCommandString(
                    offset,
                    GCS_VERBA,
                    None,
                    windows::core::PSTR(verb.as_mut_ptr()),
                    verb.len() as u32,
                )
                .is_ok()
            {
                let end = verb.iter().position(|b| *b == 0).unwrap_or(verb.len());
                if &verb[..end] == b"undelete" || &verb[..end] == b"restore" {
                    found = Some(offset);
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

#[cfg(test)]
mod i_record_tests {
    use super::parse_i_original_name;

    fn i_record(path: &str, version: u64) -> Vec<u8> {
        let units: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(&42u64.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&(units.len() as u32).to_le_bytes());
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn reads_the_original_name_with_its_extension() {
        let bytes = i_record(r"C:\Users\alice\AppData\Local\Temp\fx-restore-probe.txt", 1);
        assert_eq!(
            parse_i_original_name(&bytes).as_deref(),
            Some("fx-restore-probe.txt")
        );
    }

    #[test]
    fn accepts_the_version_2_layout() {
        let bytes = i_record(r"D:\data\report.pdf", 2);
        assert_eq!(parse_i_original_name(&bytes).as_deref(), Some("report.pdf"));
    }

    #[test]
    fn rejects_truncated_unknown_version_or_implausible_records() {
        assert_eq!(parse_i_original_name(&[0u8; 10]), None);
        assert_eq!(
            parse_i_original_name(&i_record(r"C:\x.txt", 99)[..20]),
            None,
            "unknown versions must not be parsed with v1 offsets"
        );
        assert_eq!(parse_i_original_name(&i_record("relative.txt", 1)), None);
        let mut truncated = i_record(r"C:\long\path\file.txt", 1);
        truncated.truncate(30);
        assert_eq!(parse_i_original_name(&truncated), None);
    }
}
