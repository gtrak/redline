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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::store::AppStore;
    use std::sync::{Arc, Mutex};

    /// Structural regression: the tree sidebar title and file rows must
    /// render on separate lines (not overprinted on the same row).
    #[test]
    fn tree_title_and_rows_on_separate_lines() {
        use crate::ui::root::Root;

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "# hello\n").unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();

        let base = tempfile::tempdir().unwrap();
        let mut store = AppStore::at(dir.path(), base.path().to_path_buf());
        store.set_viewport_lines(24);
        store.toggle_tree();

        let mut app = element! {
            ContextProvider(value: Context::owned(Arc::new(Mutex::new(store)))) {
                Root
            }
        };
        let s = app.to_string();

        // The tree title "*tree*" must be on its own line.
        let lines: Vec<&str> = s.lines().collect();
        let title_line = lines
            .iter()
            .find(|l| l.contains("*tree*"))
            .expect("tree title line missing");
        // The title line should not also contain a file name.
        assert!(
            !title_line.contains("main.rs"),
            "tree title overprinted with file row: {title_line:?}"
        );
        // A file row must be on a different line than the title.
        let title_idx = lines.iter().position(|l| l.contains("*tree*")).unwrap();
        let file_lines: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.contains("main.rs"))
            .map(|(i, _)| i)
            .collect();
        assert!(
            !file_lines.contains(&title_idx),
            "file row 'main.rs' overprinted on tree title line"
        );
    }
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
        View(width: 34, flex_shrink: 0.0, overflow: Overflow::Hidden, background_color: face_bg(t.view), flex_direction: FlexDirection::Column) {
            Text(
                content: "*tree*",
                color: face_color(t.view_title),
                weight: face_weight(t.view_title),
            )
            #({
                // One unambiguous cursor treatment (shared with the magit
                // family): the selected row's face background is drawn on a
                // per-row View (no invert), so the highlight is an explicit
                // white-on-blue bar independent of the theme background.
                visible
                    .iter()
                    .map(|(i, row)| {
                        let selected = *i == props.selected;
                        let face = if selected {
                            t.list_item_selected
                        } else {
                            t.list_item
                        };
                        let bg = if selected { face_bg(face) } else { face_bg(t.view) };
                        let indent = "  ".repeat(row.depth.min(8));
                        let marker = if row.is_dir { "▸ " } else { "  " };
                        element! {
                            View(key: i.to_string(), background_color: bg) {
                                Text(
                                    content: format!("{indent}{marker}{}", row.name),
                                    color: face_color(face),
                                    weight: face_weight(face),
                                )
                            }
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
