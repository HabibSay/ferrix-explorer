//! Slice-2 probe: does Ferrix really own the OLE drop registration on its own
//! window? (Tranche 3 spec §7, decision 8; CLAUDE.md rule 8.)
//!
//! The shipped application is a Slint window on winit 0.30. winit registers its own
//! `IDropTarget` on every window it creates (`platform_specific.drag_and_drop` defaults
//! to `true`), while Slint's winit backend never relays winit's
//! `WindowEvent::DragEntered`/`DragDropped` to Slint code. That target is therefore
//! unreachable *and* in the way: a second `RegisterDragDrop` on the same HWND fails with
//! `DRAGDROP_E_ALREADYREGISTERED`, which would make the slice-2 drop target dead on
//! arrival.
//!
//! `OleDropRegistration::register` answers by revoking a foreign registration and
//! retrying. This probe reproduces the three states on a plain window so the branch is
//! observed rather than assumed:
//!
//! 1. no target on the window     -> `is_drop_target_registered` says false;
//! 2. a foreign target present    -> `register` takes the window over;
//! 3. after `revoke`              -> the window is clean again.
//!
//! It lives in `fx-shell/examples`, not in a unit test, for two reasons: the window needs
//! the main thread (a Rust test harness runs its tests on threads of their own, and
//! `RegisterDragDrop` requires the thread that owns the HWND), and raw Win32 code belongs
//! in `fx-shell` only (`CLAUDE.md` rule 3).
//!
//! Usage:
//!   cargo run -p fx-shell --example ole_registration_probe
//!
//! Prints `FINDING |` lines for `docs/tranche-3-validation.md`. Nothing here proves how
//! the winit-hosted Slint window behaves on a desktop session — that row stays manual.

use std::sync::Arc;

use fx_shell::drop_target::is_drop_target_registered;
use fx_shell::{DragEffect, DragPayload, DropOutcome, DropSink, OleDropRegistration};
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, RegisterClassW, UnregisterClassW, WINDOW_EX_STYLE,
            WNDCLASSW, WS_OVERLAPPEDWINDOW,
        },
    },
    core::w,
};

/// A sink that accepts nothing: this probe only cares about registration ownership, so
/// no drag ever reaches it.
struct RejectingSink;

impl DropSink for RejectingSink {
    fn effect_at(&self, _payload: &DragPayload, _point: (i32, i32)) -> Option<DragEffect> {
        None
    }

    fn on_drop(&self, _payload: DragPayload, _point: (i32, i32)) -> DropOutcome {
        DropOutcome::Rejected
    }
}

fn main() {
    println!("ole_registration_probe: three states of the window's drop registration");

    // SAFETY: `GetModuleHandleW(None)` asks for this process's module handle, which is
    // what `CreateWindowExW` needs; it is only read, never released.
    let instance: HINSTANCE = match unsafe { GetModuleHandleW(None) } {
        Ok(module) => module.into(),
        Err(error) => {
            println!("FINDING | window: GetModuleHandleW failed: {error:?}");
            std::process::exit(1);
        }
    };

    let class_name = w!("FerrixOleRegistrationProbe");
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class_name,
        ..Default::default()
    };
    // SAFETY: the class is fully initialised, its name outlives the call, and the window
    // procedure is a valid `extern "system"` function.
    let atom = unsafe { RegisterClassW(&window_class) };
    if atom == 0 {
        println!("FINDING | window: RegisterClassW returned 0");
        std::process::exit(1);
    }

    // SAFETY: the class is registered, the handle belongs to this thread, and the title
    // is a NUL-terminated wide string literal. No parent and no menu are passed.
    let hwnd: HWND = match unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("Ferrix OLE registration probe"),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            320,
            200,
            None,
            None,
            Some(instance),
            None,
        )
    } {
        Ok(hwnd) => hwnd,
        Err(error) => {
            println!("FINDING | window: CreateWindowExW failed: {error:?}");
            std::process::exit(1);
        }
    };

    let sink: Arc<dyn DropSink> = Arc::new(RejectingSink);

    // State 1: the window carries no target. The probe registers a throw-away target and
    // revokes it again, so the window is left as it was found.
    let registered_before = is_drop_target_registered(hwnd, Arc::clone(&sink));
    println!("FINDING | state 1 (fresh window): registered={registered_before} (expected false)");

    // State 2: our own registration takes the window.
    let first = OleDropRegistration::register(hwnd, Arc::clone(&sink));
    let first_ok = first.is_ok();
    match &first {
        Ok(_) => println!("FINDING | state 2 (fresh window): RegisterDragDrop Ok (expected Ok)"),
        Err(error) => {
            println!("FINDING | state 2 (fresh window): RegisterDragDrop failed: {error:?}")
        }
    }

    // State 3: a foreign registration already sits on the window. This is the situation
    // winit creates in the shipped application; `register` must revoke and take over
    // instead of failing with DRAGDROP_E_ALREADYREGISTERED.
    let registered_now = is_drop_target_registered(hwnd, Arc::clone(&sink));
    println!("FINDING | state 3 (target present): registered={registered_now} (expected true)");
    let takeover = OleDropRegistration::register(hwnd, Arc::clone(&sink));
    match &takeover {
        Ok(_) => println!(
            "FINDING | state 3 (target present): takeover RegisterDragDrop Ok — the \
             DRAGDROP_E_ALREADYREGISTERED branch works (expected Ok)"
        ),
        Err(error) => println!(
            "FINDING | state 3 (target present): takeover RegisterDragDrop failed: {error:?} \
             (expected Ok)"
        ),
    }

    // Cleanup: drop the takeover guard, then the first guard, and confirm the window is
    // clean. `RegisterDragDrop` is refcount-free, so the second registration replaced the
    // first and one `RevokeDragDrop` clears the window.
    drop(takeover);
    drop(first);
    let registered_after = is_drop_target_registered(hwnd, sink);
    println!("FINDING | state 4 (after revoke): registered={registered_after} (expected false)");

    // SAFETY: the window was created on this thread and is not used after this point; the
    // class was registered above and no window of it survives.
    unsafe {
        let _ = DestroyWindow(hwnd);
        let _ = UnregisterClassW(class_name, Some(instance));
    }

    let pass = !registered_before && first_ok && registered_now && !registered_after;
    println!(
        "FINDING | verdict: {} — no claim about the winit-hosted Slint window is made here; \
         that row stays manual in docs/tranche-3-validation.md",
        if pass { "PASS" } else { "FAIL" }
    );
    if !pass {
        std::process::exit(1);
    }
}

/// The probe window's message procedure: it never receives a message because the probe
/// runs no message loop, so every message gets the default handling.
extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    // SAFETY: the default procedure handles every message this probe does not; the window
    // handle is the one the system passed in.
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(hwnd, message, wparam, lparam)
    }
}
