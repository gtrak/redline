//! Pure terminal-width helpers (plan 004 issue 05d; moved here from
//! `src/ui/file_view.rs` in 05e so the app layer does not call into the UI
//! layer): the char-index <-> display-column conversions shared by the
//! file-view renderer, the root's hardware-cursor positioning, and the
//! store's click-to-char mapping. Plain Rust — a function takes only a
//! `&str` and a `usize` (zero iocraft, per the `src/model/` layering rule).

/// The terminal-cell width of one char: wide (CJK) chars are 2, combining
/// marks 0; `unicode-width`'s `None` (unknown width) is treated as 1.
pub fn char_display_width(c: char) -> usize {
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

/// issue-mid-line-tabs: expand every TAB in `line` to spaces out to the
/// next 8-column stop, given the running display column at `line`'s start
/// (`code_start` — the frame column the line's first char is drawn at).
/// The 8-stop rule is the same arithmetic `record_anchor` / `leading_indent`
/// own, so the expanded text's cells agree with the anchor (computed on the
/// source line) by construction, and a raw tab never reaches the canvas
/// (whose `width().unwrap_or(0)` would merge it into the preceding cell)
/// or the width helpers (whose `char_display_width('\t') == 1` would
/// undercount it).
///
/// Returns the expanded text (no tabs) plus two maps, both keyed by `line`
///'s own char/byte offsets:
/// - `byte_map`: original byte offset → expanded byte offset
///   (length `line.len() + 1`; exact at char boundaries — the internal
///   bytes of a multibyte char map to that char's expanded start, and a
///   span never starts inside one). For re-basing highlight spans and
///   search-match ranges: a tab is 1 byte but up to 8 cells, so a one-byte
///   error here is silently mis-coloured text on exactly the tab-carrying
///   lines.
/// - `char_map`: original char index → expanded char index (length
///   `char_count + 1`, strictly increasing). For re-basing the inserted
///   marker cells and the point's char index.
///
/// `code_start` is where the line's first char sits in the frame: a tab
/// expands to the stop of the running frame column, so an indented line
/// (drawn at `code_start`) and a column-0 line expand to the same source
/// column.
pub fn expand_tabs(line: &str, code_start: usize) -> (String, Vec<usize>, Vec<usize>) {
    let mut out = String::with_capacity(line.len() + 8);
    let mut abs = code_start;
    let mut byte_map: Vec<usize> = Vec::with_capacity(line.len() + 1);
    let mut char_map: Vec<usize> = Vec::new();
    for c in line.chars() {
        char_map.push(out.chars().count());
        let ebyte = out.len();
        byte_map.push(ebyte);
        match c {
            '\t' => {
                let n = 8 - abs % 8;
                for _ in 0..n {
                    out.push(' ');
                }
                abs += n;
            }
            other => {
                out.push(other);
                abs += char_display_width(other);
            }
        }
        // Internal bytes of a multibyte char: map to the char's expanded
        // start (a span never starts inside one, but the map is complete).
        for _ in 1..c.len_utf8() {
            byte_map.push(ebyte);
        }
    }
    byte_map.push(out.len());
    char_map.push(out.chars().count());
    (out, byte_map, char_map)
}

/// issue-mid-line-tabs: the FRAME display column (running from
/// `code_start`) of char index `idx` in `line`, where a tab advances to
/// the next 8-column stop of the running frame column (the same rule
/// `record_anchor` / `leading_indent` own). With no tabs and `code_start ==
/// 0` this is `char_index_to_display_col`. Used by the click-mapping, which
/// works on the unexpanded source line (not the row's expanded text).
pub fn char_index_to_display_col_tabs(line: &str, code_start: usize, idx: usize) -> usize {
    let mut abs = code_start;
    for (i, c) in line.chars().enumerate() {
        if i == idx {
            return abs;
        }
        abs += match c {
            '\t' => 8 - abs % 8,
            other => char_display_width(other),
        };
    }
    abs
}

/// issue-mid-line-tabs: the char index whose cell contains display column
/// `col` (RELATIVE to the line's start — the line is drawn at `code_start`
/// in the frame) when `line`'s tabs advance to the next 8-column stop of
/// the running ABSOLUTE frame column (`code_start` + the relative running
/// column). The inverse of `char_index_to_display_col_tabs`; a tab's cell
/// spans its whole 8-stop run, so any column in that run maps to the tab's
/// own char. With no tabs and `code_start == 0` this is
/// `display_col_to_char_index`.
pub fn display_col_to_char_index_tabs(line: &str, code_start: usize, col: usize) -> usize {
    let mut rel = 0usize;
    let mut abs = code_start;
    for (i, c) in line.chars().enumerate() {
        let w = match c {
            '\t' => 8 - abs % 8,
            other => char_display_width(other),
        };
        if rel + w > col {
            return i;
        }
        rel += w;
        abs += w;
    }
    line.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn width_table_pins_022_delta_and_stable_anchors() {
        // deps batch B (unicode-width 0.1.14 -> 0.2.2, Unicode 17.0 tables):
        // the FULL-DOMAIN sweep (examples/unicode_width_sweep.rs; recorded
        // raw output in tools/unicode_width_sweep.md) found 458 of
        // 1,112,064 scalars changed width. The delta is EAST ASIAN WIDTH
        // RECLASSIFICATION, not ambiguous-width handling: Neutral -> Wide
        // (1 -> 2; e.g. musical symbols U+1D300-U+1D356, Cyrillic
        // U+4DC0-U+4DFF), mark -> zero-width (2 -> 0; Kanbun
        // U+16FF0-U+16FF1; 1 -> 0; e.g. Kangxi components U+1ACF-U+1ADD /
        // U+1AE0-U+1AEB), and reclassified spacing (0 -> 1; U+1171E,
        // U+11A3A). 0 of the 458 deltas are in the East-Asian-Width
        // "Ambiguous" class, and that class changed ZERO widths across the
        // bump (n=138,197 -> 138,232, all still 1-cell in the default,
        // non-CJK table) — so it is pinned below as a STABLE anchor, not
        // as the delta.
        //
        // First half: one anchor per MEASURED 0.2.2 delta bucket, so a
        // table regression that restores an old width on these scalars
        // would fail here before it could silently shift cursor placement,
        // truncation, or the gutter math for a buffer line containing one.
        assert_eq!(char_display_width('\u{2630}'), 2, "1->2: U+2630 (Neutral -> Wide)");
        assert_eq!(char_display_width('\u{1acf}'), 0, "1->0: U+1ACF (now combining)");
        assert_eq!(char_display_width('\u{16ff0}'), 0, "2->0: U+16FF0 Kanbun pou (now zero-width)");
        assert_eq!(char_display_width('\u{1171e}'), 1, "0->1: U+1171E (now spacing)");
        //
        // Second half: the stable anchors across the same table update —
        // the Ambiguous class stays 1-cell in the non-CJK table, and the
        // unambiguous narrow / wide / emoji / combining anchors stay put.
        let ambiguous: [(char, &str); 3] = [
            ('\u{00a9}', "© COPYRIGHT SIGN"),
            ('\u{0100}', "Ā A WITH MACRON"),
            ('\u{2261}', "≡ IDENTICAL TO"),
        ];
        for (c, name) in ambiguous {
            assert_eq!(char_display_width(c), 1, "{name} stays 1 cell in the non-CJK table");
        }
        assert_eq!(char_display_width('a'), 1, "narrow stays 1");
        assert_eq!(char_display_width('\u{4e2d}'), 2, "中 stays 2");
        assert_eq!(char_display_width('\u{1f980}'), 2, "🦀 stays 2");
        assert_eq!(char_display_width('\u{0301}'), 0, "combining acute stays 0");
    }

    // ── issue-mid-line-tabs: tab expansion + the tab-aware conversions ─

    #[test]
    fn expand_tabs_replaces_a_mid_line_tab_with_spaces_to_the_stop() {
        // `x\t` + 9 y's + ` Z`: source chars x(0) tab(1) y(2..10) space(11)
        // Z(12). The tab at frame col 1 runs to col 8 (7 spaces), shifting
        // the tail by 6, so Z (source char 12) sits at expanded char/byte 18.
        let line = format!("x\t{} Z", "y".repeat(9));
        let (expanded, byte_map, char_map) = expand_tabs(&line, 0);
        assert_eq!(expanded, format!("x{}{} Z", " ".repeat(7), "y".repeat(9)));
        assert!(!expanded.contains('\t'), "the row text must carry no raw tab");
        assert_eq!(char_map[line.chars().count()], 19, "expanded char count");
        assert_eq!(char_map[12], 18, "Z's char index, 6 past its source char (the tab)");
        assert_eq!(byte_map[12], 18, "Z's byte offset, 6 past its source byte (the tab)");
        assert_eq!(byte_map[line.len()], expanded.len(), "EOL maps to the expanded end");
    }

    #[test]
    fn expand_tabs_anchors_the_stop_to_code_start_not_zero() {
        // The SAME source, drawn at code_start 4: the tab's stop is owned
        // from the running frame column, so the expansion differs from a
        // column-0 line (a tab at frame col 13 runs to 16, 3 spaces).
        let line = "let a\tb"; // tab at frame col 4+5 = 9 -> stop 16 (7 spaces)
        let (expanded, byte_map, char_map) = expand_tabs(line, 4);
        // `let a` = 5 cells (cols 4..9); the tab at col 9 -> stop 16 (7);
        // `b` at col 16.
        assert_eq!(expanded, format!("let a{}b", " ".repeat(7)));
        assert_eq!(char_map[6], 12, "b's expanded char index (5 + 7 spaces)");
        assert_eq!(byte_map[6], 12, "b's expanded byte index (ASCII)");
        // A tab at frame col 13 (running past a multiple of 8): `abcd\tX` at
        // code_start 4 -> `abcd` cols 4..8, tab at col 8 -> stop 16 (8), so
        // 8 spaces. `abef\t` at code_start 9 -> tab at col 9+4=13 -> stop 16
        // (3 spaces): the stop is owned from code_start, not 0.
        let (e2, _, _) = expand_tabs("abcd\tX", 4);
        assert_eq!(e2, format!("abcd{}X", " ".repeat(8)));
        let (e3, _, _) = expand_tabs("abef\t", 9);
        assert_eq!(e3, format!("abef{}", " ".repeat(3)));
    }

    #[test]
    fn expand_tabs_no_tab_is_the_identity() {
        let line = "let x = 1;";
        let (expanded, byte_map, char_map) = expand_tabs(line, 0);
        assert_eq!(expanded, line);
        assert_eq!(byte_map.len(), line.len() + 1);
        assert!(byte_map.iter().enumerate().all(|(i, &v)| i == v), "identity byte map");
        assert_eq!(char_map, (0..=line.chars().count()).collect::<Vec<_>>(), "identity char map");
    }

    #[test]
    fn char_index_to_display_col_tabs_matches_record_anchor_arithmetic() {
        // The reproducer's source line, drawn at col 0: a char after a
        // mid-line tab sits at the 8-stop column, NOT one cell past the
        // tab (the char_display_width('\t') == 1 undercount).
        let line = format!("x\t{} Z", "y".repeat(9));
        let z_char = line.chars().take_while(|&c| c != 'Z').count(); // 12
        assert_eq!(char_index_to_display_col_tabs(&line, 0, z_char), 18, "Z at the 8-stop column");
        // A tab's own cell is where its run begins (col 1 here).
        assert_eq!(char_index_to_display_col_tabs(&line, 0, 1), 1, "the tab sits at col 1");
        // code_start 8 (a full stop): the whole line shifts right by 8, so
        // Z lands at 26 — the 8-stop rule runs from code_start, not 0.
        assert_eq!(char_index_to_display_col_tabs(&line, 8, z_char), 26);
    }

    #[test]
    fn display_col_to_char_index_tabs_maps_into_the_tab_run() {
        let line = format!("x\t{} Z", "y".repeat(9));
        // The tab's run is cols 1..8; any column in it maps to the tab's
        // own char (index 1), not the char before or after.
        for col in 1..8 {
            assert_eq!(display_col_to_char_index_tabs(&line, 0, col), 1, "col {col} is inside the tab");
        }
        // Z (char 12) at col 18; the cell before it (17) is the space
        // (char 11) — a one-cell error would return the wrong char.
        assert_eq!(display_col_to_char_index_tabs(&line, 0, 18), 12, "Z");
        assert_eq!(display_col_to_char_index_tabs(&line, 0, 17), 11, "the space before Z");
    }
}
