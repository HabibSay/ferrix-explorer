//! Per-folder view state: remembered ViewMode and ColumnLayout.
//!
//! Persisted alongside window.ini in %APPDATA%\FerrixExplorer\view_state.ini.
//! The format is a simple hand-rolled ini:
//!   [C:\Users\alice]
//!   view_mode=Details
//!   sort_col=Name
//!   sort_dir=Ascending
//!
//! The same module holds the native-menu settings (blocklist and the global switch)
//! required by the Tranche 3 design; they are persisted in their own file,
//! menu_extensions.ini, because the file-level management UI is out of this tranche.

use fx_domain::view::{Column, ColumnLayout, SortDirection, SortKey, ViewMode};
use std::collections::HashMap;
use std::{fs, io, path::PathBuf};

// ── FolderViewState ───────────────────────────────────────────────────────────

/// Remembered view state for one folder path.
#[derive(Clone, Debug, Default)]
pub struct FolderViewState {
    pub view_mode: ViewMode,
    pub column_layout: ColumnLayout,
    pub sort_key: SortKey,
}

// ── ViewStateStore ────────────────────────────────────────────────────────────

/// In-memory store of per-folder view states with load/save.
pub struct ViewStateStore {
    states: HashMap<String, FolderViewState>,
    path: PathBuf,
}

impl ViewStateStore {
    /// Load from `%APPDATA%\FerrixExplorer\view_state.ini`, defaulting to an empty
    /// store when the file does not exist.
    pub fn load() -> io::Result<Self> {
        let path = view_state_path()?;
        let states = match fs::read_to_string(&path) {
            Ok(contents) => parse_view_state(&contents),
            Err(e) if e.kind() == io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(e),
        };
        Ok(ViewStateStore { states, path })
    }

    /// Create an empty in-memory store (no file backing) as a fallback.
    pub fn new_empty() -> Self {
        // Use a path that will not accidentally collide with a real file.
        ViewStateStore {
            states: HashMap::new(),
            path: PathBuf::from("__ferrix_no_backing_file__"),
        }
    }
    /// Return the remembered state for `folder`, or the default state if nothing was saved.
    pub fn get(&self, folder: &str) -> FolderViewState {
        self.states.get(folder).cloned().unwrap_or_default()
    }

    /// Update or insert the state for `folder`.
    pub fn set(&mut self, folder: &str, state: FolderViewState) {
        self.states.insert(folder.to_owned(), state);
    }

    /// Persist the entire store to disk.
    pub fn save(&self) -> io::Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("view_state path has no parent directory"))?;
        fs::create_dir_all(parent)?;

        let mut out = String::new();
        for (folder, state) in &self.states {
            out.push('[');
            out.push_str(folder);
            out.push_str("]\n");
            out.push_str("view_mode=");
            out.push_str(view_mode_to_str(state.view_mode));
            out.push('\n');
            out.push_str("sort_col=");
            out.push_str(column_to_str(state.sort_key.primary));
            out.push('\n');
            out.push_str("sort_dir=");
            out.push_str(sort_dir_to_str(state.sort_key.direction));
            out.push('\n');
        }
        fs::write(&self.path, out)
    }
}

// ── Serialisation helpers ─────────────────────────────────────────────────────

fn view_mode_to_str(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::List => "List",
        ViewMode::Details => "Details",
    }
}

fn str_to_view_mode(s: &str) -> ViewMode {
    match s {
        "List" => ViewMode::List,
        _ => ViewMode::Details,
    }
}

fn column_to_str(col: Column) -> &'static str {
    match col {
        Column::Name => "Name",
        Column::Size => "Size",
        Column::Modified => "Modified",
        Column::Type => "Type",
        Column::Attributes => "Attributes",
    }
}

fn str_to_column(s: &str) -> Column {
    match s {
        "Size" => Column::Size,
        "Modified" => Column::Modified,
        "Type" => Column::Type,
        "Attributes" => Column::Attributes,
        _ => Column::Name,
    }
}

fn sort_dir_to_str(dir: SortDirection) -> &'static str {
    match dir {
        SortDirection::Ascending => "Ascending",
        SortDirection::Descending => "Descending",
    }
}

fn str_to_sort_dir(s: &str) -> SortDirection {
    match s {
        "Descending" => SortDirection::Descending,
        _ => SortDirection::Ascending,
    }
}

// ── Parser ────────────────────────────────────────────────────────────────────

fn parse_view_state(contents: &str) -> HashMap<String, FolderViewState> {
    let mut map = HashMap::new();
    let mut current_folder: Option<String> = None;
    let mut view_mode = ViewMode::default();
    let mut sort_col = Column::Name;
    let mut sort_dir = SortDirection::Ascending;

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            // Flush the previous section before starting a new one.
            if let Some(folder) = current_folder.take() {
                map.insert(
                    folder,
                    FolderViewState {
                        view_mode,
                        column_layout: ColumnLayout::default(),
                        sort_key: SortKey {
                            primary: sort_col,
                            direction: sort_dir,
                            secondary: None,
                        },
                    },
                );
            }
            current_folder = Some(line[1..line.len() - 1].to_owned());
            view_mode = ViewMode::default();
            sort_col = Column::Name;
            sort_dir = SortDirection::Ascending;
        } else if let Some((key, value)) = line.split_once('=') {
            match key.trim() {
                "view_mode" => view_mode = str_to_view_mode(value.trim()),
                "sort_col" => sort_col = str_to_column(value.trim()),
                "sort_dir" => sort_dir = str_to_sort_dir(value.trim()),
                _ => {} // future keys are silently ignored
            }
        }
    }
    // Flush the last section.
    if let Some(folder) = current_folder {
        map.insert(
            folder,
            FolderViewState {
                view_mode,
                column_layout: ColumnLayout::default(),
                sort_key: SortKey {
                    primary: sort_col,
                    direction: sort_dir,
                    secondary: None,
                },
            },
        );
    }
    map
}

fn view_state_path() -> io::Result<PathBuf> {
    let base_dirs = directories::BaseDirs::new()
        .ok_or_else(|| io::Error::other("AppData could not be resolved"))?;
    Ok(base_dirs
        .config_dir()
        .join("FerrixExplorer")
        .join("view_state.ini"))
}

// ── MenuExtensionState ─────────────────────────────────────────────────────────

/// Local state of the native context menu: which extensions are blocked, and whether
/// the native menu is used at all.
///
/// The file holds one line per extension identity (`GetCommandString` verb tokens and
/// the CLSID/ProgID the Shell reports), so the user can turn a single extension off
/// without losing the rest (spec §8.5), and one global switch for the whole native
/// menu, the escape hatch for a machine without extensions (spec §8.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuExtensionState {
    /// False when the native menu is never used; Ferrix then shows its own menu only.
    pub native_menu_enabled: bool,
    /// The extensions the user turned off, by identity.
    pub blocked_extensions: Vec<String>,
}

impl Default for MenuExtensionState {
    fn default() -> Self {
        Self {
            native_menu_enabled: true,
            blocked_extensions: Vec::new(),
        }
    }
}

impl MenuExtensionState {
    /// Loads the state from `%APPDATA%\FerrixExplorer\menu_extensions.ini`.
    ///
    /// A missing file yields the defaults: the native menu is on and nothing is blocked.
    pub fn load() -> io::Result<Self> {
        let path = menu_extensions_path()?;
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        Ok(parse_menu_extensions(&contents))
    }

    /// Persists the state, replacing any previous file.
    pub fn save(&self) -> io::Result<()> {
        let path = menu_extensions_path()?;
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("menu extensions path has no parent directory"))?;
        fs::create_dir_all(parent)?;
        fs::write(&path, format_menu_extensions(self))
    }

    /// Whether the given extension identity may run.
    pub fn is_blocked(&self, identity: &str) -> bool {
        self.blocked_extensions
            .iter()
            .any(|blocked| blocked.eq_ignore_ascii_case(identity))
    }

    /// Blocks an extension identity, keeping the others untouched.
    pub fn block(&mut self, identity: &str) {
        if !self.is_blocked(identity) {
            self.blocked_extensions.push(identity.to_string());
        }
    }

    /// Allows an extension identity again.
    pub fn unblock(&mut self, identity: &str) {
        self.blocked_extensions
            .retain(|blocked| !blocked.eq_ignore_ascii_case(identity));
    }
}

fn parse_menu_extensions(contents: &str) -> MenuExtensionState {
    let mut state = MenuExtensionState::default();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "native_menu" => {
                state.native_menu_enabled = matches!(value.trim(), "on" | "true" | "1");
            }
            "blocked" => {
                let identity = value.trim();
                if !identity.is_empty() {
                    state.block(identity);
                }
            }
            _ => {}
        }
    }
    state
}

fn format_menu_extensions(state: &MenuExtensionState) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "native_menu={}\n",
        if state.native_menu_enabled {
            "on"
        } else {
            "off"
        }
    ));
    for identity in &state.blocked_extensions {
        out.push_str(&format!("blocked={identity}\n"));
    }
    out
}

fn menu_extensions_path() -> io::Result<PathBuf> {
    let base_dirs = directories::BaseDirs::new()
        .ok_or_else(|| io::Error::other("AppData could not be resolved"))?;
    Ok(base_dirs
        .config_dir()
        .join("FerrixExplorer")
        .join("menu_extensions.ini"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use fx_domain::view::{SortDirection, SortKey, ViewMode};

    fn make_store_at(path: PathBuf) -> ViewStateStore {
        ViewStateStore {
            states: HashMap::new(),
            path,
        }
    }

    #[test]
    fn round_trip_two_folders() {
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        let path = tmp.path().to_path_buf();

        let mut store = make_store_at(path.clone());

        store.set(
            r"C:\Users\alice",
            FolderViewState {
                view_mode: ViewMode::List,
                column_layout: ColumnLayout::default(),
                sort_key: SortKey {
                    primary: Column::Size,
                    direction: SortDirection::Descending,
                    secondary: None,
                },
            },
        );
        store.set(
            r"C:\Users\bob",
            FolderViewState {
                view_mode: ViewMode::Details,
                column_layout: ColumnLayout::default(),
                sort_key: SortKey {
                    primary: Column::Modified,
                    direction: SortDirection::Ascending,
                    secondary: None,
                },
            },
        );

        store.save().expect("save");

        let loaded = {
            let contents = fs::read_to_string(&path).expect("read");
            parse_view_state(&contents)
        };

        let alice = loaded.get(r"C:\Users\alice").expect("alice");
        assert!(matches!(alice.view_mode, ViewMode::List));
        assert_eq!(alice.sort_key.primary, Column::Size);
        assert_eq!(alice.sort_key.direction, SortDirection::Descending);

        let bob = loaded.get(r"C:\Users\bob").expect("bob");
        assert!(matches!(bob.view_mode, ViewMode::Details));
        assert_eq!(bob.sort_key.primary, Column::Modified);
        assert_eq!(bob.sort_key.direction, SortDirection::Ascending);
    }

    #[test]
    fn missing_file_gives_empty_store() {
        let path = PathBuf::from(r"C:\__nonexistent_ferrix_test__\view_state.ini");
        let store = make_store_at(path);
        let state = store.get(r"C:\any\folder");
        // Default view mode is Details.
        assert!(matches!(state.view_mode, ViewMode::Details));
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let contents =
            "[C:\\test]\nview_mode=List\nunknown_key=whatever\nsort_col=Name\nsort_dir=Ascending\n";
        let map = parse_view_state(contents);
        let state = map.get(r"C:\test").expect("entry");
        assert!(matches!(state.view_mode, ViewMode::List));
    }

    #[test]
    fn menu_extensions_default_to_the_native_menu_being_on() {
        let state = MenuExtensionState::default();
        assert!(state.native_menu_enabled);
        assert!(state.blocked_extensions.is_empty());
    }

    #[test]
    fn menu_extensions_round_trip_through_their_file_format() {
        let mut state = MenuExtensionState::default();
        state.block("7zip");
        state.block("git");
        let parsed = parse_menu_extensions(&format_menu_extensions(&state));
        assert_eq!(parsed, state);
        assert!(parsed.is_blocked("7Zip"));
        assert!(parsed.is_blocked("git"));
        assert!(!parsed.is_blocked("onedrive"));
    }

    #[test]
    fn menu_extensions_can_be_switched_off_entirely() {
        let parsed = parse_menu_extensions("native_menu=off\n");
        assert!(!parsed.native_menu_enabled);
        let parsed = parse_menu_extensions("native_menu=on\n");
        assert!(parsed.native_menu_enabled);
    }

    #[test]
    fn unblocking_an_extension_keeps_the_others_blocked() {
        let mut state = MenuExtensionState::default();
        state.block("7zip");
        state.block("git");
        state.unblock("7zip");
        assert!(!state.is_blocked("7zip"));
        assert!(state.is_blocked("git"));
    }

    #[test]
    fn blocking_the_same_extension_twice_records_it_once() {
        let mut state = MenuExtensionState::default();
        state.block("7zip");
        state.block("7zip");
        assert_eq!(state.blocked_extensions.len(), 1);
    }
}
