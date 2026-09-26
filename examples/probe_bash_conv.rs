//! Throwaway probe (plan 017, issue 07 — Bash `source` / `.`): dump the
//! pinned tree-sitter-bash 0.25.1 grammar's parse of the source forms
//! (relative / absolute / quoted / bare / dynamic / tilde / glob /
//! extra-args / nested) plus the function-definition shapes and the
//! `.`-in-other-positions traps, so the convention wiring cites the
//! grammar, not memory. Prints once and returns — bounded by
//! construction (one fixed-depth pass per fixture, no loops over
//! input, no blocking reads).

use redline_syntax::registry::LanguageId;

fn parse(source: &str) -> tree_sitter::Tree {
    let grammar = redline_syntax::language::spec(LanguageId::Bash).grammar.unwrap()();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).unwrap();
    parser.parse(source.as_bytes(), None).unwrap()
}

/// Recursively dump every `command` node: its `name` field (kind +
/// text) and every `argument` field (kind + child kinds + text).
fn dump_commands(tag: &str, source: &str) {
    let tree = parse(source);
    let root = tree.root_node();
    let bytes = source.as_bytes();
    println!("== {tag} ==");
    println!("  has_error = {}", root.has_error());
    println!("  SEXP: {}", root.to_sexp());
    dump_commands_at(root, bytes);
}

fn dump_commands_at(node: tree_sitter::Node, bytes: &[u8]) {
    if node.kind() == "command" {
        let name = node
            .child_by_field_name("name")
            .map(|n| {
                format!(
                    "{} {:?}",
                    n.kind(),
                    n.utf8_text(bytes).unwrap_or("<err>")
                )
            })
            .unwrap_or_else(|| "<absent>".to_string());
        println!("  command {:?}  name={}", node.utf8_text(bytes).unwrap(), name);
        // The command's arguments are its `argument`-field children.
        let mut i = 0;
        while let Some(c) = node.child(i) {
            i += 1;
            let field = field_of(c);
            if field.as_deref() != Some("argument") {
                continue;
            }
            let kids: Vec<String> = (0..c.child_count())
                .filter_map(|k| {
                    let ch = c.child(k)?;
                    Some(format!("{} {:?}", ch.kind(), ch.utf8_text(bytes).unwrap_or("<err>")))
                })
                .collect();
            println!(
                "      arg: {} {:?} children={:?}",
                c.kind(),
                c.utf8_text(bytes).unwrap_or("<err>"),
                kids
            );
        }
    }
    let mut i = 0;
    while let Some(c) = node.child(i) {
        i += 1;
        dump_commands_at(c, bytes);
    }
}

/// The grammar field name of child `c` within its parent.
fn field_of(c: tree_sitter::Node) -> Option<String> {
    let parent = c.parent()?;
    let mut i = 0;
    while let Some(ch) = parent.child(i) {
        if ch.byte_range() == c.byte_range() && ch.kind_id() == c.kind_id() {
            return parent.field_name_for_child(i as u32).map(str::to_string);
        }
        i += 1;
    }
    None
}

/// Recursively dump every `function_definition`: its `name` field
/// (kind + text) and the body kind.
fn dump_functions(tag: &str, source: &str) {
    let tree = parse(source);
    let root = tree.root_node();
    let bytes = source.as_bytes();
    println!("== {tag} ==");
    println!("  has_error = {}", root.has_error());
    println!("  SEXP: {}", root.to_sexp());
    dump_functions_at(root, bytes);
}

fn dump_functions_at(node: tree_sitter::Node, bytes: &[u8]) {
    if node.kind() == "function_definition" {
        let name = node
            .child_by_field_name("name")
            .map(|n| format!("{} {:?}", n.kind(), n.utf8_text(bytes).unwrap_or("<err>")))
            .unwrap_or_else(|| "<absent>".to_string());
        let body = node
            .child_by_field_name("body")
            .map(|n| n.kind())
            .unwrap_or("<absent>");
        println!(
            "  function_definition {:?}  name={}  body={}",
            node.utf8_text(bytes).unwrap(),
            name,
            body
        );
    }
    let mut i = 0;
    while let Some(c) = node.child(i) {
        i += 1;
        dump_functions_at(c, bytes);
    }
}

fn main() {
    dump_commands("the three audit forms", "source ./sub/helper.sh\n. ./other.sh\nsource helper\n");
    dump_commands("relative without ./", "source sub/helper.sh\n");
    dot_commands("dot relative without ./", ". sub/other.sh\n");
    dump_commands("absolute path", "source /abs/path/helper.sh\n");
    dump_commands("quoted string arg", "source \"sub/helper.sh\"\n");
    dump_commands(
        "dynamic args (variable / expansion)",
        "source $VAR\nsource \"${X:-./sub/helper.sh}\"\nsource $(pwd)/x.sh\n",
    );
    dump_commands("tilde", "source ~/bin/helper.sh\n");
    dump_commands("glob", "source ./sub/*.sh\n");
    dump_commands("extra args after the path", "source ./a.sh --flag val\n");
    dump_commands("bare dot command", ". helper\n");
    dump_commands(
        "nested (function body + if branch)",
        "wrap_fn() { source ./nested/x.sh; }\nif true; then . ./y.sh; fi\n",
    );
    dot_commands("dot in other positions", "echo .\nx=1.5\n./run.sh\necho a.b\n");
    dump_functions(
        "function definitions",
        "helper_fn() { :; }\nfunction wrapped_fn { :; }\nfunction both_fn() { :; }\n",
    );
}

/// Same as dump_commands but the fixtures are `.`-form (kept as its own
/// call so the fixtures read naturally).
fn dot_commands(tag: &str, source: &str) {
    dump_commands(tag, source);
}
