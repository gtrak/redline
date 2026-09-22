//! The `Snapshot` read model: one render's worth of store state,
//! extracted as owned values so the `Mutex` guard can be dropped
//! before the element tree is built.

use std::sync::{Arc, Mutex};

use iocraft::hooks::State;

use crate::app::store::AppStore;
use crate::app::store::jump_highlight_intensity;
use crate::app::store::{BufferRow, DirtyCounts, FileViewRow, PickerCandidate, ResultRow, TransientMenuRow, ViewId};
use crate::model::sections::MagitRow;
/// One render's worth of store state, extracted as owned values so the
/// `Mutex` guard can be dropped before the element tree is built.
pub(super) struct Snapshot {
    pub(super) quit: bool,
    pub(super) project: String,
    pub(super) view: ViewId,
    pub(super) view_name: String,
    pub(super) pending: String,
    pub(super) activity: String,
    pub(super) message: String,
    pub(super) home_title: String,
    pub(super) home_rows: Vec<TransientMenuRow>,
    pub(super) buffer_rows: Vec<BufferRow>,
    pub(super) buffer_list_selected: usize,
    pub(super) picker: bool,
    pub(super) prompt: String,
    pub(super) query: String,
    pub(super) selected: usize,
    pub(super) candidates: Vec<PickerCandidate>,
    pub(super) total: usize,
    pub(super) preview: String,
    pub(super) magit_rows: Vec<MagitRow>,
    pub(super) magit_top_row: usize,
    pub(super) magit_total_rows: usize,
    // issue 002: transient menu overlay
    pub(super) menu_open: bool,
    pub(super) menu_rows: Vec<TransientMenuRow>,
    pub(super) menu_height: u32,
    // issue 08: log / blame / commit-diff / commit editor
    pub(super) log_title: String,
    pub(super) log_rows: Vec<MagitRow>,
    pub(super) blame_title: String,
    pub(super) blame_rows: Vec<MagitRow>,
    pub(super) commit_diff_title: String,
    pub(super) commit_diff_rows: Vec<MagitRow>,
    pub(super) commit_diff_top_row: usize,
    pub(super) commit_diff_total_rows: usize,
    pub(super) commit_editor_title: String,
    pub(super) commit_editor_rows: Vec<MagitRow>,
    pub(super) dirty: Option<DirtyCounts>,
    // File view (issue 03).
    /// The pre-computed rendered rows: code rows + the virtual annotation
    /// note rows (plan 005 issue 02 — the notes render ABOVE their anchored
    /// code rows, annotations-render-fold). Each row carries its buffer-line
    /// index; the cursor/click math translate through them.
    pub(super) file_view_rows: Vec<FileViewRow>,
    /// The total number of rendered rows for the buffer (canvas bottom
    /// scroll indicator; plan 005 issue 02).
    pub(super) file_view_total_rows: usize,
    pub(super) file_view_title: String,
    pub(super) file_view_top_line: usize,
    pub(super) file_view_total_lines: usize,
    pub(super) file_view_viewport_lines: usize,
    // annotations-render-fold: the note blocks are folded away (the
    // annotated-line margin indicator carries the state — the thin-bar
    // marker instead of the ordinary one).
    pub(super) file_view_notes_folded: bool,
    // plan 004 issue 05b: the file-view point (line, col) the cursor tracks.
    pub(super) file_view_point_line: usize,
    pub(super) file_view_point_col: usize,
    // File watching (issue 04): the current buffer's "changed on disk"
    // conflict marker.
    pub(super) file_view_changed_on_disk: bool,
    // issue 03 (sweep): whether the current buffer is editable, driving the
    // per-kind "changed on disk" banner hint (M-x reload-buffer vs g).
    pub(super) file_view_current_buffer_editable: bool,
    // plan 005 issue 01: the status-line buffer mode word (Edit / Read-only;
    // empty outside the buffer view).
    pub(super) buffer_mode: String,
    // Tree sidebar (issue 09).
    pub(super) tree_visible: bool,
    pub(super) tree_rows: Vec<crate::app::store::TreeRow>,
    pub(super) tree_selected: usize,
    // Terminal width (0 in the static render path; the hardware cursor is
    // live-only, so the tree offset clamps to a real width in practice).
    pub(super) terminal_width: u16,
    // Symbol navigation (issue 05): which-function and indexing indicator.
    pub(super) which_function: String,
    pub(super) indexing: String,
    // Tooling-aware jump (plan 006 issue 02): resolving indicator.
    pub(super) resolving: String,
    // External crate indexing (plan 006 issue 03): crate-index indicator.
    pub(super) crate_indexing: String,
    // Search (issue 06): results view + status-line indicator.
    pub(super) search_title: String,
    pub(super) search_rows: Vec<ResultRow>,
    pub(super) search_top_row: usize,
    pub(super) search_total_rows: usize,
    pub(super) search_selected_row: Option<usize>,
    pub(super) search_running: bool,
    pub(super) search_error: Option<String>,
    pub(super) searching: String,
    // Plan 004 row 11: file-view position display (Top/Bot/L{n},{pct}%).
    pub(super) position: String,
    // plan 005 issue 02: the current file's annotation count ("1 note" /
    // "3 notes"; empty when there are none).
    pub(super) annotations: String,
    // Plan 004 issue 03: region line range for the file view's region face.
    pub(super) region_lines: Option<(usize, usize)>,
    // Plan 004 issue 03: region size for the status line display.
    pub(super) region_size: Option<usize>,
}

/// One render's snapshot: read the tick (establishing the render's
/// dependency on it) and pull every field out of the store in one lock.
pub(super) fn build(
    store: Arc<Mutex<AppStore>>,
    tick: State<u64>,
    terminal_width: u16,
) -> Snapshot {
    // Establish the render's dependency on the revision tick: iocraft
    // re-renders when a State read during the previous render changes.
    // Without this read, tick bumps from the event handler / bus drains
    // are invisible (written but never observed) and the UI stays on its
    // first frame.
    let _revision = tick.get();
    let mut s = store.lock().unwrap();
    let (top_line, total_lines, viewport_lines) = s.file_view_scroll_info();
    let (search_rows, search_top_row, search_total_rows, search_selected_row) =
        s.search_view_info();
    let (magit_rows, magit_top_row, magit_total_rows) = s.magit_view_info();
    // Issue 003-02 shared windowing: the log / blame / commit-diff panes
    // render their pre-computed visible window (the store keeps the
    // cursor row in view; paging resets the log window).
    let (log_rows, _log_top, _log_total) = s.log_view_info();
    let (blame_rows, _blame_top, _blame_total) = s.blame_view_info();
    let (commit_diff_rows, commit_diff_top_row, commit_diff_total_rows) =
        s.commit_diff_view_info();
    // jump-highlight: the landing row's highlight is attached AFTER the
    // rows are built — the fade intensity is computed HERE (in the
    // snapshot) as a pure function of the landing's age and the
    // terminal's color capability, so `render_row` stays dumb (it maps
    // an intensity to a color). The highlight only applies to the CURRENT
    // buffer (a cross-file highlight is not rendered anywhere) and to
    // code rows (not note rows).
    let mut file_view_rows = s.file_view_rows();
    if let Some(h) = s.jump_highlight()
        && s
            .buffers
            .current()
            .is_some_and(|k| *k == h.buffer_key)
    {
        let intensity = jump_highlight_intensity(
            h.set_at.elapsed(),
            crate::ui::truecolor_enabled(),
        );
        if intensity > 0.0
            && let Some(row) = file_view_rows
                .iter_mut()
                .find(|r| !r.is_note && r.line == h.line)
        {
            row.highlight = Some((h.start, h.end, intensity));
        }
    }
    Snapshot {
        quit: s.quit,
        project: s.project_display().to_string(),
        view: s.render_view(),
        view_name: s.view_name_display(),
        pending: s.pending_display(),
        activity: s.activity_display(),
        message: s.message.clone(),
        home_title: s.home_title(),
        home_rows: s.home_body_rows(),
        buffer_rows: s.buffer_rows(),
        buffer_list_selected: s.buffer_list_selected(),
        picker: s.picker_open(),
        prompt: s.picker_prompt().to_string(),
        query: s.picker_query().to_string(),
        selected: s.picker_selected(),
        candidates: s
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.clone())
            .collect(),
        total: s.picker_count().1,
        preview: s.picker_preview().to_string(),
        magit_rows,
        magit_top_row,
        magit_total_rows,
        menu_open: s.menu_open(),
        menu_rows: s.menu_rows(),
        menu_height: s.menu_height(),
        log_title: s.log_title(),
        log_rows,
        blame_title: s.blame_title(),
        blame_rows,
        commit_diff_title: s.commit_diff_title(),
        commit_diff_rows,
        commit_diff_top_row,
        commit_diff_total_rows,
        commit_editor_title: s.commit_editor_title(),
        commit_editor_rows: s.commit_editor_rows(),
        dirty: s.dirty_counts(),
        file_view_rows,
        file_view_total_rows: s.file_view_total_rows(),
        file_view_title: s.view_name_display(),
        file_view_top_line: top_line,
        file_view_total_lines: total_lines,
        file_view_viewport_lines: viewport_lines,
        file_view_notes_folded: s.note_rows_folded(),
        file_view_point_line: s.file_view_point().0,
        file_view_point_col: s.file_view_point().1,
        file_view_changed_on_disk: s.current_buffer_changed_on_disk(),
        file_view_current_buffer_editable: s.current_buffer_editable(),
        buffer_mode: s.buffer_mode_display(),
        tree_visible: s.tree_visible(),
        tree_rows: s.tree_rows(),
        tree_selected: s.tree_selected(),
        terminal_width,
        which_function: s.which_function(),
        indexing: s.indexing_display(),
        resolving: s.resolving_display(),
        crate_indexing: s.crate_indexing_display(),
        search_title: s.search_title(),
        search_rows,
        search_top_row,
        search_total_rows,
        search_selected_row,
        search_running: s.search_running(),
        search_error: s.search_error(),
        searching: s.search_display(),
        position: s.file_view_position_display(),
        annotations: s.annotation_count_display(),
        region_lines: s.region_line_range(),
        region_size: s.region_size_bytes(),
    }
}
