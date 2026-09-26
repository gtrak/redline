use super::*;

    #[tokio::test]
    async fn xref_resolver_hit_opens_tooling_picker_and_ret_lands_read_only() {
        // (jump-ambiguity, test d) a resolve event with a resolved source
        // (outside the project root, external) does NOT jump silently —
        // it joins the Xref picker as the preselected, `tooling`-marked
        // row; RET lands read-only: point on the resolved line, jump
        // entry recorded, status cleared, and the external path never
        // enters the project recents.
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
        // (jump-ambiguity) the ASYNC origin: the point moves while the
        // resolve is in flight — the picker's origin must be the M-.
        // press position (line 0, col 2), not this moved point.
        s.set_point(0, 1, 1);
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
        // The tooling row: the picker is open, the row is PRESELECTED
        // (row 0) and marked `tooling` (the weak resolve is visible, not
        // a mystery).
        assert!(s.picker_open(), "the tooling hit joins the picker");
        assert_eq!(s.picker_kind(), Some(PickerKind::Xref));
        let abs = ext.path().to_string_lossy().into_owned();
        let filtered = s.picker_filtered();
        assert!(
            filtered[0].0.name == format!("{abs}:2"),
            "the tooling row is preselected: {:?}",
            filtered[0].0.name
        );
        assert!(
            filtered[0].0.detail.contains("tooling"),
            "the row is marked tooling: {:?}",
            filtered[0].0.detail
        );
        assert!(s.resolving_display().is_empty(), "status activity cleared");
        // RET accepts the top guess: the external source lands read-only.
        s.run_selected();
        let key = s.buffers.current().map(String::from).unwrap();
        assert_eq!(key, abs, "external buffer is current");
        let buf = s.buffers.get(&key).unwrap();
        assert!(!buf.editable, "external source is read-only");
        assert_eq!(s.point_line(), 1, "point on the resolved (1-based line 2) definition");
        // jump-column-landings: the app-side refinement lands on the
        // resolved item's name column (`pub fn |spawn` → col 7), not the
        // line start (col 0) — the pre-fix col-0 landing would fail here.
        assert_eq!(s.point_col(), 7, "the `spawn` name's column, not col 0");
        assert!(s.message.contains("jumped to"), "jump report, got: {}", s.message);
        assert_eq!(
            s.project_store.recents.list(&root).to_vec(),
            recents_before,
            "external landing never records a recent"
        );
        // The jump origin is the M-. PRESS position (line 0, col 2) —
        // not the moved point (line 0, col 1) the picker was opened
        // under.
        s.jump_back();
        assert_eq!(s.buffers.current().map(String::from).unwrap(), origin_key, "jump-back returns to the origin");
        assert_eq!((s.point_line(), s.point_col()), (0, 2), "jump-back lands at the M-. press point (async origin)");
    }

    #[tokio::test]
    async fn xref_resolver_miss_reports_graceful_message() {
        // All providers miss → the event carries the error; the message
        // reports it and nothing else changes (no buffer, no jump, no
        // panic). F2 (plan 017 audit): the single-attempt shape — the
        // provider's own reason LEADS the report, the generic tail
        // follows byte-for-byte.
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
            error: Some("no crate `tokio`; no tooling provider could resolve symbol `tokio::spawn` (tried 1 provider(s): rust)".into()),
        };
        s.apply_resolve_event(&event);
        assert!(
            s.message.starts_with("no provider resolution for `tokio::spawn`: no crate `tokio`;"),
            "the provider's own reason reaches the minibuffer: got: {}",
            s.message
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
        // F2 (plan 017 audit): the REAL chain's event carries the cargo
        // provider's OWN miss reason — it reaches the minibuffer (it used
        // to be swallowed by the chain-level generic; a miss with a named
        // reason must read differently from a bare miss).
        assert!(
            s.message.contains("not a cargo project"),
            "the provider's own reason surfaces: {}", s.message
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
        s.start_symbol_resolution("Deserialize", "src/main.rs", None);
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
        s.start_symbol_resolution("plain_local_name", "src/main.rs", None);
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
        s.start_symbol_resolution("legacyJoin", "src/app.js", None);
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
        // (jump-ambiguity) the tooling hit joins the Xref picker
        // (marked row, preselected) — RET accepts it and the view is now
        // the sibling file.
        s.apply_resolve_event(&event);
        assert!(s.picker_open(), "the tooling hit joins the picker");
        assert!(
            s.picker_filtered()[0].0.name.starts_with("src/legacy-util.js:"),
            "the sibling's tooling row is preselected: {:?}",
            s.picker_filtered()[0].0.name
        );
        s.run_selected();
        assert_eq!(
            s.view_name_display(),
            "src/legacy-util.js",
            "the view is now the sibling file"
        );
        let cur: &str = s.buffers.current().expect("a current buffer");
        assert!(
            !s.external_buffers.contains(cur),
            "the landed sibling is an EDITABLE project buffer (the project branch of the tooling landing), never the read-only external one"
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
        // (jump-ambiguity) `target` is a UNIQUE CROSS-FILE candidate →
        // the picker (best preselected); the supersede bump still fires.
        s.set_point_line(1);
        s.xref_find_definitions();
        assert_eq!(s.resolve_generation, in_flight + 1, "the hit superseded the in-flight resolve");
        assert!(s.picker_open(), "cross-file unique: picker");
        s.run_selected();
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
        assert!(!s.picker_open(), "a stale resolve must not open a picker");
    }

    #[tokio::test]
    async fn xref_resolver_line_zero_opens_tooling_picker_top_of_file() {
        // 006-02b item 6 + jump-ambiguity: a provider emitting line 0 is
        // treated as "no line" — the row shows the TOP OF THE FILE as the
        // (visible, weak) landing, preselected; RET lands there (no
        // underflow, no silent jump).
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
        let abs = ext.path().to_string_lossy().into_owned();
        assert!(s.picker_open(), "the tooling hit joins the picker");
        assert!(
            s.picker_filtered()[0].0.name == format!("{abs}:1"),
            "the weak resolve shows the top of the file, preselected: {:?}",
            s.picker_filtered()[0].0.name
        );
        s.run_selected();
        assert_eq!(s.point_line(), 0, "line 0 lands at the top of the file");
        assert!(s.message.contains("jumped to"), "msg: {}", s.message);
    }

    // ── plan 006 issue 03: navigate within external (crate) sources ────
