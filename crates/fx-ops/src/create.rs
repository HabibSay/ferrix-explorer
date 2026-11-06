//! Creating folders and empty files with Explorer's unique naming.
//!
//! The base name ("New folder", "New text document") is supplied by the UI from the string
//! resources; this module only numbers it and refuses invalid names.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::{
    error::OperationError,
    rename::{numbered_name, validate_name},
};

const MAX_ATTEMPTS: u32 = 10_000;

pub fn create_directory(parent: &Path, base: &str) -> Result<PathBuf, OperationError> {
    validate_name(base)?;
    for attempt in 0..MAX_ATTEMPTS {
        let candidate = parent.join(candidate_name(base, attempt));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(OperationError::from_io(&error)),
        }
    }
    Err(OperationError::AlreadyExists)
}

pub fn create_file(parent: &Path, base: &str) -> Result<PathBuf, OperationError> {
    validate_name(base)?;
    for attempt in 0..MAX_ATTEMPTS {
        let candidate = parent.join(candidate_name(base, attempt));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(OperationError::from_io(&error)),
        }
    }
    Err(OperationError::AlreadyExists)
}

fn candidate_name(base: &str, attempt: u32) -> String {
    if attempt == 0 {
        base.to_owned()
    } else {
        numbered_name(base, attempt + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_numbered_folders() {
        let dir = tempfile::tempdir().unwrap();
        let first = create_directory(dir.path(), "New folder").unwrap();
        let second = create_directory(dir.path(), "New folder").unwrap();
        assert_eq!(first, dir.path().join("New folder"));
        assert_eq!(second, dir.path().join("New folder (2)"));
        assert!(first.is_dir());
        assert!(second.is_dir());
    }

    #[test]
    fn creates_numbered_files_without_clobbering() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("New text document.txt"), b"keep").unwrap();

        let created = create_file(dir.path(), "New text document.txt").unwrap();
        assert_eq!(created, dir.path().join("New text document (2).txt"));
        assert_eq!(
            fs::read(dir.path().join("New text document.txt")).unwrap(),
            b"keep"
        );
        assert_eq!(fs::metadata(&created).unwrap().len(), 0);
    }

    #[test]
    fn refuses_an_invalid_base_name() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            create_directory(dir.path(), "CON"),
            Err(OperationError::InvalidName("reserved-device"))
        );
        assert_eq!(
            create_file(dir.path(), "a:b"),
            Err(OperationError::InvalidName("invalid-character"))
        );
    }

    #[test]
    fn missing_parent_is_reported_as_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone");
        assert_eq!(
            create_directory(&missing, "New folder"),
            Err(OperationError::NotFound)
        );
    }
}
