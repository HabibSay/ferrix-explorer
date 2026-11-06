//! The undo journal.
//!
//! Only reversible actions are recorded: a copy is not one, because "undo copy" would delete
//! data the user may have edited. Undo exposes the top entry only and refuses when its
//! preconditions no longer hold, rather than reaching further back and surprising the user.
//! Every inversion re-verifies before acting.

use std::{fs, path::PathBuf};

use crate::{
    control::OperationControl,
    error::OperationError,
    plan::PathPair,
    port::{PlatformOps, RecycledItem},
    progress::{OperationId, OperationKind},
    transfer::move_path,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReversibleAction {
    Rename { pairs: Vec<PathPair> },
    Move { pairs: Vec<PathPair> },
    CreateDirectory { paths: Vec<PathBuf> },
    CreateFile { paths: Vec<PathBuf> },
    Recycle { items: Vec<RecycledItem> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoEntry {
    pub operation: OperationId,
    pub kind: OperationKind,
    pub action: ReversibleAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoError {
    NothingToUndo,
    NothingToRedo,
    /// The action can no longer be inverted safely; the reason is machine-readable.
    Refused(&'static str),
    Failed(OperationError),
}

#[derive(Debug, Default)]
pub struct UndoStack {
    entries: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a completed reversible action. Any redo history is invalidated.
    pub fn record(&mut self, entry: UndoEntry) {
        self.entries.push(entry);
        self.redo.clear();
    }

    pub fn can_undo(&self) -> Option<&UndoEntry> {
        self.entries.last()
    }

    pub fn can_redo(&self) -> Option<&UndoEntry> {
        self.redo.last()
    }

    pub fn undo_len(&self) -> usize {
        self.entries.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    pub fn undo(
        &mut self,
        port: &dyn PlatformOps,
        control: &OperationControl,
    ) -> Result<UndoEntry, UndoError> {
        let entry = self.entries.pop().ok_or(UndoError::NothingToUndo)?;
        match invert(&entry, port, control) {
            Ok(()) => {
                self.redo.push(entry.clone());
                Ok(entry)
            }
            Err(error) => {
                self.entries.push(entry);
                Err(error)
            }
        }
    }

    pub fn redo(
        &mut self,
        port: &dyn PlatformOps,
        control: &OperationControl,
    ) -> Result<UndoEntry, UndoError> {
        let entry = self.redo.pop().ok_or(UndoError::NothingToRedo)?;
        match reapply(&entry, port, control) {
            Ok(()) => {
                self.entries.push(entry.clone());
                Ok(entry)
            }
            Err(error) => {
                self.redo.push(entry);
                Err(error)
            }
        }
    }
}

fn invert(
    entry: &UndoEntry,
    port: &dyn PlatformOps,
    control: &OperationControl,
) -> Result<(), UndoError> {
    match &entry.action {
        ReversibleAction::Rename { pairs } | ReversibleAction::Move { pairs } => {
            for pair in pairs {
                if !exists(&pair.to) {
                    return Err(UndoError::Refused("the renamed item is gone"));
                }
                if exists(&pair.from) {
                    return Err(UndoError::Refused("the original path is occupied"));
                }
            }
            for pair in pairs.iter().rev() {
                move_path(&pair.to, &pair.from, port, control).map_err(UndoError::Failed)?;
            }
            Ok(())
        }
        ReversibleAction::CreateDirectory { paths } => {
            for path in paths {
                let metadata = fs::symlink_metadata(path).map_err(|error| {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        UndoError::Refused("the folder no longer exists")
                    } else {
                        UndoError::Failed(OperationError::from_io(&error))
                    }
                })?;
                if !metadata.is_dir() {
                    return Err(UndoError::Refused("the path is no longer a folder"));
                }
                let mut entries = fs::read_dir(path)
                    .map_err(|error| UndoError::Failed(OperationError::from_io(&error)))?;
                if entries.next().is_some() {
                    return Err(UndoError::Refused("the folder is not empty"));
                }
            }
            for path in paths {
                fs::remove_dir(path)
                    .map_err(|error| UndoError::Failed(OperationError::from_io(&error)))?;
            }
            Ok(())
        }
        ReversibleAction::CreateFile { paths } => {
            for path in paths {
                let metadata = fs::symlink_metadata(path).map_err(|error| {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        UndoError::Refused("the file no longer exists")
                    } else {
                        UndoError::Failed(OperationError::from_io(&error))
                    }
                })?;
                if !metadata.is_file() {
                    return Err(UndoError::Refused("the path is no longer a file"));
                }
                if metadata.len() != 0 {
                    return Err(UndoError::Refused("the file is no longer empty"));
                }
            }
            for path in paths {
                fs::remove_file(path)
                    .map_err(|error| UndoError::Failed(OperationError::from_io(&error)))?;
            }
            Ok(())
        }
        ReversibleAction::Recycle { items } => {
            if items.iter().any(|item| !item.verified) {
                return Err(UndoError::Refused(
                    "the Recycle Bin entry cannot be identified",
                ));
            }
            for item in items {
                port.restore_recycled(item)
                    .map_err(|_| UndoError::Failed(OperationError::RestoreUnavailable))?;
                if !exists(&item.original) {
                    return Err(UndoError::Failed(OperationError::VerificationFailed));
                }
            }
            Ok(())
        }
    }
}

fn reapply(
    entry: &UndoEntry,
    port: &dyn PlatformOps,
    control: &OperationControl,
) -> Result<(), UndoError> {
    match &entry.action {
        ReversibleAction::Rename { pairs } | ReversibleAction::Move { pairs } => {
            for pair in pairs {
                if !exists(&pair.from) {
                    return Err(UndoError::Refused("the original item is gone"));
                }
                if exists(&pair.to) {
                    return Err(UndoError::Refused("the target path is occupied"));
                }
            }
            for pair in pairs {
                move_path(&pair.from, &pair.to, port, control).map_err(UndoError::Failed)?;
            }
            Ok(())
        }
        ReversibleAction::CreateDirectory { paths } => {
            for path in paths {
                if exists(path) {
                    return Err(UndoError::Refused("the folder already exists"));
                }
            }
            for path in paths {
                fs::create_dir(path)
                    .map_err(|error| UndoError::Failed(OperationError::from_io(&error)))?;
            }
            Ok(())
        }
        ReversibleAction::CreateFile { paths } => {
            for path in paths {
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .map_err(|error| UndoError::Failed(OperationError::from_io(&error)))?;
            }
            Ok(())
        }
        ReversibleAction::Recycle { .. } => Err(UndoError::Refused(
            "redo of a delete is not offered; delete again explicitly",
        )),
    }
}

fn exists(path: &std::path::Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::{RecycleError, RecyclePrecheck, RestoreError, VolumeKind};

    struct TestPlatform {
        recreate_on_restore: bool,
    }

    impl PlatformOps for TestPlatform {
        fn volume_id(&self, _path: &std::path::Path) -> std::io::Result<u64> {
            Ok(0)
        }
        fn volume_kind(&self, _path: &std::path::Path) -> std::io::Result<VolumeKind> {
            Ok(VolumeKind::Fixed)
        }
        fn free_space(&self, _path: &std::path::Path) -> std::io::Result<u64> {
            Ok(u64::MAX)
        }
        fn replace_file(
            &self,
            replacement: &std::path::Path,
            destination: &std::path::Path,
        ) -> std::io::Result<()> {
            fs::rename(replacement, destination)
        }
        fn can_recycle(&self, _paths: &[PathBuf]) -> Result<RecyclePrecheck, RecycleError> {
            Ok(RecyclePrecheck::Possible)
        }
        fn recycle(&self, _paths: &[PathBuf]) -> Result<Vec<RecycledItem>, RecycleError> {
            Err(RecycleError {
                code: 0,
                message: "not used".to_owned(),
            })
        }
        fn restore_recycled(&self, item: &RecycledItem) -> Result<(), RestoreError> {
            if self.recreate_on_restore {
                fs::write(&item.original, b"restored").map_err(|error| RestoreError {
                    message: error.to_string(),
                })?;
                Ok(())
            } else {
                Err(RestoreError {
                    message: "gone".to_owned(),
                })
            }
        }
    }

    fn rename_entry(dir: &std::path::Path) -> UndoEntry {
        UndoEntry {
            operation: OperationId::next(),
            kind: OperationKind::Rename,
            action: ReversibleAction::Rename {
                pairs: vec![PathPair {
                    from: dir.join("old.txt"),
                    to: dir.join("new.txt"),
                }],
            },
        }
    }

    #[test]
    fn undo_renames_back_and_redo_reapplies() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.txt");
        let new = dir.path().join("new.txt");
        fs::write(&old, b"x").unwrap();
        fs::rename(&old, &new).unwrap();

        let mut stack = UndoStack::new();
        stack.record(rename_entry(dir.path()));

        let platform = TestPlatform {
            recreate_on_restore: false,
        };
        stack.undo(&platform, &OperationControl::new()).unwrap();
        assert!(old.exists());
        assert!(!new.exists());

        stack.redo(&platform, &OperationControl::new()).unwrap();
        assert!(new.exists());
        assert!(!old.exists());
    }

    #[test]
    fn undo_refuses_when_the_renamed_item_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("old.txt"), b"x").unwrap();
        // The entry claims a rename that never happened: `to` is missing.
        let mut stack = UndoStack::new();
        stack.record(rename_entry(dir.path()));

        let platform = TestPlatform {
            recreate_on_restore: false,
        };
        assert_eq!(
            stack.undo(&platform, &OperationControl::new()),
            Err(UndoError::Refused("the renamed item is gone"))
        );
        assert_eq!(stack.undo_len(), 1, "a refused undo keeps the entry");
    }

    #[test]
    fn undo_of_a_creation_never_deletes_data() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("New folder");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("kept.txt"), b"x").unwrap();

        let mut stack = UndoStack::new();
        stack.record(UndoEntry {
            operation: OperationId::next(),
            kind: OperationKind::CreateDirectory,
            action: ReversibleAction::CreateDirectory {
                paths: vec![folder.clone()],
            },
        });

        let platform = TestPlatform {
            recreate_on_restore: false,
        };
        assert_eq!(
            stack.undo(&platform, &OperationControl::new()),
            Err(UndoError::Refused("the folder is not empty"))
        );
        assert!(folder.exists());
        assert!(folder.join("kept.txt").exists());
    }

    #[test]
    fn empty_creation_can_be_undone() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("New folder");
        fs::create_dir(&folder).unwrap();

        let mut stack = UndoStack::new();
        stack.record(UndoEntry {
            operation: OperationId::next(),
            kind: OperationKind::CreateDirectory,
            action: ReversibleAction::CreateDirectory {
                paths: vec![folder.clone()],
            },
        });

        let platform = TestPlatform {
            recreate_on_restore: false,
        };
        stack.undo(&platform, &OperationControl::new()).unwrap();
        assert!(!folder.exists());
    }

    #[test]
    fn a_created_file_that_was_written_to_is_not_removed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("New text document.txt");
        fs::write(&file, b"typed").unwrap();

        let mut stack = UndoStack::new();
        stack.record(UndoEntry {
            operation: OperationId::next(),
            kind: OperationKind::CreateFile,
            action: ReversibleAction::CreateFile {
                paths: vec![file.clone()],
            },
        });

        let platform = TestPlatform {
            recreate_on_restore: false,
        };
        assert_eq!(
            stack.undo(&platform, &OperationControl::new()),
            Err(UndoError::Refused("the file is no longer empty"))
        );
        assert_eq!(fs::read(&file).unwrap(), b"typed");
    }

    #[test]
    fn undo_of_a_delete_requires_a_verified_recycle_entry() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("gone.txt");

        let mut stack = UndoStack::new();
        stack.record(UndoEntry {
            operation: OperationId::next(),
            kind: OperationKind::Recycle,
            action: ReversibleAction::Recycle {
                items: vec![RecycledItem {
                    original: original.clone(),
                    recycled_name: None,
                    verified: false,
                }],
            },
        });

        let platform = TestPlatform {
            recreate_on_restore: true,
        };
        assert_eq!(
            stack.undo(&platform, &OperationControl::new()),
            Err(UndoError::Refused(
                "the Recycle Bin entry cannot be identified"
            ))
        );

        stack.record(UndoEntry {
            operation: OperationId::next(),
            kind: OperationKind::Recycle,
            action: ReversibleAction::Recycle {
                items: vec![RecycledItem {
                    original: original.clone(),
                    recycled_name: Some("$R1".to_owned()),
                    verified: true,
                }],
            },
        });
        stack.undo(&platform, &OperationControl::new()).unwrap();
        assert!(original.exists());
    }

    #[test]
    fn nothing_to_undo_is_reported() {
        let mut stack = UndoStack::new();
        let platform = TestPlatform {
            recreate_on_restore: false,
        };
        assert_eq!(
            stack.undo(&platform, &OperationControl::new()),
            Err(UndoError::NothingToUndo)
        );
        assert_eq!(
            stack.redo(&platform, &OperationControl::new()),
            Err(UndoError::NothingToRedo)
        );
    }

    #[test]
    fn a_new_record_clears_redo() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("old.txt"), b"x").unwrap();
        fs::rename(dir.path().join("old.txt"), dir.path().join("new.txt")).unwrap();

        let mut stack = UndoStack::new();
        stack.record(rename_entry(dir.path()));
        let platform = TestPlatform {
            recreate_on_restore: false,
        };
        stack.undo(&platform, &OperationControl::new()).unwrap();
        assert_eq!(stack.redo_len(), 1);

        stack.record(rename_entry(dir.path()));
        assert_eq!(stack.redo_len(), 0);
    }
}
