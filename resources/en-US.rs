//! English (en-US) UI strings for the native host and the app-local Shell menu.
//!
//! Shared Slint strings live in `resources/en-US.slint`. Add new locales as sibling files and
//! keep identifiers stable; no user-visible text should be hard-coded in UI code.

/// Caption of the top-level window. Also used as the window's UI Automation name.
pub const APP_TITLE: &str = "Ferrix Explorer — Tranche 2";

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

/// Placeholder shown in address bar autocomplete.
pub const AUTOCOMPLETE_PLACEHOLDER: &str = "Type a path…";

/// Label for the subfolder menu.
pub const SUBFOLDER_MENU_LABEL: &str = "Subfolders";

/// Tooltip for the back navigation button.
pub const BACK_BUTTON_LABEL: &str = "Back (Alt+Left)";

/// Tooltip for the forward navigation button.
pub const FORWARD_BUTTON_LABEL: &str = "Forward (Alt+Right)";

/// Tooltip for the up navigation button.
pub const UP_BUTTON_LABEL: &str = "Up (Alt+Up)";

/// Tooltip for the refresh button.
pub const REFRESH_BUTTON_LABEL: &str = "Refresh (F5)";

// ── Tranche 2 — file operations ───────────────────────────────────────────────

/// Activity row and menu label: paste from the clipboard.
pub const ACTIVITY_PASTE: &str = "Paste";

/// Activity row label: send the selection to the Recycle Bin.
pub const ACTIVITY_RECYCLE: &str = "Recycle";

/// Activity row label: permanent deletion.
pub const ACTIVITY_DELETE_PERMANENTLY: &str = "Delete permanently";

/// Activity row label: rename.
pub const ACTIVITY_RENAME: &str = "Rename";

/// Activity row label and default name: new folder.
pub const ACTIVITY_NEW_FOLDER: &str = "New folder";

/// Activity row label: new file.
pub const ACTIVITY_NEW_FILE: &str = "New file";

/// Activity row label: undo.
pub const ACTIVITY_UNDO: &str = "Undo";

/// Activity row label: redo.
pub const ACTIVITY_REDO: &str = "Redo";

/// Activity row and menu label: checksums.
pub const ACTIVITY_CHECKSUMS: &str = "Checksums";

/// Activity row label: the read-only pass of the empty-folder cleaner.
pub const ACTIVITY_CLEAN_SCAN: &str = "Clean empty folders (scan)";

/// Activity row and menu label: the destructive pass of the cleaner.
pub const ACTIVITY_CLEAN: &str = "Clean empty folders";

/// Prefix of a retry row label, e.g. "Retry Paste".
pub const ACTIVITY_RETRY_PREFIX: &str = "Retry ";

/// Context menu: select entries by glob pattern.
pub const MENU_SELECT_BY_PATTERN: &str = "Select by pattern";

/// Context menu: invert the current selection.
pub const MENU_INVERT_SELECTION: &str = "Invert selection";

/// Context menu: create a new file.
pub const MENU_NEW_FILE: &str = "New file";

/// Context menu: copy the full path of the selection.
pub const MENU_COPY_PATH: &str = "Copy path";

/// Context menu: copy the name of the selection.
pub const MENU_COPY_NAME: &str = "Copy name";

/// Context menu: open the properties of the selection.
pub const MENU_PROPERTIES: &str = "Properties";

/// Context menu: open the "Open with" dialog of the selection.
pub const MENU_OPEN_WITH: &str = "Open with";

/// Status bar: the native menu could not be shown, so Ferrix showed its own menu.
pub const STATUS_MENU_FALLBACK: &str =
    "The Windows menu could not be shown; using the Ferrix menu.";

/// Status bar: the chosen extension is turned off in the local list.
pub const STATUS_MENU_EXTENSION_BLOCKED: &str =
    "That extension is turned off in Ferrix's settings.";

/// Status bar: a drag ended in the folder the items came from.
pub const STATUS_DROP_IN_SOURCE: &str = "Dropped in the source folder — nothing to do.";

/// Status bar prefix when a rename is refused, followed by the reason.
pub const STATUS_RENAME_REFUSED: &str = "Rename refused: ";

/// Status bar when the current location is not a local folder.
pub const STATUS_NEEDS_LOCAL_FOLDER: &str = "File operations need a local folder.";

/// Status bar when an operation ended without a more specific reason.
pub const STATUS_OPERATION_FAILED: &str = "Operation failed";

/// Status bar: the cleaner scan found nothing to remove.
pub const STATUS_NO_EMPTY_FOLDERS: &str = "No empty folders found.";

/// Status bar: the listing is complete.
pub const STATUS_DONE: &str = "Done.";

/// Status bar: a listing is in flight.
pub const STATUS_LOADING: &str = "Loading\u{2026}";

/// Status bar suffix counting matched entries.
pub const ENTRIES_MATCHED: &str = "entries matched";

/// Status bar suffix counting empty folders.
pub const EMPTY_FOLDERS: &str = "empty folders";

/// Progress suffix counting files.
pub const PROGRESS_FILES: &str = "files";

/// Accessible progress wording: the percent suffix.
pub const PROGRESS_PERCENT: &str = "percent";

/// Drag feedback: the Shell will copy.
pub const DRAG_COPY: &str = "Copy (drag)";

/// Drag feedback: the Shell will move.
pub const DRAG_MOVE: &str = "Move (drag)";

/// Default name of a newly created file.
pub const NEW_FILE_NAME: &str = "New file.txt";

/// Activity row state: queued, not started.
pub const STATE_PENDING: &str = "pending";

/// Activity row state: transferring.
pub const STATE_RUNNING: &str = "running";

/// Activity row state: paused by the user.
pub const STATE_PAUSED: &str = "paused";

/// Activity row state: finished with nothing failed.
pub const STATE_COMPLETED: &str = "completed";

/// Activity row state: finished with per-item failures.
pub const STATE_COMPLETED_WITH_WARNINGS: &str = "completed with warnings";

/// Activity row state: ended on an operation-level error.
pub const STATE_FAILED: &str = "failed";

/// Activity row state: ended because the user cancelled.
pub const STATE_CANCELLED: &str = "cancelled";

/// Entry kind column: directory.
pub const KIND_FOLDER: &str = "Folder";

/// Entry kind column: regular file.
pub const KIND_FILE: &str = "File";

/// Entry kind column: symbolic link.
pub const KIND_SYMLINK: &str = "Symlink";

/// Entry kind column: any other reparse point.
pub const KIND_REPARSE_POINT: &str = "Reparse Point";
