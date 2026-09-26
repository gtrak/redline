//! Throwaway probe (plan 017, issue 03 — Java conventions): dump the
//! pinned tree-sitter-java grammar's parse of the import forms so the
//! convention wiring cites the grammar, not memory. Prints once and
//! returns — bounded by construction (no loops, no blocking reads).

use redline_syntax::registry::LanguageId;

const SOURCE: &str = "package com.example;\nimport a.b.C;\nimport a.b.*;\nimport static a.b.D.m;\nclass A {\n    int c;\n    void f() {\n        C x;\n        int y = a.b.C.valueOf(1);\n    }\n}\n";

fn main() {
    let grammar = redline_syntax::language::spec(LanguageId::Java)
        .grammar
        .unwrap()();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).unwrap();
    let tree = parser.parse(SOURCE.as_bytes(), None).unwrap();
    let root = tree.root_node();
    println!("has_error = {}", root.has_error());
    // (1) The structural sexp (one line).
    println!("SEXP: {}", root.to_sexp());
    // (2) The import_declaration / package_declaration children, exactly
    // as the extractor sees them (kind + text, plus the named children).
    let bytes = SOURCE.as_bytes();
    println!("--- top-level import/package nodes ---");
    for i in 0..root.child_count() {
        let c = root.child(i).unwrap();
        if c.kind() != "import_declaration" && c.kind() != "package_declaration" {
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
            if !cc.is_named() {
                continue;
            }
            println!(
                "    {} [{}..{}] {:?}",
                cc.kind(),
                cc.start_byte(),
                cc.end_byte(),
                cc.utf8_text(bytes).unwrap()
            );
        }
    }
    // (3) The reference shapes: the bare `C` at the use site and the
    // explicitly-qualified `a.b.C` (node kinds the M-. extraction sees).
    println!("--- type_identifier / scoped nodes at reference sites ---");
    print_refs(root, SOURCE);
}

fn print_refs(node: tree_sitter::Node, source: &str) {
    let bytes = source.as_bytes();
    for i in 0..node.child_count() {
        let c = node.child(i).unwrap();
        if node.start_byte() > 60
            && matches!(
                c.kind(),
                "type_identifier" | "scoped_type_identifier" | "scoped_identifier"
            )
        {
            println!(
                "{} [{}..{}] {:?} (parent {})",
                c.kind(),
                c.start_byte(),
                c.end_byte(),
                c.utf8_text(bytes).unwrap(),
                node.kind()
            );
        }
        if c.child_count() > 0 {
            print_refs(c, source);
        }
    }
}

#[cfg(test)]
mod t {
    use super::*;
    #[test]
    fn dump_all_children_including_anonymous() {
        let grammar = redline_syntax::language::spec(LanguageId::Java).grammar.unwrap()();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar).unwrap();
        let src = "import static a.b.D.m;\n";
        let tree = parser.parse(src.as_bytes(), None).unwrap();
        let root = tree.root_node();
        let import = root.child(0).unwrap();
        println!("import kind {} {:?} children:", import.kind(), import.utf8_text(src.as_bytes()).unwrap());
        for j in 0..import.child_count() {
            let cc = import.child(j).unwrap();
            println!("  [{}] named={} {:?}", j, cc.is_named(), cc.utf8_text(src.as_bytes()).unwrap());
        }
        // Also: a `C x;` local declaration's type child field name.
        let src2 = "class A { void f() { C x; } }\n";
        let tree2 = parser.parse(src2.as_bytes(), None).unwrap();
        let r2 = tree2.root_node();
        let mut stack = vec![r2];
        while let Some(n) = stack.pop() {
            if n.kind() == "local_variable_declaration" {
                println!("local_var {:?} type child field_name={:?}", n.utf8_text(src2.as_bytes()).unwrap(), n.child_by_field_name("type").map(|t| t.utf8_text(src2.as_bytes()).unwrap()));
            }
            for i in 0..n.child_count() {
                if let Some(c) = n.child(i) {
                    stack.push(c);
                }
            }
        }
    }
}
