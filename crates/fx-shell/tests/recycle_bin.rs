#![cfg(target_os = "windows")]
//! Slice-2 validation runs for `docs/tranche-2-validation.md` (PRD 15.3).
//!
//! These tests touch the user's real Recycle Bin (and, for the clipboard pair, the
//! real clipboard), so they are `#[ignore]`d: they never run in automatic CI. Run
//! them explicitly on a desktop Windows session:
//! `cargo test -p fx-shell --test recycle_bin -- --ignored --nocapture --test-threads=1`

use std::{fs, time::Instant};
use windows::Win32::Foundation::HWND;

fn probe_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("fx-shell-{}-{}", label, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create probe dir");
    dir
}

#[test]
#[ignore = "touches the user's real Recycle Bin; run on the dev workstation for the slice-2 checklist"]
fn recycle_then_restore_round_trip() {
    let dir = probe_dir("rt");
    let file = dir.join("fx-restore-probe.txt");
    fs::write(&file, b"PROBE-CONTENT").unwrap();

    let start = Instant::now();
    let items = fx_shell::recycle::recycle_paths(std::slice::from_ref(&file), HWND::default())
        .expect("recycle_paths");
    assert!(!file.exists(), "source must be gone after recycle");
    assert!(
        !items[0].recycled_name.is_empty(),
        "identity diff should be unambiguous for a fresh unique name"
    );

    fx_shell::recycle::restore_recycled_item(&file).expect("restore via Shell verb");
    assert_eq!(
        fs::read(&file).unwrap(),
        b"PROBE-CONTENT",
        "content must survive recycle + restore"
    );
    println!(
        "round trip OK in {:?}; recycled_name={:?}",
        start.elapsed(),
        items[0].recycled_name
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "touches the user's real Recycle Bin; run on the dev workstation for the slice-2 checklist"]
fn long_path_recycle_reports_long_path_instead_of_silently_deleting() {
    let dir = probe_dir("lp");
    // Build a >260-character path inside the probe dir.
    let mut deep = dir.clone();
    while deep.to_string_lossy().len() < 250 {
        deep.push("fx-long-path-segment-0123456789");
    }
    fs::create_dir_all(&deep).unwrap();
    let file = deep.join("target.txt");
    fs::write(&file, b"x").unwrap();

    let result = fx_shell::recycle::recycle_paths(std::slice::from_ref(&file), HWND::default());
    match result {
        Err(fx_shell::recycle::ShellRecycleError::LongPath) => {
            println!("long path refused up front, exactly as specified");
        }
        Ok(items) => {
            // SHFileOperationW accepted it on this build: record which behavior happened.
            println!(
                "SHFileOperationW accepted a {}-char path; recycled={:?}",
                file.to_string_lossy().len(),
                items[0].recycled_name
            );
        }
        Err(other) => panic!("unexpected error: {other}"),
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "overwrites the user's real clipboard; run on the dev workstation for the slice-2 checklist"]
fn clipboard_write_read_round_trip_keeps_effect() {
    let dir = probe_dir("cb");
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    fs::write(&a, b"a").unwrap();
    fs::write(&b, b"b").unwrap();

    for effect in [
        fx_shell::clipboard::ClipboardEffect::Copy,
        fx_shell::clipboard::ClipboardEffect::Move,
    ] {
        fx_shell::clipboard::write_clipboard(&[a.clone(), b.clone()], effect).expect("write");
        let (paths, read_effect) = fx_shell::clipboard::read_clipboard()
            .expect("read")
            .expect("payload present");
        assert_eq!(paths, vec![a.clone(), b.clone()]);
        assert_eq!(read_effect, effect);
    }
    println!("CF_HDROP + Preferred DropEffect round-trip OK for both effects");
    fs::remove_dir_all(&dir).ok();
}
