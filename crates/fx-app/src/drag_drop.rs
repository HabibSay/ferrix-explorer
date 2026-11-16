//! Tranche 3, slice 2: the application side of the OLE drag-and-drop.
//!
//! Two directions live here:
//!
//! * **Dropping into Ferrix.** Explorer (or any other OLE application) drops files on our
//!   window; `fx-shell/drop_target.rs` reads the payload and hands it to the [`DropSink`]
//!   implemented by this module. The sink resolves *where* the drop landed, decides the
//!   effect, and hands the work to the [`DropHost`], so no file I/O and no orchestrator
//!   lock happens inside the shell's modal drag loop (rule 4).
//!
//! * **Dragging out of Ferrix.** The UI reports that a row drag crossed the threshold; the
//!   host builds the payload from the current selection and calls [`fx_shell::run_drag`],
//!   which runs `DoDragDrop`. A drop back onto our own window then arrives through the
//!   same [`DropSink`] path, which is what decision 8 asks for: OLE is the single transport.
//!
//! The geometry the hit-test needs (the list viewport position in window coordinates, its
//! height, the row height and the scroll offset) is *published by the UI* rather than
//! queried: the OLE callbacks run inside the shell's own modal loop, so asking the UI at
//! that moment could re-enter a callback that already holds the orchestrator's lock.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use fx_ops::drag::{DropAction, DropModifiers, is_same_folder_drop, resolve_action};
use fx_ops::port::PlatformOps;
use fx_shell::{DragEffect, DragPayload, DropOutcome, DropSink};

use crate::platform_ops::WindowsPlatformOps;
use crate::strings;

/// Where the file list sits, in the window's own coordinate space.
///
/// Published by the UI in **logical** pixels relative to the window origin, which is the
/// space the OLE drop point is converted into before it is compared against this.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ListGeometry {
    /// Top of the file list viewport, in logical pixels from the window origin.
    pub viewport_top: f32,
    /// Height of the file list viewport, in logical pixels.
    pub viewport_height: f32,
    /// Height of one row, in logical pixels.
    pub row_height: f32,
    /// Vertical scroll offset of the list, in logical pixels. Positive when scrolled down.
    pub scroll_offset: f32,
    /// Number of rows the list currently shows.
    pub row_count: usize,
}

/// One row of the file list, as the hit-test sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct ListRow {
    /// Identifier of the entry, matching the view model.
    pub id: u64,
    /// Filesystem path of the entry.
    pub path: PathBuf,
    /// Whether the entry is a directory (only a directory can receive a drop).
    pub is_dir: bool,
}

/// Everything the drop sink needs to resolve a drop point.
///
/// The UI keeps this current: the rows whenever the list is pushed, the geometry
/// whenever the viewport or the window changes. The OLE callbacks only ever read it.
#[derive(Clone, Debug)]
pub struct DropContext {
    /// Geometry of the file list.
    pub geometry: ListGeometry,
    /// The rows, in the order the UI shows them.
    pub rows: Vec<ListRow>,
    /// The folder the list currently shows; the drop target of the empty list area.
    pub background: Option<PathBuf>,
    /// Window scale factor: physical pixels per logical pixel. The shell hands the drop
    /// point in physical pixels, the geometry is published in logical ones.
    pub scale_factor: f32,
}

impl Default for DropContext {
    fn default() -> Self {
        Self {
            geometry: ListGeometry::default(),
            rows: Vec::new(),
            background: None,
            scale_factor: 1.0,
        }
    }
}

impl DropContext {
    /// Replaces the rows and the folder they belong to.
    pub fn set_rows(&mut self, rows: Vec<ListRow>, background: Option<PathBuf>) {
        self.geometry.row_count = rows.len();
        self.rows = rows;
        self.background = background;
    }

    /// Replaces the geometry and the scale factor, keeping the row list.
    pub fn set_geometry(&mut self, geometry: ListGeometry, scale_factor: f32) {
        let row_count = self.rows.len();
        self.geometry = ListGeometry {
            row_count,
            ..geometry
        };
        self.set_scale_factor(scale_factor);
    }

    /// Updates the window scale factor: physical pixels per logical pixel.
    ///
    /// The window can move to a monitor with another scale factor without the list
    /// geometry changing, so the factor is refreshed while the app runs.
    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        self.scale_factor = if scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
    }
}

/// The row under a client-space point, or `None` when the point is outside the list.
///
/// Pure arithmetic on the published geometry: the client point is in logical pixels relative
/// to the window origin, the rows start at `viewport_top`, and the list is scrolled by
/// `scroll_offset`. A point on a row that is not a directory is *not* a drop target, so it
/// yields `None` as well — Explorer refuses to drop onto a file.
pub fn row_at(context: &DropContext, client_y: f32) -> Option<&ListRow> {
    let geometry = context.geometry;
    if geometry.row_height <= 0.0 || client_y < geometry.viewport_top {
        return None;
    }
    let offset = client_y - geometry.viewport_top + geometry.scroll_offset;
    let index = (offset / geometry.row_height).floor();
    if index < 0.0 {
        return None;
    }
    let row = context.rows.get(index as usize)?;
    if row.is_dir { Some(row) } else { None }
}

/// Converts a shell drop point into the logical window coordinate the geometry uses.
///
/// The shell hands the cursor in physical pixels; the list geometry is published in
/// logical ones. A missing or broken scale factor falls back to 1.0 rather than
/// dividing by zero.
pub fn logical_y(point: (i32, i32), scale_factor: f32) -> f32 {
    let scale = if scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    point.1 as f32 / scale
}

/// Where a drop point lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropTarget {
    /// The folder the drop would go into: a folder row, or the list background.
    Folder(PathBuf),
    /// Inside the list, but there is no local folder behind it — the view is a location
    /// that cannot receive drops (spec §1: refuse with a message, never redirect).
    Unsupported,
    /// Outside the list, or over a file: not a target at all.
    Refused,
}

/// The folder a drop at `client_y` lands in, or `None` when the point is not a target.
///
/// Three cases: a folder row receives the drop, the empty area of the list receives it in
/// the folder the list shows (Explorer's background drop), and everything else — outside
/// the viewport, or over a file row — refuses it, which is what makes the shell show its
/// "no" cursor.
pub fn target_at(context: &DropContext, client_y: f32) -> DropTarget {
    let geometry = context.geometry;
    if geometry.row_height <= 0.0
        || client_y < geometry.viewport_top
        || client_y >= geometry.viewport_top + geometry.viewport_height
    {
        return DropTarget::Refused;
    }
    match row_at(context, client_y) {
        // A folder row takes the drop itself.
        Some(row) => DropTarget::Folder(row.path.clone()),
        // `row_at` returns `None` either for a file row or for a point past the last row.
        None if index_at(context, client_y).is_some() => DropTarget::Refused,
        None => match &context.background {
            Some(folder) => DropTarget::Folder(folder.clone()),
            None => DropTarget::Unsupported,
        },
    }
}

/// The row index a point falls on, when it falls on one at all.
fn index_at(context: &DropContext, client_y: f32) -> Option<usize> {
    let geometry = context.geometry;
    if geometry.row_height <= 0.0 || client_y < geometry.viewport_top {
        return None;
    }
    let offset = client_y - geometry.viewport_top + geometry.scroll_offset;
    let index = (offset / geometry.row_height).floor();
    if index < 0.0 {
        return None;
    }
    let index = index as usize;
    (index < context.rows.len()).then_some(index)
}

/// The effect a drop on `destination` would perform, given the modifier keys and the
/// source's preference.
///
/// The default (no modifier) follows the Tranche 2 rule: a move inside one volume, a copy
/// across volumes. The `Ctrl`/`Shift`/`Ctrl+Shift` overrides are Explorer's.
pub fn effect_for(
    modifiers: DropModifiers,
    preferred: DropAction,
    same_volume: bool,
) -> DropAction {
    resolve_action(modifiers, preferred, same_volume)
}

/// Maps an effect onto the shell's vocabulary.
pub fn shell_effect(action: DropAction) -> DragEffect {
    match action {
        DropAction::Copy => DragEffect::Copy,
        DropAction::Move => DragEffect::Move,
        DropAction::Shortcut => DragEffect::Shortcut,
    }
}

/// Maps the shell's vocabulary onto the policy vocabulary.
pub fn policy_effect(effect: DragEffect) -> DropAction {
    match effect {
        DragEffect::Copy => DropAction::Copy,
        DragEffect::Move => DropAction::Move,
        DragEffect::Shortcut => DropAction::Shortcut,
    }
}

/// The shell's reading of the modifier keys, in the policy vocabulary.
pub fn shell_modifiers() -> DropModifiers {
    let shell = fx_shell::DropModifiers::current();
    DropModifiers {
        control: shell.control,
        shift: shell.shift,
    }
}

/// What the sink hands to the host once a drop is resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedDrop {
    /// The dropped paths.
    pub paths: Vec<PathBuf>,
    /// The folder the files land in.
    pub destination: PathBuf,
    /// The effect the drop performs.
    pub action: DropAction,
}

/// The host's side of a drop: accepting the work and telling the user what happened.
///
/// The sink runs inside the shell's modal drag loop, so [`Self::submit`] must not block on
/// the orchestrator's lock (the context menu shows its popup from inside that lock, and the
/// same thread could be re-entered by this very drop). The implementation therefore only
/// pushes the resolved drop onto a channel; the dispatcher task takes the lock and runs the
/// transfer.
pub trait DropHost: Send + Sync {
    /// Hands a resolved drop over; `false` means the drop could not be accepted.
    fn submit(&self, drop: &ResolvedDrop) -> bool;

    /// Shows `message` on the status line, without taking any lock.
    fn announce(&self, message: &'static str);
}

/// The application side of the OLE drop target.
///
/// It holds the published [`DropContext`], owns the policy call (modifier keys, volume
/// comparison) and delegates acceptance to the [`DropHost`].
pub struct FerrixDropSink {
    context: Arc<Mutex<DropContext>>,
    port: Arc<WindowsPlatformOps>,
    host: Arc<dyn DropHost>,
    /// Reads the modifier keys that are held right now.
    modifiers: Box<dyn Fn() -> DropModifiers + Send + Sync>,
    /// Last `(source, destination)` volume comparison, so `DragOver` stays cheap.
    volume_memo: Mutex<Option<(PathBuf, PathBuf, bool)>>,
}

impl FerrixDropSink {
    /// Builds a sink over the published context.
    ///
    /// `modifiers` reads the modifier keys that are held right now: the shell's
    /// `GetAsyncKeyState` in production, a fixed set in the tests.
    pub fn new(
        context: Arc<Mutex<DropContext>>,
        port: Arc<WindowsPlatformOps>,
        host: Arc<dyn DropHost>,
        modifiers: impl Fn() -> DropModifiers + Send + Sync + 'static,
    ) -> Self {
        Self {
            context,
            port,
            host,
            modifiers: Box::new(modifiers),
            volume_memo: Mutex::new(None),
        }
    }

    /// Where a shell drop point lands, in logical window coordinates.
    fn target_at(&self, point: (i32, i32)) -> DropTarget {
        match self.context.lock() {
            Ok(context) => {
                let client_y = logical_y(point, context.scale_factor);
                target_at(&context, client_y)
            }
            // A context that cannot be read is not a target; refusing is the safe answer.
            Err(_) => DropTarget::Refused,
        }
    }

    /// What the drop would do, given the live modifier keys and the source's preference.
    fn action_for(&self, payload: &DragPayload, destination: &Path) -> DropAction {
        // The source advertises its preference in the shell's vocabulary; the policy
        // speaks the operations' vocabulary.
        let preferred = policy_effect(payload.preferred);
        let first = match payload.paths.first() {
            Some(first) => first,
            None => return preferred,
        };
        let same_volume = self.same_volume(first, destination);
        effect_for((self.modifiers)(), preferred, same_volume)
    }

    /// Whether the source and the destination live on the same volume.
    ///
    /// Two volume serial reads on the first drop point of a pair, then the answer is
    /// remembered: `DragOver` fires continuously while the pointer moves and must stay
    /// cheap (spec §7).
    fn same_volume(&self, source: &Path, destination: &Path) -> bool {
        if let Ok(memo) = self.volume_memo.lock()
            && let Some((memo_source, memo_destination, memo_volume)) = memo.as_ref()
            && memo_source == source
            && memo_destination == destination
        {
            return *memo_volume;
        }
        // An unidentifiable volume is treated as one volume, which is the behaviour the
        // Tranche 2 drop path already had.
        let volume = match (
            self.port.volume_id(source),
            self.port.volume_id(destination),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => true,
        };
        if let Ok(mut memo) = self.volume_memo.lock() {
            *memo = Some((source.to_path_buf(), destination.to_path_buf(), volume));
        }
        volume
    }
}

impl DropSink for FerrixDropSink {
    fn effect_at(&self, payload: &DragPayload, point: (i32, i32)) -> Option<DragEffect> {
        if payload.paths.is_empty() {
            return None;
        }
        let destination = match self.target_at(point) {
            DropTarget::Folder(destination) => destination,
            DropTarget::Unsupported | DropTarget::Refused => return None,
        };
        let action = self.action_for(payload, &destination);
        // Dropping files back onto the folder they already live in does nothing: the
        // shell shows its "no" cursor instead of offering a transfer that would refuse.
        if is_no_op_drop(&payload.paths, &destination, action) {
            return None;
        }
        Some(shell_effect(action))
    }

    fn on_drop(&self, payload: DragPayload, point: (i32, i32)) -> DropOutcome {
        if payload.paths.is_empty() {
            return DropOutcome::Rejected;
        }
        let destination = match self.target_at(point) {
            DropTarget::Folder(destination) => destination,
            DropTarget::Unsupported => {
                // Refuse with a message, never silently (spec §1).
                self.host.announce(strings::STATUS_NEEDS_LOCAL_FOLDER);
                return DropOutcome::Rejected;
            }
            DropTarget::Refused => return DropOutcome::Rejected,
        };
        let action = self.action_for(&payload, &destination);
        if is_no_op_drop(&payload.paths, &destination, action) {
            self.host.announce(strings::STATUS_DROP_IN_SOURCE);
            return DropOutcome::Rejected;
        }
        let resolved = ResolvedDrop {
            paths: payload.paths,
            destination,
            action,
        };
        if self.host.submit(&resolved) {
            DropOutcome::Executed {
                effect: shell_effect(action),
            }
        } else {
            DropOutcome::Rejected
        }
    }
}

/// Whether dropping `paths` into `destination` would be a no-op.
///
/// Tranche 2 rule 15: copying or moving rows back into the folder they came from does
/// nothing, and the user is told so instead of seeing a no-op transfer. A shortcut is the
/// exception — a link inside the source folder is a real, different file.
pub fn is_no_op_drop(paths: &[PathBuf], destination: &Path, action: DropAction) -> bool {
    if action == DropAction::Shortcut {
        return false;
    }
    paths
        .first()
        .and_then(|path| path.parent())
        .is_some_and(|parent| is_same_folder_drop(Some(parent), destination))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sample list: two folders and a file, in a 200-pixel viewport.
    fn sample_context() -> DropContext {
        DropContext {
            geometry: ListGeometry {
                viewport_top: 100.0,
                viewport_height: 200.0,
                row_height: 22.0,
                scroll_offset: 0.0,
                row_count: 3,
            },
            rows: vec![
                ListRow {
                    id: 1,
                    path: PathBuf::from(r"C:\a"),
                    is_dir: true,
                },
                ListRow {
                    id: 2,
                    path: PathBuf::from(r"C:\b.txt"),
                    is_dir: false,
                },
                ListRow {
                    id: 3,
                    path: PathBuf::from(r"C:\c"),
                    is_dir: true,
                },
            ],
            background: Some(PathBuf::from(r"C:\current")),
            scale_factor: 1.0,
        }
    }

    /// The folder a point resolves to, or `None` when the point is not a folder target.
    fn folder_at(context: &DropContext, client_y: f32) -> Option<PathBuf> {
        match target_at(context, client_y) {
            DropTarget::Folder(folder) => Some(folder),
            DropTarget::Unsupported | DropTarget::Refused => None,
        }
    }

    #[test]
    fn a_point_on_a_folder_row_resolves_that_folder() {
        let context = sample_context();
        assert_eq!(
            row_at(&context, 101.0).map(|row| row.path.clone()),
            Some(PathBuf::from(r"C:\a"))
        );
        assert_eq!(
            row_at(&context, 145.0).map(|row| row.path.clone()),
            Some(PathBuf::from(r"C:\c"))
        );
        assert_eq!(folder_at(&context, 145.0), Some(PathBuf::from(r"C:\c")));
    }

    #[test]
    fn a_point_on_a_file_row_is_not_a_drop_target() {
        let context = sample_context();
        assert!(row_at(&context, 122.0).is_none());
        assert_eq!(target_at(&context, 122.0), DropTarget::Refused);
    }

    #[test]
    fn a_point_above_or_below_the_list_is_not_a_drop_target() {
        let context = sample_context();
        assert!(row_at(&context, 99.0).is_none());
        assert_eq!(target_at(&context, 99.0), DropTarget::Refused);
        assert_eq!(target_at(&context, 300.0), DropTarget::Refused);
    }

    #[test]
    fn empty_list_area_drops_into_the_folder_the_list_shows() {
        let context = sample_context();
        // Past the last row (3 rows end at 166) but still inside the viewport.
        assert_eq!(
            folder_at(&context, 170.0),
            Some(PathBuf::from(r"C:\current"))
        );
        // An empty list is all background.
        let mut empty = sample_context();
        empty.rows.clear();
        empty.geometry.row_count = 0;
        assert_eq!(folder_at(&empty, 150.0), Some(PathBuf::from(r"C:\current")));
    }

    #[test]
    fn a_view_without_a_local_folder_is_refused_and_says_so() {
        let mut context = sample_context();
        context.background = None;
        // The list area is inside the view, but there is no local folder behind it: the
        // sink turns this into the "needs a local folder" message rather than silence.
        assert_eq!(target_at(&context, 170.0), DropTarget::Unsupported);
        // A folder row still resolves — the row carries its own path.
        assert_eq!(
            target_at(&context, 101.0),
            DropTarget::Folder(PathBuf::from(r"C:\a"))
        );
    }

    #[test]
    fn scrolling_shifts_the_row_under_the_point() {
        let mut context = sample_context();
        context.geometry.scroll_offset = 22.0;
        // The first row scrolled out of view: the point that used to be on row 1 now
        // falls on row 2, which is a file — so the drop is refused there.
        assert_eq!(target_at(&context, 101.0), DropTarget::Refused);
        // Row 3 now sits where row 2 used to be.
        assert_eq!(folder_at(&context, 123.0), Some(PathBuf::from(r"C:\c")));
    }

    #[test]
    fn a_physical_shell_point_is_converted_with_the_window_scale() {
        let context = sample_context();
        // The shell hands physical pixels; 290 physical pixels at scale 2 is 145 logical.
        let y = logical_y((0, 290), 2.0);
        assert_eq!(y, 145.0);
        assert_eq!(folder_at(&context, y), Some(PathBuf::from(r"C:\c")));
        // A zero or broken scale factor must not divide by zero.
        assert_eq!(logical_y((0, 290), 0.0), 290.0);
    }

    #[test]
    fn the_effect_follows_the_modifier_keys() {
        let none = DropModifiers::default();
        assert_eq!(effect_for(none, DropAction::Copy, true), DropAction::Copy);
        assert_eq!(effect_for(none, DropAction::Move, true), DropAction::Move);
        // A move across volumes degrades to a copy, as in Tranche 2.
        assert_eq!(effect_for(none, DropAction::Move, false), DropAction::Copy);
    }

    #[test]
    fn dropping_into_the_source_folder_is_a_no_op() {
        let paths = vec![PathBuf::from(r"C:\a\file.txt")];
        assert!(is_no_op_drop(&paths, Path::new(r"C:\a"), DropAction::Copy));
        assert!(is_no_op_drop(&paths, Path::new(r"C:\a"), DropAction::Move));
        assert!(!is_no_op_drop(&paths, Path::new(r"C:\b"), DropAction::Copy));
        // A shortcut into the source folder is a new file, not a no-op.
        assert!(!is_no_op_drop(
            &paths,
            Path::new(r"C:\a"),
            DropAction::Shortcut
        ));
    }

    #[test]
    fn effects_map_between_the_two_vocabularies() {
        for action in [DropAction::Copy, DropAction::Move, DropAction::Shortcut] {
            assert_eq!(policy_effect(shell_effect(action)), action);
        }
    }

    // ── The sink itself ─────────────────────────────────────────────────────

    /// A host that records what the sink hands over.
    #[derive(Default)]
    struct RecordingHost {
        submitted: Mutex<Vec<ResolvedDrop>>,
        announced: Mutex<Vec<&'static str>>,
    }

    impl DropHost for RecordingHost {
        fn submit(&self, drop: &ResolvedDrop) -> bool {
            self.submitted.lock().expect("host lock").push(drop.clone());
            true
        }

        fn announce(&self, message: &'static str) {
            self.announced.lock().expect("host lock").push(message);
        }
    }

    /// A sink over `context`, with the recording host it hands work to.
    fn sink_over(context: DropContext) -> (FerrixDropSink, Arc<RecordingHost>) {
        let host = Arc::new(RecordingHost::default());
        let sink = FerrixDropSink::new(
            Arc::new(Mutex::new(context)),
            Arc::new(WindowsPlatformOps),
            host.clone(),
            // No modifier key is held: the negotiation is deterministic.
            DropModifiers::default,
        );
        (sink, host)
    }

    /// A payload carrying one file.
    fn payload() -> DragPayload {
        DragPayload {
            paths: vec![PathBuf::from(r"C:\dropped.txt")],
            preferred: DragEffect::Copy,
        }
    }

    #[test]
    fn a_drop_on_a_folder_row_is_handed_over() {
        let (sink, host) = sink_over(sample_context());
        let outcome = sink.on_drop(payload(), (0, 101));
        assert!(matches!(outcome, DropOutcome::Executed { .. }));
        let submitted = host.submitted.lock().expect("host lock");
        assert_eq!(submitted.len(), 1);
        assert_eq!(submitted[0].destination, PathBuf::from(r"C:\a"));
        assert_eq!(submitted[0].paths, vec![PathBuf::from(r"C:\dropped.txt")]);
        // No modifier and a copy-preferring source: the drop copies.
        assert_eq!(submitted[0].action, DropAction::Copy);
    }

    #[test]
    fn a_drop_on_a_file_row_is_refused_without_a_message() {
        let (sink, host) = sink_over(sample_context());
        let outcome = sink.on_drop(payload(), (0, 122));
        assert_eq!(outcome, DropOutcome::Rejected);
        assert!(host.submitted.lock().expect("host lock").is_empty());
        assert!(host.announced.lock().expect("host lock").is_empty());
    }

    #[test]
    fn a_drop_outside_the_list_is_refused() {
        let (sink, host) = sink_over(sample_context());
        assert_eq!(sink.on_drop(payload(), (0, 99)), DropOutcome::Rejected);
        assert_eq!(sink.on_drop(payload(), (0, 400)), DropOutcome::Rejected);
        assert!(host.submitted.lock().expect("host lock").is_empty());
    }

    #[test]
    fn a_drop_in_a_view_without_a_local_folder_says_so() {
        let mut context = sample_context();
        context.background = None;
        let (sink, host) = sink_over(context);
        // The empty area of the list has no local folder behind it: the drop is
        // refused and the user is told why (spec §1).
        assert_eq!(sink.on_drop(payload(), (0, 170)), DropOutcome::Rejected);
        assert!(host.submitted.lock().expect("host lock").is_empty());
        assert_eq!(
            *host.announced.lock().expect("host lock"),
            vec![strings::STATUS_NEEDS_LOCAL_FOLDER]
        );
    }

    #[test]
    fn a_drop_into_the_source_folder_is_refused_with_a_message() {
        let mut context = sample_context();
        // The list shows `C:\a`, and the dropped file lives in it.
        context.rows = vec![ListRow {
            id: 1,
            path: PathBuf::from(r"C:\a"),
            is_dir: true,
        }];
        context.background = Some(PathBuf::from(r"C:\a"));
        let (sink, host) = sink_over(context);
        let payload = DragPayload {
            paths: vec![PathBuf::from(r"C:\a\file.txt")],
            preferred: DragEffect::Copy,
        };
        assert_eq!(sink.on_drop(payload, (0, 101)), DropOutcome::Rejected);
        assert!(host.submitted.lock().expect("host lock").is_empty());
        assert_eq!(
            *host.announced.lock().expect("host lock"),
            vec![strings::STATUS_DROP_IN_SOURCE]
        );
    }

    #[test]
    fn the_effect_is_refused_where_the_drop_is_refused() {
        let (sink, _host) = sink_over(sample_context());
        // On a folder row: an effect.
        assert_eq!(sink.effect_at(&payload(), (0, 101)), Some(DragEffect::Copy));
        // On a file row and outside the list: no effect, so the shell shows its
        // "no" cursor.
        assert_eq!(sink.effect_at(&payload(), (0, 122)), None);
        assert_eq!(sink.effect_at(&payload(), (0, 400)), None);
    }

    #[test]
    fn the_modifier_keys_change_the_negotiated_effect() {
        let host = Arc::new(RecordingHost::default());
        let sink = FerrixDropSink::new(
            Arc::new(Mutex::new(sample_context())),
            Arc::new(WindowsPlatformOps),
            host.clone(),
            || DropModifiers {
                control: true,
                shift: true,
            },
        );
        // Ctrl+Shift creates a shortcut (13.9).
        assert_eq!(
            sink.effect_at(&payload(), (0, 101)),
            Some(DragEffect::Shortcut)
        );
    }

    #[test]
    fn a_payload_without_files_is_refused() {
        let (sink, host) = sink_over(sample_context());
        let empty = DragPayload {
            paths: Vec::new(),
            preferred: DragEffect::Copy,
        };
        assert_eq!(sink.effect_at(&empty, (0, 101)), None);
        assert_eq!(sink.on_drop(empty, (0, 101)), DropOutcome::Rejected);
        assert!(host.submitted.lock().expect("host lock").is_empty());
    }
}
