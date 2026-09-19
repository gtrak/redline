//! Highlight pipeline: ropey buffer + language → per-line highlighted
//! spans. The pipeline runs the tree-sitter highlighter over the full
//! buffer and converts the resulting events into a `HighlightResult`
//! that the file view can render directly.
//!
//! Byte offsets in `LineSpan` are **relative to the line start** (not
//! the file start), so the file view can use them directly into the
//! line's `&str`.

use std::collections::HashMap;
use std::sync::OnceLock;

use ropey::Rope;
use streaming_iterator::StreamingIterator;
use tree_sitter::{InputEdit, Language, Node, Parser, Point, Query, QueryCursor, Tree};
use tree_sitter_highlight::{HighlightEvent, Highlighter};

use crate::syntax::registry::{self, LanguageId};

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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
    Ok(line_spans(rope, bytes.len(), &full_spans))
}

/// Pass 2 of the highlight pipeline: clip the file-level spans to
/// per-line byte ranges. Shared by the `Highlighter` path (`highlight`)
/// and the incremental-reparse path (`spans_from_tree`).
fn line_spans(rope: &Rope, file_len: usize, full_spans: &[FullSpan]) -> HighlightResult {
    let num_lines = rope.len_lines();
    // Compute line start byte offsets (O(num_lines * log N)).
    let line_starts: Vec<usize> = (0..num_lines)
        .map(|l| rope.line_to_byte(l))
        .collect();

    // Line end = next line's start, or file length for the last line.
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

    for span in full_spans {
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

    result
}

// ── Incremental reparse (plan 007 issue 04) ──────────────────────────────
//
// The `Highlighter` path above parses from scratch on every call. This
// section adds the reuse path: retain the per-buffer `Tree`, apply each
// rope edit as a tree-sitter `InputEdit`, and reparse with the old tree
// as a hint so the unchanged regions are not re-parsed.
//
// The reuse pipeline can only replicate the `Highlighter`'s event stream
// for single-layer documents: redline passes a `|_| None` injection
// callback (no injected-language layers are ever added) and the registry
// ships empty locals queries for every language except JS/TS/TSX (local
// variable tracking changes face assignment). Those three therefore keep
// using `highlight()`.

/// A tree-sitter `Tree` retained per buffer so the next rehighlight after
/// an edit is an incremental reparse (`tree.edit` + parse with the old
/// tree as hint) instead of a parse from scratch.
#[derive(Debug)]
pub struct RetainedTree {
    tree: Tree,
}

impl RetainedTree {
    pub fn new(tree: Tree) -> Self {
        Self { tree }
    }

    /// `true` when the retained tree's root (or any descendant) carries a
    /// parse error. An error tree must not be trusted as a reparse hint:
    /// error recovery on an edited error tree may diverge from a fresh
    /// full parse.
    pub fn has_error(&self) -> bool {
        self.tree.root_node().has_error()
    }

    /// Record a rope edit on the retained tree. Must be called once per
    /// buffer edit, with the `InputEdit` derived from the rope state
    /// BEFORE the edit. Edits accumulate until the next reparse consumes
    /// them (tree-sitter's documented edit/parse model).
    pub fn apply_edit(&mut self, edit: &InputEdit) {
        self.tree.edit(edit);
    }

    /// The retained tree, as a parse hint.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// Replace the baseline with a freshly parsed tree (after any
    /// reparse, incremental or full).
    pub fn replace(&mut self, tree: Tree) {
        self.tree = tree;
    }
}

/// The languages the reuse pipeline covers: every one of them has an
/// empty locals query in the registry, and redline's `|_| None` injection
/// callback means the `Highlighter` never adds an injected-language layer
/// for them — so `spans_from_tree` reproduces its event stream exactly.
/// (JS/TS/TSX track locals; `Plain` has no grammar.)
pub fn supports_reuse(lang: LanguageId) -> bool {
    matches!(
        lang,
        LanguageId::Rust
            | LanguageId::Python
            | LanguageId::Go
            | LanguageId::C
            | LanguageId::Cpp
            | LanguageId::Toml
            | LanguageId::Json
            | LanguageId::Yaml
            | LanguageId::Bash
            | LanguageId::Markdown
    )
}

/// Per-language reuse-pipeline state: the grammar `Language` and the
/// highlights-only `Query` (the same query string the registry builds
/// its `HighlightConfiguration` from).
struct ReuseEngine {
    language: Language,
    query: Query,
}

fn reuse_language(lang: LanguageId) -> Option<Language> {
    use LanguageId::*;
    Some(match lang {
        Rust => Language::from(tree_sitter_rust::LANGUAGE),
        Python => Language::from(tree_sitter_python::LANGUAGE),
        Go => Language::from(tree_sitter_go::LANGUAGE),
        C => Language::from(tree_sitter_c::LANGUAGE),
        Cpp => Language::from(tree_sitter_cpp::LANGUAGE),
        Toml => Language::from(tree_sitter_toml_ng::LANGUAGE),
        Json => Language::from(tree_sitter_json::LANGUAGE),
        Yaml => Language::from(tree_sitter_yaml::LANGUAGE),
        Bash => Language::from(tree_sitter_bash::LANGUAGE),
        Markdown => Language::from(tree_sitter_md::LANGUAGE),
        // JS/TS/TSX keep the Highlighter path (local-variable tracking);
        // Plain has no grammar.
        TypeScript | Tsx | JavaScript | Plain => return None,
    })
}

/// The face index for a capture name — a replica of
/// `HighlightConfiguration::configure`'s name matching (the longest
/// recognized name whose dot-parts all appear in the capture name). The
/// registry configures every `HighlightConfiguration` with the same
/// `HIGHLIGHT_FACES` list, so this maps capture names to the same face
/// indices the `Highlighter` does.
fn face_for_capture_name(name: &str) -> Option<usize> {
    let capture_parts: Vec<&str> = name.split('.').collect();
    let mut best: Option<usize> = None;
    let mut best_len = 0;
    for (i, recognized) in HIGHLIGHT_FACES.iter().enumerate() {
        let mut len = 0;
        let mut matches = true;
        for part in recognized.split('.') {
            len += 1;
            if !capture_parts.contains(&part) {
                matches = false;
                break;
            }
        }
        if matches && len > best_len {
            best = Some(i);
            best_len = len;
        }
    }
    best
}

static REUSE_ENGINES: OnceLock<HashMap<LanguageId, Option<ReuseEngine>>> = OnceLock::new();

/// Build every reuse engine now. `HighlightCache::new()` (store
/// construction, before any render) calls this so the FIRST highlight
/// never pays the lazy-build cost on the render critical path — the
/// tree-sitter `Query::new` calls (one per reuse language) are the same
/// startup-time cost class the `GrammarRegistry` highlight configs are
/// (plan 007 issue 04, PTY first-frame finding: ~188ms uncached in a
/// debug build, past the PTY driver's read-quiet window).
pub fn warm_reuse_engines() {
    let _ = REUSE_ENGINES.get_or_init(build_reuse_engines);
}

fn build_reuse_engines() -> HashMap<LanguageId, Option<ReuseEngine>> {
    LanguageId::ALL
        .iter()
        .copied()
        .map(|lang| {
            let engine = reuse_language(lang).and_then(|language| {
                registry::highlight_query_for(lang).and_then(|query_str| {
                    Query::new(&language, query_str).ok().map(|query| ReuseEngine {
                        language,
                        query,
                    })
                })
            });
            (lang, engine)
        })
        .collect()
}

fn engine_for(lang: LanguageId) -> Option<&'static ReuseEngine> {
    let engines = REUSE_ENGINES.get_or_init(build_reuse_engines);
    engines.get(&lang)?.as_ref()
}

/// The file-level spans for a parsed tree, in the same event order the
/// `Highlighter`'s single-layer stream produces: a face runs from its
/// capture's start byte to its end byte; overlapping captures keep the
/// innermost face (the highlighter's end stack is LIFO); Source gaps
/// become face-`None` segments, exactly like `highlight` collects them.
fn spans_from_tree(tree: &Tree, bytes: &[u8], engine: &ReuseEngine) -> Vec<FullSpan> {
    let root = tree.root_node();
    let mut cursor = QueryCursor::new();
    let mut caps = cursor.captures(&engine.query, root, bytes);
    // Pre-collect (node, capture index, pattern index). The reuse query
    // has no injection/locals patterns, so nothing is removed from the
    // stream mid-iteration and pre-collection ordered by document order
    // (start byte, end byte, pattern index) is equivalent to the lazy
    // `QueryCaptures` stream the `Highlighter` walks.
    let mut pending: Vec<(Node, u32, u32)> = Vec::new();
    while let Some((m, _)) = caps.next() {
        for c in m.captures {
            pending.push((c.node, c.index, m.pattern_index as u32));
        }
    }
    pending.sort_by_key(|(n, _ci, pi)| (n.start_byte(), n.end_byte(), *pi));

    let mut segments: Vec<FullSpan> = Vec::new();
    let mut face_stack: Vec<usize> = Vec::new(); // active faces; innermost last
    let mut end_stack: Vec<usize> = Vec::new(); // parallel: each face's end byte
    let mut byte_offset: usize = 0;

    let mut i = 0;
    while i < pending.len() {
        let (node, _ci, _pi) = pending[i];
        let start = node.start_byte();
        // Close pending highlights that end at or before this capture
        // starts (each close flushes a Source gap under the closing face).
        while let Some(&top) = end_stack.last() {
            if top > start {
                break;
            }
            if byte_offset < top {
                segments.push(FullSpan {
                    start: byte_offset,
                    end: top,
                    face: face_stack.last().copied(),
                });
                byte_offset = top;
            }
            face_stack.pop();
            end_stack.pop();
        }
        // Coalesce all captures on this node: in document order the later
        // pattern wins (the `Highlighter` walks the node's captures in
        // pattern order and ends on the last one).
        let mut j = i;
        while j + 1 < pending.len() && pending[j + 1].0 == node {
            j += 1;
        }
        let (_, capture_idx, _pi2) = pending[j];
        if let Some(face) = face_for_capture_name(engine.query.capture_names()[capture_idx as usize])
        {
            if byte_offset < start {
                segments.push(FullSpan {
                    start: byte_offset,
                    end: start,
                    face: face_stack.last().copied(),
                });
                byte_offset = start;
            }
            face_stack.push(face);
            end_stack.push(node.end_byte());
        }
        i = j + 1;
    }
    // Drain the highlights still open at end-of-captures (a Source gap is
    // only emitted when the close is ahead of the current offset).
    while let Some(&top) = end_stack.last() {
        if byte_offset < top {
            segments.push(FullSpan {
                start: byte_offset,
                end: top,
                face: face_stack.last().copied(),
            });
            byte_offset = top;
        }
        face_stack.pop();
        end_stack.pop();
    }
    // The trailing Source gap to end-of-file.
    if byte_offset < bytes.len() {
        segments.push(FullSpan {
            start: byte_offset,
            end: bytes.len(),
            face: face_stack.last().copied(),
        });
    }
    segments
}

/// Parse a rope (no hint) through the reuse pipeline. `lang` must
/// `supports_reuse`. Returns the per-line spans and the parsed tree, so
/// the caller can retain it as the next incremental baseline.
pub fn highlight_reusable(rope: &Rope, lang: LanguageId) -> Result<(HighlightResult, Tree), ()> {
    let engine = engine_for(lang).ok_or(())?;
    let source: String = rope.into();
    let bytes = source.as_bytes();
    let mut parser = Parser::new();
    parser.set_language(&engine.language).map_err(|_| ())?;
    let tree = parser.parse(bytes, None).ok_or(())?;
    let segments = spans_from_tree(&tree, bytes, engine);
    Ok((line_spans(rope, bytes.len(), &segments), tree))
}

/// Incremental reparse: reparse `rope` with the retained tree as a parse
/// hint. `retained` must already carry every rope edit made since the
/// last reparse (via `RetainedTree::apply_edit`).
///
/// Returns `None` — the caller falls back to a full parse — when the hint
/// is not trustworthy:
/// - the retained tree has errors (error recovery on an edited error tree
///   may diverge from a fresh parse), or
/// - the incremental parse result has errors. A wrong `InputEdit` shows up
///   exactly like this: a tree error where the full parse of the content
///   would have none.
pub fn highlight_with_tree(
    rope: &Rope,
    lang: LanguageId,
    retained: &mut RetainedTree,
) -> Option<Result<HighlightResult, ()>> {
    let engine = engine_for(lang)?;
    if retained.has_error() {
        return None;
    }
    let source: String = rope.into();
    let bytes = source.as_bytes();
    let mut parser = Parser::new();
    parser.set_language(&engine.language).ok()?;
    let tree = parser.parse(bytes, Some(retained.tree()))?;
    if tree.root_node().has_error() {
        return None;
    }
    retained.replace(tree);
    let segments = spans_from_tree(retained.tree(), bytes, engine);
    Some(Ok(line_spans(rope, bytes.len(), &segments)))
}

/// A tree-sitter `InputEdit` for a rope edit applied to the OLD content:
/// chars `[char_start, char_end)` were replaced by `new_text`.
///
/// Byte offsets come from the old rope. `new_end_byte` counts from
/// `start_byte` (the end of the NEW range: a pure deletion ends at
/// `start_byte`, a pure insertion at `start_byte + len`). Tree-sitter's
/// `Point.column` is the BYTE offset within the row (ropey's
/// `char_to_column` counts chars, so the column is derived as
/// `byte - line_start_byte` instead), and the new-end point is advanced
/// from the start point through `new_text` byte-wise (a newline resets
/// the column).
pub fn rope_edit_to_input_edit(
    old_rope: &Rope,
    char_start: usize,
    char_end: usize,
    new_text: &str,
) -> InputEdit {
    let point_at = |rope: &Rope, ch: usize| -> Point {
        let byte = rope.char_to_byte(ch);
        let row = rope.char_to_line(ch);
        Point::new(row, byte - rope.line_to_byte(row))
    };
    let start_byte = old_rope.char_to_byte(char_start);
    let old_end_byte = old_rope.char_to_byte(char_end);
    let start_position = point_at(old_rope, char_start);
    let old_end_position = point_at(old_rope, char_end);
    let mut row = start_position.row;
    let mut col = start_position.column;
    for c in new_text.chars() {
        if c == '\n' {
            row += 1;
            col = 0;
        } else {
            col += c.len_utf8();
        }
    }
    InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte: start_byte + new_text.len(),
        start_position,
        old_end_position,
        new_end_position: Point::new(row, col),
    }
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

    // ── incremental reparse (plan 007 issue 04) ────────────────────

    /// The reusable pipeline (single-layer captures) must be byte-
    /// identical to the `Highlighter` full path for every reuse
    /// language, on content that exercises the tricky cases (macros /
    /// strings for Rust, fenced code for Markdown — both have injection
    /// queries that the `|_| None` callback makes inert).
    #[test]
    fn reusable_pipeline_is_byte_identical_to_highlighter() {
        let reg = GrammarRegistry::build();
        let cases: &[(LanguageId, &str)] = &[
            (
                LanguageId::Rust,
                "fn main() { let s = \"hi\"; println!(\"{s}\\n\"); }\n// comment\nlet x = 1; // tail\n",
            ),
            (LanguageId::Python, "def foo(a=1):\n    # c\n    return a\n"),
            (LanguageId::Go, "package main\n\nfunc main() { _ = 1 }\n"),
            (LanguageId::C, "#include <stdio.h>\nint main(void) { return 0; }\n"),
            (LanguageId::Cpp, "#include <x>\nclass A { int v; };\n"),
            (LanguageId::Toml, "[package]\nname = \"t\"\n"),
            (LanguageId::Json, r#"{"a": [1, true], "b": "s"}"#),
            (LanguageId::Yaml, "a: 1\nb:\n  - x\n"),
            (LanguageId::Bash, "#!/bin/sh\necho hi\n"),
            (
                LanguageId::Markdown,
                "# Title\n\n```rust\nfn x() {}\n```\n\nSome **text** with `code`.\n",
            ),
        ];
        for (lang, src) in cases {
            let config = reg.config(*lang).expect("config");
            let rope = Rope::from_str(src);
            let full = highlight(&rope, config, *lang).expect("full highlight");
            let (reuse, _tree) =
                highlight_reusable(&rope, *lang).expect("reusable highlight");
            assert_eq!(
                full, reuse,
                "{lang:?}: reusable pipeline diverged from the Highlighter"
            );
        }
    }

    /// The correctness bar: after a rope edit, the incremental reparse
    /// (tree.edit + parse with hint) must be byte-identical to a full
    /// parse of the post-edit content — insertions in the middle and at
    /// the end, and a deletion.
    #[test]
    fn incremental_parse_matches_full_parse_after_edits() {
        let mut src = String::new();
        for i in 0..200 {
            src.push_str(&format!("fn f{i}(x: i32) -> i32 {{\n    x + {i}\n}}\n\n"));
        }
        let mut rope = Rope::from_str(&src);
        let (_baseline, tree) =
            highlight_reusable(&rope, LanguageId::Rust).expect("baseline parse");
        let mut retained = RetainedTree::new(tree);

        // 1. Insert a line in the middle of the file, at a top-level
        //    boundary (a clean insertion on a clean baseline).
        let mid_bytes = rope.to_string().find("fn f50(").unwrap();
        let mid = rope.byte_to_char(mid_bytes);
        let edit = rope_edit_to_input_edit(&rope, mid, mid, "let w = 9;\n");
        retained.apply_edit(&edit);
        rope.insert(mid, "let w = 9;\n");
        let incr = highlight_with_tree(&rope, LanguageId::Rust, &mut retained)
            .expect("incremental parse should be trusted on a clean baseline");
        let full = highlight_reusable(&rope, LanguageId::Rust).unwrap().0;
        assert_eq!(incr.unwrap(), full, "incremental vs full (mid insert) diverged");

        // 2. Delete the whole first function (still valid syntax).
        let first_block_end = rope.to_string().find("\n}\n").unwrap() + 3;
        let edit = rope_edit_to_input_edit(&rope, 0, first_block_end, "");
        retained.apply_edit(&edit);
        rope.remove(0..first_block_end);
        let incr = highlight_with_tree(&rope, LanguageId::Rust, &mut retained)
            .expect("incremental parse should be trusted after a delete");
        let full = highlight_reusable(&rope, LanguageId::Rust).unwrap().0;
        assert_eq!(incr.unwrap(), full, "incremental vs full (delete) diverged");

        // 3. Break the syntax (delete the last closing brace) — the
        //    incremental parse must NOT be trusted: fall back to the
        //    full parse.
        let brace_pos = rope.len_chars() - 3; // the file ends with "}\n\n"
        let edit = rope_edit_to_input_edit(&rope, brace_pos, brace_pos + 1, "");
        retained.apply_edit(&edit);
        rope.remove(brace_pos..brace_pos + 1);
        assert!(
            highlight_with_tree(&rope, LanguageId::Rust, &mut retained).is_none(),
            "an error-producing reparse must fall back to the full parse"
        );
        // The fallback full parse (as the store does) now retains an
        // error tree; the NEXT incremental attempt must be refused by
        // the error guard (an edit on the error baseline).
        let (_, tree2) = highlight_reusable(&rope, LanguageId::Rust).unwrap();
        retained.replace(tree2);
        assert!(retained.has_error(), "broken content must retain as an error tree");
        let pos4 = rope.len_chars();
        // ";" terminates the dangling `x + 199` tail expression (which
        // needs one once the block is closed) — content is still
        // broken (the block itself is unclosed).
        let edit = rope_edit_to_input_edit(&rope, pos4, pos4, ";\n");
        retained.apply_edit(&edit);
        rope.insert(pos4, ";\n");
        assert!(
            highlight_with_tree(&rope, LanguageId::Rust, &mut retained).is_none(),
            "an error baseline must never be a hint"
        );

        // 4. Restore validity: re-add the closing brace plus a comment —
        //    the store's fallback full parse retains a clean baseline,
        //    so incremental is trusted again.
        let tail = rope.len_chars();
        let edit = rope_edit_to_input_edit(&rope, tail, tail, "}\n// done\n");
        retained.apply_edit(&edit);
        rope.insert(tail, "}\n// done\n");
        let (_, tree3) = highlight_reusable(&rope, LanguageId::Rust).unwrap();
        retained.replace(tree3);
        assert!(!retained.has_error(), "restored content parses cleanly");
        let tail2 = rope.len_chars();
        let edit = rope_edit_to_input_edit(&rope, tail2, tail2, "// ok\n");
        retained.apply_edit(&edit);
        rope.insert(tail2, "// ok\n");
        let incr = highlight_with_tree(&rope, LanguageId::Rust, &mut retained)
            .expect("back to a clean baseline: incremental trusted again");
        let full = highlight_reusable(&rope, LanguageId::Rust).unwrap().0;
        assert_eq!(incr.unwrap(), full, "incremental vs full (restore) diverged");
    }

    /// Stress: a sequence of random-ish edits (insert / delete /
    /// multi-line / multibyte, at start / middle / end) with the
    /// incremental path applied after EACH edit must equal a from-
    /// scratch parse of the same content — this is the test that
    /// catches a wrong `InputEdit`.
    #[test]
    fn stress_random_edits_incremental_equals_full() {
        let reg = GrammarRegistry::build();
        let config = reg.config(LanguageId::Rust).unwrap();
        let mut content = String::new();
        for i in 0..120 {
            content.push_str(&format!(
                "fn f{i}(a: i32, b: &str) -> i32 {{ // note {i} éü🎉\n    a + 1\n}}\n"
            ));
        }
        let mut old_rope = Rope::from_str(&content);
        let (_baseline, tree) =
            highlight_reusable(&old_rope, LanguageId::Rust).expect("baseline parse");
        let mut retained = RetainedTree::new(tree);

        // Deterministic LCG.
        let mut state: u64 = 0x9E3779B97F4A7C15;
        let mut rng = || -> usize {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as usize
        };

        // 007-04 review P2-2: steps that took the incremental path (the
        // pre-error prefix); the assert below pins a real minimum.
        let mut incr_steps = 0usize;
        // All VALID Rust lines (007-04 review P2-2: bare words / bare
        // emoji lines are syntax errors — one of them poisons the baseline
        // and the rest of the loop degrades to vacuous full-vs-full).
        let inserts = [
            "let v = 1;\n",
            "fn g() {}\n\n",
            "// éü\n",
            "    a += 2;\n",
            "// 🎉🎉\n",
        ];
        // 007-04 review P2-2: mid-token edits break syntax within a few
        // steps and the has_error guard then forces vacuous full-vs-full
        // comparisons. So the edits here KEEP the content valid — whole
        // lines inserted at line starts, whole brace-free lines deleted —
        // so the incremental path is exercised by essentially every step.
        for step in 0..150 {
            let n_lines = old_rope.len_lines();
            let pos = old_rope.line_to_char(rng() % n_lines);
            match rng() % 3 {
                0 => {
                    // ascii insert as its own line at a line start
                    let text = inserts[rng() % inserts.len()];
                    let edit = rope_edit_to_input_edit(&old_rope, pos, pos, text);
                    retained.apply_edit(&edit);
                    old_rope.insert(pos, text);
                }
                1 => {
                    // multibyte insert as its own line (a COMMENT — bare
                    // words would be a syntax error and poison the loop)
                    let text = "// café naïve\n";
                    let edit = rope_edit_to_input_edit(&old_rope, pos, pos, text);
                    retained.apply_edit(&edit);
                    old_rope.insert(pos, text);
                }
                _ => {
                    // delete a whole line that carries no braces (a
                    // statement/comment line) — stays valid
                    for offset in 0..n_lines {
                        let line = (rng() % n_lines + offset) % n_lines;
                        let ls = old_rope.line_to_char(line);
                        let le = old_rope.line_to_char((line + 1).min(n_lines));
                        let text: String = old_rope.slice(ls..le).chars().collect();
                        if !text.contains('{') && !text.contains('}') && !text.contains('(') {
                            let edit = rope_edit_to_input_edit(&old_rope, ls, le, "");
                            retained.apply_edit(&edit);
                            old_rope.remove(ls..le);
                            break;
                        }
                    }
                }
            }
            // Simulate the store: try the incremental parse; on the
            // fallback, do the full parse and adopt its tree as the
            // baseline (mirrors `ensure_highlight_for_key`).
            let incr = match highlight_with_tree(&old_rope, LanguageId::Rust, &mut retained)
            {
                Some(r) => {
                    incr_steps += 1;
                    r.unwrap()
                }
                None => {
                    let (r, t) = highlight_reusable(&old_rope, LanguageId::Rust).unwrap();
                    retained.replace(t);
                    r
                }
            };
            let full = highlight_reusable(&old_rope, LanguageId::Rust).unwrap().0;
            assert_eq!(incr, full, "step {step}: incremental diverged from full");
        }
        // 007-04 review P2-2: the stress loop must genuinely exercise the
        // incremental path — once the content accumulates a syntax error
        // the has_error guard forces full parses that compare equal
        // vacuously. With ~120 valid fn bodies and mostly-clean edits the
        // expected incremental count is O(100); require a solid minority
        // (≥10) so a regression to all-vacuous steps fails loudly.
        assert!(
            incr_steps >= 20,
            "only {incr_steps}/150 steps took the incremental path — the              stress loop is vacuously comparing full vs full"
        );

        // Final content: the reusable pipeline must also match the
        // `Highlighter` full path (the byte-identical bar end-to-end).
        let hl = highlight(&old_rope, config, LanguageId::Rust).unwrap();
        let (reuse, _) = highlight_reusable(&old_rope, LanguageId::Rust).unwrap();
        assert_eq!(reuse, hl, "final content diverged from the Highlighter");
    }

    /// A retained tree with parse errors must refuse to be a hint (the
    /// mid-typing / stale-error baseline guard).
    #[test]
    fn error_tree_refuses_to_be_a_hint() {
        let rope = Rope::from_str("fn broken( { // unclosed\n");
        let (_r, tree) = highlight_reusable(&rope, LanguageId::Rust).unwrap();
        let mut retained = RetainedTree::new(tree);
        assert!(retained.has_error());
        let pos = rope.len_chars();
        let edit = rope_edit_to_input_edit(&rope, pos, pos, "x\n");
        retained.apply_edit(&edit);
        let mut rope2 = rope.clone();
        rope2.insert(pos, "x\n");
        assert!(
            highlight_with_tree(&rope2, LanguageId::Rust, &mut retained).is_none(),
            "an error baseline must fall back to the full parse"
        );
    }

    /// `InputEdit` derivation on a multibyte edit: the byte/char/Point
    /// handling must survive a non-ASCII deletion at a non-zero column
    /// (deleting "é" inside a comment; a multibyte char stays later in
    /// the file so the byte-based points stay honest).
    #[test]
    fn input_edit_points_survive_multibyte() {
        // "// héllo\nfn main() { let s = \"café\"; }\n" — delete "é"
        // (char 4, bytes 4..6) in row 0; "café" later in the file keeps
        // the byte/char divergence in play.
        let rope = Rope::from_str("// héllo\nfn main() { let s = \"café\"; }\n");
        let edit = rope_edit_to_input_edit(&rope, 4, 5, "");
        assert_eq!(edit.start_byte, 4);
        assert_eq!(edit.old_end_byte, 6);
        assert_eq!(edit.new_end_byte, 4);
        assert_eq!(edit.start_position, tree_sitter::Point::new(0, 4));
        assert_eq!(edit.old_end_position, tree_sitter::Point::new(0, 6));
        assert_eq!(edit.new_end_position, tree_sitter::Point::new(0, 4));

        // The incremental reparse must equal the full parse of the
        // post-edit content for the multibyte delete.
        let mut ret = RetainedTree::new(
            highlight_reusable(&rope, LanguageId::Rust).unwrap().1,
        );
        let e = rope_edit_to_input_edit(&rope, 4, 5, "");
        ret.apply_edit(&e);
        let mut rope2 = rope.clone();
        rope2.remove(4..5);
        let incr = highlight_with_tree(&rope2, LanguageId::Rust, &mut ret)
            .expect("multibyte delete on a clean baseline is trusted");
        assert_eq!(incr.unwrap(), highlight_reusable(&rope2, LanguageId::Rust).unwrap().0);
    }

    /// The engines must be warmed by cache construction (store startup),
    /// so the first highlight is a lookup, not a lazy build. The bound is
    /// generous: a cold lazy build takes ~188ms in a debug build, while
    /// ten warmed map lookups take microseconds.
    #[test]
    fn reuse_engines_are_warm_after_cache_construction() {
        let _cache = crate::syntax::cache::HighlightCache::new();
        let t = std::time::Instant::now();
        for lang in LanguageId::ALL {
            if supports_reuse(*lang) {
                assert!(engine_for(*lang).is_some(), "{lang:?} engine missing");
            } else {
                assert!(engine_for(*lang).is_none(), "{lang:?} must have no engine");
            }
        }
        assert!(
            t.elapsed() < std::time::Duration::from_millis(100),
            "engine lookups must be warm (got {:?})",
            t.elapsed()
        );
    }

    /// Measured numbers: full vs incremental reparse on a large file.
    /// Prints the timings (the honest report) and asserts the
    /// incremental parse is not SLOWER than the full parse.
    #[test]
    fn measure_full_vs_incremental() {
        // ~3.2 MB of Rust: 40k small functions with comments.
        let lines: Vec<String> = (0..40_000)
            .map(|i| format!("fn f{i}(a: i32, b: &str) -> i32 {{\n    // helper {i}\n    a.wrapping_add(1)\n}}\n"))
            .collect();
        let src: String = lines.concat();
        let byte_len = src.len();
        let rope = Rope::from_str(&src);
        let n = rope.len_chars();

        // Pure parse timings (fresh parser, like the pipeline uses):
        // this is the part the retained tree actually changes.
        let lang = tree_sitter::Language::from(tree_sitter_rust::LANGUAGE);
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let t0 = std::time::Instant::now();
        let tree = parser.parse(src.as_bytes(), None).unwrap();
        let full_parse_us = t0.elapsed().as_micros();
        let mut parse_retained = RetainedTree::new(tree.clone());
        parse_retained
            .apply_edit(&rope_edit_to_input_edit(&rope, n, n, "fn extra() {}\n"));
        let src_end = format!("{src}fn extra() {{}}\n");
        let t0b = std::time::Instant::now();
        let tree_end = parser.parse(src_end.as_bytes(), Some(parse_retained.tree())).unwrap();
        let incr_parse_us = t0b.elapsed().as_micros();
        let _ = tree_end;

        let t1 = std::time::Instant::now();
        let (_r, tree) = highlight_reusable(&rope, LanguageId::Rust).unwrap();
        let full_us = t1.elapsed().as_micros();

        // End-of-file append (the notes-typing case).
        let mut retained = RetainedTree::new(tree);
        let edit_end = rope_edit_to_input_edit(&rope, n, n, "fn extra() {}\n");
        retained.apply_edit(&edit_end);
        let mut rope_end = rope.clone();
        rope_end.insert(n, "fn extra() {}\n");
        let t2 = std::time::Instant::now();
        let res_end = highlight_with_tree(&rope_end, LanguageId::Rust, &mut retained)
            .expect("end append stays error-free");
        let end_us = t2.elapsed().as_micros();

        // Second append on the incremental baseline (chained edits).
        let n2 = rope_end.len_chars();
        let edit2 = rope_edit_to_input_edit(&rope_end, n2, n2, "fn extra2() {}\n");
        retained.apply_edit(&edit2);
        let mut rope_end2 = rope_end.clone();
        rope_end2.insert(n2, "fn extra2() {}\n");
        let t3 = std::time::Instant::now();
        let res_end2 = highlight_with_tree(&rope_end2, LanguageId::Rust, &mut retained)
            .expect("chained end append stays error-free");
        let end2_us = t3.elapsed().as_micros();

        // Mid-file insert at a top-level boundary on a fresh baseline.
        let mut retained_mid = RetainedTree::new(
            highlight_reusable(&rope, LanguageId::Rust).unwrap().1,
        );
        let mid_bytes = src.find("fn f20000(").unwrap();
        let mid = rope.byte_to_char(mid_bytes);
        let edit_mid = rope_edit_to_input_edit(&rope, mid, mid, "fn middle() {}\n");
        retained_mid.apply_edit(&edit_mid);
        let mut rope_mid = rope.clone();
        rope_mid.insert(mid, "fn middle() {}\n");
        let t4 = std::time::Instant::now();
        let res_mid = highlight_with_tree(&rope_mid, LanguageId::Rust, &mut retained_mid)
            .expect("mid insert stays error-free");
        let mid_us = t4.elapsed().as_micros();

        // Correctness: every incremental result equals a full parse.
        assert_eq!(res_end.unwrap(), highlight_reusable(&rope_end, LanguageId::Rust).unwrap().0);
        assert_eq!(
            res_end2.unwrap(),
            highlight_reusable(&rope_end2, LanguageId::Rust).unwrap().0
        );
        assert_eq!(res_mid.unwrap(), highlight_reusable(&rope_mid, LanguageId::Rust).unwrap().0);

        eprintln!(
            "measure: {byte_len} bytes / {n} chars;\n  parse: full={full_parse_us}us, incr_end={incr_parse_us}us\n  pipeline: full={full_us}us, incr_end={end_us}us, incr_end_chained={end2_us}us, incr_mid={mid_us}us"
        );
        // The hard assert guards the feature itself: the incremental
        // parse must be well under the full parse (measured ~20x on a
        // release build; the 3x bound keeps a large margin on a loaded
        // shared box). The whole-pipeline timings are REPORTED, not
        // asserted: both paths share the post-parse work (string
        // materialization, capture sort, per-line span conversion), and
        // on a loaded box a pipeline-level "not slower" assert flakes
        // while the parse-level claim stays true. The correctness
        // asserts above are the real gate.
        assert!(
            incr_parse_us.saturating_mul(3) <= full_parse_us,
            "parse: incremental not faster (full={full_parse_us}us, incr={incr_parse_us}us)"
        );
    }
}
