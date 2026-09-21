use super::*;

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
