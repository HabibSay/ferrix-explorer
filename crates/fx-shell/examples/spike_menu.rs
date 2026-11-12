//! Slice-0 spike: `IContextMenu` merge — Tranche 3 decisions 4 evidence.
//!
//! Phase A (always): query the shell menu for the given paths, print every label
//! and canonical verb with the `QueryContextMenu` duration. Phase B adds the
//! popup itself: `--show` on this thread, `--helper` on a dedicated STA thread
//! (the watchdog design of decision 4, minus the timeout which needs product
//! code). Prints `FINDING |` lines for `docs/tranche-3-validation.md`.
//!
//! Usage:
//!   cargo run -p fx-shell --example spike_menu -- <path> [more paths...]
//!   cargo run -p fx-shell --example spike_menu -- --show <path>
//!   cargo run -p fx-shell --example spike_menu -- --helper <path>

use std::{os::windows::ffi::OsStrExt, path::PathBuf, time::Instant};

use windows::{
    Win32::{
        Foundation::HWND,
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree},
        UI::{
            Shell::{Common::ITEMIDLIST, GCS_VERBA, IContextMenu, SHGetDesktopFolder},
            WindowsAndMessaging::{
                CreatePopupMenu, DestroyMenu, GetCursorPos, GetMenuItemCount, GetMenuItemID,
                GetMenuStringW, HMENU, MF_BYPOSITION, TPM_RETURNCMD, TPM_RIGHTBUTTON,
                TrackPopupMenuEx,
            },
        },
    },
    core::{PCWSTR, PSTR},
};

const ID_BASE: u32 = 0x1000;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mode = if args
        .first()
        .is_some_and(|a| a == "--show" || a == "--helper")
    {
        args.remove(0)
    } else {
        String::new()
    };
    let paths: Vec<PathBuf> = args.iter().map(PathBuf::from).collect();
    if paths.is_empty() {
        eprintln!("usage: spike_menu [--show|--helper] <path> [more paths...]");
        std::process::exit(2);
    }

    println!(
        "spike_menu mode={:?} paths={paths:?}",
        if mode.is_empty() { "enumerate" } else { &mode }
    );

    // SAFETY: one COM initialization for this thread; all shell calls below run here.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    println!("CoInitializeEx: {hr:?}");

    match phase_a(&paths) {
        Ok(dump) => print_dump(&dump, "phase-A/main"),
        Err(e) => println!("FINDING | phase A failed: {e:?}"),
    }

    if mode == "--show" {
        // Re-query so the menu handle is fresh on this thread, then show it.
        match show_on_this_thread(&paths) {
            Ok(outcome) => println!("FINDING | show/main: {outcome}"),
            Err(e) => println!("FINDING | show/main failed: {e:?}"),
        }
    } else if mode == "--helper" {
        let helper_paths = paths.clone();
        let handle = std::thread::spawn(move || -> String {
            // SAFETY: fresh STA for the helper thread; the shell objects created
            // below are apartment-bound to it and never leave it.
            let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            let _ = hr;
            let outcome = match show_on_this_thread(&helper_paths) {
                Ok(o) => o,
                Err(e) => format!("helper error {e:?}"),
            };
            // SAFETY: pairs with the CoInitializeEx on this thread.
            unsafe { CoUninitialize() };
            outcome
        });
        match handle.join() {
            Ok(outcome) => println!("FINDING | show/helper: {outcome}"),
            Err(_) => println!("FINDING | show/helper: thread panicked"),
        }
    }

    // SAFETY: pairs with the main-thread CoInitializeEx above.
    unsafe { CoUninitialize() };
}

use windows::Win32::System::Com::CoUninitialize;

// ---------------------------------------------------------------- phase A

struct MenuDump {
    query_ms: u128,
    items: Vec<(String, String)>,
}

fn phase_a(paths: &[PathBuf]) -> windows::core::Result<MenuDump> {
    // SAFETY: caller initialized COM on this thread.
    let desktop = unsafe { SHGetDesktopFolder()? };

    let mut pidls: Vec<*const ITEMIDLIST> = Vec::new();
    for path in paths {
        let w = wide(path);
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        // SAFETY: `w` is live for the call; ParseDisplayName writes into `pidl`.
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

    // SAFETY: live pidl array for the duration of the call only.
    let menu: IContextMenu = unsafe { desktop.GetUIObjectOf(HWND::default(), &pidls, None)? };
    for pidl in pidls {
        // SAFETY: pidl was allocated by ParseDisplayName and is freed exactly once.
        unsafe { CoTaskMemFree(Some(pidl.cast())) };
    }

    // SAFETY: fresh popup owned by this thread.
    let hmenu = unsafe { CreatePopupMenu()? };
    let started = Instant::now();
    // CMF_DEFAULTONLY would let handlers skip optional entries; 0 == CMF_NORMAL,
    // which is what Explorer sends for a right-click (measured, not assumed).
    let hr = unsafe { menu.QueryContextMenu(hmenu, 0, ID_BASE, 0xFFFF, 0) };
    let query_ms = started.elapsed().as_millis();
    let count = (hr.0 & 0xFFFF) as u32;

    // Enumerate by position, but resolve verbs from the item's own id offset
    // (id - idCmdFirst) exactly as fx-shell/src/recycle.rs does — GetCommandString
    // takes the relative offset, not the absolute menu id.
    let mut items = Vec::new();
    // SAFETY: live menu written by QueryContextMenu; count of top-level items.
    let total = unsafe { GetMenuItemCount(Some(hmenu)) };
    for pos in 0..total.max(0) {
        let label = menu_string(&hmenu, pos as u32);
        let id = unsafe { GetMenuItemID(hmenu, pos) };
        let verb = if id == u32::MAX || id < ID_BASE {
            "<separator>".into()
        } else {
            command_verb(&menu, (id - ID_BASE) as usize)
        };
        items.push((label, verb));
    }
    let _ = count;

    // SAFETY: menu created above by this thread; destroy after dumping.
    unsafe { DestroyMenu(hmenu)? };

    Ok(MenuDump { query_ms, items })
}

fn print_dump(dump: &MenuDump, tag: &str) {
    println!(
        "  [{tag}] QueryContextMenu: {}ms, {} entries",
        dump.query_ms,
        dump.items.len()
    );
    for (i, (label, verb)) in dump.items.iter().enumerate() {
        println!("  [{tag}] {i:02} label={label:?} verb={verb:?}");
    }
    let joined_labels = dump
        .items
        .iter()
        .map(|(l, _)| l.to_lowercase())
        .collect::<Vec<_>>()
        .join("|");
    let seven_zip = joined_labels.contains("7-zip") || joined_labels.contains("7zip");
    let git = dump
        .items
        .iter()
        .any(|(l, v)| l.to_lowercase().contains("git") || v.to_lowercase().contains("git"));
    let properties = dump
        .items
        .iter()
        .any(|(l, v)| l.to_lowercase().contains("propert") || v == "properties");
    println!(
        "FINDING | menu/{tag}: entries={} query_ms={} seven_zip={seven_zip} git={git} properties_verb={properties}",
        dump.items.len(),
        dump.query_ms
    );
}

fn menu_string(hmenu: &HMENU, position: u32) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: `buf` is the writable output slice the API fills; MF_BYPOSITION
    // indexes by position in this menu, which matches the enumeration loop.
    let len = unsafe { GetMenuStringW(*hmenu, position, Some(&mut buf), MF_BYPOSITION) };
    if len > 0 {
        String::from_utf16_lossy(&buf[..len as usize])
    } else {
        String::new()
    }
}

fn command_verb(menu: &IContextMenu, offset: usize) -> String {
    // SAFETY: `buf` is writable; GCS_VERBA writes an ANSI verb token into it —
    // the same call shape fx-shell/src/recycle.rs validates in production.
    let mut buf = [0u8; 256];
    let ok = unsafe {
        menu.GetCommandString(
            offset,
            GCS_VERBA,
            None,
            PSTR(buf.as_mut_ptr()),
            buf.len() as u32,
        )
    };
    match ok {
        Ok(()) => {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..end]).into_owned()
        }
        Err(_) => "<no verb>".into(),
    }
}

// ---------------------------------------------------------------- phase B

fn show_on_this_thread(paths: &[PathBuf]) -> windows::core::Result<String> {
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
        // SAFETY: allocated by ParseDisplayName, freed once here.
        unsafe { CoTaskMemFree(Some(pidl.cast())) };
    }

    // SAFETY: popup created on this thread; TrackPopupMenuEx pumps messages on it.
    let hmenu = unsafe { CreatePopupMenu()? };
    let hr = unsafe { menu.QueryContextMenu(hmenu, 0, ID_BASE, 0xFFFF, 0) };
    let count = (hr.0 & 0xFFFF) as u32;
    let _ = count;

    let mut point = windows::Win32::Foundation::POINT::default();
    // SAFETY: caller-provided writable POINT.
    unsafe { GetCursorPos(&mut point)? };

    println!(
        "SHOW | menu displayed at ({}, {}), awaiting input (kill the process to time out)...",
        point.x, point.y
    );
    // SAFETY: null owner is documented as valid for TrackPopupMenuEx; TPM_RETURNCMD
    // returns the selected command id directly instead of routing WM_COMMAND.
    let chosen = unsafe {
        TrackPopupMenuEx(
            hmenu,
            TPM_RETURNCMD.0 | TPM_RIGHTBUTTON.0,
            point.x,
            point.y,
            HWND::default(),
            None,
        )
    };
    // SAFETY: menu created above; no longer needed after the call returns.
    unsafe { DestroyMenu(hmenu)? };

    let chosen_id = chosen.0 as u32;
    if chosen_id == 0 {
        return Ok("cancelled or dismissed (id 0)".into());
    }
    if chosen_id < ID_BASE {
        return Ok(format!("unexpected id {chosen_id}"));
    }
    let verb = command_verb(&menu, (chosen_id - ID_BASE) as usize);
    Ok(format!("chosen id={chosen_id} verb={verb:?}"))
}

// ---------------------------------------------------------------- misc

fn wide(path: &std::path::Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
