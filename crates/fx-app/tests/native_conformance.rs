//! Tranche 3 slice 5 conformance: the same fixture through both transfer routes.
//!
//! Spec §12 asks for a suite that puts one fixture through the built-in engine and through
//! `IFileOperation` and asserts they agree, so `Ctrl+Z`, the activity center and the user
//! cannot tell the routes apart. Decision 3 is the contract: the undo entries must be
//! equivalent, metadata must match, and a per-item failure must degrade the same way.
//!
//! These tests drive the Shell for real. They run the native route with
//! `show_progress: false` (`FOF_SILENT | FOF_NOCONFIRMATION`), so a runner without an
//! interactive desktop can never block on a modal dialog.
//!
//! What they cannot cover, and what stays a manual row in `docs/tranche-3-validation.md`:
//! the native progress dialog itself, the 2 GB acceptance copy, cancellation mid-run (the
//! sink's `E_ABORT` path is unvalidated — the slice-0 spike never ran on this workstation),
//! and the creation-time half of metadata parity, which depends on the host adapter's
//! `SetFileTimeW` and therefore on `WindowsPlatformOps` rather than on a test port.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool, mpsc::RecvTimeoutError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use fx_ops::{
    conflict::{ConflictAnswer, ConflictChoice, ConflictRequest, ConflictResolver},
    control::OperationControl,
    plan::{OperationPlan, PathPair, TransferKind, plan_transfer},
    port::{PlatformOps, RecycleError, RecyclePrecheck, RecycledItem, RestoreError, VolumeKind},
    progress::{OperationId, OperationKind, OperationState, OperationSummary},
    routing::{
        NativeCapability, TransferRoute, apply_native_metadata, choose_route, scan_destination,
        verify_destinations,
    },
    undo::{ReversibleAction, UndoEntry, UndoStack, move_entry},
};
use fx_shell::{NativeEvent, NativeKind, NativeTransferRequest, run_native_transfer};

/// How long a native operation may take before the test calls it hung.
const NATIVE_TIMEOUT: Duration = Duration::from_secs(120);

// ── Fixtures ──────────────────────────────────────────────────────────────────

/// A scratch directory that cleans itself up.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "ferrix-conformance-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("fixture root");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    /// A sub-directory of the fixture, created on demand.
    fn dir(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir_all(&path).expect("fixture directory");
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A small tree with a nested folder and non-ASCII names: the shapes both routes must agree
/// on, and the Unicode case spec §12 asks for.
fn tree(root: &Path) -> PathBuf {
    let source = root.join("source");
    fs::create_dir_all(source.join("子目录")).expect("nested directory");
    fs::write(source.join("notes.txt"), b"notes").expect("write notes");
    fs::write(source.join("子目录").join("файл.bin"), vec![7u8; 4096]).expect("write payload");
    source
}

fn set_mtime(path: &Path, stamp: SystemTime) {
    let file = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("reopen the fixture file");
    file.set_modified(stamp).expect("set the modification time");
}

fn modified(path: &Path) -> SystemTime {
    fs::metadata(path)
        .expect("metadata")
        .modified()
        .expect("modification time")
}

fn readonly(path: &Path) -> bool {
    fs::metadata(path)
        .expect("metadata")
        .permissions()
        .readonly()
}

// ── The built-in route ────────────────────────────────────────────────────────

/// The port the conformance suite gives `fx-ops`: `std` copy semantics, no Recycle Bin.
struct TestPort;

impl PlatformOps for TestPort {
    fn volume_id(&self, _path: &Path) -> io::Result<u64> {
        Ok(0)
    }

    fn volume_kind(&self, _path: &Path) -> io::Result<VolumeKind> {
        Ok(VolumeKind::Fixed)
    }

    fn free_space(&self, _path: &Path) -> io::Result<u64> {
        Ok(u64::MAX)
    }

    fn replace_file(&self, replacement: &Path, destination: &Path) -> io::Result<()> {
        fs::rename(replacement, destination)
    }

    fn can_recycle(&self, _paths: &[PathBuf]) -> Result<RecyclePrecheck, RecycleError> {
        Ok(RecyclePrecheck::Possible)
    }

    fn recycle(&self, _paths: &[PathBuf]) -> Result<Vec<RecycledItem>, RecycleError> {
        Err(RecycleError {
            code: 0,
            message: "the conformance suite has no Recycle Bin".to_owned(),
        })
    }

    fn restore_recycled(&self, item: &RecycledItem) -> Result<(), RestoreError> {
        fs::write(&item.original, b"restored").map_err(|error| RestoreError {
            message: error.to_string(),
        })
    }
}

/// Answers every conflict by keeping both, which a clean fixture never asks for.
struct KeepBothResolver;

impl ConflictResolver for KeepBothResolver {
    fn resolve(&mut self, _request: &ConflictRequest) -> Option<ConflictAnswer> {
        Some(ConflictAnswer::once(ConflictChoice::KeepBoth))
    }
}

/// Runs the Tranche 2 engine over `plan`, exactly as the application's worker does.
fn run_built_in(plan: &OperationPlan) -> OperationSummary {
    let port = TestPort;
    let control = OperationControl::new();
    let mut resolver = KeepBothResolver;
    let mut on_progress = |_: &fx_ops::Progress| {};
    let mut transfer = fx_ops::transfer::Transfer {
        id: OperationId::next(),
        port: &port,
        control: &control,
        resolver: &mut resolver,
        on_progress: &mut on_progress,
    };
    transfer.run(plan)
}

// ── The native route ──────────────────────────────────────────────────────────

/// Runs `plan` through `IFileOperation` and returns every event up to the terminal one.
fn run_native(plan: &OperationPlan) -> Vec<NativeEvent> {
    let request = NativeTransferRequest {
        kind: match plan.kind {
            OperationKind::Move => NativeKind::Move,
            _ => NativeKind::Copy,
        },
        sources: plan.items.iter().map(|item| item.source.clone()).collect(),
        destination: plan.destination_root.clone(),
        owner: None,
        show_progress: false,
    };
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let handle = run_native_transfer(request, event_tx, Arc::new(AtomicBool::new(false)));
    let mut events = Vec::new();
    loop {
        match event_rx.recv_timeout(NATIVE_TIMEOUT) {
            Ok(event) => {
                let terminal = matches!(event, NativeEvent::Finished { .. });
                events.push(event);
                if terminal {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                panic!("the native transfer never finished: {events:?}")
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    handle.join();
    events
}

/// The terminal event, as `(aborted, error)`.
fn terminal(events: &[NativeEvent]) -> Option<(bool, Option<fx_shell::NativeError>)> {
    events.iter().find_map(|event| match event {
        NativeEvent::Finished { aborted, error } => Some((*aborted, error.clone())),
        _ => None,
    })
}

/// The pairs the sink reported, in order.
fn native_pairs(events: &[NativeEvent]) -> Vec<PathPair> {
    events
        .iter()
        .filter_map(|event| match event {
            NativeEvent::ItemDone {
                source,
                destination,
            } => Some(PathPair {
                from: source.clone(),
                to: destination.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn count_item_failures(events: &[NativeEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, NativeEvent::ItemFailed { .. }))
        .count()
}

fn plan_pairs(plan: &OperationPlan) -> Vec<PathPair> {
    plan.items
        .iter()
        .map(|item| PathPair {
            from: item.source.clone(),
            to: item.destination.clone(),
        })
        .collect()
}

/// The endpoints of a pair, by name: the two fixtures live under different roots, so the
/// structure is what can be compared.
fn name_shape(pairs: &[PathPair]) -> Vec<(String, String)> {
    let mut shape: Vec<(String, String)> = pairs
        .iter()
        .map(|pair| {
            (
                pair.from
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                pair.to
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )
        })
        .collect();
    shape.sort();
    shape
}

fn move_pairs_of(entry: &UndoEntry) -> Vec<PathPair> {
    match &entry.action {
        ReversibleAction::Move { pairs } => pairs.clone(),
        other => panic!("a move must record a Move action, got {other:?}"),
    }
}

// ── Decision 3: undo equivalence ──────────────────────────────────────────────

#[test]
fn both_routes_record_the_same_undo_entry_for_a_move() {
    let fixture = Fixture::new("undo-parity");
    let built_in_dest = fixture.dir("built-in");
    let native_dest = fixture.dir("native");
    let second_root = fixture.dir("second-root");

    let built_in_source = tree(fixture.path());
    let native_source = tree(&second_root);

    // The built-in engine.
    let built_in_plan = plan_transfer(
        TransferKind::Move,
        std::slice::from_ref(&built_in_source),
        &built_in_dest,
    )
    .expect("plan the built-in move");
    let built_in_summary = run_built_in(&built_in_plan);
    assert_eq!(
        built_in_summary.state,
        OperationState::Completed,
        "the built-in move must complete: {:?}",
        built_in_summary.failures
    );
    assert!(built_in_dest.join("source").join("notes.txt").exists());

    // The Shell's route.
    let native_plan = plan_transfer(
        TransferKind::Move,
        std::slice::from_ref(&native_source),
        &native_dest,
    )
    .expect("plan the native move");
    let events = run_native(&native_plan);
    assert_eq!(
        terminal(&events),
        Some((false, None)),
        "the native move must finish cleanly: {events:?}"
    );
    assert!(native_dest.join("source").join("notes.txt").exists());
    assert!(
        !native_source.exists(),
        "a move must leave no source behind (post-condition)"
    );
    assert!(
        verify_destinations(OperationKind::Move, &native_plan).is_empty(),
        "the native move must satisfy the post-condition check"
    );

    // Decision 3: the same entry shape from both routes.
    let built_in_entry = move_entry(OperationId::next(), plan_pairs(&built_in_plan));
    let native_entry = move_entry(OperationId::next(), native_pairs(&events));
    assert_eq!(
        name_shape(&move_pairs_of(&built_in_entry)),
        name_shape(&move_pairs_of(&native_entry)),
        "Ctrl+Z cannot tell the routes apart"
    );
    assert_eq!(
        move_pairs_of(&native_entry).len(),
        native_plan.items.len(),
        "one pair per planned item, not one per descendant"
    );

    // And the native entry really undoes: back to the original path.
    let mut stack = UndoStack::new();
    stack.record(native_entry);
    stack
        .undo(&TestPort, &OperationControl::new())
        .expect("the native move must be reversible");
    assert!(
        native_source.join("notes.txt").exists(),
        "undo restored the tree"
    );
    assert!(
        !native_dest.join("source").exists(),
        "undo moved the tree back, it did not copy it"
    );
}

// ── Decision 9: metadata parity ───────────────────────────────────────────────

#[test]
fn a_native_copy_carries_the_same_metadata_as_a_built_in_copy() {
    let fixture = Fixture::new("metadata");
    let built_in_dest = fixture.dir("built-in");
    let native_dest = fixture.dir("native");

    let source = fixture.path().join("stamped.txt");
    fs::write(&source, b"payload").expect("write the source");
    let stamp = UNIX_EPOCH + Duration::from_secs(1_622_937_600);
    set_mtime(&source, stamp);
    fs::set_permissions(&source, {
        let mut permissions = fs::metadata(&source).expect("metadata").permissions();
        permissions.set_readonly(true);
        permissions
    })
    .expect("mark the source read-only");

    let built_in_plan = plan_transfer(
        TransferKind::Copy,
        std::slice::from_ref(&source),
        &built_in_dest,
    )
    .expect("plan the built-in copy");
    assert_eq!(
        run_built_in(&built_in_plan).state,
        OperationState::Completed
    );

    let native_plan = plan_transfer(
        TransferKind::Copy,
        std::slice::from_ref(&source),
        &native_dest,
    )
    .expect("plan the native copy");
    let events = run_native(&native_plan);
    assert_eq!(
        terminal(&events),
        Some((false, None)),
        "the native copy must finish cleanly: {events:?}"
    );

    // The post-pass the application runs after every native copy (decision 9).
    let pairs = native_pairs(&events);
    let unstamped = apply_native_metadata(&TestPort, OperationKind::Copy, &pairs);
    assert!(
        unstamped.is_empty(),
        "the post-pass must stamp every pair: {unstamped:?}"
    );

    let built_in_copy = built_in_dest.join("stamped.txt");
    let native_copy = native_dest.join("stamped.txt");
    assert_eq!(
        modified(&built_in_copy),
        modified(&source),
        "the built-in route carries the modification time"
    );
    assert_eq!(
        modified(&native_copy),
        modified(&source),
        "the native route carries it too, through the post-pass"
    );
    assert_eq!(readonly(&native_copy), readonly(&source));
    assert!(
        verify_destinations(OperationKind::Copy, &native_plan).is_empty(),
        "the native copy satisfies its post-condition"
    );
}

// ── Failure parity ────────────────────────────────────────────────────────────

#[test]
fn a_locked_source_fails_the_same_way_on_both_routes() {
    let fixture = Fixture::new("locked");
    let built_in_dest = fixture.dir("built-in");
    let native_dest = fixture.dir("native");

    let built_in_source = fixture.path().join("locked-builtin.txt");
    let native_source = fixture.path().join("locked-native.txt");
    fs::write(&built_in_source, b"x").expect("write the built-in source");
    fs::write(&native_source, b"x").expect("write the native source");

    // No sharing: a copy of either file must fail. Raw Win32 lives in fx-shell (rule 3).
    let built_in_lock = fx_shell::open_locked(&built_in_source).expect("lock the built-in source");
    let native_lock = fx_shell::open_locked(&native_source).expect("lock the native source");

    let built_in_plan = plan_transfer(
        TransferKind::Copy,
        std::slice::from_ref(&built_in_source),
        &built_in_dest,
    )
    .expect("plan the built-in copy");
    let built_in = run_built_in(&built_in_plan);

    let native_plan = plan_transfer(
        TransferKind::Copy,
        std::slice::from_ref(&native_source),
        &native_dest,
    )
    .expect("plan the native copy");
    let events = run_native(&native_plan);

    drop(built_in_lock);
    drop(native_lock);

    assert_eq!(
        built_in.state,
        OperationState::CompletedWithWarnings,
        "the built-in engine reports one failed item, never a success"
    );
    assert_eq!(built_in.failures.len(), 1);

    // The Shell's abort flag is deliberately not asserted here: it sets that flag when it
    // skips a failed item (measured on this host), which is why the application derives
    // "cancelled" from the failures instead of copying the flag.
    let (_aborted, error) = terminal(&events).expect("a terminal event");
    assert!(
        error.is_none(),
        "a per-item failure is not an operation-level error: {error:?}"
    );
    assert_eq!(
        count_item_failures(&events),
        1,
        "the sink reports exactly one failed item: {events:?}"
    );
    assert!(
        !native_dest.join("locked-native.txt").exists(),
        "the failed item must not appear at the destination"
    );
    assert_eq!(
        verify_destinations(OperationKind::Copy, &native_plan).len(),
        1,
        "the post-condition check agrees that the item did not land"
    );
}

// ── Decision 1: the gate ──────────────────────────────────────────────────────

#[test]
fn a_clean_transfer_takes_the_native_route_when_the_shell_offers_it() {
    assert!(
        fx_shell::probe_file_operation(),
        "IFileOperation must exist on Windows 10/11; without it every transfer falls back"
    );
    let fixture = Fixture::new("gate-clean");
    let destination = fixture.dir("dest");
    let source = fixture.path().join("a.txt");
    fs::write(&source, b"new").expect("write the source");

    let plan = plan_transfer(TransferKind::Copy, &[source], &destination).expect("plan");
    let scan = scan_destination(&plan).expect("scan");
    let route = choose_route(
        &scan,
        plan.kind,
        NativeCapability {
            file_operation: true,
        },
    );
    assert_eq!(route, TransferRoute::Native);
}

#[test]
fn the_gate_keeps_a_conflicting_transfer_on_the_built_in_engine() {
    let fixture = Fixture::new("gate-conflict");
    let destination = fixture.dir("dest");
    let source = fixture.path().join("a.txt");
    fs::write(&source, b"new").expect("write the source");
    fs::write(destination.join("a.txt"), b"old").expect("the destination already exists");

    let plan = plan_transfer(TransferKind::Copy, &[source], &destination).expect("plan");
    let scan = scan_destination(&plan).expect("scan");
    assert_eq!(scan.existing_destinations, vec![destination.join("a.txt")]);

    let route = choose_route(
        &scan,
        plan.kind,
        NativeCapability {
            file_operation: true,
        },
    );
    assert_eq!(
        route,
        TransferRoute::BuiltIn,
        "a conflict must reach the four-choice dialog, not the Shell's"
    );
}

// ── Long paths ────────────────────────────────────────────────────────────────

#[test]
fn a_native_copy_lands_a_path_past_the_legacy_limit() {
    let fixture = Fixture::new("longpath");
    let destination = fixture.dir("dest");

    // Built with the verbatim prefix so the fixture itself can be created at all.
    let verbatim_root = PathBuf::from(format!(r"\\?\{}", fixture.path().display()));
    let mut deepest = verbatim_root.join("long");
    fs::create_dir(&deepest).expect("long source root");
    while deepest.as_os_str().len() < 280 {
        let next = deepest.join("segment");
        if fs::create_dir(&next).is_err() {
            break;
        }
        deepest = next;
    }
    let deep_file = deepest.join("deep.txt");
    fs::write(&deep_file, b"deep payload").expect("write the deep file");
    assert!(
        deep_file.as_os_str().len() > 260,
        "the fixture must exceed MAX_PATH, got {}",
        deep_file.as_os_str().len()
    );

    let plan = plan_transfer(TransferKind::Copy, &[deep_file], &destination).expect("plan");
    let events = run_native(&plan);
    assert_eq!(
        terminal(&events),
        Some((false, None)),
        "a long-path copy must not be an operation-level failure: {events:?}"
    );
    let landed = destination.join("deep.txt");
    assert!(
        landed.exists(),
        "the Shell must place the file past the legacy limit; events: {events:?}"
    );
    assert_eq!(fs::read(&landed).expect("read the copy"), b"deep payload");
}
