use super::*;

    /// Watchlist item 1 (U-D3 / U-K, review 05): `M-.` on a
    /// type/constant name — the end-to-end jump pinned per name shape.
    /// The extraction (`symbol_at_point`) has had no case filter since
    /// 006-02b (the skipped-uppercase bug was the old line-split
    /// heuristic, already replaced); this pins each shape so the fix
    /// stays: a CamelCase type (cross-file), a SCREAMING constant,
    /// and a mixed CamelCase type in a generic-argument position.
    #[test]
    fn xref_uppercase_type_and_const_shapes_jump_directly() {
        let (mut s, _d) = store_with_index(&[
            (
                "src/lib.rs",
                "mod widget;\nconst LOCAL_CONST: u32 = 2;\nfn use_it() {\n    let w = Widget { x: 1 };\n    let v: Vec<OtherThing> = vec![];\n    let _ = LOCAL_CONST;\n}\n",
            ),
            (
                "src/widget.rs",
                "pub struct Widget { pub x: i32 }\npub struct OtherThing { pub y: i32 }\n",
            ),
        ]);
        s.open_path("src/lib.rs");
        // Line 3: "    let w = Widget { x: 1 };" — cursor inside `Widget`
        // (CamelCase type, defined in another file).
        s.set_point(3, 13, 13);
        s.xref_find_definitions();
        assert_eq!(s.view_name_display(), "src/widget.rs", "CamelCase type: cross-file jump");
        assert_eq!(s.point_line(), 0, "to the struct's definition line");

        // Line 5: "    let _ = LOCAL_CONST;" — cursor inside `LOCAL_CONST`
        // (SCREAMING constant).
        s.open_path("src/lib.rs");
        s.set_point(5, 16, 16);
        s.xref_find_definitions();
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 1, "to the const's definition line");

        // Line 4: "    let v: Vec<OtherThing> = vec![];" — cursor inside
        // `OtherThing` (mixed CamelCase, generic-argument position).
        s.open_path("src/lib.rs");
        s.set_point(4, 18, 18);
        s.xref_find_definitions();
        assert_eq!(s.view_name_display(), "src/widget.rs");
        assert_eq!(s.point_line(), 1, "to the mixed CamelCase type");
    }

    /// Watchlist item 2 (imenu flat, no impl-parent nesting): a Rust
    /// file's imenu groups the impl methods under the impl's type —
    /// the method's display is indented one level below the struct
    /// (the Rung 1 tables: the method's definition line is not inside
    /// the struct's extent, so the grouping can only come from the
    /// tables). The names (the picker's match target) stay bare, and
    /// the query re-derivation (the refilter path) keeps the same
    /// display — the pre-fix drift where the initial open indented but
    /// the refilter did not.
    #[test]
    fn imenu_groups_impl_methods_under_the_struct() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Foo { a: i32 }\nimpl Foo {\n    pub fn new() -> Self { Self { a: 0 } }\n}\npub fn free() {}\n",
        )]);
        s.open_path("src/lib.rs");
        s.open_imenu();
        assert!(s.picker_open());
        assert_eq!(s.picker_kind(), Some(PickerKind::Imenu));
        // (name, display, label, detail): the row is now name-first
        // (picker-density) — the label carries the (indented) name and the
        // detail the kind tag; `display` (the match target) keeps its
        // byte-identical pre-conversion shape.
        let rows: Vec<(String, String, String, String)> = s
            .picker_filtered()
            .iter()
            .map(|(c, _)| (
                c.name.clone(),
                c.display.clone(),
                c.label.clone(),
                c.detail.clone(),
            ))
            .collect();
        let foo = rows.iter().find(|(n, _, _, _)| n == "Foo:1").unwrap();
        assert_eq!(foo.1, "Foo  [type]", "the struct stays at top level: {rows:?}");
        assert_eq!(foo.2, "Foo", "the struct label is unindented: {rows:?}");
        assert_eq!(foo.3, "[type]", "the kind tag is the detail: {rows:?}");
        let new = rows.iter().find(|(n, _, _, _)| n == "new:3").unwrap();
        assert_eq!(
            new.1,
            "  new  [fn]",
            "the impl method is indented one level under the struct: {rows:?}"
        );
        assert_eq!(new.2, "  new", "the indent travels with the label: {rows:?}");
        assert_eq!(new.3, "[fn]", "the kind tag is the detail: {rows:?}");
        let free = rows.iter().find(|(n, _, _, _)| n == "free:5").unwrap();
        assert_eq!(free.1, "free  [fn]", "a free fn stays flat: {rows:?}");
        assert_eq!(free.2, "free", "a free fn label stays flat: {rows:?}");
        assert_eq!(free.3, "[fn]", "the kind tag on the flat row: {rows:?}");

        // The refilter path (candidates_for) must keep the SAME display.
        s.picker_query_char('n');
        s.picker_query_char('e');
        let rows: Vec<(String, String)> = s
            .picker_filtered()
            .iter()
            .map(|(c, _)| (c.name.clone(), c.display.clone()))
            .collect();
        assert_eq!(
            rows,
            vec![("new:3".to_string(), "  new  [fn]".to_string())],
            "the re-derivation keeps the grouped display: {rows:?}"
        );
        // …and the SAME name-first split (label keeps the indent).
        let re = &s.picker_filtered()[0].0;
        assert_eq!(re.label, "  new", "re-derivation keeps the label: {re:?}");
        assert_eq!(re.detail, "[fn]", "re-derivation keeps the detail: {re:?}");
    }

    /// Watchlist item 2 degradation (byte-for-byte): a non-Rust file
    /// has no Rung 1 tables — its imenu stays the PART A
    /// enclosing-extent indent, no impl-parent grouping.
    #[test]
    fn imenu_non_rust_stays_flat() {
        let (mut s, _dir) = store_with_index(&[(
            "src/app.js",
            "function outer() {\n  function inner() {}\n}\nfunction free() {}\n",
        )]);
        s.open_path("src/app.js");
        s.open_imenu();
        let rows: Vec<(String, String)> = s
            .picker_filtered()
            .iter()
            .map(|(c, _)| (c.name.clone(), c.display.clone()))
            .collect();
        // The PART A enclosing-extent indent is unchanged (inner is
        // lexically inside outer's extent)…
        let inner = rows.iter().find(|(n, _)| n.starts_with("inner")).unwrap();
        assert_eq!(
            inner.1, "  inner  [fn]",
            "the enclosing-extent indent stands: {rows:?}"
        );
        // …and a top-level fn has no indent (no grouping to add one).
        let free = rows.iter().find(|(n, _)| n.starts_with("free")).unwrap();
        assert_eq!(free.1, "free  [fn]", "a top-level fn has no indent: {rows:?}");
    }

    #[test]
    fn xref_cross_file_definition_jumps_directly() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() { lib::target(); }\n"),
            ("src/lib.rs", "pub fn target() {}\npub fn other() {}\n"),
        ]);
        // Open main.rs and position the point at the call site: line 0, the
        // column of `target` (the cursor-aware selection: `lib` before the
        // `::` is NOT the lookup, `target` is).
        s.open_path("src/main.rs");
        s.set_point(0, 17, 17);
        // `target` is only defined in lib.rs: unique → jump directly.
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique cross-file: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 0, "target is at line 0 in lib.rs");
        // 006-02b item 2: the hit bumped the generation (superseding any
        // in-flight resolve) but started no job — exactly one bump.
        assert_eq!(s.resolve_generation, 1, "a workspace hit supersedes in-flight resolves");
    }

    #[test]
    fn xref_symbol_at_point_wins_over_other_line_identifiers() {
        // Two KNOWN definitions on one line: the old rule took
        // all_defs[0] after (file, line, name) sort — an arbitrary
        // identifier on the line. The cursor-aware rule must follow the
        // point's column.
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() { foo(); bar(); }\n"),
            ("src/a.rs", "pub fn foo() {}\n"),
            ("src/b.rs", "pub fn bar() {}\n"),
        ]);
        s.open_path("src/main.rs");
        // Cursor on `foo` (col 12): jumps to a.rs, not b.rs.
        s.set_point(0, 12, 12);
        s.xref_find_definitions();
        assert_eq!(s.view_name_display(), "src/a.rs", "cursor on foo → foo's definition");
        // Back to the call site, cursor on `bar` (col 20): jumps to b.rs.
        s.open_path("src/main.rs");
        s.set_point(0, 20, 20);
        s.xref_find_definitions();
        assert_eq!(s.view_name_display(), "src/b.rs", "cursor on bar → bar's definition");
    }

    #[test]
    fn xref_same_file_definition_jumps() {
        // The user's report: struct + impl in one file is the normal case
        // and must be jumpable (the old cross-file filter made it not).
        // Cursor on the `new` of `Foo::new()`: the path token `Foo::new`
        // is kept for the resolver, the index (name-keyed) is tried with
        // the last segment `new` — the same-file impl method wins.
        let (mut s, _dir) = store_with_index(&[
            (
                "src/lib.rs",
                "pub struct Foo {}\nimpl Foo {\n    pub fn new() -> Self { Foo {} }\n}\nfn use_it() {\n    let f = Foo::new();\n}\n",
            ),
        ]);
        s.open_path("src/lib.rs");
        // Line 5: "    let f = Foo::new();" — `new` starts at col 17.
        s.set_point(5, 17, 17);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique same-file: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 2, "jumped to the same-file impl method");
        assert_eq!(s.jump_stack.len(), 2, "origin + destination recorded");
    }

    /// Same-file M-. jump-back accuracy (user report: "popping back can go
    /// to the wrong place, not where my cursor was"). The jump stack's
    /// "current position" slot goes STALE when the point moves via plain
    /// motion (no jump recorded) between two M-. landings; the jump's
    /// origin must be the cursor's ACTUAL position at the second M-., not
    /// the previous jump's destination. M-, must land exactly at that
    /// (line, col).
    #[test]
    fn xref_same_file_mdot_back_lands_at_actual_cursor_not_stale_slot() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "fn a() {}\nfn b() {\n    a();\n}\nfn c() {\n    b();\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // 1. Cursor on the `a()` call (line 2, col 4). M-. -> line 0 (fn a).
        s.set_point(2, 4, 4);
        s.xref_find_definitions();
        assert_eq!(s.point_line(), 0, "landed on fn a");
        // 2. Plain motion (no jump) to the `b()` call (line 5, col 4).
        s.set_point(5, 4, 4);
        // 3. M-. from line 5 -> line 1 (fn b).
        s.xref_find_definitions();
        assert_eq!(s.point_line(), 1, "landed on fn b");
        // 4. M-, must land exactly at the cursor (line 5, col 4) — NOT at
        //    the previous jump destination (line 0, the fn a def).
        s.jump_back();
        assert_eq!(s.point_line(), 5, "M-, lands at the actual cursor line");
        assert_eq!(s.point_col(), 4, "M-, lands at the actual cursor col");
    }

    /// Audit pin (issue: same-file jump-back): the OTHER jump entry
    /// points share `record_jump` / `JumpStack` — the stale-slot bug
    /// lived in the shared stack, so each entry point's M-, is pinned
    /// here to keep the audit durable.
    #[test]
    fn probe_same_file_origin_scenarios() {
        // S1: same-file jump through the PICKER (two same-file defs).
        let (mut s, _dir) = store_with_index(&[
            (
                "src/lib.rs",
                "fn alpha() {}\nfn beta() {}\nfn alpha() {}\nfn caller() {\n    alpha();\n}\n",
            ),
        ]);
        s.open_path("src/lib.rs");
        s.set_point(4, 8, 8); // "    alpha();" — inside `alpha`
        s.xref_find_definitions();
        assert!(s.picker_open(), "S1: picker opens");
        s.run_selected();
        s.jump_back();
        assert_eq!((s.point_line(), s.point_col()), (4, 8), "S1: picker M-, restores origin");

        // S2: origin parked at end-of-line (col == line length).
        let (mut s, _dir) = store_with_index(&[
            ("src/lib.rs", "fn alpha() {}\nfn caller() {\n    alpha();\n}\n"),
        ]);
        s.open_path("src/lib.rs");
        s.set_point(2, 12, 12); // "    alpha();" line_len 12
        s.xref_find_definitions();
        assert!(!s.picker_open(), "S2: unique def");
        s.jump_back();
        assert_eq!((s.point_line(), s.point_col()), (2, 12), "S2: end-of-line origin restored");

        // S3: same-file jump via IMENU (M-i) then M-,
        let (mut s, _dir) = store_with_index(&[
            ("src/lib.rs", "fn alpha() {}\nfn beta() {}\nfn caller() {\n    alpha();\n}\n"),
        ]);
        s.open_path("src/lib.rs");
        s.set_point(3, 8, 8);
        s.key_event(key("M-i"));
        assert!(s.picker_open(), "S3: imenu picker");
        // Select `beta` (name "beta:2").
        let idx = s
            .picker
            .as_ref()
            .and_then(|p| p.filtered.iter().position(|(c, _)| c.name.starts_with("beta")))
            .expect("beta candidate");
        s.picker.as_mut().unwrap().selected = idx;
        s.run_selected();
        s.jump_back();
        assert_eq!((s.point_line(), s.point_col()), (3, 8), "S3: imenu M-, restores origin");

        // S4: same-file jump via enclosing-symbol fallback, then M-,.
        let (mut s, _dir) = store_with_index(&[
            (
                "src/lib.rs",
                "fn alpha() {}\nfn wrapper() {\n    // note\n    alpha();\n}\n",
            ),
        ]);
        s.open_path("src/lib.rs");
        s.set_point(2, 0, 0); // comment line: no symbol at point → enclosing = wrapper
        s.xref_find_definitions();
        assert!(!s.picker_open(), "S4: direct jump");
        s.jump_back();
        assert_eq!((s.point_line(), s.point_col()), (2, 0), "S4: enclosing-fallback M-, restores origin");

        // S5: chained same-file jumps A→B→C, two M-, must land on A.
        let (mut s, _dir) = store_with_index(&[
            (
                "src/lib.rs",
                "fn aaa() {}\nfn bbb() {\n    aaa();\n}\nfn ccc() {\n    bbb();\n}\n",
            ),
        ]);
        s.open_path("src/lib.rs");
        s.set_point(5, 8, 8); // ccc body: bbb call
        s.xref_find_definitions(); // ccc body → bbb
        s.xref_find_definitions(); // bbb body → aaa
        s.jump_back();
        s.jump_back();
        assert_eq!((s.point_line(), s.point_col()), (5, 8), "S5: chained same-file jumps land on A");

        // S6: same-file jump, then C-i forward, then M-,.
        let (mut s, _dir) = store_with_index(&[
            ("src/lib.rs", "fn alpha() {}\nfn caller() {\n    alpha();\n}\n"),
        ]);
        s.open_path("src/lib.rs");
        s.set_point(2, 8, 8);
        s.xref_find_definitions();
        s.jump_forward();
        s.jump_back();
        assert_eq!((s.point_line(), s.point_col()), (2, 8), "S6: forward+back round-trip");
    }

    /// Audit pin (issue: same-file jump-back): the 010-01/010-03
    /// pre-step (self-receiver / local-binding) M-. arms — same origin
    /// capture, shared stack.
    #[test]
    fn probe_same_file_prestep_origin_scenarios() {
        // P1: self.method pre-step (010-01), same-file impl method; jump-back.
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Foo { pub n: i32 }\nimpl Foo {\n    pub fn bump(&mut self) { self.n += 1; }\n    pub fn run(&mut self) { self.bump(); }\n}\nfn main() {\n    let mut f = Foo { n: 0 };\n    f.run();\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 3: "    pub fn run(&mut self) { self.bump(); }" — inside `bump`.
        s.set_point(3, 38, 38);
        s.xref_find_definitions();
        let landed = s.point_line();
        s.jump_back();
        assert_eq!((s.point_line(), s.point_col()), (3, 38), "P1: self.method M-, restores origin (landed {landed})");

        // P2: local-binding pre-step (010-03): `let x: Foo` then `x.bump()`.
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Foo { pub n: i32 }\nimpl Foo {\n    pub fn bump(&mut self) { self.n += 1; }\n}\nfn main() {\n    let x: Foo = Foo { n: 0 };\n    x.bump();\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 6: "    x.bump();" — inside `bump` (col 7..11).
        s.set_point(6, 9, 9);
        s.xref_find_definitions();
        let landed = s.point_line();
        s.jump_back();
        assert_eq!((s.point_line(), s.point_col()), (6, 9), "P2: local-binding M-, restores origin (landed {landed})");

        // P3: struct + impl, `Foo::new()` call site -> same-file impl method.
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Foo {}\nimpl Foo {\n    pub fn new() -> Self { Foo {} }\n}\nfn use_it() {\n    let f = Foo::new();\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 5: "    let f = Foo::new();" — inside `new` (col 17..20).
        s.set_point(5, 18, 18);
        s.xref_find_definitions();
        let landed = s.point_line();
        s.jump_back();
        assert_eq!((s.point_line(), s.point_col()), (5, 18), "P3: Foo::new M-, restores origin (landed {landed})");
    }

    #[test]
    fn xref_trait_definition_jumps() {
        // The user's report: on a Trait, jump into the trait definition.
        // `trait_item` is captured by the index, so a cursor on the trait
        // name at a use site lands on the definition.
        let (mut s, _dir) = store_with_index(&[
            ("src/lib.rs", "pub trait Tr {\n    fn m(&self);\n}\n"),
            ("src/main.rs", "fn use_it<T: Tr>() {}\n"),
        ]);
        s.open_path("src/main.rs");
        // "fn use_it<T: Tr>() {}" — the `Tr` bound starts at col 13.
        s.set_point(0, 13, 13);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique trait: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 0, "jumped to `pub trait Tr` (msg: {})", s.message);
    }

    // ── 010-04: find-implementations (plan 010 Shape A, rung 4) ─────

    /// 010-04 (discriminating): a trait at point with table entries opens
    /// the Impls picker over the `impl <Trait> for <Type>` blocks (the
    /// name-keyed trait map from the same index pass); RET reuses the
    /// Xref jump path (origin captured, jump recorded).
    #[test]
    fn find_implementations_opens_picker_of_trait_impls() {
        let (mut s, _dir) = store_with_index(&[
            (
                "src/lib.rs",
                "pub struct N;\npub trait Tr {\n    fn m(&self);\n}\nimpl Tr for N {\n    fn m(&self) {}\n}\n",
            ),
            (
                "src/extra.rs",
                "use crate::lib::Tr;\nstruct S;\nimpl Tr for S {\n    fn m(&self) {}\n}\n",
            ),
            ("src/main.rs", "use crate::lib::Tr;\nfn use_it<T: Tr>() {}\n"),
        ]);
        s.open_path("src/main.rs");
        // Line 1: "fn use_it<T: Tr>() {}" — `Tr` starts at col 13.
        s.set_point(1, 13, 13);
        s.find_implementations();
        assert!(s.picker_open(), "two impls: picker");
        assert_eq!(s.picker_kind(), Some(PickerKind::Impls));
        let filtered = s.picker_filtered();
        assert_eq!(filtered.len(), 2, "two impl blocks: {filtered:?}");
        // Deterministic (file, impl line) order: extra.rs's impl (line 3,
        // 1-based) before lib.rs's (line 5, 1-based); the display carries
        // the self type (kind: the trait impl's shape).
        assert_eq!(filtered[0].0.name, "src/extra.rs:3", "{}", filtered[0].0.display);
        assert!(filtered[0].0.display.contains("[impl Tr for S]"), "{}", filtered[0].0.display);
        assert_eq!(filtered[1].0.name, "src/lib.rs:5", "{}", filtered[1].0.display);
        assert!(
            filtered[1].0.display.contains("[impl Tr for N]"),
            "{}",
            filtered[1].0.display
        );
        // picker-density: the impl row is name-first — label = the impl
        // block (the name), detail = the location; display (the match
        // target) is pinned above.
        assert_eq!(filtered[0].0.label, "impl Tr for S", "{}", filtered[0].0.label);
        assert_eq!(filtered[0].0.detail, "src/extra.rs:3", "{}", filtered[0].0.detail);
        assert_eq!(filtered[1].0.label, "impl Tr for N", "{}", filtered[1].0.label);
        assert_eq!(filtered[1].0.detail, "src/lib.rs:5", "{}", filtered[1].0.detail);
        // RET on the lib.rs candidate jumps to the impl header (line 4,
        // 0-based) and records the jump.
        s.picker_select_next();
        s.run_selected();
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 4, "the impl header (msg: {})", s.message);
        assert_eq!(s.jump_stack.len(), 2, "origin + destination recorded");
    }

    /// 010-04 (pin): honest degradation — a trait with NO table entry
    /// (no Rust file impls it) runs the EXISTING bare-symbol M-. lookup
    /// byte-for-byte: `Tr` itself is indexed (`trait_item`), so the
    /// lookup lands on the trait definition exactly as M-. would.
    #[test]
    fn find_implementations_no_table_entry_degrades_to_bare_symbol_lookup() {
        let (mut s, _dir) = store_with_index(&[
            ("src/lib.rs", "pub trait Tr {\n    fn m(&self);\n}\n"),
            ("src/main.rs", "fn use_it<T: Tr>() {}\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 13, 13);
        s.find_implementations();
        assert!(!s.picker_open(), "degraded to the M-. path: {}", s.message);
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 0, "the trait definition (msg: {})", s.message);
        assert!(s.message.contains("jumped"), "the M-. jump message: {}", s.message);
    }

    /// 010-04 (pin): a GENERIC trait's captured text (`Tr<Foo>`) never
    /// matches the bare `Tr` at point — the map key is the impl's written
    /// trait text, so this degrades to the bare-symbol lookup (never a
    /// guess).
    #[test]
    fn find_implementations_generic_trait_text_never_matches_bare_name() {
        let (mut s, _dir) = store_with_index(&[
            ("src/lib.rs", "pub trait Tr {\n    fn m(&self);\n}\n"),
            ("src/impls.rs", "struct Foo;\nimpl Tr<Foo> for Foo {\n    fn m(&self) {}\n}\n"),
            ("src/main.rs", "fn use_it<T: Tr>() {}\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 13, 13);
        s.find_implementations();
        // No entry for the bare `Tr` (the map key is `Tr<Foo>`) — the
        // bare-symbol M-. lookup lands on the trait definition instead.
        assert!(!s.picker_open(), "generic trait text never matches: {:?}", s.message);
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 0, "the trait definition (msg: {})", s.message);
    }

    /// 010-04 (pin): no symbol at point — the M-. guard message, no
    /// picker, no jump.
    #[test]
    fn find_implementations_no_symbol_under_point() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub trait Tr {\n    fn m(&self);\n}\nimpl Tr for i32 {\n    fn m(&self) {}\n}\n",
        )]);
        s.open_path("src/lib.rs");
        s.set_point(0, 13, 13); // on the `{` after `pub trait Tr`
        s.find_implementations();
        assert!(!s.picker_open());
        assert_eq!(s.message, "no symbol under point");
    }

    // ── 010-01: M-. self-receiver resolution (Shape A rung 1) ──────────

    /// 010-01 (discriminating): the extraction carries the `self.` receiver
    /// for Rust `self.<member>` — pre-010-01 it stayed bare (fields are not
    /// in the index). `myself.` / arbitrary receivers stay byte-for-byte.
    #[test]
    fn symbol_at_point_rust_self_access_carries_the_receiver() {
        let line = "    let v = self.a;";
        // `a` at col 17, and parked right after it (col 18).
        assert_eq!(
            satp(LanguageId::Rust, line, 17),
            Some(("a".into(), "self.a".into()))
        );
        assert_eq!(
            satp(LanguageId::Rust, line, 18),
            Some(("a".into(), "self.a".into()))
        );
        // Call-shaped: the same token on `self.method()`.
        let call = "        let _ = self.method();";
        // `method` starts at col 21; cursor on the `e` at col 22.
        assert_eq!(
            satp(LanguageId::Rust, call, 22),
            Some(("method".into(), "self.method".into()))
        );
        // `myself.`: the 4-char window must be EXACTLY `self` — stays bare.
        let selfy = "let v = myself.a;";
        assert_eq!(
            satp(LanguageId::Rust, selfy, 15),
            Some(("a".into(), "a".into())),
            "myself.a stays bare"
        );
        // An arbitrary receiver stays bare (byte-for-byte the pre-010-01
        // extraction — `obj.a` was never resolvable and still isn't).
        assert_eq!(
            satp(LanguageId::Rust, "let v = obj.a;", 12),
            Some(("a".into(), "a".into()))
        );
        // Non-Rust `self.x` is NOT the Rust self shape — the 011-06
        // language-aware path container handling stands (Python attribute
        // already extended; the Rust branch never fires). `x` is at col 9.
        assert_eq!(
            satp(LanguageId::Python, "y = self.x", 9),
            Some(("x".into(), "self.x".into()))
        );
    }

    /// 010-03 (pin): the local-binding pre-step's own receiver scan — a
    /// bare receiver on a Rust `x.<member>` is carried; `self.` stays
    /// 010-01's, and expression / path / call receivers are never
    /// treated as local bindings (they stay bare, byte-for-byte).
    #[test]
    fn rust_dotted_receiver_scan_rules() {
        let rd = AppStore::rust_dotted_receiver;
        // `let _ = p.x;` — `x` at col 10.
        assert_eq!(rd("let _ = p.x;", 10), Some(("p".into(), 10)));
        // Call-shaped, parked right after the name (before the `(`):
        // `go` spans col 10-11, parked at 12.
        assert_eq!(rd("let _ = p.go();", 12), Some(("p".into(), 10)));
        // `self.` is the 010-01 pre-step's — never reported here.
        assert_eq!(rd("let _ = self.x;", 13), None);
        // A `::`-path receiver (`a::b.x`): not a local binding.
        assert_eq!(rd("let _ = a::b.x;", 13), None);
        // A DOT-CHAINED receiver (`a.b.x`): the middle segment `b` is a
        // field access, never a local binding (review P1 — a misread
        // here would jump to the wrong struct's member); the call
        // variant `a.b.go()` too.
        assert_eq!(rd("let _ = a.b.x;", 12), None);
        assert_eq!(rd("let _ = a.b.go();", 14), None);
        // Expression receivers: a call / an index / a paren.
        assert_eq!(rd("let _ = f().x;", 12), None);
        assert_eq!(rd("let _ = v[0].x;", 13), None);
        assert_eq!(rd("let _ = (p).x;", 12), None);
        // The point not on a member run: the dot itself, or the
        // receiver's own run.
        assert_eq!(rd("let _ = p.x;", 9), None);
        assert_eq!(rd("let _ = p.x;", 8), None);
        // A longer receiver word is ONE bare identifier (`myself.x`
        // is the binding `myself` — not a self access, not rejected).
        assert_eq!(rd("let _ = myself.x;", 15), Some(("myself".into(), 15)));
    }

    /// 010-01 (discriminating): `self.a` inside `impl Foo` jumps to the
    /// struct field's line. `a` is NOT in the symbol index (field
    /// declarations are not outline symbols) — pre-010-01 this degraded to
    /// the enclosing-symbol fallback (the enclosing `fn`), so this outcome
    /// only exists because of the table pre-step.
    #[test]
    fn xref_self_field_jumps_to_struct_field_line() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Foo { pub a: i32 }\nimpl Foo {\n    pub fn use_it(&self) { let _ = self.a; }\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 2: "    pub fn use_it(&self) { let _ = self.a; }" — `a`
        // starts at col 40.
        s.set_point(2, 40, 40);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique same-file field: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            0,
            "jumped to the field declaration (msg: {})",
            s.message
        );
    }

    /// 010-01 (discriminating): the field lives in ANOTHER file of the
    /// project — the index's cross-file field locations carry it (the
    /// same-file-first ordering then lands in `src/model.rs`).
    #[test]
    fn xref_self_field_resolves_cross_file() {
        let (mut s, _dir) = store_with_index(&[
            ("src/model.rs", "pub struct Point { pub x: i32, pub y: i32 }\n"),
            (
                "src/main.rs",
                "use crate::model::Point;\nimpl Point {\n    fn coords(&self) { let _ = self.x; }\n}\n",
            ),
        ]);
        s.open_path("src/main.rs");
        // Line 2: "    fn coords(&self) { let _ = self.x; }" — `x` starts
        // at col 36.
        s.set_point(2, 36, 36);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique cross-file field: no picker");
        assert_eq!(s.view_name_display(), "src/model.rs");
        assert_eq!(
            s.point_line(),
            0,
            "jumped to `x` in model.rs (msg: {})",
            s.message
        );
    }

    /// 010-01: `self.method()` inside `impl Foo` jumps to the impl method
    /// (its line, from the same-file impl table).
    #[test]
    fn xref_self_method_call_jumps_to_impl_method() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Foo { pub a: i32 }\nimpl Foo {\n    pub fn method(&self) -> i32 { self.a + 1 }\n    pub fn use_it(&self) { let _ = self.method(); }\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 3: "    pub fn use_it(&self) { let _ = self.method(); }" —
        // `method` starts at col 40.
        s.set_point(3, 40, 40);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique same-file method: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            2,
            "jumped to `fn method` (msg: {})",
            s.message
        );
    }

    /// 010-01: a name that is BOTH a field and a method of the enclosing
    /// type → the picker (same-file-first, never guessed away).
    #[test]
    fn xref_self_ambiguous_member_opens_picker() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Foo { pub extra: i32 }\nimpl Foo {\n    fn extra(&self) {}\n    fn use_it(&self) { self.extra(); }\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 3: "    fn use_it(&self) { self.extra(); }" — `extra`
        // starts at col 28.
        s.set_point(3, 28, 28);
        s.xref_find_definitions();
        assert!(s.picker_open(), "field + method: picker");
        assert_eq!(s.picker_kind(), Some(PickerKind::Xref));
        let filtered = s.picker_filtered();
        assert_eq!(filtered.len(), 2, "two candidates: {filtered:?}");
        // Same-file, line-ordered: the field (line 0) before the method
        // (line 2).
        assert!(
            filtered[0].0.name.starts_with("src/lib.rs:1"),
            "field candidate first: {}",
            filtered[0].0.name
        );
        assert!(
            filtered[1].0.name.starts_with("src/lib.rs:3"),
            "method candidate second: {}",
            filtered[1].0.name
        );
    }

    /// 010-01 (pin): honest degradation — `self.a` inside a GENERIC impl
    /// (`impl<T> Foo<T>`) never resolves through the tables: the self type
    /// is not a plain identifier, so the exact pre-010-01 behavior stands
    /// (bare `a` → no index hit → the enclosing symbol takes over).
    #[test]
    fn xref_self_in_generic_impl_degrades_to_today() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Foo { pub a: i32 }\nimpl<T> Foo<T> {\n    fn f(&self) { let _ = self.a; }\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 2: "    fn f(&self) { let _ = self.a; }" — `a` starts at
        // col 31.
        s.set_point(2, 31, 31);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "degraded: no picker");
        // The enclosing-symbol fallback landed on `f` itself (its only
        // indexed definition) — today's behavior, byte-for-byte.
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            2,
            "enclosing `f` took over (msg: {})",
            s.message
        );
    }

    /// 010-01 (pin): `self.a` with NO enclosing impl (top-level / outside
    /// every impl block) degrades to the exact pre-010-01 behavior — and a
    /// non-Rust buffer is never touched by the pre-step at all.
    #[test]
    fn xref_self_without_enclosing_impl_degrades_to_today() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Foo { pub a: i32 }\nfn free() { let _ = self; }\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 1: "fn free() { let _ = self; }" — no member after `self`
        // (the token is `self`, not `self.<member>`): the pre-step never
        // fires; `self` has no definition → the enclosing `free` takes
        // over (today's behavior).
        s.set_point(1, 24, 24);
        s.xref_find_definitions();
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 1, "enclosing `free` (msg: {})", s.message);
    }

    // ── 010-03: M-. local-binding resolution (Shape A rung 3) ───────

    /// 010-03 (discriminating): `p.x` where `p` has a written annotation
    /// jumps to the struct field's line. `x` is NOT in the symbol index
    /// (field declarations are not outline symbols), so pre-010-03 this
    /// degraded to the enclosing-symbol fallback (`main`) — this outcome
    /// only exists because of the binding pre-step.
    #[test]
    fn xref_local_binding_field_jumps_to_struct_field_line() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Pt { pub x: i32 }\nfn main() {\n    let p: Pt = Pt { x: 1 };\n    let _ = p.x;\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 3: "    let _ = p.x;" — `x` at col 14.
        s.set_point(3, 14, 14);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique same-file field: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            0,
            "jumped to the field declaration (msg: {})",
            s.message
        );
    }

    /// 010-03 (discriminating): the binding's type comes from the
    /// struct-literal RHS — NO annotation on the `let`. Pre-010-03 the
    /// literal was invisible to the tables.
    #[test]
    fn xref_local_binding_struct_literal_jumps_to_field() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Pt { pub x: i32 }\nfn main() {\n    let p = Pt { x: 1 };\n    let _ = p.x;\n}\n",
        )]);
        s.open_path("src/lib.rs");
        s.set_point(3, 14, 14);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique same-file field: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            0,
            "the struct literal's type carried the jump (msg: {})",
            s.message
        );
    }

    /// 010-03 (discriminating): `let mut p: Pt` is the SAME binding as
    /// `let p: Pt` — the annotation pre-step resolves through `mut`.
    #[test]
    fn xref_local_binding_mut_resolves_like_plain() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Pt { pub x: i32 }\nfn main() {\n    let mut p: Pt = Pt { x: 1 };\n    let _ = p.x;\n}\n",
        )]);
        s.open_path("src/lib.rs");
        s.set_point(3, 14, 14);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique same-file field: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 0, "mut binding resolved (msg: {})", s.message);
    }

    /// 010-03 (discriminating): `b.go()` where TWO types define `go` —
    /// pre-010-03 the bare `go` index lookup was ambiguous (the picker
    /// over both impls); the binding's written type narrows it to B's
    /// impl method: a unique jump.
    #[test]
    fn xref_local_binding_method_call_narrows_ambiguous_impls() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct A { pub m: i32 }\n\
             pub struct B { pub m: i32 }\n\
             impl A {\n\
             \x20   fn go(&self) { let _ = self.m; }\n\
             }\n\
             impl B {\n\
             \x20   fn go(&self) { let _ = self.m; }\n\
             }\n\
             fn main() {\n\
             \x20   let b: B = B { m: 1 };\n\
             \x20   b.go();\n\
             }\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 10: "    b.go();" — `go` at col 6.
        s.set_point(10, 6, 6);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "the written type narrows to one impl: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            6,
            "jumped to B's `go` (msg: {})",
            s.message
        );
    }

    /// 010-03 (discriminating): the field lives in ANOTHER file — the
    /// binding's type resolves through the index's cross-file field
    /// locations (the same-file-first ordering lands in `src/model.rs`).
    #[test]
    fn xref_local_binding_field_resolves_cross_file() {
        let (mut s, _dir) = store_with_index(&[
            ("src/model.rs", "pub struct Point { pub x: i32, pub y: i32 }\n"),
            (
                "src/main.rs",
                "use crate::model::Point;\nfn main() {\n    let p: Point = Point { x: 1, y: 2 };\n    let _ = p.x;\n}\n",
            ),
        ]);
        s.open_path("src/main.rs");
        s.set_point(3, 14, 14);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique cross-file field: no picker");
        assert_eq!(s.view_name_display(), "src/model.rs");
        assert_eq!(
            s.point_line(),
            0,
            "jumped to `x` in model.rs (msg: {})",
            s.message
        );
    }

    /// 010-03 (pin): a SHADOWED name — the innermost binding wins.
    /// `x.f` inside the nested block resolves via the inner `x: B`, not
    /// the outer `x: A`; the same use AFTER the block (outside the
    /// shadow) resolves via the outer `A`. Pre-010-03 both degraded to
    /// the enclosing-symbol fallback.
    #[test]
    fn xref_local_binding_shadow_innermost_wins() {
        let files: &[(&str, &str)] = &[(
            "src/lib.rs",
            "pub struct A { pub f: i32 }\n\
             pub struct B { pub f: i32 }\n\
             fn main() {\n\
             \x20   let x: A;\n\
             \x20   let _o = x.f;\n\
             \x20   {\n\
             \x20       let x: B;\n\
             \x20       let _i = x.f;\n\
             \x20   }\n\
             }\n",
        )];
        // Inner use (line 7: "        let _i = x.f;") — `f` at col 19:
        // the shadow → B's field (line 1).
        let (mut s, _dir) = store_with_index(files);
        s.open_path("src/lib.rs");
        s.set_point(7, 19, 19);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique field: no picker");
        assert_eq!(
            s.point_line(),
            1,
            "inner use resolves via the inner shadow `x: B` (msg: {})",
            s.message
        );
        // Outer use (line 4: "    let _o = x.f;") — `f` at col 15: the
        // shadow is out of scope → A's field (line 0).
        let (mut s, _dir) = store_with_index(files);
        s.open_path("src/lib.rs");
        s.set_point(4, 15, 15);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique field: no picker");
        assert_eq!(
            s.point_line(),
            0,
            "outer use resolves via the outer `x: A` (msg: {})",
            s.message
        );
    }

    /// 010-03 (pin): an UNANNOTATED receiver — the pre-step misses and
    /// the exact pre-010-03 bare-`<member>` behavior stands: `x` still
    /// resolves to the indexed `fn x` (the bare path token, byte-for-
    /// byte; the extraction was never changed for non-self receivers).
    #[test]
    fn xref_local_binding_unannotated_receiver_stays_bare() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Pt { pub x: i32 }\npub fn x() {}\nfn main() {\n    let p = make();\n    let _ = p.x;\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 4: "    let _ = p.x;" — `x` at col 14. `p` is not
        // annotated (the RHS is a call, not a struct literal), so the
        // pre-step misses and the bare `x` index lookup carries it.
        s.set_point(4, 14, 14);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique bare hit: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            1,
            "the bare `x` lookup landed on `fn x` (msg: {})",
            s.message
        );
    }

    /// 010-03 (pin): a binding annotated to a type with no recorded
    /// fields or impls (`Marker` is a unit struct — nothing in the
    /// tables) degrades to today's behavior: the bare `thing` carries
    /// through to the indexed `fn thing` (the pre-step gathered no
    /// candidates and never guessed) — not a table hit, not the
    /// enclosing symbol.
    #[test]
    fn xref_local_binding_non_struct_type_degrades_to_today() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub fn thing() {}\nstruct Marker;\nfn main() {\n    let m: Marker = Marker;\n    let _ = m.thing;\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 4: "    let _ = m.thing;" — `thing` starts at col 14.
        s.set_point(4, 14, 14);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique bare hit: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            0,
            "the bare `thing` lookup landed on `fn thing` (msg: {})",
            s.message
        );
    }

    #[test]
    fn xref_ambiguous_cross_file_opens_picker() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() { target(); }\n"),
            ("src/a.rs", "pub fn target() {}\n"),
            ("src/b.rs", "pub fn target() {}\n"),
        ]);
        s.open_path("src/main.rs");
        // Cursor on `target` (col 12): defined in a.rs and b.rs → ambiguous.
        s.set_point(0, 12, 12);
        s.xref_find_definitions();
        assert!(s.picker_open(), "ambiguous: picker should be open");
        assert_eq!(s.picker_kind(), Some(PickerKind::Xref));
        assert_eq!(s.picker_filtered().len(), 2, "two candidates");
    }

    #[test]
    fn xref_ambiguous_same_file_first_in_picker() {
        // Same-file candidate sorts first in the picker (selection rule 2).
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "mod lib {\n    pub fn target() {}\n}\nfn main() { lib::target(); }\n"),
            ("src/lib.rs", "pub fn target() {}\n"),
        ]);
        s.open_path("src/main.rs");
        // Line 3: "fn main() { lib::target(); }" — `target` starts at col 17.
        s.set_point(3, 17, 17);
        s.xref_find_definitions();
        assert!(s.picker_open(), "two candidates: picker");
        let filtered = s.picker_filtered();
        assert_eq!(filtered.len(), 2);
        assert!(filtered[0].0.name.starts_with("src/main.rs:"), "same-file candidate listed first: {}", filtered[0].0.name);
        assert!(filtered[1].0.name.starts_with("src/lib.rs:"), "cross-file candidate second: {}", filtered[1].0.name);
    }

    #[test]
    fn xref_no_symbol_under_point_falls_back_to_enclosing() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() {\n    let x = 1;\n}\n"),
        ]);
        s.open_path("src/main.rs");
        // Line 1: "    let x = 1;" col 0 — no identifier AT the point.
        // Fall back to enclosing symbol: `main` (unchanged behavior).
        s.set_point_line(1);
        s.xref_find_definitions();
        // `main` is defined only in main.rs: unique → jump to main's definition (line 0).
        assert!(!s.picker_open());
        assert_eq!(s.point_line(), 0, "jumped to main's definition");
        // 006-02b item 2: the enclosing hit bumps the generation (one
        // supersede bump, no job).
        assert_eq!(s.resolve_generation, 1, "the enclosing hit supersedes in-flight resolves");
    }

    /// 010-03 review P1 (pin): a DOT-CHAINED receiver — `a.b.c` where
    /// the middle segment `b` happens to be a local binding with a
    /// written type (`D`) that ALSO has a field `c` — must NOT be
    /// misattributed to `b`: the middle segment is a field access, never
    /// a local binding, so today's bare `c` behavior stands (no jump to
    /// `D`'s `c` — the wrong struct — the enclosing `main` takes over
    /// instead).
    #[test]
    fn xref_local_binding_dot_chained_receiver_stays_bare() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct B { pub c: i32 }\npub struct D { pub c: i32 }\npub struct A { pub b: B }\nfn main() {\n    let b: D;\n    let a = A { b: B { c: 1 } };\n    let _ = a.b.c;\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 6: "    let _ = a.b.c;" — `c` at col 16. Pre-fix this
        // jumped to `D`'s `c` (line 1) through the misattributed `b`.
        s.set_point(6, 16, 16);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "no candidates: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            3,
            "the bare `c` degraded to the enclosing `main` — NOT `D`'s `c` (msg: {})",
            s.message
        );
    }

    // ── plan 006 issue 02: tooling-resolver fall-through ─────────────────────────

    #[test]
    fn symbol_at_point_identifier_and_path_token() {
        // Rust `::`: byte-for-byte the pre-011-06 extraction (no parse).
        let line = "    let h = tokio::spawn(f);";
        // Cursor inside `tokio` (col 12) → ident `tokio`, path `tokio::spawn`.
        assert_eq!(
            satp(LanguageId::Rust, line, 12),
            Some(("tokio".into(), "tokio::spawn".into()))
        );
        // Cursor inside `spawn` (col 19) → same path token.
        assert_eq!(
            satp(LanguageId::Rust, line, 19),
            Some(("spawn".into(), "tokio::spawn".into()))
        );
        // Cursor parked right after `spawn` (before the `)` — the usual
        // call-site spot) still counts.
        assert_eq!(
            satp(LanguageId::Rust, line, 24),
            Some(("spawn".into(), "tokio::spawn".into()))
        );
    }

    #[test]
    fn symbol_at_point_field_access_stays_bare() {
        // A Rust `.`-accessed field: the token stays the bare field name
        // (fields are NOT in the index, and Rust `::`-only extraction is
        // byte-for-byte preserved by 011-06 — the language-aware
        // extension covers the OTHER languages' dotted path shapes).
        let line = "    let n = obj.name;";
        assert_eq!(
            satp(LanguageId::Rust, line, 17),
            Some(("name".into(), "name".into()))
        );
        assert_eq!(
            satp(LanguageId::Rust, line, 20),
            Some(("name".into(), "name".into()))
        );
    }

    /// 011-06 (discriminating): in a non-Rust buffer, the M-. path token
    /// becomes the WHOLE dotted path when the point sits in the language's
    /// path container — this is what makes the providers' already-
    /// unit-tested dotted handling reachable from M-.. Pre-011-06 every
    /// one of these returned the BARE identifier.
    #[test]
    fn symbol_at_point_dotted_path_extends_token_per_language() {
        // Python: `json.dumps` at a use site — cursor on EITHER segment.
        let line = "y = json.dumps(x)";
        assert_eq!(
            satp(LanguageId::Python, line, 5),
            Some(("json".into(), "json.dumps".into()))
        );
        assert_eq!(
            satp(LanguageId::Python, line, 11),
            Some(("dumps".into(), "json.dumps".into()))
        );
        // Cursor parked right after `dumps` (before `(` — the usual
        // call-site spot) still counts.
        assert_eq!(
            satp(LanguageId::Python, line, 14),
            Some(("dumps".into(), "json.dumps".into()))
        );
        // Python deep chain: `os.path.join` under the MIDDLE segment.
        let deep = "os.path.join(a, b)";
        assert_eq!(
            satp(LanguageId::Python, deep, 4),
            Some(("path".into(), "os.path.join".into()))
        );
        // JS: `fakelib.apply(5)` — cursor on the member, and parked just
        // after it (before `(`); TS gets the same answer (011-03 shares
        // the JS machinery).
        let js = "fakelib.apply(5);";
        assert_eq!(
            satp(LanguageId::JavaScript, js, 10),
            Some(("apply".into(), "fakelib.apply".into()))
        );
        assert_eq!(
            satp(LanguageId::JavaScript, js, 13),
            Some(("apply".into(), "fakelib.apply".into()))
        );
        assert_eq!(
            satp(LanguageId::TypeScript, js, 10),
            Some(("apply".into(), "fakelib.apply".into()))
        );
        // Go: `fmt.Println(x)` (selector_expression) and a
        // `qualified_type` in type position.
        let go = "fmt.Println(x)";
        assert_eq!(
            satp(LanguageId::Go, go, 7),
            Some(("Println".into(), "fmt.Println".into()))
        );
        let gotype = "var v fmt.Stringer";
        assert_eq!(
            satp(LanguageId::Go, gotype, 12),
            Some(("Stringer".into(), "fmt.Stringer".into()))
        );
        // 006-02b-style separator rule for `.`: a cursor right after
        // `json` (on the dot) counts as the end of the preceding segment.
        assert_eq!(
            satp(LanguageId::Python, "json.dumps", 4),
            Some(("json".into(), "json.dumps".into()))
        );
    }

    /// 011-06 (pin): the degradation stays byte-for-byte — bare
    /// identifiers, an unimplemented language, and an identifier that is
    /// NOT a full dot-delimited segment of its container (a computed
    /// member `a[b]`, no dot at all) all keep the exact bare extraction.
    #[test]
    fn symbol_at_point_non_rust_bare_and_unsupported_shapes_stay_bare() {
        // Bare identifiers (no path container around them): the bare
        // extraction, in every language.
        assert_eq!(
            satp(LanguageId::Python, "x = 1", 0),
            Some(("x".into(), "x".into()))
        );
        assert_eq!(
            satp(LanguageId::JavaScript, "const x = 1;", 6),
            Some(("x".into(), "x".into()))
        );
        assert_eq!(
            satp(LanguageId::Go, "const Z = 3", 6),
            Some(("Z".into(), "Z".into()))
        );
        // Unimplemented language (Plain): no parse, byte-for-byte bare —
        // the SAME line in Python extends, here it does not.
        assert_eq!(
            satp(LanguageId::Plain, "json.dumps", 6),
            Some(("dumps".into(), "dumps".into()))
        );
        // 011-06 review P1: the WRONG-container shapes — a whole-path
        // upgrade requires every dot-delimited segment to be a bare
        // identifier; these containers match segment-membership but carry
        // non-identifier segments, so they degrade to bare (feeding the
        // providers `a?` / `foo()` / `(*p)` as a package name would cause
        // unintended npm/pip shell-outs in online projects).
        assert_eq!(
            satp(LanguageId::JavaScript, "a?.b", 3),
            Some(("b".into(), "b".into()))
        );
        assert_eq!(
            satp(LanguageId::JavaScript, "a.b?.c", 5),
            Some(("c".into(), "c".into()))
        );
        assert_eq!(
            satp(LanguageId::Python, "foo().bar", 8),
            Some(("bar".into(), "bar".into()))
        );
        assert_eq!(
            satp(LanguageId::Python, "foo().bar.b", 10),
            Some(("b".into(), "b".into()))
        );
        assert_eq!(
            satp(LanguageId::Go, "(*p).field", 5),
            Some(("field".into(), "field".into()))
        );
        // A computed member `a[b]`: the node IS a member_expression, but
        // `b` is not a dot-delimited SEGMENT of `a[b]` (no dot at all) —
        // never feed the providers a non-path token; the bare extraction
        // stands.
        assert_eq!(
            satp(LanguageId::JavaScript, "a[b]", 2),
            Some(("b".into(), "b".into()))
        );
        // Punctuation / whitespace around the point: no symbol (the
        // identifier-run rule is language-independent).
        assert_eq!(satp(LanguageId::Python, "let a = 1;", 7), None);
        assert_eq!(satp(LanguageId::Python, "{ ", 1), None);
    }

    /// 010-rung4-and-paths (item 2, app-side whole-path upgrade): the
    /// per-language container pins — C's `field_expression`, Cpp's
    /// `field_expression` (its `::` shape stays the byte-scan token —
    /// NOT double-handled), Toml's `dotted_key` (the index stores the
    /// dotted key as ONE symbol name, so the segment's M-. only reaches
    /// it with the whole path).
    #[test]
    fn symbol_at_point_c_cpp_toml_containers_extend_the_token() {
        // C: `o.x` — cursor on the member and parked right after it.
        // "int y = o.x;": o@8, x@10.
        let c = "int y = o.x;";
        assert_eq!(
            satp(LanguageId::C, c, 10),
            Some(("x".into(), "o.x".into()))
        );
        assert_eq!(
            satp(LanguageId::C, c, 11),
            Some(("x".into(), "o.x".into()))
        );
        // C deep chain: `o.x.y` under the MIDDLE segment (x@10).
        let cdeep = "int v = o.x.y;";
        assert_eq!(
            satp(LanguageId::C, cdeep, 10),
            Some(("x".into(), "o.x.y".into()))
        );
        // Cpp: `o.x` — field_expression, same shape as C.
        let cpp = "int y = o.x;";
        assert_eq!(
            satp(LanguageId::Cpp, cpp, 10),
            Some(("x".into(), "o.x".into()))
        );
        // Cpp deep chain: `a.b.c` under the middle segment (b@10).
        let cppdeep = "int v = a.b.c;";
        assert_eq!(
            satp(LanguageId::Cpp, cppdeep, 10),
            Some(("b".into(), "a.b.c".into()))
        );
        // Cpp `::` (no double handling): `ns::A::x` stays the byte-scan
        // whole token — the pre-010-rung4 extraction, byte-for-byte
        // (A@13).
        let qual = "auto v = ns::A::x;";
        assert_eq!(
            satp(LanguageId::Cpp, qual, 13),
            Some(("A".into(), "ns::A::x".into()))
        );
        // Toml: dotted key `a.b.c = 1` — cursor on either segment.
        let toml = "a.b.c = 1";
        assert_eq!(
            satp(LanguageId::Toml, toml, 0),
            Some(("a".into(), "a.b.c".into()))
        );
        assert_eq!(
            satp(LanguageId::Toml, toml, 4),
            Some(("c".into(), "a.b.c".into()))
        );
        // Toml table-header dotted key (`[a.b]`, b@3).
        assert_eq!(
            satp(LanguageId::Toml, "[a.b]", 3),
            Some(("b".into(), "a.b".into()))
        );
    }

    /// 010-rung4-and-paths (item 2, pin): the degradation stays
    /// byte-for-byte — C/Cpp `p->x` (the `->` segments are not bare
    /// identifier segments) and JSON keys (the pinned JSON grammar has
    /// no dotted-key node; the judgment: bare-key index lookup is the
    /// whole feature) all keep the exact bare extraction.
    #[test]
    fn symbol_at_point_c_arrow_and_json_keys_stay_bare() {
        // "int y = p->x;": x@11.
        let line = "int y = p->x;";
        assert_eq!(
            satp(LanguageId::C, line, 11),
            Some(("x".into(), "x".into()))
        );
        assert_eq!(
            satp(LanguageId::Cpp, line, 11),
            Some(("x".into(), "x".into()))
        );
        // Json: a quoted key — the bare key, byte-for-byte (k@3).
        let js = "  \"k\": 1";
        assert_eq!(
            satp(LanguageId::Json, js, 3),
            Some(("k".into(), "k".into()))
        );
    }

    /// newlang-paths (discriminating): the M-. path token becomes the
    /// WHOLE dotted path for the new-languages-lane containers — Java's
    /// `field_access` / `scoped_identifier` / `scoped_type_identifier`,
    /// C#'s `member_access_expression` / `qualified_name`, and Ruby's
    /// argumentless `call` (the node.rs position gate + container-validity
    /// rule; the kinds are pinned there — `java_member_path_comes_back_
    /// whole`, `java_scoped_type_path_comes_back_whole`,
    /// `csharp_member_path_comes_back_whole`,
    /// `csharp_qualified_name_comes_back_whole`,
    /// `ruby_method_chain_comes_back_whole`). Pre-newlang-paths every one
    /// of these returned the BARE identifier.
    #[test]
    fn symbol_at_point_java_csharp_ruby_containers_extend_the_token() {
        // Java `field_access`: `A.c` — cursor on the receiver and the
        // member. "class A { int c; void f() { int x = A.c; } }\n": A@36,
        // c@38.
        let java = "class A { int c; void f() { int x = A.c; } }\n";
        assert_eq!(
            satp(LanguageId::Java, java, 36),
            Some(("A".into(), "A.c".into()))
        );
        assert_eq!(
            satp(LanguageId::Java, java, 38),
            Some(("c".into(), "A.c".into()))
        );
        // Java deep chain: `a.b.c` under the MIDDLE segment (b@31).
        let javadeep = "class A { void f() { int v = a.b.c; } }\n";
        assert_eq!(
            satp(LanguageId::Java, javadeep, 31),
            Some(("b".into(), "a.b.c".into()))
        );
        // Java scoped type: `com.example.Foo` under the middle segment
        // (example@25).
        let javatype = "class B { void f() { com.example.Foo o; } }\n";
        assert_eq!(
            satp(LanguageId::Java, javatype, 25),
            Some(("example".into(), "com.example.Foo".into()))
        );
        // C# `member_access_expression`: `o.P` — cursor on the member
        // and parked right after it. "class A { void F() { int v = o.P;
        // } }\n": o@29, P@31.
        let cs = "class A { void F() { int v = o.P; } }\n";
        assert_eq!(
            satp(LanguageId::CSharp, cs, 31),
            Some(("P".into(), "o.P".into()))
        );
        assert_eq!(
            satp(LanguageId::CSharp, cs, 32),
            Some(("P".into(), "o.P".into()))
        );
        // C# deep chain: `a.b.c` under the middle segment (b@31).
        let csdeep = "class A { void F() { int v = a.b.c; } }\n";
        assert_eq!(
            satp(LanguageId::CSharp, csdeep, 31),
            Some(("b".into(), "a.b.c".into()))
        );
        // C# `qualified_name` in a namespace header (Inner@12).
        assert_eq!(
            satp(LanguageId::CSharp, "namespace N.Inner { class A { } }", 12),
            Some(("Inner".into(), "N.Inner".into()))
        );
        // Ruby argumentless `call`: `obj.name` — cursor on EITHER
        // segment (obj@0, name@4).
        assert_eq!(
            satp(LanguageId::Ruby, "obj.name", 0),
            Some(("obj".into(), "obj.name".into()))
        );
        assert_eq!(
            satp(LanguageId::Ruby, "obj.name", 4),
            Some(("name".into(), "obj.name".into()))
        );
        // Ruby deep chain: `a.b.c` under the middle segment (b@2).
        assert_eq!(
            satp(LanguageId::Ruby, "a.b.c", 2),
            Some(("b".into(), "a.b.c".into()))
        );
    }

    /// newlang-paths (pin): the degradation stays byte-for-byte — a Ruby
    /// call WITH arguments must never match (the node.rs container rule:
    /// a `call` is a path container only with a `receiver` field and NO
    /// `arguments` field, and node_at never returns an argument-carrying
    /// `call`), the 011-06 all-identifier guard rejects an argumentless
    /// outer chain whose receiver carries a `(...)` segment, a Java
    /// `method_invocation` stays the bare member, and Scheme (no path
    /// syntax at all) is untouched.
    #[test]
    fn symbol_at_point_java_csharp_ruby_degradation_stays_bare() {
        // Ruby: `a.b(1).c` — the OUTER call is argumentless (its
        // receiver is `a.b(1)`), so node_at returns the whole chain as
        // one `call`; the 011-06 all-identifier-segment guard then
        // rejects it (`b(1)` is not a bare identifier) → bare `c`,
        // byte-for-byte (c@7).
        assert_eq!(
            satp(LanguageId::Ruby, "a.b(1).c", 7),
            Some(("c".into(), "c".into()))
        );
        // Ruby: the argument-carrying segment ITSELF — `a.b(1)` at `b`
        // (b@2) and at its receiver `a` (a@0): node_at returns the bare
        // identifiers (an argument-carrying call is not a container), so
        // no upgrade, byte-for-byte.
        assert_eq!(
            satp(LanguageId::Ruby, "a.b(1)", 2),
            Some(("b".into(), "b".into()))
        );
        assert_eq!(
            satp(LanguageId::Ruby, "a.b(1)", 0),
            Some(("a".into(), "a".into()))
        );
        // Ruby: a bare call with arguments — `puts 1` stays the plain
        // `puts` identifier (node.rs pin `ruby_bare_call_stays_bare`).
        assert_eq!(
            satp(LanguageId::Ruby, "puts 1", 1),
            Some(("puts".into(), "puts".into()))
        );
        // Ruby: `::` inside a member chain — `Foo::Bar.new` (node_at
        // returns the whole argumentless `call`), but the `Foo::Bar`
        // segment is not a bare identifier segment → bare `new` (new@9);
        // the `Foo::Bar` half itself stays the byte-scan `::` token.
        assert_eq!(
            satp(LanguageId::Ruby, "Foo::Bar.new", 9),
            Some(("new".into(), "new".into()))
        );
        assert_eq!(
            satp(LanguageId::Ruby, "Foo::Bar.new", 6),
            Some(("Bar".into(), "Foo::Bar".into()))
        );
        // Java: `o.m(1)` at `m` — a `method_invocation` is NOT a path
        // container (node.rs pin `java_method_invocation_stays_bare`):
        // the bare `m` identifier, byte-for-byte (m@23).
        assert_eq!(
            satp(LanguageId::Java, "class A { void f() { o.m(1); } }", 23),
            Some(("m".into(), "m".into()))
        );
        // C#: `o.P` whose segment is a bare identifier still upgrades —
        // but the DEEP call `o.P().Q` (member access on an invocation)
        // carries a `()` segment → bare `Q` (Q@7).
        assert_eq!(
            satp(LanguageId::CSharp, "o.P().Q", 7),
            Some(("Q".into(), "Q".into()))
        );
        // Scheme: untouched — the flat grammar has no path container, so
        // a bare symbol stays the bare extraction (x@8).
        assert_eq!(
            satp(LanguageId::Scheme, "(define x 1)", 8),
            Some(("x".into(), "x".into()))
        );
    }

    /// 010-rung4-and-paths (item 2, app level): M-. on `p.x` in a C
    /// buffer lands via the index fall-through with the whole path — the
    /// project index has no C field symbols (the C query indexes
    /// functions / structs / macros only), so the lookup degrades to the
    /// enclosing symbol and jumps there, exactly as M-. does today;
    /// nothing new is guessed.
    #[test]
    fn xref_c_field_access_lands_via_index_fall_through() {
        let (mut s, _dir) = store_with_index(&[("c/main.c", "struct Point { int x; };\nint use_it(struct Point p) {\n    return p.x;\n}\n")]);
        s.open_path("c/main.c");
        // Line 2: "    return p.x;" — `x` at col 13.
        s.set_point(2, 13, 13);
        s.xref_find_definitions();
        // The whole path `p.x` (and the bare `x`) has no indexed
        // definition — the enclosing-symbol fall-through lands on
        // `use_it` (line 1).
        assert!(!s.picker_open(), "no picker: {}", s.message);
        assert_eq!(s.view_name_display(), "c/main.c");
        assert_eq!(s.point_line(), 1, "the enclosing function (msg: {})", s.message);
    }

    /// newlang-paths (e2e pin, Java): M-. on `A.c` in a Java buffer — the
    /// whole path `A.c` (and the bare `c`) has NO indexed definition: the
    /// Java outline indexes classes / methods only (fields are
    /// deliberately out — queries.rs), so the index fall-through lands on
    /// the enclosing method `f`, exactly like the C pin above; nothing
    /// new is guessed.
    #[test]
    fn xref_java_field_access_lands_via_index_fall_through() {
        let (mut s, _dir) = store_with_index(&[(
            "src/A.java",
            "class A {\n    int c;\n    void f() {\n        int x = A.c;\n    }\n}\n",
        )]);
        s.open_path("src/A.java");
        // Line 3 (0-based): "        int x = A.c;" — `c` at col 18.
        s.set_point(3, 18, 18);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "no picker: {}", s.message);
        assert_eq!(s.view_name_display(), "src/A.java");
        assert_eq!(
            s.point_line(),
            2,
            "the enclosing method (msg: {})",
            s.message
        );
    }

    /// newlang-paths (e2e pin, C#): M-. on `o.P` in a C# buffer — the
    /// C# outline indexes properties (queries.rs): the whole path `o.P`
    /// has no indexed symbol, but the fall-through to the last segment
    /// `P` lands on the property declaration — one candidate, direct
    /// jump (no picker).
    #[test]
    fn xref_csharp_property_access_lands_in_project_index() {
        let (mut s, _dir) = store_with_index(&[(
            "src/A.cs",
            "class A {\n    public int P { get; set; }\n    void F() {\n        int v = o.P;\n    }\n}\n",
        )]);
        s.open_path("src/A.cs");
        // Line 3 (0-based): "        int v = o.P;" — `P` at col 18.
        s.set_point(3, 18, 18);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "no picker: {}", s.message);
        assert_eq!(s.view_name_display(), "src/A.cs");
        assert_eq!(
            s.point_line(),
            1,
            "the property declaration (msg: {})",
            s.message
        );
    }

    /// newlang-paths (e2e pin, Ruby): M-. on `obj.name` in a Ruby buffer
    /// — the whole path `obj.name` (the argumentless `call` container)
    /// has no indexed symbol, but the fall-through to the last segment
    /// `name` lands on the `def name` the Ruby outline indexes — one
    /// candidate, direct jump (no picker).
    #[test]
    fn xref_ruby_method_access_lands_in_project_index() {
        let (mut s, _dir) = store_with_index(&[(
            "src/obj.rb",
            "class Obj\n  def name\n    42\n  end\nend\n\ndef show(obj)\n  puts obj.name\nend\n",
        )]);
        s.open_path("src/obj.rb");
        // Line 7 (0-based): "  puts obj.name" — `name` at col 12.
        s.set_point(7, 12, 12);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "no picker: {}", s.message);
        assert_eq!(s.view_name_display(), "src/obj.rb");
        assert_eq!(
            s.point_line(),
            1,
            "the `def name` declaration (msg: {})",
            s.message
        );
    }
