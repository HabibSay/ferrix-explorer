use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinnedStore {
    locations: Vec<PathBuf>,
}

impl PinnedStore {
    pub fn new_empty() -> Self {
        Self {
            locations: Vec::new(),
        }
    }

    pub fn locations(&self) -> &[PathBuf] {
        &self.locations
    }

    pub fn pin(&mut self, location: PathBuf) -> bool {
        if self.locations.contains(&location) {
            return false;
        }
        self.locations.push(location);
        true
    }

    pub fn unpin(&mut self, location: impl AsRef<Path>) -> bool {
        let location = location.as_ref();
        let Some(index) = self.locations.iter().position(|pinned| pinned == location) else {
            return false;
        };
        self.locations.remove(index);
        true
    }

    pub fn move_location(&mut self, from: usize, to: usize) {
        if from == to || from >= self.locations.len() || to >= self.locations.len() {
            return;
        }
        let location = self.locations.remove(from);
        self.locations.insert(to, location);
    }

    pub fn load() -> io::Result<Self> {
        let path = pinned_path()?;
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::new_empty()),
            Err(error) => return Err(error),
        };
        Ok(parse_pinned(&contents))
    }

    pub fn save(&self) -> io::Result<()> {
        let path = pinned_path()?;
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("pinned sidebar path has no parent"))?;
        fs::create_dir_all(parent)?;
        fs::write(path, format_pinned(self))
    }
}

fn pinned_path() -> io::Result<PathBuf> {
    let base_dirs = directories::BaseDirs::new()
        .ok_or_else(|| io::Error::other("AppData could not be resolved"))?;
    Ok(base_dirs
        .config_dir()
        .join("FerrixExplorer")
        .join("pinned_sidebar.ini"))
}

fn parse_pinned(contents: &str) -> PinnedStore {
    let locations = contents
        .lines()
        .filter_map(|line| line.strip_prefix("pinned="))
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect();
    PinnedStore { locations }
}

fn format_pinned(store: &PinnedStore) -> String {
    store
        .locations
        .iter()
        .map(|location| format!("pinned={}\n", location.to_string_lossy()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn add_and_remove_pinned_locations() {
        let mut store = PinnedStore::new_empty();
        store.pin("D\\Downloads".into());
        store.pin("C\\Users".into());

        assert_eq!(store.locations().len(), 2);
        assert!(store.unpin("D\\Downloads"));
        assert_eq!(store.locations(), &[PathBuf::from("C\\Users")]);
    }

    #[test]
    fn reorder_moves_a_pinned_location() {
        let mut store = PinnedStore {
            locations: vec![
                PathBuf::from("C\\Documents"),
                PathBuf::from("C\\Downloads"),
                PathBuf::from("C\\Pictures"),
            ],
        };

        store.move_location(2, 0);
        assert_eq!(
            store.locations(),
            &[
                PathBuf::from("C\\Pictures"),
                PathBuf::from("C\\Documents"),
                PathBuf::from("C\\Downloads"),
            ]
        );
    }

    #[test]
    fn duplicate_pin_is_rejected() {
        let mut store = PinnedStore::new_empty();
        assert!(store.pin(PathBuf::from("C\\Users")));
        assert!(!store.pin(PathBuf::from("C\\Users")));
        assert_eq!(store.locations().len(), 1);
    }

    #[test]
    fn serialization_round_trips_in_order() {
        let store = PinnedStore {
            locations: vec![PathBuf::from("C\\Documents"), PathBuf::from("D\\Archive")],
        };

        let parsed = parse_pinned(&format_pinned(&store));
        assert_eq!(parsed, store);
    }
}
