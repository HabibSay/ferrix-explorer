//! Conflict-dialog view state: turns an engine `ConflictRequest` into the strings the
//! Slint dialog shows, and maps the user's click back onto an engine answer.

use fx_ops::conflict::{ConflictAnswer, ConflictChoice, ConflictRequest};

/// Pre-formatted strings for the Slint conflict dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictDialogState {
    pub file_name: String,
    pub source_path: String,
    pub source_size: String,
    pub source_modified: String,
    pub destination_path: String,
    pub destination_size: String,
    pub destination_modified: String,
}

/// Maps a dialog choice id onto the engine answer. An unknown id returns `None`,
/// which the dispatcher delivers as a cancel — never a destructive default.
pub fn choice_from(choice: &str, apply_to_all: bool) -> Option<ConflictAnswer> {
    let choice = match choice {
        "replace" => ConflictChoice::Replace,
        "skip" => ConflictChoice::Skip,
        "keep-both" => ConflictChoice::KeepBoth,
        "cancel" => ConflictChoice::Cancel,
        _ => return None,
    };
    Some(if apply_to_all {
        ConflictAnswer::all_in_class(choice)
    } else {
        ConflictAnswer::once(choice)
    })
}

/// Formats the engine request for display. Verbatim `\\?\` prefixes are stripped so
/// they never reach a label, as in Tranche 1.
pub fn dialog_state_for(request: &ConflictRequest) -> ConflictDialogState {
    ConflictDialogState {
        file_name: request
            .source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        source_path: display_path(&request.source),
        source_size: request
            .source_len
            .map(crate::orchestration::format_bytes)
            .unwrap_or_else(|| "-".to_owned()),
        source_modified: format_modified(request.source_modified),
        destination_path: display_path(&request.destination),
        destination_size: request
            .target_len
            .map(crate::orchestration::format_bytes)
            .unwrap_or_else(|| "-".to_owned()),
        destination_modified: format_modified(request.target_modified),
    }
}

fn display_path(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(stripped) => stripped.to_owned(),
        None => s.into_owned(),
    }
}

fn format_modified(modified: Option<std::time::SystemTime>) -> String {
    match modified {
        Some(t) => t
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs().to_string())
            .unwrap_or_else(|_| "-".to_owned()),
        None => "-".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ConflictDialogState, choice_from, dialog_state_for};
    use fx_ops::conflict::{ConflictAnswer, ConflictChoice, ConflictClass, ConflictRequest};
    use std::{
        path::PathBuf,
        time::{Duration, UNIX_EPOCH},
    };

    fn request() -> ConflictRequest {
        ConflictRequest {
            source: PathBuf::from(r"\\?\C:\tmp\a.txt"),
            destination: PathBuf::from(r"C:\dst\a.txt"),
            class: ConflictClass::FileFile,
            source_len: Some(2048),
            target_len: Some(1024),
            source_modified: Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            target_modified: None,
            remaining: 3,
        }
    }

    #[test]
    fn the_four_choices_round_trip() {
        assert_eq!(
            choice_from("replace", false),
            Some(ConflictAnswer::once(ConflictChoice::Replace))
        );
        assert_eq!(
            choice_from("skip", true),
            Some(ConflictAnswer::all_in_class(ConflictChoice::Skip))
        );
        assert_eq!(
            choice_from("keep-both", false),
            Some(ConflictAnswer::once(ConflictChoice::KeepBoth))
        );
        assert_eq!(
            choice_from("cancel", true),
            Some(ConflictAnswer::all_in_class(ConflictChoice::Cancel))
        );
    }

    #[test]
    fn unknown_choice_is_refused() {
        assert_eq!(choice_from("overwrite", false), None);
        assert_eq!(choice_from("", false), None);
    }

    #[test]
    fn state_names_the_file_and_formats_metadata() {
        let s: ConflictDialogState = dialog_state_for(&request());
        assert_eq!(s.file_name, "a.txt");
        assert_eq!(s.source_path, r"C:\tmp\a.txt");
        assert_eq!(s.source_size, "2 KB");
        assert_eq!(s.destination_size, "1 KB");
        assert_eq!(s.source_modified, "1700000000");
        assert_eq!(s.destination_modified, "-");
    }

    #[test]
    fn missing_sizes_show_a_dash() {
        let mut r = request();
        r.source = PathBuf::from(r"C:\tmp\a.txt");
        r.source_len = None;
        let s = dialog_state_for(&r);
        assert_eq!(s.source_size, "-");
    }
}
