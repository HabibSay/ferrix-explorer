//! The OLE drag source: hands Ferrix's files to the shell so they can be dropped on
//! Explorer, the Desktop or another OLE application.
//!
//! Tranche 3, slice 2 (spec §7). The source is deliberately minimal: an `IDataObject`
//! that exposes `CF_HDROP` plus the registered `Preferred DropEffect` format, and an
//! `IDropSource` that reports the effect the drop would have.
//!
//! `CF_HDROP` is built from the same `DROPFILES` layout the clipboard writes, so the
//! shell reads the dropped paths exactly as it reads pasted ones; `clipboard.rs` stays
//! untouched (spec §7).
//!
//! The drag itself runs `DoDragDrop`, a modal pump that blocks until the user drops or
//! cancels (spec §6).

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::GlobalFree;
use windows::Win32::Foundation::{HGLOBAL, S_FALSE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, DATADIR_GET, DVASPECT_CONTENT, FORMATETC, IDataObject, IDataObject_Impl,
    IEnumFORMATETC, IEnumFORMATETC_Impl, IPersistFile, STGMEDIUM, TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE,
    IDropSource, IDropSource_Impl,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::{HRESULT, Interface, PCWSTR};
use windows_implement::implement;

use crate::drop_target::DragEffect;

/// DROPFILES structure layout (matching Windows DROPFILES).
#[repr(C)]
struct DropFiles {
    /// Offset of file list from the start of this structure.
    p_files: u32,
    /// X coordinate of the drop point.
    pt_x: i32,
    /// Y coordinate of the drop point.
    pt_y: i32,
    /// Non-client mouse position flag (unused; set to 0).
    f_nc: u32,
    /// TRUE if the file list is Unicode (UTF-16).
    f_wide: u32,
}

/// The data object the shell reads the dragged files from.
#[implement(IDataObject)]
struct DragDataObject {
    paths: Vec<PathBuf>,
    /// Shared with the drop source: the shell may change the effect mid-drag.
    effect: Arc<Mutex<DragEffect>>,
}

impl DragDataObject {
    /// The effect the drop currently would have.
    fn effect(&self) -> DragEffect {
        match self.effect.lock() {
            Ok(guard) => *guard,
            Err(_) => DragEffect::Copy,
        }
    }
}

#[allow(non_snake_case)]
impl IDataObject_Impl for DragDataObject_Impl {
    fn GetData(&self, format: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        if format.is_null() {
            return Err(windows::core::Error::from_hresult(HRESULT(
                0x80004005u32 as i32,
            )));
        }
        // SAFETY: the shell passes a readable FORMATETC that outlives the call.
        let format = unsafe { &*format };
        let preferred_format = preferred_effect_format();
        if format.cfFormat == CF_HDROP.0 {
            return Ok(self.hdrop_medium());
        }
        if Some(format.cfFormat) == preferred_format {
            return Ok(self.effect_medium());
        }
        Err(windows::core::Error::from_hresult(HRESULT(
            0x80004005u32 as i32,
        )))
    }

    fn GetDataHere(
        &self,
        format: *const FORMATETC,
        medium: *mut STGMEDIUM,
    ) -> windows::core::Result<()> {
        let data = self.GetData(format)?;
        if medium.is_null() {
            return Err(windows::core::Error::from_hresult(HRESULT(
                0x80004005u32 as i32,
            )));
        }
        // SAFETY: the shell passes a writable STGMEDIUM that outlives the call.
        unsafe { *medium = data };
        Ok(())
    }

    fn QueryGetData(&self, format: *const FORMATETC) -> HRESULT {
        if format.is_null() {
            return HRESULT(windows::Win32::Foundation::E_POINTER.0);
        }
        // SAFETY: the shell passes a readable FORMATETC that outlives the call.
        let format = unsafe { &*format };
        let known =
            format.cfFormat == CF_HDROP.0 || Some(format.cfFormat) == preferred_effect_format();
        if known {
            HRESULT(0)
        } else {
            HRESULT(windows::Win32::Foundation::E_NOTIMPL.0)
        }
    }

    fn GetCanonicalFormatEtc(&self, format: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        if format.is_null() || out.is_null() {
            return HRESULT(windows::Win32::Foundation::E_POINTER.0);
        }
        // SAFETY: both pointers are readable/writable for the duration of the call.
        let (format, out) = unsafe { (&*format, &mut *out) };
        *out = *format;
        HRESULT(0)
    }

    fn SetData(
        &self,
        _format: *const FORMATETC,
        _medium: *const STGMEDIUM,
        _release: windows::core::BOOL,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::from_hresult(HRESULT(
            0x80004005u32 as i32,
        )))
    }

    fn EnumFormatEtc(
        &self,
        direction: u32,
    ) -> windows::core::Result<windows::Win32::System::Com::IEnumFORMATETC> {
        // DATADIR_GET is the only direction a data object has to answer.
        if direction != DATADIR_GET.0 as u32 {
            return Err(windows::core::Error::from_hresult(HRESULT(
                0x80004005u32 as i32,
            )));
        }
        let enumerator: windows::Win32::System::Com::IEnumFORMATETC = FormatEnumerator {
            formats: offered_formats(),
            index: std::cell::Cell::new(0),
        }
        .into();
        Ok(enumerator)
    }

    fn DAdvise(
        &self,
        _format: *const FORMATETC,
        _flags: u32,
        _sink: windows::core::Ref<'_, windows::Win32::System::Com::IAdviseSink>,
    ) -> windows::core::Result<u32> {
        Err(windows::core::Error::from_hresult(HRESULT(
            0x80004005u32 as i32,
        )))
    }

    fn DUnadvise(&self, _connection: u32) -> windows::core::Result<()> {
        Err(windows::core::Error::from_hresult(HRESULT(
            0x80004005u32 as i32,
        )))
    }

    fn EnumDAdvise(&self) -> windows::core::Result<windows::Win32::System::Com::IEnumSTATDATA> {
        Err(windows::core::Error::from_hresult(HRESULT(
            0x80004005u32 as i32,
        )))
    }
}

impl DragDataObject {
    /// Builds the `CF_HDROP` medium the shell reads the paths from.
    fn hdrop_medium(&self) -> STGMEDIUM {
        let block = build_dropfiles(&self.paths);
        STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: windows::Win32::System::Com::STGMEDIUM_0 { hGlobal: block },
            pUnkForRelease: std::mem::ManuallyDrop::new(None),
        }
    }

    /// Builds the medium carrying the `Preferred DropEffect` value.
    fn effect_medium(&self) -> STGMEDIUM {
        let mut handle: HGLOBAL = HGLOBAL(std::ptr::null_mut());
        // SAFETY: a four-byte allocation always succeeds or is reported as an error.
        if let Ok(block) = unsafe { GlobalAlloc(GMEM_MOVEABLE, std::mem::size_of::<u32>()) } {
            // SAFETY: the block was just allocated and is locked exactly once.
            let pointer = unsafe { GlobalLock(block) };
            if !pointer.is_null() {
                // SAFETY: the lock succeeded, so the four bytes are writable.
                unsafe {
                    std::ptr::write(pointer.cast::<u32>(), self.effect().to_dropeffect().0);
                    let _ = GlobalUnlock(block);
                }
            }
            handle = block;
        }
        STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: windows::Win32::System::Com::STGMEDIUM_0 { hGlobal: handle },
            pUnkForRelease: std::mem::ManuallyDrop::new(None),
        }
    }
}

/// The registered clipboard format id of `Preferred DropEffect`.
fn preferred_effect_format() -> Option<u16> {
    let name: Vec<u16> = "Preferred DropEffect\0".encode_utf16().collect();
    // SAFETY: `name` is a NUL-terminated wide string that outlives the call.
    let id = unsafe { RegisterClipboardFormatW(PCWSTR(name.as_ptr())) };
    if id == 0 { None } else { Some(id as u16) }
}

/// One format the drag data object offers.
#[derive(Clone, Copy, Debug)]
struct FormatSpec {
    /// The clipboard format id.
    cf_format: u16,
    /// The aspect: content.
    aspect: u32,
    /// The medium: a global memory handle.
    tymed: u32,
}

/// The formats the drag data object offers, in the order the shell is told about them.
///
/// `CF_HDROP` comes first: it is the payload, and the registered
/// `Preferred DropEffect` second, the way Explorer publishes them.
fn offered_formats() -> Vec<FormatSpec> {
    let mut formats = vec![FormatSpec {
        cf_format: CF_HDROP.0,
        aspect: DVASPECT_CONTENT.0,
        tymed: TYMED_HGLOBAL.0 as u32,
    }];
    if let Some(preferred) = preferred_effect_format() {
        formats.push(FormatSpec {
            cf_format: preferred,
            aspect: DVASPECT_CONTENT.0,
            tymed: TYMED_HGLOBAL.0 as u32,
        });
    }
    formats
}

/// The format enumerator the shell asks the drag data object for.
///
/// A drop target asks for the formats before it reads them, so the enumerator is
/// part of the transport, not an extra.
#[implement(IEnumFORMATETC)]
struct FormatEnumerator {
    formats: Vec<FormatSpec>,
    index: std::cell::Cell<usize>,
}

#[allow(non_snake_case)]
impl IEnumFORMATETC_Impl for FormatEnumerator_Impl {
    fn Next(&self, celt: u32, rgelt: *mut FORMATETC, pceltfetched: *mut u32) -> HRESULT {
        let mut written = 0_u32;
        // SAFETY: the shell passes a buffer for `celt` entries and a counter; both
        // are checked for null, and at most `celt` entries are written.
        unsafe {
            while written < celt && self.index.get() < self.formats.len() {
                if rgelt.is_null() {
                    break;
                }
                let format = self.formats[self.index.get()];
                *rgelt.add(written as usize) = FORMATETC {
                    cfFormat: format.cf_format,
                    ptd: std::ptr::null_mut(),
                    dwAspect: format.aspect,
                    lindex: -1,
                    tymed: format.tymed,
                };
                self.index.set(self.index.get() + 1);
                written += 1;
            }
            if !pceltfetched.is_null() {
                *pceltfetched = written;
            }
        }
        if written < celt {
            // S_FALSE: the enumeration ended before `celt` entries.
            HRESULT(S_FALSE.0)
        } else {
            HRESULT(0)
        }
    }

    fn Skip(&self, celt: u32) -> windows::core::Result<()> {
        self.index.set(self.index.get() + celt as usize);
        Ok(())
    }

    fn Reset(&self) -> windows::core::Result<()> {
        self.index.set(0);
        Ok(())
    }

    fn Clone(&self) -> windows::core::Result<IEnumFORMATETC> {
        let clone: IEnumFORMATETC = FormatEnumerator {
            formats: self.formats.clone(),
            index: std::cell::Cell::new(self.index.get()),
        }
        .into();
        Ok(clone)
    }
}

/// Builds the `DROPFILES` block the shell reads the dragged paths from.
fn build_dropfiles(paths: &[PathBuf]) -> HGLOBAL {
    let mut wide_paths: Vec<u16> = Vec::new();
    for path in paths {
        wide_paths.extend(
            OsStr::new(path.as_os_str())
                .encode_wide()
                .collect::<Vec<u16>>(),
        );
        wide_paths.push(0);
    }
    wide_paths.push(0);

    let header_size = std::mem::size_of::<DropFiles>();
    let path_bytes = wide_paths.len() * std::mem::size_of::<u16>();
    let total_size = header_size + path_bytes;

    // SAFETY: the allocation size is non-zero and the result is checked below.
    let handle = match unsafe { GlobalAlloc(GMEM_MOVEABLE, total_size) } {
        Ok(handle) => handle,
        Err(_) => return HGLOBAL(std::ptr::null_mut()),
    };

    // SAFETY: the handle was just allocated and is locked exactly once.
    let pointer = unsafe { GlobalLock(handle) };
    if pointer.is_null() {
        // SAFETY: the allocation is ours and the lock failed, so it is freed here.
        unsafe {
            let _ = GlobalFree(Some(handle));
        }
        return HGLOBAL(std::ptr::null_mut());
    }

    // SAFETY: the block holds `total_size` bytes and the writes stay inside it.
    unsafe {
        let header = pointer.cast::<DropFiles>();
        std::ptr::write(
            header,
            DropFiles {
                p_files: header_size as u32,
                pt_x: 0,
                pt_y: 0,
                f_nc: 0,
                f_wide: 1,
            },
        );
        let destination = pointer.add(header_size).cast::<u16>();
        std::ptr::copy_nonoverlapping(wide_paths.as_ptr(), destination, wide_paths.len());
        let _ = GlobalUnlock(handle);
    }
    handle
}

/// The drop source the shell asks about the drag's effect.
#[implement(IDropSource)]
struct DragSource {
    effect: Arc<Mutex<DragEffect>>,
}

#[allow(non_snake_case)]
impl IDropSource_Impl for DragSource_Impl {
    fn QueryContinueDrag(
        &self,
        escape_pressed: windows::core::BOOL,
        _key_state: MODIFIERKEYS_FLAGS,
    ) -> HRESULT {
        if escape_pressed.as_bool() {
            // The user pressed Escape: the drag is cancelled.
            HRESULT(windows::Win32::Foundation::DRAGDROP_S_CANCEL.0)
        } else {
            HRESULT(windows::Win32::Foundation::DRAGDROP_S_DROP.0)
        }
    }

    fn GiveFeedback(&self, effect: DROPEFFECT) -> HRESULT {
        match DragEffect::from_dropeffect(effect) {
            Some(effect) => {
                if let Ok(mut current) = self.effect.lock() {
                    *current = effect;
                }
                HRESULT(0)
            }
            None => HRESULT(windows::Win32::Foundation::DRAGDROP_S_CANCEL.0),
        }
    }
}

/// The files and the effect of a drag that is about to start.
#[derive(Clone, Debug)]
pub struct DragSourceRequest {
    /// The dragged paths.
    pub paths: Vec<PathBuf>,
    /// The effect the drop has when the user holds no modifier key.
    pub effect: DragEffect,
}

/// What ended a drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragOutcome {
    /// The user dropped the files; the shell reports the effect that was applied.
    Dropped(DragEffect),
    /// The user cancelled the drag with Escape.
    Cancelled,
}

/// An `IDataObject` carrying `CF_HDROP` for `paths`, with no drag around it.
///
/// The Shell takes a data object where an array would be natural:
/// `SHMultiFileProperties` reads the selection to merge from exactly this format, so the
/// properties sheet reuses the drag source's object instead of building a second one.
/// The effect is `Copy` because nothing reads it here.
pub fn paths_data_object(paths: &[PathBuf]) -> IDataObject {
    DragDataObject {
        paths: paths.to_vec(),
        effect: Arc::new(Mutex::new(DragEffect::Copy)),
    }
    .into()
}

/// Runs an OLE drag.
///
/// Blocks until the user drops or cancels, because `DoDragDrop` is a modal pump
/// (spec §6). Must run on the thread that owns the application's OLE apartment.
pub fn run_drag(request: &DragSourceRequest) -> windows::core::Result<DragOutcome> {
    let effect = Arc::new(Mutex::new(request.effect));
    let data_object: IDataObject = DragDataObject {
        paths: request.paths.clone(),
        effect: Arc::clone(&effect),
    }
    .into();
    let source: IDropSource = DragSource { effect }.into();

    let mut effect = DROPEFFECT_NONE;
    // SAFETY: both objects are live COM objects for the duration of the call, and the
    // out-parameter points at a local variable.
    let result = unsafe {
        windows::Win32::System::Ole::DoDragDrop(
            &data_object,
            &source,
            DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK,
            &mut effect,
        )
    };
    if result.is_err() {
        return Err(windows::core::Error::from_hresult(HRESULT(
            0x80004005u32 as i32,
        )));
    }
    match DragEffect::from_dropeffect(effect) {
        Some(effect) => Ok(DragOutcome::Dropped(effect)),
        None => Ok(DragOutcome::Cancelled),
    }
}

/// The `.lnk` path a shortcut drop creates in `destination` for `source`.
///
/// Explorer's naming: the link keeps the whole name of the target and gains `.lnk`, and
/// a name that is already taken gets ` (2)`, ` (3)` … instead of replacing an existing
/// file. Returns `None` when the source has no file name of its own (a drive root), where
/// there is nothing sensible to call the link — the caller skips it rather than inventing
/// a name.
pub fn shortcut_link_path(destination: &Path, source: &Path) -> Option<PathBuf> {
    let name = source.file_name()?;

    let mut first = std::ffi::OsString::from(name);
    first.push(".lnk");
    let first = destination.join(first);
    if !first.exists() {
        return Some(first);
    }

    for counter in 2..=999_u32 {
        let mut candidate = std::ffi::OsString::from(name);
        candidate.push(format!(" ({counter}).lnk"));
        let candidate = destination.join(candidate);
        if !candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Creates a Windows shortcut (`.lnk`) at `link` pointing at `target`.
///
/// This is the half of the shortcut modifier that lands on *us*: when the drop
/// destination is an external application, that application builds the link from the
/// `DROPEFFECT_LINK` we advertise; when the destination is our own window, nobody else
/// will (spec §7). `IShellLinkW` + `IPersistFile` are the documented pair for writing
/// one.
///
/// The COM objects are created on the calling thread, which therefore needs an
/// apartment — the guard below is the PRD 10.5 discipline the other Shell helpers in
/// this crate already follow.
pub fn create_shortcut(target: &Path, link: &Path) -> windows::core::Result<()> {
    let _apartment = Apartment::new()?;
    let target_wide = wide_null(target);
    let link_wide = wide_null(link);
    unsafe {
        // SAFETY: the apartment guard is alive for the whole call, `target_wide` and
        // `link_wide` are NUL-terminated UTF-16 buffers that outlive the calls, and the
        // returned interface is only used inside this block.
        let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        shell.SetPath(PCWSTR(target_wide.as_ptr()))?;
        let persist: IPersistFile = shell.cast()?;
        persist.Save(PCWSTR(link_wide.as_ptr()), true)?;
    }
    Ok(())
}

/// A NUL-terminated UTF-16 copy of `path`, as the Shell's wide-string APIs expect it.
fn wide_null(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// STA guard for the shortcut writer: PRD 10.5 requires explicit initialization on
/// non-UI threads, and `CoCreateInstance` fails outright without it.
struct Apartment;

impl Apartment {
    fn new() -> windows::core::Result<Self> {
        // SAFETY: one COM initialization for this thread, paired with exactly one
        // CoUninitialize in `Drop`; the guard never leaves the calling thread.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        hr.ok()?;
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: paired 1:1 with the successful CoInitializeEx in `Apartment::new`.
        unsafe { CoUninitialize() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropfiles_layout_is_the_one_the_shell_expects() {
        let block = build_dropfiles(&[PathBuf::from(r"C:\a.txt"), PathBuf::from(r"C:\b.txt")]);
        assert!(!block.is_invalid());
        // SAFETY: the block was just built and is locked once.
        let pointer = unsafe { GlobalLock(block) };
        assert!(!pointer.is_null());
        // SAFETY: the lock succeeded, so the header is readable.
        let header = unsafe { &*pointer.cast::<DropFiles>() };
        assert_eq!(header.p_files as usize, std::mem::size_of::<DropFiles>());
        assert_eq!(header.f_wide, 1);
        assert_eq!(header.f_nc, 0);
        // SAFETY: the lock succeeded, so the path block is readable.
        let paths = unsafe {
            std::slice::from_raw_parts(
                pointer.add(std::mem::size_of::<DropFiles>()).cast::<u16>(),
                8,
            )
        };
        assert_eq!(
            paths,
            [
                b'C' as u16,
                b':' as u16,
                b'\\' as u16,
                b'a' as u16,
                b'.' as u16,
                b't' as u16,
                b'x' as u16,
                b't' as u16,
            ]
        );
        // SAFETY: the handle was locked above and is unlocked exactly once.
        unsafe {
            let _ = GlobalUnlock(block);
            let _ = GlobalFree(Some(block));
        }
    }

    /// A fresh workspace for the shortcut tests, honouring `TEMP`/`TMP` like the rest
    /// of the suite does.
    fn shortcut_workspace() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ferrix-drop-source-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("shortcut workspace");
        dir
    }

    /// Reads the target back out of a `.lnk` through the Shell (rule 8 evidence).
    fn read_shortcut(link: &Path) -> windows::core::Result<PathBuf> {
        use std::os::windows::ffi::OsStringExt;
        let _apartment = Apartment::new()?;
        let link_wide = wide_null(link);
        unsafe {
            // SAFETY: the apartment guard is alive, `link_wide` is NUL-terminated and
            // outlives the call, and `buffer` is a writable slice for `GetPath`.
            let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let persist: IPersistFile = shell.cast()?;
            persist.Load(
                PCWSTR(link_wide.as_ptr()),
                windows::Win32::System::Com::STGM_READ,
            )?;
            let mut buffer = vec![0u16; 32_768];
            shell.GetPath(&mut buffer, std::ptr::null_mut(), 0)?;
            let end = buffer
                .iter()
                .position(|&unit| unit == 0)
                .unwrap_or(buffer.len());
            Ok(PathBuf::from(std::ffi::OsString::from_wide(&buffer[..end])))
        }
    }

    #[test]
    fn a_shortcut_drop_writes_a_link_that_points_at_its_target() {
        let workspace = shortcut_workspace();
        let target = workspace.join("target.txt");
        std::fs::write(&target, b"payload").expect("write the target");

        let first = shortcut_link_path(&workspace, &target).expect("a name");
        assert_eq!(first, workspace.join("target.txt.lnk"));
        create_shortcut(&target, &first).expect("write the first shortcut");
        assert!(first.is_file(), "the link was not written");

        // A second drop of the same file must not replace the first link; the counter goes
        // before the `.lnk` extension, keeping the target's whole name (`target.txt (2).lnk`).
        let second = shortcut_link_path(&workspace, &target).expect("a name");
        assert_eq!(second, workspace.join("target.txt (2).lnk"));
        create_shortcut(&target, &second).expect("write the second shortcut");

        // The Shell must read the link back as pointing at the original file.
        let resolved = read_shortcut(&second).expect("read the link back");
        assert_eq!(resolved, target);

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn a_root_has_no_shortcut_name() {
        assert_eq!(
            shortcut_link_path(Path::new(r"C:\"), Path::new(r"C:\")),
            None
        );
    }
}
