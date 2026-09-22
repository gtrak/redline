//! Background symbol indexer (issue 05): a per-session, in-memory per-file
//! symbol table (name, kind, line, byte range) built over the whole project
//! with a rayon-parallel tree-sitter parse, refreshed incrementally on
//! watcher events (only changed files reparse — a full rebuild happens only
//! on project switch or a manual command).
//!
//! 010-01 (plan 010 Shape A, rung 1): the same pass also carries the Rust
//! per-file tables (struct fields, impl methods with their impl kind —
//! `RustTables`), stored per file plus a name-keyed field map for the
//! cross-file self-receiver field lookup the M-. consumption queries
//! synchronously. 010-03 (rung 3) extends the same per-file tables with
//! the local binding map (written-down types only); bindings stay
//! per-file (their type then resolves through the existing field / impl
//! maps, same-file or cross-file alike).
//!
//! Layering (plan): `nav/` is plain Rust — tree-sitter + rayon + tokio are
//! allowed; there is no iocraft. The indexer never holds a lock the UI needs:
//! the heavy parse runs on background threads and hands its result back to the
//! app through the [`IndexBus`] (a `watch` channel, latest-value-wins, mirroring
//! the project-change bus). The app installs the newest result into its store;
//! the UI reads only that installed snapshot.

mod builder;
mod progress;
mod symbol_index;

pub use builder::{assemble_index, build_index, enclosing_symbol, refresh_in_place};
pub use progress::{IndexBus, IndexEvent, IndexProgress};
pub use symbol_index::{Location, SymbolIndex, TraitImplLocation};
// Re-exported so every existing `nav::index::X` path keeps working, even
// though the bin target's non-test build never references these names
// through this module (their consumers are the cfg(test) targets).
#[allow(unused_imports)]
pub use builder::extract_file;
#[allow(unused_imports)]
pub use progress::PROGRESS_STEP;
#[allow(unused_imports)]
pub use symbol_index::{FieldLocation, TraitImpl};

// Re-exported so the public index/xref API can name the symbol type.
pub use redline_syntax::queries::Symbol;
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use redline_syntax::queries::extract_all;
    use std::fs;

    fn make_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        fs::write(
            root.join("src/main.rs"),
            "mod lib {\n    pub fn target() {}\n}\nfn main() { lib::target(); }\n",
        )
        .unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn target() {}\npub fn other() {}\n").unwrap();
        fs::write(root.join("src/notes.md"), "# Title\n\n## Sub\n\ntext\n").unwrap();
        fs::write(root.join("src/data.json"), "{\n  \"k1\": 1,\n  \"k2\": {\"k3\": 2}\n}\n").unwrap();
        fs::write(root.join("src/readme.txt"), "just some plain text\n").unwrap();
        dir
    }

    fn rel_files(root: &Path) -> Vec<String> {
        let list = crate::model::files::FileList::build(root).unwrap();
        list.files
    }

    #[test]
    fn parallel_index_finds_cross_file_definitions() {
        let dir = make_project();
        let root = dir.path();
        let files = rel_files(root);
        let index = build_index(root, &files, None);

        // `target` is defined in two files (main.rs's mod lib + lib.rs).
        assert_eq!(index.definition_count("target"), 2, "`target` defined in two files");
        // `other` is defined once (lib.rs).
        assert_eq!(index.definition_count("other"), 1);
        // `main` once (main.rs).
        assert_eq!(index.definition_count("main"), 1);
        // Markdown heading is present.
        assert_eq!(index.definition_count("Title"), 1);
        // JSON keys are NOT symbols (issue-json-yaml-no-symbols).
        assert_eq!(index.definition_count("k1"), 0, "JSON keys must not be symbols");
        assert_eq!(index.definition_count("k3"), 0, "JSON keys must not be symbols");
        // Plain-text (no code grammar) contributes no outline.
        assert!(!index.has("src/readme.txt"), "readme.txt must be plain: no symbols");
        // The TOML key `name` (not the value string) is a symbol.
        assert_eq!(index.definition_count("name"), 1, "TOML key `name` is a symbol");
    }

    #[test]
    fn incremental_refresh_reparses_only_changed_file() {
        let dir = make_project();
        let root = dir.path();
        let files = rel_files(root);
        let mut index = build_index(root, &files, None);
        let baseline_lib = index.outline("src/lib.rs").to_vec();
        let baseline_notes = index.outline("src/notes.md").to_vec();

        // Change only src/lib.rs: rename `other` to `renamed`.
        let path = root.join("src/lib.rs");
        fs::write(&path, "pub fn target() {}\npub fn renamed() {}\n").unwrap();

        let touched = refresh_in_place(root, &[path], &mut index);
        assert_eq!(touched, vec!["src/lib.rs".to_string()], "{touched:?}");

        // lib.rs's outline reflects the change; the other files are untouched.
        let names: Vec<&str> = index.outline("src/lib.rs").iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"renamed"), "{names:?}");
        assert!(!names.contains(&"other"), "{names:?}");
        // `renamed` is now resolvable cross-file; `other` is gone.
        assert_eq!(index.definition_count("renamed"), 1);
        assert_eq!(index.definition_count("other"), 0);
        // The untouched file's outline is byte-for-byte identical.
        assert_eq!(index.outline("src/notes.md"), baseline_notes.as_slice());
        // And lib.rs's own baseline (pre-change) differs from the post-change.
        assert_ne!(index.outline("src/lib.rs"), baseline_lib.as_slice());
    }

    #[test]
    fn refresh_removes_outline_for_deleted_file() {
        let dir = make_project();
        let root = dir.path();
        let files = rel_files(root);
        let mut index = build_index(root, &files, None);
        assert!(index.has("src/notes.md"));
        let path = root.join("src/notes.md");
        fs::remove_file(&path).unwrap();
        refresh_in_place(root, &[path], &mut index);
        assert!(!index.has("src/notes.md"));
        assert_eq!(index.definition_count("Title"), 0);
    }

    #[test]
    fn progress_counter_reaches_total() {
        let dir = make_project();
        let root = dir.path();
        let files = rel_files(root);
        let progress = IndexProgress::new(files.len());
        let _ = build_index(root, &files, Some(&progress));
        assert_eq!(progress.done(), files.len(), "progress must reach total");
        assert!(progress.finished());
    }

    #[test]
    fn enclosing_symbol_picks_innermost() {
        // mod > fn; a cursor inside the function body resolves to the fn,
        // and a cursor inside the mod but outside the fn resolves to the mod.
        let src = "mod outer {\n    fn f() {\n        g()\n    }\n}\n";
        let syms = extract_all(redline_syntax::registry::LanguageId::Rust, src).0;
        // mod outer: line 0..4 ; fn f: line 1..3
        let f_line = 2; // "g()"
        let enc = enclosing_symbol(&syms, f_line).expect("enclosing at line 2");
        assert_eq!(enc.name, "f", "innermost is the function, got {enc:?}");
        // Line 0 is the mod's own line (the fn starts at line 1).
        let enc_mod = enclosing_symbol(&syms, 0).expect("enclosing at line 0");
        assert_eq!(enc_mod.name, "outer");
        // Outside everything.
        assert!(enclosing_symbol(&syms, 99).is_none());
    }

    #[test]
    fn index_bus_latest_value_wins() {
        let bus = IndexBus::new();
        let mut rx = bus.subscribe();
        bus.send(IndexEvent { indexing: true, total: 3, ..Default::default() });
        bus.send(IndexEvent { indexing: false, done: 3, total: 3, ..Default::default() });
        // A receiver sees the newest published value (latest-value-wins).
        assert!(rx.has_changed().unwrap(), "a publish changed the value");
        let got = rx.borrow_and_update().clone();
        assert!(!got.indexing, "latest event wins");
        assert_eq!(got.total, 3);
    }

    #[test]
    fn same_file_duplicate_names_preserved() {
        // Two same-named functions in different mods of one file:
        // `by_name` must keep both (not last-wins).
        let src = "mod a { pub fn target() {} }\nmod b { pub fn target() {} }\n";
        let syms = extract_all(redline_syntax::registry::LanguageId::Rust, src).0;
        // Two `target` functions + two `mod` items = 4 symbols total.
        let targets: Vec<_> = syms.iter().filter(|s| s.name == "target").collect();
        assert_eq!(targets.len(), 2, "two `target` definitions: {syms:?}");
        let mut idx = SymbolIndex::new();
        idx.set_file("dup.rs", syms);
        // `definitions_of` returns both; `definition_count` is 1 (one file).
        let defs = idx.definitions_of("target");
        assert_eq!(defs.len(), 2, "both same-file duplicates preserved: {defs:?}");
        assert_eq!(idx.definition_count("target"), 1, "one file defines `target`");
        // They have different line numbers.
        assert_ne!(defs[0].symbol.line, defs[1].symbol.line);
    }

    // ── 010-01: Rust tables in the index (same pass, cross-file lookup) ──

    #[test]
    fn rust_tables_carry_with_the_index_and_support_cross_file_field_lookup() {
        let dir = make_project();
        let root = dir.path();
        // A struct in one file, an impl of it in another — the exact
        // cross-file shape the M-. self-receiver field lookup resolves.
        std::fs::write(
            root.join("src/point.rs"),
            "pub struct Point { pub x: i32, pub y: i32 }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/ops.rs"),
            "use crate::point::Point;\nimpl Point {\n    pub fn x(&self) -> i32 { self.x }\n}\n",
        )
        .unwrap();
        let files = rel_files(root);
        let index = build_index(root, &files, None);

        // Same-file seam: ops.rs's impl table (inherent, method line 2,
        // impl line 1, method name's start byte 49 — jump-column-pty: the
        // landing's char column comes from this byte).
        let ops = index.tables("src/ops.rs").expect("ops.rs has tables");
        assert_eq!(
            ops.impls.get("Point"),
            Some(&vec![redline_syntax::queries::ImplMethod {
                method: "x".into(),
                line: 2,
                impl_line: 1,
                start_byte: 49,
                kind: redline_syntax::queries::ImplKind::Inherent,
            }]),
            "ops.rs impl table: {ops:?}"
        );
        // point.rs's own struct fields (same file, zero impls there).
        let point = index.tables("src/point.rs").expect("point.rs has tables");
        assert!(point.impls.is_empty(), "point.rs has no impls");
        assert_eq!(
            point.fields.get("Point"),
            Some(&vec![
                redline_syntax::queries::StructField { field: "x".into(), line: 0, start_byte: 23 },
                redline_syntax::queries::StructField { field: "y".into(), line: 0, start_byte: 35 },
            ])
        );
        // Cross-file seam: the field `x` of struct `Point` lives in point.rs
        // even though the lookup would be issued from ops.rs's impl — and
        // the entry carries the name's start byte (23), not just the line.
        assert_eq!(
            index.field_locations("Point", "x"),
            vec![FieldLocation {
                file: "src/point.rs".into(),
                line: 0,
                start_byte: 23,
            }],
            "cross-file field location"
        );
        assert!(index.field_locations("Point", "missing").is_empty());
        assert!(index.field_locations("Other", "x").is_empty());
    }

    /// 010-01 review P1 regression: an UNCHANGED-content refresh (touch,
    /// linter rewrite, editor no-save) must keep the file's field-map
    /// contributions — `set_file_tables`' same-check now runs BEFORE the
    /// field-map removal (it previously stripped them and returned early,
    /// silently degrading `self.<field>` M-. until the next real change).
    #[test]
    fn rust_tables_same_content_refresh_keeps_field_locations() {
        let dir = make_project();
        let root = dir.path();
        std::fs::write(root.join("src/point.rs"), "pub struct Point { pub x: i32 }\n").unwrap();
        let files = rel_files(root);
        let mut index = build_index(root, &files, None);
        assert_eq!(
            index.field_locations("Point", "x"),
            vec![FieldLocation { file: "src/point.rs".into(), line: 0, start_byte: 23 }]
        );

        // Reparse the UNCHANGED file through the refresh path.
        let path = root.join("src/point.rs");
        refresh_in_place(root, std::slice::from_ref(&path), &mut index);
        assert_eq!(
            index.field_locations("Point", "x"),
            vec![FieldLocation { file: "src/point.rs".into(), line: 0, start_byte: 23 }],
            "a same-content refresh must not strip the field map"
        );
        // And the tables entry survives for method resolution.
        assert!(index.tables("src/point.rs").is_some());
        // A real content change right after still works (the map was not
        // double-removed or corrupted by the shortcut).
        std::fs::write(&path, "pub struct Point { pub x: i32, pub y: i32 }\n").unwrap();
        refresh_in_place(root, std::slice::from_ref(&path), &mut index);
        assert_eq!(
            index.field_locations("Point", "x"),
            vec![FieldLocation { file: "src/point.rs".into(), line: 0, start_byte: 23 }]
        );
        assert_eq!(
            index.field_locations("Point", "y"),
            vec![FieldLocation { file: "src/point.rs".into(), line: 0, start_byte: 35 }]
        );
    }

    /// 010-03 (010-01 review P1 mirror pin): a file whose ONLY table
    /// content is local bindings (no structs, no impls — empty fields
    /// AND impls) still stores a tables entry (bindings counted as
    /// content in the empty check), and an UNCHANGED-content refresh
    /// (touch / linter rewrite / editor no-save) restores it rather
    /// than dropping it. What this pin discriminates: the CONTENT
    /// COUNTING (pre-010-03 a bindings-only table was `empty` and
    /// never stored) and the entry's restore-on-refresh; it does NOT
    /// discriminate the same-check-before-removal ORDER itself — a
    /// bindings-only file makes no field-map contributions, so the
    /// removal loop below the shortcut is a no-op for it either way.
    /// The order discipline is discriminated by 010-01's field pin
    /// (`rust_tables_same_content_refresh_keeps_field_locations`).
    #[test]
    fn rust_tables_bindings_only_same_content_refresh_keeps_bindings() {
        let dir = make_project();
        let root = dir.path();
        std::fs::write(
            root.join("src/lib.rs"),
            "fn main() {\n    let p: Pt = make();\n    let _ = p.x;\n}\n",
        )
        .unwrap();
        let files = rel_files(root);
        let mut index = build_index(root, &files, None);
        let tables = index
            .tables("src/lib.rs")
            .expect("a bindings-only file stores its tables entry");
        assert!(tables.fields.is_empty() && tables.impls.is_empty());
        assert_eq!(tables.bindings.len(), 1, "{:?}", tables.bindings);
        assert_eq!(tables.bindings[0].binding, "p");
        assert_eq!(tables.bindings[0].type_name, "Pt");
        // P3-B pin (gate of ddd58aa): `name_byte` (the binding's NAME node,
        // distinct from the `let`'s own byte) was captured with zero
        // consumers — a local binding has no jump path that lands on it
        // (matrix: N/A), the record carries it so a future landing path has
        // the byte at the source. Pin the value so the capture is
        // documented and a regression (e.g. capturing the `let` node's byte
        // instead — 16 — instead of the name's 20) is caught here.
        assert_eq!(tables.bindings[0].let_byte, 16, "the `let`'s own start byte");
        assert_eq!(tables.bindings[0].name_byte, 20, "the name node's byte, not the `let`'s");

        // Reparse the UNCHANGED file through the refresh path: the
        // same-content shortcut must restore, not strip.
        let path = root.join("src/lib.rs");
        refresh_in_place(root, std::slice::from_ref(&path), &mut index);
        let tables = index
            .tables("src/lib.rs")
            .expect("the same-content refresh must not strip the bindings");
        assert_eq!(tables.bindings.len(), 1, "{:?}", tables.bindings);

        // A real change right after still replaces cleanly (no
        // double-removal / corruption from the shortcut).
        std::fs::write(
            &path,
            "fn main() {\n    let p: Pt = make();\n    let q: Q = Q;\n    let _ = p.x;\n}\n",
        )
        .unwrap();
        refresh_in_place(root, std::slice::from_ref(&path), &mut index);
        let tables = index.tables("src/lib.rs").expect("refreshed entry");
        let names: Vec<&str> = tables.bindings.iter().map(|b| b.binding.as_str()).collect();
        assert_eq!(names, vec!["p", "q"], "the real change replaced the map: {names:?}");
    }

    #[test]
    fn rust_tables_refresh_and_remove_with_the_file() {
        let dir = make_project();
        let root = dir.path();
        std::fs::write(root.join("src/point.rs"), "pub struct Point { pub x: i32 }\n").unwrap();
        let files = rel_files(root);
        let mut index = build_index(root, &files, None);
        assert_eq!(
            index.field_locations("Point", "x"),
            vec![FieldLocation { file: "src/point.rs".into(), line: 0, start_byte: 23 }]
        );

        // Change the struct: `x` becomes `xx` — the stale location must
        // disappear, the new one appear (the field map stays consistent).
        std::fs::write(root.join("src/point.rs"), "pub struct Point { pub xx: i32 }\n").unwrap();
        let path = root.join("src/point.rs");
        refresh_in_place(root, std::slice::from_ref(&path), &mut index);
        assert!(index.field_locations("Point", "x").is_empty(), "stale field dropped");
        assert_eq!(
            index.field_locations("Point", "xx"),
            vec![FieldLocation { file: "src/point.rs".into(), line: 0, start_byte: 23 }]
        );

        // Delete the file: its tables leave the index with the outline.
        std::fs::remove_file(&path).unwrap();
        refresh_in_place(root, &[path], &mut index);
        assert!(index.tables("src/point.rs").is_none());
        assert!(index.field_locations("Point", "xx").is_empty());
    }

    // ── P3-A (gate of ddd58aa): field_start_byte determinism ──

    /// P3-A: two structs on ONE line declaring a same-named field — the
    /// picker row carries only (file, line) + the field name, so the
    /// `(file, line, name)`-keyed fallback must answer ONE column,
    /// deterministically: the MINIMUM byte (struct A's `x`, the earliest
    /// declaration on the line). Pre-fix this was a `find_map` over
    /// `rust_fields.values()` — per-process-seeded `HashMap` order — so the
    /// same input answered A's byte in some processes and B's in others
    /// (probe-verified: 12 processes split 7/5 across 19 and 47 pre-fix,
    /// 12/12 on 19 post-fix; a single in-process assertion cannot show a
    /// per-process seed, it pins the rule's exact value).
    #[test]
    fn field_start_byte_two_structs_one_line_is_the_minimum_byte() {
        let dir = make_project();
        let root = dir.path();
        std::fs::write(
            root.join("src/duo.rs"),
            "pub struct A { pub x: i32 } pub struct B { pub x: i32 }\n",
        )
        .unwrap();
        let files = rel_files(root);
        let index = build_index(root, &files, None);

        // Both declarations are recorded under their own struct (the maps
        // are per-struct and correct: A's `x` at byte 19, B's at 47).
        assert_eq!(
            index.field_locations("A", "x"),
            vec![FieldLocation { file: "src/duo.rs".into(), line: 0, start_byte: 19 }]
        );
        assert_eq!(
            index.field_locations("B", "x"),
            vec![FieldLocation { file: "src/duo.rs".into(), line: 0, start_byte: 47 }]
        );
        // The picker fallback answers the minimum (earliest) byte — never
        // the other struct's.
        assert_eq!(index.field_start_byte("src/duo.rs", 0, "x"), Some(19));
        // Misses still degrade to None (no invented column upstream).
        assert_eq!(index.field_start_byte("src/duo.rs", 1, "x"), None);
        assert_eq!(index.field_start_byte("src/duo.rs", 0, "nope"), None);
    }

    // ── 010-04: the name-keyed trait map (find-implementations) ──

    #[test]
    fn trait_map_supports_trait_impl_lookup_across_files() {
        let dir = make_project();
        let root = dir.path();
        // Two trait impls under DIFFERENT captured trait texts (the full
        // path as written in one file, the bare name in another — the map
        // is name-keyed, so each spelling is its own key) plus an
        // inherent impl that must NOT appear under any trait key.
        std::fs::write(
            root.join("src/alpha.rs"),
            "struct A;\nimpl std::fmt::Display for A {\n    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, \"A\") }\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/beta.rs"),
            "use std::fmt::Display;\nstruct B;\nimpl Display for B {\n    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, \"B\") }\n}\nimpl B {\n    fn own(&self) {}\n}\n",
        )
        .unwrap();
        let files = rel_files(root);
        let index = build_index(root, &files, None);

        // The full-path spelling: one candidate (alpha.rs's impl header,
        // line 1, self type `A`) — one entry although the impl has one
        // method, and deduped (no multiplication).
        assert_eq!(
            index.trait_impl_locations("std::fmt::Display"),
            vec![TraitImplLocation {
                file: "src/alpha.rs".into(),
                impl_line: 1,
                self_type: "A".into(),
            }]
        );
        // The bare spelling: beta.rs's impl (line 2, self type `B`).
        assert_eq!(
            index.trait_impl_locations("Display"),
            vec![TraitImplLocation {
                file: "src/beta.rs".into(),
                impl_line: 2,
                self_type: "B".into(),
            }]
        );
        // Inherent impls never enter the map.
        assert!(index.trait_impl_locations("B").is_empty());
        assert!(index.trait_impl_locations("Other").is_empty());
    }

    /// 010-04 pin (the 010-01 review P1 discipline on the trait map): an
    /// UNCHANGED-content refresh (touch, linter rewrite, editor no-save)
    /// must keep the file's trait-map contributions — the same-check runs
    /// BEFORE the removal in `set_file_tables`, so a no-op refresh
    /// restores rather than strips (a stripped map would silently degrade
    /// find-implementations until the next real change).
    #[test]
    fn trait_map_same_content_refresh_keeps_trait_locations() {
        let dir = make_project();
        let root = dir.path();
        std::fs::write(
            root.join("src/alpha.rs"),
            "struct A;\nimpl std::fmt::Display for A {\n    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, \"A\") }\n}\n",
        )
        .unwrap();
        let files = rel_files(root);
        let mut index = build_index(root, &files, None);
        let expected = vec![TraitImplLocation {
            file: "src/alpha.rs".into(),
            impl_line: 1,
            self_type: "A".into(),
        }];
        assert_eq!(index.trait_impl_locations("std::fmt::Display"), expected);

        // Reparse the UNCHANGED file through the refresh path.
        let path = root.join("src/alpha.rs");
        refresh_in_place(root, std::slice::from_ref(&path), &mut index);
        assert_eq!(
            index.trait_impl_locations("std::fmt::Display"),
            expected,
            "a same-content refresh must not strip the trait map"
        );

        // A REAL change: rename the self type — the stale location must
        // disappear and the new one appear (the map stays consistent).
        std::fs::write(
            &path,
            "struct AA;\nimpl std::fmt::Display for AA {\n    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, \"AA\") }\n}\n",
        )
        .unwrap();
        refresh_in_place(root, std::slice::from_ref(&path), &mut index);
        assert_eq!(
            index.trait_impl_locations("std::fmt::Display"),
            vec![TraitImplLocation {
                file: "src/alpha.rs".into(),
                impl_line: 1,
                self_type: "AA".into(),
            }]
        );

        // Delete the file: the trait-map entry leaves with the outline.
        std::fs::remove_file(&path).unwrap();
        refresh_in_place(root, &[path], &mut index);
        assert!(index.trait_impl_locations("std::fmt::Display").is_empty());
    }
}

