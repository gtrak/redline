use super::*;
use crate::app::keymap::parse_key;
use crate::git::blame::BlameLine;
use crate::git::log::LogEntry;

    /// Shared git CLI for the store tests. The author identity is
    /// parameterized: the standard fixture is "Test"/"test@example.com"; the
    /// magit windowing test historically used the short "T"/"t@e.com" (it only
    /// sets commit metadata, so it is kept verbatim rather than collapsed).
    /// The GIT_CONFIG_* vars make every call hermetic: host identity/config
    /// can never leak in.
    fn git_cli(dir: &std::path::Path, args: &[&str], name: &str, email: &str) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", name)
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_COMMITTER_NAME", name)
            .env("GIT_COMMITTER_EMAIL", email)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Shared repo-priming sequence (init + identity config). `gpgsign_false`
    /// mirrors which tests configured `commit.gpgsign` — the magit windowing
    /// test never did, so it stays honest to its original body.
    fn git_repo_init(dir: &std::path::Path, name: &str, email: &str, gpgsign_false: bool) {
        git_cli(dir, &["init", "-q", "-b", "main"], name, email);
        git_cli(dir, &["config", "user.name", name], name, email);
        git_cli(dir, &["config", "user.email", email], name, email);
        if gpgsign_false {
            git_cli(dir, &["config", "commit.gpgsign", "false"], name, email);
        }
    }

    /// A store rooted in `dir` as the project start, with persistence
    /// under a throwaway sibling base (never the project dir itself —
    /// the walk must not see the persistence files — and never the
    /// real cache dir).
    fn store(dir: &std::path::Path) -> AppStore {
        let base = tempfile::tempdir().unwrap();
        AppStore::at(dir, base.path().to_path_buf())
    }

    fn key(s: &str) -> Key {
        parse_key(s).unwrap()
    }

    /// A store with an open notes buffer (the one UI-reachable modified
    /// buffer) for the quit save-prompt tests (plan 004 issue 04).
    fn notes_store() -> (tempfile::TempDir, AppStore) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        (dir, s)
    }

    /// Two modified, SAVEABLE buffers: the notes buffer plus a second
    /// editable-with-path buffer (the UI only exposes notes, so the second is
    /// built through the buffer API to exercise the multi-buffer prompt).
    fn two_modified_buffers(
        dir: &std::path::Path,
        s: &mut AppStore,
    ) -> (String, std::path::PathBuf) {
        let extra = dir.join("extra.md");
        std::fs::write(&extra, "old\n").unwrap();
        let mtime = std::fs::metadata(&extra)
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let extra_key = s
            .buffers
            .insert_rope(Some(extra.clone()), Rope::from_str("old\n"), mtime, true);
        s.mark_locally_modified(&extra_key);
        // The notes buffer (opened LAST → most recent) also gets an edit.
        s.key_event(key("z"));
        (extra_key, extra)
    }

    /// A store with a real file buffer open (`src/f.rs`): the toggle/save
    /// tests' fixture. The file content is `fn old() {}\n`.
    fn file_buffer_store() -> (tempfile::TempDir, AppStore) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/f.rs"), "fn old() {}\n").unwrap();
        let mut s = store(dir.path());
        s.open_path("src/f.rs");
        (dir, s)
    }

    /// 006-02b item 1 fixture: a project store with an external (outside-
    /// root) file open read-only via the tooling-landing path.
    fn store_with_external_buffer() -> (tempfile::TempDir, tempfile::TempDir, AppStore) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut s = store(dir.path());
        let ext = tempfile::tempdir().unwrap();
        let abs = ext.path().join("registry_src.rs");
        std::fs::write(&abs, "pub fn spawn<F>(f: F) {}\n").unwrap();
        s.open_external_path(&abs).unwrap();
        (dir, ext, s)
    }

    fn project_with_files(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.join("README.md"), "# readme\n").unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "// lib\n").unwrap();
    }

    fn store_with_lines(n_lines: usize) -> (AppStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut content = String::new();
        for i in 0..n_lines {
            content.push_str(&format!("line{}\n", i));
        }
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/big.rs"), &content).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/big.rs");
        s.set_viewport_lines(10);
        (s, dir)
    }

    /// A store with a file buffer whose lines have varying lengths (for
    /// goal-column / EOL-BOL-wrap coverage): a 2-char line between two
    /// 12-char lines.
    fn store_with_varied_lines() -> (AppStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let content = "aaaaaaaaaaaa\nbb\ncccccccccccc"; // 12 / 2 / 12 (3 lines)
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/v.rs"), content).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/v.rs");
        s.set_viewport_lines(10);
        (s, dir)
    }

    /// A store with a word-motion fixture: words, punctuation runs, an
    /// empty-line boundary, and wrap cases (no trailing newline).
    fn store_with_words() -> (AppStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        // Line 0: "hello world_foo!!"  (words: hello, world_foo)
        // Line 1: "x"                  (single-char word)
        // Line 2: "ab cd"              (two words)
        let content = "hello world_foo!!\nx\nab cd";
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/w.rs"), content).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/w.rs");
        s.set_viewport_lines(10);
        (s, dir)
    }

    fn change(paths: Vec<std::path::PathBuf>) -> ProjectChange {
        let n = paths.len();
        ProjectChange {
            seq: 1,
            paths,
            kinds: vec![crate::app::events::ChangeKind::Modify; n],
        }
    }

    fn jump_entry(buffer_key: &str, line: usize) -> JumpEntry {
        JumpEntry {
            buffer_key: buffer_key.to_string(),
            line,
            col: 0,
            label: "test".to_string(),
        }
    }

    fn store_with_index(files: &[(&str, &str)]) -> (AppStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        for (rel, content) in files {
            let path = dir.path().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, content).unwrap();
        }
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        // Build the index synchronously (no tokio runtime in unit tests).
        let files_list = crate::model::files::FileList::build(dir.path()).unwrap();
        let root = dir.path().to_path_buf();
        let index = build_index(&root, &files_list.files, None);
        s.set_index(index);
        (s, dir)
    }

    /// The point-line token extractor: identifier run at the column + the
    /// path token around it (011-06: language-aware path token).
    fn satp(lang: LanguageId, text: &str, col: usize) -> Option<(String, String)> {
        AppStore::symbol_at_point(lang, text, col)
    }

    /// 006-03 fixture: a project store (one project file) plus an
    /// EXTERNAL crate source tree (outside the project root) with a
    /// synchronously installed crate index. Returns (store, project
    /// dir, crate root).
    fn store_with_crate_index(files: &[(&str, &str)]) -> (AppStore, tempfile::TempDir, tempfile::TempDir) {
        let (mut s, dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        for (rel, content) in files {
            let path = root.path().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, content).unwrap();
        }
        let crate_files = AppStore::crate_source_files(root.path(), &["rs"]);
        let index = build_index(root.path(), &crate_files, None);
        s.apply_crate_index_event(&CrateIndexEvent {
            source_root: root.path().to_path_buf(),
            index,
        });
        (s, dir, root)
    }

    /// A store rooted in a project with known search content.
    fn search_project() -> (tempfile::TempDir, AppStore) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn target() {}\nfn main() { target(); }\ntarget();\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn target() {}\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let store = AppStore::at(dir.path(), base.path().to_path_buf());
        (dir, store)
    }

    /// Drain the search bus into the store until `Finished` (bounded).
    fn drain_search_finished(
        store: &mut AppStore,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::search::rg::SearchEvent>,
    ) {
        let start = std::time::Instant::now();
        loop {
            let mut finished = false;
            while let Ok(ev) = rx.try_recv() {
                if matches!(ev, crate::search::rg::SearchEvent::Finished { .. }) {
                    store.apply_search_event(&ev);
                    finished = true;
                    break;
                }
                store.apply_search_event(&ev);
            }
            if finished {
                return;
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "search did not finish in 5s"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Helper: create a tempdir git repo with one committed file and a
    /// staged change, return the store rooted there.
    fn git_store_with_staged(dir: &std::path::Path) -> AppStore {
        git_repo_init(dir, "Test", "test@example.com", true);
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        git_cli(dir, &["add", "a.txt"], "Test", "test@example.com");
        git_cli(dir, &["commit", "-q", "-m", "init"], "Test", "test@example.com");
        // Stage a change so the commit editor has a file list.
        std::fs::write(dir.join("a.txt"), "a\nA\n").unwrap();
        git_cli(dir, &["add", "a.txt"], "Test", "test@example.com");
        let base = tempfile::tempdir().unwrap();
        AppStore::at(dir, base.path().to_path_buf())
    }

    // ── issue 08: Finding 1 — checkout must trigger the full symbol-index
    // rebuild even when a current-generation job is already in flight ──────

    /// Walk the whole menu tree from `path`, collecting every reachable leaf
    /// command name (descending into prefixes).
    fn collect_menu_leaves(
        store: &AppStore,
        path: &crate::app::keymap::KeySeq,
    ) -> std::collections::HashSet<String> {
        let mut set = std::collections::HashSet::new();
        for e in store.menu_entries_for_path(path) {
            match &e.command {
                Some(cmd) => {
                    set.insert(cmd.clone());
                }
                None => {
                    let mut child = path.clone();
                    child.push(e.key);
                    set.extend(collect_menu_leaves(store, &child));
                }
            }
        }
        set
    }

    /// Shared git CLI wrapper for the windowing tests (the original windowing
    /// fixture identity "Test"/"t@e.com", kept verbatim).
    fn git_test_cli(dir: &std::path::Path, args: &[&str]) {
        git_cli(dir, args, "Test", "t@e.com");
    }

    /// A git repo with a base commit and one "tall" commit that grows a file
    /// to 60 lines, so the selected commit's diff overflows a small viewport.
    fn tall_commit_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git_repo_init(dir.path(), "Test", "t@e.com", true);
        std::fs::write(dir.path().join("big.txt"), "l1\n").unwrap();
        git_test_cli(dir.path(), &["add", "-A"]);
        git_test_cli(dir.path(), &["commit", "-q", "-m", "base"]);
        let mut content = String::new();
        for i in 1..=60 {
            content.push_str(&format!("line {i}\n"));
        }
        std::fs::write(dir.path().join("big.txt"), content).unwrap();
        git_test_cli(dir.path(), &["add", "-A"]);
        git_test_cli(dir.path(), &["commit", "-q", "-m", "tall"]);
        dir
    }

    fn git_store(dir: &std::path::Path) -> AppStore {
        git_repo_init(dir, "Test", "test@example.com", true);
        std::fs::write(dir.join("a.txt"), "keep\n").unwrap();
        git_cli(dir, &["add", "a.txt"], "Test", "test@example.com");
        git_cli(dir, &["commit", "-q", "-m", "init"], "Test", "test@example.com");
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir, base.path().to_path_buf());
        s.project = Some(crate::model::project::Project::new(dir.to_path_buf()));
        s
    }

    /// A store with a multi-line editable buffer (notes) for mark/region tests.
    fn notes_store_with_lines(n_lines: usize) -> (AppStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        // Type n_lines lines into the notes buffer.
        for _ in 0..n_lines {
            s.notes_insert_char('x');
            s.notes_insert_char('\n');
        }
        (s, dir)
    }

    /// A store rooted at a temp project WITH an open project (a Cargo.toml
    /// marker), so notes-key / rel-path machinery works. The temp project
    /// dir is leaked (OS-cleaned on exit, like the persistence base).
    fn store_with_project() -> AppStore {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let dir_path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let base = tempfile::tempdir().unwrap();
        AppStore::at(&dir_path, base.path().to_path_buf())
    }

    /// Open a file buffer in the store via the open_path seam.
    fn open_ann_file(store: &mut AppStore, rel: &str, content: &str) {
        let root = store.project.as_ref().unwrap().root.clone();
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
        store.open_path(rel);
        assert_eq!(store.top_view(), ViewId::Buffer);
    }

    fn ann_records(store: &AppStore) -> Vec<&Annotation> {
        store
            .notes_doc
            .entries
            .iter()
            .filter_map(|e| e.as_record())
            .collect()
    }

    /// An out-of-project file, opened read-only the way `M-.` lands it.
    /// Returns the store + the file's absolute path string.
    fn store_with_external_file() -> (AppStore, String) {
        let mut s = store_with_project();
        let ext = tempfile::tempdir().unwrap();
        let abs = ext.path().join("lib_source.rs");
        std::fs::write(&abs, "ext line one\next line two\next line three\n").unwrap();
        let abs_str = abs.to_string_lossy().into_owned();
        let key = s
            .open_external_path(&abs)
            .expect("external file opens read-only");
        assert!(s.buffers.get(&key).unwrap().path.is_some());
        (s, abs_str)
    }

    fn dump_item(path: &str, line: usize, code: &str, text: &str, orphaned: bool) -> DumpAnnotation {
        DumpAnnotation {
            path: path.to_string(),
            line,
            code: code.to_string(),
            text: text.to_string(),
            orphaned,
        }
    }

    /// (line, col) of a byte offset inside `src`.
    fn point_of(src: &str, at: usize) -> (usize, usize) {
        let line = src[..at].matches('\n').count();
        let col = at - src[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
        (line, col)
    }

mod views;
mod buffers;
mod notes;
mod file_view;
mod search;
mod magit;
mod commit;
mod navigation;
mod index_wiring;
mod picker;
mod project;
mod minibuffer;
mod keys;
mod core;
