//! The `C-x C-b` buffer-list view.

use iocraft::prelude::*;

use crate::app::store::BufferRow;
use crate::theme;
use crate::ui::{bar_bg, face_bg, face_color, face_weight};

#[derive(Default, Props)]
pub struct BufferListViewProps {
    pub rows: Vec<BufferRow>,
    pub selected: usize,
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
}

/// The `C-x C-b` list-buffers view: open buffers, MRU order, with the
/// current buffer marked.
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
                #({
                    // One unambiguous cursor treatment (shared with the magit
                    // family): the selected row's face background is drawn on a
                    // per-row View (no invert), so the highlight is an explicit
                    // white-on-blue bar independent of the theme background.
                    props.rows.iter().enumerate().map(|(i, row)| {
                        let face = if i == props.selected {
                            t.list_item_selected
                        } else {
                            t.list_item
                        };
                        let bg = if i == props.selected { bar_bg(face) } else { face_bg(t.view) };
                        let marker = if row.current { "*" } else { " " };
                        element! {
                            View(key: format!("{i}"), background_color: bg) {
                                Text(
                                    content: format!("{marker}{} ({} lines)", row.name, row.lines),
                                    color: face_color(face),
                                    weight: face_weight(face),
                                )
                            }
                        }
                    })
                })
                Text(
                    content: "RET open · C-n/C-p or arrows move · q close",
                    color: face_color(t.preview),
                )
            }
        }
    }
}
