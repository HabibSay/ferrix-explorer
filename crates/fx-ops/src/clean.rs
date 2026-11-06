//! The empty-folder cleaner: scan, report, then remove what is still empty.
//!
//! The scan never includes `root`, never follows a reparse point, and returns directories
//! deepest-first so a parent that only contained empty children becomes removable in the
//! same pass. `clean` re-verifies emptiness before every removal: a file created between the
//! scan and the confirmation turns that directory into a skip, not a deletion.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    control::OperationControl,
    error::{ItemFailure, OperationError},
    plan::same_path,
    progress::{OperationId, OperationKind, OperationSummary},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CleanPlan {
    /// Deepest first; never contains the scanned root.
    pub directories: Vec<PathBuf>,
}

pub fn scan_empty_dirs(
    root: &Path,
    control: &OperationControl,
) -> Result<CleanPlan, OperationError> {
    let metadata = fs::symlink_metadata(root).map_err(|error| OperationError::from_io(&error))?;
    if !metadata.is_dir() {
        return Err(OperationError::NotADirectory);
    }

    let mut plan = CleanPlan::default();
    scan_children(root, true, control, &mut plan.directories)?;
    Ok(plan)
}

/// Returns whether `dir` is empty after accounting for children that will be removed.
fn scan_children(
    dir: &Path,
    is_root: bool,
    control: &OperationControl,
    out: &mut Vec<PathBuf>,
) -> Result<bool, OperationError> {
    control.checkpoint()?;
    let mut empty = true;

    for entry in fs::read_dir(dir).map_err(|error| OperationError::from_io(&error))? {
        let entry = entry.map_err(|error| OperationError::from_io(&error))?;
        let path = entry.path();
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| OperationError::from_io(&error))?;

        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            if !scan_children(&path, false, control, out)? {
                empty = false;
            }
        } else {
            empty = false;
        }
    }

    if empty && !is_root {
        out.push(dir.to_path_buf());
    }
    Ok(empty)
}

pub fn clean(root: &Path, plan: &CleanPlan, control: &OperationControl) -> OperationSummary {
    let mut summary = OperationSummary::new(OperationId::next(), OperationKind::CleanEmptyFolders);
    let mut cancelled = false;

    for directory in &plan.directories {
        if control.checkpoint().is_err() {
            cancelled = true;
            break;
        }
        if same_path(directory, root) {
            continue;
        }

        match fs::read_dir(directory) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    summary.files_skipped += 1;
                    continue;
                }
            }
            Err(error) => {
                summary
                    .record_failure(ItemFailure::new(directory, OperationError::from_io(&error)));
                continue;
            }
        }

        match fs::remove_dir(directory) {
            Ok(()) => summary.files_done += 1,
            Err(error) => {
                summary.record_failure(ItemFailure::new(directory, OperationError::from_io(&error)))
            }
        }
    }

    summary.finish(cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_lists_empty_dirs_deepest_first_without_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let deep = root.join("a").join("b").join("c");
        fs::create_dir_all(&deep).unwrap();
        fs::create_dir(root.join("full")).unwrap();
        fs::write(root.join("full").join("file.txt"), b"x").unwrap();

        let plan = scan_empty_dirs(root, &OperationControl::new()).unwrap();
        assert!(!plan.directories.contains(&root.to_path_buf()));
        assert!(plan.directories.contains(&deep));
        assert!(plan.directories.contains(&root.join("a")));
        let deep_index = plan
            .directories
            .iter()
            .position(|path| path == &deep)
            .unwrap();
        let parent_index = plan
            .directories
            .iter()
            .position(|path| path == &root.join("a"))
            .unwrap();
        assert!(deep_index < parent_index, "children must come first");
        assert!(!plan.directories.contains(&root.join("full")));
    }

    #[test]
    fn clean_removes_only_what_is_still_empty() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let empty = root.join("empty");
        let becomes_full = root.join("race");
        fs::create_dir_all(&empty).unwrap();
        fs::create_dir_all(&becomes_full).unwrap();

        let plan = scan_empty_dirs(root, &OperationControl::new()).unwrap();
        assert_eq!(plan.directories.len(), 2);

        // A file appears between the scan and the confirmation.
        fs::write(becomes_full.join("late.txt"), b"x").unwrap();

        let summary = clean(root, &plan, &OperationControl::new());
        assert_eq!(summary.files_done, 1);
        assert_eq!(summary.files_skipped, 1);
        assert!(!empty.exists());
        assert!(becomes_full.exists());
        assert!(root.exists(), "the root is never removed");
    }

    #[test]
    fn scan_refuses_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file.txt");
        fs::write(&file, b"x").unwrap();
        assert_eq!(
            scan_empty_dirs(&file, &OperationControl::new()),
            Err(OperationError::NotADirectory)
        );
    }

    #[test]
    fn cancelled_scan_stops() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("a")).unwrap();
        let control = OperationControl::new();
        control.cancel();
        assert_eq!(
            scan_empty_dirs(dir.path(), &control),
            Err(OperationError::Cancelled)
        );
    }
}
