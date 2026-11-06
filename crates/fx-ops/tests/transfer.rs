//! Integration tests for the transfer engine: real files, real directories, a fake platform
//! standing in for the Windows Shell.

use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use fx_ops::{
    OperationError, OperationId, OperationState, Progress, Transfer, TransferKind,
    conflict::{ConflictAnswer, ConflictChoice, ConflictRequest, ConflictResolver, FixedResolver},
    control::OperationControl,
    plan::plan_transfer,
    port::{
        PlatformOps, RecycleError, RecyclePrecheck, RecycledItem, ReparseKind, RestoreError,
        VolumeKind,
    },
};

struct TestPlatform {
    /// Paths under this root get volume 2; everything else is volume 1.
    second_volume_root: PathBuf,
    cross_volume: bool,
    fail_metadata: bool,
    fail_replace: bool,
    free_bytes: u64,
    recycle_dir: PathBuf,
    /// Paths the fake platform classifies as symbolic links, respectively junctions.
    symlinks: Vec<PathBuf>,
    junctions: Vec<PathBuf>,
    /// Links the engine asked the platform to create: `(target, link, is_dir)`.
    created: Arc<Mutex<Vec<(PathBuf, PathBuf, bool)>>>,
}

impl TestPlatform {
    fn new(root: &Path) -> Self {
        Self {
            second_volume_root: root.to_path_buf(),
            cross_volume: false,
            fail_metadata: false,
            fail_replace: false,
            free_bytes: u64::MAX,
            recycle_dir: root.join(".recycle"),
            symlinks: Vec::new(),
            junctions: Vec::new(),
            created: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn with_free_space(mut self, bytes: u64) -> Self {
        self.free_bytes = bytes;
        self
    }

    fn classified_as_symlink(mut self, paths: &[PathBuf]) -> Self {
        self.symlinks = paths.to_vec();
        self
    }

    fn classified_as_junction(mut self, paths: &[PathBuf]) -> Self {
        self.junctions = paths.to_vec();
        self
    }
}

impl PlatformOps for TestPlatform {
    fn volume_id(&self, path: &Path) -> std::io::Result<u64> {
        if self.cross_volume && path.starts_with(&self.second_volume_root) {
            Ok(2)
        } else {
            Ok(1)
        }
    }

    fn volume_kind(&self, _path: &Path) -> std::io::Result<VolumeKind> {
        Ok(VolumeKind::Fixed)
    }

    fn free_space(&self, _path: &Path) -> std::io::Result<u64> {
        Ok(self.free_bytes)
    }

    fn replace_file(&self, replacement: &Path, destination: &Path) -> std::io::Result<()> {
        if self.fail_replace {
            return Err(std::io::Error::other("simulated replace failure"));
        }
        if destination.exists() {
            fs::remove_file(destination)?;
        }
        fs::rename(replacement, destination)
    }

    fn can_recycle(&self, _paths: &[PathBuf]) -> Result<RecyclePrecheck, RecycleError> {
        Ok(RecyclePrecheck::Possible)
    }

    fn recycle(&self, paths: &[PathBuf]) -> Result<Vec<RecycledItem>, RecycleError> {
        fs::create_dir_all(&self.recycle_dir).map_err(|error| RecycleError {
            code: 1,
            message: error.to_string(),
        })?;
        let mut items = Vec::new();
        for (index, path) in paths.iter().enumerate() {
            let name = format!("$R{index}");
            let recycled = self.recycle_dir.join(&name);
            fs::rename(path, &recycled).map_err(|error| RecycleError {
                code: 2,
                message: error.to_string(),
            })?;
            items.push(RecycledItem {
                original: path.clone(),
                recycled_name: Some(name),
                verified: true,
            });
        }
        Ok(items)
    }

    fn restore_recycled(&self, item: &RecycledItem) -> Result<(), RestoreError> {
        let Some(name) = &item.recycled_name else {
            return Err(RestoreError {
                message: "no name".to_owned(),
            });
        };
        fs::rename(self.recycle_dir.join(name), &item.original).map_err(|error| RestoreError {
            message: error.to_string(),
        })
    }

    fn apply_metadata(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        if self.fail_metadata {
            return Err(std::io::Error::other("simulated metadata failure"));
        }
        fx_ops::port::std_apply_metadata(from, to)
    }

    fn reparse_kind(&self, path: &Path) -> Option<ReparseKind> {
        if self.symlinks.iter().any(|link| link == path) {
            return Some(ReparseKind::SymbolicLink);
        }
        if self.junctions.iter().any(|link| link == path) {
            return Some(ReparseKind::Junction);
        }
        None
    }

    /// Records the request and leaves a marker file, so the assertions can tell a
    /// recreated link apart from a copied payload.
    fn create_symlink(&self, target: &Path, link: &Path, is_dir: bool) -> std::io::Result<()> {
        self.created
            .lock()
            .unwrap()
            .push((target.to_path_buf(), link.to_path_buf(), is_dir));
        fs::write(link, format!("link to {}", target.display()))
    }
}

struct ScriptedResolver {
    answers: VecDeque<Option<ConflictAnswer>>,
    calls: usize,
}

impl ScriptedResolver {
    fn new(answers: Vec<ConflictAnswer>) -> Self {
        Self {
            answers: answers.into_iter().map(Some).collect(),
            calls: 0,
        }
    }
}

impl ConflictResolver for ScriptedResolver {
    fn resolve(&mut self, _request: &ConflictRequest) -> Option<ConflictAnswer> {
        self.calls += 1;
        self.answers.pop_front().unwrap_or(None)
    }
}

fn run(
    sources: &[PathBuf],
    destination: &Path,
    kind: TransferKind,
    platform: &dyn PlatformOps,
    control: &OperationControl,
    resolver: &mut dyn ConflictResolver,
) -> fx_ops::OperationSummary {
    let plan = plan_transfer(kind, sources, destination).expect("plan");
    let mut sink = |_progress: &Progress| {};
    let mut transfer = Transfer {
        id: OperationId::next(),
        port: platform,
        control,
        resolver,
        on_progress: &mut sink,
    };
    transfer.run(&plan)
}

fn write(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

#[test]
fn copies_a_file_and_preserves_contents() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("a.txt");
    write(&source, b"hello");
    let destination = dir.path().join("dest");
    fs::create_dir(&destination).unwrap();

    let platform = TestPlatform::new(dir.path());
    let mut resolver = FixedResolver {
        answer: ConflictAnswer::once(ConflictChoice::KeepBoth),
    };
    let summary = run(
        std::slice::from_ref(&source),
        &destination,
        TransferKind::Copy,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::Completed);
    assert_eq!(summary.files_done, 1);
    assert_eq!(summary.bytes_done, 5);
    assert_eq!(fs::read(destination.join("a.txt")).unwrap(), b"hello");
    assert!(source.exists(), "a copy never deletes the source");
}

/// Creates a file symlink, returning false when the host refuses: creating a symbolic
/// link on Windows needs the SeCreateSymbolicLink privilege or Developer Mode.
fn make_symlink(target: &Path, link: &Path) -> bool {
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }
    #[cfg(not(windows))]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
}

#[test]
fn a_symbolic_link_is_recreated_as_a_link_not_copied() {
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("payload.txt");
    write(&payload, b"payload");
    let link = dir.path().join("link.txt");
    if !make_symlink(&payload, &link) {
        eprintln!("no symbolic link privilege on this host; nothing was verified");
        return;
    }
    let destination = dir.path().join("dest");
    fs::create_dir(&destination).unwrap();

    let platform = TestPlatform::new(dir.path()).classified_as_symlink(std::slice::from_ref(&link));
    let created = platform.created.clone();
    let mut resolver = FixedResolver {
        answer: ConflictAnswer::once(ConflictChoice::KeepBoth),
    };
    let summary = run(
        std::slice::from_ref(&link),
        &destination,
        TransferKind::Copy,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(
        created.lock().unwrap().clone(),
        vec![(payload.clone(), destination.join("link.txt"), false)],
        "the engine must ask the platform to recreate the link"
    );
    assert_eq!(summary.files_failed, 0, "{:?}", summary.failures);
    assert_eq!(
        fs::read(destination.join("link.txt")).unwrap(),
        b"link to payload".as_slice(),
        "the payload itself must not be copied"
    );
    assert!(link.exists(), "a copy never deletes the source");
}

#[test]
fn a_junction_stays_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("elsewhere");
    fs::create_dir(&payload).unwrap();
    write(&payload.join("inside.txt"), b"inside");
    let link = dir.path().join("junction");
    if !make_symlink(&payload, &link) {
        eprintln!("no symbolic link privilege on this host; nothing was verified");
        return;
    }
    let destination = dir.path().join("dest");
    fs::create_dir(&destination).unwrap();

    let platform =
        TestPlatform::new(dir.path()).classified_as_junction(std::slice::from_ref(&link));
    let created = platform.created.clone();
    let mut resolver = FixedResolver {
        answer: ConflictAnswer::once(ConflictChoice::KeepBoth),
    };
    let summary = run(
        std::slice::from_ref(&link),
        &destination,
        TransferKind::Copy,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert!(
        created.lock().unwrap().is_empty(),
        "a junction must never be followed or recreated"
    );
    assert!(!destination.join("junction").exists());
    assert_eq!(summary.files_failed, 1);
    assert_eq!(
        summary.failures[0].error,
        OperationError::ReparsePointUnsupported
    );
}

#[test]
fn copies_a_tree_with_unicode_names() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("photos");
    write(&source.join("東京").join("résumé.txt"), b"unicode");
    write(&source.join("東京").join("second.bin"), b"two");
    let destination = dir.path().join("dest");
    fs::create_dir(&destination).unwrap();

    let platform = TestPlatform::new(dir.path());
    let mut resolver = FixedResolver {
        answer: ConflictAnswer::once(ConflictChoice::KeepBoth),
    };
    let summary = run(
        &[source],
        &destination,
        TransferKind::Copy,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::Completed);
    assert_eq!(summary.files_done, 2);
    assert_eq!(
        fs::read(destination.join("photos").join("東京").join("résumé.txt")).unwrap(),
        b"unicode"
    );
}

#[test]
fn keep_both_never_touches_the_existing_target() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("report.txt");
    write(&source, b"new");
    let destination = dir.path().join("dest");
    write(&destination.join("report.txt"), b"old");

    let platform = TestPlatform::new(dir.path());
    let mut resolver = FixedResolver {
        answer: ConflictAnswer::once(ConflictChoice::KeepBoth),
    };
    let summary = run(
        &[source],
        &destination,
        TransferKind::Copy,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::Completed);
    assert_eq!(fs::read(destination.join("report.txt")).unwrap(), b"old");
    assert_eq!(
        fs::read(destination.join("report (2).txt")).unwrap(),
        b"new"
    );
}

#[test]
fn replace_swaps_the_target_and_leaves_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("report.txt");
    write(&source, b"new content");
    let destination = dir.path().join("dest");
    write(&destination.join("report.txt"), b"old");

    let platform = TestPlatform::new(dir.path());
    let mut resolver = FixedResolver {
        answer: ConflictAnswer::once(ConflictChoice::Replace),
    };
    let summary = run(
        &[source],
        &destination,
        TransferKind::Copy,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::Completed);
    assert_eq!(
        fs::read(destination.join("report.txt")).unwrap(),
        b"new content"
    );
    let leftovers: Vec<_> = fs::read_dir(&destination)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".fxpart-"))
        .collect();
    assert!(leftovers.is_empty(), "no temporary file may survive");
}

#[test]
fn skip_leaves_the_target_alone() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("report.txt");
    write(&source, b"new");
    let destination = dir.path().join("dest");
    write(&destination.join("report.txt"), b"old");

    let platform = TestPlatform::new(dir.path());
    let mut resolver = FixedResolver {
        answer: ConflictAnswer::once(ConflictChoice::Skip),
    };
    let summary = run(
        &[source],
        &destination,
        TransferKind::Copy,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.files_skipped, 1);
    assert_eq!(fs::read(destination.join("report.txt")).unwrap(), b"old");
    assert!(!destination.join("report (2).txt").exists());
}

#[test]
fn apply_to_all_is_scoped_to_the_conflict_class() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("src");
    write(&source.join("a.txt"), b"new a");
    write(&source.join("b.txt"), b"new b");
    let destination = dir.path().join("dest");
    write(&destination.join("src").join("a.txt"), b"old a");
    write(&destination.join("src").join("b.txt"), b"old b");

    let platform = TestPlatform::new(dir.path());
    let mut resolver =
        ScriptedResolver::new(vec![ConflictAnswer::all_in_class(ConflictChoice::Replace)]);
    let summary = run(
        &[source],
        &destination,
        TransferKind::Copy,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::Completed);
    assert_eq!(resolver.calls, 1, "the second file must reuse the decision");
    assert_eq!(
        fs::read(destination.join("src").join("a.txt")).unwrap(),
        b"new a"
    );
    assert_eq!(
        fs::read(destination.join("src").join("b.txt")).unwrap(),
        b"new b"
    );
}

#[test]
fn cancelling_while_paused_leaves_no_partial_target() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("big.bin");
    write(&source, &vec![9u8; 4 * 1024 * 1024]);
    let destination = dir.path().join("dest");
    fs::create_dir(&destination).unwrap();

    let control = Arc::new(OperationControl::new());
    control.pause();
    let canceller = Arc::clone(&control);
    let handle = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        canceller.cancel();
    });

    let platform = TestPlatform::new(dir.path());
    let mut resolver = FixedResolver {
        answer: ConflictAnswer::once(ConflictChoice::KeepBoth),
    };
    let summary = run(
        &[source],
        &destination,
        TransferKind::Copy,
        &platform,
        &control,
        &mut resolver,
    );
    handle.join().unwrap();

    assert_eq!(summary.state, OperationState::Cancelled);
    assert!(!destination.join("big.bin").exists());
    let leftovers: Vec<_> = fs::read_dir(&destination)
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn a_metadata_failure_removes_the_partial_copy() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("a.txt");
    write(&source, b"content");
    let destination = dir.path().join("dest");
    fs::create_dir(&destination).unwrap();

    let mut platform = TestPlatform::new(dir.path());
    platform.fail_metadata = true;
    let mut resolver = FixedResolver {
        answer: ConflictAnswer::once(ConflictChoice::KeepBoth),
    };
    let summary = run(
        &[source],
        &destination,
        TransferKind::Copy,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::CompletedWithWarnings);
    assert_eq!(summary.files_failed, 1);
    assert!(!destination.join("a.txt").exists());
}

#[test]
fn a_replace_failure_keeps_the_original() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("a.txt");
    write(&source, b"new");
    let destination = dir.path().join("dest");
    write(&destination.join("a.txt"), b"old");

    let mut platform = TestPlatform::new(dir.path());
    platform.fail_replace = true;
    let mut resolver = FixedResolver {
        answer: ConflictAnswer::once(ConflictChoice::Replace),
    };
    let summary = run(
        &[source],
        &destination,
        TransferKind::Copy,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::CompletedWithWarnings);
    assert_eq!(fs::read(destination.join("a.txt")).unwrap(), b"old");
}

#[test]
fn directories_merge_without_a_conflict_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("src");
    write(&source.join("new.txt"), b"new");
    let destination = dir.path().join("dest");
    write(&destination.join("src").join("existing.txt"), b"old");

    let platform = TestPlatform::new(dir.path());
    let mut resolver = ScriptedResolver::new(vec![]);
    let summary = run(
        &[source],
        &destination,
        TransferKind::Copy,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::Completed);
    assert_eq!(resolver.calls, 0, "merging directories is not a conflict");
    assert!(destination.join("src").join("existing.txt").exists());
    assert!(destination.join("src").join("new.txt").exists());
}

#[test]
fn same_volume_move_renames_and_removes_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("a.txt");
    write(&source, b"move me");
    let destination = dir.path().join("dest");
    fs::create_dir(&destination).unwrap();

    let platform = TestPlatform::new(dir.path());
    let mut resolver = FixedResolver {
        answer: ConflictAnswer::once(ConflictChoice::KeepBoth),
    };
    let summary = run(
        std::slice::from_ref(&source),
        &destination,
        TransferKind::Move,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::Completed);
    assert!(!source.exists());
    assert_eq!(fs::read(destination.join("a.txt")).unwrap(), b"move me");
}

#[test]
fn same_volume_move_of_a_folder_renames_it() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("folder");
    write(&source.join("a.txt"), b"payload");
    let destination = dir.path().join("dest");
    fs::create_dir(&destination).unwrap();

    let platform = TestPlatform::new(dir.path());
    let mut resolver = ScriptedResolver::new(vec![]);
    let summary = run(
        std::slice::from_ref(&source),
        &destination,
        TransferKind::Move,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::Completed);
    assert!(
        !source.exists(),
        "a folder move must not leave the source behind"
    );
    assert_eq!(
        fs::read(destination.join("folder").join("a.txt")).unwrap(),
        b"payload"
    );
    assert_eq!(resolver.calls, 0, "a rename is not a conflict");
}

#[test]
fn moving_a_folder_onto_an_existing_one_merges_and_removes_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("folder");
    write(&source.join("new.txt"), b"new");
    let destination = dir.path().join("dest");
    write(&destination.join("folder").join("old.txt"), b"old");

    let platform = TestPlatform::new(dir.path());
    let mut resolver = ScriptedResolver::new(vec![]);
    let summary = run(
        std::slice::from_ref(&source),
        &destination,
        TransferKind::Move,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::Completed);
    assert!(!source.exists());
    assert!(destination.join("folder").join("old.txt").exists());
    assert!(destination.join("folder").join("new.txt").exists());
}

#[test]
fn cross_volume_move_copies_then_deletes_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("src");
    write(&source.join("nested").join("a.txt"), b"payload");
    let destination = dir.path().join("dest");
    fs::create_dir(&destination).unwrap();

    let mut platform = TestPlatform::new(dir.path());
    platform.second_volume_root = destination.clone();
    platform.cross_volume = true;
    let mut resolver = FixedResolver {
        answer: ConflictAnswer::once(ConflictChoice::KeepBoth),
    };
    let summary = run(
        std::slice::from_ref(&source),
        &destination,
        TransferKind::Move,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::Completed);
    assert!(
        !source.exists(),
        "the source goes only after a verified copy"
    );
    assert_eq!(
        fs::read(destination.join("src").join("nested").join("a.txt")).unwrap(),
        b"payload"
    );
}

#[test]
fn same_folder_paste_duplicates_with_keep_both() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("note.txt");
    write(&source, b"original");

    let platform = TestPlatform::new(dir.path());
    let mut resolver = ScriptedResolver::new(vec![]);
    let summary = run(
        std::slice::from_ref(&source),
        dir.path(),
        TransferKind::Copy,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::Completed);
    assert_eq!(resolver.calls, 0);
    assert_eq!(fs::read(&source).unwrap(), b"original");
    assert_eq!(
        fs::read(dir.path().join("note (2).txt")).unwrap(),
        b"original"
    );
}

#[test]
fn closing_the_dialog_cancels_before_touching_the_target() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("a.txt");
    write(&source, b"new");
    let destination = dir.path().join("dest");
    write(&destination.join("a.txt"), b"old");

    let platform = TestPlatform::new(dir.path());
    struct CancelResolver;
    impl ConflictResolver for CancelResolver {
        fn resolve(&mut self, _request: &ConflictRequest) -> Option<ConflictAnswer> {
            None
        }
    }
    let mut resolver = CancelResolver;
    let summary = run(
        &[source],
        &destination,
        TransferKind::Copy,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::Cancelled);
    assert_eq!(fs::read(destination.join("a.txt")).unwrap(), b"old");
}

#[cfg(windows)]
#[test]
fn a_locked_file_fails_per_item_and_the_operation_continues() {
    use std::os::windows::fs::OpenOptionsExt;

    let dir = tempfile::tempdir().unwrap();
    let locked = dir.path().join("locked.txt");
    write(&locked, b"locked");
    let free = dir.path().join("free.txt");
    write(&free, b"free");

    let _guard = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&locked)
        .unwrap();

    let destination = dir.path().join("dest");
    fs::create_dir(&destination).unwrap();

    let platform = TestPlatform::new(dir.path());
    let mut resolver = FixedResolver {
        answer: ConflictAnswer::once(ConflictChoice::KeepBoth),
    };
    let summary = run(
        &[locked.clone(), free.clone()],
        &destination,
        TransferKind::Copy,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::CompletedWithWarnings);
    assert_eq!(summary.files_failed, 1);
    assert_eq!(summary.files_done, 1);
    assert!(destination.join("free.txt").exists());
    assert!(!destination.join("locked.txt").exists());
    assert!(matches!(
        summary.failures[0].error,
        OperationError::AccessDenied | OperationError::Io { .. }
    ));
}

#[test]
fn refuses_a_folder_copied_into_its_own_subfolder_at_plan_time() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("src");
    let inner = source.join("inner");
    fs::create_dir_all(&inner).unwrap();

    let platform = TestPlatform::new(dir.path());
    let mut resolver = FixedResolver {
        answer: ConflictAnswer::once(ConflictChoice::KeepBoth),
    };
    let plan = plan_transfer(TransferKind::Copy, &[source], &inner);
    assert_eq!(plan, Err(OperationError::SourceInsideDestination));

    let mut sink = |_progress: &Progress| {};
    let _ = Transfer {
        id: OperationId::next(),
        port: &platform,
        control: &OperationControl::new(),
        resolver: &mut resolver,
        on_progress: &mut sink,
    };
}

#[test]
fn a_selected_size_that_cannot_fit_is_refused_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("big.bin");
    write(&source, &vec![0u8; 4096]);
    let destination = dir.path().join("dest");
    fs::create_dir(&destination).unwrap();

    let platform = TestPlatform::new(dir.path()).with_free_space(1024);
    let mut resolver = ScriptedResolver::new(vec![]);
    let summary = run(
        std::slice::from_ref(&source),
        &destination,
        TransferKind::Copy,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::Failed);
    assert!(matches!(
        summary.error,
        Some(OperationError::InsufficientSpace { .. })
    ));
    assert_eq!(summary.bytes_done, 0);
    assert!(
        !destination.join("big.bin").exists(),
        "nothing may be written when the volume cannot hold the selection"
    );
}

#[test]
fn a_file_that_does_not_fit_fails_before_its_target_is_created() {
    let dir = tempfile::tempdir().unwrap();
    // A folder selection has no known size, so the per-file check is what refuses this copy.
    let source = dir.path().join("folder");
    write(&source.join("big.bin"), &vec![0u8; 4096]);
    let destination = dir.path().join("dest");
    fs::create_dir(&destination).unwrap();

    let platform = TestPlatform::new(dir.path()).with_free_space(1024);
    let mut resolver = ScriptedResolver::new(vec![]);
    let summary = run(
        std::slice::from_ref(&source),
        &destination,
        TransferKind::Copy,
        &platform,
        &OperationControl::new(),
        &mut resolver,
    );

    assert_eq!(summary.state, OperationState::CompletedWithWarnings);
    assert!(matches!(
        summary.failures[0].error,
        OperationError::InsufficientSpace { .. }
    ));
    assert!(!destination.join("folder").join("big.bin").exists());
}
