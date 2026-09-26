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
use crate::ui::{color, current_line_bg, text_style};

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
    /// issue-current-line-highlight: the BUFFER line index the point is on
    /// (the row that receives the current-line tint; plumbed like
    /// `region_lines` — the store owns the point, this only carries it). A
    /// line not present in `rows` tints nothing (the point off-screen).
    pub point_line: usize,
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
    point_line: usize,
    notes_folded: bool,
}

impl FileViewCanvas {
    fn from_props(props: &FileViewCanvasProps) -> Self {
        Self {
            rows: props.rows.clone(),
            total_rows: props.total_rows,
            top_line: props.top_line,
            region_lines: props.region_lines,
            point_line: props.point_line,
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
            // issue-current-line-highlight: the current-line TINT — a subtle
            // backdrop on the row carrying the point. Painted FIRST (before
            // the region face and before `draw_line`'s match/jump bands,
            // which overwrite it per cell), so it is the LOWEST precedence:
            // jump band > match band > region face > tint > view background.
            // Code rows only — a synthetic note row never carries the tint
            // (the same P3-3 rule as the region: the face belongs to rows
            // that carry buffer text). A point line absent from `rows`
            // tints nothing (the point off-screen). When the notes are
            // folded no note rows are emitted at all, so the tint simply
            // lands on the annotated code row — it is the row that carries
            // the buffer text either way.
            if !r.is_note && r.line == self.point_line {
                let bg = current_line_bg(&t);
                canvas.set_background_color(0, row as isize, w, 1, bg);
            }
            // Paint the region background for rows whose BUFFER line is
            // within the region. Note rows are EXCLUDED (P3-3): a note row
            // is synthetic — it carries no buffer text, so highlighting it
            // would promise text that `M-w` never copies (the gate called
            // this "the one place where the highlight and the copy
            // disagree"). The highlight means "this text will be copied",
            // and only code rows carry copyable text.
            if !r.is_note
                && let Some((rl_start, rl_end)) = self.region_lines
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
                // at all). issue-annotations-symbol-precise: the note's
                // indent mirrors the nesting of the code it annotates — the
                // curved corner (\u{256d} "╭") sits at THIS note's record's
                // ANCHOR column, the SAME cell as the ▴ of that record on
                // the code row directly below (the anchor relationship that
                // makes the branch read as attached), whose stroke comes up
                // from that junction and bends right into the straight ─
                // (\u{2500}) leader, then the note text.
                // issue-annotations-layout: a row may carry SEVERAL note
                // slots (a PACKED note row — the records whose display-cell
                // footprints do not collide share the row, each ╭ still at
                // its own anchor cell, the anchor relationship per slot);
                // each slot draws its own corner, its ─ leader, and its
                // text. The store (`pack_line_note_rows`) owns the packing
                // and the leader lengths: the plain slot has a single bend
                // (text at `anchor + 2`); the FURTHER-OUT slot — the
                // larger-anchor note whose base footprint collides with an
                // earlier note's text — carries the longer leader, so its
                // text starts strictly to the right of the colliding note's
                // text end and the connector still reads as reaching its
                // own anchor. A slot's text is truncated to the row width
                // AND to the next slot's own cells — a note is never
                // truncated away or overwritten by a neighbour.
                let note_style = text_style_italic(t.preview.foreground);
                let slots: Vec<(usize, usize, &str)> = if r.note_slots.is_empty() {
                    // A hand-built legacy note row (the tests' row-map
                    // fixtures): one plain slot from the row's own anchor
                    // and text — the pre-packing shape.
                    vec![(
                        r.anchors.first().copied().unwrap_or(0),
                        1,
                        r.text.as_str(),
                    )]
                } else {
                    r.note_slots
                        .iter()
                        .map(|s| (s.anchor, s.leader, s.text.as_str()))
                        .collect()
                };
                for (k, (anchor, leader, text)) in slots.iter().enumerate() {
                    canvas.set_text(
                        *anchor as isize,
                        row as isize,
                        "\u{256d}",
                        text_style(t.preview.foreground, false, false),
                    );
                    // The leader: the ─ run from `anchor + 1` to just
                    // before the text (one cell for the plain single bend).
                    let note_start = anchor + 1 + leader;
                    for cell in (anchor + 1)..note_start {
                        canvas.set_text(
                            cell as isize,
                            row as isize,
                            "\u{2500}",
                            text_style(t.preview.foreground, false, false),
                        );
                    }
                    // The neighbour guard: on a packed row the text may not
                    // reach the next slot's own cells (its ╭ at
                    // `next.anchor`) — truncate at the earlier of the row
                    // width and that boundary. Defence-in-depth, not the
                    // real guarantee: store-built rows never reach it —
                    // `pack_line_note_rows`'s first-fit disjointness is
                    // pinned in the store tests (the gate's 2,743-check
                    // sweep found 0 violations of
                    // `slot[k].anchor + 1 + leader + width(text) <=
                    // slot[k+1].anchor` for store-built rows) — so for them
                    // this min never binds; it is reachable for hand-built
                    // rows only, pinned by
                    // `note_packing_neighbour_guard_truncates_at_the_next_slot`.
                    let mut avail = w.saturating_sub(note_start);
                    if let Some((next_anchor, _, _)) = slots.get(k + 1) {
                        avail = avail.min(next_anchor.saturating_sub(note_start));
                    }
                    let display = truncate(text, avail);
                    if !display.is_empty() {
                        canvas.set_text(
                            note_start as isize,
                            row as isize,
                            &display,
                            note_style,
                        );
                    }
                }
            } else {
                // issue-annotations-symbol-precise: the row's `text` is
                // drawn at `code_start` — the line's leading indentation
                // run's display width (the code keeps its source column;
                // every indicator sits in a blank cell) or column 1 when a
                // column-0 line shifts right by exactly one cell for a
                // column-0 indicator (a mid-line-only indicator overwrites
                // a blank cell and the line stays at column 0). The row's
                // `text` has the leading run stripped (the store owns it);
                // non-annotated lines keep their full text at column 0.
                let gutter = if r.annotated { r.code_start } else { 0 };
                draw_line(
                    &mut canvas,
                    row as isize,
                    gutter,
                    w.saturating_sub(gutter),
                    r,
                    &t,
                );
                if r.annotated {
                    // The annotation INDICATORS, one per DISTINCT anchor
                    // (`r.anchors` — issue-annotations-symbol-precise, the
                    // code row carries a SET, replacing the landed single
                    // anchor): each record's ▴ sits before its own symbol.
                    // ▴ (\u{25b4}, up) in the view-title face when the note
                    // is SHOWN — it points at the note, which is ABOVE — and
                    // ▸ (\u{25b8}) in the dim preview face when FOLDED (the
                    // face that already carries the note text and the
                    // scroll indicators on this dark background). The arrow
                    // is present in BOTH states: folding must not erase the
                    // fact that an annotation exists; an unannotated line
                    // draws nothing (guarded by `r.annotated`).
                    let (glyph, face) = if self.notes_folded {
                        ("\u{25b8}", &t.preview)
                    } else {
                        ("\u{25b4}", &t.view_title)
                    };
                    for &a in &r.anchors {
                        canvas.set_text(
                            a as isize,
                            row as isize,
                            glyph,
                            text_style(face.foreground, false, false),
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

/// issue-annotation-marker-cell: the display column (in cells, RELATIVE
/// to the row text's start) of char index `idx` when an annotation
/// marker cell is INSERTED before every char index in `gaps` (sorted
/// ascending — one per record whose marker could not overwrite a blank
/// cell). Each gap pushes the char at its index and every char after it
/// right by one cell; a gap AT `idx` sits before that char, so it counts
/// too (hence `<=` — the same rule `cursor_cell` applies to the point's
/// own char). This is the mid-line re-base the whole line's highlight
/// spans and search-match ranges run through: a one-cell error here is
/// silently mis-coloured text on exactly the annotated lines, pinned by
/// `display_col_of_char_with_gaps_pins_the_span_after_the_marker`.
fn display_col_of_char_with_gaps(text: &str, gaps: &[usize], idx: usize) -> usize {
    crate::model::text_width::char_index_to_display_col(text, idx)
        + gaps.iter().filter(|&&g| g <= idx).count()
}

/// Render one line on the canvas: the base text in the default face,
/// then overlay each span with its face color, then the match overlay
/// (issue match-highlight) substitutes the search faces over the match
/// ranges.
///
/// `x_start` is the terminal cell where the text begins (the annotated
/// line's `code_start` — the leading run's display width, or column 1 for
/// the column-0 exact-1 shift; issue-annotations-symbol-precise — 0
/// otherwise). `width` is
/// the available cell count from `x_start` to the right edge (the
/// truncation budget). `matches` are this line's search-match ranges (byte
/// offsets relative to the line start; empty when no match context applies).
/// issue-annotation-marker-cell: `file_row.insertions` names the char
/// indexes at which one marker cell is INSERTED (before that char) — the
/// row's text is placed with a blank cell skipped at each such index
/// (the marker glyph itself is drawn from `file_row.anchors` by the
/// caller), so every char from the gap on — and every span / match range
/// over them — sits one display cell further right than in the plain
/// line.
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
    let gaps = &file_row.insertions;
    if text.is_empty() || width == 0 {
        return;
    }
    if spans.is_empty() && matches.is_empty() && gaps.is_empty() {
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

    for (cs, ce, face) in &segments {
        if *cs >= *ce || *cs >= total_chars {
            continue;
        }
        let end_bound = (*ce).min(total_chars);
        // issue-annotation-marker-cell: the inserted marker cells — the
        // segment's runs are split at every gap char index (the inserted
        // cell sits BEFORE that char, so no run ever crosses a gap), and
        // each run's x is the text's display column at its start plus the
        // gaps at or before it (the mid-line re-base: spans and matches
        // over a shifted tail land one cell right, per inserted cell).
        let mut s = (*cs).min(total_chars);
        while s < end_bound {
            let next_gap = gaps.iter().copied().find(|&g| g > s).unwrap_or(usize::MAX);
            let e = next_gap.min(end_bound);
            let full: String = chars[s..e].iter().collect();
            let x =
                x_start as isize + display_col_of_char_with_gaps(text, gaps, s) as isize;
            let used = (x - x_start as isize) as usize;
            if used >= width {
                break;
            }
            let segment = truncate(&full, width - used);
            if segment.is_empty() {
                break;
            }
            // The band: the selected match's background (issue
            // match-highlight) or the landing-highlight fade band
            // (jump-highlight — interpolated toward the base background by
            // intensity under truecolor).
            let (face, band) = match face {
                RowFace::View => (t.view, None),
                RowFace::Syntax(idx) => (t.syntax_face(*idx), None),
                RowFace::Match => (t.search_match, None),
                RowFace::MatchCurrent => (
                    t.search_match_current,
                    Some(color(t.search_match_current.background)),
                ),
                RowFace::Jump(intensity) => (
                    t.jump_highlight,
                    Some(crate::ui::jump_band_bg(t, *intensity)),
                ),
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
            // Advance by DISPLAY width, not char count: a run ending in a
            // wide char occupies one more cell than its char count, and
            // the next run must start where the terminal actually is
            // (plan 004 issue 05d — the dropped-space-after-wide-char
            // bug). A truncated run means the right edge was reached — the
            // later runs do not fit either.
            if display_width(&segment) < display_width(&full) {
                break;
            }
            s = e;
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
    /// issue-current-line-highlight: the buffer line the point is on (the
    /// canvas's current-line tint; the store's file-view point, the same
    /// value that drives the hardware cursor's row).
    pub point_line: usize,
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
                    point_line: props.point_line,
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
        assert!(row.anchors.is_empty());
        assert_eq!(row.code_start, 0);
        assert_eq!(row.indent_chars, 0);
        assert!(row.insertions.is_empty());
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
            anchors: vec![0],
            code_start: 1,
            indent_chars: 0,
            insertions: Vec::new(),
            text: format!("line {line}"),
            spans: Vec::new(),
            matches: Vec::new(),
            highlight: None,
            note_slots: Vec::new(),
        };
        let note = |line: usize| FileViewRow {
            line,
            is_note: true,
            annotated: false,
            anchors: vec![0],
            code_start: 0,
            indent_chars: 0,
            insertions: Vec::new(),
            text: format!("  \u{25b8} note {line}"),
            spans: Vec::new(),
            matches: Vec::new(),
            highlight: None,
            note_slots: Vec::new(),
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

    /// issue-annotations-anchor-at-symbol / symbol-precise: build a store
    /// with `content`, open it, and create one note (via the public A / type
    /// / RET key path) on each line in `notes`. Each `notes` entry is
    /// `(line, char_col, text)`: the point is moved to line `line`, then
    /// advanced `char_col` chars within the line (C-f) BEFORE the `A`, so
    /// the record's stored `col` is that CHAR offset — the anchor's input
    /// (issue-annotations-symbol-precise: char_col 0 is the line's first
    /// token, char_col > 0 a mid-line symbol). Returns the store (to wrap
    /// in the Arc) and the buffer's lines split from `content` (the
    /// GROUND-TRUTH source rows the "cell-for-cell identical" assertions
    /// compare against — read from the actual written file content, not a
    /// hardcoded string).
    fn annotated_store(content: &str, notes: &[(usize, usize, &str)]) -> (crate::app::store::AppStore, Vec<String>) {
        use crate::app::keymap::{parse_key, Key};
        let dir = tempfile::tempdir().unwrap();
        // Leak the project tempdir (as the store tests do): the notes file
        // lives under it and the rendered frame must read it after this
        // helper returns — dropping `dir` would delete the buffer and the
        // notes before the render.
        let dir_path = dir.path().to_path_buf();
        std::mem::forget(dir);
        std::fs::create_dir_all(dir_path.join("src")).unwrap();
        std::fs::write(dir_path.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir_path.join("src/target.rs"), content).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut store = crate::app::store::AppStore::at(&dir_path, base.path().to_path_buf());
        store.set_viewport_lines(24);
        store.open_path("src/target.rs");
        for (line, char_col, text) in notes {
            // Move the point from line 0 to `line`, then within the line to
            // the record column (char_col C-f steps), then create the note
            // there.
            for _ in 0..*line {
                store.key_event(parse_key("C-n").unwrap());
            }
            for _ in 0..*char_col {
                store.key_event(parse_key("C-f").unwrap());
            }
            store.key_event(parse_key("A").unwrap());
            for c in text.chars() {
                store.key_event(if c == ' ' {
                    Key::char(' ')
                } else {
                    parse_key(&c.to_string()).unwrap()
                });
            }
            store.key_event(parse_key("RET").unwrap());
        }
        (store, content.lines().map(String::from).collect())
    }

    /// Build a store with `content` and a HAND-EDITED `.redline-notes.md`
    /// carrying one `[annotation]` block per `(line, col, text)` in `notes`
    /// — records the `A` key path cannot create at arbitrary columns (it
    /// dedupes/edits to ONE record per line, so several records on a single
    /// line are only reachable by editing the notes file, and a mid-line
    /// record's `col` is only reachable that way in this helper). The store
    /// reads the file lazily on first `file_view_rows()`
    /// (issue-annotations-anchor-at-symbol P2-2).
    fn annotated_store_from_notes_file_at(content: &str, notes: &[(usize, usize, &str)]) -> crate::app::store::AppStore {
        let dir = tempfile::tempdir().unwrap();
        // Leak the project tempdir (as the other helpers do): the hand-edited
        // notes file lives under it and must persist past this helper.
        let dir_path = dir.path().to_path_buf();
        std::mem::forget(dir);
        std::fs::create_dir_all(dir_path.join("src")).unwrap();
        std::fs::write(dir_path.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir_path.join("src/target.rs"), content).unwrap();
        // Hand-edit the notes file (NOT the `A` key path): one block per
        // record. `anchor` is the line's content (the re-anchor key), so the
        // record stays on its line.
        let mut notes_file = String::from("<!-- redline-annotations:begin -->\n");
        for (line, col, text) in notes {
            let anchor = content.lines().nth(*line).unwrap_or("");
            notes_file.push_str(&format!(
                "[annotation]\npath: src/target.rs\nline: {line}\ncol: {col}\nanchor: {anchor}\nnote: {text}\norphaned: false\n"
            ));
        }
        notes_file.push_str("<!-- redline-annotations:end -->\n");
        std::fs::write(dir_path.join(".redline-notes.md"), &notes_file).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut store = crate::app::store::AppStore::at(&dir_path, base.path().to_path_buf());
        store.set_viewport_lines(24);
        store.open_path("src/target.rs");
        // Force the lazy notes load (ensure_notes_doc reads the hand-edited
        // file + re-anchors) so the render below sees BOTH records.
        let _ = store.file_view_rows();
        store
    }

    /// Build a store with `content` and a HAND-EDITED `.redline-notes.md`
    /// where each record ALSO carries a syntax tie (`syntax_kind:
    /// type_identifier`, `syntax_name: name`) — issue-annotation-
    /// marker-cell: the records the INSERTION rule applies to (tied to a
    /// symbol), which the `A` key path cannot create at arbitrary columns
    /// and the plain hand-edited helper above does not carry. Each `notes`
    /// entry is `(line, char_col, text, symbol_name)`.
    fn annotated_store_from_notes_file_tied(content: &str, notes: &[(usize, usize, &str, &str)]) -> crate::app::store::AppStore {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_path_buf();
        std::mem::forget(dir);
        std::fs::create_dir_all(dir_path.join("src")).unwrap();
        std::fs::write(dir_path.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir_path.join("src/target.rs"), content).unwrap();
        let mut notes_file = String::from("<!-- redline-annotations:begin -->\n");
        for (line, col, text, name) in notes {
            let anchor = content.lines().nth(*line).unwrap_or("");
            notes_file.push_str(&format!(
                "[annotation]\npath: src/target.rs\nline: {line}\ncol: {col}\nanchor: {anchor}\nnote: {text}\norphaned: false\nsyntax_kind: type_identifier\nsyntax_name: {name}\n"
            ));
        }
        notes_file.push_str("<!-- redline-annotations:end -->\n");
        std::fs::write(dir_path.join(".redline-notes.md"), &notes_file).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut store = crate::app::store::AppStore::at(&dir_path, base.path().to_path_buf());
        store.set_viewport_lines(24);
        store.open_path("src/target.rs");
        // Force the lazy notes load (ensure_notes_doc reads the hand-edited
        // file + re-anchors) so the render below sees the records.
        let _ = store.file_view_rows();
        store
    }

    /// The display column (in cells) where `needle` begins in `line`: the
    /// cell count of everything before it.
    fn col_of(line: &str, needle: &str) -> Option<usize> {
        let b = line.find(needle)?;
        Some(crate::model::text_width::display_width(&line[..b]))
    }

    /// The rendered row's content from display column `col` onward, sliced by
    /// DISPLAY cells (the anchor glyphs ▴/╭/─ are multi-byte in UTF-8, so a
    /// byte slice at a display boundary would panic mid-glyph).
    fn display_from(line: &str, col: usize) -> String {
        let mut acc = 0usize;
        for (i, c) in line.char_indices() {
            if acc == col {
                return line[i..].to_string();
            }
            acc += crate::model::text_width::char_display_width(c);
            if acc > col {
                return String::new(); // `col` falls inside a char
            }
        }
        String::new()
    }

    /// The line's code region: everything from the first non-whitespace char
    /// on (the leading indentation stripped).
    fn code_tail(line: &str) -> &str {
        let start = line.find(|c: char| !c.is_whitespace()).unwrap_or(line.len());
        &line[start..]
    }

    /// The line's region from char index `char_idx` on (char-sliced, not
    /// byte: the source's ground truth for a MID-LINE symbol — the
    /// indicator overwrote only the cell before the symbol, so
    /// "cell-for-cell" is asserted from the symbol on, not from the line's
    /// first token).
    fn source_tail_from(line: &str, char_idx: usize) -> String {
        line.chars().skip(char_idx).collect()
    }

    /// The display column where `needle`'s first occurrence begins, scanning
    /// the frame line by line (a whole-frame `col_of` would count newlines
    /// as cells — this isolates the single line first).
    fn code_start_col_in_frame(frame: &str, needle: &str) -> Option<usize> {
        let line = frame.lines().find(|l| l.contains(needle))?;
        col_of(line, needle)
    }

    /// The first rendered frame line whose display content contains `needle`
    /// and the SHOWN anchor glyph `\u{25b4}` (the code row, not a note row).
    fn shown_code_row(frame: &str, needle: &str) -> String {
        frame.lines()
            .find(|l| l.contains('\u{25b4}') && l.contains(needle))
            .unwrap_or_else(|| panic!("shown code row not found in frame:\n{frame}"))
            .to_string()
    }

    /// issue-annotations-anchor-at-symbol — THE strongest test: for an
    /// INDENTED symbol the rendered code row is CELL-FOR-CELL identical to
    /// the plain source row. The indicator borrows the last indentation cell,
    /// so no code character moves — the code region (from the symbol on) is
    /// byte-identical to the actual buffer line at the same display columns.
    /// This is what "the code does not move" means, and it is what a future
    /// refactor is most likely to break.
    #[test]
    fn annotation_indented_symbol_code_row_cell_for_cell_identical() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // Line 1 is indented four spaces: the symbol `if` sits at display
        // column 4. Annotating it must leave `if` at column 4.
        let content = "fn main() {\n    if x > 0 {\n}\n";
        let (store, src_lines) = annotated_store(content, &[(1, 0, "check the bounds")]);
        assert!(!store.note_rows_folded(), "notes shown by default");
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();

        let src_line = &src_lines[1]; // "    if x > 0 {"
        let code = shown_code_row(&frame, "if x > 0 {");

        // The symbol's display column in the SOURCE (the first non-ws char).
        let src_ws_end = src_line
            .char_indices()
            .find(|&(_, c)| !c.is_whitespace())
            .map(|(b, _)| crate::model::text_width::display_width(&src_line[..b]))
            .unwrap_or(0);
        assert_eq!(src_ws_end, 4, "fixture sanity: `if` is at display col 4");

        // NO CODE CHARACTER MOVED: the code's start column in the rendered
        // row is the SAME display column as in the source row.
        let rendered_code_col = col_of(&code, "if x > 0 {")
            .unwrap_or_else(|| panic!("code not found in rendered row: {code:?}"));
        assert_eq!(
            rendered_code_col, src_ws_end,
            "JITTER: the indented symbol's code moved — source col {src_ws_end}, rendered col {rendered_code_col}: {code:?}"
        );

        // CELL-FOR-CELL: from the symbol on, the rendered row equals the
        // actual buffer line cell for cell (the anchor glyph took the last
        // indentation cell, col 3; nothing after it moved).
        let rendered_tail = display_from(&code, src_ws_end).trim_end().to_string();
        let source_tail = code_tail(src_line);
        assert_eq!(
            rendered_tail, source_tail,
            "the code region must be cell-for-cell identical to the source line: rendered={rendered_tail:?} source={source_tail:?}"
        );

        // The indicator sits at the LAST indentation cell (col 3), and the
        // three cells before it are still blank (the borrowed-indentation
        // cells 0..2).
        assert_eq!(code.chars().nth(3), Some('\u{25b4}'), "anchor ▴ must be at the last indentation cell (col 3): {code:?}");
        for i in 0..3 {
            assert_eq!(code.chars().nth(i), Some(' '), "indentation cell {i} must stay blank: {code:?}");
        }
    }

    /// issue-annotations-anchor-at-symbol — the COLUMN-0 case: a symbol with
    /// no indentation to borrow shifts its line by EXACTLY ONE cell (option
    /// (a)). The indicator takes column 0 and the code moves from column 0
    /// to column 1; everything after it is cell-for-cell identical to the
    /// source. Asserted as exactly 1, not "small".
    #[test]
    fn annotation_column_zero_shifts_by_exactly_one() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        let content = "fn main() {\n}\n";
        let (store, src_lines) = annotated_store(content, &[(0, 0, "the entry point")]);
        assert!(!store.note_rows_folded(), "notes shown by default");
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();

        let src_line = &src_lines[0]; // "fn main() {"
        let code = shown_code_row(&frame, "fn main() {");

        // The indicator takes column 0; the code's start column shifts from
        // 0 to EXACTLY 1.
        assert_eq!(code.chars().next(), Some('\u{25b4}'), "column-0 anchor ▴ must be at cell 0: {code:?}");
        let rendered_code_col = col_of(&code, "fn main() {")
            .unwrap_or_else(|| panic!("code not found in rendered row: {code:?}"));
        assert_eq!(
            rendered_code_col, 1,
            "a column-0 symbol must shift by EXACTLY one cell (0 -> 1), not more: {code:?}"
        );

        // Cell-for-cell: from the symbol on, the rendered row equals the
        // source line (shifted right by the one indicator cell).
        let rendered_tail = display_from(&code, 1);
        assert_eq!(
            rendered_tail.trim_end(), src_line,
            "the column-0 code region must be cell-for-cell identical to the source: rendered={} source={:?}", rendered_tail.trim_end(), src_line
        );
    }

    /// issue-annotations-anchor-at-symbol — the fold invariant, RE-EXRESSED
    /// (the old "constant leading width 2" test is obsolete): folding must
    /// not move the code. Assert the code's start column is IDENTICAL folded
    /// and expanded, for BOTH a column-0 symbol and an indented symbol. The
    /// arrow carries the state (▴ shown / ▸ folded) at the anchor; the note
    /// row appears only when shown.
    #[test]
    fn fold_invariant_code_start_column_identical_folded_and_shown() {
        use crate::app::keymap::parse_key;
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // Line 0 is a column-0 symbol, line 1 an indented (4-space) symbol.
        let content = "fn main() {\n    if x > 0 {\n}\n";
        let (store, _lines) = annotated_store(content, &[(0, 0, "note zero"), (1, 0, "note one")]);
        assert!(!store.note_rows_folded(), "notes shown by default");
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let shown = app.to_string();
        let shown_zero = code_start_col_in_frame(&shown, "fn main() {").expect("col-0 code row in SHOWN frame");
        let shown_deep = code_start_col_in_frame(&shown, "if x > 0 {").expect("indented code row in SHOWN frame");

        // The column-0 symbol sits at cell 1 (the exact-1 shift); the
        // indented symbol keeps its source column (4).
        assert_eq!(shown_zero, 1, "col-0 symbol at cell 1 (the shift): {shown_zero}");
        assert_eq!(shown_deep, 4, "indented symbol keeps its source column (4): {shown_deep}");

        // Fold: note rows go, the arrow ▴→▸ — but the code must NOT move.
        {
            let mut st = shared.lock().unwrap();
            st.key_event(parse_key("C-c").unwrap());
            st.key_event(parse_key("a").unwrap());
            st.key_event(parse_key("h").unwrap());
        }
        let folded = app.to_string();
        let folded_zero = code_start_col_in_frame(&folded, "fn main() {").expect("col-0 code row in FOLDED frame");
        let folded_deep = code_start_col_in_frame(&folded, "if x > 0 {").expect("indented code row in FOLDED frame");

        // NO JITTER, both symbols, both states:
        assert_eq!(
            shown_zero, folded_zero,
            "JITTER (col-0): the code moved on a fold — shown={shown_zero} folded={folded_zero}\nSHOWN:\n{shown}\nFOLDED:\n{folded}"
        );
        assert_eq!(
            shown_deep, folded_deep,
            "JITTER (indented): the code moved on a fold — shown={shown_deep} folded={folded_deep}\nSHOWN:\n{shown}\nFOLDED:\n{folded}"
        );

        // The fold actually changed something (notes gone, arrow ▴→▸) —
        // otherwise the two frames are identical and this test is vacuous.
        assert!(!folded.lines().any(|l| l.contains("note zero") || l.contains("note one")),
            "folded frame must have no note rows");
        assert!(folded.lines().any(|l| l.contains('\u{25b8}') && l.contains("fn main() {")),
            "folded col-0 symbol must carry the ▸ arrow");
        assert!(folded.lines().any(|l| l.contains('\u{25b8}') && l.contains("if x > 0 {")),
            "folded indented symbol must carry the ▸ arrow");
    }

    /// issue-annotations-anchor-at-symbol (design A carried): the ANCHOR
    /// RELATIONSHIP, asserted per cell. The note row's curved corner ╭
    /// (\u{256d}) must sit at the SAME cell as the ▴ on the code row DIRECTLY
    /// BELOW it (both at the anchor column), with the ─ bend one cell right
    /// and the note text at anchor+2 — so a note's indent mirrors the
    /// nesting of the code it annotates. *Presence is not placement*: a
    /// mere "the note row has a ╭" assertion cannot catch a misplacement,
    /// so this pins the column per cell.
    #[test]
    fn note_corner_anchored_to_code_row_same_cell() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // Line 0 is a column-0 symbol (anchor at cell 0) so the note row is
        // ╭@0 / ─@1 / text@2 — the shallowest nesting.
        let content = "fn main() {\n}\n";
        let (store, _lines) = annotated_store(content, &[(0, 0, "my note")]);
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();
        let lines: Vec<&str> = frame.lines().collect();

        // The SHOWN code row: ▴ (anchor cell 0) + source (cell 1).
        let code_idx = lines
            .iter()
            .position(|l| l.contains('\u{25b4}') && l.contains("fn main() {"))
            .unwrap_or_else(|| panic!("shown code row not found:\n{frame}"));
        let code = lines[code_idx];
        let arrow_col = col_of(code, "\u{25b4}").unwrap();
        assert_eq!(arrow_col, 0, "▴ must be at cell 0 (the anchor column): {code:?}");
        assert_eq!(col_of(code, "fn main() {").unwrap(), 1, "column-0 source must start at cell 1 (the exact-1 shift): {code:?}");

        // The note row is the row DIRECTLY ABOVE the code row (adjacency).
        assert!(code_idx > 0, "no row above the code row — the note row is missing:\n{frame}");
        let note = lines[code_idx - 1];
        assert!(note.contains("my note"), "the row directly above the code row must be the note row: {note:?}\n{frame}");

        // ANCHOR: the note row's ╭ sits at the SAME cell as the code row's ▴
        // directly below it — asserted per cell, so a corner that moved one
        // cell either way is RED.
        let corner_col = col_of(note, "\u{256d}").unwrap_or_else(|| panic!("no ╭ on the note row: {note:?}"));
        assert_eq!(
            corner_col, arrow_col,
            "the note row's ╭ (cell {corner_col}) must anchor at the SAME cell as the code row's ▴ (cell {arrow_col}): note={note:?} code={code:?}"
        );
        assert_eq!(corner_col, 0, "the corner must be at cell 0 (the anchor column): {note:?}");
        // The cell-1 ─ is the corner's rightward bend; the note text starts
        // at cell 2 (anchor + 2).
        assert!(note.chars().nth(1) == Some('\u{2500}'), "note row cell 1 must be ─ (the corner's bend into the note): {note:?}");
        assert_eq!(col_of(note, "my note").unwrap(), 2, "note text must start at cell 2 (anchor + 2): {note:?}");
    }

    /// issue-annotations-anchor-at-symbol: a note's INDENT MIRRORS the
    /// nesting of the code it annotates — a note for a deeper (more
    /// indented) symbol starts further right than one for a shallower
    /// symbol. Asserted as the note text's column growing with the
    /// indentation.
    #[test]
    fn note_indent_mirrors_nesting_deeper_symbol_further_right() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // Line 0: `fn main() {` (col 0, anchor 0, note text at cell 2).
        // Line 1: `    if x > 0 {` (indent 4, anchor 3, note text at cell 5).
        let content = "fn main() {\n    if x > 0 {\n}\n";
        let (store, _lines) = annotated_store(content, &[(0, 0, "outer"), (1, 0, "inner")]);
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();
        let lines: Vec<&str> = frame.lines().collect();

        // The outer note's text column (anchor 0 → cell 2).
        let outer_note = lines.iter().find(|l| l.contains("outer")).expect("outer note row");
        let outer_col = col_of(outer_note, "outer").unwrap();
        assert_eq!(outer_col, 2, "outer note text at cell 2 (anchor 0 + 2): {outer_note:?}");
        // The inner (deeper) note's text column (anchor 3 → cell 5).
        let inner_note = lines.iter().find(|l| l.contains("inner")).expect("inner note row");
        let inner_col = col_of(inner_note, "inner").unwrap();
        assert_eq!(inner_col, 5, "inner note text at cell 5 (anchor 3 + 2): {inner_note:?}");
        assert!(
            inner_col > outer_col,
            "a deeper symbol's note must start further right than a shallower one's: inner={inner_col} outer={outer_col}"
        );
    }

    /// issue-annotations-anchor-at-symbol — TABS: the anchor is a
    /// display-column position and a tab advances to the next 8-column tab
    /// stop. A line indented by a single tab has its code at display column
    /// 8, so the anchor (the last indentation cell) is column 7 and the code
    /// does not move. Pinned with a tab-indented fixture — not left to
    /// chance.
    #[test]
    fn annotation_tab_indented_anchors_at_next_tab_stop() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // A single leading tab, then `let`: the code sits at display col 8
        // (the next tab stop), the anchor at col 7.
        let content = "fn main() {\n\tlet x = 1;\n}\n";
        let (store, src_lines) = annotated_store(content, &[(1, 0, "tabbed")]);
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();
        let code = shown_code_row(&frame, "let x = 1;");
        let src_line = &src_lines[1]; // "\tlet x = 1;"

        // The tab-expanded indent width is 8: the code must be at display
        // column 8, and the anchor (▴) at column 7.
        let rendered_code_col = col_of(&code, "let x = 1;")
            .unwrap_or_else(|| panic!("code not found in rendered row: {code:?}"));
        assert_eq!(
            rendered_code_col, 8,
            "a tab-indented line's code must sit at the next 8-column tab stop (col 8): {code:?}"
        );
        assert_eq!(code.chars().nth(7), Some('\u{25b4}'), "the anchor ▴ must be at col 7 (the last tab-expanded indentation cell): {code:?}");
        // Cell-for-cell: from col 8 on, the rendered row equals the source's
        // code region (the tab is stripped; the code sits at the tab stop).
        let rendered_tail = display_from(&code, 8);
        let source_tail = code_tail(src_line);
        assert_eq!(rendered_tail.trim_end(), source_tail, "tabbed code region must be cell-for-cell identical: rendered={rendered_tail:?} source={source_tail:?}");
    }

    /// issue-annotations-symbol-precise (re-expressed from the landed
    /// P2-2 glyph test): two records on ONE line that BOTH fall back to
    /// the same indent anchor share that ONE indicator — the note rows
    /// stack directly above the code-row indicator in record order, and
    /// folding hides them all while leaving EXACTLY ONE ▸. The `A` key path
    /// dedupes/edits to one record per line, so the fixture is a HAND-EDITED
    /// notes file (annotated_store_from_notes_file_at), not something the UI
    /// can produce. `fn main() {` is a column-0 line and both records are
    /// at char 0, so both fall back to the shared column-0 anchor (the
    /// two-indicators case is
    /// `annotation_multi_per_line_two_records_two_indicators`).
    #[test]
    fn annotation_multi_per_line_fallback_records_share_one_indicator_fold() {
        use crate::app::keymap::parse_key;
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // TWO records on line 0 (one line), both at char 0. Hand-edited
        // notes file.
        let store = annotated_store_from_notes_file_at("fn main() {\n}\n", &[(0, 0, "outer note"), (0, 0, "inner note")]);
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();
        let lines: Vec<&str> = frame.lines().collect();

        // Two note rows, both present.
        let note_rows = lines.iter().filter(|l| l.contains("outer note") || l.contains("inner note")).count();
        assert_eq!(note_rows, 2, "two records on one line -> two note rows:\n{frame}");
        let outer = lines.iter().find(|l| l.contains("outer note")).unwrap();
        let inner = lines.iter().find(|l| l.contains("inner note")).unwrap();
        // Both records fell back to the SAME (column-0) anchor: both note
        // rows carry the ╭ corner at that shared anchor column.
        let outer_anchor = col_of(outer, "\u{256d}")
            .unwrap_or_else(|| panic!("no ╭ on outer note row: {outer:?}"));
        let inner_anchor = col_of(inner, "\u{256d}")
            .unwrap_or_else(|| panic!("no ╭ on inner note row: {inner:?}"));
        assert_eq!(outer_anchor, inner_anchor, "both fallback records share the one anchor column:\n{frame}");

        // issue-annotations-layout: same anchor = maximal overlap -> they
        // stack, and the FURTHER-OUT one (the LATER record at the shared
        // anchor — its cells cannot reach left into the earlier record's,
        // only the reverse) gets the longer leader: its text starts at
        // 12 (the outer note's text end, [0, 12)), the outer keeps the
        // plain single bend and its text at 2.
        assert_eq!(col_of(outer, "outer note").unwrap(), 2, "the outer (earlier) note keeps the plain leader (text at anchor + 2): {outer:?}");
        assert_eq!(col_of(inner, "inner note").unwrap(), 12, "the further-out note's text starts at the colliding note's text end (12), not anchor + 2 (2): {inner:?}");
        for i in 2..=11 {
            assert_eq!(inner.chars().nth(i), Some('\u{2500}'), "the inner note's extended leader cell {i} is ─: {inner:?}");
        }

        // A single ▴ code row (one indicator), with BOTH notes directly
        // above it.
        let arrow_idx = lines
            .iter()
            .position(|l| l.contains("\u{25b4}") && l.contains("fn main() {"))
            .unwrap_or_else(|| panic!("no ▴ code row in shown frame:\n{frame}"));
        let arrow_count = lines.iter().filter(|l| l.contains("\u{25b4}") && l.contains("fn main() {")).count();
        assert_eq!(arrow_count, 1, "exactly one ▴ indicator for the line, despite two notes:\n{frame}");
        // Both note rows sit DIRECTLY above the code row (record order).
        assert!(arrow_idx >= 2, "both note rows must precede the code row:\n{frame}");
        assert!(lines[arrow_idx - 1].contains("outer note") || lines[arrow_idx - 1].contains("inner note"), "immediately-above row is a note:\n{frame}");
        assert!(lines[arrow_idx - 2].contains("outer note") || lines[arrow_idx - 2].contains("inner note"), "second-above row is a note:\n{frame}");
        // The note ╭ anchors at the SAME cell as the code row's ▴.
        let arrow_col = col_of(lines[arrow_idx], "\u{25b4}")
            .unwrap_or_else(|| panic!("no ▴ at the code row: {:?}", lines[arrow_idx]));
        assert_eq!(outer_anchor, arrow_col, "the note ╭ anchors at the same cell as the code row's ▴:\n{frame}");

        // Fold: BOTH note rows vanish, and EXACTLY ONE ▸ remains (the single
        // indicator on the single code row).
        {
            let mut st = shared.lock().unwrap();
            st.key_event(parse_key("C-c").unwrap());
            st.key_event(parse_key("a").unwrap());
            st.key_event(parse_key("h").unwrap());
        }
        let folded = app.to_string();
        let flines: Vec<&str> = folded.lines().collect();
        assert!(
            !flines.iter().any(|l| l.contains("outer note") || l.contains("inner note")),
            "folded: both note rows gone:\n{folded}"
        );
        let tri_count = flines.iter().filter(|l| l.contains("\u{25b8}") && l.contains("fn main() {")).count();
        assert_eq!(tri_count, 1, "folded: exactly one ▸ for the line (one code row, one indicator), despite two notes:\n{folded}");
    }

    /// All the display columns where `ch` occurs in `line` (the glyph
    /// tests need the SECOND ▴/▸'s column, which `col_of` cannot see).
    fn cols_of(line: &str, ch: char) -> Vec<usize> {
        let mut out = Vec::new();
        let mut acc = 0usize;
        for c in line.chars() {
            if c == ch {
                out.push(acc);
            }
            acc += crate::model::text_width::char_display_width(c);
        }
        out
    }

    /// issue-annotations-symbol-precise — THE new case, cell-for-cell:
    /// a MID-LINE symbol. The indicator sits at `display_col - 1` (one
    /// cell left of the annotated symbol, overwriting that blank cell),
    /// NOT at the line's indent anchor — so on a 4-space-indented line the
    /// ▴ moves from col 3 (the landed line-based rule) to the cell before
    /// the symbol, and the code stays cell-for-cell identical to the
    /// source. The note row's ╭ anchors at the SAME column as the ▴
    /// directly below it (the anchor relationship — presence is not
    /// placement).
    #[test]
    fn annotation_mid_line_symbol_indicator_at_display_col_minus_one() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // Line 1: `    let x = 1;` — the record is on `x` (char 8, display
        // col 8; the char before it is the space at display col 7) → the
        // anchor is 7, not the indent anchor 3.
        let content = "fn main() {\n    let x = 1;\n}\n";
        let (store, src_lines) = annotated_store(content, &[(1, 8, "note about x")]);
        assert!(!store.note_rows_folded(), "notes shown by default");
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();
        let lines: Vec<&str> = frame.lines().collect();

        let src_line = &src_lines[1]; // "    let x = 1;"
        let code = shown_code_row(&frame, "x = 1;");

        // The indicator sits ONE cell left of the symbol: display col 7
        // (the cell the source's space occupies). The landed line-based
        // rule would have put it at col 3 (the last indent cell) — the
        // cells between 3 and 7 prove the anchor moved to the symbol.
        let arrow_col = col_of(&code, "\u{25b4}")
            .unwrap_or_else(|| panic!("no ▴ on the code row: {code:?}"));
        assert_eq!(arrow_col, 7, "mid-line anchor at display col 7 (display_col 8 - 1), not the indent anchor 3: {code:?}");
        assert_eq!(col_of(&code, "x = 1;").unwrap(), 8, "the symbol keeps its source column (8): {code:?}");
        // The cells between the old anchor (3) and the symbol (7) are
        // source content, not gutter: col 3 blank (indent), 4-6 `let`.
        assert_eq!(code.chars().nth(3), Some(' '), "col 3 stays a blank indent cell (the old anchor is GONE): {code:?}");
        assert_eq!(col_of(&code, "let").unwrap(), 4, "`let` keeps its source column (4): {code:?}");

        // NO CODE CHARACTER MOVED: cell-for-cell from the SYMBOL on, the
        // rendered row equals the actual buffer line (the indicator
        // overwrote the space at col 7 only).
        let rendered_tail = display_from(&code, 8).trim_end().to_string();
        let source_tail = source_tail_from(src_line, 8);
        assert_eq!(
            rendered_tail, source_tail,
            "the code region must be cell-for-cell identical to the source line: rendered={rendered_tail:?} source={source_tail:?}"
        );
        // And the head: cols 0-7 render as `    let` + ▴ — the source's
        // cols 0-6 cell-for-cell.
        assert_eq!(
            &code[..7], "    let",
            "source cols 0-6 cell-for-cell (indent + `let`): {code:?}"
        );

        // ANCHOR RELATIONSHIP (per cell): the note row DIRECTLY ABOVE the
        // code row carries its ╭ at the SAME column as the ▴ below it
        // (col 7), the ─ bend at 8, the note text at 9.
        let code_idx = lines.iter().position(|l| l.contains("\u{25b4}") && l.contains("let") && l.contains("x = 1;"))
            .unwrap_or_else(|| panic!("code row not found in frame:\n{frame}"));
        assert!(code_idx > 0, "no row above the code row — the note row is missing:\n{frame}");
        let note = lines[code_idx - 1];
        assert!(note.contains("note about x"), "the row directly above the code row must be the note row: {note:?}\n{frame}");
        let corner_col = col_of(note, "\u{256d}")
            .unwrap_or_else(|| panic!("no ╭ on the note row: {note:?}"));
        assert_eq!(
            corner_col, arrow_col,
            "the note row's ╭ (col {corner_col}) must anchor at the SAME cell as the code row's ▴ (col {arrow_col}): note={note:?} code={code:?}"
        );
        assert_eq!(corner_col, 7, "the mid-line note's ╭ sits at col 7 (the record's own anchor): {note:?}");
        assert!(note.chars().nth(8) == Some('\u{2500}'), "note row col 8 must be ─ (the corner's bend): {note:?}");
        assert_eq!(col_of(note, "note about x").unwrap(), 9, "note text at col 9 (anchor + 2): {note:?}");
    }

    /// issue-annotations-symbol-precise — the UNITS TRAP, glyph-level:
    /// the record's `col` is a CHAR offset and the anchor a DISPLAY
    /// column. A symbol after two CJK chars (4 display cells) sits 2
    /// columns further right than its char offset suggests — a
    /// char-offset implementation lands the indicator two cells LEFT, on
    /// a CJK glyph, invisible on ASCII-only lines.
    #[test]
    fn annotation_wide_char_preceded_symbol_anchors_at_display_column() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // Line 1: `  中中 x = 1;` — `x` at CHAR 5 but DISPLAY col 7 (two
        // spaces + two CJK chars = 6 cells) → the anchor is 6 (the space
        // before `x`). A char-offset error would anchor at 4 (inside the
        // second 中).
        let content = "fn main() {\n  中中 x = 1;\n}\n";
        let (store, src_lines) = annotated_store(content, &[(1, 5, "cjk note")]);
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();

        let src_line = &src_lines[1]; // "  中中 x = 1;"
        let code = shown_code_row(&frame, "x = 1;");

        let arrow_col = col_of(&code, "\u{25b4}")
            .unwrap_or_else(|| panic!("no ▴ on the code row: {code:?}"));
        assert_eq!(arrow_col, 6, "the anchor is the DISPLAY column (6), not char-offset 4 (inside the second CJK glyph): {code:?}");
        assert_eq!(col_of(&code, "中中").unwrap(), 2, "the CJK chars keep their source cells (2-3, 4-5): {code:?}");
        assert_eq!(col_of(&code, "x = 1;").unwrap(), 7, "the symbol keeps its source column (7): {code:?}");

        // Cell-for-cell from the symbol on (the indicator overwrote the
        // space at col 6 only).
        let rendered_tail = display_from(&code, 7).trim_end().to_string();
        let source_tail = source_tail_from(src_line, 5);
        assert_eq!(
            rendered_tail, source_tail,
            "the code region must be cell-for-cell identical to the source line: rendered={rendered_tail:?} source={source_tail:?}"
        );

        // The note row's ╭ anchors at the same (display) column.
        let lines: Vec<&str> = frame.lines().collect();
        let note = lines.iter().find(|l| l.contains("cjk note")).expect("cjk note row");
        let corner_col = col_of(note, "\u{256d}")
            .unwrap_or_else(|| panic!("no ╭ on the note row: {note:?}"));
        assert_eq!(corner_col, 6, "the note's ╭ anchors at the display column (6): {note:?}");
    }

    /// issue-annotations-symbol-precise — one indicator PER ANNOTATION:
    /// two records on one line at two DIFFERENT symbols get two indicators
    /// at two columns and two note rows, each ╭ at its own anchor (the
    /// code row carries a SET of anchors — the landed "several records
    /// share one anchor" rule is gone). Folding leaves one ▸ PER ANNOTATION
    /// (two here). The fixture is a HAND-EDITED notes file (pushed
    /// directly); the `A` key path now addresses per symbol (not per line),
    /// so a line MAY host several records.
    /// issue-annotations-layout: these two notes MUST overlap ("note a"
    /// at anchor 3 spans cells [3, 11), reaching note b's ╭ at 5), so they
    /// stack — and note b is the FURTHER-OUT one (the larger anchor):
    /// its leader extends and its text starts at 11, strictly right of
    /// note a's text end; note a keeps the plain single-bend leader.
    #[test]
    fn annotation_multi_per_line_two_records_two_indicators_two_note_rows() {
        use crate::app::keymap::parse_key;
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // Line 1: `    a b` — record 1 on `a` (char 4 — the line's first
        // token: display 4, preceded by a space → anchor 3), record 2 on
        // `b` (char 6 — mid-line: display 6 → anchor 5). Two distinct
        // anchors.
        let store = annotated_store_from_notes_file_at("fn main() {\n    a b\n}\n", &[(1, 4, "note a"), (1, 6, "note b")]);
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();
        let lines: Vec<&str> = frame.lines().collect();

        // TWO note rows, stacked above the code row in record order.
        let note_a = lines.iter().find(|l| l.contains("note a")).expect("note a row");
        let note_b = lines.iter().find(|l| l.contains("note b")).expect("note b row");
        let corner_a = col_of(note_a, "\u{256d}").unwrap_or_else(|| panic!("no ╭ on note a: {note_a:?}"));
        let corner_b = col_of(note_b, "\u{256d}").unwrap_or_else(|| panic!("no ╭ on note b: {note_b:?}"));
        assert_eq!(corner_a, 3, "note a's ╭ at its record's anchor (col 3 — `a` is the first token): {note_a:?}");
        assert_eq!(corner_b, 5, "note b's ╭ at its record's anchor (col 5 — the cell before `b`): {note_b:?}");

        // issue-annotations-layout, the OVERLAP case pinned cell-for-cell:
        // the notes collide (note a spans [3, 11), reaching note b's ╭ at
        // 5), so they stack. The FURTHER-OUT note — the LARGER display
        // anchor (note b, the deeper symbol; the only note the other's
        // text can actually reach) — gets the longer leader: its text
        // starts at 11 (note a's text end), not at anchor + 2 (7); note a
        // (the shallower note) keeps the plain single bend and its text at
        // 5. A rule that extended the OTHER note (or shortened b's leader
        // back to 7) is RED here.
        assert_eq!(col_of(note_a, "note a").unwrap(), 5, "the shallower note keeps the plain leader (text at anchor + 2 = 5): {note_a:?}");
        assert!(note_a.chars().nth(4) == Some('\u{2500}'), "the shallower note's single bend at cell 4: {note_a:?}");
        assert_eq!(col_of(note_b, "note b").unwrap(), 11, "the further-out note's text starts at the colliding note's text end (11), not anchor + 2 (7): {note_b:?}");
        for i in 6..=10 {
            assert_eq!(note_b.chars().nth(i), Some('\u{2500}'), "note b's extended leader cells {i} are ─: {note_b:?}");
        }

        // The code row carries TWO ▴ at the two anchors (3 and 5), and the
        // code keeps its source column (`a` at 4, `b` at 6). (The rendered
        // row is `   ▴a▴b` — the ▴ cells sit BETWEEN the code chars, so the
        // search key is the glyph + both letters, not the source string.)
        let code_idx = lines.iter().position(|l| l.contains("\u{25b4}") && l.contains('a') && l.contains('b'))
            .unwrap_or_else(|| panic!("no ▴ code row:\n{frame}"));
        let code = lines[code_idx];
        assert_eq!(cols_of(code, '\u{25b4}'), vec![3, 5], "two indicators at the two anchors: {code:?}");
        assert_eq!(col_of(code, "a").unwrap(), 4, "`a` keeps its source column: {code:?}");
        assert_eq!(col_of(code, "b").unwrap(), 6, "`b` keeps its source column: {code:?}");
        // Both note rows sit directly above the code row (record order).
        assert!(code_idx >= 3, "two note rows must precede the code row (title + line 0 code + 2 notes):\n{frame}");
        assert!(lines[code_idx - 1].contains("note b") && lines[code_idx - 2].contains("note a"),
            "record order: note a then note b, directly above:\n{frame}");
        // The ANCHOR RELATIONSHIP per note: each ╭ at the same column as
        // its own ▴ on the row below.
        assert!(cols_of(code, '\u{25b4}').contains(&corner_a) && cols_of(code, '\u{25b4}').contains(&corner_b),
            "each note's ╭ anchors at its own ▴ cell:\n{frame}");

        // Fold: BOTH note rows vanish; the code row keeps BOTH ▸ (one per
        // annotation, at its own anchor).
        {
            let mut st = shared.lock().unwrap();
            st.key_event(parse_key("C-c").unwrap());
            st.key_event(parse_key("a").unwrap());
            st.key_event(parse_key("h").unwrap());
        }
        let folded = app.to_string();
        let flines: Vec<&str> = folded.lines().collect();
        assert!(
            !flines.iter().any(|l| l.contains("note a") || l.contains("note b")),
            "folded: both note rows gone:\n{folded}"
        );
        let fcode = flines.iter().find(|l| l.contains("\u{25b8}") && l.contains('a') && l.contains('b'))
            .unwrap_or_else(|| panic!("no folded code row with ▸:\n{folded}"));
        assert_eq!(cols_of(fcode, '\u{25b8}'), vec![3, 5], "folded: one ▸ PER ANNOTATION (two, at the two anchors): {fcode:?}");
    }

    // ── issue-annotations-layout: the packed note row + the longer leader ──

    /// issue-annotations-layout — the PACKING: two records on one line whose
    /// display-cell footprints do not collide are drawn on ONE note row, and
    /// the anchor relationship holds for BOTH glyphs on that row (not just
    /// the first): each ╭ at its own record's anchor cell — the same cell as
    /// that record's ▴ on the code row directly below — the ─ bend one cell
    /// right, the text at anchor + 2, and no text truncated or overwritten
    /// by the neighbour. A packing that lost the second glyph, misplaced it
    /// one cell, or let the first text run into the second is RED here.
    #[test]
    fn note_packing_disjoint_notes_share_one_row_and_anchor_per_glyph() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // Line 1: `    a` + 10 spaces + `b` — record 1 on `a` (char 4,
        // anchor 3), record 2 on `b` (char 15, anchor 14). "note a"
        // occupies [3, 11), "note b" [14, 22) — disjoint, so they pack.
        let mid = "    a".to_string() + &" ".repeat(10) + "b";
        let content = format!("fn main() {{\n{mid}\n}}\n");
        let store = annotated_store_from_notes_file_at(
            &content,
            &[(1, 4, "note a"), (1, 15, "note b")],
        );
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();
        let lines: Vec<&str> = frame.lines().collect();

        // The code row: two ▴ at the two anchors (3 and 14), the code keeps
        // its source columns (`a` at 4, `b` at 15).
        let code_idx = lines
            .iter()
            .position(|l| l.contains("\u{25b4}") && l.contains('a') && l.contains('b'))
            .unwrap_or_else(|| panic!("no ▴ code row:\n{frame}"));
        let code = lines[code_idx];
        assert_eq!(cols_of(code, '\u{25b4}'), vec![3, 14], "two indicators at the two anchors: {code:?}");

        // ONE note row carries BOTH notes — directly above the code row.
        assert_eq!(
            lines.iter().filter(|l| l.contains("note a")).count(),
            1,
            "exactly one row hosts note a:\n{frame}"
        );
        let packed = lines[code_idx - 1];
        assert!(
            packed.contains("note a") && packed.contains("note b"),
            "the row directly above the code row carries BOTH notes (packed): {packed:?}\n{frame}"
        );

        // THE ANCHOR RELATIONSHIP FOR EVERY GLYPH ON THE ROW: both ╭ at
        // their own anchor cells, each the SAME cell as its ▴ below.
        let corners = cols_of(packed, '\u{256d}');
        assert_eq!(corners, vec![3, 14], "both corners at their own anchor cells: {packed:?}");
        for &c in &corners {
            assert!(
                cols_of(code, '\u{25b4}').contains(&c),
                "each ╭ (cell {c}) anchors at the ▴ in the same cell on the row below: packed={packed:?} code={code:?}"
            );
        }
        // Each glyph's own bend and text: ─ at 4 and 15, texts at 5 and 16.
        assert!(packed.chars().nth(4) == Some('\u{2500}'), "note a's bend at cell 4 (anchor + 1): {packed:?}");
        assert!(packed.chars().nth(15) == Some('\u{2500}'), "note b's bend at cell 15 (anchor + 1): {packed:?}");
        assert_eq!(col_of(packed, "note a").unwrap(), 5, "note a's text at cell 5 (anchor + 2): {packed:?}");
        assert_eq!(col_of(packed, "note b").unwrap(), 16, "note b's text at cell 16 (anchor + 2): {packed:?}");
        // Neither text is truncated or overwritten: both are intact, and the
        // cells between the two spans stay blank.
        for i in 11..14 {
            assert_eq!(packed.chars().nth(i), Some(' '), "cell {i} between the two notes stays blank: {packed:?}");
        }
    }

    /// issue-annotations-layout — the STACK + LONGER-LEADER case: two
    /// records on one line whose footprints DO collide stack on separate
    /// rows, and the further-out one's leader is measurably longer (an
    /// assertion about cells, not a screenshot): its text starts strictly
    /// to the right of the colliding note's text end, so the connector
    /// still reads as reaching its own anchor rather than its neighbour's.
    /// Both ╭ still anchor at their own ▴ cells.
    #[test]
    fn note_packing_overlapping_notes_stack_and_further_out_leader_is_longer() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // Line 1: `    a b` — anchors 3 and 5. "note a" spans [3, 11) and
        // reaches note b's ╭ (5) — they MUST stack.
        let store = annotated_store_from_notes_file_at("fn main() {\n    a b\n}\n", &[(1, 4, "note a"), (1, 6, "note b")]);
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();
        let lines: Vec<&str> = frame.lines().collect();

        // TWO stacked rows, both directly above the code row.
        let code_idx = lines
            .iter()
            .position(|l| l.contains("\u{25b4}") && l.contains('a') && l.contains('b'))
            .unwrap_or_else(|| panic!("no ▴ code row:\n{frame}"));
        assert!(
            lines[code_idx - 1].contains("note b") && lines[code_idx - 2].contains("note a"),
            "stacked in (anchor, record) order, both above the code row:\n{frame}"
        );
        let row_a = lines[code_idx - 2];
        let row_b = lines[code_idx - 1];
        let code = lines[code_idx];

        // Note a (the shallower note): the plain single bend, text at 5.
        assert_eq!(col_of(row_a, "\u{256d}").unwrap(), 3, "note a's ╭ at its own anchor (3): {row_a:?}");
        assert!(row_a.chars().nth(4) == Some('\u{2500}'), "note a keeps the single bend at 4: {row_a:?}");
        assert_eq!(col_of(row_a, "note a").unwrap(), 5, "note a's text at anchor + 2 (5) — its leader is NOT extended: {row_a:?}");

        // Note b (the FURTHER-OUT one): the longer leader. Its text starts
        // at 11 — note a's text end, four cells past the plain anchor + 2
        // (7) — with ─ filling every cell between its ╭ and its text.
        assert_eq!(col_of(row_b, "\u{256d}").unwrap(), 5, "note b's ╭ at its own anchor (5): {row_b:?}");
        assert_eq!(col_of(row_b, "note b").unwrap(), 11, "note b's longer leader: text at the colliding note's text end (11), not anchor + 2 (7): {row_b:?}");
        for i in 6..=10 {
            assert_eq!(row_b.chars().nth(i), Some('\u{2500}'), "note b's leader cell {i} is ─ (the extension is cells, not a screenshot): {row_b:?}");
        }
        // Both anchors hold on the stacked rows: each ╭ in the same cell as
        // its ▴ on the code row below.
        let arrows = cols_of(code, '\u{25b4}');
        assert!(arrows.contains(&3) && arrows.contains(&5), "the code row keeps both ▴ (3 and 5): {code:?}");
        assert!(arrows.contains(&col_of(row_a, "\u{256d}").unwrap()) && arrows.contains(&col_of(row_b, "\u{256d}").unwrap()), "each stacked ╭ anchors at its own ▴ cell: code={code:?}");
        // No text truncated or overwritten: both notes are intact on their
        // rows (each stacked note has its own canvas row — they never share
        // one).
        assert!(row_a.contains("note a") && row_b.contains("note b"), "both texts intact: {row_a:?} {row_b:?}");
    }

    /// issue-annotations-layout — the "further out" RULE, stated and pinned.
    ///
    /// **Rule.** "Further out" means the note with the LARGER display
    /// anchor — the note further right on the line (the deeper/inner
    /// symbol; the same direction the existing note-indent mirroring
    /// encodes, where a deeper symbol's note starts further right). It is
    /// NOT "the anchor further from where the text sits" (both texts sit
    /// anchor + leader right of their own anchor — no discriminator there)
    /// and not the outer/shallower note.
    ///
    /// **Why.** The geometry forces it: for two notes with anchors
    /// `a_j <= a_i`, note i's cells (╭ at `a_i`, leader, text from
    /// `a_i + 2`) can never reach LEFT into note j's span — only note j's
    /// text, which extends right, can cover note i's ╭/leader/text. The
    /// only note that can ever be overlapped is the later, larger-anchor
    /// one, and it is the one whose connector must keep reading as
    /// reaching its OWN anchor. Extending the other note's leader would
    /// push its text further INTO the collision, not out of it.
    #[test]
    fn further_out_rule_is_the_larger_display_anchor() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // Line 1: `    a b` — the SMALLER-anchor note (a, anchor 3) carries
        // the long text ("aaaaaaaaa" spans [3, 14), reaching past note b's
        // base start 7); note b (anchor 5, "bb") is the larger-anchor, i.e.
        // the further-out, note.
        let store = annotated_store_from_notes_file_at(
            "fn main() {\n    a b\n}\n",
            &[(1, 4, "aaaaaaaaa"), (1, 6, "bb")],
        );
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();
        let lines: Vec<&str> = frame.lines().collect();

        let code_idx = lines
            .iter()
            .position(|l| l.contains("\u{25b4}") && l.contains('a') && l.contains('b'))
            .unwrap_or_else(|| panic!("no ▴ code row:\n{frame}"));
        let row_a = lines[code_idx - 2]; // the smaller-anchor note's row
        let row_b = lines[code_idx - 1]; // the larger-anchor note's row

        // The LARGER-anchor note (b) is the one with the longer leader:
        // its text starts at 14 (note a's text end), five cells past the
        // plain anchor + 2 (7).
        assert_eq!(col_of(row_b, "\u{256d}").unwrap(), 5, "note b's ╭ at its own anchor (5): {row_b:?}");
        assert_eq!(col_of(row_b, "bb").unwrap(), 14, "the larger-anchor (further-out) note's leader extends to 14: {row_b:?}");
        for i in 6..=13 {
            assert_eq!(row_b.chars().nth(i), Some('\u{2500}'), "the extended leader cell {i} is ─: {row_b:?}");
        }
        // The SMALLER-anchor note (a) is NOT the further-out one: its leader
        // stays the plain single bend and its text stays at 5. A rule that
        // extended note a (or both) instead is RED at these cells.
        assert_eq!(col_of(row_a, "\u{256d}").unwrap(), 3, "note a's ╭ at its own anchor (3): {row_a:?}");
        assert!(row_a.chars().nth(4) == Some('\u{2500}'), "note a's single bend at 4: {row_a:?}");
        assert_eq!(col_of(row_a, "aaaaaaaaa").unwrap(), 5, "the smaller-anchor note's leader is NOT extended (text at 5): {row_a:?}");
    }

    /// issue-annotations-layout — the UNITS TRAP for packing: the collision
    /// decision is in DISPLAY CELLS, never char counts. The CJK note text
    /// (5 chars = 10 cells) reaches note b's connector in cells while the
    /// char count (5 chars) would claim the notes fit; a char-based packer
    /// would draw them on one row and overwrite note b's ╭.
    #[test]
    fn note_packing_footprint_collision_counts_display_cells_not_chars() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // Line 1: `    a       b` (7 spaces) — note a on `a` (char 4,
        // anchor 3) with the CJK text (five \u{4e2d} = 10 display cells: span
        // [3, 15)); note b on `b` (char 12, display 12, anchor 11, base
        // span [11, 15)). In cells they collide (15 > 11); by char count
        // (5 chars -> char-cells [5, 10), which a char-based packer would
        // claim fits left of anchor 11) they would not — the cells must
        // win.
        let store = annotated_store_from_notes_file_at(
            "fn main() {\n    a       b\n}\n",
            &[(1, 4, "\u{4e2d}\u{4e2d}\u{4e2d}\u{4e2d}\u{4e2d}"), (1, 12, "bb")],
        );
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();
        let lines: Vec<&str> = frame.lines().collect();

        // TWO rows: the CJK text's cell span reaches note b's ╭ (11), so
        // the char-count "they fit" packing is forbidden.
        let code_idx = lines
            .iter()
            .position(|l| l.contains("\u{25b4}") && l.contains('a') && l.contains('b'))
            .unwrap_or_else(|| panic!("no ▴ code row:\n{frame}"));
        assert!(
            lines[code_idx - 1].contains("bb") && lines[code_idx - 2].contains('\u{4e2d}'),
            "the CJK note and note b stack on separate rows (a char-count packer would share one and overwrite the ╭):\n{frame}"
        );
        let row_a = lines[code_idx - 2];
        let row_b = lines[code_idx - 1];
        // Note a's CJK text is intact at cells 5-14 (anchor + 2, 10
        // cells; a char-count width would have claimed 5).
        assert_eq!(col_of(row_a, "\u{256d}").unwrap(), 3, "note a's ╭ at anchor 3: {row_a:?}");
        assert_eq!(col_of(row_a, "\u{4e2d}").unwrap(), 5, "the CJK text at cell 5, untruncated: {row_a:?}");
        // Note b's further-out leader extends to the CJK text's cell end
        // (15), not the char end (10): its text starts at 15, not 13.
        assert_eq!(col_of(row_b, "\u{256d}").unwrap(), 11, "note b's ╭ at anchor 11: {row_b:?}");
        assert_eq!(col_of(row_b, "bb").unwrap(), 15, "note b's text at the CJK note's CELL end (15), not the char end (10 → would start at 13): {row_b:?}");
        for i in 12..=14 {
            assert_eq!(row_b.chars().nth(i), Some('\u{2500}'), "note b's extended leader cell {i} is ─: {row_b:?}");
        }
    }

    /// issue-annotations-layout — THREE notes: the two whose footprints are
    /// disjoint pack onto one row, the third (whose base span collides with
    /// the second's text) stacks below with its extended leader. One line
    /// can thus produce exactly as many note rows as its collision depth
    /// requires — not one row per record.
    #[test]
    fn note_packing_three_notes_two_pack_one_stacks() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // Line 1: `    a     b     c` — anchors 3, 9, 15. "aa" spans
        // [3, 7); "bbbbbbbbbb" [9, 21) — disjoint from "aa", so they pack.
        // "cc" base [15, 19) collides with "bbbbbbbbbb" (reaches 21) -> it
        // stacks, its text starting at 21 (leader 5).
        let store = annotated_store_from_notes_file_at(
            "fn main() {\n    a     b     c\n}\n",
            &[(1, 4, "aa"), (1, 10, "bbbbbbbbbb"), (1, 16, "cc")],
        );
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();
        let lines: Vec<&str> = frame.lines().collect();

        let code_idx = lines
            .iter()
            .position(|l| l.contains("\u{25b4}") && l.contains('a') && l.contains('b') && l.contains('c'))
            .unwrap_or_else(|| panic!("no ▴ code row:\n{frame}"));
        let code = lines[code_idx];
        assert_eq!(cols_of(code, '\u{25b4}'), vec![3, 9, 15], "three indicators at the three anchors: {code:?}");

        // Exactly TWO note rows: the packed row (aa + bbbbbbbbbb) above the
        // stacked row (cc).
        let row_c = lines[code_idx - 1];
        let row_packed = lines[code_idx - 2];
        assert!(
            row_packed.contains("aa") && row_packed.contains("bbbbbbbbbb"),
            "the two disjoint notes share one row: {row_packed:?}\n{frame}"
        );
        assert!(!row_packed.contains('\u{25b4}'), "the packed row is a note row, not the code row: {row_packed:?}");
        assert!(row_c.contains("cc") && !row_c.contains("aa") && !row_c.contains("bbbbbbbbbb"), "the colliding note stacks alone: {row_c:?}\n{frame}");
        // Packed-row glyphs at their own cells: ╭ at 3 and 9, texts at 5
        // and 11.
        assert_eq!(cols_of(row_packed, '\u{256d}'), vec![3, 9], "both packed corners at their own anchors: {row_packed:?}");
        assert_eq!(col_of(row_packed, "aa").unwrap(), 5, "aa's text at anchor + 2 (5): {row_packed:?}");
        assert_eq!(col_of(row_packed, "bbbbbbbbbb").unwrap(), 11, "b's text at anchor + 2 (11): {row_packed:?}");
        for &c in &[3usize, 9] {
            assert!(cols_of(code, '\u{25b4}').contains(&c), "each packed ╭ (cell {c}) anchors at its own ▴ below");
        }
        // The stacked note: ╭ at 15, extended leader (five ─), text at 21.
        assert_eq!(col_of(row_c, "\u{256d}").unwrap(), 15, "the stacked note's ╭ at its own anchor (15): {row_c:?}");
        assert_eq!(col_of(row_c, "cc").unwrap(), 21, "the stacked note's text at the colliding note's text end (21): {row_c:?}");
        for i in 16..=20 {
            assert_eq!(row_c.chars().nth(i), Some('\u{2500}'), "the stacked note's leader cell {i} is ─: {row_c:?}");
        }
    }

    /// issue-annotations-layout — the NEIGHBOUR GUARD (defence in depth at
    /// the render boundary): store-built rows never reach it — the
    /// packer's first-fit disjointness is pinned at the store level, and
    /// the gate's sweep of store-built rows found 0 violations of
    /// `slot[k].anchor + 1 + leader + width(text) <= slot[k+1].anchor` —
    /// so this HAND-BUILT packed row with a colliding pair is the guard's
    /// only reachable input, and the pin is the truncation itself: slot 1's
    /// text stops at slot 2's own corner, never written past it. Deleting
    /// the guard leaves slot 1's text running cells 13..15 past slot 2's
    /// text end (slot 2's own glyphs overdraw cells 10..12 in draw order,
    /// but the tail past its text end is only the guard's to stop) and
    /// REDS here.
    #[test]
    fn note_packing_neighbour_guard_truncates_at_the_next_slot() {
        use iocraft::prelude::*;

        // The collision the store can never emit: slot 1's base span
        // [3, 16) (╭ @3, ─ @4, text @5..15) covers slot 2's corner (10)
        // AND its text end (13).
        let note_row = FileViewRow {
            line: 0,
            is_note: true,
            annotated: false,
            anchors: vec![3, 10],
            code_start: 0,
            indent_chars: 0,
            insertions: Vec::new(),
            text: String::new(),
            spans: Vec::new(),
            matches: Vec::new(),
            highlight: None,
            note_slots: vec![
                crate::app::store::NoteSlot {
                    anchor: 3,
                    leader: 1,
                    text: "aaaaaaaaaaa".to_string(),
                },
                crate::app::store::NoteSlot {
                    anchor: 10,
                    leader: 1,
                    text: "b".to_string(),
                },
            ],
        };
        let code_row = FileViewRow {
            line: 0,
            is_note: false,
            annotated: true,
            anchors: vec![3, 10],
            code_start: 0,
            indent_chars: 0,
            insertions: Vec::new(),
            text: "code".to_string(),
            spans: Vec::new(),
            matches: Vec::new(),
            highlight: None,
            note_slots: Vec::new(),
        };
        // Render the canvas directly at a pinned 80x2: the guard's
        // boundary is min(row width, next slot's corner), so the width must
        // exceed the collision for the slot boundary — not the row width —
        // to be the discriminator. The bare canvas is content-sized (its own
        // layout is height 0 + flex_grow, width 100%), so a fixed 80x2
        // column View gives it the rows and width to draw into.
        let mut app = element! {
            View(flex_direction: FlexDirection::Column, width: 80, height: 2) {
                FileViewCanvas(
                    rows: vec![note_row, code_row],
                    total_rows: 2usize,
                    top_line: 0usize,
                    region_lines: None,
                    point_line: 99usize, // outside the rows: this test's frame is tint-free
                    notes_folded: false,
                )
            }
        };
        let canvas = app.render(Some(80));
        // get_text trims each row at its last DRAWN cell, so the pins below
        // work on the trimmed row: slot 2's text (12) is the last drawn
        // cell, and the guard's effect is the ABSENCE of 'a' anywhere else.
        let row = canvas.get_text(0, 0, 80, 1);
        // Slot 1's own cells are intact: ╭ at 3, bend at 4, text at 5.
        assert_eq!(col_of(&row, "\u{256d}"), Some(3), "slot 1's ╭ at its anchor (3): {row:?}");
        assert_eq!(col_of(&row, "aaaaa"), Some(5), "slot 1's text starts at anchor + 2 (5): {row:?}");
        // Slot 2's cells are untouched: ╭ at 10, bend at 11, text at 12.
        assert!(row.chars().nth(10) == Some('\u{256d}'), "slot 2's ╭ at its own corner (10): {row:?}");
        assert!(row.chars().nth(11) == Some('\u{2500}'), "slot 2's bend at 11: {row:?}");
        assert!(row.chars().nth(12) == Some('b'), "slot 2's text at 12: {row:?}");
        // THE GUARD: slot 1's "aaaaaaaaaaa" (base span 5..15) is truncated
        // to the 5 cells before slot 2's corner (5..9). Without the guard
        // it would run 5..15 and cells 13..15 would keep 'a' past slot 2's
        // text — slot 2's own glyphs overdraw only 10..12 in draw order.
        assert_eq!(
            row.chars().filter(|&c| c == 'a').count(),
            5,
            "slot 1's text is truncated to the 5 cells before the next slot's corner (5..9): {row:?}"
        );
        assert!(
            row.chars().skip(13).all(|c| c != 'a'),
            "no 'a' past slot 2's text — cells 13..15 would be the unguarded intrusion: {row:?}"
        );
    }

    // ── issue-annotation-marker-cell: the inserted marker cell ───────

    /// issue-annotation-marker-cell — the mid-line re-base, pinned: a span
    /// that STARTS AFTER the inserted marker cell must map one cell right
    /// of its plain display column (a one-cell error here is silently
    /// mis-coloured / mis-placed text on exactly the annotated lines, and
    /// no full-frame test would catch it without this pin).
    #[test]
    fn display_col_of_char_with_gaps_pins_the_span_after_the_marker() {
        // `ab cd` with one inserted cell before `c` (char 2): chars left
        // of the gap keep their columns; the gap's own char sits BEHIND
        // the inserted cell; the tail re-bases by one.
        assert_eq!(display_col_of_char_with_gaps("ab cd", &[], 3), 3, "no gaps: the plain column");
        assert_eq!(display_col_of_char_with_gaps("ab cd", &[2], 0), 0);
        assert_eq!(display_col_of_char_with_gaps("ab cd", &[2], 1), 1);
        assert_eq!(display_col_of_char_with_gaps("ab cd", &[2], 2), 3, "the gap's own char counts its own gap (it sits behind the inserted cell)");
        assert_eq!(display_col_of_char_with_gaps("ab cd", &[2], 3), 4, "a span starting AFTER the marker re-bases by one");
        assert_eq!(display_col_of_char_with_gaps("ab cd", &[2], 5), 6, "EOL re-bases too");
        // Two inserted cells: the tail shifts by both; a gap strictly left
        // of the char counts, one right of it does not.
        assert_eq!(display_col_of_char_with_gaps("abcdef", &[1, 4], 1), 2, "the first gap's own char counts it");
        assert_eq!(display_col_of_char_with_gaps("abcdef", &[1, 4], 3), 4, "one gap (char 1) strictly left of char 3");
        assert_eq!(display_col_of_char_with_gaps("abcdef", &[1, 4], 4), 6, "the second gap's own char counts both");
        assert_eq!(display_col_of_char_with_gaps("abcdef", &[1, 4], 5), 7, "the tail shifts by both cells");
    }

    /// issue-annotation-marker-cell — the RENDER level: a syntax span that
    /// starts after the inserted marker cell must be drawn one cell right
    /// of its plain position, the inserted cell left empty for the marker
    /// glyph, and the head of the line byte-identical.
    #[test]
    fn draw_line_inserted_marker_shifts_text_and_the_span_after_it() {
        // The live reproduction's row: the full line is
        // `    map: HashMap<String, u32>,`; the leading 4 spaces are
        // stripped (code_start 4), the marker cell is INSERTED before char
        // 13 of the row text (the `S` of `String`): cells 4..16 render
        // `map: HashMap<` byte-identical, cell 17 is the inserted cell
        // (empty here — the ▴ is drawn by the caller's anchor loop), and
        // `String` renders at cells 18..24 with its syntax face.
        let row = FileViewRow {
            line: 0,
            is_note: false,
            annotated: true,
            anchors: vec![17],
            code_start: 4,
            indent_chars: 4,
            insertions: vec![13],
            text: "map: HashMap<String, u32>,".to_string(),
            spans: vec![redline_syntax::highlight::LineSpan {
                start: 13, // the span STARTS after the marker (row-text byte 13)
                end: 19,
                face: Some(4),
            }],
            matches: Vec::new(),
            highlight: None,
            note_slots: Vec::new(),
        };
        let t = theme::current();
        let view_fg = color(t.view.foreground);
        let span_fg = color(t.syntax_face(4).foreground);
        let mut canvas = iocraft::Canvas::new(80, 1);
        let mut sv = canvas.subview_mut(0, 0, 0, 0, 80, 1);
        draw_line(&mut sv, 0, 4, 76, &row, &t);
        let cell = |x: usize| canvas.cell(x, 0).unwrap();
        // The head is byte-identical (nothing before the marker moved):
        for (i, c) in "map: HashMap<".char_indices() {
            assert_eq!(
                cell(4 + i).text(),
                Some(c.to_string().as_str()),
                "cell {} — `map:` and `HashMap<` must be byte-identical",
                4 + i
            );
        }
        // Cell 17 is the inserted cell — empty (a one-cell error would
        // start `String` there and fill it):
        assert!(
            cell(17).text().is_none(),
            "the inserted marker cell must be empty (the ▴ is the caller's glyph): {:?}",
            cell(17).text()
        );
        // The span that starts after the marker: `String` (row-text chars
        // 13..19) renders at cells 18..24 with its syntax face — a
        // one-cell error here is silently mis-coloured, mis-placed text.
        for (i, c) in "String".char_indices() {
            let cc = cell(18 + i);
            assert_eq!(
                cc.text(),
                Some(c.to_string().as_str()),
                "cell {} — `String` must sit BEHIND the inserted cell",
                18 + i
            );
            assert_eq!(
                cc.text_style().and_then(|s| s.color),
                Some(span_fg),
                "cell {} — the span's face re-based by the insertion",
                18 + i
            );
        }
        // The tail after the span re-bases by the same one cell: `,` at
        // cell 24 (plain position 23), the view face.
        assert_eq!(cell(24).text(), Some(","));
        assert_eq!(cell(24).text_style().and_then(|s| s.color), Some(view_fg));
    }

    /// issue-annotation-marker-cell — THE live reproduction, cell-for-cell:
    /// a nested type with the record on the inner symbol (preceded by `<`
    /// — no whitespace to overwrite). The old behavior fell back to the
    /// line's indent anchor (the cell before the FIELD NAME — a different
    /// symbol). Now the line INSERTS one cell at the symbol's start: the
    /// marker's cell is adjacent to the symbol, and `map:` / `HashMap<`
    /// are byte-identical.
    #[test]
    fn annotation_nested_type_inserts_marker_cell_adjacent_to_the_symbol() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // Line 1: `    let map: HashMap<String, u32> = HashMap::new();` —
        // the record on the inner `String` (char 21; the char before it
        // is `<` — not whitespace; the capture ties the record to the
        // `String` type_identifier — the reproduction's exact tie).
        let content = "fn main() {\n    let map: HashMap<String, u32> = HashMap::new();\n}\n";
        let (store, src_lines) = annotated_store(content, &[(1, 21, "the key type")]);
        assert!(!store.note_rows_folded(), "notes shown by default");
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();
        let lines: Vec<&str> = frame.lines().collect();

        let src_line = &src_lines[1]; // "    let map: HashMap<String, u32> = HashMap::new();"
        let code = shown_code_row(&frame, "u32");

        let arrow_col = col_of(&code, "\u{25b4}")
            .unwrap_or_else(|| panic!("no ▴ on the code row: {code:?}"));
        let symbol_col = col_of(&code, "String")
            .unwrap_or_else(|| panic!("String missing from the code row: {code:?}"));
        // THE anchor relationship, per cell: the marker sits at the
        // symbol's OWN display column (21) — one cell LEFT of `String`
        // (the inserted cell) — NOT at the line's indent anchor (7) or
        // before the field name (the old, wrong, fallback position).
        assert_eq!(arrow_col, 21, "the marker is at the symbol's own display column (21), not the indent anchor 7: {code:?}");
        assert_eq!(symbol_col, 22, "`String` shifted right by EXACTLY one (the inserted cell): {code:?}");
        assert_eq!(arrow_col + 1, symbol_col, "the marker's cell must be ADJACENT to the symbol it marks");
        // `map:` and `HashMap<` byte-identical: the 21 cells before the
        // marker are the source's cells 0..21 (all ASCII, one char each).
        let head_src = &src_line[..21]; // "    let map: HashMap<"
        assert_eq!(
            &code[..21], head_src,
            "the cells before the marker must be byte-identical to the source: code={code:?} source={src_line:?}"
        );
        // From the symbol on, cell-for-cell the source's tail (shifted by
        // the one inserted cell):
        let rendered_tail = display_from(&code, 22).trim_end().to_string();
        assert_eq!(
            rendered_tail, &src_line[21..],
            "from the symbol on, cell-for-cell the source: rendered={rendered_tail:?} source={:?}", &src_line[21..]
        );

        // ANCHOR RELATIONSHIP (per cell): the note row DIRECTLY ABOVE the
        // code row carries its ╭ at the SAME column as the ▴ (21) — the
        // note row is NOT shifted by the code's insertion.
        let code_idx = lines
            .iter()
            .position(|l| l.contains("\u{25b4}") && l.contains("map: HashMap"))
            .unwrap_or_else(|| panic!("code row not found in frame:\n{frame}"));
        assert!(code_idx > 0, "no row above the code row — the note row is missing:\n{frame}");
        let note = lines[code_idx - 1];
        assert!(note.contains("the key type"), "the row directly above the code row must be the note row: {note:?}\n{frame}");
        let corner_col = col_of(note, "\u{256d}")
            .unwrap_or_else(|| panic!("no ╭ on the note row: {note:?}"));
        assert_eq!(
            corner_col, arrow_col,
            "the note row's ╭ (col {corner_col}) must stay at the marker's column (col {arrow_col}) — the code's insertion must not shift it: note={note:?} code={code:?}"
        );
        assert_eq!(corner_col, 21, "the note's ╭ sits at the marker's column (21): {note:?}");
        assert!(note.chars().nth(22) == Some('\u{2500}'), "note row col 22 must be ─ (the corner's bend): {note:?}");
        assert_eq!(col_of(note, "the key type").unwrap(), 23, "note text at col 23 (anchor + 2): {note:?}");
    }

    /// issue-annotation-marker-cell — SEVERAL annotations on one line:
    /// one insertion (the leftmost symbol with no whitespace before it),
    /// the later marker MOVES +1 WITH the code (the existing shift logic),
    /// each marker stays adjacent to its own symbol, and the anchors stay
    /// distinct. The fixture is a HAND-EDITED notes file (with syntax
    /// ties); the `A` key path now addresses per symbol (not per line), so
    /// a line MAY host several records.
    #[test]
    fn annotation_multi_line_one_insertion_later_marker_moves_with_the_code() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // Line 1: `    let v = Vec<Hmm, u32>;` — record 1 on `Hmm` (char
        // 16, display 16, preceded by `<` — the INSERTION at 16), record
        // 2 on `u32` (char 21, display 21, preceded by a space — blank
        // anchor 20, which moves +1 WITH the code to 21: the cell before
        // the shifted `u32` stays the blank one).
        let content = "fn main() {\n    let v = Vec<Hmm, u32>;\n}\n";
        let store = annotated_store_from_notes_file_tied(content, &[
            (1, 16, "note hmm", "Hmm"),
            (1, 21, "note u32", "u32"),
        ]);
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();
        let lines: Vec<&str> = frame.lines().collect();

        let code = shown_code_row(&frame, "u32");
        let src_line = "    let v = Vec<Hmm, u32>;";
        // ONE inserted cell (at 16); the later marker moved +1 WITH the
        // code (20 → 21). Each marker adjacent to its own symbol.
        let arrows = cols_of(&code, '\u{25b4}');
        assert_eq!(
            arrows, vec![16, 21],
            "one insertion at 16; the later blank-cell marker moves +1 with the code (20 -> 21): {code:?}"
        );
        assert_eq!(col_of(&code, "Hmm").unwrap(), 17, "the inserted symbol sits behind its marker: {code:?}");
        assert_eq!(col_of(&code, "u32").unwrap(), 22, "`u32` shifted +1 by the earlier insertion: {code:?}");
        // The head is byte-identical (the insertion is at 16):
        assert_eq!(&code[..16], &src_line[..16], "cells 0..15 byte-identical: {code:?}");
        // Both note rows sit directly above the code row (record order),
        // each ╭ at its own ▴.
        let code_idx = lines
            .iter()
            .position(|l| l.contains("\u{25b4}") && l.contains("Vec<"))
            .unwrap_or_else(|| panic!("code row not found:\n{frame}"));
        assert!(
            code_idx >= 2 && lines[code_idx - 1].contains("note u32") && lines[code_idx - 2].contains("note hmm"),
            "record order: hmm then u32, directly above the code row:\n{frame}"
        );
        let note_hmm = lines.iter().find(|l| l.contains("note hmm")).unwrap();
        let note_u32 = lines.iter().find(|l| l.contains("note u32")).unwrap();
        assert_eq!(
            col_of(note_hmm, "\u{256d}").unwrap(), 16,
            "note hmm's ╭ at its record's insertion anchor (16): {note_hmm:?}"
        );
        assert_eq!(
            col_of(note_u32, "\u{256d}").unwrap(), 21,
            "note u32's ╭ at its record's (shifted) anchor (21): {note_u32:?}"
        );
    }

    /// issue-annotation-marker-cell — two symbols on one line with no
    /// whitespace before EITHER: each keeps its own inserted cell
    /// (`a▴b▴c`-shaped), each marker adjacent to its own symbol, the
    /// tail shifted by both cells, the anchors distinct.
    #[test]
    fn annotation_multi_line_two_insertions_each_symbol_keeps_its_marker() {
        use crate::ui::root::Root;
        use iocraft::prelude::*;
        use std::sync::{Arc, Mutex};

        // Line 1: `    let r = (a,b);` — record on `a` (char 13, display
        // 13, preceded by `(`) and on `b` (char 15, display 15, preceded
        // by `,`): two insertions (13 and 15). Rendered: cells 0..12 byte-
        // identical, ▴@13, `a`@14, `,`@15 (shifted), ▴@16, `b`@17, `)`@18,
        // `;`@19.
        let content = "fn main() {\n    let r = (a,b);\n}\n";
        let store = annotated_store_from_notes_file_tied(content, &[
            (1, 13, "note a", "a"),
            (1, 15, "note b", "b"),
        ]);
        let shared = Arc::new(Mutex::new(store));
        let mut app = element! {
            ContextProvider(value: Context::owned(shared.clone())) {
                Root
            }
        };
        let frame = app.to_string();
        let lines: Vec<&str> = frame.lines().collect();

        let src_line = "    let r = (a,b);";
        let code = shown_code_row(&frame, ")");
        let arrows = cols_of(&code, '\u{25b4}');
        assert_eq!(
            arrows, vec![13, 16],
            "two inserted cells: the first at 13, the second pushed +1 by the first (15 -> 16): {code:?}"
        );
        // Each marker adjacent to its own symbol, the tail shifted by both
        // cells — asserted cell-for-cell:
        assert_eq!(col_of(&code, "a").unwrap(), 14, "`a` behind its marker: {code:?}");
        assert_eq!(code.chars().nth(15), Some(','), "`,` shifted +1 by the first insertion: {code:?}");
        assert_eq!(col_of(&code, "b").unwrap(), 17, "`b` behind its marker: {code:?}");
        assert_eq!(
            code.trim_end(),
            &format!("{}\u{25b4}a,\u{25b4}b);", &src_line[..13]),
            "the whole rendered row cell-for-cell: {code:?}"
        );
        assert_eq!(&code[..13], &src_line[..13], "the cells before the first marker are byte-identical: {code:?}");

        // Both note rows above the code row, each ╭ at its own ▴.
        let code_idx = lines
            .iter()
            .position(|l| l.contains("\u{25b4}") && l.contains("let r = ("))
            .unwrap_or_else(|| panic!("code row not found:\n{frame}"));
        let note_a = lines.iter().find(|l| l.contains("note a")).unwrap();
        let note_b = lines.iter().find(|l| l.contains("note b")).unwrap();
        assert_eq!(col_of(note_a, "\u{256d}").unwrap(), 13, "note a's ╭ at its anchor (13): {note_a:?}");
        assert_eq!(col_of(note_b, "\u{256d}").unwrap(), 16, "note b's ╭ at its anchor (16): {note_b:?}");
        assert!(
            code_idx >= 2 && lines[code_idx - 1].contains("note b") && lines[code_idx - 2].contains("note a"),
            "record order: a then b, directly above:\n{frame}"
        );
    }

    // ── issue-current-line-highlight: the point-row tint (backdrop) ──────
    // The tint is a BACKDROP: painted before the region face and the
    // match/jump bands, so it loses to every existing highlight per cell
    // (jump band > match band > region face > tint > view background). Each
    // precedence row is a distinct observable pinned below, per CELL
    // (canvas background_color), not by a screenshot.
    //
    // Every test here reads the process-global theme (`theme::current()`)
    // to compute its expectations, and ONE of them
    // (`current_line_face_is_read_from_the_theme_not_hard_coded`)
    // swaps that global; the truecolor pins additionally mutate the
    // process-global `COLORTERM` env var. Each test holds
    // `crate::ENV_LOCK` (see `tint_test_lock` below): one mutex excludes
    // BOTH the theme swap and the env mutation from every other env-
    // mutating/reading test in the crate.
    /// Hold the crate-level env lock for the duration of one tint test
    /// (the guard must stay alive until the test ends). These tests
    /// mutate TWO process globals — the theme (`theme::set_current`) and
    /// `COLORTERM` — so they must serialize against EVERY other
    /// env-mutating test in the crate, not just this module's own tests:
    /// `set_var`/`remove_var` are unsafe (edition 2024) because they race
    /// with ANY concurrent `env::var`/`var_os` reader on another thread
    /// (process-wide, not per-variable), and a module-local lock would
    /// not exclude `git::commit`'s `EnvScope` or `model::files`'s
    /// `EnvGuard`. `crate::ENV_LOCK` is that shared lock (main.rs);
    /// holding it also serializes the theme swap against the other tint
    /// tests, as the module-local lock did.
    fn tint_test_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A plain (non-annotated, non-note) code row at buffer line `line`.
    fn tint_code_row(line: usize, text: &str) -> FileViewRow {
        FileViewRow {
            line,
            is_note: false,
            annotated: false,
            anchors: Vec::new(),
            code_start: 0,
            indent_chars: 0,
            insertions: Vec::new(),
            text: text.to_string(),
            spans: Vec::new(),
            matches: Vec::new(),
            highlight: None,
            note_slots: Vec::new(),
        }
    }

    /// Render a `FileViewCanvas` directly at a pinned 80x`height` (the same
    /// fixed-frame seam `note_packing_neighbour_guard_truncates_at_the_next_slot`
    /// uses), so the assertions read the canvas's per-cell backgrounds.
    fn render_tint_canvas(
        rows: Vec<FileViewRow>,
        point_line: usize,
        region_lines: Option<(usize, usize)>,
        notes_folded: bool,
        height: u32,
    ) -> iocraft::Canvas {
        use iocraft::prelude::*;
        let total_rows = rows.len();
        let mut app = element! {
            View(flex_direction: FlexDirection::Column, width: 80, height: height) {
                FileViewCanvas(
                    rows: rows.clone(),
                    total_rows,
                    top_line: 0usize,
                    region_lines,
                    point_line,
                    notes_folded,
                )
            }
        };
        app.render(Some(80))
    }

    /// Every cell of canvas row `y` carries exactly `expected` (or None
    /// when the row must keep the view's normal background).
    fn assert_full_row_bg(canvas: &iocraft::Canvas, y: usize, expected: Option<iocraft::Color>) {
        for x in 0..80 {
            let cell = canvas.cell(x, y).unwrap();
            assert_eq!(
                cell.background_color,
                expected,
                "row {y} cell {x} has the wrong background: {:?} (expected {:?})",
                cell.background_color,
                expected
            );
        }
    }

    /// The point's row is tinted (the whole display row, per cell) and the
    /// rows immediately above and below keep the view's normal background
    /// (no explicit cell background).
    #[test]
    fn current_line_tint_pays_the_point_row_and_not_the_neighbours() {
        let _lock = tint_test_lock();
        let t = theme::current();
        let tint = current_line_bg(&t);
        assert_ne!(
            tint,
            color(t.view.background),
            "the tint must differ from the view background or it is invisible"
        );
        let rows: Vec<FileViewRow> = (0..=4)
            .map(|i| tint_code_row(i, &format!("line {i}")))
            .collect();
        let canvas = render_tint_canvas(rows, 2, None, false, 5);
        assert_full_row_bg(&canvas, 2, Some(tint));
        // The rows immediately above and below the point carry the view's
        // normal background — not the tint (the subtlety, asserted per cell
        // across the whole row, not by a screenshot).
        assert_full_row_bg(&canvas, 1, None);
        assert_full_row_bg(&canvas, 3, None);
        // …and so does every other row in the window.
        assert_full_row_bg(&canvas, 0, None);
        assert_full_row_bg(&canvas, 4, None);
    }

    /// Precedence row 1: a REGION span on the point's line wins over the
    /// tint (the user made that selection deliberately; it is more
    /// salient). The region face also covers the region's OTHER rows, which
    /// the tint must not touch.
    #[test]
    fn current_line_tint_loses_to_the_region_face() {
        let _lock = tint_test_lock();
        let t = theme::current();
        let tint = current_line_bg(&t);
        let region_bg = color(t.region.background);
        assert_ne!(region_bg, tint, "the faces must be distinct or the pin is vacuous");
        let rows: Vec<FileViewRow> = (0..=4)
            .map(|i| tint_code_row(i, &format!("line {i}")))
            .collect();
        // Region over buffer lines 2..=3; the point is on line 2.
        let canvas = render_tint_canvas(rows, 2, Some((2, 3)), false, 5);
        // The point's row: the region face everywhere — the tint lost.
        assert_full_row_bg(&canvas, 2, Some(region_bg));
        // The region's other row: the region face, NOT the tint (no leak).
        assert_full_row_bg(&canvas, 3, Some(region_bg));
        // Outside the region, off the point: the view's normal background.
        assert_full_row_bg(&canvas, 1, None);
        assert_full_row_bg(&canvas, 4, None);
    }

    /// Precedence row 2: a search match on the point's line — the SELECTED
    /// match's band (the match face with a background) wins over the tint
    /// on its cells; outside the band the tint stays (it is the backdrop).
    #[test]
    fn current_line_tint_loses_to_the_selected_match_band() {
        let _lock = tint_test_lock();
        let t = theme::current();
        let tint = current_line_bg(&t);
        let band = color(t.search_match_current.background);
        assert_ne!(band, tint, "the faces must be distinct or the pin is vacuous");
        let mut row = tint_code_row(1, "aaaa bbbb");
        row.matches = vec![LineMatch { start: 0, end: 4, selected: true }];
        let rows = vec![tint_code_row(0, "line 0"), row, tint_code_row(2, "line 2")];
        let canvas = render_tint_canvas(rows, 1, None, false, 3);
        // The band's cells: the match face's background, not the tint.
        for x in 0..4 {
            assert_eq!(
                canvas.cell(x, 1).unwrap().background_color,
                Some(band),
                "cell {x} (under the selected match) must carry the match band, not the tint"
            );
        }
        // The rest of the point's row keeps the tint (backdrop).
        for x in 4..80 {
            assert_eq!(
                canvas.cell(x, 1).unwrap().background_color,
                Some(tint),
                "cell {x} (past the match band) keeps the tint"
            );
        }
        assert_full_row_bg(&canvas, 0, None);
        assert_full_row_bg(&canvas, 2, None);
    }

    /// Precedence row 2 (the dim side): a NON-selected match paints no band
    /// (its face is foreground-only, as before), so on the point's row the
    /// match face's foreground sits ON the tint's backdrop — the tint never
    /// displaces an existing face, it only backs it.
    #[test]
    fn current_line_tint_stays_under_a_plain_match_face() {
        let _lock = tint_test_lock();
        let t = theme::current();
        let tint = current_line_bg(&t);
        let mut row = tint_code_row(1, "aaaa bbbb");
        row.matches = vec![LineMatch { start: 0, end: 4, selected: false }];
        let rows = vec![row];
        let canvas = render_tint_canvas(rows, 1, None, false, 1);
        // The plain match paints no band: its cells keep the tint's
        // background while carrying the match face's foreground (the single
        // rendered row is display row 0).
        for x in 0..4 {
            let cell = canvas.cell(x, 0).unwrap();
            assert_eq!(
                cell.background_color,
                Some(tint),
                "cell {x}: a plain match has no band — the tint stays its backdrop"
            );
            assert_eq!(
                cell.text_style().and_then(|s| s.color),
                Some(color(t.search_match.foreground)),
                "cell {x}: the plain match's foreground still wins the face"
            );
        }
    }

    /// Precedence row 3: a jump landing on the point's line — the jump band
    /// (already the highest precedence) wins over the tint on its cells;
    /// the rest of the row keeps the tint. (The row carries a syntax span
    /// so the line takes `draw_line`'s segment path — where the match and
    /// jump overlays run; a span-less line's early plain-text return never
    /// applied the jump band, before or after the tint.) The expected band
    /// is computed with the same `jump_band_bg` the renderer uses
    /// (truecolor fade when the terminal advertises it, the palette face
    /// otherwise), so the pin holds in both environments.
    #[test]
    fn current_line_tint_loses_to_the_jump_band() {
        let _lock = tint_test_lock();
        let t = theme::current();
        let tint = current_line_bg(&t);
        let band = crate::ui::jump_band_bg(&t, 1.0);
        assert_ne!(band, tint, "the faces must be distinct or the pin is vacuous");
        let mut row = tint_code_row(1, "fn alpha() {");
        row.spans = vec![redline_syntax::highlight::LineSpan {
            start: 0,
            end: 12,
            face: Some(0),
        }];
        row.highlight = Some((3, 8, 1.0));
        let rows = vec![tint_code_row(0, "line 0"), row, tint_code_row(2, "line 2")];
        let canvas = render_tint_canvas(rows, 1, None, false, 3);
        // The landing's cells: the jump band, not the tint.
        for x in 3..8 {
            assert_eq!(
                canvas.cell(x, 1).unwrap().background_color,
                Some(band),
                "cell {x} (under the landing) must carry the jump band, not the tint"
            );
        }
        // The rest of the point's row keeps the tint.
        for x in 0..3 {
            assert_eq!(canvas.cell(x, 1).unwrap().background_color, Some(tint), "cell {x}");
        }
        for x in 8..80 {
            assert_eq!(canvas.cell(x, 1).unwrap().background_color, Some(tint), "cell {x}");
        }
        assert_full_row_bg(&canvas, 0, None);
        assert_full_row_bg(&canvas, 2, None);
    }

    /// Precedence row 4: a synthetic NOTE row never carries the tint (the
    /// region's P3-3 rule — the face belongs to rows that carry buffer
    /// text). The note above the point's code line stays on the view's
    /// normal background; the code line is tinted.
    #[test]
    fn current_line_tint_skips_synthetic_note_rows() {
        let _lock = tint_test_lock();
        let t = theme::current();
        let tint = current_line_bg(&t);
        let note = FileViewRow {
            line: 2,
            is_note: true,
            annotated: false,
            anchors: vec![0],
            code_start: 0,
            indent_chars: 0,
            insertions: Vec::new(),
            text: String::new(),
            spans: Vec::new(),
            matches: Vec::new(),
            highlight: None,
            note_slots: Vec::new(),
        };
        let rows = vec![note, tint_code_row(2, "line 2"), tint_code_row(3, "line 3")];
        let canvas = render_tint_canvas(rows, 2, None, false, 3);
        // The note row (display row 0, buffer line 2 — the point's line)
        // is synthetic: no tint.
        assert_full_row_bg(&canvas, 0, None);
        // The code row carrying the point's line: the tint.
        assert_full_row_bg(&canvas, 1, Some(tint));
        // The next buffer line: nothing.
        assert_full_row_bg(&canvas, 2, None);
    }

    /// Precedence row 5: the point OFF-SCREEN tints nothing — a point line
    /// absent from the rendered rows leaves every row on the view's normal
    /// background.
    #[test]
    fn current_line_off_screen_tints_nothing() {
        let _lock = tint_test_lock();
        // Rows for buffer lines 0, 1, 3, 4 — the point's line (2) is not in
        // the window.
        let rows: Vec<FileViewRow> = [0usize, 1, 3, 4]
            .map(|i| tint_code_row(i, &format!("line {i}")))
            .to_vec();
        let canvas = render_tint_canvas(rows, 2, None, false, 4);
        for y in 0..4 {
            assert_full_row_bg(&canvas, y, None);
        }
    }

    /// The compound case: the point's line that is BOTH in the region and a
    /// selected match — the match band wins on its cells, the region face
    /// fills the rest of the row, and the tint loses everywhere (it is the
    /// backdrop under BOTH).
    #[test]
    fn current_line_tint_loses_where_the_point_row_is_region_and_match() {
        let _lock = tint_test_lock();
        let t = theme::current();
        let region_bg = color(t.region.background);
        let band = color(t.search_match_current.background);
        assert_ne!(band, region_bg, "the faces must be distinct or the pin is vacuous");
        let mut row = tint_code_row(1, "aaaa bbbb");
        row.matches = vec![LineMatch { start: 0, end: 4, selected: true }];
        let rows = vec![tint_code_row(0, "line 0"), row];
        let canvas = render_tint_canvas(rows, 1, Some((1, 1)), false, 2);
        // Match cells: the band. Rest of the row: the region face.
        for x in 0..4 {
            assert_eq!(
                canvas.cell(x, 1).unwrap().background_color,
                Some(band),
                "cell {x}: the match band wins on its own cells"
            );
        }
        for x in 4..80 {
            assert_eq!(
                canvas.cell(x, 1).unwrap().background_color,
                Some(region_bg),
                "cell {x}: the region face fills the row past the band"
            );
        }
        assert_full_row_bg(&canvas, 0, None);
    }

    /// The folded case: a folded annotation emits NO note rows, so the
    /// point's row is the annotated code row itself — the tint lands there
    /// (the tint belongs to rows carrying buffer text; the fold hides the
    /// notes, not the code).
    #[test]
    fn current_line_tint_lands_on_the_code_row_when_notes_are_folded() {
        let _lock = tint_test_lock();
        let t = theme::current();
        let tint = current_line_bg(&t);
        let mut row = tint_code_row(1, "    let x = 1;");
        row.annotated = true;
        row.anchors = vec![0];
        row.code_start = 1; // the column-0 indicator shape
        let canvas = render_tint_canvas(vec![row], 1, None, true, 1);
        // The (single) row is the annotated code row: the whole row is the
        // tint, and the folded ▸ indicator still renders.
        assert_full_row_bg(&canvas, 0, Some(tint));
        assert_eq!(canvas.cell(0, 0).unwrap().text(), Some("\u{25b8}"));
    }

    /// The face is CONFIG-DERIVED, not a hard-coded constant: changing the
    /// theme's `current_line` value changes the rendering. The swap uses a
    /// theme identical to the dark default in every OTHER face (only
    /// `current_line` differs), so concurrent tests reading the global
    /// theme see their expected values unchanged; the guard restores it on
    /// drop even on a failing assertion.
    #[test]
    fn current_line_face_is_read_from_the_theme_not_hard_coded() {
        let _lock = tint_test_lock();
        struct ThemeGuard(theme::Theme);
        impl Drop for ThemeGuard {
            fn drop(&mut self) {
                theme::set_current(self.0.clone());
            }
        }
        let previous = theme::current();
        let _guard = ThemeGuard(previous.clone());
        let mut custom = previous.clone();
        custom.current_line = theme::Face::new(
            previous.view.foreground,
            theme::Color::Rgb(99, 33, 7),
            false,
        );
        let custom_clone = custom.clone();
        theme::set_current(custom);

        let rows: Vec<FileViewRow> = (0..=2)
            .map(|i| tint_code_row(i, &format!("line {i}")))
            .collect();
        let canvas = render_tint_canvas(rows.clone(), 1, None, false, 3);
        // The point's row carries the CHANGED themed value — a hard-coded
        // paint (the default Rgb(35,35,35)) would redden this per cell.
        // The expected value comes from `current_line_bg` (the same function
        // the render uses), so the test is meaningful under both truecolor
        // and 16-color environments.
        let expected_tint = current_line_bg(&custom_clone);
        for x in 0..80 {
            assert_eq!(
                canvas.cell(x, 1).unwrap().background_color,
                Some(expected_tint),
                "cell {x}: the tint must follow the theme's current_line value"
            );
        }
        assert_full_row_bg(&canvas, 0, None);
        assert_full_row_bg(&canvas, 2, None);
        drop(_guard);
        // And the restored theme renders the original shade again.
        let canvas = render_tint_canvas(rows, 1, None, false, 3);
        let restored_tint = current_line_bg(&previous);
        assert_eq!(
            canvas.cell(0, 1).unwrap().background_color,
            Some(restored_tint),
            "after restore the original themed shade renders again"
        );
    }

    // ── issue-current-line-highlight (follow-up): truecolor SGR + 16-colour fallback ──

    /// Verify the exact SGR bytes emitted for the tint under truecolor:
    /// the cell carries `Color::Rgb{r:35, g:35, b:35}`, which iocraft
    /// encodes as SGR `48;2;35;35;35` (the 24-bit truecolor background).
    /// This is the exact byte sequence that reaches the terminal.
    #[test]
    fn current_line_tint_emits_truecolor_sgr_under_colorterm() {
        let _lock = tint_test_lock();
        let t = theme::current();
        // The test environment has COLORTERM=truecolor (verified at
        // gate time). If this assertion fails, the env changed.
        assert!(
            crate::ui::truecolor_enabled(),
            "test requires COLORTERM=truecolor (set it: export COLORTERM=truecolor)"
        );
        let tint = current_line_bg(&t);
        // The exact SGR bytes for this Color value (from iocraft's SgrColor
        // Display impl): `48;2;35;35;35`.
        assert_eq!(
            tint,
            iocraft::Color::Rgb { r: 35, g: 35, b: 35 },
            "truecolor: the tint must be the theme's exact RGB (SGR 48;2;35;35;35)"
        );
        // The canvas cell carries this value (the render path's output).
        let rows: Vec<FileViewRow> = (0..=2)
            .map(|i| tint_code_row(i, &format!("line {i}")))
            .collect();
        let canvas = render_tint_canvas(rows, 1, None, false, 3);
        assert_eq!(
            canvas.cell(0, 1).unwrap().background_color,
            Some(iocraft::Color::Rgb { r: 35, g: 35, b: 35 }),
            "canvas cell: the SGR 48;2;35;35;35 background is painted"
        );
    }

    /// Verify the 16-colour fallback SGR bytes: when COLORTERM is NOT
    /// truecolor/24bit, the tint falls back to `Color::DarkGrey`, which
    /// iocraft encodes as SGR `48;5;8` (the 16-colour palette's DarkGrey
    /// index). Mutation: if `current_line_bg` always returns the Rgb value
    /// (ignoring the capability check), this test reddens.
    #[test]
    fn current_line_tint_falls_back_to_palette_sgr_without_truecolor() {
        let _lock = tint_test_lock();
        let t = theme::current();
        // Save and remove COLORTERM to force the no-truecolor path.
        let prev_colorterm = std::env::var("COLORTERM").ok();
        // SAFETY: the test holds `crate::ENV_LOCK` (via `tint_test_lock`)
        // for its whole body, and every env-mutating test in the crate
        // takes that same lock — no concurrent environment access can
        // interleave.
        unsafe { std::env::remove_var("COLORTERM"); }
        // Now truecolor is disabled: the palette fallback must be used.
        assert!(!crate::ui::truecolor_enabled(), "precondition: truecolor off");
        let tint = current_line_bg(&t);
        // The exact SGR bytes for the fallback: `48;5;8` (DarkGrey).
        // The 16-colour palette's smallest step above Black. Necessarily
        // more visible (~50%) than the truecolor tint (~14%) — the honest
        // consequence of a 16-colour palette.
        assert_eq!(
            tint,
            iocraft::Color::DarkGrey,
            "16-colour: the tint must fall back to DarkGrey (SGR 48;5;8)"
        );
        // The canvas cell carries the fallback value.
        let rows: Vec<FileViewRow> = (0..=2)
            .map(|i| tint_code_row(i, &format!("line {i}")))
            .collect();
        let canvas = render_tint_canvas(rows, 1, None, false, 3);
        assert_eq!(
            canvas.cell(0, 1).unwrap().background_color,
            Some(iocraft::Color::DarkGrey),
            "canvas cell: the 16-colour fallback (SGR 48;5;8) is painted"
        );
        // Restore COLORTERM (or its absence).
        // SAFETY: same invariant as the removal above — the `ENV_LOCK`
        // guard spans the whole test body.
        match prev_colorterm {
            Some(v) => {
                // SAFETY: as above — still under `crate::ENV_LOCK`.
                unsafe { std::env::set_var("COLORTERM", v); }
            }
            None => {
                // SAFETY: as above — still under `crate::ENV_LOCK`.
                unsafe { std::env::remove_var("COLORTERM"); }
            }
        }
    }

    /// Mutation pin: the FALLBACK must not emit truecolor when the terminal
    /// does not support it. If `current_line_bg` is broken to always return
    /// the Rgb value (ignoring `truecolor_enabled()`), the canvas cell would
    /// carry `Rgb{r:35, g:35, b:35}` instead of `DarkGrey`, and this test
    /// reddens.
    #[test]
    fn current_line_tint_fallback_does_not_emit_truecolor_in_16colour_env() {
        let _lock = tint_test_lock();
        let t = theme::current();
        let prev_colorterm = std::env::var("COLORTERM").ok();
        // SAFETY: the test holds `crate::ENV_LOCK` (via `tint_test_lock`)
        // for its whole body, and every env-mutating test in the crate
        // takes that same lock — no concurrent environment access can
        // interleave.
        unsafe { std::env::set_var("COLORTERM", ""); }
        assert!(!crate::ui::truecolor_enabled(), "precondition: truecolor off");
        let tint = current_line_bg(&t);
        // The fallback must NOT be the Rgb value (the mutation: emit truecolor
        // in a 16-colour environment).
        assert_ne!(
            tint,
            iocraft::Color::Rgb { r: 35, g: 35, b: 35 },
            "MUTATION: the 16-colour fallback must NOT emit the truecolor Rgb value"
        );
        // SAFETY: same invariant as the set above — the `ENV_LOCK` guard
        // spans the whole test body, so the restore cannot race a
        // concurrent environment reader.
        match prev_colorterm {
            Some(v) => {
                // SAFETY: as above — still under `crate::ENV_LOCK`.
                unsafe { std::env::set_var("COLORTERM", v); }
            }
            None => {
                // SAFETY: as above — still under `crate::ENV_LOCK`.
                unsafe { std::env::remove_var("COLORTERM"); }
            }
        }
    }
}
