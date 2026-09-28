//! The `C-x C-b` buffer-list view.

use iocraft::prelude::*;

use crate::app::store::BufferRow;
use crate::theme;
use crate::ui::{bar_bg, face_bg, face_color, face_weight};

#[derive(Default, Props)]
pub struct BufferListViewProps {
    /// The store-windowed window of NARROWED rows (plan 018 issue 04:
    /// windowing moved to the store — the renderer no longer lists all
    /// rows, so a 50-buffer list scrolls instead of running past the
    /// viewport). Each `name` carries the shared display-source string,
    /// marker slot included.
    pub rows: Vec<BufferRow>,
    /// The narrow query (empty = the full MRU list, un-narrowed).
    pub query: String,
    /// The selected row inside the window (None: the selection is outside
    /// the window or the list is empty).
    pub selected_row: Option<usize>,
}

/// The `C-x C-b` list-buffers view: open buffers, MRU order, with the
/// current buffer marked (the marker lives in the row's display string).
#[component]
pub fn BufferListView(props: &BufferListViewProps, mut _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(flex_grow: 1.0_f32, overflow: Overflow::Hidden) {
            View(flex_direction: FlexDirection::Column, flex_grow: 1.0_f32, background_color: face_bg(t.view)) {
                Text(
                    content: "*list-buffers*",
                    color: face_color(t.view_title),
                    weight: face_weight(t.view_title),
                )
                // The narrow prompt row (plan 018 issue 04, PLAN §5.2):
                // ONE NoWrap row whose LEFTMOST text is the decision/verb
                // information — `open RET · kill d · close q` — with the
                // query trailing. The keys lead so a right-edge clip can
                // never hide them (the quit-prompt keys-first precedent).
                View(overflow: Overflow::Hidden) {
                    Text(
                        content: format!(
                            "open RET · kill d · close q  {}",
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
                    props.rows.iter().enumerate().map(|(i, row)| {
                        let face = if props.selected_row == Some(i) {
                            t.list_item_selected
                        } else {
                            t.list_item
                        };
                        let bg = if props.selected_row == Some(i) { bar_bg(face) } else { face_bg(t.view) };
                        element! {
                            View(key: format!("{i}"), background_color: bg) {
                                Text(
                                    content: format!("{} ({} lines)", row.name, row.lines),
                                    color: face_color(face),
                                    weight: face_weight(face),
                                )
                            }
                        }
                    })
                })
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::store::AppStore;
    use std::sync::{Arc, Mutex};

    /// Structural regression: the buffer list title and buffer rows must
    /// render on separate lines (not overprinted on the same row).
    #[test]
    fn buffer_list_title_and_rows_on_separate_lines() {
        use crate::ui::root::Root;

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();

        let base = tempfile::tempdir().unwrap();
        let mut store = AppStore::at(dir.path(), base.path().to_path_buf());
        store.set_viewport_lines(24);
        store.open_path("src/main.rs");
        store.push_view(crate::app::store::ViewId::BufferList);

        let mut app = element! {
            ContextProvider(value: Context::owned(Arc::new(Mutex::new(store)))) {
                Root
            }
        };
        let s = app.to_string();

        let lines: Vec<&str> = s.lines().collect();
        let title_idx = lines
            .iter()
            .position(|l| l.contains("*list-buffers*"))
            .unwrap_or_else(|| panic!("title line missing\n{s}"));
        // A buffer row (e.g. "src/main.rs") must not be on the same line.
        assert!(
            !lines[title_idx].contains("src/main.rs"),
            "buffer row 'src/main.rs' overprinted on title line:\n{s}"
        );
    }

    /// Plan 018 issue 04 (PLAN §5.2): the prompt row is ONE NoWrap row and
    /// the DECISION KEYS LEAD — `open RET · kill d · close q` sits left of
    /// the trailing query, so a right-edge clip eats the query tail, never
    /// the keys (the `7f0090a` / quit-prompt keys-first precedent).
    #[test]
    fn buffer_list_prompt_row_keys_lead_the_query() {
        use crate::ui::root::Root;

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "// lib\n").unwrap();

        let base = tempfile::tempdir().unwrap();
        let mut store = AppStore::at(dir.path(), base.path().to_path_buf());
        store.set_viewport_lines(24);
        store.open_path("src/main.rs");
        store.open_path("src/lib.rs");
        store.push_view(crate::app::store::ViewId::BufferList);
        // Type a query on the prompt, the way the user does: printable
        // chars route through the prompt guard into the narrow query.
        // (No q/n/p/d — those are the view's own decision keys.)
        for c in "zzxxytail".chars() {
            store.key_event(crate::app::keymap::parse_key(&c.to_string()).unwrap());
        }

        let mut app = element! {
            ContextProvider(value: Context::owned(Arc::new(Mutex::new(store)))) {
                Root
            }
        };
        let s = app.to_string();

        let lines: Vec<&str> = s.lines().collect();
        let prompt_idx = lines
            .iter()
            .position(|l| l.contains("zzxxytail"))
            .unwrap_or_else(|| panic!("prompt row with the query missing\n{s}"));
        let prompt = lines[prompt_idx];
        let keys_at = prompt.find("open RET").unwrap_or(usize::MAX);
        let query_at = prompt.find("zzxxytail").unwrap_or(usize::MAX);
        assert!(keys_at < query_at, "the decision keys must lead the query: {prompt:?}");
        // All three decision groups sit on the SAME row, keys-first.
        for verb in ["open RET", "kill d", "close q"] {
            assert!(prompt.contains(verb), "decision group {verb:?} missing from the prompt row: {prompt:?}");
            assert!(
                prompt.find(verb).unwrap() < query_at,
                "decision group {verb:?} must lead the query: {prompt:?}"
            );
        }
        // The query row is the row right below the title (one row of
        // chrome above the content window).
        let title_idx = lines.iter().position(|l| l.contains("*list-buffers*")).unwrap();
        assert_eq!(prompt_idx, title_idx + 1, "the prompt row sits directly under the title\n{s}");
    }
}
