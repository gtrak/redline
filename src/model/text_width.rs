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
}
