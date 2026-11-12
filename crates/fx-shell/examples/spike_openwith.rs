//! Slice-0 spike: "Open with" and native properties — Tranche 3 decision on
//! OPS-14/OPS-15 and the multi-select properties question (spec §7, §15).
//!
//! `--interactive` opens the real dialogs on the desktop: close each one to let
//! the spike proceed. Prints `FINDING |` lines for `docs/tranche-3-validation.md`.
//!
//! Usage:
//!   cargo run -p fx-shell --example spike_openwith -- <path>            (compile gate only)
//!   cargo run -p fx-shell --example spike_openwith -- --interactive <path> [more paths...]

use std::{
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::HWND,
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx},
        UI::{
            Shell::{
                Common::ITEMIDLIST, GCS_VERBA, IContextMenu, OAIF_ALLOW_REGISTRATION, OPENASINFO,
                SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW, SHGetDesktopFolder, SHOpenWithDialog,
                ShellExecuteExW,
            },
            WindowsAndMessaging::{
                CreatePopupMenu, DestroyMenu, DispatchMessageW, MSG, PM_REMOVE, PeekMessageW,
                SW_SHOWNORMAL, TranslateMessage, WM_QUIT,
            },
        },
    },
    core::{PCSTR, PCWSTR, PSTR, w},
};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let interactive = args.first().is_some_and(|a| a == "--interactive");
    if interactive {
        args.remove(0);
    }
    let paths: Vec<PathBuf> = args.iter().map(PathBuf::from).collect();
    if paths.is_empty() {
        eprintln!("usage: spike_openwith [--interactive] <path> [more paths...]");
        std::process::exit(2);
    }

    println!("spike_openwith interactive={interactive} paths={paths:?}");

    // SAFETY: one COM initialization for this thread; all shell calls below run here.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    println!("CoInitializeEx: {hr:?}");

    if !interactive {
        println!(
            "FINDING | compile/link gate: SHOpenWithDialog, ShellExecuteEx(properties) and IContextMenu::InvokeCommand are all resolved by the linker; runtime behaviour needs --interactive on a desktop session (rule 8: no claim without a run)"
        );
        // SAFETY: pairs with the CoInitializeEx above.
        unsafe { windows::Win32::System::Com::CoUninitialize() };
        return;
    }

    // --- 1. SHOpenWithDialog (OPS-15) ------------------------------------
    let file_w = wide(&paths[0]);
    let info = OPENASINFO {
        pcszFile: PCWSTR(file_w.as_ptr()),
        pcszClass: PCWSTR::null(),
        oaifInFlags: OAIF_ALLOW_REGISTRATION,
    };
    println!(
        "OPENWITH | calling SHOpenWithDialog for {} (close the dialog to continue)...",
        paths[0].display()
    );
    let started = Instant::now();
    // SAFETY: `file_w` stays alive for the call; SHOpenWithDialog blocks until
    // the dialog closes and returns the dialog's result.
    let open_with = unsafe { SHOpenWithDialog(None, &info) };
    let open_with_ms = started.elapsed().as_millis();
    println!("  SHOpenWithDialog: {open_with:?} after {open_with_ms}ms");
    println!("FINDING | openwith: hr={open_with:?} blocked_ms={open_with_ms} (modeless if ~0ms)");

    // --- 2. ShellExecuteEx verb "properties" (OPS-14, single selection) ---
    let prop_w = wide(&paths[0]);
    let mut exec = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_INVOKEIDLIST,
        hwnd: HWND::default(),
        lpVerb: w!("properties"),
        lpFile: PCWSTR(prop_w.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    println!(
        "PROPS-SEE | ShellExecuteEx(properties) for {} (close the sheet to continue)...",
        paths[0].display()
    );
    let started = Instant::now();
    // SAFETY: `prop_w` and `exec` are live for the call; the struct is fully
    // initialized above with cbSize set as the API requires.
    let see = unsafe { ShellExecuteExW(&mut exec) };
    let see_ms = started.elapsed().as_millis();
    println!("  ShellExecuteExW: {see:?} returned after {see_ms}ms");
    if see_ms < 500 {
        // Returned immediately: the sheet is modeless on this thread's queue.
        println!("PROPS-SEE | pumping messages for 45s so the modeless sheet stays alive...");
        pump(Duration::from_secs(45));
        println!("PROPS-SEE | pump finished");
    }
    println!(
        "FINDING | properties_see: hr={see:?} returned_after_ms={see_ms} (modeless if <500ms)"
    );

    // --- 3. Properties through the merged menu (multi-select candidate) ---
    if paths.len() >= 2 {
        match invoke_properties_via_menu(&paths) {
            Ok((ms, outcome)) => {
                println!("PROPS-MENU | InvokeCommand returned after {ms}ms: {outcome}");
                if ms < 500 {
                    println!("PROPS-MENU | pumping messages for 45s so the sheet stays alive...");
                    pump(Duration::from_secs(45));
                }
                println!(
                    "FINDING | properties_multi: invoke_after_ms={ms} outcome={outcome} paths={}",
                    paths.len()
                );
            }
            Err(e) => println!("FINDING | properties_multi failed: {e:?}"),
        }
    } else {
        println!(
            "FINDING | properties_multi: skipped (pass 2+ paths to exercise the combined sheet)"
        );
    }

    // SAFETY: pairs with the CoInitializeEx above.
    unsafe { windows::Win32::System::Com::CoUninitialize() };
}

// ---------------------------------------------------------------- menu invoke

/// Builds the merged menu for all paths, finds the canonical `properties` verb
/// and invokes it — the same machinery the product's context menu will use for
/// multi-select properties if ShellExecuteEx cannot take an array.
fn invoke_properties_via_menu(paths: &[PathBuf]) -> windows::core::Result<(u128, String)> {
    // SAFETY: caller initialized COM on this thread.
    let desktop = unsafe { SHGetDesktopFolder()? };
    let mut pidls: Vec<*const ITEMIDLIST> = Vec::new();
    for path in paths {
        let w = wide(path);
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        // SAFETY: `w` live for the call; pidl written by the callee.
        unsafe {
            desktop.ParseDisplayName(
                HWND::default(),
                None,
                PCWSTR(w.as_ptr()),
                None,
                &mut pidl,
                std::ptr::null_mut(),
            )?;
        }
        pidls.push(pidl as *const ITEMIDLIST);
    }
    // SAFETY: live pidls for the call only.
    let menu: IContextMenu = unsafe { desktop.GetUIObjectOf(HWND::default(), &pidls, None)? };
    for pidl in pidls {
        // SAFETY: allocated by ParseDisplayName, freed exactly once here.
        unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(pidl.cast())) };
    }

    // SAFETY: popup created on this thread.
    let hmenu = unsafe { CreatePopupMenu()? };
    // CMF flags 0 == CMF_NORMAL, as Explorer sends.
    // SAFETY: live menu and interface.
    let hr = unsafe { menu.QueryContextMenu(hmenu, 0, 0x1000, 0xFFFF, 0) };
    let count = (hr.0 & 0xFFFF) as u32;
    // GetCommandString takes the offset relative to idCmdFirst (0x1000), exactly
    // as fx-shell/src/recycle.rs does — passing the absolute id silently fails
    // for every entry and yields a false "verb not found".
    let mut properties_offset: Option<usize> = None;
    let mut verbs_resolved = 0u32;
    for i in 0..count {
        let mut vbuf = [0u8; 256];
        // SAFETY: writable ANSI buffer; offset = id - idCmdFirst.
        let ok = unsafe {
            menu.GetCommandString(
                i as usize,
                GCS_VERBA,
                None,
                PSTR(vbuf.as_mut_ptr()),
                vbuf.len() as u32,
            )
        };
        if ok.is_ok() {
            verbs_resolved += 1;
            let end = vbuf.iter().position(|&b| b == 0).unwrap_or(vbuf.len());
            let verb = String::from_utf8_lossy(&vbuf[..end]);
            if verb.eq_ignore_ascii_case("properties") {
                properties_offset = Some(i as usize);
                break;
            }
        }
    }
    // SAFETY: menu no longer needed after the id lookup.
    unsafe { DestroyMenu(hmenu)? };

    let Some(offset) = properties_offset else {
        return Ok((
            0,
            format!("no properties verb among {count} entries ({verbs_resolved} verbs resolved)"),
        ));
    };

    let info = windows::Win32::UI::Shell::CMINVOKECOMMANDINFO {
        cbSize: std::mem::size_of::<windows::Win32::UI::Shell::CMINVOKECOMMANDINFO>() as u32,
        fMask: 0,
        hwnd: HWND::default(),
        // MAKEINTRESOURCEA(offset): a high-word-zero value is a verb offset
        // relative to idCmdFirst, not a string — as recycle.rs does.
        lpVerb: PCSTR(offset as *const u8),
        lpParameters: PCSTR::null(),
        lpDirectory: PCSTR::null(),
        nShow: SW_SHOWNORMAL.0,
        dwHotKey: 0,
        hIcon: Default::default(),
    };
    println!(
        "PROPS-MENU | invoking properties offset={offset} for {} paths",
        paths.len()
    );
    let started = Instant::now();
    // SAFETY: live interface and initialized CMINVOKECOMMANDINFO.
    let invoked = unsafe { menu.InvokeCommand(&info) };
    let ms = started.elapsed().as_millis();
    Ok((ms, format!("InvokeCommand={invoked:?} offset={offset}")))
}

// ---------------------------------------------------------------- helpers

fn wide(path: &std::path::Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Drains this thread's queue for `dur` so a modeless sheet keeps working.
fn pump(dur: Duration) {
    let deadline = Instant::now() + dur;
    while Instant::now() < deadline {
        // SAFETY: `msg` is a writable local; PeekMessageW fills it or leaves it.
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
