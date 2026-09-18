//! Tree-sidebar layout constants (plan 004 issue 05g; moved here from
//! `src/ui/tree.rs`, the same layering fix as 05e's move of the width
//! helpers to `src/model/text_width.rs`): the sidebar's fixed terminal
//! width and the number of file rows its window shows. Plain Rust —
//! zero iocraft (per the `src/model/` layering rule) — so both the UI
//! renderer (layout width, visible rows) and the app store
//! (click-to-row mapping) can share them without the app layer reaching
//! into the UI layer.

/// The sidebar's fixed width in terminal columns. Shared between the
/// layout (`View(width: …)`), the root's click-column offset (plan 004
/// issue 05e) and its hardware-cursor column offset (plan 004 issue
/// 05g) so the three cannot drift.
pub const TREE_WIDTH: u16 = 34;

/// The number of file rows the sidebar window shows (the cursor is kept
/// within 5 rows of the window top). Shared between the renderer and the
/// store's click-to-row mapping (plan 004 issue 05e).
pub const TREE_VISIBLE_ROWS: usize = 8;
