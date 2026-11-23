use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabStore {
    paths: Vec<PathBuf>,
    active: usize,
}

impl TabStore {
    pub fn new_empty() -> Self {
        Self {
            paths: Vec::new(),
            active: 0,
        }
    }

    pub fn open(&mut self, path: PathBuf) {
        self.paths.push(path);
        self.active = self.paths.len().saturating_sub(1);
    }

    pub fn activate(&mut self, index: usize) {
        if index < self.paths.len() {
            self.active = index;
        }
    }

    pub fn close_active(&mut self) -> bool {
        if self.paths.is_empty() {
            return false;
        }
        self.paths.remove(self.active);
        if self.paths.is_empty() {
            self.paths.push(PathBuf::from(r"C:\"));
            self.active = 0;
        } else if self.active >= self.paths.len() {
            self.active = self.paths.len() - 1;
        }
        true
    }

    pub fn reorder(&mut self, from: usize, to: usize) {
        if from == to || from >= self.paths.len() || to >= self.paths.len() {
            return;
        }
        let active_path = self.paths.get(self.active).cloned();
        let item = self.paths.remove(from);
        self.paths.insert(to, item);
        if let Some(active_path) = active_path {
            self.active = self
                .paths
                .iter()
                .position(|path| path == &active_path)
                .unwrap_or(0);
        }
    }

    pub fn cycle_next(&mut self) {
        if self.paths.is_empty() {
            return;
        }
        self.active = (self.active + 1) % self.paths.len();
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active_path(&self) -> Option<&PathBuf> {
        self.paths.get(self.active)
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.paths.iter().any(|candidate| candidate == path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn adding_a_tab_makes_it_active() {
        let mut tabs = TabStore::new_empty();
        tabs.open(PathBuf::from(r"C:\Users"));
        tabs.open(PathBuf::from(r"D:\Projects"));

        assert_eq!(tabs.active_index(), 1);
        assert_eq!(tabs.active_path(), Some(&PathBuf::from(r"D:\Projects")));
    }

    #[test]
    fn closing_the_last_tab_creates_a_home_tab() {
        let mut tabs = TabStore::new_empty();
        tabs.open(PathBuf::from(r"C:\Users"));
        assert!(tabs.close_active());

        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.active_path(), Some(&PathBuf::from(r"C:\")));
    }

    #[test]
    fn reorder_keeps_the_active_tab() {
        let mut tabs = TabStore::new_empty();
        tabs.open(PathBuf::from(r"C:\A"));
        tabs.open(PathBuf::from(r"C:\B"));
        tabs.open(PathBuf::from(r"C:\C"));

        tabs.reorder(2, 0);
        assert_eq!(tabs.active_path(), Some(&PathBuf::from(r"C:\C")));
    }

    #[test]
    fn cycle_moves_to_the_next_tab() {
        let mut tabs = TabStore::new_empty();
        tabs.open(PathBuf::from(r"C:\A"));
        tabs.open(PathBuf::from(r"C:\B"));
        tabs.cycle_next();

        assert_eq!(tabs.active_index(), 0);
    }
}
