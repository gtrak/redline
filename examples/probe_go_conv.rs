//! Throwaway probe (plan 017, issue 05 — Go in-module import resolution):
//! dump the pinned tree-sitter-go grammar's parse of the import forms
//! (plain / aliased / dot / blank, grouped and single) so the convention
//! wiring cites the grammar, not memory. Prints once and returns — bounded
//! by construction (no loops, no blocking reads).

use redline_syntax::registry::LanguageId;

fn dump(source: &str, kinds: &[&str]) {
    let grammar = redline_syntax::language::spec(LanguageId::Go).grammar.unwrap()();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).unwrap();
    let tree = parser.parse(source.as_bytes(), None).unwrap();
    let root = tree.root_node();
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
            "{} [{}..{}] {:?}",
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
        "package main\n\nimport (\n\t. \"a/b\"\n\tx \"a/c\"\n\t_ \"a/d\"\n\t\"a/e\"\n)\n\nfunc main() {}\n",
        &["import_declaration"],
    );
    dump("package main\n\nimport \"github.com/x/y/sub\"\nimport alias2 \"github.com/x/y/sub2\"\n\nfunc main() {}\n", &[
        "import_declaration",
    ]);
    dump("package main\n\nimport \"C\"\n\nfunc main() {}\n", &["import_declaration"]);
}
