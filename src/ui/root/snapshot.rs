//! The `Snapshot` read model: one render's worth of store state,
//! extracted as owned values so the `Mutex` guard can be dropped
//! before the element tree is built.

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
    /// note rows (plan 005 issue 02). Each row carries its buffer-line
    /// index; the cursor/click math translate through them.
    pub(super) file_view_rows: Vec<FileViewRow>,
    /// The total number of rendered rows for the buffer (canvas bottom
    /// scroll indicator; plan 005 issue 02).
    pub(super) file_view_total_rows: usize,
    pub(super) file_view_title: String,
    pub(super) file_view_top_line: usize,
    pub(super) file_view_total_lines: usize,
    pub(super) file_view_viewport_lines: usize,
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
