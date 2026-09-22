//! Virtualized file view: renders only visible lines ± margin with
//! tree-sitter highlight spans. Uses a canvas component (like the
//! picker) for exact row/column placement of multi-colored text.
//!
//! The store pre-computes the visible lines (text + spans) for the
//! current viewport; this component only renders them.

use iocraft::{prelude::*, Component, ComponentDrawer, ComponentUpdater};

use crate::app::store::{FileViewRow, LineMatch};
use crate::model::text_width::{char_display_width, display_width};
use crate::theme;
use crate::ui::{color, text_style};

#[derive(Default, Props)]
struct FileViewCanvasProps {
    /// The pre-computed rendered rows (code rows + the virtual annotation
    /// note rows, plan 005 issue 02 — the notes render ABOVE their
    /// anchored code row, annotations-render-fold). Each row carries its
    /// buffer-line index (`r.line`), NOT its slice offset — the rows are no
    /// longer a dense 1:1 slice of buffer lines.
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
    /// annotations-fold-visual: the note blocks are folded away (`C-c a h`
    /// toggles them). The annotated-line margin arrow carries this state —
    /// ▸ (folded) instead of ▾ (shown) — so the fact that an annotation
    /// exists survives the fold.
    pub notes_folded: bool,
}

/// Canvas-backed file view: renders the visible rows with colored
/// spans and scroll indicators.
struct FileViewCanvas {
    rows: Vec<FileViewRow>,
    total_rows: usize,
    top_line: usize,
    region_lines: Option<(usize, usize)>,
    notes_folded: bool,
}

impl FileViewCanvas {
    fn from_props(props: &FileViewCanvasProps) -> Self {
        Self {
            rows: props.rows.clone(),
            total_rows: props.total_rows,
            top_line: props.top_line,
            region_lines: props.region_lines,
            notes_folded: props.notes_folded,
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
                // A virtual annotation note row (plan 005 issue 02,
                // annotations-render-fold: the note renders directly ABOVE
                // the anchored code row, dim and italic; it is only emitted
                // when the note blocks are SHOWN — a fold emits no note rows
                // at all). The row carries the 2-branch tree-line's UP arm:
                // the diagonal (\u{2571} "╱") at cell 0 slants up from the
                // code row's ▾ junction to the note, and the note text
                // begins at cell 2 — the SAME column the annotated code row's
                // text begins at — so the note and the code read as the two
                // aligned arms of one fork.
                canvas.set_text(
                    0,
                    row as isize,
                    "\u{2571}",
                    text_style(t.preview.foreground, false, false),
                );
                let note_style = text_style_italic(t.preview.foreground);
                let display = truncate(&r.text, w.saturating_sub(2));
                if !display.is_empty() {
                    canvas.set_text(2, row as isize, &display, note_style);
                }
            } else {
                // annotations-fold-visual: annotated lines get a 2-cell left
                // gutter — the fold ARROW at cell 0 and the tree-line branch
                // (a ─ when shown, blank when folded) at cell 1 — so the
                // code starts at cell 2 in BOTH the folded and the shown
                // state. That constant leading width is the no-jitter
                // guarantee: toggling a fold must not shift the code
                // horizontally. Non-annotated lines keep starting at cell 0.
                let gutter = if r.annotated { 2 } else { 0 };
                draw_line(
                    &mut canvas,
                    row as isize,
                    gutter,
                    w.saturating_sub(gutter),
                    r,
                    &t,
                );
                if r.annotated {
                    // annotations-fold-visual: the margin glyph is now the
                    // fold ARROW (the annotation indicator — the old bar
                    // ▎/▏ is gone): ▾ (\u{25be}) in the view-title face when
                    // the note is SHOWN (bright, the content is right there),
                    // ▸ (\u{25b8}) in the dim preview face when FOLDED (the
                    // face that already carries the note text and the scroll
                    // indicators on this same dark background — legible
                    // without shouting). The arrow is present in BOTH states:
                    // folding must not erase the fact that an annotation
                    // exists; an unannotated line draws nothing (guarded by
                    // `r.annotated`).
                    let (glyph, face) = if self.notes_folded {
                        ("\u{25b8}", &t.preview)
                    } else {
                        ("\u{25be}", &t.view_title)
                    };
                    canvas.set_text(
                        0,
                        row as isize,
                        glyph,
                        text_style(face.foreground, false, false),
                    );
                    if !self.notes_folded {
                        // The tree-line's straight OUT arm: a horizontal ─
                        // (\u{2500}) from the ▾ junction (cell 0) into the
                        // real text at cell 2. Together with the note row's
                        // diagonal this is the 2-branch fork. Absent when
                        // folded — cell 1 stays blank, so the code's start
                        // column (cell 2) is identical in both states.
                        canvas.set_text(
                            1,
                            row as isize,
                            "\u{2500}",
                            text_style(t.view_title.foreground, false, false),
                        );
                    }
                }
            }
        }

        // Scroll indicators: show "↑" when scrolled past the top, "↓" when
        // more rows remain below. The "↓" bound is in RENDERED-row space
        // (plan 005 issue 02): note rows count as rows, and a slice row
        // below the canvas bottom (the title/indicator overlap, as before)
        // also implies more below. The "↑" keys off the EMITTED window's
        // first buffer line (rows[0].line), not the raw scroll_top: the
        // window may have advanced start above scroll_top to keep the
        // point drawn (plan 005 issue 02b), and a hidden line 0 must still
        // read as scrolled-past-top. scroll_top semantics for the other
        // consumers are unchanged.
        let mut indicators = String::new();
        if self.rows.first().map_or(self.top_line, |r| r.line) > 0 {
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
                text_style(t.preview.foreground, false, false),
            );
        }
    }
}

/// Render one line on the canvas: the base text in the default face,
/// then overlay each span with its face color, then the match overlay
/// (issue match-highlight) substitutes the search faces over the match
/// ranges.
///
/// `x_start` is the terminal cell where the text begins (the annotation
/// gutter offset, plan 005 issue 02b). `width` is the available cell count
/// from `x_start` to the right edge (the truncation budget). `matches` are
/// this line's search-match ranges (byte offsets relative to the line
/// start; empty when no match context applies).
fn draw_line(
    canvas: &mut iocraft::CanvasSubviewMut<'_>,
    row: isize,
    x_start: usize,
    width: usize,
    file_row: &FileViewRow,
    t: &theme::Theme,
) {
    let text = &file_row.text;
    let spans = &file_row.spans;
    let matches = &file_row.matches;
    if text.is_empty() || width == 0 {
        return;
    }
    if spans.is_empty() && matches.is_empty() {
        let face = t.view;
        let style = text_style(face.foreground, false, face.bold);
        let display = truncate(text, width);
        canvas.set_text(x_start as isize, row, &display, style);
        return;
    }

    // Build the base segments: (char_start, char_end, Option<face_index>).
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

    // Issue match-highlight: the second pass — split the base segments at
    // the match boundaries and substitute the match faces (the selected
    // match's face wins on overlap).
    let segments = overlay_match_ranges(text, &segments, matches);

    // jump-highlight: the third pass — the landing-highlight range (the
    // snapshot's byte range + fade intensity, `None` when no landing
    // highlight applies to this line) substitutes the jump face, winning
    // on overlap (the newest information is the landing itself).
    let segments = overlay_jump_range(text, &segments, file_row.highlight);

    let mut x = x_start as isize;
    for (cs, ce, face) in &segments {
        if *cs >= *ce || *cs >= total_chars {
            continue;
        }
        let end = (*ce).min(total_chars);
        let segment: String = chars[*cs..end].iter().collect();
        let used = (x - x_start as isize) as usize;
        let remaining = width.saturating_sub(used);
        let segment = truncate(&segment, remaining);
        if segment.is_empty() {
            continue;
        }
        // The band: the selected match's background (issue match-highlight)
        // or the landing-highlight fade band (jump-highlight — interpolated
        // toward the base background by intensity under truecolor).
        let (face, band) = match face {
            RowFace::View => (t.view, None),
            RowFace::Syntax(idx) => (t.syntax_face(*idx), None),
            RowFace::Match => (t.search_match, None),
            RowFace::MatchCurrent => (
                t.search_match_current,
                Some(color(t.search_match_current.background)),
            ),
            RowFace::Jump(intensity) => (t.jump_highlight, Some(crate::ui::jump_band_bg(t, *intensity))),
        };
        // The selected match's background band (issue match-highlight):
        // the user's complaint was the cursor being hard to see when
        // jumping to a search result — the inverse-video band under the
        // matched text is the prominence the fg-only faces can't give.
        if let Some(bg) = band {
            canvas.set_background_color(x, row, display_width(&segment), 1, bg);
        }
        let style = text_style(face.foreground, false, face.bold);
        canvas.set_text(x, row, &segment, style);
        // Advance by DISPLAY width, not char count: a segment ending in a
        // wide char occupies one more cell than its char count, and the
        // next segment must start where the terminal actually is
        // (plan 004 issue 05d — the dropped-space-after-wide-char bug).
        x += display_width(&segment) as isize;
        if (x - x_start as isize) >= width as isize {
            break;
        }
    }
}

/// A cell face in the match overlay's second pass (issue match-highlight):
/// the base face (view or syntax) with the search-match faces substituted
/// over the match ranges — `Match` for the query's other matches,
/// `MatchCurrent` for the match the cursor is on (wins on overlap) — plus
/// the landing-highlight face (jump-highlight), `Jump` with its frame's
/// fade intensity, which wins on overlap in the third pass.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RowFace {
    View,
    Syntax(usize),
    Match,
    MatchCurrent,
    Jump(f32),
}

/// The match overlay (issue match-highlight): split the base syntax
/// segments at the match boundaries and substitute the match face over the
/// overlapping ranges. `matches` are LINE-RELATIVE BYTE ranges (the same
/// byte domain as the syntax spans) — they go through `byte_to_char_offset`
/// for the char-indexed segments, so a match after a multibyte character
/// highlights the right cells (a byte/char mix-up is the same bug class as
/// the isearch column fix). The selected face wins where matches overlap;
/// a range running past the text's end clips (byte_to_char_offset floors
/// to the last char) instead of panicking. Syntax faces outside the match
/// ranges are untouched.
fn overlay_match_ranges(
    text: &str,
    base: &[(usize, usize, Option<usize>)],
    matches: &[LineMatch],
) -> Vec<(usize, usize, RowFace)> {
    let total = text.chars().count();
    // Byte ranges → char ranges up front (both ends clamped to the text
    // length by byte_to_char_offset).
    let char_ranges: Vec<(usize, usize, bool)> = matches
        .iter()
        .map(|m| {
            (
                byte_to_char_offset(text, m.start),
                byte_to_char_offset(text, m.end),
                m.selected,
            )
        })
        .filter(|&(s, e, _)| s < e)
        .collect();
    if char_ranges.is_empty() {
        return base
            .iter()
            .map(|&(cs, ce, face)| {
                (
                    cs.min(total),
                    ce.min(total),
                    match face {
                        Some(i) => RowFace::Syntax(i),
                        None => RowFace::View,
                    },
                )
            })
            .filter(|&(s, e, _)| s < e)
            .collect();
    }
    // Per-char winning face: the base face, overridden by the match faces
    // — a char inside BOTH a selected and a plain match takes the
    // selected face (selected wins on overlap, the isearch "aa"-in-
    // "aaa" overlap class). The ranges are start-ascending, so each char
    // scans only the ranges that can still cover it.
    let mut faces = vec![RowFace::View; total];
    for (cs, ce, face) in base {
        let cs = (*cs).min(total);
        let ce = (*ce).min(total);
        let own = match face {
            Some(i) => RowFace::Syntax(*i),
            None => RowFace::View,
        };
        faces[cs..ce].fill(own);
    }
    for (ms, me, sel) in &char_ranges {
        let sel_face = if *sel { RowFace::MatchCurrent } else { RowFace::Match };
        for f in &mut faces[*ms..(*me).min(total)] {
            // The selected face overwrites anything; the plain face
            // overwrites only the base faces (it must not demote a
            // selected match's coverage).
            if *f != RowFace::MatchCurrent || *sel {
                *f = sel_face;
            }
        }
    }
    // Merge runs of equal faces back into segments.
    let mut out: Vec<(usize, usize, RowFace)> = Vec::new();
    for (c, &face) in faces.iter().enumerate() {
        if out.last().is_some_and(|(_, _, f)| *f == face) {
            out.last_mut().unwrap().1 = c + 1;
        } else {
            out.push((c, c + 1, face));
        }
    }
    out
}

/// jump-highlight: the landing-highlight overlay (the third pass, after
/// the match overlay): split the segments at the landing range's boundaries
/// and substitute the jump face (with the frame's fade intensity) over the
/// overlapping cells. `jump` is `Some((start, end, intensity))` with
/// LINE-RELATIVE BYTE offsets (the span layer's byte domain — it goes
/// through `byte_to_char_offset` for the char-indexed segments, so a
/// landing after a multibyte character highlights the right cells, the
/// same byte/char discipline as the match overlay). The jump face wins on
/// overlap (the landing pulse is the newest information); a range running
/// past the text's end clips instead of panicking. When no landing
/// highlight applies (`jump` is `None`), clamps segment endpoints to the
/// text length and drops zero-width segments (the same clamping `draw_line`
/// applies — not a pass-through).
fn overlay_jump_range(
    text: &str,
    segments: &[(usize, usize, RowFace)],
    jump: Option<(usize, usize, f32)>,
) -> Vec<(usize, usize, RowFace)> {
    let total = text.chars().count();
    let Some((start, end, intensity)) = jump else {
        return segments
            .iter()
            .map(|&(cs, ce, face)| (cs.min(total), ce.min(total), face))
            .filter(|&(s, e, _)| s < e)
            .collect();
    };
    let (s, e) = (byte_to_char_offset(text, start), byte_to_char_offset(text, end));
    if s >= e {
        // An empty extent (start == end after the char conversion) sets no
        // highlight — never a zero-width band.
        return segments
            .iter()
            .map(|&(cs, ce, face)| (cs.min(total), ce.min(total), face))
            .filter(|&(s, e, _)| s < e)
            .collect();
    }
    let mut out: Vec<(usize, usize, RowFace)> = Vec::new();
    for (cs, ce, face) in segments {
        let cs = (*cs).min(total);
        let ce = (*ce).min(total);
        if ce <= s || cs >= ce {
            if ce > cs {
                out.push((cs, ce, *face));
            }
            continue;
        }
        if cs >= e {
            out.push((cs, ce, *face));
            continue;
        }
        // The segment crosses the range: keep the left tail, replace the
        // covered cells with the jump face, keep the right tail.
        if cs < s {
            out.push((cs, s, *face));
        }
        out.push((s.max(cs), e.min(ce), RowFace::Jump(intensity)));
        if e < ce {
            out.push((e, ce, *face));
        }
    }
    out
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

/// A dim + italic face style (the annotation note rows, plan 005 issue 02).
fn text_style_italic(foreground: theme::Color) -> CanvasTextStyle {
    let mut style = text_style(foreground, false, false);
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
    /// rows ABOVE their anchored code rows; each carries its buffer-line
    /// index — plan 005 issue 02, annotations-render-fold).
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
    /// annotations-fold-visual: the note blocks are folded away (the
    /// annotated-line margin arrow switches to its folded ▸ state, the
    /// tree-line's up/out arms disappear, and no note rows are emitted).
    pub notes_folded: bool,
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
                    notes_folded: props.notes_folded,
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
        assert!(row.matches.is_empty());
        assert!(row.highlight.is_none());
        assert!(!row.is_note);
        assert!(!row.annotated);
    }

    // ── issue match-highlight: the match overlay's second pass ─────

    /// (a) A line with two matches produces two match segments, the
    /// selected one flagged `MatchCurrent`, the other `Match`; the rest of
    /// the line keeps its base face. Discriminates: without the overlay the
    /// whole line would be one View segment.
    #[test]
    fn overlay_two_matches_selected_flagged() {
        // "foo a foo b": matches at bytes 0..3 and 6..9; the second is
        // selected.
        let text = "foo a foo b";
        let base: Vec<(usize, usize, Option<usize>)> = vec![(0, 11, None)];
        let matches = vec![
            LineMatch {
                start: 0,
                end: 3,
                selected: false,
            },
            LineMatch {
                start: 6,
                end: 9,
                selected: true,
            },
        ];
        let segs = overlay_match_ranges(text, &base, &matches);
        assert_eq!(
            segs,
            vec![
                (0, 3, RowFace::Match),
                (3, 6, RowFace::View),
                (6, 9, RowFace::MatchCurrent),
                (9, 11, RowFace::View),
            ]
        );
    }

    /// The match overlay must not disturb syntax faces OUTSIDE the match
    /// ranges: a syntax span straddling a match is split, but the
    /// non-overlapping halves keep their syntax face.
    #[test]
    fn overlay_preserves_syntax_faces_outside_matches() {
        // "hello world": a syntax span over 0..5 ("hello"), a match over
        // 6..11 ("world").
        let text = "hello world";
        let base: Vec<(usize, usize, Option<usize>)> =
            vec![(0, 5, Some(4)), (5, 11, None)];
        let matches = vec![LineMatch {
            start: 6,
            end: 11,
            selected: false,
        }];
        let segs = overlay_match_ranges(text, &base, &matches);
        assert_eq!(
            segs,
            vec![
                (0, 5, RowFace::Syntax(4)),
                (5, 6, RowFace::View),
                (6, 11, RowFace::Match),
            ]
        );
    }

    /// The selected face wins where a selected and a plain match overlap
    /// (isearch's non-anchored find can return overlapping starts for
    /// repeated queries like "aa" in "aaa").
    #[test]
    fn overlay_selected_face_wins_on_overlap() {
        // "aa b aa": plain match 0..2, selected match 1..5 (overlapping
        // start).
        let text = "aa b aa";
        let base: Vec<(usize, usize, Option<usize>)> = vec![(0, 7, None)];
        let matches = vec![
            LineMatch {
                start: 0,
                end: 2,
                selected: false,
            },
            LineMatch {
                start: 1,
                end: 5,
                selected: true,
            },
        ];
        let segs = overlay_match_ranges(text, &base, &matches);
        // Overlap [1,2) must be the SELECTED face, not the plain one.
        assert_eq!(
            segs,
            vec![
                (0, 1, RowFace::Match),
                (1, 5, RowFace::MatchCurrent),
                (5, 7, RowFace::View),
            ]
        );
    }

    /// (d) A multibyte line: the highlighted char range is the right one
    /// (byte ≠ char). "caf\u{e9} caf\u{e9}": the second "café" is bytes 6..11
    /// but chars 5..9 — a byte-indexed overlay would highlight char 6..9,
    /// i.e. the wrong cells (shifted one cell right).
    #[test]
    fn overlay_multibyte_highlights_the_right_chars() {
        let text = "caf\u{e9} caf\u{e9}"; // 9 chars, 11 bytes
        let base: Vec<(usize, usize, Option<usize>)> = vec![(0, 9, None)];
        let matches = vec![LineMatch {
            start: 6,
            end: 11,
            selected: false,
        }];
        let segs = overlay_match_ranges(text, &base, &matches);
        assert_eq!(
            segs,
            vec![
                (0, 5, RowFace::View),
                (5, 9, RowFace::Match),
            ]
        );
        // Sanity: the highlighted chars really are the second café.
        let chars: Vec<char> = text.chars().collect();
        let highlighted: String = chars[5..9].iter().collect();
        assert_eq!(highlighted, "caf\u{e9}");
    }

    /// (e) A range running past the line's end (stale context, or a range
    /// that was never clipped) clips to the text end without panicking —
    /// the render loop then clips the rest by the visible width.
    #[test]
    fn overlay_range_beyond_text_end_clips_without_panicking() {
        let text = "short";
        let base: Vec<(usize, usize, Option<usize>)> = vec![(0, 5, None)];
        let matches = vec![LineMatch {
            start: 1,
            end: 999,
            selected: true,
        }];
        let segs = overlay_match_ranges(text, &base, &matches);
        assert_eq!(
            segs,
            vec![
                (0, 1, RowFace::View),
                (1, 5, RowFace::MatchCurrent),
            ]
        );
    }

    /// No matches: the overlay is a pure face rewrite (syntax faces
    /// survive, nothing else moves).
    #[test]
    fn overlay_no_matches_leaves_base_faces() {
        let text = "abc";
        let base: Vec<(usize, usize, Option<usize>)> = vec![(0, 3, Some(2))];
        let segs = overlay_match_ranges(text, &base, &[]);
        assert_eq!(segs, vec![(0, 3, RowFace::Syntax(2))]);
    }

    // ── jump-highlight: the landing-highlight overlay (third pass) ─────

    /// jump-highlight: the landing range substitutes the jump face (with
    /// the frame's intensity) over the covered cells; the rest of the line
    /// keeps its base face. Discriminates: without the third pass the
    /// whole line would stay base faces.
    #[test]
    fn overlay_jump_range_substitutes_jump_face() {
        let text = "fn alpha() {";
        let base: Vec<(usize, usize, RowFace)> = vec![(0, 12, RowFace::View)];
        let segs = overlay_jump_range(text, &base, Some((3, 8, 1.0)));
        assert_eq!(
            segs,
            vec![
                (0, 3, RowFace::View),
                (3, 8, RowFace::Jump(1.0)),
                (8, 12, RowFace::View),
            ]
        );
    }

    /// jump-highlight: the jump face WINS where a search match overlaps the
    /// landing range (the newest information is the landing itself).
    #[test]
    fn overlay_jump_range_wins_over_match() {
        let text = "foo a foo";
        // A plain search match over the first "foo", landing over the
        // second.
        let base: Vec<(usize, usize, RowFace)> = vec![
            (0, 3, RowFace::Match),
            (3, 9, RowFace::View),
        ];
        let segs = overlay_jump_range(text, &base, Some((6, 9, 0.5)));
        assert_eq!(
            segs,
            vec![
                (0, 3, RowFace::Match),
                (3, 6, RowFace::View),
                (6, 9, RowFace::Jump(0.5)),
            ]
        );
    }

    /// jump-highlight multibyte: the range is BYTE offsets (the span
    /// layer's domain) — "caf\u{e9} caf\u{e9}", landing on the second
    /// caf\u{e9} (bytes 6..11, chars 5..9). A char-indexed (or byte-as-char)
    /// overlay would highlight the wrong cells.
    #[test]
    fn overlay_jump_range_multibyte_highlights_the_right_chars() {
        let text = "caf\u{e9} caf\u{e9}"; // 9 chars, 11 bytes
        let base: Vec<(usize, usize, RowFace)> = vec![(0, 9, RowFace::View)];
        let segs = overlay_jump_range(text, &base, Some((6, 11, 1.0)));
        assert_eq!(
            segs,
            vec![(0, 5, RowFace::View), (5, 9, RowFace::Jump(1.0))]
        );
        let chars: Vec<char> = text.chars().collect();
        let highlighted: String = chars[5..9].iter().collect();
        assert_eq!(highlighted, "caf\u{e9}", "the highlighted chars are the landing symbol");
    }

    /// jump-highlight: with no landing highlight (`None`), an in-range segment
    /// is returned exactly as it came in.
    #[test]
    fn overlay_jump_range_none_is_identity_for_in_range_segments() {
        let text = "abc";
        let base: Vec<(usize, usize, RowFace)> = vec![(0, 3, RowFace::Syntax(2))];
        assert_eq!(overlay_jump_range(text, &base, None), base);
    }

    /// jump-highlight: the `None` path is **not** a pass-through — it clamps
    /// endpoints to the text length and drops zero-width segments (the same
    /// clamping `draw_line` applies). Pinned separately from the identity case
    /// above, because a doc that says "returns the input unchanged" is exactly
    /// the claim this branch falsifies.
    #[test]
    fn overlay_jump_range_none_clamps_out_of_range_segments() {
        let text = "abc";
        let base: Vec<(usize, usize, RowFace)> = vec![(0, 9, RowFace::Syntax(2))];
        assert_eq!(
            overlay_jump_range(text, &base, None),
            vec![(0, 3, RowFace::Syntax(2))]
        );
    }

    /// plan 005 issue 02 + annotations-render-fold: the rendered-row map
    /// round-trips buffer_line ↔ rendered_row with interleaved note rows,
    /// both directions — with the note rows ABOVE their code rows.
    /// The ordering assertion is what makes this test mean something: a
    /// test that only checked "a note row exists" would pass under either
    /// ordering and prove nothing (the row map itself is order-agnostic —
    /// it keys on `line`), so the note row's position relative to its code
    /// row is pinned here, immediately-before, per row.
    #[test]
    fn file_view_row_map_round_trips_with_note_rows() {
        // Rows (note BEFORE its code row): code 0, code 1 with note (line
        // 1) above it, code 2, code 3 + note (line 3) + note (line 3) above
        // it, code 4.
        let code = |line: usize| FileViewRow {
            line,
            is_note: false,
            annotated: true,
            text: format!("line {line}"),
            spans: Vec::new(),
            matches: Vec::new(),
            highlight: None,
        };
        let note = |line: usize| FileViewRow {
            line,
            is_note: true,
            annotated: false,
            text: format!("  \u{25b8} note {line}"),
            spans: Vec::new(),
            matches: Vec::new(),
            highlight: None,
        };
        let rows = vec![
            code(0),
            note(1),
            code(1),
            code(2),
            note(3),
            note(3),
            code(3),
            code(4),
        ];
        // annotations-render-fold DISCRIMINATOR: every note row sits
        // IMMEDIATELY BEFORE its own code row (the note is the header, not
        // the trailer) — the nearest subsequent CODE row is the note's own
        // line. This fails under the old note-below ordering (where the
        // next code row after a note is the NEXT line's code row) — a mere
        // "a note row exists" assertion would pass either way.
        for (i, r) in rows.iter().enumerate() {
            if r.is_note {
                let next_code = rows[i + 1..]
                    .iter()
                    .find(|r| !r.is_note)
                    .unwrap_or_else(|| panic!(
                        "note row {i} has no code row after it — it cannot be ABOVE its code row"
                    ));
                assert_eq!(next_code.line, r.line, "note row {i} must sit directly above its own code row (the nearest code row below it)");
            }
        }
        // buffer_line → rendered_row (code rows only).
        assert_eq!(FileViewRow::row_for_line(&rows, 0), Some(0));
        assert_eq!(FileViewRow::row_for_line(&rows, 1), Some(2));
        assert_eq!(FileViewRow::row_for_line(&rows, 2), Some(3));
        assert_eq!(FileViewRow::row_for_line(&rows, 3), Some(6));
        assert_eq!(FileViewRow::row_for_line(&rows, 4), Some(7));
        assert_eq!(FileViewRow::row_for_line(&rows, 5), None);
        // rendered_row → buffer_line (a note row maps to its anchored line).
        assert_eq!(FileViewRow::line_for_row(&rows, 0), Some(0));
        assert_eq!(FileViewRow::line_for_row(&rows, 1), Some(1), "note row → anchored line");
        assert_eq!(FileViewRow::line_for_row(&rows, 2), Some(1));
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

    /// annotations-fold-visual: NO JITTER — the code text's start column is
    /// IDENTICAL with the note SHOWN and FOLDED. Folded is the ▸ arrow, a
    /// blank cell, then the code; shown is the ▾ arrow, the ─ tree-line
    /// branch, then the code (the 2-branch fork). Both put the code at cell
    /// 2, so a fold toggle must not shift the code horizontally. This pins
    /// the exact trap a row-map test cannot catch. The row map is
    /// order/column agnostic, so it would pass even if the leading width
    /// drifted on a fold.
    #[test]
    fn fold_arrow_no_jitter_code_start_column_constant() {
        use crate::app::keymap::{parse_key, Key};
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        /// The display column (in cells) where `needle` begins in its line of
        /// the rendered frame: the cell count of everything before it.
        fn code_start_col(frame: &str, needle: &str) -> Option<usize> {
            let line = frame.lines().find(|l| l.contains(needle))?;
            let byte = line.find(needle)?;
            Some(crate::model::text_width::display_width(&line[..byte]))
        }

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/jit.rs"),
            "fn foo() {\n    let x = 1;\n}\n",
        )
        .unwrap();

        let base = tempfile::tempdir().unwrap();
        let mut store = crate::app::store::AppStore::at(dir.path(), base.path().to_path_buf());
        store.set_viewport_lines(24);
        store.open_path("src/jit.rs");
        // Annotate line 0 through the public key path (A, type, RET).
        store.key_event(parse_key("A").unwrap());
        for c in "my note".chars() {
            store.key_event(if c == ' ' {
                Key::char(' ')
            } else {
                parse_key(&c.to_string()).unwrap()
            });
        }
        store.key_event(parse_key("RET").unwrap());
        assert!(!store.note_rows_folded(), "notes shown by default");

        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let shown = app.to_string();
        let shown_col = code_start_col(&shown, "fn foo() {").unwrap_or_else(|| {
            panic!("code line not found in the SHOWN frame:\n{shown}")
        });

        // Fold: the note row goes, the arrow ▾→▸, the tree-line arms vanish —
        // but the code's start column must NOT move.
        {
            let mut st = shared.lock().unwrap();
            st.key_event(parse_key("C-c").unwrap());
            st.key_event(parse_key("a").unwrap());
            st.key_event(parse_key("h").unwrap());
        }
        let folded = app.to_string();
        let folded_col = code_start_col(&folded, "fn foo() {").unwrap_or_else(|| {
            panic!("code line not found in the FOLDED frame:\n{folded}")
        });

        // The no-jitter invariant: same code start column, both states.
        assert_eq!(
            shown_col, folded_col,
            "JITTER: the code's start column moved on a fold toggle — shown={shown_col} folded={folded_col}\nSHOWN line:\n{shown}\nFOLDED line:\n{folded}"
        );
        // And it is the 2-cell gutter (arrow + branch/blank), cell 2.
        assert_eq!(shown_col, 2, "code must start at cell 2 (the 2-cell gutter): {shown_col}");
        // The fold actually changed something (the note row / tree-line arms
        // are gone) — otherwise this test would be vacuous (it would pass
        // with the two frames identical).
        let note_present_shown = shown.lines().any(|l| l.contains("my note"));
        let note_present_folded = folded.lines().any(|l| l.contains("my note"));
        assert!(
            note_present_shown && !note_present_folded,
            "the note row must be present SHOWN and absent FOLDED (otherwise the two frames are identical and this test is vacuous): shown={note_present_shown} folded={note_present_folded}"
        );
    }
}
