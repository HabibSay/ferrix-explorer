//! View model Slint-facing pour Ferrix Explorer Tranche 1.
//!
//! Toute mutation de l'état Slint passe par `slint::invoke_from_event_loop` pour
//! garantir que les modifications n'arrivent que depuis l'event loop Slint.

use fx_domain::{
    entry::{EntryKind, FileEntry},
    filter::{FilterQuery, VisibilityRules},
    view::{Column, ColumnLayout, SortDirection, SortKey, ViewMode},
};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Représentation plate d'une entrée pour l'UI Slint.
#[derive(Clone, Debug)]
pub struct UiFileRow {
    pub id: u64,
    pub display_name: String,
    pub kind: String,
    /// Behaviour flag for the UI: a drop only lands on a folder, and that decision must
    /// not depend on the wording of the `kind` column.
    pub is_folder: bool,
    pub size_text: String,
    pub modified_text: String,
    pub type_text: String,
    pub selected: bool,
    pub icon_key: u64,
    /// The row's thumbnail, when one has arrived (Tranche 3 slice 4).
    ///
    /// `None` is the normal state while a row is still being fetched and after a miss; the
    /// row then renders without an image, exactly as it did before this slice.
    pub thumbnail: Option<std::sync::Arc<ThumbnailPixels>>,
}

/// Decoded pixels for one row's thumbnail.
///
/// Deliberately not a `slint::Image`: this type travels between the blocking pool and the
/// UI thread, and a Slint texture is not `Send`. The conversion happens on the Slint thread,
/// in [`crate::`]-level UI code, where a texture may legitimately be created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThumbnailPixels {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes of RGBA, rows top-down.
    pub pixels: Vec<u8>,
}

/// Extensions Ferrix asks the Shell for a thumbnail of (Tranche 3 slice 4).
///
/// The Shell answers for anything, but a listing of sources, text and archives would spend
/// the fetch budget on images that do not exist. The list is a filter on *requests*, not a
/// claim about what the Shell can produce: a type that is missing here simply keeps its
/// icon until the list grows.
const THUMBNAIL_EXTENSIONS: &[&str] = &[
    "avif", "bmp", "gif", "heic", "ico", "jfif", "jpeg", "jpg", "png", "svg", "tif", "tiff",
    "webp", "mp4", "mov", "m4v", "wmv", "avi", "mkv",
];

/// Whether a row is worth asking the Shell for a thumbnail.
///
/// Pure, so the request filter is tested without a Shell or a window: the extension is
/// compared case-insensitively and a file with no extension is never a candidate.
pub fn is_thumbnail_candidate(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    THUMBNAIL_EXTENSIONS
        .iter()
        .any(|known| extension.eq_ignore_ascii_case(known))
}

/// One thumbnail the view model is waiting for (Tranche 3 slice 4).
///
/// The generation travels with the request and comes back with the result: a result that
/// arrives after a navigation belongs to a listing that is gone, and applying it would put
/// one folder's picture on another folder's row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThumbnailRequest {
    /// The row the thumbnail belongs to.
    pub entry_id: u64,
    /// The listing generation at request time.
    pub generation: u32,
    /// The file to fetch.
    pub path: std::path::PathBuf,
    /// Requested edge length in pixels.
    pub size: u32,
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
    /// Entry currently being renamed inline (`None` = none).
    rename_target: Option<u64>,
    /// Bumped on every navigation, so a late thumbnail result can be recognised as stale.
    thumbnail_generation: u32,
    /// The thumbnails that have arrived, by entry id.
    thumbnails: HashMap<u64, std::sync::Arc<ThumbnailPixels>>,
    /// Rows a thumbnail has already been asked for in this generation.
    ///
    /// A row is asked for once: a miss is not retried in a loop, because a failing Shell
    /// handler would otherwise be called again on every pump.
    thumbnails_requested: HashSet<u64>,
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
            rename_target: None,
            thumbnail_generation: 0,
            thumbnails: HashMap::new(),
            thumbnails_requested: HashSet::new(),
        }
    }

    /// The current listing generation (Tranche 3 slice 4).
    pub fn thumbnail_generation(&self) -> u32 {
        self.thumbnail_generation
    }

    /// The next thumbnail worth fetching, or `None` when every candidate already has one.
    ///
    /// Only visible rows are considered: a row the user cannot see is not worth a Shell
    /// call, and when the user scrolls to it the next turn picks it up. Folders are never
    /// candidates — their picture is the folder icon, not a preview.
    pub fn next_thumbnail_request(&mut self, size: u32) -> Option<ThumbnailRequest> {
        let generation = self.thumbnail_generation;
        let requested = &self.thumbnails_requested;
        let thumbnails = &self.thumbnails;
        let entry = self.visible_entries.iter().find(|entry| {
            !matches!(entry.kind, EntryKind::Directory)
                && !thumbnails.contains_key(&entry.id.0)
                && !requested.contains(&entry.id.0)
                && is_thumbnail_candidate(&entry.path)
        })?;
        let request = ThumbnailRequest {
            entry_id: entry.id.0,
            generation,
            path: entry.path.clone(),
            size,
        };
        self.thumbnails_requested.insert(request.entry_id);
        Some(request)
    }

    /// Applies a fetched thumbnail, or refuses it as stale.
    ///
    /// Two guards, because a thumbnail can be late in two different ways: the generation
    /// catches a result that outlived a navigation, and the visibility check catches one for
    /// a row that a filter or a sort pushed out of the list while it was in flight.
    ///
    /// Returns `true` when the image was stored and the row must be repainted.
    pub fn apply_thumbnail(
        &mut self,
        generation: u32,
        entry_id: u64,
        image: std::sync::Arc<ThumbnailPixels>,
    ) -> bool {
        if generation != self.thumbnail_generation {
            return false;
        }
        if !self
            .visible_entries
            .iter()
            .any(|entry| entry.id.0 == entry_id)
        {
            return false;
        }
        self.thumbnails.insert(entry_id, image);
        true
    }

    /// The thumbnail for `entry_id`, when one has arrived.
    pub fn thumbnail(&self, entry_id: u64) -> Option<std::sync::Arc<ThumbnailPixels>> {
        self.thumbnails.get(&entry_id).cloned()
    }

    /// Where `entry_id` sits in the rendered list, or `None` when it is not rendered.
    ///
    /// The application uses this to update one row instead of republishing the listing:
    /// a thumbnail that arrived must not cost a full model rebuild on a large folder.
    pub fn visible_index(&self, entry_id: u64) -> Option<usize> {
        self.visible_entries
            .iter()
            .position(|entry| entry.id.0 == entry_id)
    }

    /// Ajoute un batch d'entrées reçu du listing (streaming).
    pub fn append_batch(&mut self, batch: Vec<FileEntry>) {
        self.all_entries.extend(batch);
        self.rebuild_visible();
    }

    /// Réinitialise pour une nouvelle navigation.
    ///
    /// The thumbnail generation moves on here, so every in-flight fetch from the folder the
    /// user just left is discarded when it lands (Tranche 3 slice 4).
    pub fn clear(&mut self) {
        self.all_entries.clear();
        self.visible_entries.clear();
        self.selected_ids.clear();
        self.thumbnails.clear();
        self.thumbnails_requested.clear();
        self.thumbnail_generation = self.thumbnail_generation.wrapping_add(1);
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
            .map(|e| self.ui_row(e))
            .collect()
    }

    /// Une entrée, mise en forme comme dans `to_ui_rows`.
    ///
    /// One definition of a rendered row, so the single-row path cannot drift from the
    /// listing path.
    fn ui_row(&self, e: &FileEntry) -> UiFileRow {
        UiFileRow {
            id: e.id.0,
            display_name: e.display_name.clone(),
            kind: format_kind(&e.kind).to_string(),
            is_folder: matches!(e.kind, EntryKind::Directory),
            size_text: format_size(e.size),
            modified_text: format_modified(e.modified),
            type_text: format_kind(&e.kind).to_string(),
            selected: self.selected_ids.contains(&e.id.0),
            icon_key: e.icon_key,
            thumbnail: self.thumbnails.get(&e.id.0).cloned(),
        }
    }

    /// The rendered row for `entry_id` and its index, without building the whole list.
    ///
    /// A thumbnail arriving repaints exactly one row; materialising every row of a large
    /// folder for that would put a listing-sized allocation pass on the UI thread once per
    /// picture, which rule 4 does not allow. Returns `None` when the entry is not rendered
    /// (filtered out, or from another listing).
    pub fn ui_row_by_id(&self, entry_id: u64) -> Option<(usize, UiFileRow)> {
        let index = self.visible_index(entry_id)?;
        let entry = self.visible_entries.get(index)?;
        Some((index, self.ui_row(entry)))
    }

    /// Rows in view order with what the OLE drop target needs (Tranche 3 slice 2):
    /// the id, the filesystem path and whether the row can receive a drop. The order
    /// matches `to_ui_rows`, which is the order the list renders.
    pub fn visible_drop_rows(&self) -> Vec<(u64, std::path::PathBuf, bool)> {
        self.visible_entries
            .iter()
            .map(|e| {
                (
                    e.id.0,
                    e.path.clone(),
                    matches!(e.kind, EntryKind::Directory),
                )
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

    /// Whether `entry_id` is a folder, decided from the entry kind and not from the
    /// wording of the kind column.
    pub fn entry_is_dir(&self, id: u64) -> Option<bool> {
        self.all_entries
            .iter()
            .find(|e| e.id.0 == id)
            .map(|e| matches!(e.kind, EntryKind::Directory))
    }

    /// Seed data for the inline rename editor: the display name and whether the entry is
    /// a folder (folders have no extension to protect from the selection).
    pub fn rename_seed(&self, id: u64) -> Option<(String, bool)> {
        self.all_entries.iter().find(|e| e.id.0 == id).map(|e| {
            (
                e.display_name.clone(),
                matches!(e.kind, EntryKind::Directory),
            )
        })
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

    /// Returns filesystem paths of all currently selected visible entries.
    pub fn selected_paths(&self) -> Vec<std::path::PathBuf> {
        self.visible_entries
            .iter()
            .filter(|e| self.selected_ids.contains(&e.id.0))
            .map(|e| e.path.clone())
            .collect()
    }

    /// Sum of `size` for all selected visible file entries (directories are excluded).
    pub fn selected_bytes(&self) -> u64 {
        self.visible_entries
            .iter()
            .filter(|e| {
                self.selected_ids.contains(&e.id.0)
                    && matches!(e.kind, fx_domain::entry::EntryKind::File)
            })
            .filter_map(|e| e.size)
            .sum()
    }

    /// Returns the id of the entry currently being renamed inline, if any.
    pub fn rename_target(&self) -> Option<u64> {
        self.rename_target
    }

    /// Sets or clears the rename target. `None` means no rename is in progress.
    pub fn set_rename_target(&mut self, id: Option<u64>) {
        self.rename_target = id;
    }

    /// First selected entry in list order — the inline-rename target for F2.
    pub fn first_selected_id(&self) -> Option<u64> {
        self.visible_entries
            .iter()
            .find(|e| self.selected_ids.contains(&e.id.0))
            .map(|e| e.id.0)
    }

    /// Whether the entry is currently selected (drag payload rule).
    pub fn is_selected(&self, id: u64) -> bool {
        self.selected_ids.contains(&id)
    }

    /// Swap the selection to exactly the visible entries not currently selected.
    pub fn invert_selection(&mut self) {
        let flipped: std::collections::HashSet<u64> = self
            .visible_entries
            .iter()
            .map(|e| e.id.0)
            .filter(|id| !self.selected_ids.contains(id))
            .collect();
        self.selected_ids = flipped;
    }

    /// Case-insensitive `*`/`?` glob over display names (OPS-11 selection by
    /// pattern). Returns how many visible entries matched; a zero match keeps the
    /// current selection untouched.
    pub fn select_by_pattern(&mut self, pattern: &str) -> usize {
        let matched: std::collections::HashSet<u64> = self
            .visible_entries
            .iter()
            .filter(|e| glob_match(pattern, &e.display_name))
            .map(|e| e.id.0)
            .collect();
        let count = matched.len();
        if count > 0 {
            self.selected_ids = matched;
        }
        count
    }
}

/// Minimal case-insensitive glob: `*` any run, `?` exactly one character.
fn glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.to_lowercase().chars().collect();
    let txt: Vec<char> = text.to_lowercase().chars().collect();
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star, mut star_t) = (usize::MAX, 0usize);
    while t < txt.len() {
        if p < pat.len() && (pat[p] == '?' || pat[p] == txt[t]) {
            p += 1;
            t += 1;
        } else if p < pat.len() && pat[p] == '*' {
            star = p;
            p += 1;
            star_t = t;
        } else if star != usize::MAX {
            p = star + 1;
            star_t += 1;
            t = star_t;
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == '*' {
        p += 1;
    }
    p == pat.len()
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
        EntryKind::Directory => crate::strings::KIND_FOLDER,
        EntryKind::File => crate::strings::KIND_FILE,
        EntryKind::Symlink => crate::strings::KIND_SYMLINK,
        EntryKind::ReparsePoint => crate::strings::KIND_REPARSE_POINT,
    }
}

/// How many **UTF-8 bytes** of `name` the inline rename editor pre-selects — the unit
/// `TextInput.set-selection-offsets` takes.
///
/// Explorer selects the base name so typing replaces it while the extension stays;
/// a leading dot is part of the name (`.gitignore`), not an extension separator.
pub fn rename_selection(name: &str, is_folder: bool) -> usize {
    if is_folder {
        return name.len();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => dot,
        _ => name.len(),
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

/// A compact date for the Modified column: `2023-11-14 22:13`.
///
/// The conversion is in UTC: `std` exposes no local offset, and this crate may not call
/// Win32. A timezone-aware date belongs in the shell probe that also reads the file time;
/// this is the single place that would change.
fn format_modified(modified: Option<std::time::SystemTime>) -> String {
    let Some(modified) = modified else {
        return String::new();
    };
    let Ok(since_epoch) = modified.duration_since(std::time::UNIX_EPOCH) else {
        return String::new();
    };
    let secs = since_epoch.as_secs() as i64;
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let time = secs.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        time / 3_600,
        (time % 3_600) / 60
    )
}

/// Days since 1970-01-01 to a Gregorian `(year, month, day)`.
///
/// Howard Hinnant's `civil_from_days`: exact for every date the file system can report,
/// and it needs no calendar crate on the UI layer.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
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

    /// A 1x1 red pixel, the smallest thing a row can be given as a thumbnail.
    fn sample_pixels() -> std::sync::Arc<ThumbnailPixels> {
        std::sync::Arc::new(ThumbnailPixels {
            width: 1,
            height: 1,
            pixels: vec![255, 0, 0, 255],
        })
    }

    /// A picture and a folder, the two shapes the thumbnail request filter has to tell apart.
    fn sample_listing() -> FileListViewModel {
        let mut vm = FileListViewModel::new();
        vm.append_batch(vec![
            make_entry(1, "photo.JPG", EntryKind::File),
            make_entry(2, "notes.txt", EntryKind::File),
            make_entry(3, "albums", EntryKind::Directory),
        ]);
        vm
    }

    /// Only image-like files are worth a Shell call: an extension match is case-insensitive,
    /// a folder name that looks like a picture is not a file, and no extension never matches.
    #[test]
    fn only_image_like_files_are_thumbnail_candidates() {
        assert!(is_thumbnail_candidate(Path::new(r"C:\pictures\a.JPG")));
        assert!(is_thumbnail_candidate(Path::new(r"C:\pictures\a.png")));
        assert!(is_thumbnail_candidate(Path::new(r"C:\clips\a.mp4")));
        assert!(!is_thumbnail_candidate(Path::new(r"C:\docs\a.txt")));
        assert!(!is_thumbnail_candidate(Path::new(r"C:\docs\README")));
    }

    /// The first request is the first visible image, folders and plain files skipped.
    #[test]
    fn the_next_request_picks_the_first_image_like_file() {
        let mut vm = sample_listing();
        let request = vm.next_thumbnail_request(64).expect("a candidate exists");
        assert_eq!(request.entry_id, 1);
        assert_eq!(request.size, 64);
        assert_eq!(request.generation, vm.thumbnail_generation());
        assert!(request.path.ends_with("photo.JPG"));
    }

    /// Once a row has its picture it is not requested again, and a listing with no images
    /// asks for nothing at all.
    #[test]
    fn a_filled_slot_is_not_requested_again() {
        let mut vm = sample_listing();
        let request = vm.next_thumbnail_request(64).expect("a candidate exists");
        assert!(vm.apply_thumbnail(request.generation, request.entry_id, sample_pixels()));
        assert!(vm.next_thumbnail_request(64).is_none());
        assert!(
            vm.visible_index(1).is_some(),
            "the row is in the rendered list"
        );
        assert!(vm.visible_index(42).is_none());
    }

    /// A result from the folder the user left is discarded: it must not paint one folder's
    /// picture onto another folder's row.
    #[test]
    fn a_result_from_a_previous_listing_is_discarded() {
        let mut vm = sample_listing();
        let stale = vm.next_thumbnail_request(64).expect("a candidate exists");

        vm.clear();
        vm.append_batch(vec![make_entry(7, "other.jpg", EntryKind::File)]);

        assert!(
            !vm.apply_thumbnail(stale.generation, 7, sample_pixels()),
            "a result carrying the old generation must be refused"
        );
        assert!(vm.to_ui_rows()[0].thumbnail.is_none());
    }

    /// A slot that is no longer visible is refused too: a filter that hid the row while its
    /// thumbnail was in flight must not resurrect it.
    #[test]
    fn a_result_for_a_row_that_left_the_list_is_discarded() {
        let mut vm = sample_listing();
        let request = vm.next_thumbnail_request(64).expect("a candidate exists");
        let request_id = request.entry_id;
        let request_generation = request.generation;
        vm.set_filter("no-such-name".to_string());

        assert!(!vm.apply_thumbnail(request_generation, request_id, sample_pixels()));
        assert!(vm.thumbnail(1).is_none());
    }

    /// A single repainted row is the row the full listing would build for it: the thumbnail
    /// path must not carry a second, drifting definition of a row.
    #[test]
    fn the_single_row_helper_matches_the_full_list() {
        let mut vm = sample_listing();
        let request = vm.next_thumbnail_request(64).expect("a candidate exists");
        assert!(vm.apply_thumbnail(request.generation, request.entry_id, sample_pixels()));

        let rows = vm.to_ui_rows();
        let (index, row) = vm
            .ui_row_by_id(request.entry_id)
            .expect("the row is rendered");
        assert_eq!(row.id, rows[index].id);
        assert_eq!(row.display_name, rows[index].display_name);
        assert_eq!(row.kind, rows[index].kind);
        assert_eq!(row.size_text, rows[index].size_text);
        assert_eq!(row.modified_text, rows[index].modified_text);
        assert_eq!(row.selected, rows[index].selected);
        assert!(
            row.thumbnail.is_some(),
            "the helper must carry the thumbnail"
        );
        assert!(vm.ui_row_by_id(42).is_none(), "an absent entry has no row");
    }

    /// An applied thumbnail reaches the rows the UI renders, and only its own row.
    #[test]
    fn an_applied_thumbnail_reaches_the_rendered_row() {
        let mut vm = sample_listing();
        let request = vm.next_thumbnail_request(64).expect("a candidate exists");
        assert!(vm.apply_thumbnail(request.generation, request.entry_id, sample_pixels()));

        let rows = vm.to_ui_rows();
        let row = rows
            .iter()
            .find(|row| row.id == request.entry_id)
            .expect("the requested row is rendered");
        assert!(row.thumbnail.is_some());
        assert!(
            rows.iter()
                .filter(|row| row.id != request.entry_id)
                .all(|row| row.thumbnail.is_none()),
            "no other row may inherit the image"
        );
    }

    /// The Modified column carries a date, not a raw epoch count.
    #[test]
    fn modified_is_shown_as_a_date() {
        use std::time::{Duration, UNIX_EPOCH};
        assert_eq!(format_modified(None), "");
        assert_eq!(format_modified(Some(UNIX_EPOCH)), "1970-01-01 00:00");
        assert_eq!(
            format_modified(Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000))),
            "2023-11-14 22:13"
        );
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

    #[test]
    fn selected_paths_returns_paths_of_selected_entries() {
        let mut vm = FileListViewModel::new();
        vm.append_batch(vec![
            make_entry(1, "a.txt", EntryKind::File),
            make_entry(2, "b.txt", EntryKind::File),
            make_entry(3, "c.txt", EntryKind::File),
        ]);
        vm.select_single(2);
        let paths = vm.selected_paths();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], PathBuf::from("b.txt"));
    }

    #[test]
    fn selected_bytes_sums_file_sizes() {
        let mut vm = FileListViewModel::new();
        let mut e1 = make_entry(1, "a.txt", EntryKind::File);
        e1.size = Some(100);
        let mut e2 = make_entry(2, "b.txt", EntryKind::File);
        e2.size = Some(200);
        let e3 = make_entry(3, "folder", EntryKind::Directory);
        vm.append_batch(vec![e1, e2, e3]);
        vm.select_all();
        // Directories have no size, so only files count.
        assert_eq!(vm.selected_bytes(), 300);
    }

    #[test]
    fn rename_target_roundtrip() {
        let mut vm = FileListViewModel::new();
        assert_eq!(vm.rename_target(), None);
        vm.set_rename_target(Some(42));
        assert_eq!(vm.rename_target(), Some(42));
        vm.set_rename_target(None);
        assert_eq!(vm.rename_target(), None);
    }

    #[test]
    fn first_selected_id_follows_list_order() {
        let mut vm = FileListViewModel::new();
        vm.append_batch(vec![
            make_entry(1, "a.txt", EntryKind::File),
            make_entry(2, "b.txt", EntryKind::File),
            make_entry(3, "c.txt", EntryKind::File),
        ]);
        vm.toggle_select(3);
        vm.toggle_select(2);
        assert_eq!(vm.first_selected_id(), Some(2));
    }

    #[test]
    fn first_selected_id_is_none_without_selection() {
        let mut vm = FileListViewModel::new();
        vm.append_batch(vec![make_entry(1, "a.txt", EntryKind::File)]);
        assert_eq!(vm.first_selected_id(), None);
    }

    #[test]
    fn invert_selection_swaps_to_the_unselected_visible_entries() {
        let mut vm = FileListViewModel::new();
        vm.append_batch(vec![
            make_entry(1, "a.txt", EntryKind::File),
            make_entry(2, "b.txt", EntryKind::File),
            make_entry(3, "c.txt", EntryKind::File),
        ]);
        vm.select_single(1);
        vm.invert_selection();
        let paths = vm.selected_paths();
        assert_eq!(paths, vec![PathBuf::from("b.txt"), PathBuf::from("c.txt")]);
        vm.invert_selection();
        assert_eq!(vm.selected_paths(), vec![PathBuf::from("a.txt")]);
    }

    #[test]
    fn select_by_pattern_is_case_insensitive_glob_on_display_name() {
        let mut vm = FileListViewModel::new();
        vm.append_batch(vec![
            make_entry(1, "Report.PDF", EntryKind::File),
            make_entry(2, "notes.txt", EntryKind::File),
            make_entry(3, "sub", EntryKind::Directory),
            make_entry(4, "cat.txt", EntryKind::File),
        ]);
        let matched = vm.select_by_pattern("*.txt");
        assert_eq!(matched, 2);
        assert_eq!(vm.selected_paths().len(), 2);
        assert_eq!(vm.select_by_pattern("rePort.pd?"), 1);
        assert_eq!(vm.select_by_pattern("no-match"), 0);
    }

    #[test]
    fn rename_seed_gives_the_display_name_and_folder_flag() {
        let mut vm = FileListViewModel::new();
        vm.append_batch(vec![
            make_entry(1, "notes.txt", EntryKind::File),
            make_entry(2, "sub", EntryKind::Directory),
        ]);
        assert_eq!(vm.rename_seed(1), Some(("notes.txt".to_string(), false)));
        assert_eq!(vm.rename_seed(2), Some(("sub".to_string(), true)));
        assert_eq!(vm.rename_seed(99), None);
    }

    #[test]
    fn rename_selection_covers_the_base_name_only() {
        assert_eq!(rename_selection("report.pdf", false), 6);
        // Archives keep the last extension out of the selection, like Explorer.
        assert_eq!(rename_selection("archive.tar.gz", false), 11);
        // Dotfiles have no base name to protect.
        assert_eq!(rename_selection(".gitignore", false), 10);
        assert_eq!(rename_selection("noextension", false), 11);
        assert_eq!(rename_selection("folder", true), 6);
        // The editor takes UTF-8 byte offsets: "café" is 5 bytes, not 4 characters.
        assert_eq!(rename_selection("café.txt", false), 5);
    }
}
