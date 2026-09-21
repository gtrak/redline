//! The root's two leaf widgets: the minibuffer line and the status line.

use iocraft::prelude::*;

use crate::app::store::DirtyCounts;
use crate::theme;
use crate::ui::{face_bg, face_color, face_weight};

#[derive(Default, Props)]
pub(super) struct MinibufferProps {
    pub message: String,
}
#[component]
pub(super) fn Minibuffer(props: &MinibufferProps, mut _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    let line = if props.message.is_empty() {
        "ready".to_string()
    } else {
        props.message.clone()
    };
    element! {
        View(flex_shrink: 0.0) {
            Text(
                content: format!(" {line}"),
                color: face_color(t.minibuffer),
                weight: face_weight(t.minibuffer),
            )
        }
    }
}
#[derive(Default, Props)]
pub(super) struct StatusLineProps {
    pub project: String,
    pub view: String,
    /// The buffer mode word (plan 005 issue 01 + plan 015 issue 02):
    /// `Accurate` in the accurate edit mode, `Edit` in annotation mode on
    /// an editable buffer, `Read-only` otherwise; empty outside the buffer
    /// view.
    pub mode: String,
    pub pending: String,
    pub activity: String,
    pub dirty: Option<DirtyCounts>,
    pub which_function: String,
    pub indexing: String,
    /// Tooling-aware jump (plan 006 issue 02): active while an M-. miss is
    /// being resolved by the provider chain (`resolving …`).
    pub resolving: String,
    /// External crate indexing (plan 006 issue 03): active while a
    /// registry source's crate index builds in the background
    /// (`indexing crate …`).
    pub crate_indexing: String,
    pub searching: String,
    pub position: String,
    /// The current file's annotation count ("1 note" / "3 notes"; empty
    /// when there are none — plan 005 issue 02).
    pub annotations: String,
    /// Region size in bytes (plan 004 issue 03); shown when a region is active.
    pub region_size: Option<usize>,
}
#[component]
pub(super) fn StatusLine(props: &StatusLineProps, mut _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    // A pending prefix puts the status line into its active face.
    let face = if props.pending.is_empty() {
        t.status_line
    } else {
        t.status_line_active
    };
    let mut text = format!("* {} *  {}", props.project, props.view);
    if !props.mode.is_empty() {
        text.push_str(&format!("  {}", props.mode));
    }
    if !props.pending.is_empty() {
        text.push_str(&format!("  [{}]", props.pending));
    }
    if let Some(d) = props.dirty
        && d.staged + d.unstaged + d.untracked > 0
    {
        // `+` staged (index vs HEAD), `~` unstaged (workdir vs index),
        // `?` untracked.
        text.push_str(&format!("  +{} ~{} ?{}", d.staged, d.unstaged, d.untracked));
    }
    if !props.which_function.is_empty() {
        text.push_str(&format!("  ({})", props.which_function));
    }
    if !props.activity.is_empty() {
        text.push_str(&format!("  *{}", props.activity));
    }
    if !props.indexing.is_empty() {
        text.push_str(&format!("  *{}", props.indexing));
    }
    if !props.resolving.is_empty() {
        text.push_str(&format!("  *{}", props.resolving));
    }
    if !props.crate_indexing.is_empty() {
        text.push_str(&format!("  *{}", props.crate_indexing));
    }
    if !props.searching.is_empty() {
        text.push_str(&format!("  *{}", props.searching));
    }
    if !props.position.is_empty() {
        text.push_str(&format!("  {}", props.position));
    }
    if !props.annotations.is_empty() {
        text.push_str(&format!("  {}", props.annotations));
    }
    if let Some(size) = props.region_size {
        text.push_str(&format!("  [{}B]", size));
    }
    element! {
        // `NoWrap` + hidden overflow: the status line must occupy EXACTLY one
        // row. A deep project path (e.g. /tmp/redline_pool/lane0/...) plus
        // mode/activity can exceed the terminal width; without this iocraft
        // wraps it onto a second row, which pushes every content row up and
        // breaks any flow that asserts the bottom row (U-J3 "status line on
        // exactly one row, no wrap artifact"; also the minibuffer-row reads).
        // Truncation is the right UX: the leftmost info (project, view) is
        // what matters most and stays visible.
        View(flex_shrink: 0.0, background_color: face_bg(face), overflow: Overflow::Hidden) {
            Text(
                content: text,
                color: face_color(face),
                weight: face_weight(face),
                wrap: TextWrap::NoWrap,
            )
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// The status line must occupy EXACTLY one row even when its text
    /// overflows the terminal width. A deep project path (the pooled
    /// lanes' `/tmp/rl/<i>/redline_*` roots) plus mode/activity can exceed
    /// 80 cols; without `NoWrap` + hidden overflow iocraft wraps it onto a
    /// second row, which pushed every content row up and broke the
    /// row-based U-J3 assertion. This is the regression pin for that layout
    /// shift: render the line at the 80-col width the PTY suites drive at
    /// and assert it stays a single row (clipped, not wrapped).
    #[test]
    fn status_line_long_text_stays_one_row() {
        // ~160 chars, well past the 80-col width below.
        let long = "r".repeat(160);
        let mut sl = element! {
            StatusLine(project: long.clone(), view: "Buffer".to_string())
        };
        let canvas = sl.render(Some(80));
        assert_eq!(
            canvas.height(),
            1,
            "status line wrapped onto a second row: {:?}",
            canvas.get_text(0, 0, 80, canvas.height())
        );
        // The text is present on that single row (truncated, not wrapped away).
        assert!(canvas.get_text(0, 0, 80, 1).contains('r'));
    }
}
