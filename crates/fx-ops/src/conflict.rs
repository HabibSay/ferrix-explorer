//! Conflict classification and resolution.
//!
//! "Apply to all" is scoped to a [`ConflictClass`], not to the whole operation: a decision
//! taken on a run of plain files never silently replaces folders or read-only targets.

use std::{path::PathBuf, time::SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConflictClass {
    FileFile,
    DirDir,
    ReadOnly,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictChoice {
    Replace,
    Skip,
    KeepBoth,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConflictAnswer {
    pub choice: ConflictChoice,
    pub apply_to_all_in_class: bool,
}

impl ConflictAnswer {
    pub fn once(choice: ConflictChoice) -> Self {
        Self {
            choice,
            apply_to_all_in_class: false,
        }
    }

    pub fn all_in_class(choice: ConflictChoice) -> Self {
        Self {
            choice,
            apply_to_all_in_class: true,
        }
    }
}

/// Everything the dialog needs to show, including the metadata comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictRequest {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub class: ConflictClass,
    pub source_len: Option<u64>,
    pub target_len: Option<u64>,
    pub source_modified: Option<SystemTime>,
    pub target_modified: Option<SystemTime>,
    /// Items left in the plan, so the dialog can say how much is at stake.
    pub remaining: usize,
}

/// Answers a conflict. `None` cancels the whole operation, which is what closing the dialog
/// means; there is no timeout that silently picks a destructive default.
pub trait ConflictResolver {
    fn resolve(&mut self, request: &ConflictRequest) -> Option<ConflictAnswer>;
}

/// Applies the same answer to every conflict; used by tests and by non-interactive callers.
pub struct FixedResolver {
    pub answer: ConflictAnswer,
}

impl ConflictResolver for FixedResolver {
    fn resolve(&mut self, _request: &ConflictRequest) -> Option<ConflictAnswer> {
        Some(self.answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn once_and_all_differ_only_in_scope() {
        let once = ConflictAnswer::once(ConflictChoice::Skip);
        let all = ConflictAnswer::all_in_class(ConflictChoice::Skip);
        assert!(!once.apply_to_all_in_class);
        assert!(all.apply_to_all_in_class);
        assert_eq!(once.choice, all.choice);
    }

    #[test]
    fn classes_are_distinct_keys() {
        let mut decisions = std::collections::HashMap::new();
        decisions.insert(ConflictClass::FileFile, ConflictChoice::Replace);
        decisions.insert(ConflictClass::ReadOnly, ConflictChoice::Skip);
        assert_eq!(
            decisions.get(&ConflictClass::FileFile),
            Some(&ConflictChoice::Replace)
        );
        assert_eq!(
            decisions.get(&ConflictClass::ReadOnly),
            Some(&ConflictChoice::Skip)
        );
        assert_eq!(decisions.get(&ConflictClass::DirDir), None);
    }
}
