use super::*;

    #[tokio::test]
    async fn crate_index_background_build_publishes_and_indicates() {
        // The synthetic tree lives OUTSIDE the project root: the index
        // build is root-agnostic (the project indexer's machinery, run
        // on spawn_blocking), and the finished index lands via the
        // CrateIndexBus exactly like the ResolveBus events do.
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(
            root.path().join("src/lib.rs"),
            "pub fn target() {}\npub mod sub { pub fn deep() {} }\n",
        )
        .unwrap();
        std::fs::write(root.path().join("src/other.rs"), "pub fn helper() {}\n").unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(root.path(), &root.path().join("src/lib.rs"));
        // The in-flight indicator mirrors the project indexing indicator.
        assert!(
            s.crate_indexing_display().starts_with("indexing crate "),
            "{}",
            s.crate_indexing_display()
        );
        // 006-03b item 5: the N/M file counter (2 .rs files in the
        // fixture; `done` may have already advanced by now).
        assert!(
            s.crate_indexing_display().ends_with("/2)…"),
            "N/M counter: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        assert_eq!(event.source_root, root.path());
        s.apply_crate_index_event(&event);
        // Installed (one LRU entry) and the indicator cleared with the
        // final event.
        assert_eq!(s.external_indexes.len(), 1);
        assert_eq!(s.crate_indexing_display(), "");
        // Crate-relative outlines are queryable.
        let arc = s.crate_index_arc(root.path()).unwrap();
        let idx = arc.lock().unwrap();
        assert_eq!(idx.definition_count("target"), 1);
        assert_eq!(idx.definition_count("helper"), 1);
        assert_eq!(idx.definition_count("deep"), 1);
        assert!(idx.has("src/lib.rs"), "crate-relative key");
        // A second start for the same root is a no-op (already cached).
        s.start_crate_indexing(root.path(), &root.path().join("src/lib.rs"));
        assert!(s.crate_indexing.is_empty(), "no duplicate build");
    }

    #[test]
    fn external_mdot_cross_file_within_crate_jumps_read_only() {
        // M-. on a type that another file of the SAME crate defines:
        // the jump stays inside the crate (crate-relative display), opens
        // read-only through the external path, and records a jump.
        let (mut s, _dir, root) = store_with_crate_index(&[
            ("src/lib.rs", "pub fn use_helper() {\n    helper();\n}\n"),
            ("src/other.rs", "pub fn helper() {}\n"),
        ]);
        s.open_external_path(&root.path().join("src/lib.rs")).unwrap();
        let origin_key = s.buffers.current().unwrap().to_string();
        // Line 1: "    helper();" — `helper` starts at col 4.
        // (jump-ambiguity) a cross-file unique candidate goes to the
        // picker (best preselected); RET accepts the top guess.
        s.set_point(1, 4, 4);
        s.xref_find_definitions();
        assert!(s.picker_open(), "unique cross-file: picker");
        assert!(
            s.picker_filtered()[0].0.name.starts_with("src/other.rs:"),
            "the cross-file candidate is preselected: {:?}",
            s.picker_filtered()[0].0.name
        );
        s.run_selected();
        assert!(
            s.message.contains("jumped to src/other.rs:1"),
            "crate-relative display, got: {}",
            s.message
        );
        let key = s.buffers.current().unwrap().to_string();
        assert_eq!(key, root.path().join("src/other.rs").to_string_lossy());
        assert_eq!(s.point_line(), 0, "landed on the definition line");
        // The landing stays external: read-only, in the external set, never
        // a project file (recents untouched — 008-01 semantics).
        assert!(!s.buffers.get(&key).unwrap().editable);
        assert!(s.external_buffers.contains(&key));
        // M-, walks back to the origin (still inside the crate).
        s.jump_back();
        assert_eq!(
            s.buffers.current().unwrap().to_string(),
            origin_key,
            "jump-back returns to the origin buffer"
        );
    }

    #[test]
    fn external_mdot_same_file_candidate_wins() {
        // The normal struct + impl-in-one-file case: the same-file
        // candidate wins the direct jump (the project path's
        // same-file-first ordering, unchanged).
        let (mut s, _dir, root) = store_with_crate_index(&[(
            "src/lib.rs",
            "pub struct Foo {}\nimpl Foo {\n    pub fn new() -> Self { Foo {} }\n}\npub fn use_it() {\n    let f = Foo::new();\n}\n",
        )]);
        s.open_external_path(&root.path().join("src/lib.rs")).unwrap();
        // Line 5: "    let f = Foo::new();" — `new` starts at col 17.
        s.set_point(5, 17, 17);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique same-file: no picker");
        assert_eq!(s.point_line(), 2, "jumped to the same-file impl method");
        assert!(
            s.message.contains("jumped to src/lib.rs:3"),
            "{}",
            s.message
        );
    }

    #[test]
    fn external_mdot_ambiguous_opens_crate_picker_and_selection_jumps() {
        // Two same-named definitions across the crate → the Xref picker
        // with CRATE-RELATIVE display paths; the query re-computation and
        // the RET selection both stay crate-rooted (the selection opens
        // read-only inside the crate, never through the project root).
        let (mut s, _dir, root) = store_with_crate_index(&[
            ("src/lib.rs", "pub fn target() {}\npub fn call() {\n    target();\n}\n"),
            ("src/other.rs", "pub fn target() {}\n"),
        ]);
        s.open_external_path(&root.path().join("src/lib.rs")).unwrap();
        // Line 2: "    target();" — `target` starts at col 4.
        s.set_point(2, 4, 4);
        s.xref_find_definitions();
        assert!(s.picker_open(), "two candidates: picker");
        assert_eq!(s.picker_kind(), Some(PickerKind::Xref));
        let filtered = s.picker_filtered();
        assert_eq!(filtered.len(), 2);
        assert!(
            filtered[0].0.name.starts_with("src/lib.rs:"),
            "same-file candidate first: {}",
            filtered[0].0.name
        );
        assert!(
            filtered[1].0.name.starts_with("src/other.rs:"),
            "crate-relative display: {}",
            filtered[1].0.name
        );
        // Query re-computation consults the CRATE index (the project
        // index has no `target` — a leaked project root would yield 0).
        s.picker_query_char('s');
        assert_eq!(
            s.picker_filtered().len(),
            2,
            "refilter against the crate index"
        );
        // Select the second candidate (DOWN + RET): the jump lands
        // inside the crate, read-only.
        s.key_event(key("DOWN"));
        assert_eq!(s.picker_selected(), 1);
        s.key_event(key("RET"));
        assert!(s.message.contains("jumped to src/other.rs:1"), "{}", s.message);
        let key = s.buffers.current().unwrap().to_string();
        assert_eq!(key, root.path().join("src/other.rs").to_string_lossy());
        assert!(s.external_buffers.contains(&key));
        assert!(!s.buffers.get(&key).unwrap().editable);
    }

    #[test]
    fn external_mdot_miss_falls_through_to_resolver() {
        // A symbol the crate index does not know: the miss keeps the
        // project path's resolver fall-through (unchanged semantics —
        // the origin project's metadata), superseding any in-flight
        // resolve. Plain (no runtime) test: the spawn is skipped and the
        // miss reported synchronously, like xref_workspace_miss_fires_….
        let (mut s, _dir, root) = store_with_crate_index(&[(
            "src/probe.rs",
            "pub fn helper() {}\ntokio::spawn(f);\n",
        )]);
        s.open_external_path(&root.path().join("src/probe.rs")).unwrap();
        // Line 1: top-level `tokio::spawn(f);` — no crate definition, no
        // enclosing symbol → resolver fall-through with the path token.
        s.set_point(1, 9, 9); // cursor inside `spawn`
        s.xref_find_definitions();
        assert_eq!(
            s.resolve_generation, 2,
            "external supersede bump + start bump"
        );
        assert!(
            s.message.contains("no provider resolution for `tokio::spawn`"),
            "graceful miss, got: {}",
            s.message
        );
        assert!(
            s.resolving_display().is_empty(),
            "no runtime: the indicator must not hang"
        );
    }

    #[test]
    fn external_imenu_lists_crate_file_outline_and_refilters() {
        // M-i inside an external buffer: the outline comes from the
        // owning crate's index (not a refusal), and the query
        // re-computation stays crate-rooted.
        let (mut s, _dir, root) = store_with_crate_index(&[(
            "src/lib.rs",
            "pub struct Foo {}\nimpl Foo {\n    pub fn new() -> Self { Foo {} }\n}\npub fn use_it() {\n    let f = Foo::new();\n}\n",
        )]);
        s.open_external_path(&root.path().join("src/lib.rs")).unwrap();
        s.open_imenu();
        assert!(s.picker_open(), "imenu opens on the external buffer");
        assert_eq!(s.picker_kind(), Some(PickerKind::Imenu));
        let names: Vec<_> = s
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.name.clone())
            .collect();
        assert_eq!(names, vec!["Foo:1", "new:3", "use_it:5"], "{names:?}");
        assert_eq!(s.picker_count(), (3, 3), "the total follows the crate index");
        // Refilter: `new` matches only the impl method (the crate index,
        // not the project's, is consulted — the project index is empty
        // here; a leaked project root would yield 0 candidates).
        s.picker_query_char('n');
        s.picker_query_char('e');
        s.picker_query_char('w');
        let filtered: Vec<_> = s
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.name.clone())
            .collect();
        assert_eq!(filtered, vec!["new:3"], "{filtered:?}");
    }

    #[test]
    fn external_index_lru_cap_evicts_oldest_and_recency_bumps() {
        // The cache caps at 3 roots; the oldest is evicted when a new
        // crate lands; a lookup bumps recency.
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let roots: Vec<tempfile::TempDir> = (0..4)
            .map(|_| tempfile::tempdir().unwrap())
            .collect();
        for r in &roots {
            s.apply_crate_index_event(&CrateIndexEvent {
                source_root: r.path().to_path_buf(),
                index: SymbolIndex::default(),
            });
        }
        let ordered: Vec<&Path> =
            s.external_indexes.iter().map(|(r, _)| r.as_path()).collect();
        assert_eq!(s.external_indexes.len(), 3, "cap holds");
        assert_eq!(
            ordered,
            vec![roots[1].path(), roots[2].path(), roots[3].path()],
            "the oldest (roots[0]) is evicted; newest last"
        );
        // A lookup bumps recency: roots[1] moves to the newest slot…
        s.crate_index_arc(roots[1].path()).unwrap();
        // …so the next landing evicts roots[2] instead (the new oldest).
        let r5 = tempfile::tempdir().unwrap();
        s.apply_crate_index_event(&CrateIndexEvent {
            source_root: r5.path().to_path_buf(),
            index: SymbolIndex::default(),
        });
        let ordered: Vec<&Path> =
            s.external_indexes.iter().map(|(r, _)| r.as_path()).collect();
        assert_eq!(
            ordered,
            vec![roots[3].path(), roots[1].path(), r5.path()],
            "recency bump changed the eviction victim (roots[2] evicted)"
        );
    }

    #[test]
    fn external_landed_crate_survives_eviction_while_current() {
        // 006-03b item 1: land in crate A (its buffer becomes current),
        // then three other crates' indexes land while A's buffer stays
        // current → A is never the eviction victim, and the next M-. in
        // A still jumps in-crate. Discriminating: without the recency
        // bump A is the OLDEST entry, the third arrival evicts it, and
        // the M-. falls through to the resolver (`no provider
        // resolution`).
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let roots: Vec<tempfile::TempDir> =
            (0..4).map(|_| tempfile::tempdir().unwrap()).collect();
        let root_a = roots[0].path();
        std::fs::create_dir_all(root_a.join("src")).unwrap();
        std::fs::write(
            root_a.join("src/lib.rs"),
            "pub fn use_helper() {\n    helper();\n}\n",
        )
        .unwrap();
        std::fs::write(root_a.join("src/other.rs"), "pub fn helper() {}\n").unwrap();
        // Land in A: its index arrives, its buffer becomes current.
        s.apply_crate_index_event(&CrateIndexEvent {
            source_root: root_a.to_path_buf(),
            index: build_index(root_a, &AppStore::crate_source_files(root_a, &["rs"]), None),
        });
        s.open_external_path(&root_a.join("src/lib.rs")).unwrap();
        // Consult the three other crates: their indexes land while A's
        // buffer stays current (the cap is 3 — each arrival evicts the
        // oldest).
        for r in &roots[1..] {
            s.apply_crate_index_event(&CrateIndexEvent {
                source_root: r.path().to_path_buf(),
                index: SymbolIndex::default(),
            });
        }
        let ordered: Vec<&Path> =
            s.external_indexes.iter().map(|(r, _)| r.as_path()).collect();
        assert_eq!(s.external_indexes.len(), 3, "cap holds");
        assert!(
            ordered.contains(&root_a),
            "crate A (the current buffer's crate) survived eviction: {ordered:?}"
        );
        // The next M-. in A still resolves in-crate (jump-ambiguity:
        // cross-file unique → the picker; RET accepts the top guess).
        s.set_point(1, 4, 4); // "    helper();" — `helper` starts at col 4.
        s.xref_find_definitions();
        assert!(s.picker_open(), "cross-file unique: picker");
        assert!(
            s.picker_filtered()[0].0.name.starts_with("src/other.rs:"),
            "the in-crate candidate is preselected: {:?}",
            s.picker_filtered()[0].0.name
        );
        s.run_selected();
        assert!(
            s.message.contains("jumped to src/other.rs:1"),
            "in-crate jump intact, got: {}",
            s.message
        );
    }

    #[test]
    fn external_rel_normalizes_backslash_separators() {
        // 006-03b item 3: a `\`-containing relative path resolves the
        // same index key as its `/` form (the Windows native-separator
        // case; on Linux a backslash-bearing component simulates it,
        // on Windows `src\lib.rs` is the real two-component path).
        let (mut s, _dir, root) = store_with_crate_index(&[(
            "src/lib.rs",
            "pub fn helper() {}\n",
        )]);
        let root_path = root.path();
        let slashy = root_path.join("src/lib.rs");
        let backslashed = root_path.join("src\\lib.rs");
        let r_slash = AppStore::crate_rel(&slashy, root_path);
        let r_back = AppStore::crate_rel(&backslashed, root_path);
        assert_eq!(r_slash.as_deref(), Some("src/lib.rs"));
        assert_eq!(r_slash, r_back, "the \\ form normalizes to the / key");
        // Both forms outline the same file through the cached index
        // (the `current_buffer_outline` lookup site).
        let arc = s.crate_index_arc(root_path).unwrap();
        let idx = arc.lock().unwrap();
        let outline_slash = idx.outline(&r_slash.unwrap());
        assert!(!outline_slash.is_empty());
        assert_eq!(idx.outline(&r_back.unwrap()), outline_slash);
        // A non-nested path is a clean miss (the 006-03b item 4
        // no-panic path), never an unwrap.
        assert!(AppStore::crate_rel(
            Path::new("elsewhere/lib.rs"),
            root_path
        )
        .is_none());
    }

    #[test]
    fn external_resolver_from_file_is_crate_relative() {
        // 006-03b item 2: the resolver fall-through's `from_file` for an
        // external buffer is crate-relative (the index's own key shape),
        // never the absolute path.
        let (mut s, _dir, root) = store_with_crate_index(&[("src/probe.rs", "pub fn helper() {}\n")]);
        let path = root.path().join("src/probe.rs");
        s.open_external_path(&path).unwrap();
        assert_eq!(s.resolver_from_file(&path), "src/probe.rs");
        // A still-in-flight root (not yet installed) is crate-relative
        // too…
        let inflight = tempfile::tempdir().unwrap();
        s.crate_indexing.push((
            inflight.path().to_path_buf(),
            "inflight".to_string(),
            Arc::new(IndexProgress::new(1)),
        ));
        assert_eq!(
            s.resolver_from_file(&inflight.path().join("src/lib.rs")),
            "src/lib.rs"
        );
        // …and a path under NO known root falls back to the file name
        // (relative, never absolute).
        assert_eq!(
            s.resolver_from_file(Path::new("/tmp/stray/lib.rs")),
            "lib.rs"
        );
    }

    #[test]
    fn external_cache_does_not_leak_into_project_mdot() {
        // Regression: a project file's M-. resolves against the PROJECT
        // index even when a cached crate index defines the same name.
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() { target(); }\n"),
            ("src/lib.rs", "pub fn target() {}\n"),
        ]);
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src/crate.rs"), "pub fn target() {}\n").unwrap();
        let crate_files = AppStore::crate_source_files(root.path(), &["rs"]);
        s.apply_crate_index_event(&CrateIndexEvent {
            source_root: root.path().to_path_buf(),
            index: build_index(root.path(), &crate_files, None),
        });
        s.open_path("src/main.rs");
        // Line 0: "fn main() { target(); }" — `target` starts at col 12.
        // (jump-ambiguity) the project candidate is UNIQUE CROSS-FILE →
        // the picker; the PROJECT definition must be the preselected
        // row (the crate index's same-named one must not leak in).
        s.set_point(0, 12, 12);
        s.xref_find_definitions();
        let filtered = s.picker_filtered();
        assert!(
            filtered[0].0.name.starts_with("src/lib.rs:"),
            "the PROJECT definition is preselected (not the crate's): {:?}",
            filtered[0].0.name
        );
        s.run_selected();
        assert_eq!(
            s.view_name_display(),
            "src/lib.rs",
            "the PROJECT definition wins (not the crate's)"
        );
    }

    #[test]
    fn blame_external_buffer_says_why() {
        // Registry / tooling sources are not git checkouts: the refusal
        // stays, but the message now explains it (006-03 item 4).
        let (_dir, _ext, mut s) = store_with_external_buffer();
        s.open_blame();
        assert_eq!(s.message, "no git history for external sources");
    }

    #[test]
    fn previous_project_buffer_mdot_still_refused() {
        // The lifted refusal applies only to EXTERNAL buffers: a previous
        // project's file (outside the new root but not an external
        // landing) keeps the plain refusal.
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        for d in [&dir1, &dir2] {
            std::fs::write(d.path().join("Cargo.toml"), "[package]\n").unwrap();
            std::fs::create_dir_all(d.path().join("src")).unwrap();
        }
        std::fs::write(dir1.path().join("src/a.rs"), "fn a() {}\n").unwrap();
        let mut s = store(dir1.path());
        s.open_path("src/a.rs");
        s.switch_project_root(dir2.path().to_str().unwrap());
        s.xref_find_definitions();
        assert_eq!(s.message, "buffer not in project");
    }

    #[tokio::test]
    async fn crate_index_refuses_oversized_tree() {
        // Above the file cap the build is refused with a clear message
        // (no known registry crate approaches 2000 .rs files).
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        for i in 0..(EXT_INDEX_FILE_CAP + 1) {
            std::fs::write(
                root.path().join(format!("f{i}.rs")),
                "fn f() {}\n",
            )
            .unwrap();
        }
        s.start_crate_indexing(root.path(), &root.path().join("f0.rs"));
        assert!(
            s.message.contains("crate too large to index")
                && s.message.contains("2001"),
            "{}",
            s.message
        );
        assert!(s.crate_indexing.is_empty(), "no in-flight build armed");
        assert!(s.external_indexes.is_empty(), "no cache entry");
    }

    // ── plan 011 issue 04: per-language source index ───────────────────

    /// 011-04 (extended by 011-07): the walk's extension sets stay in
    /// lockstep with `registry.rs`'s extension map (the authority):
    /// every walked extension must resolve (through the registry map) to
    /// a language whose OWN set includes it — the sets are per grammar
    /// family.
    #[test]
    fn source_extensions_round_trip_through_registry_map() {
        use redline_syntax::registry::resolve_language;
        for lang in [
            LanguageId::Rust,
            LanguageId::JavaScript,
            LanguageId::TypeScript,
            LanguageId::Tsx,
            LanguageId::Python,
            LanguageId::Go,
            LanguageId::C,
            LanguageId::Cpp,
            LanguageId::Markdown,
            LanguageId::Java,
            LanguageId::CSharp,
            LanguageId::Ruby,
            LanguageId::Scheme,
        ] {
            for ext in AppStore::source_extensions_for(lang) {
                let resolved = resolve_language(&format!("a.{ext}"));
                assert!(
                    AppStore::source_extensions_for(resolved).contains(ext),
                    "`{ext}` (owned by {lang:?}) must resolve to a language"
                );
            }
        }
        // Documented exclusion: the registry maps `rsi` to Rust, but the
        // walk indexes `rs` only (rust-analyzer interface files are not
        // definition sources).
        assert!(
            !AppStore::source_extensions_for(LanguageId::Rust).contains(&"rsi"),
            "rsi stays out of the Rust index set"
        );
        // Languages whose grammar queries exist but whose files are not
        // M-. definition sources (011-04 judgment, carried by 011-07):
        // no walk set, no index.
        for lang in [
            LanguageId::Plain,
            LanguageId::Json,
            LanguageId::Toml,
            LanguageId::Yaml,
            LanguageId::Bash,
        ] {
            assert!(AppStore::source_extensions_for(lang).is_empty());
        }
    }

    /// 011-04: the extension predicate is PER LANGUAGE — a JS/TS tree
    /// collects js/jsx/ts/tsx/mjs/cjs and only those; a Python tree
    /// py/pyi; a Rust tree `rs` (the 006-03 set, unchanged). Nested
    /// `node_modules` is never walked (item 4).
    #[test]
    fn crate_source_files_selects_own_language_extensions() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        for rel in [
            "src/index.js",
            "src/util.mjs",
            "src/cfg.cjs",
            "src/view.jsx",
            "src/types.ts",
            "src/app.tsx",
            "README.md",
            "package.json",
            "node_modules/leftpad/index.js",
            "node_modules/leftpad/package.json",
        ] {
            let path = p.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "// x\n").unwrap();
        }
        let mut js = AppStore::crate_source_files(
            p,
            &["js", "jsx", "ts", "tsx", "mjs", "cjs"],
        );
        js.sort();
        assert_eq!(
            js,
            vec![
                "src/app.tsx",
                "src/cfg.cjs",
                "src/index.js",
                "src/types.ts",
                "src/util.mjs",
                "src/view.jsx",
            ],
            "own-language extensions only: no .md/.json, no node_modules"
        );

        let pyr = tempfile::tempdir().unwrap();
        std::fs::write(pyr.path().join("foo.py"), "\n").unwrap();
        std::fs::create_dir_all(pyr.path().join("types")).unwrap();
        std::fs::write(pyr.path().join("types/stubs.pyi"), "\n").unwrap();
        std::fs::write(pyr.path().join("notes.txt"), "\n").unwrap();
        let mut py = AppStore::crate_source_files(pyr.path(), &["py", "pyi"]);
        py.sort();
        assert_eq!(py, vec!["foo.py", "types/stubs.pyi"]);

        // Rust: the 006-03 set, unchanged.
        let rr = tempfile::tempdir().unwrap();
        std::fs::write(rr.path().join("a.rs"), "\n").unwrap();
        std::fs::write(rr.path().join("b.rsi"), "\n").unwrap();
        std::fs::write(rr.path().join("c.md"), "\n").unwrap();
        let rs = AppStore::crate_source_files(rr.path(), &["rs"]);
        assert_eq!(rs, vec!["a.rs"]);
    }

    /// 011-04 (item 4): a `node_modules` DIRECTORY inside the root is
    /// skipped even mid-path — but a root that IS a `node_modules` dir
    /// (a landed `node_modules/<pkg>`) still indexes its own files (the
    /// boundary is a component AFTER the root, never the file name).
    #[test]
    fn crate_source_files_node_modules_boundary() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("dist")).unwrap();
        std::fs::write(p.join("index.js"), "\n").unwrap();
        std::fs::write(p.join("dist/bundle.js"), "\n").unwrap();
        // A directory NAMED like a source file: the file itself is fine.
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("src/util.js"), "\n").unwrap();
        let mut files = AppStore::crate_source_files(p, &["js"]);
        files.sort();
        assert_eq!(files, vec!["dist/bundle.js", "index.js", "src/util.js"]);

        // The root ITSELF named node_modules (a landed package dir):
        // its own files are walked.
        let nm = tempfile::tempdir().unwrap();
        let p2 = nm.path();
        std::fs::create_dir_all(p2.join("inner")).unwrap();
        std::fs::write(p2.join("own.js"), "\n").unwrap();
        std::fs::write(p2.join("inner/dep.js"), "\n").unwrap();
        std::fs::create_dir_all(p2.join("inner/node_modules/other")).unwrap();
        std::fs::write(p2.join("inner/node_modules/other/x.js"), "\n").unwrap();
        let mut files2 = AppStore::crate_source_files(p2, &["js"]);
        files2.sort();
        assert_eq!(
            files2,
            vec!["inner/dep.js", "own.js"],
            "the root node_modules is indexed; nested ones are not"
        );
    }

    /// F5 (a): a landed dependency inside a git repo's `node_modules/`
    /// (gitignored by the PROJECT's `.gitignore`) must still be walked —
    /// before F5, `ignore::Walk::new`'s defaults (`parents +
    /// git_ignore + require_git`) applied the project's ignore sources to
    /// this walk and zeroed the file list; `.git/info/exclude` is
    /// asserted too.
    #[test]
    fn crate_source_files_walks_gitignored_node_modules_dependency() {
        let proj = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(proj.path().join(".git")).unwrap();
        std::fs::write(proj.path().join(".gitignore"), "node_modules/\n").unwrap();
        std::fs::create_dir_all(proj.path().join(".git/info")).unwrap();
        std::fs::write(
            proj.path().join(".git/info/exclude"),
            "skipped-by-exclude.js\n",
        )
        .unwrap();
        let root = proj.path().join("node_modules/demo");
        for rel in ["index.js", "lib/skipped-by-exclude.js", "README.md"] {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "// x\n").unwrap();
        }
        let mut files = AppStore::crate_source_files(
            &root,
            &["js", "jsx", "ts", "tsx", "mjs", "cjs"],
        );
        files.sort();
        // The project's `node_modules/` rule and `.git/info/exclude`
        // must NOT reach this walk: both js files are the landed
        // dependency's own source (README.md is dropped by the
        // extension set).
        assert_eq!(
            files,
            vec!["index.js", "lib/skipped-by-exclude.js"],
            "the project's ignore sources must not apply to the dependency walk"
        );
    }

    /// F5 (b): a landed dependency under a HIDDEN directory (`.venv`)
    /// must still be walked — before F5, `hidden(true)` (a
    /// `WalkBuilder` default) pruned hidden entries below the walk
    /// root: hidden directories inside the package (`.venv/lib/...` is
    /// only hidden-above the root — the root itself is always visited —
    /// but local plugin dirs, cache dirs, and venv layouts whose root is
    /// the hidden dir itself) vanished from the index. Non-source files
    /// stay out via the extension set; a `.git` inside the dependency
    /// is pruned explicitly (the walker's docs).
    #[test]
    fn crate_source_files_walks_hidden_venv_dependency() {
        let hidden = tempfile::tempdir().unwrap();
        // A hidden-named walk root (`.venv`): the root entry itself is
        // always visited, but every hidden component BELOW it used to be
        // pruned by `hidden(true)` before F5.
        let root = hidden.path().join(".venv");
        for rel in [
            "lib/site-packages/demo/__init__.py",
            "lib/site-packages/demo/pkg/mod.py",
            "lib/site-packages/demo/data.txt",
        ] {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "x\n").unwrap();
        }
        let mut files = AppStore::crate_source_files(&root, &["py", "pyi"]);
        files.sort();
        assert_eq!(
            files,
            vec![
                "lib/site-packages/demo/__init__.py",
                "lib/site-packages/demo/pkg/mod.py"
            ],
            "the venv tree under the hidden root must be walked (data.txt is not python)"
        );
        // And hidden dirs INSIDE a non-hidden package root are walked too
        // (local plugin dirs holding source).
        let pkg = tempfile::tempdir().unwrap();
        for rel in ["__init__.py", ".local/plugin.pyi"] {
            let path = pkg.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "x\n").unwrap();
        }
        let mut files2 = AppStore::crate_source_files(pkg.path(), &["py", "pyi"]);
        files2.sort();
        assert_eq!(
            files2,
            vec![".local/plugin.pyi", "__init__.py"],
            "hidden dirs inside the package must be walked"
        );
        // A `.git` inside the dependency is never indexed (defensive
        // prune, since `hidden(false)` now walks dotdirs).
        let gitpkg = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(gitpkg.path().join(".git")).unwrap();
        std::fs::write(gitpkg.path().join(".git/config"), "[core]\n").unwrap();
        std::fs::write(gitpkg.path().join("mod.py"), "x\n").unwrap();
        let files3 = AppStore::crate_source_files(gitpkg.path(), &["py", "pyi"]);
        assert_eq!(files3, vec!["mod.py"], ".git must not be indexed");
    }

    /// F5 end-to-end regression guard: a landed `node_modules/<pkg>`
    /// dependency inside a git repo (the project `.gitignore`s
    /// `node_modules/`) indexes through the full path and resolves M-.
    /// in-crate via the external-buffer xref.
    ///
    /// **NOT a discriminating test for F5**: a pattern matching an ancestor
    /// ABOVE the walk root does not apply, so the pre-F5 walker already saw
    /// these files (measured: `["index.js"]`, not zero). The pre-F5 failure
    /// modes are hidden pruning, `.git/info/exclude` / global excludes, and
    /// BELOW-root `.gitignore` patterns — covered by the two tests that DO
    /// fail pre-fix (`crate_source_files_walks_gitignored_node_modules_dependency`,
    /// `crate_source_files_walks_hidden_venv_dependency`).
    #[test]
    fn git_repo_node_modules_dependency_resolves_in_crate_via_m_dot() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let proj = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(proj.path().join(".git")).unwrap();
        std::fs::write(proj.path().join(".gitignore"), "node_modules/\n").unwrap();
        let root = proj.path().join("node_modules/demo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/lib.js"),
            "export function useIt() {\n    helper();\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/helper.js"),
            "export function helper() {}\n",
        )
        .unwrap();
        let files = AppStore::crate_source_files(
            &root,
            &["js", "jsx", "ts", "tsx", "mjs", "cjs"],
        );
        assert_eq!(
            files.len(),
            2,
            "F5: both of the dependency's own files must be walked: {files:?}"
        );
        s.apply_crate_index_event(&CrateIndexEvent {
            source_root: root.clone(),
            index: build_index(&root, &files, None),
        });
        let abs = root.join("src/lib.js");
        s.open_external_path(&abs).unwrap();
        s.set_point(1, 4, 4); // line 1 = "    helper();" — `helper` at col 4.
        s.xref_find_definitions();
        // jump-ambiguity: a unique definition now opens the picker rather
        // than jumping silently — the best candidate is preselected, so
        // accepting it lands exactly where the silent jump used to.
        assert!(
            s.picker_open(),
            "the dependency's resolve must open the picker, got: {}",
            s.message
        );
        assert_eq!(s.picker_kind(), Some(PickerKind::Xref));
        s.run_selected();
        assert!(
            s.message.contains("jumped to src/helper.js:1"),
            "in-crate M-. must resolve in the landed dependency, got: {}",
            s.message
        );
    }

    /// 011-04 discriminating: a synthetic out-of-root JS tree indexes
    /// through the FULL path (language derivation from the landed file →
    /// per-language walk → build_index → CrateIndexBus): crate-relative
    /// keys and per-language symbol extraction. Pre-011-04 this tree
    /// indexed NOTHING (the walk was `.rs`-only).
    #[tokio::test]
    async fn crate_index_builds_for_js_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("src/index.js"), "export function alpha() {}\n").unwrap();
        std::fs::write(p.join("util.mjs"), "export function beta() {}\n").unwrap();
        std::fs::write(p.join("README.md"), "docs\n").unwrap();
        std::fs::create_dir_all(p.join("node_modules/leftpad")).unwrap();
        std::fs::write(
            p.join("node_modules/leftpad/index.js"),
            "export function leftpad() {}\n",
        )
        .unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("src/index.js"));
        // 2 OWN source files (README.md + node_modules excluded): the
        // N/M counter is the file-set's size, not the tree's.
        assert!(
            s.crate_indexing_display().ends_with(")…")
                && s.crate_indexing_display().contains("/2)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        assert_eq!(event.source_root, p);
        s.apply_crate_index_event(&event);
        assert_eq!(s.external_indexes.len(), 1);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(idx.has("src/index.js"), "crate-relative key (.js)");
        assert!(idx.has("util.mjs"), "crate-relative key (.mjs)");
        assert_eq!(idx.definition_count("alpha"), 1, "js symbol extraction");
        assert_eq!(idx.definition_count("beta"), 1);
        assert!(!idx.has("README.md"), "non-source file not indexed");
        assert!(
            !idx.has("node_modules/leftpad/index.js"),
            "item 4: nested node_modules never indexed"
        );
    }

    /// 011-04 discriminating: a synthetic out-of-root Python tree
    /// indexes through the full path (the `pyi` stub extension is the
    /// registry's python-map pin; `build_index`'s symbol extraction is
    /// language-aware via `registry::resolve_language` + the query
    /// registry — no Rust-only path).
    #[tokio::test]
    async fn crate_index_builds_for_python_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("pkg")).unwrap();
        std::fs::write(
            p.join("pkg/foo.py"),
            "def gamma():\n    return 1\n\nclass Gamma:\n    pass\n",
        )
        .unwrap();
        std::fs::write(p.join("pkg/stubs.pyi"), "def delta(x: int) -> int: ...\n")
            .unwrap();
        std::fs::write(p.join("setup.cfg"), "[x]\n").unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("pkg/foo.py"));
        assert!(
            s.crate_indexing_display().contains("/2)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        s.apply_crate_index_event(&event);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(idx.has("pkg/foo.py"), "crate-relative key (.py)");
        assert!(idx.has("pkg/stubs.pyi"), "crate-relative key (.pyi)");
        assert_eq!(idx.definition_count("gamma"), 1, "python symbol extraction");
        assert_eq!(idx.definition_count("Gamma"), 1);
        assert_eq!(idx.definition_count("delta"), 1, "pyi stub extraction");
        assert!(!idx.has("setup.cfg"));
    }

    /// 011-04: the Go set indexes Go definitions through the same walk +
    /// build_index path (the go toolchain is absent in the sandbox —
    /// the PROVIDER is unit-covered elsewhere; symbol extraction needs
    /// no toolchain, it is pure tree-sitter).
    #[test]
    fn crate_index_walk_and_extraction_for_go_tree() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("pkg")).unwrap();
        std::fs::write(p.join("pkg/greet.go"), "package pkg\n\nfunc Greet() {}\n").unwrap();
        std::fs::write(p.join("README.md"), "docs\n").unwrap();
        let files = AppStore::crate_source_files(p, &["go"]);
        assert_eq!(files, vec!["pkg/greet.go"]);
        let index = build_index(p, &files, None);
        assert!(index.has("pkg/greet.go"));
        assert_eq!(index.definition_count("Greet"), 1, "go symbol extraction");
    }

    /// 011-07 discriminating: a C tree indexes through the full path
    /// (language derivation from the landed `.c` file → the `c/h` walk →
    /// build_index → CrateIndexBus). HEADERS ARE DEFINITION SOURCES:
    /// the `.h` file's symbols are in the index alongside the `.c`'s.
    /// Pre-011-07 the C set was empty, so the walk found 0 files and the
    /// build never armed — the `/2)` indicator assert fails on any
    /// regression to the empty set.
    #[tokio::test]
    async fn crate_index_builds_for_c_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::create_dir_all(p.join("include")).unwrap();
        std::fs::write(
            p.join("src/lib.c"),
            "#include \"defs.h\"\nint alpha(int a) { return a; }\n",
        )
        .unwrap();
        std::fs::write(
            p.join("include/defs.h"),
            "#ifndef DEFS_H\nstruct Gamma { int x; };\nint beta(int a) { return a; }\n#endif\n",
        )
        .unwrap();
        std::fs::write(p.join("README.md"), "docs\n").unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("src/lib.c"));
        // 2 OWN source files (README.md excluded): the N/M counter is the
        // file-set's size, not the tree's.
        assert!(
            s.crate_indexing_display().contains("/2)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        assert_eq!(event.source_root, p);
        s.apply_crate_index_event(&event);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(idx.has("src/lib.c"), "crate-relative key (.c)");
        assert!(
            idx.has("include/defs.h"),
            "header (.h) IS a C definition source"
        );
        assert_eq!(idx.definition_count("alpha"), 1, "c symbol extraction");
        assert_eq!(idx.definition_count("beta"), 1, "header function extraction");
        assert_eq!(idx.definition_count("Gamma"), 1, "header struct extraction");
        assert!(!idx.has("README.md"), "non-source file not indexed");
    }

    /// 011-07 discriminating: a C++ tree indexes through the full path,
    /// landing on a HEADER (`include/api.hpp`) — the header is both a
    /// walk extension and a valid landed_file. All six registry C++
    /// extensions are walked (the `.hh` file proves the non-`.hpp` map
    /// keys).
    #[tokio::test]
    async fn crate_index_builds_for_cpp_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("include")).unwrap();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::create_dir_all(p.join("legacy")).unwrap();
        std::fs::write(p.join("include/api.hpp"), "struct Widget { int x; };\n")
            .unwrap();
        std::fs::write(
            p.join("src/widget.cc"),
            "#include \"api.hpp\"\nWidget make_widget() { return {0}; }\n",
        )
        .unwrap();
        std::fs::write(
            p.join("legacy/old.hh"),
            "int legacy_value() { return 0; }\n",
        )
        .unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("include/api.hpp"));
        assert!(
            s.crate_indexing_display().contains("/3)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        s.apply_crate_index_event(&event);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(
            idx.has("include/api.hpp"),
            "crate-relative key (.hpp, the landed header)"
        );
        assert!(idx.has("src/widget.cc"), "crate-relative key (.cc)");
        assert!(
            idx.has("legacy/old.hh"),
            ".hh map key walked, not just .hpp"
        );
        assert_eq!(idx.definition_count("Widget"), 1, "cpp struct extraction");
        assert_eq!(
            idx.definition_count("make_widget"),
            1,
            "cpp function extraction"
        );
        assert_eq!(idx.definition_count("legacy_value"), 1, "cpp header extraction");
    }

    /// 011-07 discriminating: a Markdown tree indexes through the full
    /// path. VERIFIED HONEST SCOPE (extracted, not assumed): the
    /// definition query captures both ATX headings (`# Alpha` → symbol
    /// `Alpha`) and SETEXT headings (`Delta\n====` → symbol `Delta` — the
    /// setext branch's node shape is `setext_heading → paragraph → inline`,
    /// verified against the pinned tree-sitter-md 0.5.1 grammar, re-pinned
    /// by the grammar-bumps suite). So the
    /// M-. targets in a markdown dependency are all headings: no
    /// paragraphs, no links. All three registry map keys (`md`,
    /// `markdown`, `mdx`) are walked.
    #[tokio::test]
    async fn crate_index_builds_for_markdown_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("docs")).unwrap();
        std::fs::write(p.join("README.md"), "# Alpha\n\njust a paragraph\n").unwrap();
        std::fs::write(p.join("docs/changelog.markdown"), "# Beta\n").unwrap();
        std::fs::write(p.join("docs/guide.mdx"), "# Gamma\n").unwrap();
        // Setext heading: the query's setext branch captures the heading
        // text (011-07 follow-up: the branch was structurally unmatchable
        // before and was fixed to `setext_heading → paragraph → inline`).
        std::fs::write(p.join("docs/setext.md"), "Delta\n====\n").unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("README.md"));
        assert!(
            s.crate_indexing_display().contains("/4)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        s.apply_crate_index_event(&event);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(idx.has("README.md"), "crate-relative key (.md)");
        assert!(idx.has("docs/changelog.markdown"), ".markdown map key walked");
        assert!(idx.has("docs/guide.mdx"), ".mdx map key walked");
        assert_eq!(idx.definition_count("Alpha"), 1, "atx heading extraction");
        assert_eq!(idx.definition_count("Beta"), 1, "atx heading extraction");
        assert_eq!(idx.definition_count("Gamma"), 1, "mdx atx heading extraction");
        assert!(idx.has("docs/setext.md"), "setext heading file indexed");
        assert_eq!(
            idx.definition_count("Delta"),
            1,
            "setext heading extraction (1 heading)"
        );
    }

    /// 011-07 follow-up (new-languages lane): a Java tree indexes through
    /// the full path — the `.java` walk set (the registry's only Java map
    /// key) collects the source and the outline extraction lands the
    /// class + method.
    #[tokio::test]
    async fn crate_index_builds_for_java_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(
            p.join("src/Widget.java"),
            "public class Widget {\n    public int get() { return 0; }\n}\n",
        )
        .unwrap();
        std::fs::write(p.join("README.md"), "docs\n").unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("src/Widget.java"));
        // 1 OWN source file (README.md excluded by the `java` walk set):
        // the N/M counter is the file-set's size, not the tree's.
        assert!(
            s.crate_indexing_display().contains("/1)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        s.apply_crate_index_event(&event);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(idx.has("src/Widget.java"), "crate-relative key (.java)");
        assert_eq!(idx.definition_count("Widget"), 1, "java class extraction");
        assert_eq!(idx.definition_count("get"), 1, "java method extraction");
        assert!(!idx.has("README.md"), "non-source file not indexed");
    }

    /// 011-07 follow-up (new-languages lane): a C# tree indexes through
    /// the full path — the `.cs` walk set collects the source and the
    /// outline extraction lands the namespace, class, property, and
    /// method.
    #[tokio::test]
    async fn crate_index_builds_for_csharp_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(
            p.join("src/Widget.cs"),
            "namespace App {\n    public class Widget {\n\
             \x20   public int Value { get; set; }\n\
             \x20   public int Get() => 0;\n    }\n}\n",
        )
        .unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("src/Widget.cs"));
        assert!(
            s.crate_indexing_display().contains("/1)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        s.apply_crate_index_event(&event);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(idx.has("src/Widget.cs"), "crate-relative key (.cs)");
        assert_eq!(idx.definition_count("App"), 1, "csharp namespace extraction");
        assert_eq!(idx.definition_count("Widget"), 1, "csharp class extraction");
        assert_eq!(idx.definition_count("Value"), 1, "csharp property extraction");
        assert_eq!(idx.definition_count("Get"), 1, "csharp method extraction");
    }

    /// 011-07 follow-up (new-languages lane): a Ruby tree indexes through
    /// the full path — the `.rb` walk set collects the source and the
    /// outline extraction lands the module, class, and def.
    #[tokio::test]
    async fn crate_index_builds_for_ruby_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("lib")).unwrap();
        std::fs::write(
            p.join("lib/helper.rb"),
            "module Util\n  class Helper\n    def run\n      1\n    end\n  end\nend\n",
        )
        .unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("lib/helper.rb"));
        assert!(
            s.crate_indexing_display().contains("/1)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        s.apply_crate_index_event(&event);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(idx.has("lib/helper.rb"), "crate-relative key (.rb)");
        assert_eq!(idx.definition_count("Util"), 1, "ruby module extraction");
        assert_eq!(idx.definition_count("Helper"), 1, "ruby class extraction");
        assert_eq!(idx.definition_count("run"), 1, "ruby def extraction");
    }

    /// 011-07 follow-up (new-languages lane): a Scheme tree indexes
    /// through the full path — ALL FOUR registry map keys
    /// (`scm`/`ss`/`sls`/`sld`) are walked, one fixture per key, and the
    /// flat-grammar outline extraction lands each form kind: the
    /// `define` function, the `define` variable, the `define-library`
    /// name, the nested `define`, and the `define-macro`.
    #[tokio::test]
    async fn crate_index_builds_for_scheme_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("lib")).unwrap();
        std::fs::write(
            p.join("lib/entry.scm"),
            "(define (entry-point args)\n  (void))\n",
        )
        .unwrap();
        std::fs::write(p.join("lib/core.ss"), "(define counter 0)\n").unwrap();
        std::fs::write(
            p.join("lib/defs.sld"),
            "(define-library (app core)\n  (export run!)\n  (define (run! x) x))\n",
        )
        .unwrap();
        std::fs::write(
            p.join("lib/lib.sls"),
            "(define-macro (twice a b) a)\n",
        )
        .unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("lib/entry.scm"));
        // All four walk-set extensions are OWN source files.
        assert!(
            s.crate_indexing_display().contains("/4)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        s.apply_crate_index_event(&event);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(idx.has("lib/entry.scm"), "`.scm` map key walked");
        assert!(idx.has("lib/core.ss"), "`.ss` map key walked");
        assert!(idx.has("lib/defs.sld"), "`.sld` map key walked");
        assert!(idx.has("lib/lib.sls"), "`.sls` map key walked");
        assert_eq!(
            idx.definition_count("entry-point"),
            1,
            "scheme define-fn extraction"
        );
        assert_eq!(idx.definition_count("counter"), 1, "scheme define-var extraction");
        assert_eq!(idx.definition_count("app"), 1, "scheme define-library extraction");
        assert_eq!(
            idx.definition_count("run!"),
            1,
            "scheme nested define extraction"
        );
        assert_eq!(idx.definition_count("twice"), 1, "scheme define-macro extraction");
    }

    /// 011-07 follow-up: the setext branch, pinned directly against the
    /// extractor (the e2e test above pins the same truth through the
    /// full walk -> build_index path; this is the minimal unit twin).
    /// The pinned tree-sitter-md 0.5.1 shape is
    /// `setext_heading -> paragraph -> inline`, so `Delta\n====` yields
    /// exactly one Heading symbol named `Delta` (atx and setext both
    /// extract; paragraphs and links do not).
    #[test]
    fn setext_markdown_file_contributes_heading_symbols() {
        let syms = redline_syntax::queries::extract_symbols(
            LanguageId::Markdown,
            "Delta\n====\n",
        );
        assert_eq!(syms.len(), 1, "exactly the setext heading: {syms:?}");
        assert_eq!(syms[0].name, "Delta");
        assert_eq!(syms[0].kind, redline_syntax::queries::SymbolKind::Heading);
    }

    /// 011-04: the walk is the OWNING language's, never a global "index
    /// everything" — a non-source landing (Plain) indexes nothing even
    /// though the tree is full of other languages' source files.
    #[tokio::test]
    async fn crate_index_plain_language_landing_builds_nothing() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        for rel in ["a.rs", "b.py", "c.js", "d.go"] {
            std::fs::write(p.join(rel), "x\n").unwrap();
        }
        s.start_crate_indexing(p, &p.join("notes.txt"));
        assert!(s.crate_indexing.is_empty(), "no build armed");
        assert!(s.external_indexes.is_empty(), "no cache entry");
        assert_eq!(s.message, "", "no refusal message");
    }

    /// 011-04 (item 4): the refusal cap applies per LANGUAGE set — a JS
    /// tree past `EXT_INDEX_FILE_CAP` own-language files is refused
    /// exactly as a Rust tree is (the node_modules skip bounds the
    /// transitive-dep case; the cap is the backstop for a genuinely
    /// huge dependency's own tree).
    #[tokio::test]
    async fn crate_index_refuses_oversized_js_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        for i in 0..(EXT_INDEX_FILE_CAP + 1) {
            std::fs::write(
                root.path().join(format!("f{i}.js")),
                "// x\n",
            )
            .unwrap();
        }
        s.start_crate_indexing(root.path(), &root.path().join("f0.js"));
        assert!(
            s.message.contains("crate too large to index")
                && s.message.contains("2001"),
            "{}",
            s.message
        );
        assert!(s.crate_indexing.is_empty(), "no in-flight build armed");
        assert!(s.external_indexes.is_empty(), "no cache entry");
    }

    // ── issue 05: which-function test (finding: enclosing-symbol) ──────

    #[test]
    fn apply_index_event_updates_index_and_clears_indexing() {
        let (mut s, _dir) = store_with_index(&[
            ("src/a.rs", "fn a() {}\n"),
        ]);
        // Simulate an indexing event.
        let mut new_index = SymbolIndex::new();
        new_index.set_file(
            "src/b.rs",
            vec![redline_syntax::queries::Symbol {
                name: "b".into(),
                kind: redline_syntax::queries::SymbolKind::Function,
                line: 0,
                start_byte: 3,
                end_byte: 4,
                end_line: 0,
            }],
        );
        let event = IndexEvent {
            index: new_index,
            indexing: false,
            done: 1,
            total: 1,
            generation: 0,
        };
        s.indexing = Some((0, 1, 0)); // simulate in-flight (gen=0)
        s.apply_index_event(&event);
        assert!(s.indexing.is_none(), "indexing cleared after event");
        assert_eq!(s.index().total(), 1);
        assert!(s.index().has("src/b.rs"));
    }

    // ── issue 05: Finding 1 — pending changes coalesced on flight clear ──

    #[tokio::test]
    async fn refresh_index_coalesces_pending_changes_during_flight() {
        // Set up a project with P1 and P2.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root.join("src/p1.rs"), "fn p1() {}\n").unwrap();
        std::fs::write(root.join("src/p2.rs"), "fn p2() {}\n").unwrap();

        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(&root, base.path().to_path_buf());
        s.project = Some(Project::new(root.clone()));

        // Build the initial index synchronously.
        let files_list = crate::model::files::FileList::build(&root).unwrap();
        let index = build_index(&root, &files_list.files, None);
        s.set_index(index);

        // Start an incremental refresh for P1.
        let p1 = root.join("src/p1.rs");
        s.refresh_index(std::slice::from_ref(&p1));
        // The job is now in flight (gen=0, the default).
        assert!(s.indexing.is_some(), "job is in flight after refresh_index(P1)");

        // While the job is in flight, call refresh_index with P2.
        let p2 = root.join("src/p2.rs");
        s.refresh_index(std::slice::from_ref(&p2));
        // P2 should be accumulated in the pending set (not dropped).
        assert!(
            s.pending_index_changes.contains(&p2),
            "P2 accumulated in pending set during flight"
        );

        // Drive apply_index_event to completion (the in-flight job's event).
        let event = IndexEvent {
            index: s.index.clone(),
            indexing: false,
            done: 1,
            total: 1,
            generation: 0,
        };
        s.apply_index_event(&event);

        // The pending set should be drained (P2 coalesced into a new job).
        assert!(
            s.pending_index_changes.is_empty(),
            "pending set drained after flight clear"
        );
        // A new job for P2 should now be in flight (the pending changes were
        // coalesced into one incremental job — single-flight maintained).
        assert!(
            s.indexing.is_some(),
            "new job in flight for coalesced pending changes"
        );
    }

    // ── issue 05: Finding 2 — stale-generation events discarded ──────────

    /// The incremental index filter (this issue; the R3 third walker).
    /// Each arm discriminates: a root-only `.gitignore` check would keep
    /// `src/gen/out.rs` (the NESTED rule only), and a non-ignoring
    /// filter would drop the negation (`!keep.log`) and the deletion.
    #[test]
    fn indexable_changes_filters_ignored_paths_and_keeps_negations_and_deletions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join(".gitignore"),
            "ignored.txt\nbuild/\n*.log\n!keep.log\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("src/gen")).unwrap();
        std::fs::create_dir_all(root.join("build")).unwrap();
        std::fs::create_dir_all(root.join("graft")).unwrap();
        std::fs::write(root.join("src/gen/.gitignore"), "out.rs\n").unwrap();
        for rel in [
            "src/keep.rs",
            "src/gen/out.rs",
            "src/gen/visible.rs",
            "build/out.bin",
            "ignored.txt",
            "drop.log",
            "keep.log",
            "graft/card.md",
        ] {
            std::fs::write(root.join(rel), "x\n").unwrap();
        }
        // A deleted, non-ignored file: gone from disk, still in the index
        // — the filter must NOT drop it (the `remove_file` arm needs it).
        std::fs::write(root.join("src/gone.rs"), "fn gone() {}\n").unwrap();
        std::fs::remove_file(root.join("src/gone.rs")).unwrap();

        let changed: Vec<std::path::PathBuf> = [
            "src/keep.rs",
            "src/gen/out.rs",
            "src/gen/visible.rs",
            "build/out.bin",
            "ignored.txt",
            "drop.log",
            "keep.log",
            "graft/card.md",
            "src/gone.rs",
        ]
        .iter()
        .map(|rel| root.join(*rel))
        .collect();

        let kept = AppStore::indexable_changes(&changed, root);
        let kept_rel: std::collections::BTreeSet<&str> = kept
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().to_str().unwrap())
            .collect();
        assert_eq!(
            kept_rel,
            std::collections::BTreeSet::from([
                "keep.log",          // negation (!keep.log) survives *.log
                "src/keep.rs",       // non-ignored: still indexed
                "src/gen/visible.rs", // sibling of the nested-ignored file
                "src/gone.rs",       // deleted indexed file: removal still runs
            ]),
            "only the non-ignored changes (and the indexed-file deletion) survive"
        );
    }

    /// The incremental filter mirrors the walk's `hidden(true)` (F4) and,
    /// in a git repo, `.git/info/exclude` (F1): a batch mixing hidden
    /// paths, repo-excluded paths, a `.gitignore`-ignored path, and a
    /// deletion keeps exactly what the walk would index — one shared
    /// memo across the batch (F3).
    #[test]
    fn indexable_changes_drops_hidden_and_repo_excluded_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // git repo with per-repo excludes (the walk's native source).
        std::fs::create_dir_all(root.join(".git/info")).unwrap();
        std::fs::write(root.join(".git/info/exclude"), "local/\n").unwrap();
        for rel in [
            "src/keep.rs",
            "src/gen/out.rs",
            ".venv/lib/site.py",
            "local/build.tmp",
            "src/gone.rs",
        ] {
            std::fs::create_dir_all(root.join(rel).parent().unwrap()).unwrap();
            std::fs::write(root.join(rel), "x\n").unwrap();
        }
        std::fs::write(root.join("src/gen/.gitignore"), "out.rs\n").unwrap();
        // A deleted, non-ignored file: still indexed (the `remove_file`
        // arm needs it).
        std::fs::remove_file(root.join("src/gone.rs")).unwrap();

        let changed: Vec<std::path::PathBuf> = [
            "src/keep.rs",
            "src/gen/out.rs",
            ".venv/lib/site.py",
            "local/build.tmp",
            "src/gone.rs",
        ]
        .iter()
        .map(|rel| root.join(*rel))
        .collect();

        let kept = AppStore::indexable_changes(&changed, root);
        let kept_rel: std::collections::BTreeSet<&str> = kept
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().to_str().unwrap())
            .collect();
        assert_eq!(
            kept_rel,
            std::collections::BTreeSet::from(["src/keep.rs", "src/gone.rs"]),
            "hidden (.venv/), repo-excluded (local/), and .gitignore-ignored (src/gen/out.rs) are all dropped by the walk, so all are dropped here"
        );
    }

    /// End-to-end (this issue): the incremental job must NOT parse in an
    /// ignored change (nested OR root rule), MUST reparse a non-ignored
    /// change and the negated file, and MUST still drop a deleted, indexed
    /// file. Driven through `refresh_index` → spawned job →
    /// `apply_index_event` exactly as the UI drain does.
    #[tokio::test]
    async fn refresh_index_respects_gitignore_chain_and_still_removes_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("src/gen")).unwrap();
        std::fs::create_dir_all(root.join("build")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            root.join(".gitignore"),
            "build/\n*.log\n!keep.log\ntmp.md\n!keep.md\n",
        )
        .unwrap();
        std::fs::write(root.join("src/gen/.gitignore"), "out.rs\n").unwrap();
        std::fs::write(root.join("src/gen/out.rs"), "fn gen_sym() {}\n").unwrap();
        std::fs::write(root.join("build/tool.rs"), "fn tool_sym() {}\n").unwrap();
        std::fs::write(root.join("tmp.md"), "# Tmp\n").unwrap();
        std::fs::write(root.join("keep.md"), "# Alpha\n").unwrap();
        std::fs::write(root.join("src/main.rs"), "fn kept() {}\n").unwrap();
        std::fs::write(root.join("src/aux.rs"), "fn aux() {}\n").unwrap();

        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(&root, base.path().to_path_buf());
        let files_list = crate::model::files::FileList::build(&root).unwrap();
        let index = build_index(&root, &files_list.files, None);
        // The full build already excludes the ignored files (unchanged path).
        assert!(!index.has("src/gen/out.rs"), "full build excluded the nested-ignored file");
        assert!(!index.has("build/tool.rs"), "full build excluded the root-ignored file");
        assert!(!index.has("tmp.md"));
        assert!(index.has("keep.md"), "negation: the full build keeps it");
        assert!(index.has("src/aux.rs"));
        s.set_index(index);

        let mut rx = s.index_bus.subscribe();
        // Change the ignored files (nested + root rule), the negated file,
        // and a plain file; delete the indexed `src/aux.rs`.
        std::fs::write(root.join("src/gen/out.rs"), "fn gen_changed() {}\n").unwrap();
        std::fs::write(root.join("build/tool.rs"), "fn tool_changed() {}\n").unwrap();
        std::fs::write(root.join("keep.md"), "# Beta\n").unwrap();
        std::fs::write(root.join("src/main.rs"), "fn kept2() {}\n").unwrap();
        std::fs::remove_file(root.join("src/aux.rs")).unwrap();
        s.refresh_index(&[
            root.join("src/gen/out.rs"),
            root.join("build/tool.rs"),
            root.join("keep.md"),
            root.join("src/main.rs"),
            root.join("src/aux.rs"),
        ]);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("incremental index job published within 30s");
        let event = rx.borrow_and_update().clone();
        s.apply_index_event(&event);

        let idx = s.index();
        // (a) the NESTED-ignored change was not parsed into the index.
        assert!(!idx.has("src/gen/out.rs"), "nested .gitignore must block the reparse");
        assert_eq!(idx.definition_count("gen_changed"), 0);
        // (b) the root-ignored change was not parsed into the index.
        assert!(!idx.has("build/tool.rs"), "root .gitignore must block the reparse");
        assert_eq!(idx.definition_count("tool_changed"), 0);
        assert!(!idx.has("tmp.md"));
        // (c) the NEGATED file went through the reparse (negation honored).
        assert_eq!(idx.definition_count("Beta"), 1, "!keep.md must still be reindexed");
        // (d) the non-ignored change was reparsed (filter is subtractive).
        assert_eq!(idx.definition_count("kept2"), 1, "non-ignored change must reparse");
        // A deleted, indexed (non-ignored) file still hits the Err arm.
        assert!(!idx.has("src/aux.rs"), "deletion of an indexed file must remove it");
    }

    #[test]
    fn switch_project_root_discards_stale_index_events() {
        // Create two distinct project roots.
        let dir_a = tempfile::tempdir().unwrap();
        let root_a = dir_a.path().to_path_buf();
        std::fs::create_dir_all(root_a.join("src")).unwrap();
        std::fs::write(root_a.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root_a.join("src/a.rs"), "fn a() {}\n").unwrap();

        let dir_b = tempfile::tempdir().unwrap();
        let root_b = dir_b.path().to_path_buf();
        std::fs::create_dir_all(root_b.join("src")).unwrap();
        std::fs::write(root_b.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root_b.join("src/b.rs"), "fn b() {}\n").unwrap();

        // Create a store rooted at project A.
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(&root_a, base.path().to_path_buf());
        s.project = Some(Project::new(root_a.clone()));

        // Simulate A's index job in flight (gen=0).
        s.indexing = Some((0, 1, 0));

        // Switch to project B: generation is bumped, index reset.
        s.switch_project_root(root_b.to_str().unwrap());
        assert_eq!(s.index_generation, 1, "generation bumped on project switch");
        assert_eq!(s.index().total(), 0, "index reset on project switch");

        // Simulate A's job completing (stale event, gen=0).
        let mut a_index = SymbolIndex::new();
        a_index.set_file(
            "src/a.rs",
            vec![redline_syntax::queries::Symbol {
                name: "a".into(),
                kind: redline_syntax::queries::SymbolKind::Function,
                line: 0,
                start_byte: 3,
                end_byte: 4,
                end_line: 0,
            }],
        );
        let stale_event = IndexEvent {
            index: a_index,
            indexing: false,
            done: 1,
            total: 1,
            generation: 0, // stale: from project A
        };
        s.apply_index_event(&stale_event);
        // The stale event must be discarded: index stays empty.
        assert_eq!(
            s.index().total(),
            0,
            "stale event from project A discarded"
        );

        // Simulate B's job completing (current event, gen=1).
        let mut b_index = SymbolIndex::new();
        b_index.set_file(
            "src/b.rs",
            vec![redline_syntax::queries::Symbol {
                name: "b".into(),
                kind: redline_syntax::queries::SymbolKind::Function,
                line: 0,
                start_byte: 3,
                end_byte: 4,
                end_line: 0,
            }],
        );
        let b_event = IndexEvent {
            index: b_index,
            indexing: false,
            done: 1,
            total: 1,
            generation: 1, // current: from project B
        };
        s.apply_index_event(&b_event);
        // B's event must be applied: index contains B's symbols, not A's.
        assert_eq!(s.index().total(), 1, "B's event applied");
        assert!(s.index().has("src/b.rs"), "B's index contains B's file");
        assert!(!s.index().has("src/a.rs"), "A's file not in B's index");
    }

    // ── search & references (issue 06) ───────────────────────────────

    #[tokio::test]
    async fn checkout_branch_bumps_generation_and_starts_full_rebuild() {
        // A clean repo with a second branch so checkout succeeds (the
        // dirty-tree guard requires a clean tree; the branch must exist).
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        git_repo_init(&root, "Test", "test@example.com", true);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git_cli(&root, &["add", "a.txt"], "Test", "test@example.com");
        git_cli(&root, &["commit", "-q", "-m", "init"], "Test", "test@example.com");
        git_cli(&root, &["branch", "feature"], "Test", "test@example.com");

        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(&root, base.path().to_path_buf());
        s.project = Some(Project::new(root.clone()));

        // Build an initial index so start_indexing has files to index.
        let files_list = crate::model::files::FileList::build(&root).unwrap();
        let index = build_index(&root, &files_list.files, None);
        s.set_index(index);

        // The hole: a current-generation (gen 0) job is already in flight and
        // there are pending changes. Without the fix, checkout's start_indexing
        // would early-return on the single-flight guard and the full rebuild
        // would be silently skipped.
        s.indexing = Some((0, 1, 0));
        s.pending_index_changes.insert(root.join("a.txt"));

        s.checkout_branch("feature");

        // The generation must be bumped so the in-flight gen-0 job is now stale
        // (its event is discarded), and the pending set cleared.
        assert_eq!(s.index_generation, 1, "generation bumped on checkout");
        assert!(
            s.pending_index_changes.is_empty(),
            "pending changes cleared on checkout"
        );
        // A new full-rebuild job must be in flight for the new generation,
        // proving the single-flight guard did NOT skip the rebuild.
        let Some((_, _, job_gen)) = s.indexing else {
            panic!("no index job in flight after checkout");
        };
        assert_eq!(job_gen, 1, "full-rebuild job started for the new generation");
        // HEAD actually moved to the target branch.
        let head = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&head.stdout).trim(),
            "feature",
            "HEAD moved to feature"
        );
    }

    /// A disk reload gives the buffer a new mtime: the retained tree's
    /// (buffer, mtime) key no longer matches, so the rebuild is a full
    /// parse — the stale-tree fallback.
    #[test]
    fn reload_breaks_the_retained_tree_epoch() {
        let mut store = store_with_project();
        open_ann_file(&mut store, "notes.md", "# t\n");
        let key = store.buffers.current().unwrap().to_string();
        store.buffers.get_mut(&key).unwrap().editable = true;
        for c in "x\n".chars() {
            store.notes_insert_char(c);
        }
        store.ensure_highlight();
        let mtime1 = store.buffers.get(&key).unwrap().mtime;
        assert!(
            store.highlight_cache.retain_contains(&TreeKey::new(&key, mtime1)),
            "edited buffer must have a retained tree"
        );
        // Simulate `reload_buffer`: new content from disk, new mtime.
        let new_mtime =
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(4_000_000_000);
        {
            let buf = store.buffers.get_mut(&key).unwrap();
            buf.rope = Rope::from_str("# t\nreloaded\n");
            buf.mtime = new_mtime;
            buf.locally_modified = false;
        }
        assert!(
            !store.highlight_cache.retain_contains(&TreeKey::new(&key, new_mtime)),
            "a reloaded buffer must not find its stale retained tree"
        );
        store.ensure_highlight();
        assert!(
            store.highlight_cache.retain_contains(&TreeKey::new(&key, new_mtime)),
            "the rebuilt full parse must be retained under the new epoch"
        );
    }

    #[test]
    fn indexing_display_full_build_shows_advancing_counter() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        // Simulate a full index in progress: 50 done of 100.
        store.indexing = Some((50, 100, 0));
        store.indexing_incremental = false;
        assert_eq!(store.indexing_display(), "indexing 50/100");
        // Advance.
        store.indexing = Some((75, 100, 0));
        assert_eq!(store.indexing_display(), "indexing 75/100");
    }

    #[test]
    fn indexing_display_incremental_is_silent() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        // Incremental refreshes are fast; flashing an indicator per
        // file-change batch on a busy repo is noise, so they render nothing.
        store.indexing = Some((3, 3, 1));
        store.indexing_incremental = true;
        assert_eq!(store.indexing_display(), "");
        // The single-flight state is untouched by the display decision.
        assert!(store.indexing.is_some());
    }

    #[test]
    fn indexing_display_idle_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        assert_eq!(store.indexing_display(), "");
    }

    #[test]
    fn progress_publisher_emits_every_k_files() {
        use crate::nav::index::{IndexBus, IndexProgress, PROGRESS_STEP};
        let bus = IndexBus::new();
        let mut rx = bus.subscribe();
        let progress = IndexProgress::new(100).with_publisher(bus, 42);
        // Simulate 25 files done: should emit one progress event.
        for _ in 0..PROGRESS_STEP {
            progress.note_file_done();
        }
        assert_eq!(progress.done(), PROGRESS_STEP);
        // The bus should have received a progress event.
        let event = rx.borrow_and_update().clone();
        assert!(event.indexing, "progress event must have indexing=true");
        assert_eq!(event.done, PROGRESS_STEP);
        assert_eq!(event.total, 100);
        assert_eq!(event.generation, 42);
    }

    // ── issue 002: transient menu + discard ──────────────────────────────

