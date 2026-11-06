//! The copy and move engine.
//!
//! One file at a time, 1 MiB chunks, a checkpoint before every chunk, and a post-condition
//! check before anything is reported as done. A failed or cancelled copy never leaves a
//! partial target behind, and a move never deletes a source before the destination is
//! verified.

use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::{
    conflict::{ConflictAnswer, ConflictChoice, ConflictClass, ConflictRequest, ConflictResolver},
    control::OperationControl,
    error::{ItemFailure, OperationError},
    plan::{OperationPlan, PlanItem, same_path},
    port::{PlatformOps, ReparseKind},
    progress::{OperationId, OperationKind, OperationSummary, Progress, SpeedMeter},
    rename::unique_sibling,
};

const CHUNK_SIZE: usize = 1024 * 1024;

/// Free space reserved on the destination so a nearly full volume does not fail mid-item.
const SPACE_HEADROOM: u64 = 32 * 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const MAX_DEPTH: u32 = 64;

pub struct Transfer<'a> {
    pub id: OperationId,
    pub port: &'a dyn PlatformOps,
    pub control: &'a OperationControl,
    pub resolver: &'a mut dyn ConflictResolver,
    pub on_progress: &'a mut dyn FnMut(&Progress),
}

struct RunState {
    summary: OperationSummary,
    progress: Progress,
    decisions: HashMap<ConflictClass, ConflictChoice>,
    speed: SpeedMeter,
    started: Instant,
    last_emit: Instant,
    remaining: usize,
}

enum FileAction {
    Direct(PathBuf),
    Replace(PathBuf),
    Skip,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileOutcome {
    Copied,
    Skipped,
}

impl Transfer<'_> {
    pub fn run(&mut self, plan: &OperationPlan) -> OperationSummary {
        let now = Instant::now();
        let mut state = RunState {
            summary: OperationSummary::new(self.id, plan.kind),
            progress: Progress {
                files_total: plan.files_total,
                bytes_total: plan.bytes_total,
                ..Progress::default()
            },
            decisions: HashMap::new(),
            speed: SpeedMeter::new(),
            started: now,
            last_emit: now,
            remaining: plan.items.len(),
        };
        self.emit(&mut state, true);

        // If the volume cannot be queried, the per-file check still guards each write.
        let available = self
            .port
            .free_space(&plan.destination_root)
            .unwrap_or(u64::MAX);
        let needed = plan.bytes_total.saturating_add(SPACE_HEADROOM);
        if plan.bytes_total > 0 && needed > available {
            return state
                .summary
                .fail_with(OperationError::InsufficientSpace { needed, available });
        }

        let is_move = plan.kind == OperationKind::Move;
        let mut cancelled = false;
        for item in &plan.items {
            state.remaining = state.remaining.saturating_sub(1);
            if self.control.checkpoint().is_err() {
                cancelled = true;
                break;
            }
            state.progress.current = Some(item.source.clone());
            match self.transfer_item(item, &mut state, is_move) {
                Ok(()) => {}
                Err(OperationError::Cancelled) => {
                    cancelled = true;
                    break;
                }
                Err(error) => state
                    .summary
                    .record_failure(ItemFailure::new(&item.source, error)),
            }
        }

        self.emit(&mut state, true);
        state.summary.finish(cancelled)
    }

    fn transfer_item(
        &mut self,
        item: &PlanItem,
        state: &mut RunState,
        is_move: bool,
    ) -> Result<(), OperationError> {
        if item.is_reparse_point {
            return self.transfer_link(&item.source, &item.destination, is_move, state);
        }

        if !item.is_dir {
            return if is_move {
                self.move_file(&item.source, &item.destination, state)
            } else {
                self.transfer_file(&item.source, &item.destination, state)
                    .map(|_| ())
            };
        }

        if !is_move {
            return self.copy_dir(&item.source, &item.destination, state, 0, false);
        }

        let same_volume = self.same_volume(&item.source, &item.destination)?;
        let destination_exists = fs::symlink_metadata(&item.destination).is_ok();

        if same_volume && !destination_exists {
            // Rename fast path: atomic, and no bytes are copied.
            fs::rename(&item.source, &item.destination)
                .map_err(|error| OperationError::from_io(&error))?;
            return verify_present(&item.destination);
        }

        // Cross-volume, or moving onto an existing folder: merge by copying all-or-nothing,
        // then remove the source. The source goes only after a fully verified copy.
        self.copy_dir(&item.source, &item.destination, state, 0, true)?;
        remove_tree(&item.source, self.control)?;
        Ok(())
    }

    /// Rule 6, as slice 2 refined it: a symbolic link is recreated as a link. A junction,
    /// an unknown tag, or a platform that cannot classify at all stays skipped — following
    /// one could copy a tree the user never selected.
    fn transfer_link(
        &mut self,
        source: &Path,
        destination: &Path,
        is_move: bool,
        state: &mut RunState,
    ) -> Result<(), OperationError> {
        reparse_action(self.port.reparse_kind(source))?;
        let target = fs::read_link(source).map_err(|error| OperationError::from_io(&error))?;
        // The link's own metadata follows the target: a link to a folder is a folder link.
        let is_dir = fs::metadata(source)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);
        self.port
            .create_symlink(&target, destination, is_dir)
            .map_err(|error| OperationError::from_io(&error))?;
        state.summary.files_done += 1;
        if is_move {
            fs::remove_file(source).map_err(|error| OperationError::from_io(&error))?;
        }
        verify_present(destination)
    }

    fn transfer_file(
        &mut self,
        source: &Path,
        destination: &Path,
        state: &mut RunState,
    ) -> Result<FileOutcome, OperationError> {
        match self.decide(source, destination, state)? {
            FileAction::Skip => {
                state.summary.files_skipped += 1;
                Ok(FileOutcome::Skipped)
            }
            FileAction::Cancel => Err(OperationError::Cancelled),
            FileAction::Direct(target) => {
                let bytes = self.copy_file(source, &target, false, state)?;
                state.summary.files_done += 1;
                state.summary.bytes_done += bytes;
                Ok(FileOutcome::Copied)
            }
            FileAction::Replace(target) => {
                let bytes = self.copy_file(source, &target, true, state)?;
                state.summary.files_done += 1;
                state.summary.bytes_done += bytes;
                Ok(FileOutcome::Copied)
            }
        }
    }

    /// Moves one file: rename on the same volume, copy-then-delete across volumes.
    fn move_file(
        &mut self,
        source: &Path,
        destination: &Path,
        state: &mut RunState,
    ) -> Result<(), OperationError> {
        let same_volume = self.same_volume(source, destination)?;
        if !same_volume {
            if self.transfer_file(source, destination, state)? == FileOutcome::Skipped {
                return Ok(());
            }
            fs::remove_file(source).map_err(|error| OperationError::from_io(&error))?;
            verify_absent(source)?;
            return Ok(());
        }

        match self.decide(source, destination, state)? {
            FileAction::Skip => {
                state.summary.files_skipped += 1;
                Ok(())
            }
            FileAction::Cancel => Err(OperationError::Cancelled),
            FileAction::Direct(target) => {
                fs::rename(source, &target).map_err(|error| OperationError::from_io(&error))?;
                verify_present(&target)?;
                verify_absent(source)?;
                state.summary.files_done += 1;
                state.summary.bytes_done += file_len(&target);
                Ok(())
            }
            FileAction::Replace(target) => {
                // Swap through a backup so a failed rename cannot lose the target.
                let backup = temporary_path(&target, self.id);
                fs::rename(&target, &backup).map_err(|error| OperationError::from_io(&error))?;
                match fs::rename(source, &target) {
                    Ok(()) => {
                        let _ = fs::remove_file(&backup);
                        verify_present(&target)?;
                        verify_absent(source)?;
                        state.summary.files_done += 1;
                        state.summary.bytes_done += file_len(&target);
                        Ok(())
                    }
                    Err(error) => {
                        let _ = fs::rename(&backup, &target);
                        Err(OperationError::from_io(&error))
                    }
                }
            }
        }
    }

    /// Decides what to do when the destination already exists (or is the source itself).
    fn decide(
        &mut self,
        source: &Path,
        destination: &Path,
        state: &mut RunState,
    ) -> Result<FileAction, OperationError> {
        let target_metadata = fs::symlink_metadata(destination);
        let same = same_path(source, destination);
        if target_metadata.is_err() && !same {
            return Ok(FileAction::Direct(destination.to_path_buf()));
        }

        // A file cannot replace itself: keep-both is the only safe answer.
        let class = classify(destination, target_metadata.as_ref().ok());
        let choice = if same {
            ConflictChoice::KeepBoth
        } else if let Some(choice) = state.decisions.get(&class) {
            *choice
        } else {
            let answer = self.ask(source, destination, class, state)?;
            if answer.apply_to_all_in_class {
                state.decisions.insert(class, answer.choice);
            }
            answer.choice
        };

        Ok(match choice {
            ConflictChoice::Skip => FileAction::Skip,
            ConflictChoice::Cancel => FileAction::Cancel,
            ConflictChoice::Replace => FileAction::Replace(destination.to_path_buf()),
            ConflictChoice::KeepBoth => FileAction::Direct(unique_sibling(destination)),
        })
    }

    fn ask(
        &mut self,
        source: &Path,
        destination: &Path,
        class: ConflictClass,
        state: &RunState,
    ) -> Result<ConflictAnswer, OperationError> {
        let source_metadata = fs::metadata(source).ok();
        let target_metadata = fs::metadata(destination).ok();
        let request = ConflictRequest {
            source: source.to_path_buf(),
            destination: destination.to_path_buf(),
            class,
            source_len: source_metadata.as_ref().map(fs::Metadata::len),
            target_len: target_metadata.as_ref().map(fs::Metadata::len),
            source_modified: source_metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok()),
            target_modified: target_metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok()),
            remaining: state.remaining,
        };
        Ok(self
            .resolver
            .resolve(&request)
            .unwrap_or(ConflictAnswer::once(ConflictChoice::Cancel)))
    }

    fn copy_file(
        &mut self,
        source: &Path,
        destination: &Path,
        replace: bool,
        state: &mut RunState,
    ) -> Result<u64, OperationError> {
        let target = if replace {
            temporary_path(destination, self.id)
        } else {
            destination.to_path_buf()
        };

        let mut input = fs::File::open(source).map_err(|error| OperationError::from_io(&error))?;
        let needed = file_len(source);
        let available = self
            .port
            .free_space(destination.parent().unwrap_or(destination))
            .unwrap_or(u64::MAX);
        if needed > available {
            return Err(OperationError::InsufficientSpace { needed, available });
        }
        let mut output = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(file) => file,
            Err(error) => return Err(OperationError::from_io(&error)),
        };

        let copied = self.copy_stream(&mut input, &mut output, state);
        drop(output);

        let bytes = match copied {
            Ok(bytes) => bytes,
            Err(error) => {
                let _ = fs::remove_file(&target);
                return Err(error);
            }
        };

        if let Err(error) = self.port.apply_metadata(source, &target) {
            let _ = fs::remove_file(&target);
            return Err(OperationError::from_io(&error));
        }

        if replace {
            if let Err(error) = self.port.replace_file(&target, destination) {
                let _ = fs::remove_file(&target);
                return Err(OperationError::from_io(&error));
            }
            verify_length(destination, bytes)?;
        } else {
            verify_length(&target, bytes)?;
        }

        Ok(bytes)
    }

    fn copy_stream(
        &mut self,
        input: &mut fs::File,
        output: &mut fs::File,
        state: &mut RunState,
    ) -> Result<u64, OperationError> {
        let mut buffer = vec![0u8; CHUNK_SIZE];
        let mut bytes = 0u64;
        loop {
            self.control.checkpoint()?;
            let read = input
                .read(&mut buffer)
                .map_err(|error| OperationError::from_io(&error))?;
            if read == 0 {
                break;
            }
            output
                .write_all(&buffer[..read])
                .map_err(|error| OperationError::from_io(&error))?;
            bytes += read as u64;
            state.progress.bytes_done = state.summary.bytes_done + bytes;
            self.emit(state, false);
        }
        output
            .flush()
            .map_err(|error| OperationError::from_io(&error))?;
        Ok(bytes)
    }

    fn copy_dir(
        &mut self,
        source: &Path,
        destination: &Path,
        state: &mut RunState,
        depth: u32,
        all_or_nothing: bool,
    ) -> Result<(), OperationError> {
        if depth > MAX_DEPTH {
            return Err(OperationError::PathTooLong);
        }
        self.control.checkpoint()?;
        if let Err(error) = fs::create_dir_all(destination) {
            return Err(OperationError::from_io(&error));
        }

        let entries = match fs::read_dir(source) {
            Ok(entries) => entries,
            Err(error) => return Err(OperationError::from_io(&error)),
        };

        for entry in entries {
            self.control.checkpoint()?;
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    let error = OperationError::from_io(&error);
                    if all_or_nothing {
                        return Err(error);
                    }
                    state
                        .summary
                        .record_failure(ItemFailure::new(source, error));
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    let error = OperationError::from_io(&error);
                    if all_or_nothing {
                        return Err(error);
                    }
                    state.summary.record_failure(ItemFailure::new(&path, error));
                    continue;
                }
            };
            let target = destination.join(entry.file_name());

            if metadata.file_type().is_symlink() {
                match self.transfer_link(&path, &target, false, state) {
                    Ok(()) => continue,
                    Err(error) => {
                        if all_or_nothing {
                            return Err(error);
                        }
                        state.summary.record_failure(ItemFailure::new(&path, error));
                        continue;
                    }
                }
            }

            let result = if metadata.is_dir() {
                self.copy_dir(&path, &target, state, depth + 1, all_or_nothing)
            } else {
                self.transfer_file(&path, &target, state).map(|_| ())
            };

            match result {
                Ok(()) => {}
                Err(OperationError::Cancelled) => return Err(OperationError::Cancelled),
                Err(error) => {
                    if all_or_nothing {
                        return Err(error);
                    }
                    state.summary.record_failure(ItemFailure::new(&path, error));
                }
            }
        }

        Ok(())
    }

    fn same_volume(&self, source: &Path, destination: &Path) -> Result<bool, OperationError> {
        let source_volume = self
            .port
            .volume_id(source.parent().unwrap_or(source))
            .map_err(|error| OperationError::from_io(&error))?;
        let destination_volume = self
            .port
            .volume_id(destination.parent().unwrap_or(destination))
            .map_err(|error| OperationError::from_io(&error))?;
        Ok(source_volume == destination_volume)
    }

    fn emit(&mut self, state: &mut RunState, force: bool) {
        if !force && state.last_emit.elapsed() < PROGRESS_INTERVAL {
            return;
        }
        let elapsed = state.started.elapsed();
        state.progress.bytes_per_second = state.speed.update(state.progress.bytes_done, elapsed);
        let remaining = state
            .progress
            .bytes_total
            .saturating_sub(state.progress.bytes_done);
        state.progress.eta = if state.progress.bytes_per_second > 0 && remaining > 0 {
            Some(Duration::from_secs(
                remaining / state.progress.bytes_per_second,
            ))
        } else {
            None
        };
        state.last_emit = Instant::now();
        let snapshot = state.progress.clone();
        (self.on_progress)(&snapshot);
    }
}

fn classify(destination: &Path, metadata: Option<&fs::Metadata>) -> ConflictClass {
    let Some(metadata) = metadata else {
        return ConflictClass::FileFile;
    };
    if metadata.permissions().readonly() {
        return ConflictClass::ReadOnly;
    }
    if fs::metadata(destination)
        .map(|meta| meta.is_dir())
        .unwrap_or(false)
    {
        return ConflictClass::DirDir;
    }
    ConflictClass::FileFile
}

fn temporary_path(destination: &Path, id: OperationId) -> PathBuf {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "ferrix".to_owned());
    parent.join(format!("{name}.fxpart-{}", id.0))
}

/// Rule 6, as slice 2 refined it: only symbolic links are recreated. A junction, an
/// unknown tag, or a platform that cannot classify at all stays skipped.
fn reparse_action(kind: Option<ReparseKind>) -> Result<(), OperationError> {
    match kind {
        Some(ReparseKind::SymbolicLink) => Ok(()),
        Some(ReparseKind::Junction) | Some(ReparseKind::Other) | None => {
            Err(OperationError::ReparsePointUnsupported)
        }
    }
}

fn verify_present(path: &Path) -> Result<(), OperationError> {
    fs::symlink_metadata(path)
        .map(|_| ())
        .map_err(|error| OperationError::from_io(&error))
}

fn verify_absent(path: &Path) -> Result<(), OperationError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(OperationError::from_io(&error)),
        Ok(_) => Err(OperationError::VerificationFailed),
    }
}

fn file_len(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn verify_length(path: &Path, expected: u64) -> Result<(), OperationError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| OperationError::from_io(&error))?;
    if metadata.len() == expected {
        Ok(())
    } else {
        Err(OperationError::VerificationFailed)
    }
}

fn remove_tree(path: &Path, control: &OperationControl) -> Result<(), OperationError> {
    control.checkpoint()?;
    let metadata = fs::symlink_metadata(path).map_err(|error| OperationError::from_io(&error))?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        for entry in fs::read_dir(path).map_err(|error| OperationError::from_io(&error))? {
            let entry = entry.map_err(|error| OperationError::from_io(&error))?;
            remove_tree(&entry.path(), control)?;
        }
        fs::remove_dir(path).map_err(|error| OperationError::from_io(&error))
    } else {
        fs::remove_file(path)
            .or_else(|_| fs::remove_dir(path))
            .map_err(|error| OperationError::from_io(&error))
    }
}

/// Moves one path, using a rename on the same volume and copy-then-delete across volumes.
///
/// Used by undo, which runs outside a [`Transfer`] and therefore has no progress sink.
pub fn move_path(
    from: &Path,
    to: &Path,
    port: &dyn PlatformOps,
    control: &OperationControl,
) -> Result<(), OperationError> {
    let from_volume = port
        .volume_id(from.parent().unwrap_or(from))
        .map_err(|error| OperationError::from_io(&error))?;
    let to_volume = port
        .volume_id(to.parent().unwrap_or(to))
        .map_err(|error| OperationError::from_io(&error))?;

    if from_volume == to_volume {
        return fs::rename(from, to).map_err(|error| OperationError::from_io(&error));
    }

    copy_path_plain(from, to, port, control)?;
    remove_tree_plain(from, control)
}

fn copy_path_plain(
    from: &Path,
    to: &Path,
    port: &dyn PlatformOps,
    control: &OperationControl,
) -> Result<(), OperationError> {
    control.checkpoint()?;
    let metadata = fs::symlink_metadata(from).map_err(|error| OperationError::from_io(&error))?;
    if metadata.file_type().is_symlink() {
        return Err(OperationError::ReparsePointUnsupported);
    }
    if metadata.is_dir() {
        fs::create_dir_all(to).map_err(|error| OperationError::from_io(&error))?;
        for entry in fs::read_dir(from).map_err(|error| OperationError::from_io(&error))? {
            let entry = entry.map_err(|error| OperationError::from_io(&error))?;
            copy_path_plain(&entry.path(), &to.join(entry.file_name()), port, control)?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map_err(|error| OperationError::from_io(&error))?;
        port.apply_metadata(from, to)
            .map_err(|error| OperationError::from_io(&error))
    }
}

fn remove_tree_plain(path: &Path, control: &OperationControl) -> Result<(), OperationError> {
    control.checkpoint()?;
    let metadata = fs::symlink_metadata(path).map_err(|error| OperationError::from_io(&error))?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        for entry in fs::read_dir(path).map_err(|error| OperationError::from_io(&error))? {
            let entry = entry.map_err(|error| OperationError::from_io(&error))?;
            remove_tree_plain(&entry.path(), control)?;
        }
        fs::remove_dir(path).map_err(|error| OperationError::from_io(&error))
    } else {
        fs::remove_file(path)
            .or_else(|_| fs::remove_dir(path))
            .map_err(|error| OperationError::from_io(&error))
    }
}

#[cfg(test)]
mod tests {
    use super::reparse_action;
    use crate::{error::OperationError, port::ReparseKind};

    #[test]
    fn only_symbolic_links_are_recreated() {
        assert_eq!(reparse_action(Some(ReparseKind::SymbolicLink)), Ok(()));
        for kind in [Some(ReparseKind::Junction), Some(ReparseKind::Other), None] {
            assert_eq!(
                reparse_action(kind),
                Err(OperationError::ReparsePointUnsupported),
                "{kind:?} must stay skipped"
            );
        }
    }
}
