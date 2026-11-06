//! Helpers for Win32 verbatim (long-path) prefixes.
//!
//! On Windows, paths longer than MAX_PATH (260 chars) require a `\\?\` prefix.
//! These helpers make it easy to strip or add that prefix as needed.

use std::path::{Path, PathBuf};

/// The verbatim prefix added by Windows extended-length path APIs.
const VERBATIM_PREFIX: &str = r"\\?\";

/// Strips the `\\?\` verbatim prefix from a path string if present.
///
/// Returns the original string unchanged when the prefix is absent.
pub fn strip_verbatim_prefix(s: &str) -> &str {
    s.strip_prefix(VERBATIM_PREFIX).unwrap_or(s)
}

/// Returns `true` if the path string is long enough to require a `\\?\` prefix
/// on Windows (more than 260 characters).
pub fn needs_verbatim_prefix(path: &Path) -> bool {
    path.as_os_str().len() > 260
}

/// Returns the path with the `\\?\` prefix added if `needs_verbatim_prefix` is true.
///
/// If the path already starts with `\\?\` it is returned unchanged.
pub fn with_verbatim_prefix_if_needed(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s.starts_with(VERBATIM_PREFIX) {
        return path.to_path_buf();
    }
    if needs_verbatim_prefix(path) {
        PathBuf::from(format!("{VERBATIM_PREFIX}{s}"))
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_prefix_present() {
        assert_eq!(strip_verbatim_prefix(r"\\?\C:\long\path"), r"C:\long\path");
    }

    #[test]
    fn strip_prefix_absent() {
        assert_eq!(strip_verbatim_prefix(r"C:\short"), r"C:\short");
    }

    #[test]
    fn short_path_no_prefix_needed() {
        let p = Path::new(r"C:\Users\alice\file.txt");
        assert!(!needs_verbatim_prefix(p));
    }

    #[test]
    fn long_path_prefix_needed() {
        // Build a path > 260 chars.
        let long: String = format!("C:\\{}", "a".repeat(260));
        let p = PathBuf::from(&long);
        assert!(needs_verbatim_prefix(&p));
    }

    #[test]
    fn with_prefix_adds_when_needed() {
        let long: String = format!("C:\\{}", "a".repeat(260));
        let p = PathBuf::from(&long);
        let result = with_verbatim_prefix_if_needed(&p);
        assert!(result.to_string_lossy().starts_with(r"\\?\"));
    }

    #[test]
    fn with_prefix_no_op_when_short() {
        let p = PathBuf::from(r"C:\short");
        let result = with_verbatim_prefix_if_needed(&p);
        assert_eq!(result, p);
    }

    #[test]
    fn with_prefix_no_op_when_already_prefixed() {
        let p = PathBuf::from(r"\\?\C:\already");
        let result = with_verbatim_prefix_if_needed(&p);
        assert_eq!(result, p);
    }
}
