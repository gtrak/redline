use super::*;

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
            ("src/lib.rs", "pub fn target() {\n\n}\n"),
        ]);
        s.open_path("src/main.rs");
        // Direct hit under the point → no fall-through.
        s.set_point(0, 12, 12);
        s.xref_find_definitions();
        assert_eq!(s.resolve_generation, 1, "direct hit: one supersede bump, no job");
        // Enclosing hit (plan-017 B5: the point carries no token — the
        // blank line inside `target` — so the by-line enclosing answer
        // still stands and skips the resolver; a point on a DIFFERENT
        // token no longer reaches the enclosing guess at all).
        s.open_path("src/lib.rs");
        s.set_point(1, 0, 0);
        s.xref_find_definitions();
        assert!(s.picker_open(), "enclosing fallback: picker (msg: {})", s.message);
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
        s.start_symbol_resolution("os.path.join", "main.py", None);
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
