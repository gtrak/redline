//! Project file-tree sidebar (treemacs-lite, issue 09): a left column listing
//! the ignore-aware file list from the existing walk, with a visible cursor,
//! `RET` to open the selected file, and optional buffer-follow (off by
//! default). The store owns the tree state (rows, selection, visibility);
//! this component is a pure renderer.

use iocraft::prelude::*;

use crate::app::store::TreeRow;
use crate::theme;
use crate::ui::{face_bg, face_color, face_weight};

#[derive(Default, Props)]
pub struct TreeSidebarProps {
    pub rows: Vec<TreeRow>,
    pub selected: usize,
}

/// A fixed-width left column of indented file rows, the selected one
/// highlighted (reverse-video cursor).
#[component]
pub fn TreeSidebar(props: &TreeSidebarProps, mut _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    // The visible window: the rows around the selection. A small top offset
    // leaves room for the header.
    let start = props.selected.saturating_sub(5);
    let visible: Vec<(usize, &TreeRow)> = props
        .rows
        .iter()
        .enumerate()
        .skip(start)
        .take(8)
        .collect();
    element! {
        View(width: 34, flex_shrink: 0.0, overflow: Overflow::Hidden, background_color: face_bg(t.view)) {
            Text(
                content: "*tree*",
                color: face_color(t.view_title),
                weight: face_weight(t.view_title),
            )
            #({
                visible
                    .iter()
                    .map(|(i, row)| {
                        let selected = *i == props.selected;
                        let face = if selected {
                            t.list_item_selected
                        } else {
                            t.list_item
                        };
                        let indent = "  ".repeat(row.depth.min(8));
                        let marker = if row.is_dir { "▸ " } else { "  " };
                        element! {
                            Text(
                                key: i.to_string(),
                                content: format!("{indent}{marker}{}", row.name),
                                color: face_color(face),
                                invert: selected,
                                weight: face_weight(face),
                            )
                        }
                    })
            })
            Text(
                content: "RET open · ↑/↓ move · C-c p t toggle",
                color: face_color(t.preview),
            )
        }
    }
}
