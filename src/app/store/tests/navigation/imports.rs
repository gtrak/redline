use super::*;

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
