//! English (en-US) UI strings for the native host and the app-local Shell menu.
//!
//! Shared Slint strings live in `resources/en-US.slint`. Add new locales as sibling files and
//! keep identifiers stable; no user-visible text should be hard-coded in UI code.

/// Caption of the top-level window. Also used as the window's UI Automation name.
pub const APP_TITLE: &str = "Ferrix Explorer — Tranche 0";

/// Label of the app-local `IContextMenu` command.
pub const ABOUT_MENU_LABEL: &str = "About Ferrix";

/// Prefix for the fatal logging-setup message written to stderr.
pub const LOGGING_UNAVAILABLE: &str = "Ferrix Explorer could not initialize file logging: ";

// ── Tranche 1 — navigation UI ─────────────────────────────────────────────────

/// Accessible label for the address bar input.
pub const ADDRESS_BAR_LABEL: &str = "Address";

/// Accessible label for the breadcrumb navigation region.
pub const BREADCRUMB_LABEL: &str = "Navigation path";

/// Accessible label for the file list.
pub const FILE_LIST_LABEL: &str = "Files";

/// Default status bar text.
pub const STATUS_READY: &str = "Ready";

/// Status bar suffix for item count.
pub const STATUS_ITEMS_COUNT: &str = "items";

/// Column header: Name.
pub const COL_NAME: &str = "Name";

/// Column header: Size.
pub const COL_SIZE: &str = "Size";

/// Column header: Date modified.
pub const COL_MODIFIED: &str = "Date modified";

/// Column header: Type.
pub const COL_TYPE: &str = "Type";

/// Column header: Attributes.
pub const COL_ATTRIBUTES: &str = "Attributes";

/// Error: access denied.
pub const ERR_ACCESS_DENIED: &str = "Access denied";

/// Error: folder not found.
pub const ERR_NOT_FOUND: &str = "Folder not found";

/// Error: unsupported location.
pub const ERR_UNSUPPORTED_LOCATION: &str = "This location type is not supported yet";

/// Error: path is not a directory.
pub const ERR_NOT_A_DIRECTORY: &str = "Not a directory";

/// Menu label: navigate back.
pub const MENU_NAVIGATE_BACK: &str = "Back";

/// Menu label: navigate forward.
pub const MENU_NAVIGATE_FORWARD: &str = "Forward";

/// Menu label: navigate up.
pub const MENU_NAVIGATE_UP: &str = "Up";

/// Menu label: refresh.
pub const MENU_REFRESH: &str = "Refresh";

/// Shortcut tip: navigate back.
pub const SHORTCUT_TIP_BACK: &str = "Alt+Left";

/// Shortcut tip: navigate forward.
pub const SHORTCUT_TIP_FORWARD: &str = "Alt+Right";

/// Shortcut tip: navigate up.
pub const SHORTCUT_TIP_UP: &str = "Alt+Up";

/// Shortcut tip: refresh.
pub const SHORTCUT_TIP_REFRESH: &str = "F5";

/// Shortcut tip: focus address bar.
pub const SHORTCUT_TIP_ADDRESS_BAR: &str = "Ctrl+L or Alt+D";

/// Shortcut tip: select all.
pub const SHORTCUT_TIP_SELECT_ALL: &str = "Ctrl+A";

/// Suffix for selected-count announcement.
pub const SELECTED_COUNT_FMT: &str = "selected";

/// Prefix for folder-change announcement.
pub const FOLDER_CHANGE_FMT: &str = "Navigated to";
