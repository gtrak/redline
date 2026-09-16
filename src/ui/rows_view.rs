//! Shared renderer for the magit-family views (status, log, blame,
//! commit-diff, commit editor): a titled, theme-colored list of
//! [`MagitRow`]s plus a help line. Issue 07's status view is the original
//! consumer; issue 08 reuses it for log / blame / commit-diff / commit-editor.

use iocraft::prelude::*;

use crate::model::sections::MagitRow;
use crate::theme;
use crate::ui::diff_view::row_face;
use crate::ui::{face_bg, face_color, face_weight};

#[derive(Default, Props)]
pub struct MagitRowsViewProps {
    pub title: String,
    pub rows: Vec<MagitRow>,
    pub help: String,
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

    /// Structural regression: the MagitRowsView title and first row must
    /// render on separate lines (not overprinted on the same row).
    #[test]
    fn magit_rows_view_title_and_rows_on_separate_lines() {
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
        // Open the log view (uses MagitRowsView internally).
        store.open_log();

        let title = store.log_title();
        let rows = store.log_rows();
        assert!(!rows.is_empty(), "expected non-empty log rows");

        let mut app = element! {
            ContextProvider(value: Context::owned(Arc::new(Mutex::new(store)))) {
                Root
            }
        };
        let s = app.to_string();

        // The title must be on its own line.
        let lines: Vec<&str> = s.lines().collect();
        let title_idx = lines
            .iter()
            .position(|l| l.contains(&title))
            .unwrap_or_else(|| panic!("title '{title}' line missing\n{s}"));
        // The first row's text must be on a different line than the title.
        let first_row_text = &rows[0].text;
        if !first_row_text.is_empty() {
            let row_lines: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, l)| l.contains(first_row_text))
                .map(|(i, _)| i)
                .collect();
            assert!(
                !row_lines.contains(&title_idx),
                "first log row '{first_row_text}' overprinted on title line\n{s}"
            );
        }
    }
}

/// The generic magit-family buffer: a title, the colored rows, and a help
/// line. Each row's face comes from [`row_face`] (diff coloring is shared
/// with the status buffer).
#[component]
pub fn MagitRowsView(
    props: &MagitRowsViewProps,
    mut _hooks: Hooks,
) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(flex_grow: 1.0_f32, overflow: Overflow::Hidden) {
            View(flex_direction: FlexDirection::Column, flex_grow: 1.0_f32, background_color: face_bg(t.view)) {
                Text(
                    content: &props.title,
                    color: face_color(t.view_title),
                    weight: face_weight(t.view_title),
                )
                #(props.rows.iter().enumerate().map(|(i, row)| {
                    let face = row_face(row.role, row.selected, &t);
                    element! {
                        Text(
                            key: i.to_string(),
                            content: &row.text,
                            color: face_color(face),
                            invert: row.selected,
                            weight: face_weight(face),
                        )
                    }
                }))
                Text(
                    content: &props.help,
                    color: face_color(t.preview),
                )
            }
        }
    }
}
