//! Token-class byte ranges (comment/string) for reference filtering
//! (issue 06): one tree-sitter QUERY per language, built from that
//! language's highlight query string, capturing every face whose name is
//! `comment` or a `string.*` face — except where a pinned highlight query
//! is minimal and captures no such face (TypeScript, TSX, C++: a
//! dedicated supplementary query targets the grammar's comment/string
//! node kinds), and except Markdown, which has no comment or string node
//! kinds at all (the filter is inactive there; documented exception). The
//! issue 05 query infrastructure (thread-local parser + cached `Query`)
//! is reused; all tree-sitter churn stays in `src/syntax/`.
//!
//! `search/references` asks for these ranges per file: hits whose byte
//! offset falls inside one are dropped (comment/string hits); files whose
//! language is `Plain` get no ranges at all (the documented plain-search
//! fallback — comment/string hits are kept).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

use crate::syntax::language::{spec, TokenClass};
use crate::syntax::queries::language_for;
use crate::syntax::registry::LanguageId;

/// The token-class (comment/string) query for `lang` — a field read
/// over the descriptor table (the old match, which re-pinned the TS/TSX
/// and C++ dedicated queries and the Markdown exception alongside a
/// second copy of the highlight-query pin, is gone):
/// `Dedicated` targets the grammar's comment/string node kinds directly
/// (the pinned highlight query captures no such face); `Inactive` is
/// Markdown's documented exception (no comment or string node kinds
/// at all — the filter stays inactive, hits are kept);
/// `FromHighlight` uses the row's own highlight query (most pinned
/// highlight queries already capture the `comment`/`string*` faces).
fn token_class_query_for(lang: LanguageId) -> Option<&'static str> {
    match spec(lang).token_class {
        TokenClass::FromHighlight => spec(lang).highlight_query,
        TokenClass::Dedicated(query) => Some(query),
        TokenClass::Inactive => None, // documented exception
    }
}

/// The comment/string byte ranges in `source` for `lang`, sorted and
/// merged (overlapping ranges collapse into one). Empty for plain text,
/// an unparseable source, or a file with no comment/string tokens.
/// Runs on the calling thread (a search worker); the parser and query
/// cache are thread-local so they are cheap to reuse across files.
pub fn comment_string_ranges(lang: LanguageId, source: &str) -> Vec<Range<usize>> {
    if lang == LanguageId::Plain {
        return Vec::new();
    }
    let Some(query_str) = token_class_query_for(lang) else {
        return Vec::new();
    };
    let Some(language) = language_for(lang) else {
        return Vec::new();
    };

    TL.with(|tl| {
        let mut tl = tl.borrow_mut();
        if tl.parser.set_language(&language).is_err() {
            return Vec::new();
        }
        let tree = match tl.parser.parse(source, None) {
            Some(t) => t,
            None => return Vec::new(),
        };

        // Build (and cache) the highlight query for this language. The
        // query is a superset of the faces the highlight pipeline
        // `configure`s; we keep only the comment/string captures below.
        let cached = tl
            .queries
            .entry(lang)
            .or_insert_with(|| Query::new(&language, query_str).ok());
        let query = match cached.as_ref() {
            Some(q) => q,
            None => return Vec::new(),
        };

        let bytes = source.as_bytes();
        let mut out: Vec<Range<usize>> = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), bytes);
        while let Some(m) = matches.next() {
            for cap in m.captures {
                let name = query.capture_names()[cap.index as usize];
                if name == "comment" || name.starts_with("string") {
                    let node = cap.node;
                    out.push(node.start_byte()..node.end_byte());
                }
            }
        }
        out.sort_by_key(|r| (r.start, r.end));
        let mut merged: Vec<Range<usize>> = Vec::new();
        for r in out {
            match merged.last_mut() {
                Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
                _ => merged.push(r),
            }
        }
        merged
    })
}

thread_local! {
    static TL: RefCell<ThreadLocal> = RefCell::new(ThreadLocal::new());
}

struct ThreadLocal {
    parser: Parser,
    queries: HashMap<LanguageId, Option<Query>>,
}

impl ThreadLocal {
    fn new() -> Self {
        Self {
            parser: Parser::new(),
            queries: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// True when `pos` falls inside any of `ranges`.
    fn in_ranges(ranges: &[Range<usize>], pos: usize) -> bool {
        ranges.iter().any(|r| r.contains(&pos))
    }

    #[test]
    fn rust_line_comment_range_covers_comment_text() {
        let src = "fn main() {\n    // a target here\n    let x = 1;\n}\n";
        let ranges = comment_string_ranges(LanguageId::Rust, src);
        assert!(!ranges.is_empty(), "rust line comment must produce a range");
        let pos = src.find("target").unwrap();
        assert!(in_ranges(&ranges, pos), "`target` in the comment must be in a range: {ranges:?}");
        // The code identifier is NOT in any range.
        let code_pos = src.find("let").unwrap();
        assert!(!in_ranges(&ranges, code_pos));
    }

    #[test]
    fn rust_string_range_covers_string_content() {
        let src = "fn main() {\n    let s = \"the target\";\n    println!(\"{s}\");\n}\n";
        let ranges = comment_string_ranges(LanguageId::Rust, src);
        let pos = src.find("the target").unwrap();
        assert!(in_ranges(&ranges, pos), "string content must be in a range: {ranges:?}");
    }

    #[test]
    fn rust_block_comment_range_covers_block() {
        let src = "/*\n   target inside a block comment\n*/\nfn main() {}\n";
        let ranges = comment_string_ranges(LanguageId::Rust, src);
        let pos = src.find("target").unwrap();
        assert!(in_ranges(&ranges, pos), "block comment must be in a range: {ranges:?}");
    }

    #[test]
    fn rust_code_identifier_outside_all_ranges() {
        let src = "struct Point { x: i32 }\nfn main() { let point = Point { x: 1 }; }\n";
        let ranges = comment_string_ranges(LanguageId::Rust, src);
        for word in ["Point", "main", "point", "let"] {
            let pos = src.find(word).unwrap();
            assert!(!in_ranges(&ranges, pos), "`{word}` must not be in a range: {ranges:?}");
        }
    }

    #[test]
    fn plain_text_has_no_ranges() {
        // The documented fallback: plain files contribute no ranges, so
        // their comment/string-looking hits are kept by the filter.
        assert!(comment_string_ranges(LanguageId::Plain, "// target\n\"target\"\n").is_empty());
    }

    #[test]
    fn python_comment_and_string_ranges() {
        let src = "# target in comment\ndef f():\n    return \"target in string\"\n";
        let ranges = comment_string_ranges(LanguageId::Python, src);
        let comment_pos = src.find("target in comment").unwrap();
        let string_pos = src.find("target in string").unwrap();
        assert!(in_ranges(&ranges, comment_pos), "python comment: {ranges:?}");
        assert!(in_ranges(&ranges, string_pos), "python string: {ranges:?}");
        let code_pos = src.find("def").unwrap();
        assert!(!in_ranges(&ranges, code_pos));
    }

    #[test]
    fn ranges_are_sorted_and_merged() {
        // Two overlapping comments in one file must yield merged ranges
        // (a later range never starts before an earlier range ends).
        let src = "// first target\n// second target\nfn f() {}\n// third target\n";
        let ranges = comment_string_ranges(LanguageId::Rust, src);
        for w in 1..ranges.len() {
            assert!(
                ranges[w].start >= ranges[w - 1].end,
                "ranges must be sorted and non-overlapping: {ranges:?}"
            );
        }
    }

    /// Token-filter honesty per grammar (review finding: only Rust and
    /// Python were proven; a query without `comment`/`string*` captures
    /// silently yields no filtering): every grammar with comment or
    /// string node kinds yields ranges covering a marker word placed in
    /// a comment and/or string. Json has no comments by language spec
    /// (string marker only); Markdown is the documented exception (its
    /// grammar has no comment or string node kinds — the filter is
    /// inactive there).
    #[test]
    fn token_filter_ranges_per_grammar() {
        // (lang, source, marker words that must fall inside a range).
        let cases: &[(LanguageId, &str, &[&str])] = &[
            (
                LanguageId::Rust,
                "fn f() {\n// cmtarget here\nlet s = \"strtarget\";\n}\n",
                &["cmtarget", "strtarget"],
            ),
            (
                LanguageId::TypeScript,
                "function f() {\n// cmtarget here\nlet s = \"strtarget\";\nlet t = `tpl strtarget2`;\n}\n",
                &["cmtarget", "strtarget", "strtarget2"],
            ),
            (
                LanguageId::Tsx,
                "function f() {\n// cmtarget here\nlet s = \"strtarget\";\nlet t = `tpl strtarget2`;\n}\n",
                &["cmtarget", "strtarget", "strtarget2"],
            ),
            (
                LanguageId::JavaScript,
                "function f() {\n// cmtarget here\nlet s = \"strtarget\";\n}\n",
                &["cmtarget", "strtarget"],
            ),
            (
                LanguageId::Python,
                "# cmtarget here\ndef f():\n    s = \"strtarget\"\n",
                &["cmtarget", "strtarget"],
            ),
            (
                LanguageId::Go,
                "package p\n// cmtarget here\nvar s = \"strtarget\"\n",
                &["cmtarget", "strtarget"],
            ),
            (
                LanguageId::C,
                "// cmtarget here\nconst char *s = \"strtarget\";\n",
                &["cmtarget", "strtarget"],
            ),
            (
                LanguageId::Cpp,
                "// cmtarget here\nconst char* s = \"strtarget\";\nchar c = 'x';\nint f() {}\n",
                &["cmtarget", "strtarget", "x"],
            ),
            (
                LanguageId::Toml,
                "# cmtarget here\ns = \"strtarget\"\n",
                &["cmtarget", "strtarget"],
            ),
            (
                LanguageId::Json,
                "{\n  \"key\": \"strtarget\"\n}\n",
                &["strtarget"], // JSON has no comments (language spec)
            ),
            (
                LanguageId::Yaml,
                "# cmtarget here\ns: \"strtarget\"\n",
                &["cmtarget", "strtarget"],
            ),
            (
                LanguageId::Bash,
                "# cmtarget here\ns=\"strtarget\"\n",
                &["cmtarget", "strtarget"],
            ),
        ];
        for (lang, src, markers) in cases {
            let ranges = comment_string_ranges(*lang, src);
            assert!(!ranges.is_empty(), "{lang:?}: ranges must be non-empty");
            for m in *markers {
                let pos = src.find(m).unwrap_or_else(|| panic!("marker `{m}` absent from {src:?}"));
                assert!(
                    ranges.iter().any(|r| r.contains(&pos)),
                    "{lang:?}: `{m}` at byte {pos} must be inside a token range: {ranges:?} (src {src:?})"
                );
            }
        }
        // Documented exception: the Markdown grammar has no comment or
        // string node kinds, so the filter stays inactive (its ranges
        // remain empty and its hits are kept — the fallback behavior).
        assert!(
            comment_string_ranges(LanguageId::Markdown, "# cmtarget\n\nstrtarget\n").is_empty(),
            "markdown has no comment/string node kinds: ranges must stay empty"
        );
    }
}

