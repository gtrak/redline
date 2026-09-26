//! STEP 1 probe (issue-language-aware-symbols): dump what the pinned
//! tree-sitter-clojure (0.1.0) and tree-sitter-scheme (0.24.7) grammars
//! produce for namespaced / hyphenated symbols, plus the JS `$` and Ruby
//! `?`/`!` suffix cases the audit table flags.
//!
//! Run with `--nocapture` to read the dumps; the assertions pin the
//! grammar-shape facts the per-language word rule relies on.

fn dump(lang: tree_sitter::Language, name: &str, src: &str) {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();
    let tree = parser.parse(src.as_bytes(), None).unwrap();
    let root = tree.root_node();
    let mut out = String::new();
    let mut stack = vec![(root, 0)];
    while let Some((n, d)) = stack.pop() {
        let text: String = n
            .utf8_text(src.as_bytes())
            .map(|t| t.chars().take(28).collect())
            .unwrap_or_else(|_| "<non-utf8>".to_string());
        out.push_str(&format!(
            "{}{} [{}..{}) `{}`{}\n",
            "  ".repeat(d),
            n.kind(),
            n.start_byte(),
            n.end_byte(),
            text,
            if n.has_error() { " ERROR" } else { "" }
        ));
        for i in (0..n.child_count()).rev() {
            if let Some(c) = n.child(i) {
                stack.push((c, d + 1));
            }
        }
    }
    println!("=== {name} ===\n{out}sexp: {}\n", root.to_sexp());
}

#[test]
fn probe_clojure_fixture() {
    dump(
        tree_sitter::Language::from(tree_sitter_clojure::LANGUAGE),
        "CLOJURE fixture",
        r#"
(ns app.core
  (:require [some.ns :as jwks]
            [other.ns :refer [helper-x]]))

(defn handler [token]
  (jwks/fetch-issuer-info :jwks/issuer-url)
  (println ::jwks/local)
  (some.ns/things))
"#,
    );

    // The report's exact case: one line, the namespaced var call.
    dump(
        tree_sitter::Language::from(tree_sitter_clojure::LANGUAGE),
        "CLOJURE line",
        "(jwks/fetch-issuer-info :jwks/issuer-url)\n",
    );

    // The auto-resolve keyword form.
    dump(
        tree_sitter::Language::from(tree_sitter_clojure::LANGUAGE),
        "CLOJURE ::kw",
        "(println ::jwks/local)\n",
    );
}

#[test]
fn probe_scheme_fixture() {
    dump(
        tree_sitter::Language::from(tree_sitter_scheme::LANGUAGE),
        "SCHEME",
        "(define (fetch-issuer-info x)\n  (+ x 1))\n(foo bar-baz)\n",
    );
}

#[test]
fn probe_js_dollar_ruby_suffix() {
    dump(
        tree_sitter::Language::from(tree_sitter_javascript::LANGUAGE),
        "JS $",
        "const $foo = bar$; function $bar() {}\n",
    );
    dump(
        tree_sitter::Language::from(tree_sitter_ruby::LANGUAGE),
        "RUBY ?/!",
        "def save!\nend\ndef empty?\nend\nx = 1 ? 2 : 3\n",
    );
    dump(
        tree_sitter::Language::from(tree_sitter_toml_ng::LANGUAGE),
        "TOML -",
        "[a-b]\nkey-x = 1\n",
    );
}

#[test]
fn probe_scheme_slash_dot() {
    dump(
        tree_sitter::Language::from(tree_sitter_scheme::LANGUAGE),
        "SCHEME / . : '",
        "(a/b c.d :e 'f)\n",
    );
}

/// The quote is a SEPARATE grammar node in both lisp families (the
/// word-rule decision: `'` stays a boundary, never a symbol constituent):
/// tree-sitter-clojure wraps it in a `quoting_lit` (children `'` + a BARE
/// `sym_lit`), tree-sitter-scheme in a `quote` (children `'` + a BARE
/// `symbol`) — the symbol nodes themselves carry no quote char.
#[test]
fn probe_quote_is_a_separate_node() {
    let clj = tree_sitter::Language::from(tree_sitter_clojure::LANGUAGE);
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&clj).unwrap();
    let tree = parser.parse(b"(foo 'bar)\n", None).unwrap();
    let root = tree.root_node();
    let listing = root.child(0).expect("the list form");
    let mut kinds = Vec::new();
    let mut i = 0;
    while let Some(c) = listing.child(i) {
        i += 1;
        kinds.push(c.kind());
    }
    // `(`, `sym_lit` foo, `quoting_lit` 'bar, `)` — the quote is its own
    // node, and the symbol inside it is the bare `bar`.
    assert_eq!(
        kinds,
        vec!["(", "sym_lit", "quoting_lit", ")"],
        "the quote form is a separate node: {kinds:?}"
    );
    let quoting = listing.child(2).expect("the quoting_lit");
    let qkinds: Vec<&str> = (0..quoting.child_count())
        .filter_map(|i| quoting.child(i).map(|c| c.kind()))
        .collect();
    assert_eq!(qkinds, vec!["'", "sym_lit"], "`'` leaf + bare symbol: {qkinds:?}");
    let sym = quoting.child(1).expect("the bare symbol");
    assert_eq!(
        sym.utf8_text(b"(foo 'bar)\n").unwrap(),
        "bar",
        "the quoted symbol carries no quote char"
    );

    let sch = tree_sitter::Language::from(tree_sitter_scheme::LANGUAGE);
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&sch).unwrap();
    let tree = parser.parse(b"(define x 'y)\n", None).unwrap();
    let listing = tree.root_node().child(0).expect("the define form");
    let mut i = 0;
    let quote = loop {
        let Some(c) = listing.child(i) else { break None };
        i += 1;
        if c.kind() == "quote" {
            break Some(c);
        }
    };
    let quote = quote.expect("scheme `quote` node");
    let qkinds: Vec<&str> = (0..quote.child_count())
        .filter_map(|i| quote.child(i).map(|c| c.kind()))
        .collect();
    assert_eq!(qkinds, vec!["'", "symbol"], "`'` leaf + bare symbol: {qkinds:?}");
}
