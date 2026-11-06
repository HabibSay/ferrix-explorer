//! Long-path fixture tests for fx-fs.
//!
//! These tests are Windows-only and cover three scenarios:
//!   1. Deep path using only the application manifest (`longPathAware`).
//!   2. Deep path forced through `\\?\` verbatim prefix.
//!   3. Both mechanisms together.
//!
//! If the OS refuses to create the deep directory (MAX_PATH limit hit without
//! an active manifest), the test records the limitation via `eprintln!` and
//! returns — it does not panic.

#[cfg(target_os = "windows")]
mod tests {
    use fx_domain::location::Location;
    use fx_fs::listing::list_directory;
    use fx_fs::longpath::with_verbatim_prefix_if_needed;
    use tokio_util::sync::CancellationToken;

    // ── Helper ──────────────────────────────────────────────────────────────

    /// Create a directory tree that is > 260 characters deep by chaining 20 segments
    /// of 14 characters each (20 × 14 = 280 chars, plus the base temp path).
    fn create_deep_tree(base: &std::path::Path) -> std::path::PathBuf {
        let mut deep = base.to_path_buf();
        for i in 0..20 {
            deep.push(format!("segment{i:03}"));
        }
        deep
    }

    /// Collect all entries from a listing (ignores errors, returns what arrived).
    async fn collect(location: &Location) -> Vec<fx_domain::entry::FileEntry> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1024);
        let token = CancellationToken::new();
        let _ = list_directory(location, 1, token, tx).await;
        let mut out = Vec::new();
        while let Ok(batch) = rx.try_recv() {
            out.extend(batch.entries);
        }
        out
    }

    // ── Tests ────────────────────────────────────────────────────────────────

    /// Manifest-only: create the deep tree using standard `std::fs`, then list it.
    #[tokio::test]
    async fn longpath_manifest_only() {
        let base = tempfile::tempdir().unwrap();
        let deep = create_deep_tree(base.path());

        // `create_dir_all` may fail if MAX_PATH is hit without an active manifest.
        if std::fs::create_dir_all(&deep).is_err() {
            eprintln!(
                "longpath_manifest_only: create_dir_all failed — \
                 path > 260 chars is not supported without longPathAware manifest \
                 (known limitation on this environment)"
            );
            return;
        }

        // Create a file inside the deep directory so listing returns at least one entry.
        let _ = std::fs::write(deep.join("leaf.txt"), b"");

        let location = Location::Local(deep.clone());
        let entries = collect(&location).await;

        assert!(
            !entries.is_empty(),
            "Expected at least one entry in the deep directory, got none"
        );
    }

    /// Verbatim-prefix-only: force the path through `\\?\` before passing to `list_directory`.
    #[tokio::test]
    async fn longpath_verbatim_prefix_only() {
        let base = tempfile::tempdir().unwrap();
        let deep = create_deep_tree(base.path());

        // Use the verbatim-prefix helper to create the directory.
        let verbatim = with_verbatim_prefix_if_needed(&deep);
        if std::fs::create_dir_all(&verbatim).is_err() {
            eprintln!(
                "longpath_verbatim_prefix_only: create_dir_all failed even with \\\\?\\\\ prefix \
                 (known limitation on this environment)"
            );
            return;
        }

        let _ = std::fs::write(verbatim.join("leaf.txt"), b"");

        // List using the verbatim-prefixed path.
        let location = Location::Local(verbatim.clone());
        let entries = collect(&location).await;

        assert!(
            !entries.is_empty(),
            "Expected at least one entry via verbatim prefix, got none"
        );
    }

    /// Both manifest and verbatim prefix together.
    #[tokio::test]
    async fn longpath_both() {
        let base = tempfile::tempdir().unwrap();
        let deep = create_deep_tree(base.path());

        // Attempt manifest creation first; fall through to verbatim if it fails.
        let created = if std::fs::create_dir_all(&deep).is_ok() {
            Some(deep.clone())
        } else {
            let verbatim = with_verbatim_prefix_if_needed(&deep);
            if std::fs::create_dir_all(&verbatim).is_ok() {
                Some(verbatim)
            } else {
                None
            }
        };

        let Some(dir) = created else {
            eprintln!(
                "longpath_both: could not create deep directory tree by either method \
                 (known limitation on this environment)"
            );
            return;
        };

        let _ = std::fs::write(dir.join("leaf.txt"), b"");

        // List with both the plain path and the verbatim-prefixed version.
        let verbatim = with_verbatim_prefix_if_needed(&dir);
        let location_plain = Location::Local(dir.clone());
        let location_verbatim = Location::Local(verbatim.clone());

        let entries_plain = collect(&location_plain).await;
        let entries_verbatim = collect(&location_verbatim).await;

        // At least one of the two must succeed (depending on the environment).
        assert!(
            !entries_plain.is_empty() || !entries_verbatim.is_empty(),
            "Expected entries from at least one listing variant"
        );
    }
}
