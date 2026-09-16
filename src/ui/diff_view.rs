//! Shared, theme-colored diff rendering: maps a magit/diff row to a theme
//! face. Used by the magit status buffer (issue 07) and reused by the
//! log/commit diff buffers (issue 08), so added/deleted/context/hunk-header
//! coloring lives in exactly one place.

use crate::model::sections::RowRole;
use crate::theme;

/// The face for a magit/diff row, given its role and whether the row is the
/// section under the cursor. Diff body lines never carry the cursor.
pub fn row_face(role: RowRole, selected: bool, t: &theme::Theme) -> theme::Face {
    match role {
        RowRole::DiffContext => t.diff_context,
        RowRole::DiffAdd => t.diff_add,
        RowRole::DiffDelete => t.diff_delete,
        RowRole::HunkHeader => t.diff_hunk_header,
        // Section headings (branch, group, file): highlight the one under
        // the cursor.
        _ => {
            if selected {
                t.section_heading_selected
            } else {
                t.section_heading
            }
        }
    }
}
