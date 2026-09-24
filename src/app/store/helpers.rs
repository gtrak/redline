use super::{CommitEditorState, PickerCandidate};
use ropey::Rope;

use crate::git::blame::BlameLine;
use crate::git::log::{relative_time_from, LogEntry};


/// Pure scroll-anchor math for a buffer reload (issue 04).
///
/// `old_top` is the scroll top (first visible line, 0-based) before the
/// reload; `new_total` is the line count after the reload. The reader's line
/// anchor is preserved when it still exists (`old_top < new_total`); when the
/// anchor line has vanished (the file shrank past it), the view clamps to the
/// last line so the reader snaps to the end of the now-shorter file.
///
/// Extracted as a pure function so it is testable without notify / IO.
pub fn reload_anchor(old_top: usize, new_total: usize) -> usize {
    if new_total == 0 {
        return 0;
    }
    old_top.min(new_total - 1)
}


pub(super) fn file_candidate(rel: &str) -> PickerCandidate {
    let label = rel.rsplit('/').next().unwrap_or(rel);
    PickerCandidate {
        name: rel.to_string(),
        display: rel.to_string(),
        // picker-density: name-first — the file name left, the path
        // right-aligned (truncates tail-keeping, so the name survives
        // long paths). A root-level file has no directory component, so
        // the name IS the path: `detail` stays empty and the row draws
        // the name once (left-anchored), not twice.
        label: label.to_string(),
        detail: if label == rel { String::new() } else { rel.to_string() },
        docs: String::new(),
        category: "file".to_string(),
        ann_col: None,
    }
}

/// Build the commit editor's pre-filled text (a magit-style comment block
/// listing the staged files) and place the cursor at the end (on the trailing
/// empty line, where the user types the message).
pub(super) fn prefill_commit_message(staged: &[(String, char)]) -> (String, usize) {
    let mut s = String::new();
    s.push_str("# Please enter the commit message for these changes.\n");
    s.push_str("# Lines starting with '#' are ignored; C-c C-c commits, C-c C-k aborts.\n");
    s.push_str("#\n");
    s.push_str("# Staged changes:\n");
    if staged.is_empty() {
        s.push_str("#   (nothing staged)\n");
    } else {
        for (path, letter) in staged {
            s.push_str(&format!("#   {letter} {path}\n"));
        }
    }
    s.push_str("#\n");
    let len = s.len();
    (s, len)
}

/// Extract the commit message from the editor text: drop `#`-prefixed comment
/// lines and trim leading/trailing blank lines. An empty result means the
/// user typed no message.
pub(super) fn extract_commit_message(rope: &Rope) -> String {
    let text = rope.to_string();
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .map(|l| l.to_string())
        .collect();
    while !lines.is_empty() && lines.first().map(|s| s.trim().is_empty()).unwrap_or(true) {
        lines.remove(0);
    }
    while !lines.is_empty() && lines.last().map(|s| s.trim().is_empty()).unwrap_or(true) {
        lines.pop();
    }
    lines.join("\n")
}

/// Move the commit-editor cursor to a neighbouring line, keeping the column
/// (clamped to the target line's length).
pub(super) fn editor_cursor_line(ed: &mut CommitEditorState, delta: i32) {
    let len = ed.rope.len_lines();
    let line = ed.rope.char_to_line(ed.cursor.min(ed.rope.len_chars()));
    let line_start = ed.rope.line_to_char(line);
    let col = ed.cursor - line_start;
    let target = line as i64 + delta as i64;
    if target < 0 || target >= len as i64 {
        return;
    }
    let target = target as usize;
    let t_start = ed.rope.line_to_char(target);
    let t_len = ed
        .rope
        .get_line(target)
        .map(|l| l.len_chars())
        .unwrap_or(0);
    ed.cursor = t_start + col.min(t_len);
}

// ── Shared windowing (issue 003-02) ──────────────────────────────────────
//
// The single windowing mechanism for every long-content pane (magit status,
// commit-diff, blame, log, editable buffers). Three pure pieces of math,
// extracted from the magit status buffer's shipped logic so each pane reuses
// one implementation. All are plain Rust (no iocraft), so the UI layer only
// renders whatever the store pre-computes.

/// The number of rows that fit in a pane's content area: the viewport minus
/// the pinned chrome rows (the title, a scroll indicator, and the help line),
/// at least one. `viewport_lines` is the content height set on resize.
pub(super) fn pane_window(viewport_lines: usize) -> usize {
    viewport_lines.saturating_sub(2).max(1)
}

/// The first/last visible row indices for a scroll window over `total` rows.
/// `scroll` is clamped into `[0, total)`; `end` is bounded by `total`. Returns
/// `(start, end)` (a half-open range); `(0, 0)` when `total` is 0.
pub(super) fn window_slice(scroll: usize, total: usize, window: usize) -> (usize, usize) {
    if total == 0 {
        return (0, 0);
    }
    let start = scroll.min(total.saturating_sub(1));
    (start, (start + window).min(total))
}

/// The new scroll offset that keeps `cursor` inside the visible window:
/// scroll up when the cursor is above the top row, scroll down when it is
/// below the last visible row (the cursor then lands on the last visible row).
/// `cursor` must be `< total`. Pure; returns the clamped offset.
pub(super) fn keep_cursor_visible(scroll: usize, cursor: usize, total: usize, window: usize) -> usize {
    if total == 0 {
        return 0;
    }
    let mut scroll = scroll;
    if cursor < scroll {
        scroll = cursor;
    } else if cursor >= scroll + window {
        scroll = cursor + 1 - window;
    }
    scroll.min(total.saturating_sub(1))
}

/// The new scroll offset that puts buffer line `point_line` on screen row
/// `desired_row`: `scroll_top = point_line - desired_row`, clamped to
/// `[0, total - viewport]`. Returns `None` when the buffer does not scroll
/// at all (nothing to recenter). Shared by `recenter` (C-l — the desired
/// row is cycle-selected) and the jump-landing recenter (plan 004 issue
/// 07 — always the fresh MIDDLE row; the only difference from `recenter`
/// is that `recenter_cycle` is NOT advanced: a jump is not a `C-l`).
pub(super) fn recenter_top_for(point_line: usize, desired_row: usize, total: usize, viewport: usize) -> Option<usize> {
    if total <= 1 {
        return None;
    }
    let vp = viewport.max(1);
    let max_scroll = total.saturating_sub(vp);
    if max_scroll == 0 {
        return None;
    }
    Some((point_line as i64 - desired_row as i64).clamp(0, max_scroll as i64) as usize)
}


/// One log row: `<short_id> <subject>  <author>  <date>`.
pub(super) fn log_entry_display(e: &LogEntry) -> String {
    format!("{} {}  {}  {}", e.short_id, e.subject, e.author, e.date)
}

/// One blame row: aligned `<hash> <author> <age>  <text>`.
pub(super) fn blame_line_display(line: &BlameLine, now: i64, author_w: usize) -> String {
    let age = relative_time_from(line.time, now);
    format!(
        "{:<7} {:<author_w$} {:<5} {}",
        line.short_id, line.author, age, line.text
    )
}
