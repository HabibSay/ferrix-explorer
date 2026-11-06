//! View model Slint-facing pour Ferrix Explorer Tranche 1.
//!
//! Toute mutation de l'état Slint passe par `slint::invoke_from_event_loop` pour
//! garantir que les modifications n'arrivent que depuis l'event loop Slint.

use fx_domain::{
    entry::{EntryKind, FileEntry},
    filter::{FilterQuery, VisibilityRules},
    view::{Column, ColumnLayout, SortDirection, SortKey, ViewMode},
};
use std::collections::HashSet;

/// Représentation plate d'une entrée pour l'UI Slint.
#[derive(Clone, Debug)]
pub struct UiFileRow {
    pub id: u64,
    pub display_name: String,
    pub kind: String,
    pub size_text: String,
    pub modified_text: String,
    pub type_text: String,
    pub selected: bool,
    pub icon_key: u64,
}

/// L'état complet du view model, manipulé uniquement depuis l'event loop.
pub struct FileListViewModel {
    /// Toutes les entrées du répertoire courant (non filtrées).
    all_entries: Vec<FileEntry>,
    /// Entrées filtrées et triées, prêtes pour l'UI.
    visible_entries: Vec<FileEntry>,
    /// Filtres actifs.
    filter: FilterQuery,
    visibility: VisibilityRules,
    /// Ordre de tri actif.
    sort_key: SortKey,
    /// Sélection courante (par id brut).
    selected_ids: HashSet<u64>,
    /// Mise en page des colonnes.
    column_layout: ColumnLayout,
    /// Mode d'affichage actif (List ou Details).
    _view_mode: ViewMode,
}

impl FileListViewModel {
    pub fn new() -> Self {
        Self {
            all_entries: Vec::new(),
            visible_entries: Vec::new(),
            filter: FilterQuery::default(),
            visibility: VisibilityRules::default(),
            sort_key: SortKey::default(),
            selected_ids: HashSet::new(),
            column_layout: ColumnLayout::default(),
            _view_mode: ViewMode::default(),
        }
    }

    /// Ajoute un batch d'entrées reçu du listing (streaming).
    pub fn append_batch(&mut self, batch: Vec<FileEntry>) {
        self.all_entries.extend(batch);
        self.rebuild_visible();
    }

    /// Réinitialise pour une nouvelle navigation.
    pub fn clear(&mut self) {
        self.all_entries.clear();
        self.visible_entries.clear();
        self.selected_ids.clear();
    }

    /// Applique filtre + visibilité + tri pour reconstruire `visible_entries`.
    pub fn rebuild_visible(&mut self) {
        let mut visible: Vec<FileEntry> = self
            .all_entries
            .iter()
            .filter(|e| self.visibility.is_visible(e) && self.filter.matches(e))
            .cloned()
            .collect();
        sort_entries(&mut visible, &self.sort_key);
        self.visible_entries = visible;
    }

    /// Met à jour le texte de filtre et reconstruit la liste visible.
    pub fn set_filter(&mut self, text: String) {
        self.filter = FilterQuery { text };
        self.rebuild_visible();
    }

    /// Met à jour l'ordre de tri et reconstruit.
    pub fn set_sort_key(&mut self, key: SortKey) {
        self.sort_key = key;
        self.rebuild_visible();
    }

    /// Sélection simple (efface la sélection précédente).
    pub fn select_single(&mut self, id: u64) {
        self.selected_ids.clear();
        self.selected_ids.insert(id);
    }

    /// Toggle Ctrl+click.
    pub fn toggle_select(&mut self, id: u64) {
        if self.selected_ids.contains(&id) {
            self.selected_ids.remove(&id);
        } else {
            self.selected_ids.insert(id);
        }
    }

    /// Shift+click: sélectionne la plage entre le dernier sélectionné et `id`.
    pub fn range_select(&mut self, id: u64) {
        let target_idx = self.visible_entries.iter().position(|e| e.id.0 == id);
        let anchor_idx = self
            .visible_entries
            .iter()
            .rposition(|e| self.selected_ids.contains(&e.id.0));
        if let (Some(t), Some(a)) = (target_idx, anchor_idx) {
            let (lo, hi) = if a <= t { (a, t) } else { (t, a) };
            for e in &self.visible_entries[lo..=hi] {
                self.selected_ids.insert(e.id.0);
            }
        } else {
            self.select_single(id);
        }
    }

    /// Ctrl+A: sélectionne toutes les entrées visibles.
    pub fn select_all(&mut self) {
        for e in &self.visible_entries {
            self.selected_ids.insert(e.id.0);
        }
    }

    /// Retourne les lignes prêtes pour Slint.
    pub fn to_ui_rows(&self) -> Vec<UiFileRow> {
        self.visible_entries
            .iter()
            .map(|e| UiFileRow {
                id: e.id.0,
                display_name: e.display_name.clone(),
                kind: format_kind(&e.kind).to_string(),
                size_text: format_size(e.size),
                modified_text: format_modified(e.modified),
                type_text: format_kind(&e.kind).to_string(),
                selected: self.selected_ids.contains(&e.id.0),
                icon_key: e.icon_key,
            })
            .collect()
    }

    pub fn selected_count(&self) -> usize {
        self.selected_ids.len()
    }

    pub fn entry_count(&self) -> usize {
        self.visible_entries.len()
    }

    pub fn column_layout(&self) -> &ColumnLayout {
        &self.column_layout
    }

    pub fn sort_key(&self) -> &SortKey {
        &self.sort_key
    }

    /// Return the filesystem path for `entry_id`, if it is in the current listing.
    pub fn entry_path(&self, id: u64) -> Option<std::path::PathBuf> {
        self.all_entries
            .iter()
            .find(|e| e.id.0 == id)
            .map(|e| e.path.clone())
    }

    /// Return the kind string for `entry_id`.
    pub fn entry_kind_str(&self, id: u64) -> Option<&'static str> {
        self.all_entries
            .iter()
            .find(|e| e.id.0 == id)
            .map(|e| format_kind(&e.kind))
    }

    /// Update the active view mode (List or Details). No rebuild needed.
    pub fn set_view_mode(&mut self, _mode: fx_domain::view::ViewMode) {
        // ViewMode is consumed by the orchestrator for column layout decisions;
        // the view model stores it for round-trip persistence.
        self._view_mode = _mode;
    }

    /// Return the active view mode.
    pub fn view_mode(&self) -> fx_domain::view::ViewMode {
        self._view_mode
    }

    /// Update visibility rules (show hidden/system files) and rebuild the visible list.
    pub fn set_visibility_rules(&mut self, rules: VisibilityRules) {
        self.visibility = rules;
        self.rebuild_visible();
    }

    /// Update the column layout (widths, order, visibility). No rebuild needed.
    pub fn set_column_layout(&mut self, layout: ColumnLayout) {
        self.column_layout = layout;
    }
}

impl Default for FileListViewModel {
    fn default() -> Self {
        Self::new()
    }
}

/// Trie les entrées: dossiers en premier, puis case-insensitive sur la colonne primaire.
fn sort_entries(entries: &mut [FileEntry], key: &SortKey) {
    entries.sort_by(|a, b| {
        // Dossiers en premier
        let a_dir = matches!(a.kind, EntryKind::Directory);
        let b_dir = matches!(b.kind, EntryKind::Directory);
        if a_dir != b_dir {
            return if a_dir {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }
        // Colonne primaire
        let ord = compare_by_column(a, b, key.primary);
        let ord = if matches!(key.direction, SortDirection::Descending) {
            ord.reverse()
        } else {
            ord
        };
        // Colonne secondaire si égalité
        if ord == std::cmp::Ordering::Equal
            && let Some((sec_col, sec_dir)) = key.secondary
        {
            let ord2 = compare_by_column(a, b, sec_col);
            return if matches!(sec_dir, SortDirection::Descending) {
                ord2.reverse()
            } else {
                ord2
            };
        }
        ord
    });
}

fn compare_by_column(a: &FileEntry, b: &FileEntry, col: Column) -> std::cmp::Ordering {
    match col {
        Column::Name => a
            .display_name
            .to_ascii_lowercase()
            .cmp(&b.display_name.to_ascii_lowercase()),
        Column::Size => a.size.cmp(&b.size),
        Column::Modified => a.modified.cmp(&b.modified),
        Column::Type => format_kind(&a.kind).cmp(format_kind(&b.kind)),
        Column::Attributes => std::cmp::Ordering::Equal,
    }
}

fn format_kind(kind: &EntryKind) -> &'static str {
    match kind {
        EntryKind::Directory => "Folder",
        EntryKind::File => "File",
        EntryKind::Symlink => "Symlink",
        EntryKind::ReparsePoint => "Reparse Point",
    }
}

fn format_size(size: Option<u64>) -> String {
    match size {
        None => String::new(),
        Some(s) if s < 1024 => format!("{s} B"),
        Some(s) if s < 1024 * 1024 => format!("{} KB", s / 1024),
        Some(s) if s < 1024 * 1024 * 1024 => format!("{} MB", s / (1024 * 1024)),
        Some(s) => format!("{} GB", s / (1024 * 1024 * 1024)),
    }
}

fn format_modified(modified: Option<std::time::SystemTime>) -> String {
    match modified {
        None => String::new(),
        Some(t) => match t.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => format!("{}", d.as_secs()),
            Err(_) => String::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fx_domain::entry::{EntryId, EntryKind, FileAttributes, FileEntry};
    use std::path::PathBuf;

    fn make_entry(id: u64, name: &str, kind: EntryKind) -> FileEntry {
        FileEntry {
            id: EntryId(id),
            path: PathBuf::from(name),
            display_name: name.to_string(),
            kind,
            size: None,
            modified: None,
            attributes: FileAttributes::default(),
            icon_key: 0,
        }
    }

    #[test]
    fn folders_sort_before_files() {
        let mut vm = FileListViewModel::new();
        vm.append_batch(vec![
            make_entry(1, "z_file.txt", EntryKind::File),
            make_entry(2, "a_folder", EntryKind::Directory),
        ]);
        let rows = vm.to_ui_rows();
        assert_eq!(rows[0].kind, "Folder");
        assert_eq!(rows[1].kind, "File");
    }

    #[test]
    fn filter_is_case_insensitive() {
        let mut vm = FileListViewModel::new();
        vm.append_batch(vec![
            make_entry(1, "Hello.txt", EntryKind::File),
            make_entry(2, "world.rs", EntryKind::File),
        ]);
        vm.set_filter("HELLO".to_string());
        let rows = vm.to_ui_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].display_name, "Hello.txt");
    }

    #[test]
    fn ctrl_a_selects_all_visible() {
        let mut vm = FileListViewModel::new();
        vm.append_batch(vec![
            make_entry(1, "a.txt", EntryKind::File),
            make_entry(2, "b.txt", EntryKind::File),
        ]);
        vm.select_all();
        assert_eq!(vm.selected_count(), 2);
    }

    #[test]
    fn selection_survives_rebuild() {
        let mut vm = FileListViewModel::new();
        vm.append_batch(vec![
            make_entry(1, "a.txt", EntryKind::File),
            make_entry(2, "b.txt", EntryKind::File),
        ]);
        vm.select_single(1);
        vm.rebuild_visible();
        let rows = vm.to_ui_rows();
        assert!(rows.iter().any(|r| r.id == 1 && r.selected));
    }
}
