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
}
