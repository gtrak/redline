//! Virtualized file view: renders only visible lines ± margin with
//! tree-sitter highlight spans. Uses a canvas component (like the
//! picker) for exact row/column placement of multi-colored text.
//!
//! The store pre-computes the visible lines (text + spans) for the
//! current viewport; this component only renders them.

use iocraft::{prelude::*, Component, ComponentDrawer, ComponentUpdater};

use crate::app::store::FileViewLine;
use crate::theme;
use crate::ui::color;

#[derive(Default, Props)]
struct FileViewCanvasProps {
    pub lines: Vec<FileViewLine>,
    pub total_lines: usize,
    pub top_line: usize,
    /// The region's line range (start_line, end_line inclusive) in buffer
    /// line indices, or `None` when no mark is set. The store computes this
    /// from the byte range using the rope (plan 004 issue 03).
    pub region_lines: Option<(usize, usize)>,
}

/// Canvas-backed file view: renders the visible lines with colored
/// spans and scroll indicators.
struct FileViewCanvas {
    lines: Vec<FileViewLine>,
    total_lines: usize,
    top_line: usize,
    region_lines: Option<(usize, usize)>,
}

impl FileViewCanvas {
    fn from_props(props: &FileViewCanvasProps) -> Self {
        Self {
            lines: props.lines.clone(),
            total_lines: props.total_lines,
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

        for (row, line) in self.lines.iter().enumerate() {
            if row >= h {
                break;
            }
            let buffer_line = self.top_line + row;
            // Paint the region background for lines within the region.
            if let Some((rl_start, rl_end)) = self.region_lines
                && (rl_start..=rl_end).contains(&buffer_line)
            {
                let bg = color(t.region.background);
                canvas.set_background_color(0, row as isize, w, 1, bg);
            }
            draw_line(&mut canvas, row as isize, w, &line.text, &line.spans, &t);
        }

        // Scroll indicators: show "↑" when scrolled past the top,
        // "↓" when more lines remain below.
        let mut indicators = String::new();
        if self.top_line > 0 {
            indicators.push('↑');
        }
        if self.top_line + h < self.total_lines {
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

// ── plan 004 issue 05d: char-index <-> display-column conversion ──────────

/// The terminal-cell width of one char: wide (CJK) chars are 2, combining
/// marks 0; `unicode-width`'s `None` (unknown width) is treated as 1.
fn char_display_width(c: char) -> usize {
    use unicode_width::UnicodeWidthChar;
    c.width().unwrap_or(1)
}

/// The number of terminal cells `s` occupies when rendered at display
/// column 0 (plan 004 issue 05d).
pub fn display_width(s: &str) -> usize {
    s.chars().map(char_display_width).sum()
}

/// The display (cell) column of char index `idx` in `line` — the display
/// width of the prefix `[0, idx)`; `idx` beyond EOL clamps to EOL
/// (plan 004 issue 05d). Combining marks (width 0) share their base
/// char's column.
pub fn char_index_to_display_col(line: &str, idx: usize) -> usize {
    line.chars().take(idx).map(char_display_width).sum()
}

/// The char index a clicked display column `col` refers to in `line`
/// (plan 004 issue 05d): a column inside a wide char maps to that char, a
/// column exactly at a char boundary maps to the following char, and a
/// column at or past the line's total width maps to EOL (the char count).
pub fn display_col_to_char_index(line: &str, col: usize) -> usize {
    let mut acc = 0usize;
    for (i, c) in line.chars().enumerate() {
        let w = char_display_width(c);
        if acc + w > col {
            return i;
        }
        acc += w;
    }
    line.chars().count()
}

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
    pub lines: Vec<FileViewLine>,
    pub total_lines: usize,
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
                    lines: props.lines.clone(),
                    total_lines: props.total_lines,
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

    #[test]
    fn display_width_counts_wide_and_combining() {
        assert_eq!(display_width("abcd"), 4);
        assert_eq!(display_width(""), 0);
        assert_eq!(display_width("中中"), 4);
        assert_eq!(display_width("a中b"), 4);
        // Combining marks add no cell (e + U+0301 = 1 cell).
        assert_eq!(display_width("e\u{301}"), 1);
        assert_eq!(display_width("CJK: abcd中 efgh"), 16);
    }

    #[test]
    fn char_index_to_display_col_ascii() {
        assert_eq!(char_index_to_display_col("abcd", 0), 0);
        assert_eq!(char_index_to_display_col("abcd", 3), 3);
        assert_eq!(char_index_to_display_col("abcd", 99), 4, "clamps to EOL");
    }

    #[test]
    fn char_index_to_display_col_wide() {
        // 中 is char 9 and occupies display cols 9-10.
        let line = "CJK: abcd中 efgh";
        assert_eq!(char_index_to_display_col(line, 0), 0);
        assert_eq!(char_index_to_display_col(line, 9), 9);
        assert_eq!(char_index_to_display_col(line, 10), 11, "after the wide char");
        assert_eq!(char_index_to_display_col(line, 13), 14, "'g'");
        assert_eq!(char_index_to_display_col(line, 15), 16, "EOL");
    }

    #[test]
    fn char_index_to_display_col_combining() {
        // e, combining acute (width 0), x: the combining char shares its
        // base's cell.
        let line = "e\u{301}x";
        assert_eq!(char_index_to_display_col(line, 1), 1);
        assert_eq!(char_index_to_display_col(line, 2), 1);
        assert_eq!(char_index_to_display_col(line, 3), 2);
    }

    #[test]
    fn display_col_to_char_index_ascii() {
        assert_eq!(display_col_to_char_index("abcd", 0), 0);
        assert_eq!(display_col_to_char_index("abcd", 3), 3);
        assert_eq!(display_col_to_char_index("abcd", 4), 4, "at width → EOL");
        assert_eq!(display_col_to_char_index("abcd", 99), 4, "past width → EOL");
    }

    #[test]
    fn display_col_to_char_index_wide() {
        // 中 occupies display cols 9-10 (char 9); 'g' (char 13) sits at
        // display col 14.
        let line = "CJK: abcd中 efgh";
        assert_eq!(display_col_to_char_index(line, 0), 0);
        assert_eq!(display_col_to_char_index(line, 8), 8);
        assert_eq!(display_col_to_char_index(line, 9), 9, "inside 中 (first cell)");
        assert_eq!(display_col_to_char_index(line, 10), 9, "inside 中 (second cell)");
        assert_eq!(display_col_to_char_index(line, 11), 10, "space");
        assert_eq!(display_col_to_char_index(line, 14), 13, "'g'");
        assert_eq!(display_col_to_char_index(line, 16), 15, "at width → EOL");
    }

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
    fn file_view_line_default_is_empty() {
        let line = FileViewLine::default();
        assert!(line.text.is_empty());
        assert!(line.spans.is_empty());
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
