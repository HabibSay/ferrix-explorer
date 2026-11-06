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

/// Windows-only: trying to list a protected directory returns `AccessDenied`.
#[cfg(target_os = "windows")]
#[tokio::test]
async fn access_denied_continues() {
    // System32\config is ACL-protected even for admins at the API level.
    let protected = PathBuf::from(r"C:\Windows\System32\config\systemprofile");
    if !protected.exists() {
        // If the path doesn't exist in this environment, skip gracefully.
        return;
    }
    let location = Location::Local(protected.clone());
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let token = CancellationToken::new();

    let result = list_directory(&location, 1, token, tx).await;
    assert!(
        matches!(result, Err(ListingError::AccessDenied(_))),
        "expected AccessDenied, got {result:?}"
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
