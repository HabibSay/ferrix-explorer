//! Planning a transfer before anything touches the disk.
//!
//! Planning is where the destructive mistakes are prevented: copying a folder into its own
//! descendant, a destination that is not a folder, a source that vanished. The engine can
//! then assume a coherent plan.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{error::OperationError, progress::OperationKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferKind {
    Copy,
    Move,
}

impl TransferKind {
    pub fn operation_kind(self) -> OperationKind {
        match self {
            Self::Copy => OperationKind::Copy,
            Self::Move => OperationKind::Move,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathPair {
    pub from: PathBuf,
    pub to: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanItem {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub is_dir: bool,
    /// `None` for directories and reparse points: their size needs a walk, and Tranche 2
    /// deliberately does not walk a tree twice just to print an ETA.
    pub size: Option<u64>,
    pub is_reparse_point: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationPlan {
    pub kind: OperationKind,
    pub destination_root: PathBuf,
    pub items: Vec<PlanItem>,
    pub files_total: u64,
    pub bytes_total: u64,
}

pub fn plan_transfer(
    kind: TransferKind,
    sources: &[PathBuf],
    destination: &Path,
) -> Result<OperationPlan, OperationError> {
    if sources.is_empty() {
        return Err(OperationError::InvalidName("no source selected"));
    }

    let metadata =
        fs::symlink_metadata(destination).map_err(|error| OperationError::from_io(&error))?;
    if !metadata.is_dir() {
        return Err(OperationError::NotADirectory);
    }

    let mut items = Vec::with_capacity(sources.len());
    let mut files_total = 0;
    let mut bytes_total = 0;

    for source in sources {
        let metadata =
            fs::symlink_metadata(source).map_err(|error| OperationError::from_io(&error))?;
        let is_reparse_point = metadata.file_type().is_symlink();
        let is_dir = metadata.is_dir() && !is_reparse_point;

        if is_dir && is_descendant(source, destination) {
            return Err(OperationError::SourceInsideDestination);
        }

        let name = source
            .file_name()
            .ok_or(OperationError::InvalidName("source has no file name"))?;
        let target = destination.join(name);

        // `source == target` is a legitimate same-folder paste; the engine forces keep-both
        // for that case instead of replacing a file with itself.
        let size = if is_dir || is_reparse_point {
            None
        } else {
            Some(metadata.len())
        };
        if let Some(size) = size {
            files_total += 1;
            bytes_total += size;
        }

        items.push(PlanItem {
            source: source.clone(),
            destination: target,
            is_dir,
            size,
            is_reparse_point,
        });
    }

    Ok(OperationPlan {
        kind: kind.operation_kind(),
        destination_root: destination.to_path_buf(),
        items,
        files_total,
        bytes_total,
    })
}

/// Whether two paths name the same file, tolerating a verbatim prefix and Windows casing.
pub fn same_path(left: &Path, right: &Path) -> bool {
    comparable(left) == comparable(right)
}

/// Whether `candidate` is strictly inside `ancestor`.
pub fn is_descendant(ancestor: &Path, candidate: &Path) -> bool {
    let ancestor = comparable(ancestor);
    let candidate = comparable(candidate);
    candidate.len() > ancestor.len() && candidate.starts_with(&ancestor)
}

fn comparable(path: &Path) -> Vec<String> {
    let text = path.as_os_str().to_string_lossy();
    let stripped = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        rest
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest
    } else {
        text.as_ref()
    };
    stripped
        .split(['\\', '/'])
        .filter(|part| !part.is_empty())
        .map(fold_case)
        .collect()
}

fn fold_case(part: &str) -> String {
    #[cfg(windows)]
    {
        part.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        part.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    #[test]
    fn same_path_ignores_verbatim_prefix_and_case() {
        assert!(same_path(
            Path::new(r"C:\Temp\File.txt"),
            Path::new(r"\\?\C:\Temp\FILE.TXT")
        ));
        assert!(!same_path(Path::new(r"C:\Temp\A"), Path::new(r"C:\Temp\B")));
    }

    #[test]
    fn descendant_is_strict() {
        assert!(is_descendant(
            Path::new(r"C:\Temp"),
            Path::new(r"C:\Temp\Sub")
        ));
        assert!(is_descendant(
            Path::new(r"C:\Temp"),
            Path::new(r"C:\Temp\Sub\Deeper")
        ));
        assert!(!is_descendant(Path::new(r"C:\Temp"), Path::new(r"C:\Temp")));
        assert!(!is_descendant(
            Path::new(r"C:\Temp"),
            Path::new(r"C:\TempFolder")
        ));
    }

    #[test]
    fn plan_counts_files_and_bytes() {
        let dir = tempdir();
        let destination = dir.path().join("dest");
        fs::create_dir(&destination).unwrap();
        let source = dir.path().join("a.txt");
        fs::write(&source, b"hello").unwrap();

        let plan = plan_transfer(TransferKind::Copy, &[source], &destination).unwrap();
        assert_eq!(plan.kind, OperationKind::Copy);
        assert_eq!(plan.files_total, 1);
        assert_eq!(plan.bytes_total, 5);
        assert_eq!(plan.items[0].destination, destination.join("a.txt"));
    }

    #[test]
    fn plan_dir_has_unknown_size() {
        let dir = tempdir();
        let destination = dir.path().join("dest");
        fs::create_dir(&destination).unwrap();
        let source = dir.path().join("folder");
        fs::create_dir(&source).unwrap();

        let plan = plan_transfer(TransferKind::Move, &[source], &destination).unwrap();
        assert_eq!(plan.files_total, 0);
        assert_eq!(plan.bytes_total, 0);
        assert!(plan.items[0].is_dir);
        assert!(plan.items[0].size.is_none());
    }

    #[test]
    fn refuses_to_copy_a_folder_into_itself() {
        let dir = tempdir();
        let source = dir.path().join("folder");
        fs::create_dir(&source).unwrap();
        let inner = source.join("inner");
        fs::create_dir(&inner).unwrap();

        let error = plan_transfer(TransferKind::Copy, &[source], &inner).unwrap_err();
        assert_eq!(error, OperationError::SourceInsideDestination);
    }

    #[test]
    fn refuses_a_destination_that_is_not_a_folder() {
        let dir = tempdir();
        let file = dir.path().join("target.txt");
        fs::write(&file, b"x").unwrap();
        let source = dir.path().join("source.txt");
        fs::write(&source, b"y").unwrap();

        let error = plan_transfer(TransferKind::Copy, &[source], &file).unwrap_err();
        assert_eq!(error, OperationError::NotADirectory);
    }

    #[test]
    fn refuses_an_empty_selection() {
        let dir = tempdir();
        let error = plan_transfer(TransferKind::Copy, &[], dir.path()).unwrap_err();
        assert_eq!(error, OperationError::InvalidName("no source selected"));
    }

    #[test]
    fn same_folder_paste_is_planned_not_refused() {
        let dir = tempdir();
        let source = dir.path().join("note.txt");
        fs::write(&source, b"content").unwrap();

        let plan = plan_transfer(
            TransferKind::Copy,
            std::slice::from_ref(&source),
            dir.path(),
        )
        .unwrap();
        assert_eq!(plan.items[0].destination, source);
    }

    #[test]
    fn missing_source_is_not_found() {
        let dir = tempdir();
        let missing = dir.path().join("nope.txt");
        let error = plan_transfer(TransferKind::Copy, &[missing], dir.path()).unwrap_err();
        assert_eq!(error, OperationError::NotFound);
    }

    #[test]
    fn unicode_names_survive_planning() {
        let dir = tempdir();
        let destination = dir.path().join("dest");
        fs::create_dir(&destination).unwrap();
        let source: PathBuf = dir.path().join("résumé – 東京.txt");
        fs::write(&source, b"u").unwrap();

        let plan = plan_transfer(TransferKind::Copy, &[source], &destination).unwrap();
        assert_eq!(
            plan.items[0].destination,
            destination.join("résumé – 東京.txt")
        );
    }
}
