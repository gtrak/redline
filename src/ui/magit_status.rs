//! The magit status buffer view: renders the status section tree as a flat
//! list of theme-colored rows (section headings, collapsed/expanded, and
//! diff body lines). All fold/cursor logic lives in the model; this view is
//! a pure renderer.

use iocraft::prelude::*;

use crate::model::sections::MagitRow;
use crate::theme;
use crate::ui::diff_view::row_face;
use crate::ui::{face_bg, face_color, face_weight};

#[derive(Default, Props)]
pub struct MagitStatusViewProps {
    /// The visible window of status rows (pre-computed by the store; the
    /// cursor row is always inside it).
    pub rows: Vec<MagitRow>,
    /// The scroll offset (index of the first visible row in the full list).
    pub top_row: usize,
    /// The total number of status rows (drives the scroll indicators).
    pub total_rows: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::store::AppStore;
    use std::sync::{Arc, Mutex};

    fn git_cli(dir: &std::path::Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C").arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .expect("run git");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// Structural regression: the magit status title and the first row
    /// must render on separate lines (not overprinted on the same row).
    #[test]
    fn magit_status_title_and_rows_on_separate_lines() {
        use crate::ui::root::Root;

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "# hello\n").unwrap();
        git_cli(dir.path(), &["init", "-q", "-b", "main"]);
        git_cli(dir.path(), &["config", "user.name", "Test"]);
        git_cli(dir.path(), &["config", "user.email", "test@example.com"]);
        git_cli(dir.path(), &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.path().join("a.txt"), "a\n").unwrap();
        git_cli(dir.path(), &["add", "a.txt"]);
        git_cli(dir.path(), &["commit", "-q", "-m", "init"]);

        let base = tempfile::tempdir().unwrap();
        let mut store = AppStore::at(dir.path(), base.path().to_path_buf());
        store.set_viewport_lines(24);
        store.open_magit_status();

        let rows = store.magit_rows();
        assert!(!rows.is_empty(), "expected non-empty magit rows");

        let mut app = element! {
            ContextProvider(value: Context::owned(Arc::new(Mutex::new(store)))) {
                Root
            }
        };
        let s = app.to_string();

        // The title "*magit-status*" must be on its own line: the line
        // containing it must not also contain a section header or file row.
        let lines: Vec<&str> = s.lines().collect();
        let title_line = lines
            .iter()
            .find(|l| l.contains("*magit-status*"))
            .expect("title line missing");
        // The title line should not also contain a section heading like "##"
        assert!(
            !title_line.contains("##"),
            "title overprinted with section header: {title_line:?}"
        );
        // The first row's text (e.g. a section heading) must be on a
        // different line than the title.
        let first_row_text = &rows[0].text;
        if !first_row_text.is_empty() {
            let row_lines: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, l)| l.contains(first_row_text))
                .map(|(i, _)| i)
                .collect();
            let title_idx = lines
                .iter()
                .position(|l| l.contains("*magit-status*"))
                .unwrap();
            assert!(
                !row_lines.contains(&title_idx),
                "first row '{first_row_text}' overprinted on title line"
            );
        }
    }
}

/// The magit status buffer (`C-x g`): the section tree, rendered with
/// collapsible sections and theme-colored diffs.
#[component]
pub fn MagitStatusView(
    props: &MagitStatusViewProps,
    mut _hooks: Hooks,
) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(flex_grow: 1.0_f32, overflow: Overflow::Hidden) {
            View(flex_direction: FlexDirection::Column, flex_grow: 1.0_f32, background_color: face_bg(t.view)) {
                Text(
                    content: "*magit-status*",
                    color: face_color(t.view_title),
                    weight: face_weight(t.view_title),
                )
                #({
                    // One unambiguous cursor treatment: the selected row gets
                    // its face's explicit background (white-on-blue bar) with NO
                    // invert. A per-row View carries the background so it is
                    // independent of the theme's view background — an invert
                    // would swap the face's white foreground with the view
                    // background, which is white in the light theme and would
                    // erase the cursor. Unselected rows keep the view background.
                    props.rows.iter().enumerate().map(|(i, row)| {
                        let face = row_face(row.role, row.selected, &t);
                        let bg = if row.selected { face_bg(face) } else { face_bg(t.view) };
                        element! {
                            View(key: i.to_string(), background_color: bg) {
                                Text(
                                    content: &row.text,
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
                        Some(element! { Text(content: ind, color: face_color(t.preview)) })
                    }
                })
                Text(
                    content: "s stage · u unstage · TAB fold · RET visit · n/p move · g refresh · q back",
                    color: face_color(t.preview),
                )
            }
        }
    }
}
