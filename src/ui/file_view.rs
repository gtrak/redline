//! Virtualized file view: renders only visible lines ± margin with
//! tree-sitter highlight spans. Uses a canvas component (like the
//! picker) for exact row/column placement of multi-colored text.
//!
//! The store pre-computes the visible lines (text + spans) for the
//! current viewport; this component only renders them.

use iocraft::{prelude::*, Component, ComponentDrawer, ComponentUpdater};

use crate::app::store::FileViewRow;
use crate::model::text_width::{char_display_width, display_width};
use crate::theme;
use crate::ui::color;

#[derive(Default, Props)]
struct FileViewCanvasProps {
    /// The pre-computed rendered rows (code rows + the virtual annotation
    /// note rows, plan 005 issue 02). Each row carries its buffer-line
    /// index (`r.line`), NOT its slice offset — the rows are no longer a
    /// dense 1:1 slice of buffer lines.
    pub rows: Vec<FileViewRow>,
    /// The total number of RENDERED rows for the buffer (buffer lines +
    /// visible note rows; the bottom scroll indicator's bound).
    pub total_rows: usize,
    /// The first visible BUFFER line (window top).
    pub top_line: usize,
    /// The region's line range (start_line, end_line inclusive) in buffer
    /// line indices, or `None` when no mark is set. The store computes this
    /// from the byte range using the rope (plan 004 issue 03).
    pub region_lines: Option<(usize, usize)>,
}

/// Canvas-backed file view: renders the visible rows with colored
/// spans and scroll indicators.
struct FileViewCanvas {
    rows: Vec<FileViewRow>,
    total_rows: usize,
    top_line: usize,
    region_lines: Option<(usize, usize)>,
}

impl FileViewCanvas {
    fn from_props(props: &FileViewCanvasProps) -> Self {
        Self {
            rows: props.rows.clone(),
            total_rows: props.total_rows,
            top_line: props.top_line,
            region_lines: props.region_lines,
        }
    }
}

impl Component for FileViewCanvas {
    type Props<'a> = FileViewCanvasProps;

    fn new(props: &Self::Props<'_>) -> Self {
        Self::from_props(props)
    }

    fn update(
        &mut self,
        props: &mut Self::Props<'_>,
        _hooks: Hooks,
        updater: &mut ComponentUpdater,
    ) {
        *self = Self::from_props(props);
        updater.set_layout_style(iocraft::taffy::style::Style {
            size: iocraft::taffy::geometry::Size {
                width: iocraft::taffy::style::Dimension::Percent(1.0),
                height: iocraft::taffy::style::Dimension::Length(0.0),
            },
            flex_grow: 1.0,
            ..Default::default()
        });
    }

    fn draw(&mut self, drawer: &mut ComponentDrawer<'_>) {
        let layout = drawer.layout();
        let mut canvas = drawer.canvas();
        let t = theme::current();
        let w = layout.size.width.max(1.0) as usize;
        let h = layout.size.height.max(1.0) as usize;

        for (row, r) in self.rows.iter().enumerate() {
            if row >= h {
                break;
            }
            // Paint the region background for rows whose BUFFER line is
            // within the region (note rows inherit their anchored line).
            if let Some((rl_start, rl_end)) = self.region_lines
                && (rl_start..=rl_end).contains(&r.line)
            {
                let bg = color(t.region.background);
                canvas.set_background_color(0, row as isize, w, 1, bg);
            }
            if r.is_note {
                // A virtual annotation note row (plan 005 issue 02): dim
                // and italic, directly under the anchored code row.
                let style = text_style_italic(t.preview.foreground);
                let display = truncate(&r.text, w);
                if !display.is_empty() {
                    canvas.set_text(0, row as isize, &display, style);
                }
            } else {
                draw_line(&mut canvas, row as isize, w, &r.text, &r.spans, &t);
                // The annotation margin marker (plan 005 issue 02): always
                // on for annotated lines, independent of the note-row
                // toggle (C-c a). Overlaid at cell 0.
                if r.annotated {
                    canvas.set_text(
                        0,
                        row as isize,
                        "\u{258e}",
                        text_style(t.view_title.foreground, false),
                    );
                }
            }
        }

        // Scroll indicators: show "↑" when scrolled past the top, "↓" when
        // more rows remain below. The "↓" bound is in RENDERED-row space
        // (plan 005 issue 02): note rows count as rows, and a slice row
        // below the canvas bottom (the title/indicator overlap, as before)
        // also implies more below.
        let mut indicators = String::new();
        if self.top_line > 0 {
            indicators.push('↑');
        }
        if self.rows.len() > h || self.rows.len() < self.total_rows {
            indicators.push('↓');
        }
        if !indicators.is_empty() {
            let x = (w as i32).saturating_sub(indicators.len() as i32 + 1) as isize;
            canvas.set_text(
                x,
                (h as isize).saturating_sub(1),
                &indicators,
                text_style(t.preview.foreground, false),
            );
        }
    }
}

/// Render one line on the canvas: the base text in the default face,
/// then overlay each span with its face color.
fn draw_line(
    canvas: &mut iocraft::CanvasSubviewMut<'_>,
    row: isize,
    width: usize,
    text: &str,
    spans: &[crate::syntax::highlight::LineSpan],
    t: &theme::Theme,
) {
    if text.is_empty() {
        return;
    }
    if spans.is_empty() {
        let face = t.view;
        let style = text_style(face.foreground, face.bold);
        let display = truncate(text, width);
        canvas.set_text(0, row, &display, style);
        return;
    }

    // Build segments: (char_start, char_end, Option<face_index>).
    let chars: Vec<char> = text.chars().collect();
    let total_chars = chars.len();
    let mut segments: Vec<(usize, usize, Option<usize>)> = Vec::new();

    let mut prev_byte = 0;
    for span in spans {
        let char_start = byte_to_char_offset(text, span.start);
        let char_end = byte_to_char_offset(text, span.end);
        if prev_byte < span.start {
            let gap_start = byte_to_char_offset(text, prev_byte);
            if gap_start < char_start {
                segments.push((gap_start, char_start, None));
            }
        }
        if char_start < char_end {
            segments.push((char_start, char_end, span.face));
        }
        prev_byte = span.end.max(prev_byte);
    }
    if prev_byte < text.len() {
        let gap_start = byte_to_char_offset(text, prev_byte);
        if gap_start < total_chars {
            segments.push((gap_start, total_chars, None));
        }
    }

    let mut x = 0isize;
    for (cs, ce, face_idx) in &segments {
        if *cs >= *ce || *cs >= total_chars {
            continue;
        }
        let end = (*ce).min(total_chars);
        let segment: String = chars[*cs..end].iter().collect();
        let remaining = width.saturating_sub(x as usize);
        let segment = truncate(&segment, remaining);
        if segment.is_empty() {
            continue;
        }
        let face = match face_idx {
            Some(idx) => t.syntax_face(*idx),
            None => t.view,
        };
        let style = text_style(face.foreground, face.bold);
        canvas.set_text(x, row, &segment, style);
        // Advance by DISPLAY width, not char count: a segment ending in a
        // wide char occupies one more cell than its char count, and the
        // next segment must start where the terminal actually is
        // (plan 004 issue 05d — the dropped-space-after-wide-char bug).
        x += display_width(&segment) as isize;
        if x >= width as isize {
            break;
        }
    }
}

/// Convert a byte offset in `text` to a char offset.
/// If `byte` is not on a char boundary, rounds down to the previous boundary.
fn byte_to_char_offset(text: &str, byte: usize) -> usize {
    if byte >= text.len() {
        return text.chars().count();
    }
    // Find the floor char boundary (largest char boundary <= byte).
    // UTF-8 multi-byte sequences are at most 4 bytes, so this is O(1).
    let boundary = (0..=byte).rev().find(|&b| text.is_char_boundary(b)).unwrap_or(0);
    text[..boundary].chars().count()
}

fn text_style(foreground: theme::Color, bold: bool) -> CanvasTextStyle {
    let mut style = CanvasTextStyle::default();
    style.color = Some(color(foreground));
    if bold {
        style.weight = Weight::Bold;
    }
    style
}

/// A dim + italic face style (the annotation note rows, plan 005 issue 02).
fn text_style_italic(foreground: theme::Color) -> CanvasTextStyle {
    let mut style = text_style(foreground, false);
    style.italic = true;
    style
}

// ── plan 004 issue 05d: char-index <-> display-column conversion ──────────
// The pure width helpers (char_display_width / display_width /
// char_index_to_display_col / display_col_to_char_index) live in
// `crate::model::text_width` (moved there in plan 004 issue 05e so the app
// layer does not call into the UI layer); only the render-local `truncate`
// stays here.

/// Truncate `s` to fit at most `max` terminal cells (plan 004 issue 05d):
/// wide chars straddling the boundary are dropped whole rather than
/// emitted half. `max` is in cells, not chars.
fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut used = 0usize;
    let mut out = String::new();
    for c in s.chars() {
        let w = char_display_width(c);
        if used + w > max {
            break;
        }
        used += w;
        out.push(c);
    }
    out
}

#[derive(Default, Props)]
pub struct FileViewProps {
    pub title: String,
    /// The pre-computed rendered rows (code rows + virtual annotation note
    /// rows; each carries its buffer-line index — plan 005 issue 02).
    pub rows: Vec<FileViewRow>,
    /// The total number of rendered rows for the buffer (the canvas's
    /// bottom scroll indicator; plan 005 issue 02).
    pub total_rows: usize,
    /// The first visible buffer line (window top).
    pub top_line: usize,
    pub viewport_lines: usize,
    /// The current buffer has an un-reconciled disk change (the "changed on
    /// disk" conflict marker; issue 04).
    pub changed_on_disk: bool,
    /// Whether the current buffer is editable (drives the "changed on disk"
    /// banner hint: editable → `M-x reload-buffer`, plain → `g`).
    pub buffer_editable: bool,
    /// The region's line range (start_line, end_line inclusive) in buffer
    /// line indices, or `None` when no mark is set (plan 004 issue 03).
    pub region_lines: Option<(usize, usize)>,
}

/// The "changed on disk" banner hint, accurate per buffer kind: on an
/// editable buffer plain `g` self-inserts by design (the reachable reload
/// path is `M-x reload-buffer`), while a plain file buffer reloads with `g`.
fn changed_on_disk_hint(buffer_editable: bool) -> &'static str {
    if buffer_editable {
        "  ⚠ changed on disk — press M-x reload-buffer to reload"
    } else {
        "  ⚠ changed on disk — press g to reload"
    }
}

/// The virtualized file view: titled, renders visible lines with
/// highlight spans, shows scroll indicators, and a "changed on disk" banner
/// when the current buffer has an un-reconciled disk change.
#[component]
pub fn FileView(props: &FileViewProps, mut _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(flex_grow: 1.0_f32, overflow: Overflow::Hidden) {
            View(
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0_f32,
                background_color: crate::ui::face_bg(t.view),
            ) {
                Text(
                    content: &props.title,
                    color: crate::ui::face_color(t.view_title),
                    weight: crate::ui::face_weight(t.view_title),
                )
                #(if props.changed_on_disk {
                    Some(element! {
                        Text(
                            content: changed_on_disk_hint(props.buffer_editable),
                            color: crate::ui::face_color(t.preview),
                        )
                    })
                } else {
                    None
                })
                FileViewCanvas(
                    rows: props.rows.clone(),
                    total_rows: props.total_rows,
                    top_line: props.top_line,
                    region_lines: props.region_lines,
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compute the visible line slice for a given scroll position.
    pub fn visible_slice(
        total_lines: usize,
        top_line: usize,
        viewport_lines: usize,
    ) -> (usize, usize) {
        let last = total_lines.saturating_sub(1);
        let start = top_line.min(last);
        let end = (top_line + viewport_lines).min(total_lines);
        if start >= end {
            (start.min(last), start.min(last))
        } else {
            (start, end)
        }
    }

    #[test]
    fn visible_slice_small_file() {
        let (start, end) = visible_slice(5, 0, 10);
        assert_eq!((start, end), (0, 5));
    }

    #[test]
    fn visible_slice_exact_boundary() {
        let (start, end) = visible_slice(10, 0, 10);
        assert_eq!((start, end), (0, 10));
        let (start, end) = visible_slice(100, 90, 10);
        assert_eq!((start, end), (90, 100));
    }

    #[test]
    fn visible_slice_scrolled_past_end() {
        let (start, end) = visible_slice(5, 100, 10);
        assert!(start < 5, "start must be clamped: {start}");
        assert!(end <= 5, "end must be <= total: {end}");
    }

    #[test]
    fn visible_slice_50k_line_buffer() {
        let total = 50_000;
        let viewport = 24;
        let (start, end) = visible_slice(total, 49_990, viewport);
        assert_eq!((start, end), (49_990, 50_000));
        let (start, end) = visible_slice(total, 0, viewport);
        assert_eq!((start, end), (0, 24));
        let (start, end) = visible_slice(total, 25_000, viewport);
        assert_eq!((start, end), (25_000, 25_024));
    }

    #[test]
    fn big_file_fallback_slice_math() {
        let total = 1_000_000;
        let (start, end) = visible_slice(total, 0, 24);
        assert_eq!((start, end), (0, 24));
    }

    #[test]
    fn byte_to_char_offset_ascii() {
        assert_eq!(byte_to_char_offset("hello", 0), 0);
        assert_eq!(byte_to_char_offset("hello", 5), 5);
        assert_eq!(byte_to_char_offset("hello", 10), 5);
    }

    #[test]
    fn byte_to_char_offset_multibyte() {
        assert_eq!(byte_to_char_offset("café", 0), 0);
        assert_eq!(byte_to_char_offset("café", 4), 3);
        assert_eq!(byte_to_char_offset("café", 5), 4);
    }

    // ── plan 004 issue 05d: char-index <-> display-column conversion ────
    // (the pure width helpers' tests live in `crate::model::text_width`;
    // only the render-local `truncate` stays here.)

    #[test]
    fn truncate_counts_cells_not_chars() {
        // 中 (2 cells) straddling the boundary is dropped whole.
        assert_eq!(truncate("abcd", 2), "ab");
        assert_eq!(truncate("a中b", 2), "a");
        assert_eq!(truncate("a中b", 3), "a中");
        assert_eq!(truncate("a中b", 4), "a中b", "4 cells fits exactly");
        assert_eq!(truncate("a中b", 0), "");
        assert_eq!(truncate("e\u{301}x", 2), "e\u{301}x", "combining adds no cell");
    }

    #[test]
    fn file_view_row_default_is_empty() {
        let row = FileViewRow::default();
        assert!(row.text.is_empty());
        assert!(row.spans.is_empty());
        assert!(!row.is_note);
        assert!(!row.annotated);
    }

    /// plan 005 issue 02: the rendered-row map round-trips buffer_line ↔
    /// rendered_row with interleaved note rows, both directions.
    #[test]
    fn file_view_row_map_round_trips_with_note_rows() {
        // Rows: code 0, code 1 + note (line 1), code 2, code 3 + note (line
        // 3) + note (line 3), code 4.
        let code = |line: usize| FileViewRow {
            line,
            is_note: false,
            annotated: true,
            text: format!("line {line}"),
            spans: Vec::new(),
        };
        let note = |line: usize| FileViewRow {
            line,
            is_note: true,
            annotated: false,
            text: format!("  \u{25b8} note {line}"),
            spans: Vec::new(),
        };
        let rows = vec![
            code(0),
            code(1),
            note(1),
            code(2),
            code(3),
            note(3),
            note(3),
            code(4),
        ];
        // buffer_line → rendered_row (code rows only).
        assert_eq!(FileViewRow::row_for_line(&rows, 0), Some(0));
        assert_eq!(FileViewRow::row_for_line(&rows, 1), Some(1));
        assert_eq!(FileViewRow::row_for_line(&rows, 2), Some(3));
        assert_eq!(FileViewRow::row_for_line(&rows, 3), Some(4));
        assert_eq!(FileViewRow::row_for_line(&rows, 4), Some(7));
        assert_eq!(FileViewRow::row_for_line(&rows, 5), None);
        // rendered_row → buffer_line (a note row maps to its anchored line).
        assert_eq!(FileViewRow::line_for_row(&rows, 0), Some(0));
        assert_eq!(FileViewRow::line_for_row(&rows, 1), Some(1));
        assert_eq!(FileViewRow::line_for_row(&rows, 2), Some(1), "note row → anchored line");
        assert_eq!(FileViewRow::line_for_row(&rows, 3), Some(2));
        assert_eq!(FileViewRow::line_for_row(&rows, 4), Some(3));
        assert_eq!(FileViewRow::line_for_row(&rows, 5), Some(3));
        assert_eq!(FileViewRow::line_for_row(&rows, 6), Some(3));
        assert_eq!(FileViewRow::line_for_row(&rows, 7), Some(4));
        assert_eq!(FileViewRow::line_for_row(&rows, 8), None);
        // Round-trip: for every code row, line_for_row(row_for_line(l)) == l.
        for line in 0..=4 {
            let r = FileViewRow::row_for_line(&rows, line).unwrap();
            assert_eq!(FileViewRow::line_for_row(&rows, r), Some(line));
        }
    }

    /// The "changed on disk" banner hint is accurate per buffer kind: a plain
    /// file buffer says `g`, an editable buffer says `M-x reload-buffer`
    /// (plain `g` self-inserts by design on editable buffers).
    #[test]
    fn changed_on_disk_hint_plain_says_g() {
        let hint = changed_on_disk_hint(false);
        assert!(hint.contains("press g to reload"), "plain hint must say g: {hint}");
        assert!(hint.contains("changed on disk"), "hint must keep the marker: {hint}");
        assert!(
            !hint.contains("M-x reload-buffer"),
            "plain hint must not say M-x reload-buffer: {hint}"
        );
    }

    #[test]
    fn changed_on_disk_hint_editable_says_reload_buffer() {
        let hint = changed_on_disk_hint(true);
        assert!(
            hint.contains("M-x reload-buffer"),
            "editable hint must say M-x reload-buffer: {hint}"
        );
        assert!(hint.contains("changed on disk"), "hint must keep the marker: {hint}");
        assert!(
            !hint.contains("press g to reload"),
            "editable hint must not say press g: {hint}"
        );
    }

    /// Static render test: the FileView renders the title and content.
    #[test]
    fn file_view_static_render() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn main() {\n    println!(\"hi\");\n}\n",
        )
        .unwrap();

        let base = tempfile::tempdir().unwrap();
        let mut store = crate::app::store::AppStore::at(dir.path(), base.path().to_path_buf());
        store.set_viewport_lines(24);
        store.open_path("src/main.rs");

        let mut app = element! {
            ContextProvider(value: Context::owned(Arc::new(Mutex::new(store)))) {
                Root
            }
        };
        let s = app.to_string();
        assert!(s.contains("src/main.rs"), "title missing:\n{s}");
    }
}
