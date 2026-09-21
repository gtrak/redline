use super::*;

    #[test]
    fn jump_landing_far_down_lands_on_middle_row() {
        let (mut s, _dir) = store_with_lines(200);
        s.set_viewport_lines(21);
        // Jump DOWN from the top of the file to line 150.
        s.set_point_line(150);
        s.recenter_landing();
        assert_eq!(
            s.scroll_top(),
            140,
            "top = 150 - vp/2; a minimal scroll would leave the point on the last row (top 130)"
        );
    }

    #[test]
    fn jump_landing_far_up_lands_on_middle_row() {
        let (mut s, _dir) = store_with_lines(200);
        s.set_viewport_lines(21);
        // Scroll to the bottom, then jump UP to line 50.
        s.set_point_line(199); // minimal scroll → top 179
        assert_eq!(s.scroll_top(), 179);
        s.set_point_line(50);
        s.recenter_landing();
        assert_eq!(
            s.scroll_top(),
            40,
            "top = 50 - vp/2; a minimal scroll would leave the point on row 0 (top 50)"
        );
    }

    #[test]
    fn jump_landing_short_file_clamps_without_panic() {
        // The buffer barely exceeds the 21-row viewport (the last line is
        // the trailing empty line rope counts after the final newline).
        let (mut s, _dir) = store_with_lines(25);
        s.set_viewport_lines(21);
        s.set_point_line(0);
        s.recenter_landing();
        assert_eq!(s.scroll_top(), 0, "clamped at the top");
        let max_scroll = s.current_line_count() - 21;
        s.set_point_line(s.current_line_count() - 2); // last content line
        s.recenter_landing();
        assert_eq!(
            s.scroll_top(),
            max_scroll,
            "clamped at max_scroll (a middle-row landing is unreachable there)"
        );
        // A buffer that FITS the viewport is a no-op (nothing to recenter):
        // whole file visible, top stays 0, no panic.
        let (mut s2, _dir2) = store_with_lines(10);
        s2.set_viewport_lines(21);
        s2.set_point_line(9);
        s2.recenter_landing();
        assert_eq!(s2.scroll_top(), 0, "fits the viewport: top stays 0");
    }

    #[test]
    fn jump_landing_recenters_even_when_point_is_visible() {
        // emacs recenter repositions the window unconditionally — a jump
        // to a line already in view still lands it on the middle row.
        let (mut s, _dir) = store_with_lines(200);
        s.set_viewport_lines(21);
        s.set_scroll_top(5);
        s.set_point_line(12); // visible at row 7
        s.recenter_landing();
        assert_eq!(
            s.scroll_top(),
            2,
            "window repositioned to the middle row; minimal scroll would keep top 5"
        );
    }

    #[test]
    fn jump_landing_does_not_perturb_recenter_cycle() {
        // `recenter_landing` ITSELF does not touch the cycle: it must not
        // advance it (a jump is not a `C-l`) and must not rely on resetting
        // it. NOTE: in production every key dispatches through `dispatch()`,
        // which resets `recenter_cycle` for any command that is not literally
        // `recenter` (the pre-existing 05c `recenter-last-op` behavior, which
        // matches emacs keying `recenter-top-bottom` off `last-command`) — so
        // this test drives the helpers directly, below `dispatch`, to isolate
        // the helper's own contract.
        let (mut s, _dir) = store_with_lines(200);
        s.set_viewport_lines(21);
        s.set_point_line(50);
        s.recenter(); // fresh → middle: top 40, cycle 1
        assert_eq!(s.scroll_top(), 40);
        s.recenter(); // top: top 50, cycle 2
        assert_eq!(s.scroll_top(), 50);
        // A jump landing in between:
        s.set_point_line(150);
        s.recenter_landing();
        assert_eq!(s.scroll_top(), 140);
        assert_eq!(
            s.recenter_cycle, 2,
            "the jump neither reset nor advanced the cycle"
        );
        s.recenter(); // the cycle continues where it left off: bottom row
        assert_eq!(
            s.scroll_top(),
            130,
            "3rd C-l is the BOTTOM position (150 - (vp-1)); a reset cycle would give middle (140)"
        );
    }

    // ── plan 004 row 11: position display ─────────────────────────────

    #[test]
    fn jump_stack_back_forward_round_trip() {
        let mut stack = JumpStack::default();
        let p0 = jump_entry("a.rs", 0);
        let d1 = jump_entry("b.rs", 10);
        let d2 = jump_entry("c.rs", 20);

        // First jump: P0 → D1.
        stack.record_jump(&p0, &d1);
        assert_eq!(stack.len(), 2);
        assert_eq!(stack.back().unwrap().line, 0, "back: P0");
        assert_eq!(stack.forward().unwrap().line, 10, "forward: D1");

        // Second jump: D1 → D2 (from D1, which is the current position).
        // We need to simulate being at D1: pos is now 1 (after forward).
        // record_jump truncates to pos+1 = 2, so it keeps [P0, D1] and adds D2.
        stack.record_jump(&d1, &d2);
        assert_eq!(stack.len(), 3);

        // Back twice: D1, then P0.
        assert_eq!(stack.back().unwrap().line, 10, "back: D1");
        assert_eq!(stack.back().unwrap().line, 0, "back: P0");

        // Forward twice: D1, then D2.
        assert_eq!(stack.forward().unwrap().line, 10, "forward: D1");
        assert_eq!(stack.forward().unwrap().line, 20, "forward: D2");

        // At the end: forward is None.
        assert!(stack.forward().is_none());
    }

    #[test]
    fn jump_stack_back_at_start_returns_none() {
        let mut stack = JumpStack::default();
        let p0 = jump_entry("a.rs", 0);
        let d1 = jump_entry("b.rs", 10);
        stack.record_jump(&p0, &d1);
        // Go back to the start.
        stack.back();
        assert!(stack.back().is_none(), "no further back");
    }

    #[test]
    fn jump_stack_new_jump_truncates_forward_history() {
        let mut stack = JumpStack::default();
        let p0 = jump_entry("a.rs", 0);
        let d1 = jump_entry("b.rs", 10);
        let d2 = jump_entry("c.rs", 20);
        let d3 = jump_entry("d.rs", 30);

        // P0 → D1 → D2.
        stack.record_jump(&p0, &d1);
        // Now at D1 (pos=1). Jump D1 → D2.
        stack.record_jump(&d1, &d2);
        assert_eq!(stack.len(), 3);

        // Go back to D1 (pos=1).
        stack.back();
        // New jump from D1: D1 → D3. Truncates D2 from forward history.
        stack.record_jump(&d1, &d3);
        assert_eq!(stack.len(), 3, "D2 was truncated: [P0, D1, D3]");

        // Back: D1, then P0.
        assert_eq!(stack.back().unwrap().line, 10);
        assert_eq!(stack.back().unwrap().line, 0);
        // Forward: D1, then D3 (not D2).
        assert_eq!(stack.forward().unwrap().line, 10);
        assert_eq!(stack.forward().unwrap().line, 30);
    }

    // ── issue 05: xref tests (finding #2) ─────────────────────────────

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
        let rows: Vec<(String, String)> = s
            .picker_filtered()
            .iter()
            .map(|(c, _)| (c.name.clone(), c.display.clone()))
            .collect();
        let foo = rows.iter().find(|(n, _)| n == "Foo:1").unwrap();
        assert_eq!(foo.1, "Foo  [type]", "the struct stays at top level: {rows:?}");
        let new = rows.iter().find(|(n, _)| n == "new:3").unwrap();
        assert_eq!(
            new.1,
            "  new  [fn]",
            "the impl method is indented one level under the struct: {rows:?}"
        );
        let free = rows.iter().find(|(n, _)| n == "free:5").unwrap();
        assert_eq!(free.1, "free  [fn]", "a free fn stays flat: {rows:?}");

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

    /// 011-06 × 011-02 interplay (pin): a dotted token resolves on its OWN
    /// path — the import walks are only for BARE symbols, so any
    /// separator-containing symbol carries NO hint (the provider's own
    /// dotted machinery does the work; no double-application).
    #[test]
    fn resolver_scope_dotted_symbols_carry_no_import_hint() {
        let psrc = "import json\n\njson.dumps('x')\n";
        let pbyte = psrc.find("dumps").expect("fixture");
        assert!(
            AppStore::python_scope_for(psrc, pbyte, "json.dumps").is_empty(),
            "a dotted python symbol never gets an import-walk hint"
        );
        let gsrc = "import \"fmt\"\n\nfunc main() {\n\tfmt.Println(1)\n}\n";
        let gbyte = gsrc.find("Println").expect("fixture");
        assert!(
            AppStore::go_scope_for(gsrc, gbyte, "fmt.Println").is_empty(),
            "a dotted go symbol never gets a dot-import hint"
        );
    }

    /// 011-06 (discriminating, app level): M-. on `json.dumps` in a python
    /// buffer — the resolver context carries the WHOLE dotted path (the
    /// no-runtime miss message echoes the exact token fed; pre-011-06 it
    /// would have said the bare `dumps`).
    #[test]
    fn xref_python_dotted_token_reaches_resolver() {
        let (mut s, _dir) = store_with_index(&[
            ("main.py", "import json\n\njson.dumps(x)\n"),
        ]);
        s.open_path("main.py");
        s.set_point(2, 7, 7); // cursor inside `dumps` on line 3
        s.xref_find_definitions();
        assert!(
            s.message.contains("no provider resolution for `json.dumps`"),
            "the resolver got the dotted path, got: {}", s.message
        );
        assert_eq!(s.resolve_generation, 2, "fall-through fired exactly once");
    }

    /// 011-06 (discriminating, app level): M-. on `fakelib.apply` in a js
    /// buffer carries the dotted path to the resolver (pre-011-06 the
    /// `::`-only extraction fed the bare `apply`).
    #[test]
    fn xref_js_dotted_token_reaches_resolver() {
        let (mut s, _dir) = store_with_index(&[
            ("main.js", "import * as fakelib from \"fakelib\";\n\nfakelib.apply(5);\n"),
        ]);
        s.open_path("main.js");
        s.set_point(2, 10, 10); // cursor inside `apply` on line 3
        s.xref_find_definitions();
        assert!(
            s.message.contains("no provider resolution for `fakelib.apply`"),
            "the resolver got the dotted path, got: {}", s.message
        );
        assert_eq!(s.resolve_generation, 2, "fall-through fired exactly once");
    }

    #[test]
    fn symbol_at_point_boundaries_and_garbage() {
        // Rust: the byte-for-byte boundary behavior (011-06 kept it).
        assert_eq!(
            satp(LanguageId::Rust, "fn main() {}", 0),
            Some(("fn".into(), "fn".into()))
        );
        // Punctuation (the `(` after a name): the run before the point.
        assert_eq!(
            satp(LanguageId::Rust, "call(1)", 4),
            Some(("call".into(), "call".into()))
        );
        // Whitespace with nothing identifier-ish around: no symbol.
        assert_eq!(satp(LanguageId::Rust, "let a = 1;", 7), None);
        assert_eq!(satp(LanguageId::Rust, "{ ", 1), None);
        // A leading `::` does not extend past itself (empty path segment).
        assert_eq!(
            satp(LanguageId::Rust, "::inner", 3),
            Some(("inner".into(), "inner".into()))
        );
        // Column at the very end of the line: the trailing run counts.
        assert_eq!(
            satp(LanguageId::Rust, "let x = 1;", 9),
            Some(("1".into(), "1".into()))
        );
        // Mid-line identifier.
        assert_eq!(
            satp(LanguageId::Rust, "let x = 1;", 4),
            Some(("x".into(), "x".into()))
        );
        // Deep path: `a::b::c` under the middle segment.
        assert_eq!(
            satp(LanguageId::Rust, "use a::b::c;", 7),
            Some(("b".into(), "a::b::c".into()))
        );
        // 006-02b item 4: the cursor on the SECOND colon of a `::`
        // separator counts as the end of the preceding segment (the first
        // colon already did, via the "run before the point" rule).
        assert_eq!(
            satp(LanguageId::Rust, "a::b", 1),
            Some(("a".into(), "a::b".into()))
        );
        assert_eq!(
            satp(LanguageId::Rust, "a::b", 2),
            Some(("a".into(), "a::b".into()))
        );
        assert_eq!(
            satp(LanguageId::Rust, "use a::b::c;", 6),
            Some(("a".into(), "a::b::c".into()))
        );
    }

    #[test]
    fn xref_workspace_miss_fires_resolver_fallthrough() {
        // `tokio::spawn` at top level: no index definition, no enclosing
        // symbol → the M-. workspace miss falls through to the resolver.
        // Plain (no runtime) unit test: the spawn is skipped, the
        // generation is bumped, and the miss is reported synchronously.
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "tokio::spawn(f);\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 2, 2); // cursor inside `tokio`
        s.xref_find_definitions();
        // 006-02b item 2: the xref-entry supersede bump + the
        // start_symbol_resolution bump — two bumps for a fall-through.
        assert_eq!(s.resolve_generation, 2, "fall-through fired exactly once");
        assert!(
            s.resolving_display().is_empty(),
            "no runtime: the indicator must not hang"
        );
        assert!(
            s.message.contains("no provider resolution for `tokio::spawn`"),
            "graceful miss message, got: {}", s.message
        );
    }

    #[test]
    fn xref_workspace_hit_and_enclosing_hit_skip_resolver() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() { target(); }\n"),
            ("src/lib.rs", "pub fn target() {}\n"),
        ]);
        s.open_path("src/main.rs");
        // Direct hit under the point → no fall-through.
        s.set_point(0, 12, 12);
        s.xref_find_definitions();
        assert_eq!(s.resolve_generation, 1, "direct hit: one supersede bump, no job");
        // Enclosing hit → no fall-through either (a second supersede bump).
        s.open_path("src/lib.rs");
        s.set_point(0, 0, 0);
        s.xref_find_definitions();
        assert_eq!(s.resolve_generation, 2, "enclosing hit: one supersede bump, no job");
    }

    /// (011-01) The context language follows the buffer's extension — the
    /// lowercase registry name the providers' `languages()` expect. An
    /// unknown extension stays `None` (the chain keeps the pre-dispatch
    /// in-order walk for those).
    #[test]
    fn resolution_language_maps_buffer_extensions() {
        let (s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        assert_eq!(s.resolution_language("main.py"), Some("python".into()));
        assert_eq!(s.resolution_language("src/lib.js"), Some("javascript".into()));
        assert_eq!(s.resolution_language("a.ts"), Some("typescript".into()));
        assert_eq!(s.resolution_language("a.tsx"), Some("tsx".into()));
        // .jsx/.mjs/.cjs map to JavaScript (registry.rs has no Jsx variant):
        // a JSX buffer must dispatch to the js provider, never silently fall
        // back to the pre-dispatch walk (011-01 review P2-3).
        assert_eq!(s.resolution_language("a.jsx"), Some("javascript".into()));
        assert_eq!(s.resolution_language("a.mjs"), Some("javascript".into()));
        assert_eq!(s.resolution_language("a.cjs"), Some("javascript".into()));
        // A registry language with no provider maps to its name (an honest
        // zero-eligible bail) rather than falling back to cargo.
        assert_eq!(s.resolution_language("a.c"), Some("c".into()));
        assert_eq!(s.resolution_language("main.go"), Some("go".into()));
        // A Rust buffer still carries "rust" (the existing path, unchanged).
        assert_eq!(s.resolution_language("src/main.rs"), Some("rust".into()));
        // Unknown extension: no language, pre-dispatch behavior.
        assert_eq!(s.resolution_language("notes.txt"), None);
    }

    /// (011-01) A workspace miss in a NON-RUST buffer fires the resolver
    /// fall-through (the app wiring is language-agnostic; the provider
    /// itself is selected by `SymbolContext.language` inside the chain).
    /// Plain (no runtime) unit test: synchronous graceful miss.
    #[test]
    fn python_buffer_miss_fires_resolver_fallthrough() {
        let (mut s, _dir) = store_with_index(&[
            ("main.py", "import os\nx = os.path.join('a', 'b')\n"),
        ]);
        s.open_path("main.py");
        s.start_symbol_resolution("os.path.join", "main.py");
        assert_eq!(s.resolve_generation, 1, "the fall-through fired exactly once");
        assert!(s.resolving_display().is_empty());
        // 011-01 review P2-1: the pre-fix assertion was non-discriminating —
        // the no-runtime fast path shares this message prefix. NOTE the two
        // paths cannot both be asserted here: `start_symbol_resolution` checks
        // for a background runtime BEFORE dispatching, and in a unit test
        // there is none, so the no-runtime branch IS the reachable path here
        // (the real chain + dispatch run off the input path under tokio).
        // What this test CAN pin, and what discriminates dispatch: the
        // language is mapped from the buffer's extension at the context seam
        // (asserted in `resolution_language_maps_buffer_extensions`), and a
        // REGRESSION that broke dispatch would be visible there, not here.
        // So assert the runtime seam explicitly instead of the two prefixes
        // being interchangeable.
        assert!(
            s.message.contains("no background runtime"),
            "unit tests have no runtime, so the fast path is expected: {msg}",
            msg = s.message
        );
        assert!(
            s.message.contains("no provider resolution for `os.path.join`"),
            "graceful miss message, got: {}", s.message
        );
    }

    // ── 007-03: the resolver fall-through's scope hint ─────────────────

    /// The fall-through context carries the `use` path for a bare symbol.
    #[test]
    fn resolver_scope_carries_use_path_for_bare_symbol() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "use serde::Deserialize;\nfn main() { let _d: Deserialize = D; }\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(1, 15, 15); // inside the BARE `Deserialize`
        assert_eq!(
            s.resolver_scope("Deserialize"),
            vec!["serde".to_string(), "Deserialize".to_string()]
        );
    }

    /// `use x as y` (alias): the context carries the ALIASED (original)
    /// path, so `y` resolves to the original item.
    #[test]
    fn resolver_scope_alias_carries_original_path() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "use serde::Deserialize as D;\nfn main() { let _d: D = D::default(); }\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(1, 21, 21); // inside the first (bare) `D`
        assert_eq!(
            s.resolver_scope("D"),
            vec!["serde".to_string(), "Deserialize".to_string()]
        );
    }

    /// A bare symbol with NO `use` declaration in scope: the scope stays
    /// EMPTY (the providers keep their byte-for-byte no-hint behavior —
    /// std/prelude names are never guessed).
    #[test]
    fn resolver_scope_empty_without_use() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() { let x = 9; }\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 18, 18); // inside `x` (a let binding, no import)
        assert!(s.resolver_scope("x").is_empty());
    }

    /// A path-shaped symbol carries the ENCLOSING scope (007-01's
    /// `scope_path_at`), not an import.
    #[test]
    fn resolver_scope_path_symbol_carries_enclosing_scope() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() { tokio::spawn(f); }\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 15, 15); // inside `tokio`
        assert_eq!(s.resolver_scope("tokio::spawn"), vec!["main".to_string()]);
    }

    /// Group imports (`use serde::{…}`): the alias entry and the plain
    /// entry both carry their original paths.
    #[test]
    fn resolver_scope_group_imports() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "use serde::{Deserialize as D, Serialize};\nfn main() {}\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 26, 26); // inside the alias `D`
        assert_eq!(
            s.resolver_scope("D"),
            vec!["serde".to_string(), "Deserialize".to_string()]
        );
        s.set_point(0, 32, 32); // inside `Serialize`
        assert_eq!(
            s.resolver_scope("Serialize"),
            vec!["serde".to_string(), "Serialize".to_string()]
        );
    }

    /// Innermost module wins: a `use` in the enclosing `mod` shadows the
    /// top-level one (Rust's module scoping).
    #[test]
    fn resolver_scope_innermost_use_wins() {
        let (mut s, _dir) = store_with_index(&[
            (
                "src/main.rs",
                "use a::Thing;\nmod inner {\n    use b::Thing;\n    fn f() {}\n}\n",
            ),
        ]);
        s.open_path("src/main.rs");
        s.set_point(2, 12, 12); // inside `Thing` of `use b::Thing;`
        assert_eq!(s.resolver_scope("Thing"), vec!["b".to_string(), "Thing".to_string()]);
    }

    /// Non-Rust buffers degrade to an empty scope (007-01's layer is
    /// Rust-only; the providers keep their no-hint behavior).
    #[test]
    fn resolver_scope_non_rust_buffer_is_empty() {
        let (mut s, _dir) = store_with_index(&[
            ("main.py", "import json\nprint(json)\n"),
        ]);
        s.open_path("main.py");
        s.set_point(0, 7, 7); // inside `json`
        assert!(s.resolver_scope("json").is_empty());
    }

    /// `crate::`-prefixed imports never name an external crate: no hint
    /// (the bare symbol keeps the providers' no-hint behavior). A plain
    /// same-crate `use inner::Thing;` DOES carry its two-segment hint —
    /// it is the user's own import path, not a guess; the provider then
    /// bails honestly ("crate `inner` is not in the cargo graph") when no
    /// package bears that name.
    #[test]
    fn resolver_scope_crate_prefix_is_not_guessed() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "use crate::Thing;\nfn main() {}\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 9, 9); // inside `Thing` of `use crate::Thing;`
        assert!(s.resolver_scope("Thing").is_empty(), "crate:: prefix never hints");
    }

    /// 007-03 review P2: the negative import shapes must all yield an EMPTY
    /// hint (a miss, never a guessed crate). Pins glob, single-segment, and
    /// the `self`/`super` prefixes.
    #[test]
    fn resolver_scope_negative_import_shapes_are_not_guessed() {
        for (name, src, symbol) in [
            (
                "glob",
                "use serde::*;\nfn main() { let _ = Deserialize; }\n",
                "Deserialize",
            ),
            (
                "single-segment",
                "use serde;\nfn main() { let _ = serde; }\n",
                "serde",
            ),
            (
                "self-prefix",
                "use self::Thing;\nfn main() { let _ = Thing; }\n",
                "Thing",
            ),
            (
                "super-prefix",
                "mod m { use super::Thing; fn f() { let _ = Thing; } }\n",
                "Thing",
            ),
        ] {
            let (mut s, _dir) = store_with_index(&[("src/main.rs", src)]);
            s.open_path("src/main.rs");
            // Point at the USE in code (the last occurrence), not the import.
            let at = src.rfind(symbol).expect("fixture symbol");
            let line = src[..at].matches('\n').count();
            let col = at - src[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
            s.set_point(line, col, col);
            assert!(
                s.resolver_scope(symbol).is_empty(),
                "{name}: a non-resolvable import must never hint"
            );
        }
    }

    /// 007-03 review P2: a non-resolvable entry in a group must SKIP, not
    /// abort the group — `use a::b::{self, c};` still resolves bare `c`.
    #[test]
    fn resolver_scope_group_skips_bad_entries() {
        let src = "use serde::{self as s2, Deserialize};\nfn main() {}\n";
        let (mut s, _dir) = store_with_index(&[("src/main.rs", src)]);
        s.open_path("src/main.rs");
        let at = src.find("Deserialize").expect("fixture symbol");
        s.set_point(0, at, at);
        assert_eq!(
            s.resolver_scope("Deserialize"),
            vec!["serde".to_string(), "Deserialize".to_string()],
            "a preceding bad group entry must not discard a later good one"
        );
    }

    #[tokio::test]
    async fn xref_resolver_hit_lands_read_only_jump() {
        // A resolve event with a resolved source (outside the project root,
        // external) lands as a jump: read-only buffer, point on the
        // resolved line, jump entry recorded, status cleared, and the
        // external path never enters the project recents.
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "tokio::spawn(f);\n"),
        ]);
        // An "external" source file outside the project root.
        let ext = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(ext.path(), "extern crate dep;\npub fn spawn<F>(f: F) {}\n").unwrap();
        let root = s.project.as_ref().map(|p| p.root.to_string_lossy().into_owned()).unwrap();
        s.open_path("src/main.rs");
        s.set_point(0, 2, 2);
        let origin_key = s.buffers.current().map(String::from).unwrap();
        let recents_before = s.project_store.recents.list(&root).to_vec();
        s.xref_find_definitions();
        assert_eq!(s.resolve_generation, 2, "xref supersede bump + start bump");
        assert!(s.resolving_display().contains("`tokio::spawn`"), "status activity while pending");
        // Land a fabricated (provider-shaped) hit for the in-flight job.
        let event = ResolveEvent {
            generation: 2,
            symbol: "tokio::spawn".into(),
            source: Some(ResolvedSource {
                file: ext.path().to_path_buf(),
                source_root: ext.path().parent().unwrap().to_path_buf(),
                external: true,
                line: Some(2),
            }),
            error: None,
        };
        s.apply_resolve_event(&event);
        let key = s.buffers.current().map(String::from).unwrap();
        assert_eq!(key, ext.path().to_string_lossy().into_owned(), "external buffer is current");
        let buf = s.buffers.get(&key).unwrap();
        assert!(!buf.editable, "external source is read-only");
        assert_eq!(s.point_line(), 1, "point on the resolved (1-based line 2) definition");
        assert!(s.message.contains("jumped to"), "jump report, got: {}", s.message);
        assert!(s.resolving_display().is_empty(), "status activity cleared");
        assert_eq!(
            s.project_store.recents.list(&root).to_vec(),
            recents_before,
            "external landing never records a recent"
        );
        // Jump entry recorded: back lands on the origin buffer/line.
        s.jump_back();
        assert_eq!(s.buffers.current().map(String::from).unwrap(), origin_key, "jump-back returns to the origin");
    }

    #[tokio::test]
    async fn xref_resolver_miss_reports_graceful_message() {
        // All providers miss → the event carries the error; the message
        // reports it and nothing else changes (no buffer, no jump, no panic).
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "tokio::spawn(f);\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 2, 2);
        let before = s.buffers.current().map(String::from).unwrap();
        s.xref_find_definitions();
        let event = ResolveEvent {
            generation: 2,
            symbol: "tokio::spawn".into(),
            source: None,
            error: Some("no tooling provider could resolve symbol `tokio::spawn` (tried 1 provider(s): rust): no crate `tokio`".into()),
        };
        s.apply_resolve_event(&event);
        assert!(
            s.message.starts_with("no provider resolution for `tokio::spawn`:"),
            "got: {}", s.message
        );
        assert_eq!(s.buffers.current().map(String::from).unwrap(), before, "view unchanged on a miss");
        assert!(s.resolving_display().is_empty());
    }

    #[tokio::test]
    async fn xref_resolver_event_end_to_end_miss_without_cargo() {
        // Real fall-through end to end (no network): a project WITHOUT a
        // Cargo.toml (`.projectile` is a root marker, not one) makes the
        // cargo provider fail fast, and the event published on the
        // ResolveBus lands the graceful report.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join(".projectile"), "\n").unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "tokio::spawn(f);\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        let mut rx = s.resolve_bus.subscribe();
        s.open_path("src/main.rs");
        s.set_point(0, 2, 2);
        s.xref_find_definitions();
        assert_eq!(s.resolve_generation, 2, "fall-through started (xref bump + start bump)");
        let event = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("resolve event published within 30s");
        assert!(event.is_ok());
        let event = rx.borrow_and_update().clone();
        assert_eq!(event.generation, 2);
        assert!(event.source.is_none(), "miss: no source, got error: {:?}", event.error);
        s.apply_resolve_event(&event);
        assert!(
            s.message.starts_with("no provider resolution for `tokio::spawn`"),
            "got: {}", s.message
        );
        assert!(
            s.message.contains("tried 1 provider(s): rust"),
            "provider chain named in the report: {}", s.message
        );
        assert!(s.resolving_display().is_empty());
    }

    #[tokio::test]
    async fn xref_resolver_fallthrough_carries_use_scope_hint() {
        // 007-03 end to end: a BARE symbol imported via `use` carries the
        // use-path into the `SymbolContext`, so the cargo provider lands it
        // in the (locally cached) serde registry source instead of the
        // bare-symbol bail. Guard: the ambient `~/.cargo` must have a serde
        // registry source (this dev box does; the resolver crate's own
        // integration tests assume the same warm cache).
        let registry_src = std::path::PathBuf::from(
            std::env::var("CARGO_HOME")
                .unwrap_or_else(|_| format!("{}/.cargo", std::env::var("HOME").unwrap_or_default())),
        )
        .join("registry/src");
        // Discover a CACHED plain `serde-<semver>` registry source (not
        // serde_* / serde-untagged) and pin the dependency to that exact
        // version so `cargo metadata` never needs a fresh index fetch.
        let mut serde_dir: Option<std::path::PathBuf> = None;
        let mut serde_version: Option<String> = None;
        // The registry source dirs live under `registry/src/<index-hash>/`.
        let index_dirs: Vec<std::path::PathBuf> = std::fs::read_dir(&registry_src)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        for index_dir in &index_dirs {
            let Ok(entries) = std::fs::read_dir(index_dir) else { continue };
            for entry in entries.flatten() {
                let name = match entry.file_name().to_str() {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                let Some(v) = name.strip_prefix("serde-") else {
                    continue;
                };
                if !v.split('.').next().is_some_and(|c| c.chars().all(|c| c.is_ascii_digit())) {
                    continue; // serde-untagged, …
                }
                if !entry.path().join("src").is_dir() {
                    continue;
                }
                let better = match &serde_version {
                    None => true,
                    Some(cur) => {
                        let key = |s: &str| {
                            s.split('.')
                                .map(|p| {
                                    p.chars()
                                        .take_while(|c| c.is_ascii_digit())
                                        .collect::<String>()
                                })
                                .map(|p| p.parse::<u64>().unwrap_or(0))
                                .collect::<Vec<_>>()
                        };
                        key(v) > key(cur)
                    }
                };
                if better {
                    serde_dir = Some(entry.path());
                    serde_version = Some(v.to_string());
                }
            }
            break; // one index-hash dir per CARGO_HOME
        }
        let Some(serde_dir) = serde_dir else { return; };
        let serde_version = serde_version.expect("set with the dir");
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            format!(
                "[package]\nname = \"xscope\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
                 [dependencies]\nserde = \"={serde_version}\"\n"
            ),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "use serde::Deserialize;\nfn main() {}\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        let mut rx = s.resolve_bus.subscribe();
        s.open_path("src/main.rs");
        s.set_point(0, 9, 9); // on `Deserialize` (bare, use-imported)
        s.start_symbol_resolution("Deserialize", "src/main.rs");
        let _ = tokio::time::timeout(std::time::Duration::from_secs(120), rx.changed())
            .await
            .expect("resolve event published within 120s");
        let event = rx.borrow_and_update().clone();
        let source = event
            .source
            .expect("the bare use-imported symbol resolves via the scope hint");
        assert!(source.external, "the serde registry source is external");
        assert!(
            source.file.starts_with(&serde_dir),
            "landed in the serde registry source: {:?}",
            source.file
        );
        let file_name = source.file.to_string_lossy().into_owned();
        assert!(
            file_name.contains("serde-"),
            "file in a serde-<version> dir: {file_name}"
        );

        // Degradation pin at the same seam: a BARE symbol with NO `use`
        // keeps today's byte-for-byte "needs scope info" behavior — the
        // provider never gets a hint, so the whole chain reports a miss
        // naming the symbol (the bare bail fires before any cargo work).
        s.start_symbol_resolution("plain_local_name", "src/main.rs");
        let _ = tokio::time::timeout(std::time::Duration::from_secs(60), rx.changed())
            .await
            .expect("second resolve event published within 60s");
        let event = rx.borrow_and_update().clone();
        let err = event.error.expect("a miss (no use declares the name)");
        assert!(
            err.contains("no tooling provider could resolve symbol `plain_local_name`"),
            "no hint → the chain's miss report: {err}"
        );
        assert!(
            !err.contains("jumped"),
            "a bare unimported symbol never resolves: {err}"
        );
    }

    /// 011-08 fix-jsrel P2-7 live leg: with the app now EMITTING the
    /// relative hint, M-. on a relative use site lands in the SIBLING
    /// file through the REAL provider chain: workspace-local
    /// (`external = false`) and opened through `open_resolved_source`'s
    /// project branch — an EDITABLE project buffer, never the read-only
    /// external path. (The provider-side twin goldens are the js corpus'
    /// `relative-*` probes.)
    #[tokio::test]
    async fn xref_relative_import_lands_in_sibling_file_editable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        // The project marker (project detection is marker-driven).
        std::fs::write(dir.path().join("package.json"), "{}\n").unwrap();
        let src = "import { legacyJoin } from \"./legacy-util\";\nfunction f() { legacyJoin(); }\n";
        std::fs::write(dir.path().join("src/app.js"), src).unwrap();
        std::fs::write(
            dir.path().join("src/legacy-util.js"),
            "function legacyJoin() {}\nmodule.exports = { legacyJoin };\n",
        )
        .unwrap();
        let sibling = std::fs::canonicalize(dir.path().join("src/legacy-util.js")).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        let mut rx = s.resolve_bus.subscribe();
        s.open_path("src/app.js");
        let at = src.rfind("legacyJoin").expect("fixture");
        let line = src[..at].matches('\n').count();
        let col = at - src[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
        s.set_point(line, col, col);
        s.start_symbol_resolution("legacyJoin", "src/app.js");
        let _ = tokio::time::timeout(std::time::Duration::from_secs(60), rx.changed())
            .await
            .expect("resolve event published within 60s");
        let event = rx.borrow_and_update().clone();
        let source = event
            .source
            .as_ref()
            .expect("the relative use site resolves (the app emits the hint)");
        assert!(!source.external, "the sibling file is workspace-local (external = false)");
        assert_eq!(source.file, sibling, "landed in the sibling file");
        assert_eq!(
            source.line,
            Some(1),
            "the member definition line (1-based)")
        ;
        s.apply_resolve_event(&event);
        assert_eq!(
            s.view_name_display(),
            "src/legacy-util.js",
            "the view is now the sibling file"
        );
        let cur: &str = s.buffers.current().expect("a current buffer");
        assert!(
            !s.external_buffers.contains(cur),
            "the landed sibling is an EDITABLE project buffer (the project branch of open_resolved_source), never the read-only external one"
        );
    }

    #[test]
    fn xref_resolver_stale_generation_event_discarded() {
        // An event from a superseded request (or a previous project) must be
        // discarded: no jump, no message — and (006-02b item 3) it CLEARS
        // the resolving indicator, because the latest-wins drain means this
        // stale send may have overwritten the current job's event in the
        // channel; a stuck indicator would otherwise persist until the next
        // action.
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "tokio::spawn(f);\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 2, 2);
        s.xref_find_definitions();
        assert_eq!(s.resolve_generation, 2, "xref bump + start bump");
        // Simulate a superseded request bumping the generation (as a second
        // M-. that started its own job would), then deliver the OLD job's
        // event.
        s.resolve_generation += 1;
        s.resolving = Some(("resolving `other`…".into(), 3));
        let before = s.message.clone();
        let stale = ResolveEvent {
            generation: 2,
            symbol: "tokio::spawn".into(),
            source: None,
            error: Some("boom".into()),
        };
        s.apply_resolve_event(&stale);
        assert_eq!(s.message, before, "stale event changed no reported state");
        assert!(
            s.resolving_display().is_empty(),
            "a stale event clears the resolving indicator (latest-wins: the current job's event may have been overwritten)"
        );
        // And the stale job's own activity text (gen 2) is hidden by the
        // display gate once the generation no longer matches.
        s.resolving = Some(("resolving `tokio::spawn`…".into(), 2));
        assert!(s.resolving_display().is_empty(), "stale-generation activity hidden by the display gate");
    }

    #[tokio::test]
    async fn xref_workspace_hit_supersedes_in_flight_resolve() {
        // 006-02b item 2: with a resolve in flight, a successful M-. 
        // workspace hit bumps the generation, so the in-flight job's stale
        // event (even a registry-source HIT) discards itself and never
        // opens the external source or records a jump.
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "tokio::spawn(f);\ntarget();\n"),
            ("src/lib.rs", "pub fn target() {}\n"),
        ]);
        let ext = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(ext.path(), "pub fn spawn<F>(f: F) {}\n").unwrap();
        s.open_path("src/main.rs");
        s.set_point(0, 2, 2); // cursor inside `tokio` → workspace miss
        s.xref_find_definitions();
        let in_flight = s.resolve_generation;
        assert!(!s.resolving_display().is_empty(), "resolver in flight");
        // The user's NEXT M-. is a workspace hit (line 1: `target();`).
        s.set_point_line(1);
        s.xref_find_definitions();
        assert_eq!(s.resolve_generation, in_flight + 1, "the hit superseded the in-flight resolve");
        assert_eq!(s.view_name_display(), "src/lib.rs", "the hit landed");
        let before = s.view_name_display();
        // Now the stale job's event (a registry-source hit) arrives.
        let stale = ResolveEvent {
            generation: in_flight,
            symbol: "tokio::spawn".into(),
            source: Some(ResolvedSource {
                file: ext.path().to_path_buf(),
                source_root: ext.path().parent().unwrap().to_path_buf(),
                external: true,
                line: Some(1),
            }),
            error: None,
        };
        s.apply_resolve_event(&stale);
        assert_eq!(s.view_name_display(), before, "the stale hit did not open the registry source");
    }

    #[tokio::test]
    async fn xref_resolver_line_zero_lands_at_top() {
        // 006-02b item 6: a provider emitting line 0 is treated as "no
        // line" — the landing is the top of the file (no underflow).
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "tokio::spawn(f);\n"),
        ]);
        let ext = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(ext.path(), "extern crate dep;\npub fn spawn<F>(f: F) {}\n").unwrap();
        s.open_path("src/main.rs");
        s.set_point(0, 2, 2);
        s.xref_find_definitions();
        let event = ResolveEvent {
            generation: s.resolve_generation,
            symbol: "tokio::spawn".into(),
            source: Some(ResolvedSource {
                file: ext.path().to_path_buf(),
                source_root: ext.path().parent().unwrap().to_path_buf(),
                external: true,
                line: Some(0),
            }),
            error: None,
        };
        s.apply_resolve_event(&event);
        assert_eq!(s.point_line(), 0, "line 0 lands at the top of the file");
        assert!(s.message.contains("jumped to"), "msg: {}", s.message);
    }

    // ── plan 006 issue 03: navigate within external (crate) sources ────

    #[test]
    fn which_function_enclosing_symbol_nested() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "mod outer {\n    fn f() {\n        g()\n    }\n}\n"),
        ]);
        s.open_path("src/main.rs");
        // Line 2 ("        g()"): inside fn f, inside mod outer.
        // The innermost enclosing symbol is `f`.
        s.set_point_line(2);
        assert_eq!(s.which_function(), "f");
        // Line 0 ("mod outer {"): inside mod outer, outside fn f.
        s.set_point_line(0);
        assert_eq!(s.which_function(), "outer");
        // Line 99: outside everything.
        s.set_point_line(99);
        assert_eq!(s.which_function(), "");
    }

    // ── issue 05: apply_index_event test ──────────────────────────────

    #[test]
    fn m_dot_includes_uppercase_identifiers() {
        // The original bug this guarded: M-. skipping uppercase-initial
        // names (types/constants). Selection now goes through
        // symbol_at_point, which has no case filter: the cursor on `Foo`
        // or `BAR` yields the identifier (the `::`-path token is kept for
        // the resolver).
        let line = "let x = Foo::BAR;";
        // `Foo` starts at col 8 (cursor inside the identifier).
        assert_eq!(
            satp(LanguageId::Rust, line, 8),
            Some(("Foo".into(), "Foo::BAR".into()))
        );
        // The second `:` of the `::` separator (col 12) counts as the end
        // of the preceding segment (006-02b item 4).
        assert_eq!(
            satp(LanguageId::Rust, line, 12),
            Some(("Foo".into(), "Foo::BAR".into()))
        );
        // `BAR` starts at col 13; just after it (col 16) still counts.
        assert_eq!(
            satp(LanguageId::Rust, line, 13),
            Some(("BAR".into(), "Foo::BAR".into()))
        );
        assert_eq!(
            satp(LanguageId::Rust, line, 16),
            Some(("BAR".into(), "Foo::BAR".into()))
        );
    }

    #[test]
    fn imenu_indent_depth_reflects_enclosing_extents() {
        // The imenu outline uses the index's symbol nesting. We verify the
        // depth calculation: a symbol at depth N has N strictly-enclosing
        // extents. This is tested via the index's outline structure.
        use crate::nav::index::SymbolIndex;
        use crate::syntax::queries::{Symbol, SymbolKind};
        let mut idx = SymbolIndex::new();
        // A file with nested symbols: fn outer { struct Inner { fn method } }
        // The inner struct is depth 1 (enclosed by outer), method is depth 2.
        let rel = "test.rs".to_string();
        let outer = Symbol {
            name: "outer".into(),
            kind: SymbolKind::Function,
            line: 0,
            end_line: 99,
            start_byte: 0,
            end_byte: 5,
        };
        let inner = Symbol {
            name: "Inner".into(),
            kind: SymbolKind::Type,
            line: 5,
            end_line: 90,
            start_byte: 10,
            end_byte: 15,
        };
        let method = Symbol {
            name: "method".into(),
            kind: SymbolKind::Function,
            line: 10,
            end_line: 85,
            start_byte: 20,
            end_byte: 26,
        };
        idx.set_file(&rel, vec![outer, inner, method]);
        let outline = idx.outline(&rel);
        // The outline should show nesting: outer at depth 0, Inner at depth 1,
        // method at depth 2 (or however the outline represents depth).
        assert!(!outline.is_empty(), "outline must be non-empty");
        assert_eq!(outline.len(), 3, "all three symbols must be in the outline");
    }

    /// Discriminating: a BARE imported symbol in a JS buffer now carries
    /// the package path (007-03 did this for Rust only; pre-011-02 the
    /// hint was empty for every non-Rust buffer).
    #[test]
    fn resolver_scope_js_named_import_carries_package_path() {
        let src = "import { doThing } from \"acme\";\nfunction f() { doThing(); }\n";
        let (mut s, _dir) = store_with_index(&[("src/index.js", src)]);
        s.open_path("src/index.js");
        let at = src.rfind("doThing").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("doThing"),
            vec!["acme".to_string(), "doThing".to_string()]
        );
    }

    /// JS alias → original: `import { doThing as dt }` carries the
    /// ORIGINAL path for the bare alias.
    #[test]
    fn resolver_scope_js_alias_carries_original() {
        let src = "import { doThing as dt } from \"acme\";\ndt();\n";
        let (mut s, _dir) = store_with_index(&[("src/index.js", src)]);
        s.open_path("src/index.js");
        let at = src.rfind("dt").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("dt"),
            vec!["acme".to_string(), "doThing".to_string()]
        );
    }

    /// JS default import: the local binding carries the package + name
    /// (the provider's definition scan then pins the real line, or bails
    /// honestly when it can't — the hint is the user's import, not a
    /// guess).
    #[test]
    fn resolver_scope_js_default_import_carries_package() {
        let src = "import doThing from \"acme\";\ndoThing();\n";
        let (mut s, _dir) = store_with_index(&[("src/index.js", src)]);
        s.open_path("src/index.js");
        let at = src.rfind("doThing").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("doThing"),
            vec!["acme".to_string(), "doThing".to_string()]
        );
    }

    /// JS namespace import: bare `ns` names the package entry (`["pkg"]`);
    /// `ns.member` rewrites to the package's real path + member. An
    /// identity alias (`import * as acme from "acme"`) still carries the
    /// path — the provider's rewrite rule treats it as a no-op because the
    /// hint IS the symbol's own path.
    #[test]
    fn resolver_scope_js_namespace_import() {
        let src = "import * as ac from \"acme\";\nimport * as acme from \"acme\";\nac.doThing();\nacme.doThing();\n";
        let (mut s, _dir) = store_with_index(&[("src/index.js", src)]);
        s.open_path("src/index.js");
        // Bare namespace → the package entry.
        let at = src.find("import * as ac ").unwrap() + "import * as ac".len();
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(s.resolver_scope("ac"), vec!["acme".to_string()]);
        // `ns.member` → the package's real path + member.
        let at = src.find("ac.doThing()").expect("fixture");
        let (line, col) = point_of(src, at + 1); // inside `doThing`
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("ac.doThing"),
            vec!["acme".to_string(), "doThing".to_string()]
        );
        // Identity alias: the hint equals the symbol's own path (the
        // provider's rewrite is a no-op there — pinned at the provider).
        let at = src.find("acme.doThing()").expect("fixture");
        let (line, col) = point_of(src, at + 1);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("acme.doThing"),
            vec!["acme".to_string(), "doThing".to_string()]
        );
    }

    /// No-import / absolute / bare-`.` / bare-`..` / side-effect
    /// specifiers are NEVER guessed: the hint stays empty (the provider
    /// keeps its exact no-hint / dedicated-bail behavior — the JS
    /// provider bails dedicated on absolute specs, and a bare `.`/`..`
    /// never reaches its relative branch). Relative (`./`/`../`) specs
    /// LEFT this list in the fix-jsrel P2-7 follow-up: they hint now
    /// (pinned in
    /// `resolver_scope_js_relative_import_carries_sibling_path`).
    #[test]
    fn resolver_scope_js_no_import_absolute_and_side_effect_are_not_guessed() {
        let cases = [
            ("no import", "function f() { doThing(); }\n"),
            (
                "absolute path",
                "import { doThing } from \"/opt/acme\";\ndoThing();\n",
            ),
            (
                "bare dot (current dir)",
                "import { doThing } from \".\";\ndoThing();\n",
            ),
            (
                "bare dotdot (parent dir)",
                "import { doThing } from \"..\";\ndoThing();\n",
            ),
            (
                "side-effect only",
                "import \"acme\";\ndoThing();\n",
            ),
            (
                "relative side-effect (binds nothing)",
                "import \"./acme\";\ndoThing();\n",
            ),
        ];
        for (name, src) in cases {
            let (mut s, _dir) = store_with_index(&[("src/index.js", src)]);
            s.open_path("src/index.js");
            let at = src.rfind("doThing").expect("fixture");
            let (line, col) = point_of(src, at);
            s.set_point(line, col, col);
            assert!(
                s.resolver_scope("doThing").is_empty(),
                "{name}: no hint expected"
            );
        }
    }

    /// 011-08 follow-up (fix-jsrel P2-7): a relative specifier (`./…` /
    /// `../…`) now CARRIES its hint — item included (the
    /// `SymbolContext.scope` contract) — because the JS provider landed
    /// it: it resolves against the importing buffer's directory and lands
    /// in the sibling file, workspace-local (`external = false`). The
    /// sibling's PRESENCE is what the provider checks (a dedicated bail
    /// when the file is absent — the provider's corpus goldens); the
    /// app-side hint is the import declaration itself, never a guess.
    #[test]
    fn resolver_scope_js_relative_import_carries_sibling_path() {
        let src = "\
            import { legacyJoin } from \"./legacy-util\";
            import { joinTwo as legacyTwo } from \"../lib/legacy-util\";
            import legacyDefault from \"./legacy-default.js\";
            import { default as legacyDef2 } from \"./legacy-whole\";
            import * as legacyNs from \"./legacy-ns\";
            const { cjsJoin } = require(\"./legacy-cjs\");
            const legacyWhole = require(\"./legacy-whole\");
            legacyJoin();
            legacyTwo();
            legacyDefault();
            legacyDef2();
            legacyNs.helper();
            cjsJoin();
            legacyWhole();
            ";
        let (mut s, _dir) = store_with_index(&[
            ("src/index.js", src),
            // The siblings really ARE present (the hint does not stat the
            // disk — the provider does, and bails dedicated when absent).
            ("src/legacy-util.js", "function legacyJoin() {}\n"),
            ("lib/legacy-util.js", "function joinTwo() {}\n"),
            ("src/legacy-default.js", "export default function legacyDefault() {}\n"),
            ("src/legacy-ns.js", "export function helper() {}\n"),
            ("src/legacy-cjs.js", "function cjsJoin() {}\nmodule.exports = { cjsJoin };\n"),
            ("src/legacy-whole.js", "module.exports = {};\n"),
        ]);
        s.open_path("src/index.js");
        let mut probe = |symbol: &str| {
            let at = src.rfind(symbol).expect("fixture");
            let (line, col) = point_of(src, at);
            s.set_point(line, col, col);
            s.resolver_scope(symbol)
        };
        // Named relative → the sibling's path, item included.
        assert_eq!(
            probe("legacyJoin"),
            vec!["./legacy-util".to_string(), "legacyJoin".to_string()]
        );
        // `../` relative + alias → the ORIGINAL name.
        assert_eq!(
            probe("legacyTwo"),
            vec!["../lib/legacy-util".to_string(), "joinTwo".to_string()]
        );
        // An extension-carrying relative spec stays verbatim (the provider
        // lands the exact file — no walk).
        assert_eq!(
            probe("legacyDefault"),
            vec!["./legacy-default.js".to_string(), "legacyDefault".to_string()]
        );
        // `{ default as D }` → entry-only (the one shape that deliberately
        // drops the member — `default` names the entry itself).
        assert_eq!(probe("legacyDef2"), vec!["./legacy-whole".to_string()]);
        // Whole-module CJS binding → the specifier alone (the entry).
        assert_eq!(probe("legacyWhole"), vec!["./legacy-whole".to_string()]);
        // CJS destructuring → the sibling's path, item included.
        assert_eq!(
            probe("cjsJoin"),
            vec!["./legacy-cjs".to_string(), "cjsJoin".to_string()]
        );
        // Relative namespace import: bare `ns` names the entry…
        assert_eq!(probe("legacyNs"), vec!["./legacy-ns".to_string()]);
        // …and `ns.member` carries the entry + member (the provider's
        // relative branch takes the member from the hint's last segment).
        assert_eq!(
            probe("legacyNs.helper"),
            vec!["./legacy-ns".to_string(), "helper".to_string()]
        );
    }

    /// JS subpath specifiers keep their `/` (the provider drops the
    /// subpath to the base package); CJS `require` destructuring follows
    /// the same rules as ESM named imports.
    #[test]
    fn resolver_scope_js_subpath_and_cjs_require() {
        let src = "import { doThing } from \"acme/sub\";\nconst { doThing2 } = require(\"acme2\");\nconst { doThing3: dt3 } = require(\"acme3\");\nconst acme4 = require(\"acme4\");\ndoThing();\ndoThing2();\ndt3();\nacme4();\n";
        let (mut s, _dir) = store_with_index(&[("src/index.cjs", src)]);
        s.open_path("src/index.cjs");
        let mut probe = |symbol: &str| {
            let at = src.rfind(symbol).expect("fixture");
            let (line, col) = point_of(src, at);
            s.set_point(line, col, col);
            s.resolver_scope(symbol)
        };
        assert_eq!(
            probe("doThing"),
            vec!["acme/sub".to_string(), "doThing".to_string()]
        );
        assert_eq!(
            probe("doThing2"),
            vec!["acme2".to_string(), "doThing2".to_string()]
        );
        assert_eq!(
            probe("dt3"),
            vec!["acme3".to_string(), "doThing3".to_string()]
        );
        // The whole-module binding names the entry itself (no item).
        assert_eq!(probe("acme4"), vec!["acme4".to_string()]);
    }

    /// TS `import type { D }` carries the same hint as a value import
    /// (type-only imports still name the item for the provider's scan).
    #[test]
    fn resolver_scope_ts_import_type_carries_package_path() {
        let src = "import type { D } from \"acme\";\nlet d: D;\n";
        let (mut s, _dir) = store_with_index(&[("src/index.ts", src)]);
        s.open_path("src/index.ts");
        let at = src.rfind("D").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("D"),
            vec!["acme".to_string(), "D".to_string()]
        );
    }

    /// Discriminating: a BARE imported symbol in a Python buffer carries
    /// the module path + item (pre-011-02 the hint was empty).
    #[test]
    fn resolver_scope_python_from_import_carries_module_path() {
        let src = "from acme import doThing\n\ndoThing()\n";
        let (mut s, _dir) = store_with_index(&[("main.py", src)]);
        s.open_path("main.py");
        let at = src.rfind("doThing").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("doThing"),
            vec!["acme".to_string(), "doThing".to_string()]
        );
    }

    /// Python `from a.b import X as Y`: the bare alias carries the FULL
    /// original module path + original item.
    #[test]
    fn resolver_scope_python_from_submodule_alias_carries_original() {
        let src = "from acme.sub import doThing as dt\n\ndt()\n";
        let (mut s, _dir) = store_with_index(&[("main.py", src)]);
        s.open_path("main.py");
        let at = src.rfind("dt").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("dt"),
            vec!["acme".to_string(), "sub".to_string(), "doThing".to_string()]
        );
    }

    /// Python module alias (`import a.b as c`) carries the module chain
    /// for the bare alias; a plain `import a.b` binds ONLY the top-level
    /// `a` — the bare `b` is never guessed.
    #[test]
    fn resolver_scope_python_module_alias_and_top_level_only() {
        let src = "import acme.sub\nimport acme.sub2 as s\n\ns.fn()\n";
        let (mut s, _dir) = store_with_index(&[("main.py", src)]);
        s.open_path("main.py");
        // Bare `s` (the alias): the module chain.
        let at = src.find("s.fn()").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("s"),
            vec!["acme".to_string(), "sub2".to_string()]
        );
        // Plain `import acme.sub`: bare `sub` is NOT bound (only `acme`)
        // → no hint (never guessed).
        let at = src.find("acme.sub").expect("fixture");
        let (line, col) = point_of(src, at + "acme.".len());
        s.set_point(line, col, col);
        assert!(s.resolver_scope("sub").is_empty());
    }

    /// Python local imports shadow module-level ones (innermost block
    /// wins — the bounded walk mirrors the Rust mod-scope rule); within
    /// one block the LAST re-import at/before the point wins.
    #[test]
    fn resolver_scope_python_local_import_shadows_module_level() {
        let src = "from a import Thing\ndef f():\n    from b import Thing\n    return Thing\n\nThing()\n";
        let (mut s, _dir) = store_with_index(&[("main.py", src)]);
        s.open_path("main.py");
        // Inside the function: the local import wins.
        let at = src.rfind("return Thing").expect("fixture");
        let (line, col) = point_of(src, at + "return ".len());
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("Thing"),
            vec!["b".to_string(), "Thing".to_string()]
        );
        // Module level: the module-level import.
        let at = src.rfind("Thing()").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("Thing"),
            vec!["a".to_string(), "Thing".to_string()]
        );
    }

    /// Python same-block re-import: the last matching statement at/before
    /// the point shadows the earlier one (imports are statements, unlike
    /// Rust's module-scoped `use`).
    #[test]
    fn resolver_scope_python_later_reimport_shadows() {
        let src = "from a import Thing\nx = Thing\nfrom b import Thing\ny = Thing\n";
        let (mut s, _dir) = store_with_index(&[("main.py", src)]);
        s.open_path("main.py");
        let before = src.find("x = Thing").expect("fixture");
        let (line, col) = point_of(src, before + 4);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("Thing"),
            vec!["a".to_string(), "Thing".to_string()]
        );
        let after = src.rfind("y = Thing").expect("fixture");
        let (line, col) = point_of(src, after + 4);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("Thing"),
            vec!["b".to_string(), "Thing".to_string()]
        );
    }

    /// Python relative imports, wildcards, and dotted symbols are NEVER
    /// guessed: the hint stays empty (the provider keeps its exact
    /// no-hint bail / own-path behavior).
    #[test]
    fn resolver_scope_python_relative_and_wildcard_are_not_guessed() {
        for (name, src, symbol) in [
            ("relative", "from . import Thing\nThing()\n", "Thing"),
            ("relative two", "from ..mod import Thing\nThing()\n", "Thing"),
            ("wildcard", "from acme import *\ndoThing()\n", "doThing"),
            ("dotted symbol", "import os\nos.path.join('a', 'b')\n", "os.path.join"),
        ] {
            let (mut s, _dir) = store_with_index(&[("main.py", src)]);
            s.open_path("main.py");
            let at = src.rfind(symbol).expect("fixture");
            let (line, col) = point_of(src, at);
            s.set_point(line, col, col);
            assert!(
                s.resolver_scope(symbol).is_empty(),
                "{name}: no hint expected"
            );
        }
    }

    /// Discriminating: a BARE symbol dot-imported in Go carries the
    /// local package name (the import path's last segment) + the symbol.
    #[test]
    fn resolver_scope_go_dot_import_carries_package_name() {
        let src = "package main\n\nimport . \"github.com/pkg/errors\"\n\nfunc main() {\n\t_ = New(\"x\")\n}\n";
        let (mut s, _dir) = store_with_index(&[("main.go", src)]);
        s.open_path("main.go");
        let at = src.rfind("New").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("New"),
            vec!["errors".to_string(), "New".to_string()]
        );
    }

    /// Go grouped dot imports work the same (the `import ( … )` form).
    #[test]
    fn resolver_scope_go_grouped_dot_import() {
        let src = "package main\n\nimport (\n\t. \"github.com/pkg/errors\"\n\t\"fmt\"\n)\n\nfunc main() {\n\t_ = New(\"x\")\n}\n";
        let (mut s, _dir) = store_with_index(&[("main.go", src)]);
        s.open_path("main.go");
        let at = src.rfind("New").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("New"),
            vec!["errors".to_string(), "New".to_string()]
        );
    }

    /// Go plain/aliased imports bind a package NAME (always used
    /// qualified) — never a bare-item hint; dotted symbols carry their
    /// own package name → empty. Several dot imports make the origin of a
    /// bare item ambiguous → never guessed.
    #[test]
    fn resolver_scope_go_non_dot_and_ambiguous_imports_are_not_guessed() {
        for (name, src, symbol) in [
            (
                "plain import",
                "package main\n\nimport \"github.com/x/y\"\n\nfunc main() {\n\t_ = y.Fn\n}\n",
                "y",
            ),
            (
                "aliased import",
                "package main\n\nimport yy \"github.com/a/b\"\n\nfunc main() {\n\t_ = yy.Fn\n}\n",
                "yy",
            ),
            (
                "two dot imports",
                "package main\n\nimport (\n\t. \"github.com/a/aa\"\n\t. \"github.com/b/bb\"\n)\n\nfunc main() {\n\t_ = Fn\n}\n",
                "Fn",
            ),
            (
                "dotted symbol",
                "package main\n\nimport \"github.com/x/y\"\n\nfunc main() {\n\t_ = y.Fn\n}\n",
                "y.Fn",
            ),
        ] {
            let (mut s, _dir) = store_with_index(&[("main.go", src)]);
            s.open_path("main.go");
            let at = src.rfind(symbol).expect("fixture");
            let (line, col) = point_of(src, at);
            s.set_point(line, col, col);
            assert!(
                s.resolver_scope(symbol).is_empty(),
                "{name}: no hint expected"
            );
        }
    }

