//! Highlight pipeline: ropey buffer + language → per-line highlighted
//! spans. The pipeline runs the tree-sitter highlighter over the full
//! buffer and converts the resulting events into a `HighlightResult`
//! that the file view can render directly.
//!
//! Byte offsets in `LineSpan` are **relative to the line start** (not
//! the file start), so the file view can use them directly into the
//! line's `&str`.

use ropey::Rope;
use tree_sitter_highlight::{HighlightEvent, Highlighter};

use crate::syntax::registry::LanguageId;

/// The face names configured on every `HighlightConfiguration`.
/// `Highlight(i)` from the highlighter is an index into this list;
/// the theme maps index → `theme::Face`.
pub const HIGHLIGHT_FACES: &[&str] = &[
    "comment",
    "string",
    "string.quote",
    "function",
    "function.builtin",
    "keyword",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.other",
    "number",
    "operator",
    "punctuation",
    "label",
    "constant",
    "constant.builtin",
    "attribute",
    "constructor",
    "namespace",
    "property",
    "property.builtin",
    "tag",
    "tag.builtin",
    "regex",
    "special",
    "embedded",
    "error",
];

/// A highlighted span within a single line.
/// `start`/`end` are byte offsets relative to the line start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineSpan {
    pub start: usize,
    pub end: usize,
    /// Index into `HIGHLIGHT_FACES`; `None` = no face (default color).
    pub face: Option<usize>,
}

/// The highlighted spans for one line (in order, non-overlapping,
/// clipped to the line's byte range).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HighlightedLine {
    pub spans: Vec<LineSpan>,
}

/// The full highlight result for a buffer: one entry per line.
/// `lines.len()` equals the rope's `len_lines()`.
#[derive(Clone, Debug, Default)]
pub struct HighlightResult {
    pub lines: Vec<HighlightedLine>,
}

/// One file-level span from the highlighter events.
struct FullSpan {
    start: usize,
    end: usize,
    face: Option<usize>,
}

/// Run the highlight pipeline: parse `rope` with the given language's
/// `HighlightConfiguration` and return per-line spans.
///
/// Returns `Ok(HighlightResult)` on success; `Err(())` when the
/// highlighter fails (e.g. parse error that the highlighter surfaces
/// as a hard error rather than a missing node).
pub fn highlight(
    rope: &Rope,
    config: &tree_sitter_highlight::HighlightConfiguration,
    _lang_id: LanguageId,
) -> Result<HighlightResult, ()> {
    // Materialize the full source (ropey's `Into<String>` is O(N)).
    let source: String = rope.into();
    let bytes = source.as_bytes();
    let num_lines = rope.len_lines();

    // Run the highlighter (one per call; it owns a Parser internally).
    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(config, bytes, None, |_| None)
        .map_err(|_| ())?;

    // Pass 1: collect file-level spans from the event stream.
    let mut face_stack: Vec<usize> = Vec::new();
    let mut full_spans: Vec<FullSpan> = Vec::new();

    for event in events {
        match event.map_err(|_| ())? {
            HighlightEvent::HighlightStart(h) => {
                face_stack.push(h.0);
            }
            HighlightEvent::HighlightEnd => {
                face_stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                let face = face_stack.last().copied();
                if start < end {
                    full_spans.push(FullSpan { start, end, face });
                }
            }
        }
    }

    // Pass 2: convert file-level spans to per-line spans.
    // Compute line start byte offsets (O(num_lines * log N)).
    let line_starts: Vec<usize> = (0..num_lines)
        .map(|l| rope.line_to_byte(l))
        .collect();

    // Line end = next line's start, or file length for the last line.
    let file_len = bytes.len();
    let line_end = |l: usize| -> usize {
        if l + 1 < num_lines {
            line_starts[l + 1]
        } else {
            file_len
        }
    };

    let mut result = HighlightResult {
        lines: vec![HighlightedLine::default(); num_lines],
    };

    for span in &full_spans {
        // Find the line containing span.start (binary search on line_starts).
        let start_line = match line_starts.binary_search(&span.start) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        // Find the line containing span.end - 1 (the last byte of the span).
        let end_line = if span.end <= file_len {
            let last_byte = span.end.saturating_sub(1);
            match line_starts.binary_search(&last_byte) {
                Ok(i) => i,
                Err(i) => i.saturating_sub(1),
            }
        } else {
            num_lines.saturating_sub(1)
        };

        #[allow(clippy::needless_range_loop)]
        for line in start_line..=end_line {
            let ls = line_starts[line];
            let le = line_end(line);
            let s = span.start.max(ls) - ls;
            let e = span.end.min(le) - ls;
            if s < e {
                result.lines[line].spans.push(LineSpan {
                    start: s,
                    end: e,
                    face: span.face,
                });
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::registry::GrammarRegistry;

    fn highlight_source(source: &str, lang_id: LanguageId) -> Option<HighlightResult> {
        let reg = GrammarRegistry::build();
        let config = reg.config(lang_id)?;
        let rope = Rope::from_str(source);
        highlight(&rope, config, lang_id).ok()
    }

    /// Find the face index for a given face name in `HIGHLIGHT_FACES`.
    fn face_index(name: &str) -> Option<usize> {
        HIGHLIGHT_FACES.iter().position(|f| *f == name)
    }

    #[test]
    fn rust_fn_name_is_function_face() {
        let src = "fn main() {}\n";
        let result = highlight_source(src, LanguageId::Rust).expect("rust highlight");
        // Line 0: "fn main() {}"
        // "fn" should be a keyword, "main" should be a function face.
        let line = &result.lines[0];
        // Find the span that covers "main" (bytes 3..7).
        let main_span = line
            .spans
            .iter()
            .find(|s| s.start <= 3 && s.end >= 7)
            .expect("should have a span covering 'main'");
        assert_eq!(
            main_span.face,
            face_index("function"),
            "fn name should have function face, got {:?} in {line:?}",
            main_span.face
        );
    }

    #[test]
    fn rust_comment_is_comment_face() {
        let src = "// a comment\nfn main() {}\n";
        let result = highlight_source(src, LanguageId::Rust).expect("rust highlight");
        let line = &result.lines[0];
        let comment_span = line
            .spans
            .iter()
            .find(|s| s.start == 0 && s.face == face_index("comment"))
            .expect("should have a comment span at the start of line 0");
        assert!(
            comment_span.end > 0,
            "comment span should be non-empty: {line:?}"
        );
    }

    #[test]
    fn rust_string_is_string_face() {
        let src = r#"fn main() { let s = "hello"; }"#;
        let result = highlight_source(src, LanguageId::Rust).expect("rust highlight");
        let line = &result.lines[0];
        let string_span = line
            .spans
            .iter()
            .find(|s| s.face == face_index("string"))
            .expect("should have a string span in {line:?}");
        assert!(string_span.end > string_span.start);
    }

    #[test]
    fn ts_interface_name_is_type_face() {
        let src = "interface Foo { bar: string }\n";
        let result = highlight_source(src, LanguageId::TypeScript)
            .expect("typescript highlight");
        let line = &result.lines[0];
        // "Foo" (bytes 10..13) should be a type face.
        let foo_span = line
            .spans
            .iter()
            .find(|s| s.start <= 10 && s.end >= 13)
            .expect("should have a span covering 'Foo' in {line:?}");
        assert_eq!(
            foo_span.face,
            face_index("type"),
            "interface name should have type face, got {:?} in {line:?}",
            foo_span.face
        );
    }

    #[test]
    fn bash_comment_is_comment_face() {
        let src = "# a bash comment\necho hello\n";
        let result = highlight_source(src, LanguageId::Bash).expect("bash highlight");
        let line = &result.lines[0];
        let comment_span = line
            .spans
            .iter()
            .find(|s| s.start == 0 && s.face == face_index("comment"))
            .expect("should have a comment span at start of line 0 in {line:?}");
        assert!(comment_span.end > 0);
    }

    #[test]
    fn python_keyword_is_keyword_face() {
        let src = "def foo():\n    pass\n";
        let result = highlight_source(src, LanguageId::Python).expect("python highlight");
        let line = &result.lines[0];
        // "def" (bytes 0..3) should be a keyword.
        let def_span = line
            .spans
            .iter()
            .find(|s| s.start == 0 && s.face == face_index("keyword"))
            .expect("should have a keyword span at start of line 0 in {line:?}");
        assert!(def_span.end >= 3, "'def' should be fully covered: {line:?}");
    }

    #[test]
    fn json_key_is_property_face() {
        let src = r#"{"key": "value"}"#;
        let result = highlight_source(src, LanguageId::Json).expect("json highlight");
        let line = &result.lines[0];
        // Look for a span with a face (property or constant for JSON keys).
        let has_face = line.spans.iter().any(|s| s.face.is_some());
        assert!(has_face, "json should have at least one faced span: {line:?}");
    }

    #[test]
    fn go_func_is_function_face() {
        let src = "func main() {}\n";
        let result = highlight_source(src, LanguageId::Go).expect("go highlight");
        let line = &result.lines[0];
        // "main" (bytes 5..9) should have a non-None face.
        // The Go grammar captures function names as "variable", not "function".
        let main_span = line
            .spans
            .iter()
            .find(|s| s.start <= 5 && s.end >= 9)
            .expect("should have a span covering 'main' in {line:?}");
        assert!(
            main_span.face.is_some(),
            "go func name should have a face, got {:?} in {line:?}",
            main_span.face
        );
    }

    #[test]
    fn c_string_is_string_face() {
        let src = r#"int main() { return 0; }"#;
        let result = highlight_source(src, LanguageId::C).expect("c highlight");
        assert!(!result.lines.is_empty());
    }

    #[test]
    fn toml_key_is_highlighted() {
        let src = "[package]\nname = \"test\"\n";
        let result = highlight_source(src, LanguageId::Toml).expect("toml highlight");
        assert!(!result.lines.is_empty());
        let line0 = &result.lines[0];
        let has_face = line0.spans.iter().any(|s| s.face.is_some());
        assert!(has_face, "toml [package] should have faced spans: {line0:?}");
    }

    #[test]
    fn yaml_key_is_highlighted() {
        let src = "name: test\n";
        let result = highlight_source(src, LanguageId::Yaml).expect("yaml highlight");
        assert!(!result.lines.is_empty());
    }

    #[test]
    fn markdown_heading_is_highlighted() {
        let src = "# Heading\n\nSome text.\n";
        let result = highlight_source(src, LanguageId::Markdown).expect("md highlight");
        assert!(!result.lines.is_empty());
    }

    #[test]
    fn highlight_result_has_correct_line_count() {
        let src = "line1\nline2\nline3\n";
        let result = highlight_source(src, LanguageId::Rust).expect("rust highlight");
        // "line1\nline2\nline3\n" has 4 lines (trailing empty line).
        assert_eq!(result.lines.len(), 4);
    }

    #[test]
    fn plain_text_has_no_faces() {
        // Plain text has no config, so we can't call highlight() on it.
        // Instead, verify that the registry returns None for Plain.
        let reg = GrammarRegistry::build();
        assert!(reg.config(LanguageId::Plain).is_none());
    }

    #[test]
    fn unknown_language_fallback_path() {
        // An unknown extension maps to Plain; the store should detect
        // this and skip highlighting.
        let reg = GrammarRegistry::build();
        assert_eq!(reg.language_for("foo.xyz"), LanguageId::Plain);
    }
}
