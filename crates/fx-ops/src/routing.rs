//! Destination probe and route selection for the native transfer (Tranche 3, slice 1).
//!
//! Decision 1 of the Tranche 3 design: the Shell's own `IFileOperation` route is used
//! only for a clean copy or move. A cheap probe over the planned entries decides it —
//! one stat per planned destination, plus a recursive probe of the source tree when the
//! plan contains directories. Everything else runs the Tranche 2 engine with its
//! four-choice conflict dialog, so no user-visible conflict behaviour regresses.
//!
//! This module is pure `std`: no COM, no UI, no `unsafe`. The caller supplies the
//! [`NativeCapability`] it probed once on the host.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::plan::OperationPlan;
use crate::progress::OperationKind;

/// What the host can do natively, probed once by `fx-app`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativeCapability {
    /// `IFileOperation` co-created successfully on this machine.
    pub file_operation: bool,
}

/// The result of the destination probe.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConflictScan {
    /// Planned destinations that already exist on disk.
    pub existing_destinations: Vec<PathBuf>,
    /// The transfer carries a reparse point, at the top level or deeper.
    pub reparse_points: bool,
}

/// Which engine performs the transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferRoute {
    /// The Shell's `IFileOperation` route, with the Shell's own progress dialog.
    Native,
    /// The Tranche 2 engine, with its four-choice conflict dialog.
    BuiltIn,
}

/// Probes the planned destinations.
///
/// One stat per planned destination yields the entries that already exist. Each source
/// is stat'ed too, because the plan only flags symbolic links while Tranche 2 refuses
/// junctions and other reparse kinds (rule 6) and the Shell copies them by its own
/// rules. When the plan contains a directory, its source tree is walked as well: a
/// nested reparse point would change rule 6 behaviour under the Shell just as a
/// top-level one does.
pub fn scan_destination(plan: &OperationPlan) -> io::Result<ConflictScan> {
    let mut scan = ConflictScan::default();

    for item in &plan.items {
        if item.is_reparse_point {
            scan.reparse_points = true;
        } else if let Ok(metadata) = fs::symlink_metadata(&item.source)
            && is_reparse_point(&metadata)
        {
            scan.reparse_points = true;
        }
        match fs::symlink_metadata(&item.destination) {
            Ok(_) => scan.existing_destinations.push(item.destination.clone()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }

    for item in &plan.items {
        if item.is_dir && probe_tree(&item.source, &|_, metadata| is_reparse_point(metadata))? {
            scan.reparse_points = true;
        }
    }

    Ok(scan)
}

/// Total routing function: the native route only for a clean copy or move.
///
/// Every other combination — another kind, an existing destination, a reparse point or
/// a missing native capability — runs the built-in engine.
pub fn choose_route(
    scan: &ConflictScan,
    kind: OperationKind,
    native: NativeCapability,
) -> TransferRoute {
    let clean = scan.existing_destinations.is_empty() && !scan.reparse_points;
    match kind {
        OperationKind::Copy | OperationKind::Move if clean && native.file_operation => {
            TransferRoute::Native
        }
        _ => TransferRoute::BuiltIn,
    }
}

/// Walks `root` and reports whether `detector` finds a reparse point.
///
/// The walk stops at the first hit: the answer is a yes/no, not an inventory.
fn probe_tree(root: &Path, detector: &dyn Fn(&Path, &fs::Metadata) -> bool) -> io::Result<bool> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if detector(&path, &metadata) {
                return Ok(true);
            }
            if metadata.is_dir() {
                pending.push(path);
            }
        }
    }
    Ok(false)
}

/// Whether the entry is a reparse point (symbolic link, junction, or cloud placeholder).
#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    has_reparse_attribute(metadata.file_attributes())
}

/// Whether the Windows file attributes mark the entry as a reparse point.
#[cfg(windows)]
fn has_reparse_attribute(attributes: u32) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// Whether the entry is a reparse point. Off Windows only symbolic links exist.
#[cfg(not(windows))]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{OperationPlan, PlanItem};

    fn plan_item(source: PathBuf, destination: PathBuf, is_dir: bool) -> PlanItem {
        PlanItem {
            source,
            destination,
            is_dir,
            size: None,
            is_reparse_point: false,
        }
    }

    fn plan_of(items: Vec<PlanItem>, kind: OperationKind) -> OperationPlan {
        OperationPlan {
            kind,
            destination_root: PathBuf::from("C:\\dest"),
            items,
            files_total: 0,
            bytes_total: 0,
        }
    }

    #[test]
    fn clean_copy_with_capability_routes_native() {
        let scan = ConflictScan::default();
        let route = choose_route(
            &scan,
            OperationKind::Copy,
            NativeCapability {
                file_operation: true,
            },
        );
        assert_eq!(route, TransferRoute::Native);
    }

    #[test]
    fn clean_move_with_capability_routes_native() {
        let scan = ConflictScan::default();
        let route = choose_route(
            &scan,
            OperationKind::Move,
            NativeCapability {
                file_operation: true,
            },
        );
        assert_eq!(route, TransferRoute::Native);
    }

    #[test]
    fn existing_destination_routes_built_in() {
        let scan = ConflictScan {
            existing_destinations: vec![PathBuf::from("C:\\dest\\a.txt")],
            reparse_points: false,
        };
        let route = choose_route(
            &scan,
            OperationKind::Copy,
            NativeCapability {
                file_operation: true,
            },
        );
        assert_eq!(route, TransferRoute::BuiltIn);
    }

    #[test]
    fn reparse_point_routes_built_in() {
        let scan = ConflictScan {
            existing_destinations: Vec::new(),
            reparse_points: true,
        };
        let route = choose_route(
            &scan,
            OperationKind::Move,
            NativeCapability {
                file_operation: true,
            },
        );
        assert_eq!(route, TransferRoute::BuiltIn);
    }

    #[test]
    fn missing_capability_routes_built_in() {
        let scan = ConflictScan::default();
        let route = choose_route(&scan, OperationKind::Copy, NativeCapability::default());
        assert_eq!(route, TransferRoute::BuiltIn);
    }

    #[test]
    fn destructive_and_read_only_kinds_never_route_native() {
        let scan = ConflictScan::default();
        let native = NativeCapability {
            file_operation: true,
        };
        for kind in [
            OperationKind::Rename,
            OperationKind::CreateDirectory,
            OperationKind::CreateFile,
            OperationKind::Recycle,
            OperationKind::DeletePermanent,
            OperationKind::Restore,
            OperationKind::Checksum,
            OperationKind::CleanEmptyFolders,
        ] {
            assert_eq!(
                choose_route(&scan, kind, native),
                TransferRoute::BuiltIn,
                "{kind:?} must not take the native route"
            );
        }
    }

    #[test]
    fn scan_finds_no_conflict_for_a_fresh_destination() {
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("a.txt");
        fs::write(&source, b"hello").expect("write");
        let destination = dir.path().join("dest");
        fs::create_dir(&destination).expect("dest dir");

        let plan = plan_of(
            vec![plan_item(source, destination.join("a.txt"), false)],
            OperationKind::Copy,
        );

        let scan = scan_destination(&plan).expect("scan");
        assert!(scan.existing_destinations.is_empty());
        assert!(!scan.reparse_points);
    }

    #[test]
    fn scan_finds_an_existing_file_destination() {
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("a.txt");
        fs::write(&source, b"hello").expect("write");
        let destination = dir.path().join("dest");
        fs::create_dir(&destination).expect("dest dir");
        let taken = destination.join("a.txt");
        fs::write(&taken, b"older").expect("write");

        let plan = plan_of(
            vec![plan_item(source, taken.clone(), false)],
            OperationKind::Copy,
        );

        let scan = scan_destination(&plan).expect("scan");
        assert_eq!(scan.existing_destinations, vec![taken]);
    }

    #[test]
    fn scan_finds_an_existing_directory_destination() {
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("folder");
        fs::create_dir(&source).expect("source dir");
        let destination = dir.path().join("dest");
        fs::create_dir(&destination).expect("dest dir");
        let taken = destination.join("folder");
        fs::create_dir(&taken).expect("existing dir");

        let plan = plan_of(
            vec![plan_item(source, taken.clone(), true)],
            OperationKind::Copy,
        );

        let scan = scan_destination(&plan).expect("scan");
        assert_eq!(scan.existing_destinations, vec![taken]);
    }

    #[test]
    fn scan_flags_a_reparse_point_the_plan_declares() {
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("link.txt");
        fs::write(&source, b"x").expect("write");

        let mut item = plan_item(source, dir.path().join("dest").join("link.txt"), false);
        item.is_reparse_point = true;
        let plan = plan_of(vec![item], OperationKind::Copy);

        let scan = scan_destination(&plan).expect("scan");
        assert!(scan.reparse_points);
    }

    #[test]
    fn probe_tree_finds_a_nested_entry_the_detector_flags() {
        let dir = tempfile::tempdir().expect("temp dir");
        let nested = dir.path().join("sub");
        fs::create_dir(&nested).expect("nested dir");
        fs::write(nested.join("deep.txt"), b"x").expect("write");

        let found = probe_tree(dir.path(), &|path: &Path, _| {
            path.file_name().is_some_and(|name| name == "deep.txt")
        })
        .expect("probe");
        assert!(found);
    }

    #[test]
    fn probe_tree_reports_nothing_for_a_plain_tree() {
        let dir = tempfile::tempdir().expect("temp dir");
        let nested = dir.path().join("sub");
        fs::create_dir(&nested).expect("nested dir");
        fs::write(nested.join("deep.txt"), b"x").expect("write");

        let found = probe_tree(dir.path(), &|_, _| false).expect("probe");
        assert!(!found);
    }

    #[cfg(windows)]
    #[test]
    fn windows_reparse_attribute_is_recognised() {
        assert!(has_reparse_attribute(0x400));
        assert!(has_reparse_attribute(0x400 | 0x80));
        assert!(!has_reparse_attribute(0x80));
        assert!(!has_reparse_attribute(0));
    }
}
