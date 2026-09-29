//! Project file-tree sidebar (treemacs-lite, issue 09): a left column listing
//! the ignore-aware file list from the existing walk, with a visible cursor,
//! `RET` to open the selected file, and optional buffer-follow (off by
//! default). The store owns the tree state (rows, selection, visibility);
//! this component is a pure renderer.

use iocraft::prelude::*;

use crate::app::store::TreeRow;
use redline_model::tree_layout::TREE_WIDTH;
use crate::theme;
use crate::ui::{bar_bg, face_bg, face_color, face_weight};

#[derive(Default, Props)]
pub struct TreeSidebarProps {
    /// The store-owned window of tree rows (U-E12 windowing re-home, PLAN §1
    /// row 4 — the ONE surface whose windowing used to live HERE): the
    /// narrowed projection already windowed by the store around the
    /// selection (`AppStore::tree_view_info` — with an empty narrow query
    /// that IS the full-list window). The invariant: when the window is
    /// non-empty the cursor row is always inside it.
    pub rows: Vec<TreeRow>,
    /// The IN-WINDOW index of the selected row (the store's window always
    /// contains it; 0 when the window is empty, where nothing highlights).
    pub selected: usize,
    /// The tree narrow query (U-E12); empty = no narrowing.
    pub query: String,
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

    /// U-E12: the narrow prompt row — ONE NoWrap row directly under the
    /// `*tree*` title whose LEFTMOST text is the decision information
    /// (`↑/↓ · C-g clear`) with the query trailing: keys lead so a
    /// right-edge clip eats the query tail, never the keys (the
    /// 018-03/04/U-E10 prompt shape, 34-col edition). With an empty query
    /// the FULL placeholder (`· type to narrow`) is visible — the key
    /// groups fit the column width exactly.
    #[test]
    fn tree_prompt_row_keys_lead_the_query() {
        use crate::ui::root::Root;

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "# hello\n").unwrap();
        std::fs::write(dir.path().join("src/gamma.rs"), "fn main() {}\n").unwrap();

        let base = tempfile::tempdir().unwrap();
        let mut store = AppStore::at(dir.path(), base.path().to_path_buf());
        store.set_viewport_lines(24);
        store.toggle_tree();
        // Type a query on the prompt, the way the user does: printable
        // chars route through the prompt guard into the narrow query
        // (top view is Home — the sidebar's key surface).
        for c in "gam".chars() {
            store.key_event(crate::app::keymap::parse_key(&c.to_string()).unwrap());
        }
        assert_eq!(store.tree_narrow_query(), "gam", "the typed chars must reach the tree narrow query");

        let mut app = element! {
            ContextProvider(value: Context::owned(Arc::new(Mutex::new(store)))) {
                Root
            }
        };
        // Width-bounded (the PTY contract: 80 cols) so the 34-col tree
        // column clips exactly where it clips live.
        let s = app.render(Some(80)).to_string();
        let lines: Vec<&str> = s.lines().collect();
        let title_idx = lines
            .iter()
            .position(|l| l.contains("*tree*"))
            .unwrap_or_else(|| panic!("tree title line missing\n{s}"));
        let prompt = lines.get(title_idx + 1).copied().unwrap_or("");
        // The prompt row sits directly under the title (one row of chrome
        // above the content window — the re-homed window starts below it).
        assert!(
            prompt.contains("↑/↓ · C-g clear"),
            "the decision keys are missing from the prompt row: {prompt:?}\n{s}"
        );
        // Keys lead: both key groups sit left of the trailing query.
        let query_at = prompt.find("gam").unwrap_or(usize::MAX);
        assert!(query_at != usize::MAX, "the typed query must trail on the prompt row: {prompt:?}");
        for k in ["↑/↓", "C-g clear"] {
            let k_at = prompt.find(k).unwrap_or(usize::MAX);
            assert!(
                k_at < query_at,
                "decision group {k:?} must lead the query: {prompt:?}"
            );
        }

        // Empty query: the FULL placeholder is visible (the 33-col
        // `↑/↓ · C-g clear  · type to narrow` fits the 34-col column
        // exactly — a longer key group would clip it).
        let mut store = AppStore::at(dir.path(), base.path().to_path_buf());
        store.set_viewport_lines(24);
        store.toggle_tree();
        let mut app = element! {
            ContextProvider(value: Context::owned(Arc::new(Mutex::new(store)))) {
                Root
            }
        };
        let s = app.render(Some(80)).to_string();
        let lines: Vec<&str> = s.lines().collect();
        let title_idx = lines
            .iter()
            .position(|l| l.contains("*tree*"))
            .unwrap_or_else(|| panic!("tree title line missing\n{s}"));
        let prompt = lines.get(title_idx + 1).copied().unwrap_or("");
        assert!(
            prompt.contains("· type to narrow"),
            "the empty-query placeholder must render in full: {prompt:?}"
        );
    }

    /// U-E12 windowing re-home: the renderer draws EXACTLY the rows it is
    /// handed (the store's window) — no internal `.skip/.take`: a window
    /// of two rows with the second selected renders two rows, the bar on
    /// the second, and nothing else (the pre-re-home renderer would have
    /// re-windowed over `props.rows`, hiding rows the store chose).
    #[test]
    fn tree_renderer_draws_exactly_the_store_window_it_is_handed() {
        use crate::app::store::TreeRow;
        use iocraft::prelude::*;

        let rows = vec![
            TreeRow { depth: 0, name: "alpha.txt".into(), is_dir: false, rel_path: "alpha.txt".into() },
            TreeRow { depth: 1, name: "gamma.rs".into(), is_dir: false, rel_path: "src/gamma.rs".into() },
        ];
        let selected: usize = 1;
        let mut app = element! {
            TreeSidebar(rows: rows, selected, query: "gam".to_string())
        };
        let s = app.to_string();
        // Both rows the store handed render (no renderer-side windowing
        // dropped the first one) — on distinct lines, the bar on row 2.
        let a = s.lines().position(|l| l.contains("alpha.txt"))
            .unwrap_or_else(|| panic!("row 1 of the store window did not render\n{s}"));
        let g = s.lines().position(|l| l.contains("gamma.rs"))
            .unwrap_or_else(|| panic!("row 2 of the store window did not render\n{s}"));
        assert_ne!(a, g, "the two handed rows must render on distinct lines\n{s}");
    }
}

/// A fixed-width left column of indented file rows, the selected one
/// highlighted (reverse-video cursor). U-E12: the rows arrive STORE-
/// WINDOWED (the `.skip/.take` windowing that lived in this renderer is
/// gone — the store owns it, like every other list surface) and the column
/// carries the narrow prompt row under the title (keys lead, query trails).
#[component]
pub fn TreeSidebar(props: &TreeSidebarProps, mut _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(width: TREE_WIDTH, flex_shrink: 0.0, overflow: Overflow::Hidden, background_color: face_bg(t.view), flex_direction: FlexDirection::Column) {
            Text(
                content: "*tree*",
                color: face_color(t.view_title),
                weight: face_weight(t.view_title),
            )
            // The narrow prompt row (U-E12, the 018-03/04/U-E10 prompt
            // shape): ONE NoWrap row whose LEFTMOST text is the decision
            // information — `↑/↓ · C-g clear` (RET open and C-c p t toggle
            // stay on the help line below: the 34-col column fits exactly
            // these two key groups AND the full placeholder) — with the
            // query trailing. The keys lead so a right-edge clip eats the
            // query tail (recognisable), never the keys (unguessable) —
            // the quit-prompt keys-first precedent (`7f0090a`). The
            // advertised decision keys fall through to the sidebar's
            // commands in `tree_narrow_key_event` — the query never steals
            // a bound key.
            View(overflow: Overflow::Hidden) {
                Text(
                    content: format!(
                        "↑/↓ · C-g clear  {}",
                        if props.query.is_empty() { "· type to narrow".to_string() } else { props.query.clone() }
                    ),
                    color: face_color(t.preview),
                    wrap: TextWrap::NoWrap,
                )
            }
            #({
                // One unambiguous cursor treatment (shared with the magit
                // family): the selected row's face background is drawn on a
                // per-row View (no invert), so the highlight is an explicit
                // white-on-blue bar independent of the theme background.
                // `i` is the IN-WINDOW index — the store's window always
                // contains the cursor row.
                props
                    .rows
                    .iter()
                    .enumerate()
                    .map(|(i, row)| {
                        let selected = i == props.selected;
                        let face = if selected {
                            t.list_item_selected
                        } else {
                            t.list_item
                        };
                        let bg = if selected { bar_bg(face) } else { face_bg(t.view) };
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
