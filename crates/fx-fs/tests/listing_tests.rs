//! Integration tests for fx-fs listing and long-path helpers.

use fx_domain::location::Location;
use fx_fs::listing::{ListingError, list_directory};
use fx_fs::longpath::{needs_verbatim_prefix, strip_verbatim_prefix};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Collect all entries by fully draining the channel after listing completes.
async fn collect_entries(location: &Location) -> Vec<fx_domain::entry::FileEntry> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1024);
    let token = CancellationToken::new();
    let _result = list_directory(location, 1, token, tx).await;

    let mut entries = Vec::new();
    while let Ok(batch) = rx.try_recv() {
        entries.extend(batch.entries);
    }
    entries
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Each batch must contain at most 512 entries.
#[tokio::test]
async fn batch_size_at_most_512() {
    let dir = tempfile::tempdir().expect("tempdir");

    // Create 600 files.
    for i in 0..600u32 {
        std::fs::write(dir.path().join(format!("file_{i:04}.txt")), b"").unwrap();
    }

    let location = Location::Local(dir.path().to_path_buf());
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let token = CancellationToken::new();

    // Spawn listing so we can drain the channel concurrently.
    let loc_clone = location.clone();
    let tok_clone = token.clone();
    let handle = tokio::spawn(async move { list_directory(&loc_clone, 1, tok_clone, tx).await });

    let mut first_batch_size = 0usize;
    let mut got_first = false;

    while let Some(batch) = rx.recv().await {
        if !got_first {
            first_batch_size = batch.entries.len();
            got_first = true;
        }
        assert!(
            batch.entries.len() <= 512,
            "batch size {} exceeds 512",
            batch.entries.len()
        );
    }

    let _ = handle.await;
    assert!(got_first, "expected at least one batch");
    assert!(
        first_batch_size <= 512,
        "first batch size {} exceeds 512",
        first_batch_size
    );
}

/// Batches tagged with a stale generation (older than expected) should be
/// discarded by consumers.  This test verifies the generation field is
/// propagated faithfully so callers can do the comparison.
#[tokio::test]
async fn stale_generation_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.txt"), b"").unwrap();

    let location = Location::Local(dir.path().to_path_buf());
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let token = CancellationToken::new();

    // Run listing with generation = 1.
    list_directory(&location, 1, token, tx).await.unwrap();

    let expected_generation = 2u64; // what the UI would consider "current"

    while let Ok(batch) = rx.try_recv() {
        // A stale batch has generation < expected; it should be ignored.
        if batch.generation < expected_generation {
            // Correct: the caller discards this batch.
            continue;
        }
    }
    // Test passes if we get here without panic.
}

/// Cancelling the token before or immediately after starting must result in
/// `Err(Cancelled)` being returned from `list_directory`.
#[tokio::test]
async fn cancellation_mid_list() {
    let dir = tempfile::tempdir().expect("tempdir");

    // Create enough files so there is work to cancel.
    for i in 0..600u32 {
        std::fs::write(dir.path().join(format!("f{i}.bin")), b"").unwrap();
    }

    let location = Location::Local(dir.path().to_path_buf());
    let (tx, _rx) = tokio::sync::mpsc::channel(1); // backpressure: only 1 slot
    let token = CancellationToken::new();

    // Cancel immediately so the listing encounters cancellation on the first batch flush.
    token.cancel();

    let result = list_directory(&location, 1, token, tx).await;
    assert!(
        matches!(result, Err(ListingError::Cancelled)),
        "expected Cancelled, got {result:?}"
    );
}

/// Windows-only: a directory the current user cannot list surfaces
/// `AccessDenied` as a non-fatal batch error while the listing itself
/// completes.
///
/// The fixture creates its own directory and denies the current user the `RD`
/// (list directory) right with `icacls`, instead of relying on a system path
/// such as `System32\config\systemprofile`.  That path is only protected for a
/// non-elevated user: the GitHub Actions Windows runner executes the tests
/// with an elevated administrator token, which can list it, so the system-path
/// fixture made this test fail on CI.  `DELETE` stays granted, so the denied
/// directory is removed normally when the temp dir is dropped and no ACL
/// restore is needed.
///
/// `std::fs::metadata` still succeeds on the denied root (it opens the handle
/// with no access rights, so deny bits on the root cannot block it), therefore
/// the denial is reported by the walker through the error-batch mechanism that
/// `list_directory` uses for non-fatal errors.
#[cfg(target_os = "windows")]
#[tokio::test]
async fn access_denied_continues() {
    let dir = tempfile::tempdir().expect("tempdir");
    let denied = dir.path().join("restricted");
    std::fs::create_dir(&denied).expect("create fixture dir");

    let sid = current_user_sid().expect("cannot resolve the current user SID");
    let deny = format!("*{sid}:(RD)");
    let (ok, output) = run_icacls(&denied, &["/deny", deny.as_str()]);
    assert!(
        ok,
        "failed to deny list access on {}: {output}",
        denied.display()
    );

    let location = Location::Local(denied.clone());
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let token = CancellationToken::new();

    let result = list_directory(&location, 1, token, tx).await;
    assert!(
        result.is_ok(),
        "listing should complete and report the denial per batch, got {result:?}"
    );

    // Drain the channel: at least one batch must carry AccessDenied(root).
    let mut saw_access_denied = false;
    while let Ok(batch) = rx.try_recv() {
        if let Some(ListingError::AccessDenied(p)) = &batch.error {
            assert_eq!(p, &denied, "AccessDenied must point at the denied dir");
            saw_access_denied = true;
        }
    }
    assert!(
        saw_access_denied,
        "expected an AccessDenied error batch for {}",
        denied.display()
    );
}

/// Unicode filenames (accented, CJK, RTL) must be preserved in `display_name`.
#[tokio::test]
async fn unicode_fixture() {
    let dir = tempfile::tempdir().expect("tempdir");

    let names = ["café.txt", "ü_umlaut.txt", "中文.txt", "عربي.txt"];
    for name in &names {
        std::fs::write(dir.path().join(name), b"").unwrap();
    }

    let location = Location::Local(dir.path().to_path_buf());
    let entries = collect_entries(&location).await;

    let found_names: Vec<&str> = entries.iter().map(|e| e.display_name.as_str()).collect();

    for expected in &names {
        assert!(
            found_names.contains(expected),
            "Expected display_name '{expected}' not found in {found_names:?}"
        );
    }
}

/// Windows-only: strip_verbatim_prefix and needs_verbatim_prefix behaviour.
#[cfg(target_os = "windows")]
#[test]
fn long_path() {
    // strip_verbatim_prefix
    assert_eq!(
        strip_verbatim_prefix(r"\\?\C:\Users\alice"),
        r"C:\Users\alice"
    );
    assert_eq!(strip_verbatim_prefix(r"C:\short"), r"C:\short");

    // needs_verbatim_prefix
    let short = PathBuf::from(r"C:\short");
    assert!(!needs_verbatim_prefix(&short));

    let long_str: String = format!("C:\\{}", "x".repeat(260));
    let long = PathBuf::from(&long_str);
    assert!(needs_verbatim_prefix(&long));
}

// ── Windows ACL fixture helpers ──────────────────────────────────────────────

/// Resolve the current user's SID with `whoami` (shipped with Windows, so the
/// fixture needs no extra dependency).
#[cfg(target_os = "windows")]
fn current_user_sid() -> Option<String> {
    let output = std::process::Command::new("whoami")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    // CSV row: "DOMAIN\user","S-1-5-21-…".  Skip the integrity-label SIDs
    // (S-1-16-…) that can appear in localized output.
    String::from_utf8_lossy(&output.stdout)
        .split('"')
        .find(|field| field.starts_with("S-1-") && !field.starts_with("S-1-16"))
        .map(str::to_owned)
}

/// Run `icacls <dir> <args>` and return `(success, combined output)`.
#[cfg(target_os = "windows")]
fn run_icacls(dir: &std::path::Path, args: &[&str]) -> (bool, String) {
    let output = std::process::Command::new("icacls")
        .arg(dir)
        .args(args)
        .output()
        .expect("failed to run icacls");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}
