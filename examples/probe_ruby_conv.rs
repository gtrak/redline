//! Throwaway probe (plan 017, issue 06 — Ruby require / require_relative /
//! autoload): dump the pinned tree-sitter-ruby 0.23.1 grammar's parse of
//! the import method-call forms (bare vs receiver) and the constant
//! definition shapes, so the convention wiring cites the grammar, not
//! memory. Prints once and returns — bounded by construction (one
//! fixed-depth pass per fixture, no loops over input, no blocking reads).

use redline_syntax::registry::LanguageId;

fn parse(source: &str) -> tree_sitter::Tree {
    let grammar = redline_syntax::language::spec(LanguageId::Ruby).grammar.unwrap()();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).unwrap();
    parser.parse(source.as_bytes(), None).unwrap()
}

/// Dump the `call` nodes of `source`: the method name, the presence of a
/// `receiver` field (bare vs `self.require` / `foo.require_relative`), and
/// the argument node kinds.
fn dump_calls(tag: &str, source: &str) {
    let tree = parse(source);
    let root = tree.root_node();
    let bytes = source.as_bytes();
    println!("== {tag} ==");
    println!("  has_error = {}", root.has_error());
    println!("  SEXP: {}", root.to_sexp());
    let mut i = 0;
    while let Some(n) = root.child(i) {
        i += 1;
        if n.kind() != "call" {
            continue;
        }
        let method = n
            .child_by_field_name("method")
            .and_then(|m| m.utf8_text(bytes).ok())
            .unwrap_or("<none>");
        let receiver = n
            .child_by_field_name("receiver")
            .map(|r| format!("{} {:?}", r.kind(), r.utf8_text(bytes).unwrap()))
            .unwrap_or_else(|| "<absent>".to_string());
        println!(
            "  call {:?}  method={}  receiver={}",
            n.utf8_text(bytes).unwrap(),
            method,
            receiver
        );
        if let Some(args) = n.child_by_field_name("arguments") {
            let mut k = 0;
            while let Some(a) = args.child(k) {
                k += 1;
                println!(
                    "      arg: {} {:?}",
                    a.kind(),
                    a.utf8_text(bytes).unwrap()
                );
            }
        }
    }
}

/// Dump the top-level definition node's name field (class / module /
/// assignment) — the constant-definition shapes the local-constant
/// exclusion reads.
fn dump_defs(tag: &str, source: &str) {
    let tree = parse(source);
    let root = tree.root_node();
    let bytes = source.as_bytes();
    println!("== {tag} ==");
    println!("  has_error = {}", root.has_error());
    println!("  SEXP: {}", root.to_sexp());
    let mut i = 0;
    while let Some(n) = root.child(i) {
        i += 1;
        match n.kind() {
            "class" | "module" => {
                if let Some(name) = n.child_by_field_name("name") {
                    println!(
                        "  {}: name field -> {} {:?}",
                        n.kind(),
                        name.kind(),
                        name.utf8_text(bytes).unwrap()
                    );
                }
            }
            "assignment" => {
                if let Some(left) = n.child_by_field_name("left") {
                    println!(
                        "  assignment: left field -> {} {:?}",
                        left.kind(),
                        left.utf8_text(bytes).unwrap()
                    );
                }
            }
            _ => {}
        }
    }
}

fn main() {
    dump_calls(
        "bare forms",
        "require 'a/b'\nrequire_relative 'c'\nautoload :D, 'd/e'\n",
    );
    dump_calls(
        "receiver forms (must NOT count)",
        "self.require 'x'\nfoo.require_relative 'y'\nOther.autoload :Z, 'z/w'\n",
    );
    dump_defs("class", "class Thing\nend\n");
    dump_defs("class-qualified (scope_resolution name)", "class Thing::Inner\nend\n");
    dump_defs("module", "module Thing\nend\n");
    dump_defs("constant assignment", "Thing = 1\n");
    dump_defs("local-var assignment (identifier, not a constant)", "x = 5\n");
}
