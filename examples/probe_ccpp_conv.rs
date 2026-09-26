//! Throwaway probe (plan 017, issue 04 — C/C++ quoted #include):
//! dump the pinned tree-sitter-c / tree-sitter-cpp grammar's parse of the
//! include + using / namespace-alias forms so the convention wiring cites
//! the grammar, not memory. Prints once and returns — bounded by
//! construction (no loops, no blocking reads).

use redline_syntax::registry::LanguageId;

fn dump(lang: LanguageId, source: &str, kinds: &[&str]) {
    let grammar = redline_syntax::language::spec(lang).grammar.unwrap()();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).unwrap();
    let tree = parser.parse(source.as_bytes(), None).unwrap();
    let root = tree.root_node();
    println!("== {lang:?} ==");
    println!("has_error = {}", root.has_error());
    println!("SEXP: {}", root.to_sexp());
    let bytes = source.as_bytes();
    println!("--- top-level nodes of interest ---");
    for i in 0..root.child_count() {
        let c = root.child(i).unwrap();
        if !kinds.contains(&c.kind()) {
            continue;
        }
        println!(
            "{} [{}..{}] {:?} named_children:",
            c.kind(),
            c.start_byte(),
            c.end_byte(),
            c.utf8_text(bytes).unwrap()
        );
        for j in 0..c.child_count() {
            let cc = c.child(j).unwrap();
            println!(
                "    {} [{}..{}] named={} {:?}",
                cc.kind(),
                cc.start_byte(),
                cc.end_byte(),
                cc.is_named(),
                cc.utf8_text(bytes).unwrap()
            );
            // Recurse one level (the string_literal's string_content).
            for k in 0..cc.child_count() {
                let ccc = cc.child(k).unwrap();
                println!(
                    "        {} [{}..{}] {:?}",
                    ccc.kind(),
                    ccc.start_byte(),
                    ccc.end_byte(),
                    ccc.utf8_text(bytes).unwrap()
                );
            }
        }
    }
}

fn main() {
    dump(
        LanguageId::C,
        "#include \"sub/thing.h\"\n#include <stdio.h>\n#include \"gen/missing_xyz.h\"\nint main(void) { return 0; }\n",
        &["preproc_include"],
    );
    dump(
        LanguageId::C,
        "#include \"x.h\"\n#include \"elsewhere/x.h\"\n",
        &["preproc_include"],
    );
    dump(
        LanguageId::Cpp,
        "#include <vector>\n#include \"local/a/b.h\"\n#include \"x.h\"\nnamespace my = myns;\nusing namespace foo;\nusing Foo = Bar;\n",
        &["preproc_include", "namespace_alias_definition", "using_declaration"],
    );
}
