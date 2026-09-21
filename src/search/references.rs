//! `M-?` references (issue 06): a word-boundary, fixed-string search over
//! the project, filtered by tree-sitter token class — comment/string hits
//! are dropped where a grammar exists; plain word-boundary search is the
//! fallback elsewhere (plan decision #4: "references from embedded
//! ripgrep with tree-sitter comment/string filtering").
//!
//! The ripgrep layer supplies candidates only: the filtering is the
//! per-file [`references_filter`] hook (comment/string byte ranges from
//! `redline_syntax::tokens`), which the pipeline's sink applies to each hit.

use std::ops::Range;
use std::path::Path;

use redline_syntax::registry::{resolve_language, LanguageId};
use redline_syntax::tokens::comment_string_ranges;
use crate::search::rg::{SearchConfig, SearchBus};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// The per-file token-class filter for references search: comment/string
/// ranges when the file's language has a grammar (those hits are
/// dropped); `None` (no filtering) for plain text — the documented
/// fallback, where comment/string hits are kept.
pub fn references_filter(path: &Path, text: &str) -> Option<Vec<Range<usize>>> {
    let lang = resolve_language(&path.to_string_lossy());
    if lang == LanguageId::Plain {
        return None;
    }
    Some(comment_string_ranges(lang, text))
}

/// The `SearchConfig` for a references search of `symbol` rooted at
/// `root`: fixed-string + word-boundary + case-sensitive identifiers.
pub fn references_config(root: std::path::PathBuf, symbol: String) -> SearchConfig {
    SearchConfig {
        root,
        pattern: symbol,
        word: true,
        fixed: true,
        case_smart: false, // identifiers are case-sensitive
        case_insensitive: false,
        glob: None,
        file_type: None,
        filter: Some(Arc::new(references_filter)),
        cancel: Arc::new(AtomicBool::new(false)),
    }
}

/// Spawn a references search (see `rg::spawn` for the streaming /
/// cancellation contract).
pub fn spawn_references(cfg: SearchConfig, bus: &SearchBus, generation: usize) {
    crate::search::rg::spawn(cfg, bus, generation);
}

/// Extract the "symbol under point" from a line of text at byte column
/// `col`. Identifier candidates (alphanumeric/underscore runs) are
/// ranked: (1) the identifier the point is on wins outright (e.g. point
/// on `bar` in `use foo::bar;` searches `bar`, not the earlier `foo`);
/// (2) otherwise, among identifiers at or before the point, a known
/// identifier wins, else the nearest one to the left of the point;
/// (3) otherwise (point before the first identifier) the old fallback:
/// the first identifier the symbol index knows about, else the first
/// identifier on the line. `None` when the line has no identifier.
pub fn symbol_under_point(line: &str, col: usize, is_known: impl Fn(&str) -> bool) -> Option<&str> {
    // Identifier byte ranges (start, end, text).
    let mut ids: Vec<(usize, usize, &str)> = Vec::new();
    let mut start = 0;
    for (i, c) in line.char_indices() {
        if !crate::model::buffer::is_word_char(c) {
            if start < i {
                ids.push((start, i, &line[start..i]));
            }
            start = i + c.len_utf8();
        }
    }
    if start < line.len() {
        ids.push((start, line.len(), &line[start..]));
    }
    if ids.is_empty() {
        return None;
    }
    // (1) The point is on an identifier (inclusive bounds: a point
    // right after an identifier counts as being on it).
    if let Some((_, _, id)) = ids.iter().find(|t| t.0 <= col && col <= t.1) {
        return Some(id);
    }
    // (2) At or before the point: known first, else nearest to the left.
    let at_or_before: Vec<&(usize, usize, &str)> = ids.iter().filter(|&&(s, _, _)| s <= col).collect();
    if let Some((_, _, id)) = at_or_before.iter().rev().find(|&&(_, _, id)| is_known(id)) {
        return Some(id);
    }
    if let Some((_, _, id)) = at_or_before.last() {
        return Some(id);
    }
    // (3) Point before the first identifier: first known, else first.
    ids.iter()
        .find(|&&(_, _, id)| is_known(id))
        .or_else(|| ids.first())
        .map(|&(_, _, id)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::rg::SearchEvent;
    use std::fs;
    use std::time::Duration;

    /// C15: symbol extraction uses the crate-wide Unicode word-char rule —
    /// `é` is a word character, so a multibyte identifier is extracted
    /// whole, not truncated at its ASCII prefix.
    #[test]
    fn symbol_under_point_multibyte_identifier() {
        // "fn café()" — the identifier spans bytes 3..8 (`café`).
        assert_eq!(symbol_under_point("fn café()", 5, |_| false), Some("café"));
        assert_eq!(symbol_under_point("fn café()", 4, |_| false), Some("café"));
        // CJK identifier: extracted whole (the `(` separator keeps it a
        // single identifier).
        assert_eq!(symbol_under_point("(漢字)", 4, |_| false), Some("漢字"));
    }

    /// Build a project with `src/main.rs` holding the same identifier in
    /// code, a comment, and a string (the spec's reference-filtering case).
    fn ref_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        fs::write(
            dir.path().join("src/main.rs"),
            "fn target() {}\n// call target here\nfn main() { let s = \"target in a string\"; target(); }\n",
        )
        .unwrap();
        dir
    }

    /// Drain until Finished (bounded), returning the events.
    fn drain_until_finished(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<SearchEvent>,
    ) -> Vec<SearchEvent> {
        let start = std::time::Instant::now();
        let mut out = Vec::new();
        loop {
            while let Ok(ev) = rx.try_recv() {
                if matches!(ev, SearchEvent::Finished { .. }) {
                    return out;
                }
                out.push(ev);
            }
            if start.elapsed() > Duration::from_secs(5) {
                return out;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Rust (a grammar language): the comment and string hits are dropped;
    /// the code hits remain.
    #[test]
    fn references_drops_comment_and_string_hits() {
        let dir = ref_project();
        let cfg = references_config(dir.path().to_path_buf(), "target".into());
        let (bus, mut rx) = SearchBus::new();
        spawn_references(cfg, &bus, 0);
        let events = drain_until_finished(&mut rx);

        // Hand computation for `target` in src/main.rs with filtering:
        // line 1 `fn target() {}` (code) and line 3 `target();` (code)
        // remain; line 2 (comment) and line 3's `"target in a string"`
        // occurrence are dropped. The line-oriented sink reports line 3
        // once (its code match survives; the in-line string occurrence is
        // a separate byte offset that is filtered).
        let hit_lines: Vec<u64> = events
            .iter()
            .filter_map(|ev| match ev {
                SearchEvent::Hit { line_no, .. } => Some(*line_no),
                _ => None,
            })
            .collect();
        assert!(hit_lines.contains(&1), "code hit on line 1 kept: {hit_lines:?}");
        assert!(hit_lines.contains(&3), "code hit on line 3 kept: {hit_lines:?}");
        assert!(
            !hit_lines.contains(&2),
            "the comment hit (line 2) must be dropped: {hit_lines:?}"
        );
        // The string occurrence is on line 3: the hit must come from the
        // code call, so its column must NOT be inside the string literal.
        let hit3 = events
            .iter()
            .find_map(|ev| match ev {
                SearchEvent::Hit { line_no, col, line, .. } if *line_no == 3 => {
                    Some((*col, line.clone()))
                }
                _ => None,
            })
            .expect("line 3 hit");
        let (col, line_text) = hit3;
        let col = col.expect("a fixed-string hit must carry a column");
        // The code call `target();` starts at byte 32 of
        // `fn main() { let s = "target in a string"; target(); }`.
        let expected = line_text[32..].find("target").unwrap() + 32;
        assert_eq!(col as usize, expected, "line 3 hit must be the code call");
        drop(bus);
    }

    /// Fallback (plain text, no grammar): comment/string hits are KEPT.
    #[test]
    fn references_fallback_keeps_comment_and_string_hits() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        // A plain-text file (`.txt` has no grammar): the same identifier in
        // comment-like and string-like lines must all be kept.
        fs::write(
            dir.path().join("src/notes.txt"),
            "# target in a comment\n\"target in a string\"\ntarget in code\n",
        )
        .unwrap();
        let cfg = references_config(dir.path().to_path_buf(), "target".into());
        let (bus, mut rx) = SearchBus::new();
        spawn_references(cfg, &bus, 0);
        let events = drain_until_finished(&mut rx);
        let hit_lines: Vec<u64> = events
            .iter()
            .filter_map(|ev| match ev {
                SearchEvent::Hit { line_no, .. } => Some(*line_no),
                _ => None,
            })
            .collect();
        assert_eq!(hit_lines, vec![1, 2, 3], "plain text keeps all hits: {hit_lines:?}");
        drop(bus);
    }

    /// `symbol_under_point`: the identifier under the point wins; known
    /// identifiers win among those at or before the point; before the
    /// first identifier the old first-known/first fallback applies.
    #[test]
    fn symbol_under_point_prefers_point_position() {
        // `use foo::bar;`: point on `bar` (byte 10) searches `bar`, even
        // though `foo` is the known identifier (the review's example).
        assert_eq!(
            symbol_under_point("use foo::bar;", 10, |id| id == "foo"),
            Some("bar")
        );
        // Point on `target`: the identifier under the point.
        assert_eq!(
            symbol_under_point("fn main() { target(); }", 13, |id| id == "target"),
            Some("target")
        );
        // Point at col 0 on the first identifier (`let` spans 0..3).
        assert_eq!(symbol_under_point("let x = 1;", 0, |_| false), Some("let"));
        // Point on a separator, before a known identifier: the nearest
        // identifier left of the point (`main` ends before the gap at 11).
        assert_eq!(
            symbol_under_point("fn main() { target(); }", 11, |id| id == "target"),
            Some("main")
        );
        // Point before the first identifier (leading space): first known
        // on the line wins, else the first identifier.
        assert_eq!(
            symbol_under_point("  fn main() { target(); }", 0, |id| id == "target"),
            Some("target")
        );
        assert_eq!(
            symbol_under_point("  fn main();", 0, |_| false),
            Some("fn")
        );
        // Empty line: none.
        assert_eq!(symbol_under_point("   ", 0, |_| true), None);
    }
}
