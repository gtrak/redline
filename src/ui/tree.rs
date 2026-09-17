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
    ///
    /// Non-vacuous by construction: it requires the tree's own rows to
    /// actually render (the first two must be present on the frame) and to
    /// land on lines distinct from the title and from each other. A layout
    /// that clips the rows away, or collapses them onto the title line, fails.
    #[test]
    fn tree_title_and_rows_on_separate_lines() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "# hello\n").unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();

        let base = tempfile::tempdir().unwrap();
        let mut store = AppStore::at(dir.path(), base.path().to_path_buf());
        store.set_viewport_lines(24);
        store.toggle_tree();

        // Capture the actual tree rows (by name) before the store is wrapped
        // in Arc<Mutex<_>> for the render context, so the assertions check
        // the rows that really rendered rather than a hard-coded string that
        // could be absent.
        let tree_names: Vec<String> = store
            .tree_rows()
            .iter()
            .map(|r| r.name.clone())
            .collect();
        assert!(
            tree_names.len() >= 2,
            "need >=2 tree rows to assert distinct lines: {tree_names:?}"
        );

        let mut app = element! {
            ContextProvider(value: Context::owned(Arc::new(Mutex::new(store)))) {
                Root
            }
        };
        // Render at a fixed 80-column width — the standard terminal — so the
        // structural layout is deterministic and matches what the user sees
        // (the default content-sized static width is too narrow to assert on).
        let s = app.render(Some(80)).to_string();
        let lines: Vec<&str> = s.lines().collect();

        // The tree title "*tree*" must be present on its own line.
        let title_idx = lines
            .iter()
            .position(|l| l.contains("*tree*"))
            .expect("tree title line missing");
        // The title line must not also carry the first tree row.
        let first = tree_names[0].as_str();
        assert!(
            !lines[title_idx].contains(first),
            "tree title shares a line with the first row {first:?}: {}",
            lines[title_idx]
        );

        // Non-vacuous core: the first two tree rows must each render, on a line
        // distinct from the title line and distinct from each other.
        let line_of = |name: &str| lines.iter().position(|l| l.contains(name));
        let i0 = line_of(first)
            .unwrap_or_else(|| panic!("first tree row {first:?} did not render\n{s}"));
        let second = tree_names[1].as_str();
        let i1 = line_of(second)
            .unwrap_or_else(|| panic!("second tree row {second:?} did not render\n{s}"));
        assert_ne!(i0, title_idx, "first tree row {first:?} overprinted on the title line");
        assert_ne!(i1, title_idx, "second tree row {second:?} overprinted on the title line");
        assert_ne!(
            i0, i1,
            "tree rows not on distinct lines (first={i0}, second={i1})\n{s}"
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
