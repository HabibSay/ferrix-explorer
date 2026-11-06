//! Accessibility contracts for Ferrix Explorer.
//!
//! The Tranche 0 HWND uses the Windows default UI Automation provider: its window caption is
//! exposed as the element name and the HWND is exposed as a Window control. Custom UI controls
//! must add providers/semantics as they are introduced. Validate this contract with Narrator and
//! Accessibility Insights on Windows; it cannot be verified on non-Windows hosts.

/// Accessible identity for the top-level Ferrix window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootAccessibility {
    /// Name exposed from the native window caption.
    pub name: &'static str,
    /// Native UI Automation control type of an HWND top-level window.
    pub role: &'static str,
}

/// Build the Tranche 0 top-level accessibility contract from the actual window caption.
///
/// Callers pass the same string that is handed to `CreateWindowExW`, so the accessible name cannot
/// drift from the caption. The `Window` role is the UI Automation control type that Windows reports
/// for a top-level HWND served by the default provider.
pub const fn root_accessibility(window_title: &'static str) -> RootAccessibility {
    RootAccessibility {
        name: window_title,
        role: "Window",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_name_is_taken_from_the_window_caption() {
        let root = root_accessibility("Example title");
        assert_eq!(root.name, "Example title");
        assert_eq!(root.role, "Window");
        assert!(!root.name.is_empty());
    }
}
