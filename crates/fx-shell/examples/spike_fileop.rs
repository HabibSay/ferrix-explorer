//! Slice-0 spike: `IFileOperation` mechanics — Tranche 3 decisions 1, 3, 9 evidence.
//!
//! Measures on this machine: sink event order, copy metadata parity, move
//! post-conditions, long-path behaviour, and sink-driven cancellation. Prints
//! `FINDING |` lines intended for `docs/tranche-3-validation.md`.
//!
//! Usage: `cargo run -p fx-shell --example spike_fileop [-- copy|move|longpath|cancel]`

use std::{
    fs::{self, OpenOptions},
    os::windows::{ffi::OsStrExt, io::AsRawHandle},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use windows::{
    Win32::{
        Foundation::{E_ABORT, FILETIME, HANDLE},
        Storage::FileSystem::{GetFileAttributesW, SetFileTime},
        System::Com::{CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree},
        UI::Shell::{
            FOF_NOCONFIRMATION, FOF_SILENT, IFileOperation, IFileOperationProgressSink,
            IFileOperationProgressSink_Impl, IShellItem, SHCreateItemFromParsingName,
            SIGDN_FILESYSPATH,
        },
    },
    core::{GUID, PCWSTR, Result as WinResult, implement},
};

/// `CLSID_FileOperation` from `shobjidl.h`; the windows crate does not export it.
const CLSID_FILE_OPERATION: GUID = GUID::from_values(
    0x3AD05575,
    0x8857,
    0x4850,
    [0x92, 0x77, 0x11, 0xB8, 0x5B, 0xDB, 0x8E, 0x09],
);

fn main() {
    let which = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    // SAFETY: every test below runs COM on this thread; one initialization for the process.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    println!("CoInitializeEx: {hr:?}");

    let results = match which.as_str() {
        "copy" => vec![test_copy_metadata()],
        "move" => vec![test_move_tree_unicode()],
        "longpath" => vec![test_long_path()],
        "cancel" => vec![test_cancel()],
        _ => vec![
            test_copy_metadata(),
            test_move_tree_unicode(),
            test_long_path(),
            test_cancel(),
        ],
    };

    // SAFETY: pairs with the CoInitializeEx above on the same thread.
    unsafe { CoUninitialize() };

    for r in results {
        match r {
            Ok(()) => println!("RESULT | ok"),
            Err(e) => println!("RESULT | FAILED {e:?}"),
        }
    }
}

use windows::Win32::System::Com::CoUninitialize;

// ---------------------------------------------------------------- sink

#[implement(IFileOperationProgressSink)]
struct SpikeSink {
    copies_seen: AtomicU32,
    abort_after: u32,
}

impl SpikeSink {
    fn new(abort_after: u32) -> Self {
        Self {
            copies_seen: AtomicU32::new(0),
            abort_after,
        }
    }
}

#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for SpikeSink_Impl {
    fn StartOperations(&self) -> WinResult<()> {
        println!("  [sink] StartOperations");
        Ok(())
    }

    fn FinishOperations(&self, hrresult: windows::core::HRESULT) -> WinResult<()> {
        println!("  [sink] FinishOperations hr={hrresult:?}");
        Ok(())
    }

    fn PreRenameItem(
        &self,
        _flags: u32,
        item: windows::core::Ref<'_, IShellItem>,
        new_name: &PCWSTR,
    ) -> WinResult<()> {
        println!(
            "  [sink] PreRenameItem {} -> {}",
            item_path(&item),
            unsafe { new_name.to_string() }.unwrap_or_default()
        );
        Ok(())
    }

    fn PostRenameItem(
        &self,
        _flags: u32,
        item: windows::core::Ref<'_, IShellItem>,
        new_name: &PCWSTR,
        hr: windows::core::HRESULT,
        newly: windows::core::Ref<'_, IShellItem>,
    ) -> WinResult<()> {
        println!(
            "  [sink] PostRenameItem {} -> {} hr={hr:?} new={}",
            item_path(&item),
            unsafe { new_name.to_string() }.unwrap_or_default(),
            item_path(&newly)
        );
        Ok(())
    }

    fn PreMoveItem(
        &self,
        _flags: u32,
        item: windows::core::Ref<'_, IShellItem>,
        _dest: windows::core::Ref<'_, IShellItem>,
        new_name: &PCWSTR,
    ) -> WinResult<()> {
        println!(
            "  [sink] PreMoveItem {} dest_name={}",
            item_path(&item),
            unsafe { new_name.to_string() }.unwrap_or_default()
        );
        Ok(())
    }

    fn PostMoveItem(
        &self,
        _flags: u32,
        item: windows::core::Ref<'_, IShellItem>,
        _dest: windows::core::Ref<'_, IShellItem>,
        new_name: &PCWSTR,
        hr: windows::core::HRESULT,
        newly: windows::core::Ref<'_, IShellItem>,
    ) -> WinResult<()> {
        println!(
            "  [sink] PostMoveItem {} hr={hr:?} new={}",
            item_path(&item),
            item_path(&newly)
        );
        let _ = new_name;
        Ok(())
    }

    fn PreCopyItem(
        &self,
        _flags: u32,
        item: windows::core::Ref<'_, IShellItem>,
        _dest: windows::core::Ref<'_, IShellItem>,
        new_name: &PCWSTR,
    ) -> WinResult<()> {
        println!(
            "  [sink] PreCopyItem {} dest_name={}",
            item_path(&item),
            unsafe { new_name.to_string() }.unwrap_or_default()
        );
        Ok(())
    }

    fn PostCopyItem(
        &self,
        _flags: u32,
        item: windows::core::Ref<'_, IShellItem>,
        _dest: windows::core::Ref<'_, IShellItem>,
        new_name: &PCWSTR,
        hr: windows::core::HRESULT,
        newly: windows::core::Ref<'_, IShellItem>,
    ) -> WinResult<()> {
        println!(
            "  [sink] PostCopyItem {} hr={hr:?} new={}",
            item_path(&item),
            item_path(&newly)
        );
        let _ = new_name;
        if self.abort_after > 0 {
            let seen = self.copies_seen.fetch_add(1, Ordering::SeqCst) + 1;
            if seen >= self.abort_after {
                println!("  [sink] returning E_ABORT after {seen} copies (decision: cancel)");
                return Err(E_ABORT.into());
            }
        }
        Ok(())
    }

    fn PreDeleteItem(
        &self,
        _flags: u32,
        item: windows::core::Ref<'_, IShellItem>,
    ) -> WinResult<()> {
        println!("  [sink] PreDeleteItem {}", item_path(&item));
        Ok(())
    }

    fn PostDeleteItem(
        &self,
        _flags: u32,
        item: windows::core::Ref<'_, IShellItem>,
        hr: windows::core::HRESULT,
        newly: windows::core::Ref<'_, IShellItem>,
    ) -> WinResult<()> {
        println!(
            "  [sink] PostDeleteItem {} hr={hr:?} new={}",
            item_path(&item),
            item_path(&newly)
        );
        Ok(())
    }

    fn PreNewItem(
        &self,
        _flags: u32,
        _dest: windows::core::Ref<'_, IShellItem>,
        new_name: &PCWSTR,
    ) -> WinResult<()> {
        println!(
            "  [sink] PreNewItem {}",
            unsafe { new_name.to_string() }.unwrap_or_default()
        );
        Ok(())
    }

    fn PostNewItem(
        &self,
        _flags: u32,
        _dest: windows::core::Ref<'_, IShellItem>,
        new_name: &PCWSTR,
        _template: &PCWSTR,
        _attrs: u32,
        hr: windows::core::HRESULT,
        newly: windows::core::Ref<'_, IShellItem>,
    ) -> WinResult<()> {
        println!(
            "  [sink] PostNewItem {} hr={hr:?} new={}",
            unsafe { new_name.to_string() }.unwrap_or_default(),
            item_path(&newly)
        );
        Ok(())
    }

    fn UpdateProgress(&self, work_total: u32, work_so_far: u32) -> WinResult<()> {
        println!("  [sink] UpdateProgress {work_so_far}/{work_total}");
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

// ---------------------------------------------------------------- helpers

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn shell_item(path: &Path) -> WinResult<IShellItem> {
    let w = wide(path);
    // SAFETY: `w` stays alive for the call; SHCreateItemFromParsingName copies what it needs.
    unsafe { SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None) }
}

fn item_path(item: &windows::core::Ref<'_, IShellItem>) -> String {
    let Some(item) = item.as_ref() else {
        return "<null>".into();
    };
    // SAFETY: valid interface reference; the returned PWSTR is freed on every path below.
    unsafe {
        match item.GetDisplayName(SIGDN_FILESYSPATH) {
            Ok(p) => {
                let s = PCWSTR(p.0).to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0.cast()));
                s
            }
            Err(_) => "<no display name>".into(),
        }
    }
}

fn to_filetime(t: SystemTime) -> FILETIME {
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;
    let since_unix = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let intervals = since_unix.as_nanos() as u64 / 100 + EPOCH_DIFF;
    FILETIME {
        dwLowDateTime: intervals as u32,
        dwHighDateTime: (intervals >> 32) as u32,
    }
}

/// Sets creation and last-write times; `std` can only read creation time.
fn set_times(path: &Path, created: SystemTime, modified: SystemTime) -> std::io::Result<()> {
    let file = OpenOptions::new().write(true).open(path)?;
    let handle = HANDLE(file.as_raw_handle() as *mut _);
    let c = to_filetime(created);
    let m = to_filetime(modified);
    // SAFETY: `handle` is a live write handle owned by `file`; the FILETIME values
    // are in-process locals valid for the duration of the call.
    unsafe { SetFileTime(handle, Some(&c), None, Some(&m)) }.map_err(std::io::Error::from)
}

fn file_attributes(path: &Path) -> u32 {
    let w = wide(path);
    // SAFETY: `w` is a live NUL-terminated buffer for the call.
    unsafe { GetFileAttributesW(PCWSTR(w.as_ptr())) }
}

fn time_of(t: std::io::Result<SystemTime>) -> String {
    match t {
        Ok(v) => format!("{v:?}"),
        Err(e) => format!("ERR({e})"),
    }
}

fn new_op() -> WinResult<IFileOperation> {
    // SAFETY: CLSID is a compile-time constant from shobjidl.h; COM is initialized.
    let op: IFileOperation = unsafe { CoCreateInstance(&CLSID_FILE_OPERATION, None, CLSCTX_ALL)? };
    // SAFETY: flag combination is a plain value; no UI, no confirmations, for automation.
    unsafe { op.SetOperationFlags(FOF_NOCONFIRMATION | FOF_SILENT)? };
    Ok(op)
}

use windows::Win32::System::Com::CoCreateInstance;

fn spike_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "fx-spike-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&root).expect("spike root");
    root
}

// ---------------------------------------------------------------- tests

/// Copy one file; compare creation time, last-write time and attributes (decision 9 / rule 7).
fn test_copy_metadata() -> WinResult<()> {
    println!("=== TEST copy_metadata ===");
    let root = spike_root("copy");
    let src_dir = root.join("src");
    let dst_dir = root.join("dst");
    fs::create_dir_all(&src_dir).unwrap();
    fs::create_dir_all(&dst_dir).unwrap();

    let src = src_dir.join("échantillon-métadonnées.txt");
    let payload = vec![b'x'; 1024];
    fs::write(&src, &payload).unwrap();

    let created = UNIX_EPOCH + Duration::from_secs(1_577_836_800); // 2020-01-01
    let modified = UNIX_EPOCH + Duration::from_secs(1_622_937_600); // 2021-06-06
    set_times(&src, created, modified).unwrap();

    let src_before = fs::metadata(&src).unwrap();
    let attrs_before = file_attributes(&src);

    let op = new_op()?;
    let sink: IFileOperationProgressSink = SpikeSink::new(0).into();
    // SAFETY: sink is a live interface; global registration outlives PerformOperations.
    let cookie = unsafe { op.Advise(&sink)? };
    let src_item = shell_item(&src)?;
    let dst_item = shell_item(&dst_dir)?;
    // SAFETY: both shell items and the null new-name pointer are valid for the call.
    unsafe { op.CopyItem(&src_item, &dst_item, PCWSTR::null(), None)? };
    let started = std::time::Instant::now();
    // SAFETY: same live objects; PerformOperations runs the queued operation.
    let perform = unsafe { op.PerformOperations() };
    let elapsed = started.elapsed();
    // SAFETY: same operation object, queried after PerformOperations returned.
    let aborted = unsafe { op.GetAnyOperationsAborted().unwrap_or_default() };
    // SAFETY: cookie came from Advise on this object.
    unsafe { op.Unadvise(cookie)? };

    let dst = dst_dir.join("échantillon-métadonnées.txt");
    let dst_meta = fs::metadata(&dst).ok();
    let attrs_after = file_attributes(&dst);

    println!("  PerformOperations: {perform:?} elapsed={elapsed:?} aborted={aborted:?}");
    match &dst_meta {
        Some(m) => println!(
            "  dest created={} modified={} len={}",
            time_of(Ok(m.created().unwrap())),
            time_of(Ok(m.modified().unwrap())),
            m.len()
        ),
        None => println!("  dest MISSING"),
    }
    println!(
        "  src  created={} modified={} attrs=0x{attrs_before:X}",
        time_of(Ok(src_before.created().unwrap())),
        time_of(Ok(src_before.modified().unwrap()))
    );
    println!("  dest attrs=0x{attrs_after:X}");

    let created_parity = dst_meta
        .as_ref()
        .and_then(|m| m.created().ok())
        .is_some_and(|c| c == src_before.created().unwrap());
    let mtime_parity = dst_meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .is_some_and(|m| m == src_before.modified().unwrap());
    let readonly_copied = attrs_after & 0x1 != 0 || attrs_before & 0x1 == 0;
    println!(
        "FINDING | copy: created_parity={created_parity} mtime_parity={mtime_parity} readonly_copied={readonly_copied} perform={perform:?} abort_flag={aborted:?}"
    );

    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// Move a Unicode tree; source must be gone, destination complete (post-conditions).
fn test_move_tree_unicode() -> WinResult<()> {
    println!("=== TEST move_tree_unicode ===");
    let root = spike_root("move");
    let src_dir = root.join("source");
    let dst_parent = root.join("target");
    fs::create_dir_all(&src_dir).unwrap();
    fs::create_dir_all(&dst_parent).unwrap();

    let tree = src_dir.join("dossier éàç");
    fs::create_dir_all(tree.join("子目录")).unwrap();
    fs::write(tree.join("file-α.txt"), b"alpha").unwrap();
    fs::write(tree.join("子目录").join("файл.bin"), vec![7u8; 4096]).unwrap();

    let op = new_op()?;
    let sink: IFileOperationProgressSink = SpikeSink::new(0).into();
    // SAFETY: live sink registered for the whole operation.
    let cookie = unsafe { op.Advise(&sink)? };
    let src_item = shell_item(&tree)?;
    let dst_item = shell_item(&dst_parent)?;
    // SAFETY: live items, null new name (keep the folder's own name).
    unsafe { op.MoveItem(&src_item, &dst_item, PCWSTR::null(), None)? };
    let perform = unsafe { op.PerformOperations() };
    let aborted = unsafe { op.GetAnyOperationsAborted().unwrap_or_default() };
    // SAFETY: cookie from Advise on this object.
    unsafe { op.Unadvise(cookie)? };

    let moved = dst_parent.join("dossier éàç");
    let src_gone = !tree.exists();
    let dest_ok = moved.join("file-α.txt").exists()
        && moved.join("子目录").join("файл.bin").exists()
        && fs::metadata(moved.join("子目录").join("файл.bin")).is_ok_and(|m| m.len() == 4096);
    println!(
        "  PerformOperations: {perform:?} aborted={aborted:?} src_gone={src_gone} dest_ok={dest_ok}"
    );
    println!("FINDING | move: src_gone={src_gone} dest_complete={dest_ok} perform={perform:?}");

    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// Long-path copy: display form vs `\\?\` verbatim form (Tranche 2's flagged weakness).
fn test_long_path() -> WinResult<()> {
    println!("=== TEST long_path ===");
    let temp = std::env::temp_dir();
    let verbatim_base = format!(r"\\?\{}", temp.display());
    let base = PathBuf::from(&verbatim_base).join(format!("fx-spike-long-{}", std::process::id()));
    fs::create_dir_all(&base).unwrap();

    let mut deep = base.clone();
    let mut i = 0u32;
    while deep.as_os_str().len() < 290 {
        deep = deep.join(format!("seg{i:02}"));
        // SAFETY-free: std with a verbatim prefix handles >260 paths on modern Windows.
        if let Err(e) = fs::create_dir(&deep) {
            println!(
                "  fixture creation stopped at len={} err={e}",
                deep.as_os_str().len()
            );
            break;
        }
        i += 1;
    }
    let deep_file = deep.join("deep-é.txt");
    if let Err(e) = fs::write(&deep_file, b"deep payload") {
        println!(
            "FINDING | longpath: fixture write failed at len={} err={e}",
            deep.as_os_str().len()
        );
        let _ = fs::remove_dir_all(&base);
        return Ok(());
    }
    println!("  fixture len={}", deep_file.as_os_str().len());

    let dst_dir = root_short(&temp);
    fs::create_dir_all(&dst_dir).unwrap();

    // Attempt A: display form (no verbatim prefix), as the UI would present it.
    let display_str = deep_file
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string();
    let attempt = |label: &str, src_path: &Path| -> String {
        let op = match new_op() {
            Ok(op) => op,
            Err(e) => return format!("{label}: new_op failed {e:?}"),
        };
        let src_item = match shell_item(src_path) {
            Ok(i) => i,
            Err(e) => return format!("{label}: shell_item failed {e:?}"),
        };
        let dst_item = match shell_item(&dst_dir) {
            Ok(i) => i,
            Err(e) => return format!("{label}: dest failed {e:?}"),
        };
        // SAFETY: live items; null new name; per-item none, global sink not needed here.
        let queued = unsafe { op.CopyItem(&src_item, &dst_item, PCWSTR::null(), None) };
        if let Err(e) = queued {
            return format!("{label}: CopyItem queued error {e:?}");
        }
        // SAFETY: same live objects.
        let perform = unsafe { op.PerformOperations() };
        let aborted = unsafe { op.GetAnyOperationsAborted().unwrap_or_default() };
        let dest_exists = dst_dir
            .join(src_path.file_name().unwrap_or_default())
            .exists();
        format!("{label}: perform={perform:?} aborted={aborted:?} dest_exists={dest_exists}")
    };

    let a = attempt("display-form", Path::new(&display_str));
    println!("  {a}");
    let b = attempt("verbatim-form", &deep_file);
    println!("  {b}");

    let display_works = a.contains("dest_exists=true");
    let verbatim_works = b.contains("dest_exists=true");
    println!(
        "FINDING | longpath: display_form={display_works} verbatim={verbatim_works} (fixture_len={})",
        deep_file.as_os_str().len()
    );

    let _ = fs::remove_dir_all(&dst_dir);
    let _ = fs::remove_dir_all(&base);
    Ok(())
}

fn root_short(temp: &Path) -> PathBuf {
    temp.join(format!("fx-spike-longdst-{}", std::process::id()))
}

/// Copy 40 files; the sink returns E_ABORT after the 3rd completed copy.
fn test_cancel() -> WinResult<()> {
    println!("=== TEST cancel_via_sink ===");
    let root = spike_root("cancel");
    let src_dir = root.join("src");
    let dst_dir = root.join("dst");
    fs::create_dir_all(&src_dir).unwrap();
    fs::create_dir_all(&dst_dir).unwrap();
    for i in 0..40 {
        fs::write(src_dir.join(format!("f{i:02}.bin")), vec![b'a'; 512]).unwrap();
    }

    let op = new_op()?;
    let sink: IFileOperationProgressSink = SpikeSink::new(3).into();
    // SAFETY: live sink registered globally; unadvised below on every path.
    let cookie = unsafe { op.Advise(&sink)? };

    let mut queue_error: Option<windows::core::Error> = None;
    for entry in fs::read_dir(&src_dir).unwrap() {
        let path = entry.unwrap().path();
        let src_item = match shell_item(&path) {
            Ok(i) => i,
            Err(e) => {
                queue_error = Some(e);
                break;
            }
        };
        let dst_item = match shell_item(&dst_dir) {
            Ok(i) => i,
            Err(e) => {
                queue_error = Some(e);
                break;
            }
        };
        // SAFETY: live items for the call; null new name.
        if let Err(e) = unsafe { op.CopyItem(&src_item, &dst_item, PCWSTR::null(), None) } {
            queue_error = Some(e);
            break;
        }
    }
    if let Some(e) = queue_error {
        println!("  queue failed early: {e:?}");
    }

    let started = std::time::Instant::now();
    // SAFETY: queued items and sink are live.
    let perform = unsafe { op.PerformOperations() };
    let elapsed = started.elapsed();
    // SAFETY: same object post-perform.
    let aborted = unsafe { op.GetAnyOperationsAborted().unwrap_or_default() };
    // SAFETY: cookie from Advise.
    unsafe { op.Unadvise(cookie)? };

    let copied = fs::read_dir(&dst_dir).unwrap().count();
    println!(
        "  PerformOperations: {perform:?} elapsed={elapsed:?} aborted={aborted:?} files_in_dest={copied}/40"
    );
    println!(
        "FINDING | cancel: sink_E_ABORT stops the run; perform={perform:?} aborted={aborted:?} copied_before_stop={copied} (partial output remains on disk)"
    );

    let _ = fs::remove_dir_all(&root);
    Ok(())
}
