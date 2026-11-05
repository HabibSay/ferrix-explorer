//! Per-folder view state: remembered ViewMode and ColumnLayout.
//!
//! Persisted alongside window.ini in %APPDATA%\FerrixExplorer\view_state.ini.
//! The format is a simple hand-rolled ini:
//!   [C:\Users\alice]
//!   view_mode=Details
//!   sort_col=Name
//!   sort_dir=Ascending

use fx_domain::view::{Column, ColumnLayout, SortDirection, SortKey, ViewMode};
use std::collections::HashMap;
use std::{fs, io, path::PathBuf};

// ── FolderViewState ───────────────────────────────────────────────────────────

/// Remembered view state for one folder path.
#[derive(Clone, Debug)]
pub struct FolderViewState {
    pub view_mode: ViewMode,
    pub column_layout: ColumnLayout,
    pub sort_key: SortKey,
}

impl Default for FolderViewState {
    fn default() -> Self {
        FolderViewState {
            view_mode: ViewMode::default(),
            column_layout: ColumnLayout::default(),
            sort_key: SortKey::default(),
        }
    }
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
        let parent = self.path.parent().ok_or_else(|| {
            io::Error::other("view_state path has no parent directory")
        })?;
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
        let contents = "[C:\\test]\nview_mode=List\nunknown_key=whatever\nsort_col=Name\nsort_dir=Ascending\n";
        let map = parse_view_state(contents);
        let state = map.get(r"C:\test").expect("entry");
        assert!(matches!(state.view_mode, ViewMode::List));
    }
}
