//! The search results view (issue 06): file groups with per-file counts
//! (live while the search streams), match lines with line numbers, the
//! running total in the title, and a help line. `n`/`p` navigate
//! matches, `RET` jumps to the match (recording a jump-stack entry so
//! `M-,` returns), `g` re-runs, `q`/`ESC` cancel/close, `C-g` cancels
//! the in-flight search.
//!
//! The store pre-computes the visible row window (`search_view_info`);
//! this component only renders.

use iocraft::prelude::*;

use crate::app::store::ResultRow;
use crate::theme;
use crate::ui::{bar_bg, face_bg, face_color, face_weight};

#[derive(Default, Props)]
pub struct ResultsViewProps {
    pub title: String,
    /// The visible row window (pre-computed by the store).
    pub rows: Vec<ResultRow>,
    pub top_row: usize,
    pub total_rows: usize,
    /// The selected hit's row, relative to the window (`None` when the
    /// selection scrolled out of the window or there are no hits).
    pub selected_row: Option<usize>,
    pub running: bool,
    pub error: Option<String>,
}

fn row_text(row: &ResultRow) -> String {
    match row {
        ResultRow::Header {
            file,
            count,
            final_count,
        } => {
            let live = if *final_count { "" } else { "…" };
            format!("{file}  ({count}{live})")
        }
        ResultRow::Hit { hit, .. } => format!("{:>6}  {}", hit.line_no, hit.line),
    }
}

#[component]
pub fn ResultsView(props: &ResultsViewProps, mut _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(flex_grow: 1.0_f32, overflow: Overflow::Hidden) {
            View(flex_direction: FlexDirection::Column, flex_grow: 1.0_f32, background_color: face_bg(t.view)) {
                Text(
                    content: &props.title,
                    color: face_color(t.view_title),
                    weight: face_weight(t.view_title),
                )
                #(if let Some(e) = &props.error {
                    Some(element! {
                        Text(
                            content: format!("error: {e}"),
                            color: face_color(t.diff_delete),
                        )
                    })
                } else {
                    None
                })
                #(if props.rows.is_empty() {
                    Some(element! {
                        Text(
                            content: if props.running {
                                "searching…"
                            } else {
                                "no matches"
                            },
                            color: face_color(t.preview),
                        )
                    })
                } else {
                    None
                })
                #({
                    // One unambiguous cursor treatment (shared with the magit
                    // family): the selected row's face background is drawn on a
                    // per-row View (no invert), so the highlight is an explicit
                    // white-on-blue bar independent of the theme background.
                    props.rows.iter().enumerate().map(|(i, row)| {
                        let selected = props.selected_row == Some(i);
                        let face = if selected {
                            t.list_item_selected
                        } else {
                            match row {
                                ResultRow::Header { .. } => t.section_heading,
                                ResultRow::Hit { .. } => t.view,
                            }
                        };
                        let bg = if selected { bar_bg(face) } else { face_bg(t.view) };
                        element! {
                            View(key: i.to_string(), background_color: bg) {
                                Text(
                                    content: row_text(row),
                                    color: face_color(face),
                                    weight: face_weight(face),
                                )
                            }
                        }
                    })
                })
                #({
                    // Scroll indicators (same convention as the file view).
                    let mut ind = String::new();
                    if props.top_row > 0 {
                        ind.push('↑');
                    }
                    if props.top_row + props.rows.len() < props.total_rows {
                        ind.push('↓');
                    }
                    if ind.is_empty() {
                        None
                    } else {
                        Some(element! {
                            Text(content: ind, color: face_color(t.preview))
                        })
                    }
                })
                Text(
                    content: "RET jump (M-, returns) · n/p next/prev · g re-run · q/ESC close · C-g cancel search",
                    color: face_color(t.preview),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::store::{AppStore, ResultRow};
    use crate::search::rg::SearchEvent;
    use std::sync::{Arc, Mutex};

    /// Row rendering: headers show the file + live count, hits show the
    /// line number + text.
    #[test]
    fn row_text_renders_headers_and_hits() {
        let header = ResultRow::Header {
            file: "src/main.rs".to_string(),
            count: 3,
            final_count: true,
        };
        assert_eq!(row_text(&header), "src/main.rs  (3)");

        let header_live = ResultRow::Header {
            file: "src/main.rs".to_string(),
            count: 1,
            final_count: false,
        };
        assert_eq!(row_text(&header_live), "src/main.rs  (1…)");

        let hit = ResultRow::Hit {
            hit: crate::search::rg::Hit {
                file: "src/main.rs".to_string(),
                line_no: 12,
                col: Some(4),
                line: "fn target() {}".to_string(),
            },
            hit_index: 0,
        };
        assert_eq!(row_text(&hit), "    12  fn target() {}");
    }

    /// A store rooted in a temp project (never touches the real cache
    /// dir; the persistence base is a throwaway sibling so the walk
    /// never sees the persistence files).
    fn store(dir: &std::path::Path) -> AppStore {
        let base = tempfile::tempdir().unwrap();
        AppStore::at(dir, base.path().to_path_buf())
    }

    /// Drain the search bus into the store until `Finished` (bounded).
    fn drain_search_to_finished(
        store: &mut AppStore,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<SearchEvent>,
        deadline: std::time::Duration,
    ) -> Vec<SearchEvent> {
        let start = std::time::Instant::now();
        let mut out = Vec::new();
        loop {
            while let Ok(ev) = rx.try_recv() {
                let done = matches!(ev, SearchEvent::Finished { .. });
                store.apply_search_event(&ev);
                out.push(ev);
                if done {
                    return out;
                }
            }
            if start.elapsed() > deadline {
                return out;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Static render: a finished project search renders the title
    /// (query + running counts), the grouped rows (file header with
    /// final count, match lines), and the help line.
    #[test]
    fn results_view_static_render() {
        use crate::ui::root::Root;

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn target() {}\ntarget();\n").unwrap();

        let mut store = store(dir.path());
        store.set_viewport_lines(24);
        store.start_project_search("target".to_string());

        // The search view is on top; drain the bus until it finishes.
        let mut rx = store.search_rx().unwrap();
        let events = drain_search_to_finished(&mut store, &mut rx, std::time::Duration::from_secs(5));
        assert!(
            events.iter().any(|e| matches!(
                e,
                SearchEvent::Finished { cancelled: false, .. }
            )),
            "the search must finish: {events:?}"
        );
        // Two hits in one file group: src/main.rs lines 1 and 2.
        let hits = store
            .search_view_info()
            .0
            .iter()
            .filter(|r| matches!(r, ResultRow::Hit { .. }))
            .count();
        assert_eq!(hits, 2, "two match rows expected");

        let mut app = element! {
            ContextProvider(value: Context::owned(Arc::new(Mutex::new(store)))) {
                Root
            }
        };
        let s = app.to_string();
        assert!(s.contains("Search: 'target'"), "title missing:\n{s}");
        assert!(s.contains("src/main.rs  (2)"), "file group header missing:\n{s}");
        assert!(s.contains("fn target() {}"), "match line missing:\n{s}");
        assert!(s.contains("RET jump"), "help line missing:\n{s}");
        assert!(s.contains("*  *search*"), "status line view name missing:\n{s}");
    }

    /// Structural regression: the search results title and first row must
    /// render on separate lines (not overprinted on the same row).
    #[test]
    fn results_view_title_and_rows_on_separate_lines() {
        use crate::ui::root::Root;

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn target() {}\ntarget();\n").unwrap();

        let mut store = store(dir.path());
        store.set_viewport_lines(24);
        store.start_project_search("target".to_string());

        let mut rx = store.search_rx().unwrap();
        drain_search_to_finished(&mut store, &mut rx, std::time::Duration::from_secs(5));

        let (rows, _, _, _) = store.search_view_info();
        assert!(!rows.is_empty(), "expected non-empty search rows");

        let title = store.search_title();

        let mut app = element! {
            ContextProvider(value: Context::owned(Arc::new(Mutex::new(store)))) {
                Root
            }
        };
        let s = app.to_string();

        let lines: Vec<&str> = s.lines().collect();
        let title_idx = lines
            .iter()
            .position(|l| l.contains(&title))
            .unwrap_or_else(|| panic!("title '{title}' line missing\n{s}"));
        // The first row text must not be on the same line as the title.
        let first_row_text = row_text(&rows[0]);
        assert!(
            !lines[title_idx].contains(&first_row_text),
            "first search row '{first_row_text}' overprinted on title line:\n{s}"
        );
    }
}
