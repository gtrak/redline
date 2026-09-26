//! loop-03: below-PTY unit twins of `tools/sweep_flows.py`'s flows.
//!
//! Every twin drives the store through the SAME entry points the PTY driver
//! targets (`AppStore::key_event` for keystrokes — the exact `Key` the
//! Root component's terminal-event path produces — and `AppStore::
//! apply_project_change` for watcher deliveries, the same `ProjectChange`
//! the watcher task publishes), and asserts the flow's positive signals on
//! store state plus the width-bounded static render
//! (`crate::ui::root::render_at_width(store, 80)`, the 80x24 PTY matrix
//! contract) — never on looser state than the PTY leg asserted.
//!
//! Naming: `unit_flow_<sweep name>` mirrors the sweep's flow name so the
//! mapping is auditable (the kept/converted ledger lives in
//! `docs/ux-testing-plan.md`). What stays in the thin PTY tier: input
//! encoding through the real terminal, frame cadence / repaint races
//! (ann-delete transient echo, U-BHN debounce), process lifecycle
//! (quit-dump, exit codes), resize, mouse, the raw CUP stream, and the
//! in-flight search cancel (U-H2 search) whose discriminating condition
//! (C-g landing while a real rg walk is in flight) is a wall-clock race
//! no store call can reproduce.

#![allow(clippy::too_many_arguments)]

use super::{AppStore, ViewId};
use crate::app::events::{ChangeKind, ProjectChange};
use crate::app::keymap::{parse_key, Key};

/// Parse a single key token (`"C-x"`, `"RET"`, `"^@"` = NUL, …).
fn key(s: &str) -> Key {
    parse_key(s).unwrap()
}

/// A printable char key (a bare space is not nameable by `parse_key`).
fn key_char(c: char) -> Key {
    if c == ' ' {
        Key::char(' ')
    } else {
        parse_key(&c.to_string()).unwrap()
    }
}

/// C-SPC: the terminal delivers it as the NUL byte, which crossterm/iocraft
/// decode as `Char(' ') + CONTROL` (the store's set-mark binding).
fn key_null() -> Key {
    Key::ctrl_char(' ')
}

/// Build the symbol index synchronously and install it (no tokio runtime
/// in unit tests — the UI's drain task can't run, so the index job's
/// output is produced directly; `store_with_index` in the `store` test
/// module does the
/// same).
fn install_index(s: &mut AppStore, root: &std::path::Path) {
    let files_list = crate::model::files::FileList::build(root).unwrap();
    let index = crate::nav::index::build_index(root, &files_list.files, None);
    s.set_index(index);
}

/// A store rooted in `dir` with persistence under a throwaway sibling
/// base, the PTY-matrix viewport (24-row terminal → 21 content rows).
fn store_in(dir: &std::path::Path) -> AppStore {
    let base = tempfile::tempdir().unwrap();
    let mut s = AppStore::at(dir, base.path().to_path_buf());
    s.set_viewport_lines(21);
    s
}

/// The width-bounded static render of a frame (80 cols — the PTY matrix
/// bold cell). Consumes the store; assert state first, render last.
fn render80(store: AppStore) -> String {
    crate::ui::root::render_at_width(store, 80)
}

/// Whitespace-collapsed frame text (the sweep's `flat_text`: a prompt or
/// message that wraps past 80 cols must not defeat substring asserts).
fn flat(frame: &str) -> String {
    frame
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn rows_containing(frame: &str, token: &str) -> Vec<usize> {
    frame
        .lines()
        .enumerate()
        .filter_map(|(i, l)| l.contains(token).then_some(i))
        .collect()
}

/// Run git in `dir`; panic on failure (fixture hygiene). Delegates to the
/// crate-wide harness: the standard "Test"/"test@example.com" fixture
/// identity plus the hermetic env block. Note this file's two former copies
/// set NO env at all, so they inherited the developer's `~/.gitconfig` —
/// routing them through `crate::test_support` is what fixes that.
fn git(dir: &std::path::Path, args: &[&str]) {
    crate::test_support::git_cli(dir, args, "Test", "test@example.com");
}

/// Git stdout (read-only fixture checks: diff/log/status).
fn git_out(dir: &std::path::Path, args: &[&str]) -> String {
    crate::test_support::git_cli(dir, args, "Test", "test@example.com")
}

/// `src/main.rs` content mirroring the PTY fixture's expectations: the
/// `target_one` symbol at the top (which-function), `target` hits for the
/// search flows, multiple lines for the mark/region flows.
const MAIN_RS: &str = "pub fn target_one() -> u32 {\n    1\n}\n\nfn target() -> u32 {\n    target_one()\n}\n\nfn main() {\n    let a = target();\n    let b = target_one();\n    println!(\"hello {a} {b}\");\n}\n\n// filler lines for the motion flows\n// two\n// three\n// four\n// five\n";

/// A tempdir git repo mirroring the PTY sweep's baseline (`/tmp/redline_pyte_repo`
/// after `tools/fixture.py`): log `init` → `commit 4` → `commit 5`, and a
/// working tree with a STAGED change on `src/lib.rs` and an UNSTAGED change
/// on `README.md` (the sweep's `+1 ~1` baseline).
fn fixture_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(p.join("Cargo.toml"), "[package]\n").unwrap();
    std::fs::write(p.join("README.md"), "# commit fixture\n").unwrap();
    std::fs::write(p.join("src/main.rs"), MAIN_RS).unwrap();
    std::fs::write(
        p.join("src/lib.rs"),
        "pub fn target_lib() {}\npub fn other_lib() {}\n",
    )
    .unwrap();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Test"]);
    git(p, &["config", "user.email", "test@example.com"]);
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "init"]);
    std::fs::write(
        p.join("src/lib.rs"),
        "pub fn target_lib() {}\npub fn other_lib() {}\n// c4\n",
    )
    .unwrap();
    git(p, &["commit", "-q", "-am", "commit 4"]);
    std::fs::write(p.join("README.md"), "# commit fixture\nmore\n").unwrap();
    git(p, &["commit", "-q", "-am", "commit 5"]);
    // Working-tree baseline: staged src/lib.rs + unstaged README.md.
    append(p.join("src/lib.rs").as_path(), "staged_change_marker\n");
    git(p, &["add", "src/lib.rs"]);
    append(p.join("README.md").as_path(), "unstaged_change_marker\n");
    dir
}

fn append(path: &std::path::Path, text: &str) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

/// The magit-context fixture (U-CDS / U-BLW): a throwaway repo with a base
/// commit and one tall commit (`big.txt` → 60 lines + sentinel), mirroring
/// the sweep's `_ensure_win_diff_repo`.
fn win_diff_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::write(p.join("big.txt"), "l1\n").unwrap();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Test"]);
    git(p, &["config", "user.email", "test@example.com"]);
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "base"]);
    let mut tall = String::new();
    for i in 1..=60 {
        tall.push_str(&format!("line {i}\n"));
    }
    tall.push_str("BOTTOM_SENTINEL\n");
    std::fs::write(p.join("big.txt"), tall).unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "tall-lines"]);
    dir
}

/// Open `path` (project-relative) through the find-file picker, the same
/// key path the PTY legs drive.
fn open_via_finder(store: &mut AppStore, query: &str) {
    store.key_event(key("C-x"));
    store.key_event(key("C-f"));
    assert!(store.picker_open(), "find-file picker must open");
    for c in query.chars() {
        store.key_event(key_char(c));
    }
    store.key_event(key("RET"));
    assert!(!store.picker_open(), "RET must close the picker");
}

/// Drain the search bus into the store until `Finished` (bounded) — the
/// unit analogue of the PTY's `wait_done`.
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
            start.elapsed() < std::time::Duration::from_secs(10),
            "search did not finish in 10s"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Publish a single-path change the way the watcher task does.
fn publish_change(store: &mut AppStore, path: &std::path::Path, kind: ChangeKind) {
    store.apply_project_change(&ProjectChange {
        seq: 1,
        paths: vec![path.to_path_buf()],
        kinds: vec![kind],
    });
}

/// True when the current buffer is the notes buffer (the only UI-reachable
/// locally-owned buffer in the sweep).
fn current_is_notes(s: &AppStore) -> bool {
    s.buffers
        .current_buffer()
        .and_then(|b| b.path.as_ref())
        .map(|p| p.file_name().map(|n| n == ".redline-notes.md").unwrap_or(false))
        .unwrap_or(false)
}

// ════════════════════════════ batch 1: home / finders / pending ═══════

/// U-A1 Cold start: HOME frame — project + "redline" header, the derived
/// command groups, the standard help line, the status line, and ready.
/// `*scratch*` no longer exists at boot.
#[test]
fn unit_flow_a1() {
    let repo = fixture_repo();
    let name = repo.path().file_name().unwrap().to_string_lossy().to_string();
    let frame = render80(store_in(repo.path()));
    let lines: Vec<&str> = frame.lines().collect();
    // Header (row 0): project + "redline".
    assert!(lines[0].contains("redline") && lines[0].contains(&name), "header: {frame}");
    // The derived command groups, each exactly once (no-wrap rows).
    let once = |t: &str| rows_containing(&frame, t).len() == 1;
    for cat in ["[buffers]", "[files]", "[git]"] {
        assert!(once(cat), "category {cat} exactly once:\n{frame}");
    }
    assert!(
        frame.contains("C-x g") && frame.contains("C-x C-f") && frame.contains("C-x C-c"),
        "group key sequences: {frame}"
    );
    // Help line + ready minibuffer.
    assert!(frame.contains("C-x C-c quit") && frame.contains("? menu"), "{frame}");
    assert!(frame.contains("ready"), "{frame}");
    // Status line: "* <project> *  home", on the last row.
    assert!(
        lines.last().unwrap().contains(&format!("* {name} *  home")),
        "status line: {:?}",
        lines.last()
    );
    assert!(!frame.contains("*scratch*"), "no auto-created scratch: {frame}");
    // Boot state.
    let s = store_in(repo.path());
    assert_eq!(s.top_view(), ViewId::Home);
    assert!(!s.quit);
}

/// U-B1: `C-x C-f` prompt, filter, RET opens the highlighted file.
#[test]
fn unit_flow_b1() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("C-f"));
    let prompt_ok = s.picker_prompt() == "Find file: ";
    for c in "main".chars() {
        s.key_event(key_char(c));
    }
    let filtered = s
        .picker_filtered()
        .iter()
        .any(|(c, _)| c.name.contains("src/main.rs"));
    s.key_event(key("RET"));
    let opened = s.top_view() == ViewId::Buffer
        && s
            .buffers
            .current_buffer()
            .map(|b| b.line_count() > 4)
            .unwrap_or(false);
    let frame = render80(s);
    assert!(
        prompt_ok && filtered && opened,
        "prompt={prompt_ok} filtered={filtered} opened={opened}\n{frame}"
    );
    let lines: Vec<&str> = frame.lines().collect();
    assert!(lines[0].contains("src/main.rs"), "title: {frame}");
    assert!(frame.contains("fn target_one"), "content: {frame}");
}

/// U-B2: ignored paths (`.git/`, `target/`) never appear in candidates.
/// The full candidate list must be on screen before the absence checks —
/// the count line IS the positive gate (and the df95113 layout pin: the
/// count line is on-screen at width 80).
#[test]
fn unit_flow_b2() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("C-f"));
    // Full list rendered: prompt + count line present in the 80-col frame.
    let (n, m) = s.picker_count();
    assert!(m > 0 && n == m, "full list: {n} of {m}");
    let frame = render80(s);
    assert!(
        flat(&frame).contains(&format!("{} of {m}", n)),
        "count line on-screen (full list gate): {frame}"
    );
    assert!(!frame.contains(".git/"), "git dir leaked: {frame}");
    assert!(!frame.contains("target/"), "target dir leaked: {frame}");
}

/// U-B3 (no-match leg): no match shows a clean empty state (count 0).
#[test]
fn unit_flow_b3() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("C-f"));
    for c in "zzzznomatch".chars() {
        s.key_event(key_char(c));
    }
    let (n, m) = s.picker_count();
    assert_eq!(n, 0, "no candidates");
    assert!(m > 0, "total still the walk size: {m}");
    let frame = render80(s);
    assert!(
        flat(&frame).contains(&format!("0 of {m}")),
        "clean empty state (count line present): {frame}"
    );
}

/// U-B4 (06a): `C-x C-b` buffer list — at boot the table is EMPTY (no
/// scratch auto-creation); opening a file first yields exactly that one
/// buffer row.
#[test]
fn unit_flow_b4() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "main");
    assert_eq!(s.buffers.len(), 1, "one buffer after open");
    s.key_event(key("C-x"));
    s.key_event(key("C-b"));
    assert_eq!(s.top_view(), ViewId::BufferList);
    assert_eq!(s.buffer_rows().len(), 1, "exactly one buffer row");
    {
        let mut s2 = store_in(repo.path());
        open_via_finder(&mut s2, "main");
        s2.key_event(key("C-x"));
        s2.key_event(key("C-b"));
        let frame = render80(s2);
        assert!(frame.contains("*list-buffers*"), "{frame}");
        assert!(!frame.contains("*scratch*"), "no scratch row: {frame}");
    }
    s.key_event(key("q"));
    assert_ne!(s.top_view(), ViewId::BufferList, "q closes the list");
}

/// U-B5 populate leg: recents populate as files are visited.
#[test]
fn unit_flow_b5() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "main");
    s.key_event(key("C-c"));
    s.key_event(key("p"));
    s.key_event(key("e"));
    assert_eq!(s.picker_prompt(), "Recent file: ");
    let main_listed = s
        .picker_filtered()
        .iter()
        .any(|(c, _)| c.name.contains("src/main.rs"));
    let frame = render80(s);
    assert!(frame.contains("Recent file:"), "{frame}");
    assert!(main_listed, "visited file listed in recents: {frame}");
}

/// U-B6 deleted-file edge: a recents entry for a file deleted on disk is
/// skipped; the picker stays usable.
#[test]
fn unit_flow_b6() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "main");
    open_via_finder(&mut s, "lib");
    let lib_path = repo.path().join("src/lib.rs");
    let backup = repo.path().join("src/lib.rs.bak");
    std::fs::rename(&lib_path, &backup).unwrap();
    let result;
    {
        s.key_event(key("C-c"));
        s.key_event(key("p"));
        s.key_event(key("e"));
        assert_eq!(s.picker_prompt(), "Recent file: ");
        let names: Vec<String> = s
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.name.clone())
            .collect();
        let main_listed = names.iter().any(|n| n.contains("src/main.rs"));
        let lib_absent = !names.iter().any(|n| n == "src/lib.rs");
        let frame = render80(s);
        assert!(frame.contains("Recent file:"), "{frame}");
        result = (main_listed, lib_absent);
    }
    std::fs::rename(&backup, &lib_path).unwrap();
    let (main_listed, lib_absent) = result;
    assert!(
        main_listed && lib_absent,
        "surviving-recent={main_listed} deleted-file-absent={lib_absent}"
    );
}

/// U-H1 pending prefix: `C-x` shows pending in the status; an unmatchable
/// key cancels.
#[test]
fn unit_flow_h1() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    let pending = s.pending_display() == "C-x";
    let frame = render80(s);
    let status_shown = frame.lines().last().unwrap().contains("[C-x]");
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("z")); // unmatchable tail -> cancels
    let cleared = s.pending.is_empty() && s.message.contains("unbound key");
    assert!(
        pending && status_shown && cleared,
        "pending-shown={pending} status-bracket={status_shown} cleared={cleared}\n{frame}"
    );
}

/// U-H2 idle: C-g on an idle frame is a clean no-op — the whole frame
/// (content, minibuffer, status) byte-identical afterwards.
#[test]
fn unit_flow_h2_idle() {
    let repo = fixture_repo();
    let before = render80(store_in(repo.path()));
    let mut s = store_in(repo.path());
    s.key_event(key("C-g"));
    let after = render80(s);
    assert_eq!(before, after, "idle C-g must be a byte-identical no-op");
}

/// U-H2 picker: C-g closes the picker and echoes cancel, back in the view.
#[test]
fn unit_flow_h2_picker() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("C-f"));
    assert!(s.picker_open(), "picker opened");
    s.key_event(key("C-g"));
    assert!(!s.picker_open(), "C-g closes the picker");
    assert_eq!(s.message, "cancel", "cancel echo");
    assert_eq!(s.top_view(), ViewId::Home, "back in the view");
    let frame = render80(s);
    assert!(!frame.contains("Find file:"), "picker gone: {frame}");
}

/// U-H2 pending: C-x arms a prefix; C-g clears it (with echo).
#[test]
fn unit_flow_h2_pending() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    let pending_shown = s.pending_display() == "C-x";
    s.key_event(key("C-g"));
    let cleared = s.pending.is_empty() && s.message == "cancel";
    assert!(pending_shown && cleared, "shown={pending_shown} cleared={cleared}");
}

/// U-H2 isearch: C-s arms isearch; C-g exits it (with echo), view kept.
#[test]
fn unit_flow_h2_isearch() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "main");
    s.key_event(key("C-s"));
    let isearch_on = s.isearch_active();
    // A second store drives to the same state for the frame assert
    // (render80 consumes the store).
    let mut f = store_in(repo.path());
    open_via_finder(&mut f, "main");
    f.key_event(key("C-s"));
    let frame = render80(f);
    assert!(frame.contains("I-search"), "isearch armed: {frame}");
    s.key_event(key("C-g"));
    let exited = !s.isearch_active();
    let view_kept = s.top_view() == ViewId::Buffer;
    assert!(
        isearch_on && exited && view_kept && s.message == "cancel",
        "on={isearch_on} exited={exited} view-kept={view_kept} msg={:?}\n{frame}",
        s.message
    );
}

/// U-H3 unknown key echoes in the minibuffer.
#[test]
fn unit_flow_h3() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("z"));
    assert_eq!(s.message, "unbound key: z");
    let frame = render80(s);
    assert!(frame.contains("unbound key: z"), "{frame}");
}

/// 06a: bare `q` on the home view is UNBOUND — an "unbound key" echo, the
/// app stays alive and the home frame is unchanged (content rows only;
/// only the minibuffer carries the echo).
#[test]
fn unit_flow_qquit() {
    let repo = fixture_repo();
    let before = render80(store_in(repo.path()));
    let mut s = store_in(repo.path());
    s.key_event(key("q"));
    let echo = s.message.contains("unbound key: q");
    let alive = !s.quit;
    let after = render80(s);
    // The content frame is unchanged; only the minibuffer echoes.
    let same_content = before.lines().zip(after.lines()).take_while(|(b, a)| b == a).count()
        >= after.lines().count().saturating_sub(2);
    assert!(
        echo && alive && same_content,
        "echo={echo} alive={alive} frame:\n{after}"
    );
}

/// Finding (plan 002): opening notes must NOT flag a false "changed on
/// disk" — the app's own creation event is consumed by design (the store's
/// created-paths guard). The PTY leg keeps the wall-clock hold; this twin
/// pins the guard itself.
#[test]
fn unit_flow_notes_no_false_marker() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("n"));
    let notes_open = current_is_notes(&s);
    let notes_path = repo.path().join(".redline-notes.md");
    // The watcher's creation event for the file WE created: must be
    // consumed, no marker.
    publish_change(&mut s, &notes_path, ChangeKind::Create);
    let no_marker = !s.current_buffer_changed_on_disk();
    assert!(
        notes_open && no_marker,
        "notes-open={notes_open} creation event must not raise a false marker (msg={:?})",
        s.message
    );
}

/// Finding (plan 002): the palette must not show demo/placeholder
/// commands.
#[test]
fn unit_flow_palette_no_demo() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("M-x"));
    assert!(s.picker_open(), "palette open");
    let names: Vec<String> = s
        .picker_filtered()
        .iter()
        .map(|(c, _)| c.name.clone())
        .collect();
    let real = names.iter().any(|n| n == "quit");
    let demo_leak = names.iter().any(|n| {
        n == "demo-message-1" || n == "demo-message-2" || n == "insert-demo-text"
    });
    let frame = render80(s);
    assert!(
        real && !demo_leak,
        "real={real} demo-leak={demo_leak} ({} candidates)\n{frame}",
        names.len()
    );
}

/// Finding (plan 002): graft cache cards (agent output, not project
/// source) must not surface as file-finder candidates; the walk prunes
/// the whole subtree, and a `graft` query yields a clean empty state.
#[test]
fn unit_flow_graft() {
    let repo = fixture_repo();
    let graft = repo.path().join("graft");
    std::fs::create_dir_all(&graft).unwrap();
    std::fs::write(graft.join("card.md"), "# agent cache card\n").unwrap();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("C-f"));
    let full = s.picker_filtered();
    let no_graft_full = !full.iter().any(|(c, _)| c.name.contains("graft"));
    for c in "graft".chars() {
        s.key_event(key_char(c));
    }
    let (n, m) = s.picker_count();
    assert_eq!(n, 0, "graft query must yield a clean empty state");
    assert!(m > 0, "total nonzero: {m}");
    let frame = render80(s);
    assert!(
        no_graft_full && flat(&frame).contains(&format!("0 of {m}")),
        "graft-in-full-list={no_graft_full}\n{frame}"
    );
}

/// Finding (plan 002): editable-buffer key interception must let multi-key
/// sequences through while editing notes: a plain printable self-inserts;
/// the C-x g tail DISPATCHES (opens magit), it is not typed into the
/// buffer. (The PTY leg keeps the real-terminal input-encoding smoke.)
#[test]
fn unit_flow_editable_keys() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("n"));
    let notes_open = current_is_notes(&s);
    assert!(notes_open, "notes open");
    for c in "abc".chars() {
        s.key_event(key_char(c));
    }
    let text = s.buffer_text();
    assert!(text.contains("abc"), "self-insert landed: {text:?}");
    s.key_event(key("C-x"));
    assert_eq!(s.pending_display(), "C-x", "prefix arms while editing");
    s.key_event(key("g"));
    let magit_opened = s.top_view() == ViewId::MagitStatus;
    assert!(magit_opened, "C-x g must dispatch to magit, not type into notes");
    s.key_event(key("q"));
    let back_to_notes = current_is_notes(&s);
    let intact = s.buffer_text().contains("abc") && !s.buffer_text().contains("abcg");
    assert!(
        back_to_notes && intact,
        "back={back_to_notes} intact={intact} text={:?}",
        s.buffer_text()
    );
}

/// U-J3 80x24: the status line renders on exactly one row (the bottom
/// row), with no spill onto the minibuffer row, across two views — at the
/// width-bounded static render (the layout-correctness class the PTY leg
/// still smoke-tests live).
#[test]
fn unit_flow_j3() {
    let repo = fixture_repo();
    let name = repo.path().file_name().unwrap().to_string_lossy().to_string();
    // File view (loop-03 review P2-5: the BUFFER status line carries mode
    // + which-function + position — the closest-to-overflow case, strictly
    // stronger than the sparser home header).
    {
        let mut s = store_in(repo.path());
        open_via_finder(&mut s, "leg");
        let frame = render80(s);
        let rows = rows_containing(&frame, &name);
        let last = frame.lines().count() - 1;
        assert!(rows.contains(&last), "buffer status row carries the project: {frame:?}");
        assert!(
            !rows.iter().any(|&r| r == last - 1),
            "no spill onto the minibuffer row: {frame:?}"
        );
    }
    // Magit status view.
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("g"));
    assert_eq!(s.top_view(), ViewId::MagitStatus);
    let frame = render80(s);
    let rows = rows_containing(&frame, &name);
    let last = frame.lines().count() - 1;
    assert!(rows.contains(&last), "magit status row: {frame}");
    assert!(
        !rows.iter().any(|&r| r == last - 1),
        "magit no spill onto the minibuffer row: {frame}"
    );
}

/// plan-004-issue-05h: in the `C-x C-b` list, `n`/`p` move the selection
/// (no unbound echo), `d` kills the selected buffer (count drops by one,
/// the list STAYS OPEN, selection clamped), and `q` still closes.
#[test]
fn unit_flow_buffer_list_np() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "main");
    open_via_finder(&mut s, "lib");
    s.key_event(key("C-x"));
    s.key_event(key("C-b"));
    let list_open = s.top_view() == ViewId::BufferList;
    let count_before = s.buffer_rows().len();
    assert_eq!(count_before, 2, "two real buffers (06a: no scratch)");

    let sel0 = s.buffer_list_selected();
    s.key_event(key("n"));
    let n_moved = s.buffer_list_selected() != sel0 && !s.message.contains("unbound key");
    s.key_event(key("p"));
    let p_back = s.buffer_list_selected() == sel0 && !s.message.contains("unbound key");

    s.key_event(key("n"));
    s.key_event(key("d"));
    let list_still_open = s.top_view() == ViewId::BufferList;
    let d_dropped = s.buffer_rows().len() == count_before - 1;
    let clamped = s.buffer_list_selected() < s.buffer_rows().len();
    let no_echo = !s.message.contains("unbound key");

    s.key_event(key("q"));
    let q_closed = s.top_view() != ViewId::BufferList;
    assert!(
        list_open && n_moved && p_back && list_still_open && d_dropped && clamped
            && q_closed,
        "open={list_open} n-moved={n_moved} p-back={p_back} stays-open={list_still_open} \
         dropped={d_dropped} clamped={clamped} no-echo={no_echo} q-closed={q_closed}"
    );
}

// ═══════════════════════ batch 2: motion / search / magit / watcher ═══

/// A 60-line `tall_sweep.rs` fixture (the U-C1 motion fixture).
fn tall_sweep_repo(lines: usize, name: &str) -> tempfile::TempDir {
    let dir = fixture_repo();
    let mut content = String::new();
    for i in 1..=lines {
        content.push_str(&format!("line {i}\n"));
    }
    std::fs::write(dir.path().join(format!("src/{name}")), content).unwrap();
    git(dir.path(), &[
        "add",
        &format!("src/{name}"),
    ]);
    dir
}

/// U-C1 Motion: a page key scrolls (the window moves, point's screen row
/// held). 015-03 re-pin (was `C-d`): `C-d` was freed from half-page scroll
/// for delete-char-forward in accurate mode, so the motion flow drives the
/// still-bound `C-v` (scroll-page-down) instead.
#[test]
fn unit_flow_c1() {
    let repo = tall_sweep_repo(60, "tall_sweep.rs");
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "tall_sweep");
    assert_eq!(s.top_view(), ViewId::Buffer, "tall file opened");
    let (top_before, _, viewport) = s.file_view_scroll_info();
    let point_before = s.file_view_point().0;
    assert_eq!(top_before, 0);
    s.key_event(key("C-v"));
    let (top_after, _, _) = s.file_view_scroll_info();
    let point_after = s.file_view_point().0;
    // (a) Real scroll: the top visible line changed (non-vacuous).
    assert_ne!(top_before, top_after, "C-v must scroll the window");
    assert!(top_after <= viewport, "page scroll: {top_after}");
    // (b) Point's screen row preserved: same relative row in the window.
    assert_eq!(
        point_before.saturating_sub(top_before),
        point_after.saturating_sub(top_after),
        "point's screen row must be held (top {top_before}->{top_after}, point \
         {point_before}->{point_after})"
    );
}

/// U-C6 M-< / M-> / G: top/bottom scroll and G→M-< round trip (bottom
/// anchor: the last content row reads the final line, not just "the
/// first row changed").
#[test]
fn unit_flow_c6() {
    let repo = tall_sweep_repo(50, "long_sweep.rs");
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "long_sweep");
    assert!(s
        .file_view_rows()
        .first()
        .map(|r| r.text == "line 1")
        .unwrap_or(false),
        "top row line 1");
    // M-END: bottom anchor — the last content row reads the final line.
    // (jump-ambiguity: point-buffer-end moved from M-> to M-END; M-> is
    // now the Xref force-list hotkey, which here would open the picker.)
    s.key_event(key("M-END"));
    let m_gt_bottom_anchor = s
        .file_view_rows()
        .iter()
        .rev()
        .find(|r| !r.text.is_empty())
        .map(|r| r.text.as_str())
        == Some("line 50");
    assert!(m_gt_bottom_anchor, "M-> bottom anchor (last content row line 50)");
    // M-<: back to top.
    s.key_event(key("M-<"));
    let rows = s.file_view_rows();
    let back_top = rows.iter().find(|r| !r.text.is_empty()).map(|r| r.text.as_str())
        == Some("line 1");
    assert!(back_top, "M-< round trip to top");
    // G: bottom again, same anchor.
    s.key_event(key("G"));
    let g_anchor = s
        .file_view_rows()
        .iter()
        .rev()
        .find(|r| !r.text.is_empty())
        .map(|r| r.text.as_str())
        == Some("line 50");
    assert!(g_anchor, "G bottom anchor (line 50)");
    // M-< round trip after G.
    s.key_event(key("M-<"));
    let g_roundtrip = s
        .file_view_rows()
        .iter()
        .find(|r| !r.text.is_empty())
        .map(|r| r.text.as_str())
        == Some("line 1");
    assert!(g_roundtrip, "G→M-< round trip to top");
}

/// U-E1: `C-c p s s` — the prompt, the query, and the results with the
/// grouped count (the PTY's `wait_done` is the bus drain here).
#[test]
fn unit_flow_e1() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    let mut rx = s.search_rx().unwrap();
    for tok in ["C-c", "p", "s", "s"] {
        s.key_event(key(tok));
    }
    assert_eq!(s.message, "Search: ", "prompt active");
    for c in "target".chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("RET"));
    drain_search_finished(&mut s, &mut rx);
    assert_eq!(s.top_view(), ViewId::Search, "results view");
    assert!(!s.search_running(), "search finished");
    let (rows, _, total, _) = s.search_view_info();
    assert!(total > 0 && !rows.is_empty(), "hits present");
    let frame = render80(s);
    assert!(frame.contains("matches in"), "grouped count label: {frame}");
    assert!(frame.contains("fn target_one"), "hit content: {frame}");
}

/// U-E3 results navigation: n moves the selection; RET jumps to the file.
#[test]
fn unit_flow_e3() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    let mut rx = s.search_rx().unwrap();
    for tok in ["C-c", "p", "s", "s"] {
        s.key_event(key(tok));
    }
    for c in "target".chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("RET"));
    drain_search_finished(&mut s, &mut rx);
    let sel_before = s.search_view_info().3;
    s.key_event(key("n"));
    let sel_after = s.search_view_info().3;
    assert!(
        sel_before != sel_after,
        "n must move the selection ({sel_before:?}->{sel_after:?})"
    );
    s.key_event(key("RET"));
    assert_eq!(s.top_view(), ViewId::Buffer, "RET jumps to the hit's file");
    let jumped = s
        .buffers
        .current_buffer()
        .and_then(|b| b.path.as_ref())
        .map(|p| p.ends_with("main.rs"))
        .unwrap_or(false);
    assert!(jumped, "landed in src/main.rs");
    let frame = render80(s);
    let lines: Vec<&str> = frame.lines().collect();
    assert!(lines[0].contains("src/main.rs"), "title: {frame}");
}

/// U-E2 cancel paths: ESC and q close the results view.
#[test]
fn unit_flow_e2() {
    let drive = |repo: &std::path::Path| -> AppStore {
        let mut s = store_in(repo);
        let mut rx = s.search_rx().unwrap();
        for tok in ["C-c", "p", "s", "s"] {
            s.key_event(key(tok));
        }
        for c in "target".chars() {
            s.key_event(key_char(c));
        }
        s.key_event(key("RET"));
        drain_search_finished(&mut s, &mut rx);
        s
    };
    let repo = fixture_repo();
    // ESC leg.
    let mut s = drive(repo.path());
    assert_eq!(s.top_view(), ViewId::Search, "search was open");
    s.key_event(key("ESC"));
    assert!(
        s.top_view() != ViewId::Search && !s.quit,
        "ESC must close the results view, app alive"
    );
    // q leg.
    let mut s = drive(repo.path());
    assert_eq!(s.top_view(), ViewId::Search, "search was open");
    s.key_event(key("q"));
    assert!(
        s.top_view() != ViewId::Search && !s.quit,
        "q must close the results view, app alive"
    );
}

/// U-F1: `C-x g` status — Staged/Unstaged sections + dirty counts in the
/// status line.
#[test]
fn unit_flow_f1() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("g"));
    assert_eq!(s.top_view(), ViewId::MagitStatus);
    let d = s.dirty_counts();
    assert_eq!(
        d.map(|x| (x.staged, x.unstaged)),
        Some((1, 1)),
        "baseline +1 ~1: {d:?}"
    );
    let frame = render80(s);
    assert!(frame.contains("Staged") && frame.contains("Unstaged"), "{frame}");
    let status = frame.lines().last().unwrap();
    assert!(status.contains("+1 ~1"), "dirty counts in status line: {status:?}");
}

/// U-F2 stage/unstage: a keypress toggles the git index (git diff
/// --cached agrees). The fixture is restored via git afterwards.
#[test]
fn unit_flow_f2() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    let cached_names = || {
        let out = git_out(repo.path(), &["diff", "--cached", "--name-only"]);
        out.lines().map(|l| l.to_string()).collect::<Vec<_>>()
    };
    let before = cached_names();
    assert!(before.iter().any(|n| n == "src/lib.rs"), "baseline: lib staged");
    s.key_event(key("C-x"));
    s.key_event(key("g"));
    s.key_event(key("u")); // unstage the row under the cursor (staged lib.rs)
    let after_u = cached_names();
    assert_ne!(before, after_u, "`u` must toggle the git index");
    assert!(
        !after_u.iter().any(|n| n == "src/lib.rs"),
        "lib unstaged: {after_u:?}"
    );
    // Restore the fixture baseline (test hygiene, as the PTY leg does).
    git(repo.path(), &["add", "src/lib.rs"]);
    let restored = cached_names() == before;
    assert!(restored, "fixture restored");
}

/// U-F3: magit fold/unfold + RET visit — file sections start folded
/// (hunk rows hidden); TAB reveals, TAB hides, and RET on the file row
/// opens the file.
#[test]
fn unit_flow_f3() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("g"));
    // Positive gate: magit status rendered (Staged section present).
    assert!(s.magit_rows().iter().any(|r| r.text.contains("Staged")));
    let has_hunk = |s: &AppStore| {
        s.magit_rows()
            .iter()
            .any(|r| r.text.contains("@@"))
            && s.magit_rows().iter().any(|r| r.text.contains("staged_change_marker"))
    };
    let folded = !has_hunk(&s);
    s.key_event(key("TAB"));
    let unfolded = has_hunk(&s);
    s.key_event(key("TAB"));
    let refolded = !has_hunk(&s);
    s.key_event(key("TAB"));
    s.key_event(key("RET"));
    let visited = s.top_view() == ViewId::Buffer
        && s
            .buffers
            .current_buffer()
            .and_then(|b| b.path.as_ref())
            .map(|p| p.ends_with("lib.rs"))
            .unwrap_or(false);
    assert!(
        folded && unfolded && refolded && visited,
        "folded={folded} unfolded={unfolded} refolded={refolded} visited={visited}"
    );
    let frame = render80(s);
    assert!(frame.contains("staged_change_marker"), "file content: {frame}");
}

/// A dedicated throwaway commit repo (the sweep's REPO5): one base commit,
/// a STAGED change on src/lib.rs, an UNSTAGED change on README.md.
fn commit_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(p.join("src/lib.rs"), "pub fn target_lib() {}\npub fn other_lib() {}\n").unwrap();
    std::fs::write(p.join("README.md"), "# commit fixture\n").unwrap();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Test"]);
    git(p, &["config", "user.email", "test@example.com"]);
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "base"]);
    append(p.join("src/lib.rs").as_path(), "staged_change_marker\n");
    git(p, &["add", "src/lib.rs"]);
    append(p.join("README.md").as_path(), "unstaged_change_marker\n");
    dir
}

/// U-F5 commit flow: stage the unstaged file, open the commit editor
/// (`c`), type a message, commit (`C-c C-c`) — verified by `git log` and
/// the status buffer going clean.
#[test]
fn unit_flow_f5() {
    let repo = commit_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("g"));
    // Cursor on the staged file; n -> Unstaged group; n -> README row; s.
    s.key_event(key("n"));
    s.key_event(key("n"));
    s.key_event(key("s"));
    let staged_both = {
        let out = git_out(repo.path(), &["diff", "--cached", "--name-only"]);
        let names: Vec<String> = out.lines().map(|l| l.to_string()).collect();
        names.contains(&"src/lib.rs".to_string()) && names.contains(&"README.md".to_string())
    };
    assert!(staged_both, "both files staged");
    s.key_event(key("c"));
    assert_eq!(s.top_view(), ViewId::CommitEditor, "commit editor open");
    let editor_render = s
        .commit_editor_rows()
        .iter()
        .any(|r| r.text.contains("Staged changes"))
        && s.commit_editor_title().contains("commit");
    assert!(editor_render, "editor chrome (title + staged section)");
    for c in "sweepf5marker".chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("C-c"));
    s.key_event(key("C-c"));
    let log_top = git_out(repo.path(), &["log", "--oneline", "-1"]).trim().to_string();
    assert!(log_top.contains("sweepf5marker"), "git log top: {log_top:?}");
    let status_clean = git_out(repo.path(), &["status", "--porcelain"]).trim().is_empty();
    assert!(status_clean, "working tree clean after commit");
    let frame = render80(s);
    assert!(
        !frame.contains("M src/lib.rs") && !frame.contains("M README.md"),
        "magit clean after commit: {frame}"
    );
}

/// U-F6 log: `l` lists commits, a key moves the selection (exactly one
/// cursor row), and RET opens the selected commit's diff.
#[test]
fn unit_flow_f6() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("g"));
    s.key_event(key("l"));
    assert_eq!(s.top_view(), ViewId::Log, "log view");
    let rows_render = {
        let rows = s.log_rows();
        let t: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
        t.iter().any(|l| l.contains("commit 5"))
            && t.iter().any(|l| l.contains("commit 4"))
            && t.iter().any(|l| l.contains("init"))
    };
    assert!(rows_render, "log rows render commit 5 / commit 4 / init");
    let sel_before = s.log.as_ref().map(|l| l.selected).unwrap_or(0);
    s.key_event(key("DOWN"));
    let sel_after = s.log.as_ref().map(|l| l.selected).unwrap_or(0);
    assert!(
        sel_before != sel_after,
        "down must move the (single) selection ({sel_before}->{sel_after})"
    );
    s.key_event(key("RET"));
    assert_eq!(s.top_view(), ViewId::CommitDiff, "RET opens the commit diff");
    let frame = render80(s);
    let lines: Vec<&str> = frame.lines().collect();
    assert!(
        lines[0].contains("commit")
            && frame.contains("read-only")
            && lines.last().unwrap().contains("commit-diff"),
        "diff pane chrome: {frame}"
    );
}

/// U-F7 blame: `b` blames the current buffer's file — per-line rows with
/// a commit-hash/author prefix and exactly one selected (cursor) row.
#[test]
fn unit_flow_f7() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "main");
    // The current-file check reads the status line's which-function
    // (a symbol from src/main.rs), not the view title. The index job runs
    // off-thread in the live app; build it synchronously here.
    install_index(&mut s, repo.path());
    let file_current = s.which_function().contains("target_one");
    assert!(
        file_current,
        "which-function names the current file's symbol: {:?}",
        s.which_function()
    );
    s.key_event(key("C-x"));
    s.key_event(key("g"));
    s.key_event(key("b"));
    assert_eq!(s.top_view(), ViewId::Blame, "blame view");
    assert!(
        s.blame_title().contains("blame: src/main.rs"),
        "title: {}",
        s.blame_title()
    );
    let rows = s.blame_rows();
    let per_line = rows.iter().filter(|r| r.text.contains("Test")).count() >= 5;
    assert!(per_line, "per-line author token (>=5): {} rows", rows.len());
    let cursor = s.blame.as_ref().map(|b| b.selected).unwrap_or(0);
    assert!(cursor < rows.len(), "exactly one selected row: {cursor}");
    let frame = render80(s);
    let lines: Vec<&str> = frame.lines().collect();
    assert!(lines[0].contains("blame: src/main.rs"), "title row: {frame}");
}

/// U-F8 branch/stash: `y` opens the branch picker (lists the local
/// branch); `z` with no stashes shows the empty state, not a panic.
#[test]
fn unit_flow_f8() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("g"));
    s.key_event(key("y"));
    assert!(s.picker_open(), "branch picker open");
    let branch_listed = s
        .picker_filtered()
        .iter()
        .any(|(c, _)| c.display.contains("main"));
    assert!(branch_listed, "branch picker lists the local branch");
    s.key_event(key("C-g"));
    assert!(!s.picker_open(), "C-g closes the branch picker");
    s.key_event(key("z"));
    let empty_state = s.message.contains("no stashes");
    assert!(empty_state, "stash empty state (msg={:?})", s.message);
}

/// U-CDS commit-diff scroll (thin): the commit-diff pane scrolls past the
/// 80x24 viewport — M-> lands on the last page (the sentinel row shows),
/// M-< round-trips to the top (the sentinel hides).
#[test]
fn unit_flow_cds() {
    let repo = win_diff_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "big");
    s.key_event(key("C-x"));
    s.key_event(key("g"));
    s.key_event(key("l"));
    s.key_event(key("RET")); // newest (tall) commit's diff
    assert_eq!(s.top_view(), ViewId::CommitDiff, "commit diff open");
    let top_hidden = !s
        .commit_diff_view_info()
        .0
        .iter()
        .any(|r| r.text.contains("BOTTOM_SENTINEL"));
    assert!(top_hidden, "sentinel hidden at top");
    s.key_event(key("M->"));
    let (_, _, total) = s.commit_diff_view_info();
    assert!(total > s.viewport_lines, "diff is taller than the viewport: {total}");
    let bottom_shown = s
        .commit_diff_view_info()
        .0
        .iter()
        .any(|r| r.text.contains("BOTTOM_SENTINEL"));
    assert!(bottom_shown, "M-> shows the sentinel row");
    s.key_event(key("M-<"));
    let roundtrip = !s
        .commit_diff_view_info()
        .0
        .iter()
        .any(|r| r.text.contains("BOTTOM_SENTINEL"));
    assert!(roundtrip, "M-< hides the sentinel row again");
    let frame = render80(s);
    assert!(
        frame.contains("read-only") && frame.contains("commit diff"),
        "diff pane still open after the round trip: {frame}"
    );
}

/// U-BLW blame windowing (thin): the cursor-following blame window keeps
/// the (single) cursor row in view across C-n moves and M-> to the last
/// line.
#[test]
fn unit_flow_blw() {
    let repo = win_diff_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "big");
    s.key_event(key("C-x"));
    s.key_event(key("g"));
    s.key_event(key("b"));
    assert_eq!(s.top_view(), ViewId::Blame, "blame open");
    let in_window = |s: &AppStore| -> bool {
        let (rows, top, total) = s.blame_view_info();
        let sel = s.blame.as_ref().map(|b| b.selected).unwrap_or(0);
        !rows.is_empty() && top <= sel && sel < top + rows.len() && sel < total
    };
    let mut all_in = true;
    for _ in 0..20 {
        s.key_event(key("C-n"));
        all_in &= in_window(&s);
    }
    s.key_event(key("M->"));
    let last_ok = in_window(&s)
        && s.blame.as_ref().map(|b| b.selected == b.lines.len() - 1).unwrap_or(false);
    assert!(
        all_in && last_ok,
        "cursor-in-window across C-n x20 = {all_in}, M-> last line = {last_ok}"
    );
}

/// A throwaway repo whose notes file is pre-seeded with 30 lines (the
/// sweep's WIN_NOTES_REPO) — taller than the 21-row viewport.
fn notes_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::write(p.join("README.md"), "# notes fixture\n").unwrap();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Test"]);
    git(p, &["config", "user.email", "test@example.com"]);
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "init"]);
    let mut notes = String::new();
    for i in 1..=29 {
        notes.push_str(&format!("note line {i}\n"));
    }
    notes.push_str("note line 30"); // last line, no trailing newline
    std::fs::write(p.join(".redline-notes.md"), notes).unwrap();
    dir
}

/// U-NSL notes-scroll (thin): a notes buffer taller than the viewport
/// keeps the active (insertion) row in view — typing near the bottom
/// scrolls the last line into view.
#[test]
fn unit_flow_nsl() {
    let repo = notes_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("n"));
    assert_eq!(s.top_view(), ViewId::Buffer, "notes open");
    let before_last = s.file_view_rows().last().map(|r| r.text.clone());
    let bottom_hidden = before_last.as_deref() != Some("note line 30");
    assert!(bottom_hidden, "bottom line hidden before typing: {before_last:?}");
    s.key_event(key("Z"));
    let rows_after = s.file_view_rows();
    let after_last = rows_after.last().map(|r| r.text.as_str());
    let edited = after_last == Some("note line 30Z");
    assert!(
        edited,
        "self-insert must scroll the insertion row into view: {after_last:?}"
    );
    let frame = render80(s);
    assert!(frame.contains("note line 30Z"), "{frame}");
}

/// U-G1 live-edit scroll preservation: an external append at the bottom of
/// a viewed file repaints the view and preserves the scroll anchor.
#[test]
fn unit_flow_g1() {
    let repo = fixture_repo();
    let gw = repo.path().join("src/g_watch.rs");
    std::fs::write(&gw, "gwatch line one\ngwatch line two\n").unwrap();
    git(repo.path(), &["add", "src/g_watch.rs"]);
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "g_watch");
    let top_before = s.file_view_scroll_info().0;
    append(&gw, "G1_BOTTOM_APPEND\n");
    publish_change(&mut s, &gw, ChangeKind::Modify);
    assert!(!s.current_buffer_changed_on_disk(), "no marker on plain buffer");
    let top_after = s.file_view_scroll_info().0;
    assert_eq!(top_before, top_after, "scroll anchor preserved");
    let frame = render80(s);
    assert!(frame.contains("G1_BOTTOM_APPEND"), "view repainted: {frame}");
}

/// U-G2 agent churn (state half): rapid disk writes coalesce — the view
/// shows the FINAL content, and the app stays responsive (a keypress
/// repaints). The bounded-repaint property is the debounce coalescing
/// (unit-tested in the watcher module).
#[test]
fn unit_flow_g2() {
    let repo = fixture_repo();
    let gw = repo.path().join("src/g_watch.rs");
    std::fs::write(&gw, "gwatch line one\ngwatch line two\n").unwrap();
    git(repo.path(), &["add", "src/g_watch.rs"]);
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "g_watch");
    let pre_point_line = s.point_line();
    for i in 0..10 {
        append(&gw, &format!("G2_CHURN_{i}\n"));
    }
    publish_change(&mut s, &gw, ChangeKind::Modify);
    assert!(
        s.buffer_text().contains("G2_CHURN_9"),
        "final content shown after the burst"
    );
    // Responsive = the store is still coherent after the burst: the next
    // keypress processes (the point moves — the original U-G2 "a keypress
    // repaints" half, asserted as an effect, not just a render) and the
    // view still renders the final content.
    s.key_event(key("C-n"));
    assert_ne!(
        s.point_line(),
        pre_point_line,
        "a keypress after the watcher burst still moves the point"
    );
    let frame = render80(s);
    assert!(frame.contains("G2_CHURN_9"), "post-burst frame coherent: {frame}");
}

#[test]
fn unit_flow_f4() {
    let repo = fixture_repo();
    let gw = repo.path().join("src/g_watch.rs");
    std::fs::write(&gw, "gwatch line one\ngwatch line two\n").unwrap();
    git(repo.path(), &["add", "src/g_watch.rs"]);
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "g_watch");
    append(&gw, "F4_WATCHER_APPEND\n");
    publish_change(&mut s, &gw, ChangeKind::Modify);
    let reloaded = s.buffer_text().contains("F4_WATCHER_APPEND");
    let no_marker = !s.current_buffer_changed_on_disk();
    assert!(reloaded && no_marker, "auto-reload, no marker");
    s.key_event(key("g"));
    let g_reload = s.message.contains("reloaded");
    assert!(g_reload, "g force-reload echo (msg={:?})", s.message);
}

/// U-G3 conflict path: a locally-owned buffer shows the 'changed on disk'
/// marker ONLY after a real external disk edit; the creation event is
/// consumed (no false marker); force-reload supersedes the marker.
#[test]
fn unit_flow_g3() {
    let repo = fixture_repo();
    let notes_path = repo.path().join(".redline-notes.md");
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("n"));
    // One keystroke: the notes buffer becomes locally-owned.
    s.key_event(key("x"));
    assert!(s.current_buffer_editable(), "notes are editable");
    // The creation event is consumed: no false marker.
    publish_change(&mut s, &notes_path, ChangeKind::Create);
    let no_false_marker = !s.current_buffer_changed_on_disk();
    let typed_char_present = s.buffer_text().contains('x');
    // A real external edit: the marker lands.
    append(&notes_path, "\nexternal_change_marker\n");
    publish_change(&mut s, &notes_path, ChangeKind::Modify);
    let marker_landed = s.current_buffer_changed_on_disk();
    // Force-reload via the M-x command path (the reload-buffer command).
    s.key_event(key("M-x"));
    for c in "reload-buffer".chars() {
        s.key_event(key_char(c));
    }
    let (n, _) = s.picker_count();
    assert_eq!(n, 1, "the command filter must be unique");
    s.key_event(key("RET"));
    let marker_cleared = !s.current_buffer_changed_on_disk();
    let reloaded = s.buffer_text().contains("external_change_marker");
    let local_edit_superseded = !s
        .buffer_text()
        .lines()
        .any(|l| l.trim() == "x");
    assert!(
        no_false_marker
            && typed_char_present
            && marker_landed
            && marker_cleared
            && reloaded
            && local_edit_superseded,
        "false={no_false_marker} typed={typed_char_present} marker={marker_landed} \
         cleared={marker_cleared} reloaded={reloaded} superseded={local_edit_superseded}"
    );
}

/// U-G6 suspend (state half): `M-x toggle-watcher` OFF suspends (message +
/// state) and ON again resumes. The no-reload-while-suspended leg is the
/// watcher SOURCE's gate — it stays in the thin PTY tier (the store's
/// apply path is deliberately bypass-free, mirroring the live contract).
#[test]
fn unit_flow_g6() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    let toggle = |s: &mut AppStore| {
        s.key_event(key("M-x"));
        for c in "toggle-watcher".chars() {
            s.key_event(key_char(c));
        }
        s.key_event(key("RET"));
    };
    toggle(&mut s);
    let suspended_msg = s.message.contains("file watching suspended");
    let suspended = s.watcher_suspended();
    toggle(&mut s);
    let resumed_msg = s.message.contains("file watching resumed");
    let resumed = !s.watcher_suspended();
    assert!(
        suspended_msg && suspended && resumed_msg && resumed,
        "suspend-msg={suspended_msg} suspend-state={suspended} resume-msg={resumed_msg} \
         resume-state={resumed}"
    );
}

/// U-G5 project switch (`C-c p p`): register a SECOND project in an
/// isolated cache, drive the switch, and verify the landing on the new
/// project (status line + find-file picker).
#[test]
fn unit_flow_g5() {
    let repo1 = fixture_repo();
    let repo2 = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(repo2.path().join("src")).unwrap();
    std::fs::write(repo2.path().join("src/b.py"), "def hello():\n    pass\n").unwrap();
    git(repo2.path(), &["init", "-q"]);
    // A shared, isolated persistence base: both projects registered.
    let base = tempfile::tempdir().unwrap();
    {
        let mut reg = AppStore::at(repo1.path(), base.path().to_path_buf());
        reg.project_store.registry.upsert(repo2.path());
        reg.project_store.save_registry().unwrap();
    }
    let mut s = AppStore::at(repo1.path(), base.path().to_path_buf());
    s.set_viewport_lines(21);
    let name2 = repo2.path().file_name().unwrap().to_string_lossy().to_string();
    s.key_event(key("C-c"));
    s.key_event(key("p"));
    s.key_event(key("p"));
    let picker_ok = s.picker_prompt() == "Switch project: "
        && s
            .picker_filtered()
            .iter()
            .any(|(c, _)| c.name.contains(&name2));
    assert!(picker_ok, "switch picker lists the 2nd project");
    s.key_event(key("RET"));
    let switched = s.project_display() == name2;
    let msg = s.message.contains(&format!("project: {name2}"));
    let landed = s.picker_prompt() == "Find file: "
        && s
            .picker_filtered()
            .iter()
            .any(|(c, _)| c.name.contains("src/b.py"));
    assert!(
        switched && msg && landed,
        "switched={switched} msg={msg} landed-in-new-project-find-file={landed} (msg={:?})",
        s.message
    );
}

// ═══════════════════ batch 3: quit prompts / edit mode / annotations / marks ═══════════════

/// A store with the notes buffer open in `repo` (the UI-reachable modified
/// buffer).
fn notes_store(repo: &std::path::Path) -> AppStore {
    let mut s = store_in(repo);
    s.key_event(key("C-x"));
    s.key_event(key("n"));
    s
}

/// quit-prompt-none: unmodified notes + `C-x C-c` → immediate quit, no
/// prompt rendered.
#[test]
fn unit_flow_quit_prompt_none() {
    let repo = fixture_repo();
    let mut s = notes_store(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("C-c"));
    assert!(s.quit, "unmodified buffers must quit immediately");
    assert!(!s.quit_prompt_active(), "no save prompt with zero modified buffers");
    assert!(
        !s.message.contains("(y, n, !, C-g)"),
        "msg={:?}",
        s.message
    );
}

/// quit-prompt-y: modified notes + `C-x C-c` → the prompt names the path
/// (on screen at width 80, including across a wrap); `y` writes the file
/// and the quit proceeds.
#[test]
fn unit_flow_quit_prompt_y() {
    let repo = fixture_repo();
    let mut s = notes_store(repo.path());
    s.key_event(key("Q"));
    s.key_event(key("Y"));
    s.key_event(key("C-x"));
    s.key_event(key("C-c"));
    assert!(s.quit_prompt_active(), "the prompt must hold the quit");
    let notes_path = repo.path().join(".redline-notes.md");
    assert_eq!(
        s.quit_prompt_buffer().unwrap(),
        notes_path.display().to_string(),
        "prompt names the modified buffer's path"
    );
    // The prompt is on screen at width 80 (the PTY's flat-text check: a
    // wrap must not defeat it). The keys lead the message, so they are the
    // left edge and survive the row clip (issue-quit-prompt-keys-invisible).
    let mut s2 = notes_store(repo.path());
    s2.key_event(key("Q"));
    s2.key_event(key("Y"));
    s2.key_event(key("C-x"));
    s2.key_event(key("C-c"));
    let frame = render80(s2);
    let f = flat(&frame);
    assert!(
        f.contains("(y, n, !, C-g)") && f.contains(".redline-notes.md"),
        "prompt on screen at width 80:\n{frame}"
    );
    // Back on the first store: `y` saves and quits.
    s.key_event(key("y"));
    assert!(s.quit, "y on the last modified buffer must quit");
    let on_disk = std::fs::read_to_string(&notes_path).unwrap();
    assert!(on_disk.contains("QY"), "y must write the edit to disk");
}

/// quit-prompt-n: `n` exits without writing — the edit is knowingly
/// discarded (the file is byte-identical to the seed).
#[test]
fn unit_flow_quit_prompt_n() {
    let repo = fixture_repo();
    let mut s = notes_store(repo.path());
    let notes_path = repo.path().join(".redline-notes.md");
    let seed = std::fs::read_to_string(&notes_path).unwrap();
    s.key_event(key("X"));
    s.key_event(key("C-x"));
    s.key_event(key("C-c"));
    assert!(s.quit_prompt_active());
    s.key_event(key("n"));
    assert!(s.quit, "n on the last modified buffer must quit");
    let on_disk = std::fs::read_to_string(&notes_path).unwrap();
    assert_eq!(on_disk, seed, "n must NOT write the edit to disk");
}

/// quit-prompt-cg: `C-g` cancels the whole quit — prompt gone, buffer
/// content intact, the store is still alive; a re-quit re-enters the
/// prompt.
#[test]
fn unit_flow_quit_prompt_cg() {
    let repo = fixture_repo();
    let mut s = notes_store(repo.path());
    s.key_event(key("Q"));
    s.key_event(key("Z"));
    s.key_event(key("C-x"));
    s.key_event(key("C-c"));
    assert!(s.quit_prompt_active());
    s.key_event(key("C-g"));
    assert!(!s.quit && !s.quit_prompt_active(), "C-g cancels the quit");
    assert_eq!(s.message, "cancel");
    let intact = s.buffer_text().contains("QZ");
    // Re-quit: the buffer is still modified → the prompt returns.
    s.key_event(key("C-x"));
    s.key_event(key("C-c"));
    let reprompted = s.quit_prompt_active();
    s.key_event(key("n"));
    let finished = s.quit;
    assert!(
        intact && reprompted && finished,
        "intact={intact} reprompted={reprompted} finished={finished}"
    );
}

/// quit-prompt-save-fail: a failing save reports the error, RE-PROMPTS THE
/// SAME buffer (a second `y` still routes to the prompt), and `C-g`
/// cancels out with the file untouched.
#[test]
#[cfg(unix)]
fn unit_flow_quit_prompt_save_fail() {
    let repo = fixture_repo();
    let mut s = notes_store(repo.path());
    let notes_path = repo.path().join(".redline-notes.md");
    let seed = std::fs::read_to_string(&notes_path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&notes_path, std::fs::Permissions::from_mode(0o444)).unwrap();
    }
    s.key_event(key("F"));
    s.key_event(key("C-x"));
    s.key_event(key("C-c"));
    assert!(s.quit_prompt_active());
    s.key_event(key("y"));
    let failed = s.message.contains("save failed");
    // Re-prompt of the SAME buffer: a second `y` is still routed to the
    // prompt (and fails again), not treated as typing.
    s.key_event(key("y"));
    let reprompt_same = s.quit_prompt_active() && s.message.contains("save failed");
    s.key_event(key("C-g"));
    let cancelled = !s.quit && !s.quit_prompt_active();
    let on_disk = std::fs::read_to_string(&notes_path).unwrap();
    assert!(
        failed && reprompt_same && cancelled && on_disk == seed,
        "failed={failed} reprompt-same={reprompt_same} cancelled={cancelled} \
         file-unchanged={}",
        on_disk == seed
    );
}

/// A dedicated `src/edit.rs` fixture (created before startup so the walk
/// lists it; removed by the tempdir), mirroring the sweep's edit-mode legs.
fn edit_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = fixture_repo();
    let path = dir.path().join("src/edit.rs");
    std::fs::write(&path, "edit line one\nedit line two\n").unwrap();
    git(dir.path(), &["add", "src/edit.rs"]);
    (dir, path)
}

fn open_edit_file(store: &mut AppStore) {
    open_via_finder(store, "edit.rs");
}

/// edit-toggle: C-x C-q toggles Read-only ⇄ Accurate (015-02: the mode
/// toggle); the status line shows the mode word; with no edits the
/// toggle back is immediate (no confirm prompt).
#[test]
fn unit_flow_edit_toggle() {
    let (repo, _) = edit_repo();
    let mut s = store_in(repo.path());
    open_edit_file(&mut s);
    let ro = s.buffer_mode_display() == "Read-only";
    let frame = render80({
        let mut s2 = store_in(repo.path());
        open_edit_file(&mut s2);
        s2
    });
    let status_ro = frame.lines().last().unwrap().contains("Read-only");
    s.key_event(key("C-x"));
    s.key_event(key("C-q"));
    // 015-02 re-pin (was `== "Edit"`): C-x C-q enters the Accurate mode
    // (the file buffer's editable state is the mode itself).
    let edit_on = s.buffer_mode_display() == "Accurate";
    let editable_msg = s.message.contains("editable");
    // No edits yet: toggle straight back (no confirm prompt).
    s.key_event(key("C-x"));
    s.key_event(key("C-q"));
    let ro_again = s.buffer_mode_display() == "Read-only";
    let no_confirm = !s.message.contains("Discard unsaved edits");
    s.key_event(key("C-x"));
    s.key_event(key("C-q"));
    // 015-02 re-pin (was `== "Edit"`): the second toggle re-enters Accurate.
    let edit_again = s.buffer_mode_display() == "Accurate";
    assert!(
        ro && status_ro && edit_on && editable_msg && ro_again && no_confirm && edit_again,
        "ro={ro} status-ro={status_ro} edit={edit_on} msg={editable_msg} ro-again={ro_again} \
         no-confirm={no_confirm} edit-again={edit_again} (msg={:?})\n{frame}",
        s.message
    );
}

/// edit-save: type + save — the edit lands, C-x C-s writes it to disk
/// (asserted byte-for-byte), and the saved-path self-write is suppressed
/// (no false 'changed on disk' marker).
#[test]
fn unit_flow_edit_save() {
    let (repo, path) = edit_repo();
    let mut s = store_in(repo.path());
    open_edit_file(&mut s);
    s.key_event(key("C-x"));
    s.key_event(key("C-q")); // into edit mode
    // 015-03: Accurate mode inserts at the point; park the point at the end
    // of the buffer so the typed edit lands there (the flow asserts the disk
    // ends with it).
    s.key_event(key("M-END"));
    s.key_event(key("z"));
    s.key_event(key("z"));
    let typed = s.buffer_text().contains("zz");
    s.key_event(key("C-x"));
    s.key_event(key("C-s"));
    let wrote = s.message.contains("wrote");
    let disk = std::fs::read_to_string(&path).unwrap();
    let disk_ok = disk.ends_with("zz");
    // The watcher event for OUR OWN save (mtime still matches the
    // recorded one) must not raise a false marker.
    publish_change(&mut s, &path, ChangeKind::Modify);
    let no_false_marker = !s.current_buffer_changed_on_disk();
    assert!(
        typed && wrote && disk_ok && no_false_marker,
        "typed={typed} wrote={wrote} disk={disk_ok} no-false-marker={no_false_marker}"
    );
}

/// edit-confirm: toggle back with unsaved edits arms the discard confirm;
/// C-g cancels (edit mode + the text are kept); re-arming and `y` accepts:
/// the unsaved text is discarded (the buffer re-reads the disk content) and
/// the buffer goes read-only — the disk is never written on the confirm.
#[test]
fn unit_flow_edit_confirm() {
    let (repo, path) = edit_repo();
    let mut s = store_in(repo.path());
    open_edit_file(&mut s);
    let base = std::fs::read_to_string(&path).unwrap();
    // Into edit mode with unsaved edits, then toggle back: the discard
    // confirm arms (the flip is deferred until the answer).
    s.key_event(key("C-x"));
    s.key_event(key("C-q"));
    s.key_event(key("q"));
    s.key_event(key("q"));
    s.key_event(key("C-x"));
    s.key_event(key("C-q"));
    let prompt = s.message.contains("Discard unsaved edits");
    // 015-02 re-pin (was `== "Edit"`): the buffer is in the Accurate mode.
    let still_edit = s.buffer_mode_display() == "Accurate";
    s.key_event(key("C-g"));
    // 015-02 re-pin (was `== "Edit"`): cancel keeps the Accurate mode.
    let cancel_kept_edit = s.buffer_mode_display() == "Accurate";
    let cancel_kept_text = s.buffer_text().contains("qq");
    // Re-arm and accept: y discards the unsaved 'qq'.
    s.key_event(key("C-x"));
    s.key_event(key("C-q"));
    let prompt_again = s.message.contains("Discard unsaved edits");
    s.key_event(key("y"));
    let read_only = s.buffer_mode_display() == "Read-only";
    let discarded = !s.buffer_text().contains("qq");
    let disk_untouched = std::fs::read_to_string(&path).unwrap() == base;
    assert!(
        prompt && still_edit && cancel_kept_edit && cancel_kept_text
            && prompt_again && read_only && discarded && disk_untouched,
        "prompt={prompt} still-edit={still_edit} cancel-kept-edit={cancel_kept_edit} \
         cancel-kept-text={cancel_kept_text} re-armed={prompt_again} ro={read_only} \
         discarded={discarded} disk-untouched={disk_untouched} (msg={:?})",
        s.message
    );
}

/// edit-conflict: external change while in edit mode → 'changed on disk'
/// marker and NO auto-reload (the buffer text stays intact). Contrast:
/// a plain read-only file buffer still auto-reloads with no marker.
#[test]
fn unit_flow_edit_conflict() {
    let (repo, path) = edit_repo();
    let mut s = store_in(repo.path());
    open_edit_file(&mut s);
    // Contrast leg: read-only auto-reload, no marker.
    append(&path, "EXT1");
    publish_change(&mut s, &path, ChangeKind::Modify);
    let reloaded_ro = s.buffer_text().contains("EXT1");
    let no_marker_ro = !s.current_buffer_changed_on_disk();
    // Edit mode, type, then an external append must CONFLICT, not clobber.
    s.key_event(key("C-x"));
    s.key_event(key("C-q"));
    // 015-03: Accurate mode inserts at the point; park it at the end so the
    // typed edit lands after the externally-appended "EXT1" (the flow asserts
    // the buffer holds "EXT1qq").
    s.key_event(key("M-END"));
    s.key_event(key("q"));
    s.key_event(key("q"));
    append(&path, "EXT2");
    publish_change(&mut s, &path, ChangeKind::Modify);
    let marker = s.current_buffer_changed_on_disk();
    let edit_intact = s.buffer_text().contains("EXT1qq");
    let no_clobber = !s.buffer_text().contains("EXT2");
    assert!(
        reloaded_ro && no_marker_ro && marker && edit_intact && no_clobber,
        "ro-reload={reloaded_ro} no-marker-ro={no_marker_ro} marker={marker} \
         intact={edit_intact} no-clobber={no_clobber}"
    );
}

/// The inline-annotation suite's fixtures (created before startup so the
/// cached walk lists them).
fn ann_repo() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = fixture_repo();
    let ann = dir.path().join("src/notes_ann.rs");
    let ann2 = dir.path().join("src/notes_ann2.rs");
    let mut lines = "ann line one\nann line two\nann line three\nann line four\n"
        .to_string();
    for i in 5..=30 {
        lines.push_str(&format!("ann filler {i:02}\n"));
    }
    std::fs::write(&ann, lines).unwrap();
    let mut cu = String::new();
    for i in 1..=30 {
        cu.push_str(&format!("cu line {i}\n"));
    }
    std::fs::write(&ann2, cu).unwrap();
    git(dir.path(), &["add", "src/notes_ann.rs", "src/notes_ann2.rs"]);
    (dir, ann, ann2)
}

/// ann-create: A on a line → the prompt; typing + RET commits — a ▎ marker
/// on the code row, the note as a row directly ABOVE it (annotations-
/// render-fold), the status count, and a real record in .redline-notes.md.
#[test]
fn unit_flow_ann_create() {
    let (repo, _ann, _ann2) = ann_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "notes_ann.rs");
    s.key_event(key("C-n"));
    s.key_event(key("C-n")); // point at line 2 ("ann line three")
    s.key_event(key("A"));
    assert!(s.note_prompt_active(), "annotation prompt active");
    for c in "check bounds".chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("RET"));
    let saved = s.message.contains("note saved");
    assert!(saved, "note saved echo (msg={:?})", s.message);
    // Marker + note row directly ABOVE the annotated code row.
    let rows = s.file_view_rows();
    let code_idx = rows
        .iter()
        .position(|r| r.text == "ann line three")
        .expect("annotated code row present");
    let marker = rows[code_idx].annotated;
    let above = rows.get(code_idx - 1).map(|r| r.text.contains("check bounds")).unwrap_or(false);
    let not_below = rows.get(code_idx + 1).is_none_or(|r| !r.text.contains("check bounds"));
    // Status count.
    let count = s.annotation_count_display().contains("1 note");
    // The on-disk record.
    let notes = std::fs::read_to_string(repo.path().join(".redline-notes.md"))
        .expect("notes file created");
    let disk_rec = notes.contains("[annotation]")
        && notes.contains("note: check bounds")
        && notes.contains("anchor: ann line three");
    let frame = render80(s);
    assert!(
        marker && above && not_below && count && disk_rec,
        "marker={marker} above={above} not-below={not_below} count={count} disk={disk_rec}\n{frame}"
    );
}

/// ann-toggle (annotations-fold-visual): `C-c a h` is the TOGGLE — it
/// hides the note rows (the margin arrow switches ▾→▸) and `C-c a h`
/// again shows them (▸→▾). There is no separate `C-c a s`: one toggle,
/// not a pair.
#[test]
fn unit_flow_ann_toggle() {
    let (repo, _ann, _ann2) = ann_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "notes_ann.rs");
    s.key_event(key("C-n"));
    s.key_event(key("C-n"));
    s.key_event(key("A"));
    for c in "check bounds".chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("RET"));
    let with_note = |st: &mut AppStore| {
        st.file_view_rows()
            .iter()
            .any(|r| r.text.contains("check bounds"))
    };
    assert!(with_note(&mut s), "note row present before toggle");
    assert!(!s.note_rows_folded());
    s.key_event(key("C-c"));
    s.key_event(key("a"));
    s.key_event(key("h"));
    let hidden_msg = s.message.contains("note rows: hidden");
    let note_gone = !with_note(&mut s);
    let marker_stays = s
        .file_view_rows()
        .iter()
        .any(|r| r.text == "ann line three" && r.annotated);
    let folded_state = s.note_rows_folded();
    // C-c a h again: the same key TOGGLES back (no separate C-c a s).
    s.key_event(key("C-c"));
    s.key_event(key("a"));
    s.key_event(key("h"));
    let shown_msg = s.message.contains("note rows: shown");
    let note_back = with_note(&mut s);
    let unfolded_state = !s.note_rows_folded();
    assert!(
        hidden_msg && note_gone && marker_stays && folded_state
            && shown_msg && note_back && unfolded_state,
        "hidden={hidden_msg} gone={note_gone} marker={marker_stays} folded={folded_state} \
         shown={shown_msg} back={note_back} unfolded={unfolded_state}"
    );
}

/// ann-crossing: with the cursor on the annotated line, C-n lands on the
/// NEXT CODE line (L4, not the note row) and C-p returns (L3); the note
/// row now sits directly ABOVE the annotated code row (the rendered slice
/// reads note → code → next-code, not code → note).
#[test]
fn unit_flow_ann_crossing() {
    let (repo, _ann, _ann2) = ann_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "notes_ann.rs");
    s.key_event(key("C-n"));
    s.key_event(key("C-n"));
    s.key_event(key("A"));
    for c in "check bounds".chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("RET"));
    s.key_event(key("C-n"));
    let l4 = s.file_view_position_display().starts_with("L4,");
    s.key_event(key("C-p"));
    let l3 = s.file_view_position_display().starts_with("L3,");
    let rows = s.file_view_rows();
    let r3 = rows.iter().position(|r| r.text == "ann line three");
    let r4 = rows.iter().position(|r| r.text == "ann line four");
    let note_above = r3
        .zip(r4)
        .and_then(|(a, b)| (b - a == 1).then(|| rows[a - 1].is_note && rows[a - 1].line == 2))
        .unwrap_or(false);
    assert!(
        l4 && l3 && note_above,
        "L4={l4} L3={l3} note-above={note_above} (pos={:?})",
        s.file_view_position_display()
    );
}

/// ann-fold (annotations-render-fold): fold with the cursor mid-file — the
/// cursor stays on the same CODE line across the toggle, the scroll window
/// does not move (the row count changed but the point's buffer line is
/// unchanged), and the row map round-trips after each toggle. This is the
/// pinned trap: a fold that corrupted the map, the cursor line, or the
/// scroll position would fail one of the three legs.
#[test]
fn unit_flow_ann_fold() {
    let (repo, _ann, _ann2) = ann_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "notes_ann.rs");
    s.key_event(key("C-n"));
    s.key_event(key("C-n"));
    s.key_event(key("A"));
    for c in "check bounds".chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("RET"));
    // The cursor mid-file (line 10 of 30 — far from any note row).
    s.key_event(key("M-<"));
    for _ in 0..10 {
        s.key_event(key("C-n"));
    }
    let code_line = s.point_line();
    let scroll = s.scroll_top();
    let rows_shown = s.file_view_rows();
    assert_eq!(
        rows_shown.iter().filter(|r| r.is_note).count(),
        1,
        "the note row is emitted (unfolded): {:?}",
        rows_shown.iter().map(|r| (r.line, r.is_note)).collect::<Vec<_>>()
    );
    // C-c a h: hide (toggle). The cursor's code row and the scroll window
    // must both survive the row-count change untouched.
    s.key_event(key("C-c"));
    s.key_event(key("a"));
    s.key_event(key("h"));
    let hidden = s.message.contains("note rows: hidden");
    let rows_hidden = s.file_view_rows();
    let cursor_same_code_line = s.point_line() == code_line;
    let scroll_stable = s.scroll_top() == scroll;
    let note_gone = rows_hidden.iter().all(|r| !r.is_note);
    // The row map round-trips after the toggle: for every window line,
    // line_for_row(row_for_line(l)) == l (the map must survive the
    // row-count change, not merely stay non-empty).
    let lines = s.buffers.current_buffer().unwrap().line_count();
    let map_round_trips_hidden = (0..lines)
        .filter_map(|l| {
            crate::app::store::FileViewRow::row_for_line(&rows_hidden, l)
                .map(|r| (l, r))
        })
        .all(|(l, r)| crate::app::store::FileViewRow::line_for_row(&rows_hidden, r) == Some(l));
    // The margin indicator carries the folded state (the store side of the
    // UI marker switch: the rows still flag the annotated line).
    let marker_folded_state = s.note_rows_folded()
        && rows_hidden
            .iter()
            .any(|r| r.text == "ann line three" && r.annotated);
    // C-c a h again: show (toggle back — the single toggle, no C-c a s).
    // Same invariants back, and the note row is immediately ABOVE its code
    // row again.
    s.key_event(key("C-c"));
    s.key_event(key("a"));
    s.key_event(key("h"));
    let shown = s.message.contains("note rows: shown");
    let rows_shown2 = s.file_view_rows();
    let note_back_above = rows_shown2.iter().enumerate().any(|(i, r)| {
        r.is_note
            && r.text.contains("check bounds")
            && rows_shown2.get(i + 1).is_some_and(|n| !n.is_note && n.line == r.line)
    });
    let lines2 = s.buffers.current_buffer().unwrap().line_count();
    let map_round_trips_shown = (0..lines2)
        .filter_map(|l| {
            crate::app::store::FileViewRow::row_for_line(&rows_shown2, l)
                .map(|r| (l, r))
        })
        .all(|(l, r)| crate::app::store::FileViewRow::line_for_row(&rows_shown2, r) == Some(l));
    assert!(
        hidden && cursor_same_code_line && scroll_stable && note_gone
            && map_round_trips_hidden && marker_folded_state
            && shown && s.point_line() == code_line && s.scroll_top() == scroll
            && note_back_above && map_round_trips_shown && !s.note_rows_folded(),
        "hidden={hidden} cursor-same-code-line={cursor_same_code_line} \
         scroll-stable={scroll_stable} note-gone={note_gone} map-hidden={map_round_trips_hidden} \
         marker-folded={marker_folded_state} shown={shown} \
         note-back-above={note_back_above} map-shown={map_round_trips_shown}"
    );
}

/// ann-aliases (annotations-render-fold): `C-c a n` reaches the
/// new-annotation prompt (the `A` command) and `C-c a l` reaches the
/// annotations picker (the `C-c n a` command) — the genuine aliases through
/// the store's own key_event path, not a binding-table read.
#[test]
fn unit_flow_ann_aliases() {
    let (repo, _ann, _ann2) = ann_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "notes_ann.rs");
    s.key_event(key("C-n"));
    s.key_event(key("C-n"));
    // C-c a n: the new-annotation prompt on the line at point.
    s.key_event(key("C-c"));
    s.key_event(key("a"));
    s.key_event(key("n"));
    let prompt = s.note_prompt_active();
    let prompt_text = s.message.contains("Note: ");
    s.key_event(key("C-g")); // cancel (no record written)
    let cancelled = !s.note_prompt_active();
    // C-c a l: the annotations picker (zero annotations at this point —
    // the picker opens empty, 0/0).
    s.key_event(key("C-c"));
    s.key_event(key("a"));
    s.key_event(key("l"));
    let picker = s.picker_open();
    let picker_kind = s.picker_kind() == Some(crate::app::store::PickerKind::Annotations);
    let empty_list = s.picker_count() == (0, 0);
    s.key_event(key("C-g"));
    let closed = !s.picker_open();
    assert!(
        prompt && prompt_text && cancelled && picker && picker_kind && empty_list && closed,
        "prompt={prompt} prompt-text={prompt_text} cancelled={cancelled} \
         picker={picker} kind={picker_kind} empty={empty_list} closed={closed} (msg={:?})",
        s.message
    );
}

/// ann-notes-editable: in the EDITABLE notes buffer `d`/`A` self-insert as
/// printables (no prompt, no silent delete, no unbound-key echo).
#[test]
fn unit_flow_ann_notes_editable() {
    let (repo, _ann, _ann2) = ann_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("n"));
    let edit_mode = s.buffer_mode_display() == "Edit";
    let before = s.buffer_text().clone();
    s.key_event(key("d"));
    let no_delete = !s.message.contains("deleted annotation")
        && !s.message.contains("no annotation");
    let self_inserted = s.buffer_text().ends_with('d')
        && s.buffer_text().len() > before.len();
    s.key_event(key("A"));
    let no_prompt = !s.message.contains("Note: ");
    let a_inserted = s.buffer_text().ends_with("dA");
    assert!(
        edit_mode && no_delete && self_inserted && no_prompt && a_inserted,
        "edit-mode={edit_mode} no-delete={no_delete} d-inserted={self_inserted} \
         no-prompt={no_prompt} A-inserted={a_inserted} (text={:?} msg={:?})",
        s.buffer_text(),
        s.message
    );
}

/// ann-drift: an out-of-band edit moves the anchored line — the
/// auto-reload's re-anchor pass moves the cue with the content.
#[test]
fn unit_flow_ann_drift() {
    let (repo, ann, _ann2) = ann_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "notes_ann.rs");
    s.key_event(key("C-n"));
    s.key_event(key("C-n"));
    s.key_event(key("A"));
    for c in "check bounds".chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("RET"));
    let old = std::fs::read_to_string(&ann).unwrap();
    std::fs::write(&ann, format!("top extra line\n{old}")).unwrap();
    publish_change(&mut s, &ann, ChangeKind::Modify);
    let reloaded = s.buffer_text().starts_with("top extra line");
    let marker_moved = s
        .file_view_rows()
        .iter()
        .any(|r| r.text == "ann line three" && r.annotated);
    // annotations-render-fold: the note row renders ABOVE the anchored line
    // (the drift re-anchor keeps the cue attached to the content — now as
    // a header, not a trailer).
    let note_above = s.file_view_rows().iter().enumerate().any(|(i, r)| {
        r.is_note
            && r.text.contains("check bounds")
            && s.file_view_rows()
                .get(i + 1)
                .is_some_and(|n| n.text == "ann line three" && !n.is_note)
    });
    let count = s.annotation_count_display().contains("1 note");
    assert!(
        reloaded && marker_moved && note_above && count,
        "reloaded={reloaded} marker={marker_moved} note={note_above} count={count}"
    );
}

/// ann-orphan: delete the anchored line out-of-band — the annotation is
/// NOT lost: it is flagged orphaned and the record stays in the notes
/// file.
#[test]
fn unit_flow_ann_orphan() {
    let (repo, ann, _ann2) = ann_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "notes_ann.rs");
    s.key_event(key("C-n"));
    s.key_event(key("C-n"));
    s.key_event(key("A"));
    for c in "check bounds".chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("RET"));
    let text = std::fs::read_to_string(&ann).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| *l != "ann line three").collect();
    std::fs::write(&ann, lines.join("\n") + "\n").unwrap();
    publish_change(&mut s, &ann, ChangeKind::Modify);
    let orphaned = s
        .file_view_rows()
        .iter()
        .any(|r| r.text.contains("orphaned"));
    let record_kept = s.annotation_count_display().contains("1 note");
    let disk_kept = std::fs::read_to_string(repo.path().join(".redline-notes.md"))
        .unwrap()
        .contains("note: check bounds");
    assert!(
        orphaned && record_kept && disk_kept,
        "orphaned={orphaned} record={record_kept} disk={disk_kept}"
    );
}

/// ann-delete (state twin): d on the annotated line deletes the record —
/// cue gone, count gone, disk record gone; d on an unannotated line is a
/// no-op with a clear message. (The PTY leg KEEPS the transient-echo /
/// repaint-race assertion — the thin tier's raison d'être.)
#[test]
fn unit_flow_ann_delete() {
    let (repo, _ann, _ann2) = ann_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "notes_ann.rs");
    s.key_event(key("C-n"));
    s.key_event(key("C-n"));
    s.key_event(key("A"));
    for c in "check bounds".chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("RET"));
    s.key_event(key("d"));
    let echo = s.message.contains("deleted annotation: check bounds");
    let cue_gone = !s
        .file_view_rows()
        .iter()
        .any(|r| r.text.contains("check bounds"));
    let count_gone = !s.annotation_count_display().contains("1 note");
    let disk_gone = !std::fs::read_to_string(repo.path().join(".redline-notes.md"))
        .unwrap()
        .contains("check bounds");
    // d on an unannotated line: the clear no-op message (issue-annotation-
    // per-symbol-creation: the delete keys on the record at the point's
    // cell, so the message says "at point").
    s.key_event(key("C-p"));
    s.key_event(key("d"));
    let no_ann_msg = s.message.contains("no annotation at point");
    assert!(
        echo && cue_gone && count_gone && disk_gone && no_ann_msg,
        "echo={echo} cue-gone={cue_gone} count-gone={count_gone} disk-gone={disk_gone} \
         no-ann-msg={no_ann_msg} (msg={:?})",
        s.message
    );
}

/// ann-cu guard: C-u is still half-page scroll (the annotate bindings did
/// not shadow it). At the bottom of a 30-line file the window top is
/// "cu line 11"; C-u moves the window up a half-page to "cu line 1".
#[test]
fn unit_flow_ann_cu() {
    let (repo, _ann, _ann2) = ann_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "notes_ann2.rs");
    s.key_event(key("G"));
    let at_bottom = s.file_view_position_display() == "Bot";
    let top_before = s.file_view_rows().iter().find(|r| !r.text.is_empty()).map(|r| r.text.clone());
    s.key_event(key("C-u"));
    let top_after = s.file_view_rows().iter().find(|r| !r.text.is_empty()).map(|r| r.text.clone());
    let moved_up = top_before.as_deref() == Some("cu line 11") && top_after.as_deref() == Some("cu line 1");
    assert!(
        at_bottom && moved_up && top_before != top_after,
        "at-bottom={at_bottom} top {top_before:?} -> {top_after:?} moved-up={moved_up}"
    );
}

/// U-M1..M7: mark/region/kill/yank — C-SPC sets the mark, C-n/C-p move the
/// point (region active), M-w copies to the ring, C-y yanks cross-buffer,
/// M-y yank-pops (end of ring), and C-g clears the region.
#[test]
fn unit_flow_mark_kill_yank() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "main");
    s.key_event(key("C-n"));
    s.key_event(key("C-n")); // point at line 2
    s.key_event(key_null()); // C-SPC (NUL): set the mark
    let mark_set = s.message == "Mark set";
    s.key_event(key("C-p"));
    s.key_event(key("C-p")); // point back at line 0
    let region = s.region_line_range();
    assert!(region.is_some(), "region active: {region:?}");
    // M-w: copy the region to the kill ring (read-only: buffer unchanged).
    let text_before = s.buffer_text().clone();
    s.key_event(key("M-w"));
    let copy_echo = s.message.contains("copied to kill ring");
    let ring = s.kill_ring.len() == 1;
    let buffer_unchanged = s.buffer_text() == text_before;
    // Cross-buffer yank into the notes buffer.
    s.key_event(key("C-x"));
    s.key_event(key("n"));
    let before = s.buffer_text().clone();
    s.key_event(key("C-y"));
    let yank_landed = s.buffer_text() != before;
    let yank_done = !s.message.contains("Kill ring is empty");
    // M-y: yank-pop (only one ring entry → "end of kill ring", not a panic).
    s.key_event(key("M-y"));
    let alive_after_m_y = !s.message.contains("unbound");
    // C-g clears the region (back on the file view).
    s.key_event(key("C-x"));
    s.key_event(key("C-f"));
    for c in "main".chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("RET"));
    s.key_event(key("C-g"));
    let region_cleared = s.region_line_range().is_none();
    assert!(
        mark_set && ring && copy_echo && buffer_unchanged && yank_landed && yank_done
            && alive_after_m_y && region_cleared,
        "mark={mark_set} ring={ring} copy={copy_echo} unchanged={buffer_unchanged} \
         yank={yank_landed}/{yank_done} m-y-alive={alive_after_m_y} cleared={region_cleared} \
         (msg={:?}, region={region:?})",
        s.message
    );
}

/// U-M8: C-x C-x exchanges point and mark (point moves to the mark's
/// line; a second exchange round-trips).
#[test]
fn unit_flow_mark_exchange() {
    let repo = fixture_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "main");
    s.key_event(key_null()); // mark at line 0
    let mark_set = s.message == "Mark set";
    s.key_event(key("C-n"));
    s.key_event(key("C-n")); // point at line 2
    let point_b = s.file_view_point().0;
    s.key_event(key("C-x"));
    s.key_event(key("C-x"));
    let point_a = s.file_view_point().0;
    let exchanged_to_mark = point_a != point_b && point_a == 0;
    s.key_event(key("C-x"));
    s.key_event(key("C-x"));
    let roundtrip = s.file_view_point().0 == point_b;
    assert!(
        mark_set && exchanged_to_mark && roundtrip,
        "mark={mark_set} point {point_b}->{point_a} roundtrip={roundtrip}"
    );
}

// ═══════════════ batch 4: loop-04 — the other suites' unit twins ════════════
// Below-PTY twins of tools/drive_windowing.py, drive_windowing_panes.py,
// drive_xref.py, drive_external_notes.py, drive_external_crate.py,
// drive_external_use.py, drive_syntax_notes.py, and ux_sweep.py's keymap
// coverage (same discipline as batches 1-3: `key_event` with the exact Key
// the Root terminal path produces, state asserts + `render80`). The kept/
// converted ledger lives in docs/ux-testing-plan.md; the thin PTY tiers
// keep the terminal-only residue (input encoding, raw pixels, process
// lifecycle, hardware cursor).
//
// Provider-level resolution (cargo metadata against the registry / stdlib
// lookup) belongs to the crates/redline-resolve corpus. Here the provider's
// OUTPUT is published provider-shaped (`land_resolved` / `miss_resolved` /
// `apply_crate_index_event`) exactly as the UI's bus drains would — the
// store-side landing / reporting / ownership-guard / recenter behavior is
// what these twins pin.

use super::{format_notes_dump, CrateIndexEvent, PickerKind, ResolveEvent};
use redline_resolve::ResolvedSource;

/// A provider-shaped HIT for the in-flight M-. job (the store's child
/// module reads its own generation counter — the same shape the UI's
/// ResolveBus drain publishes).
fn land_resolved(
    s: &mut AppStore,
    file: &std::path::Path,
    source_root: &std::path::Path,
    line: u32,
) {
    let event = ResolveEvent {
        generation: s.resolve_generation,
        symbol: String::new(),
        source: Some(ResolvedSource {
            file: file.to_path_buf(),
            source_root: source_root.to_path_buf(),
            external: true,
            line: Some(line),
        }),
        error: None,
    };
    s.apply_resolve_event(&event);
}

/// A provider-shaped MISS for the in-flight job: the store's graceful miss
/// report path (the provider's own error text is the corpus's). F2 (plan
/// 017 audit): the EXACT single-attempt shape the real chain now reports —
/// the provider's own reason LEADS (the status line clips the right edge),
/// the generic `(tried 1 provider(s): rust)` tail follows byte-for-byte.
fn miss_resolved(s: &mut AppStore, symbol: &str, detail: &str) {
    let event = ResolveEvent {
        generation: s.resolve_generation,
        symbol: symbol.to_string(),
        source: None,
        error: Some(format!(
            "{detail}; no tooling provider could resolve symbol `{symbol}` (tried 1 provider(s): rust)"
        )),
    };
    s.apply_resolve_event(&event);
}

/// drive_windowing.py's fixture (/tmp/redline_tall_repo): 30 changed files
/// — the magit status buffer (~34 rows) exceeds the 19-row magit window, so
/// the cursor-following window must scroll under `n`.
fn tall_magit_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    for i in 0..30 {
        std::fs::write(p.join(format!("f{i:02}.txt")), "base\n").unwrap();
    }
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Test"]);
    git(p, &["config", "user.email", "test@example.com"]);
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "init"]);
    for i in 0..30 {
        append(p.join(format!("f{i:02}.txt")).as_path(), "change\n");
    }
    dir
}

/// drive_windowing_panes.py's LOG_REPO: 30 commits — page 1 (25 entries)
/// exceeds the log's 19-row window.
fn log_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Test"]);
    git(p, &["config", "user.email", "test@example.com"]);
    for i in 0..30 {
        std::fs::write(p.join(format!("s{i}.txt")), format!("filler {i}\n")).unwrap();
        git(p, &["add", "-A"]);
        git(p, &["commit", "-q", "-m", &format!("commit {i}")]);
    }
    dir
}

/// The drive_xref.py fixture: a `leg.rs` with the cross-file call, the
/// `tokio::spawn` path token, the external path-dep probe, and the bare-
/// `other` miss line; a 220-line `long.rs` whose `far_target` definition
/// sits at 1-based line 160; and the `call.rs` call site.
fn xref_repo() -> tempfile::TempDir {
    let dir = fixture_repo();
    let p = dir.path();
    std::fs::write(
        p.join("src/leg.rs"),
        "fn leg() {\n    target_lib();\n}\ntokio::spawn(f);\nextdep::ext_target();\nlet other = 9;\n",
    )
    .unwrap();
    let mut lines = String::new();
    for i in 1..=220 {
        if i == 160 {
            lines.push_str("pub fn far_target() {\n    // body\n}\n");
        } else {
            lines.push_str(&format!("// filler {i}\n"));
        }
    }
    std::fs::write(p.join("src/long.rs"), lines).unwrap();
    std::fs::write(p.join("src/call.rs"), "fn caller() {\n    far_target();\n}\n").unwrap();
    git(p, &["add", "src/leg.rs", "src/long.rs", "src/call.rs"]);
    dir
}

/// The external-suite fixture: a project with the top-level probe call
/// (src/probe.rs) + a fake registry-crate root OUTSIDE it (src/rope.rs the
/// landing file — `Rope` + the `RopeBuilder` call on 1-based line 3; and
/// src/rope_builder.rs, the in-crate M-. target). The store-side twin of
/// the ropey-1.6.1 registry source the PTY suites land in.
fn ext_notes_setup() -> (tempfile::TempDir, tempfile::TempDir) {
    let repo = fixture_repo();
    let ext = tempfile::tempdir().unwrap();
    let p = ext.path();
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(
        p.join("src/rope.rs"),
        "pub struct Rope;\npub fn make() {\n    RopeBuilder::new();\n}\n",
    )
    .unwrap();
    std::fs::write(
        p.join("src/rope_builder.rs"),
        "pub struct RopeBuilder;\n",
    )
    .unwrap();
    std::fs::write(
        repo.path().join("src/probe.rs"),
        "fn main() {\n    let r = ropey::Rope::new();\n}\nropey::Rope::new();\n",
    )
    .unwrap();
    git(repo.path(), &["add", "src/probe.rs"]);
    (repo, ext)
}

/// drive_external_use.py's repo: the use-scope probes (bare `Deserialize`
/// line 2, bare unimported `other` line 3).
fn ext_use_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(
        p.join("src/main.rs"),
        "use serde::Deserialize;\nlet d = Deserialize;\nlet other = 9;\n",
    )
    .unwrap();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Test"]);
    git(p, &["config", "user.email", "test@example.com"]);
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "use-scope"]);
    dir
}

/// drive_syntax_notes.py's fixture: src/synleg.rs with the single target fn.
fn synleg_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = fixture_repo();
    let leg = dir.path().join("src/synleg.rs");
    std::fs::write(&leg, "fn target_one() {\n    let x = 1;\n    x\n}\n").unwrap();
    git(dir.path(), &["add", "src/synleg.rs"]);
    (dir, leg)
}

/// `M-g g <n> RET` through the key path (the PTY legs' goto-line).
fn goto_line(s: &mut AppStore, n: u32) {
    s.key_event(key("M-g"));
    s.key_event(key("g"));
    for c in n.to_string().chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("RET"));
}

// ═══════════════ windowing (drive_windowing.py / _panes.py) ═══════════════

/// drive_windowing (28-step `n`-follow on a 30-file status): the
/// cursor-following magit window keeps the (single) cursor row in view on
/// every step, the view stays open, and the window actually scrolls.
#[test]
fn unit_flow_win_magit_follow() {
    let repo = tall_magit_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("g"));
    assert_eq!(s.top_view(), ViewId::MagitStatus, "magit status open");
    let (_win, top0, total) = s.magit_view_info();
    assert!(total > 19, "status buffer taller than the window: {total}");
    let in_window = |s: &AppStore| -> bool {
        let (win, top, total) = s.magit_view_info();
        s.magit_rows()
            .iter()
            .position(|r| r.selected)
            .map(|i| i >= top && i < top + win.len() && i < total)
            .unwrap_or(false)
    };
    let single = |s: &AppStore| -> bool {
        s.magit_rows().iter().filter(|r| r.selected).count() == 1
    };
    assert!(single(&s) && in_window(&s), "initial: one cursor row, in view");
    let mut all_ok = true;
    let mut prev_top = top0;
    let mut scrolled = 0usize;
    for _ in 1..=27 {
        s.key_event(key("n"));
        all_ok &= s.top_view() == ViewId::MagitStatus && single(&s) && in_window(&s);
        let top = s.magit_view_info().1;
        if top != prev_top {
            scrolled += 1;
            prev_top = top;
        }
    }
    assert!(
        all_ok,
        "cursor in view (single blue row) on all 27 n-steps"
    );
    assert!(
        scrolled > 0 && prev_top > top0,
        "window scrolled on {scrolled}/27 steps (top {top0}->{prev_top})"
    );
    let frame = render80(s);
    assert!(frame.contains("s stage"), "pinned help line at width 80: {frame}");
}

/// drive_windowing_panes commit-diff: M-> lands on the FULL last page (the
/// sentinel AND its neighbor are both visible — the full-last-page
/// semantic), M-< round-trips to the top, and C-n / C-p step the diff
/// window one row.
#[test]
fn unit_flow_panes_diff() {
    let repo = win_diff_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "big");
    s.key_event(key("C-x"));
    s.key_event(key("g"));
    s.key_event(key("l"));
    s.key_event(key("RET")); // newest (tall) commit's diff
    assert_eq!(s.top_view(), ViewId::CommitDiff, "commit diff open");
    let has = |s: &AppStore, t: &str| {
        s.commit_diff_view_info()
            .0
            .iter()
            .any(|r| r.text.contains(t))
    };
    assert!(!has(&s, "BOTTOM_SENTINEL"), "sentinel hidden at top");
    s.key_event(key("M->"));
    assert!(
        has(&s, "BOTTOM_SENTINEL") && has(&s, "line 59"),
        "full last page: sentinel AND the row before it are visible"
    );
    s.key_event(key("M-<"));
    assert!(!has(&s, "BOTTOM_SENTINEL"), "M-< round-trips to the top");
    // C-n steps the window down one row; C-p steps it back.
    let top = |s: &AppStore| {
        s.commit_diff_view_info().0.first().map(|r| r.text.clone())
    };
    let before = top(&s);
    s.key_event(key("C-n"));
    let after = top(&s);
    assert_ne!(before, after, "C-n steps the diff window down");
    s.key_event(key("C-p"));
    assert_eq!(top(&s), before, "C-p steps the diff window back");
    let frame = render80(s);
    assert!(
        frame.contains("read-only") && frame.contains("commit diff"),
        "pane still open after the round trip: {frame}"
    );
}

/// drive_windowing_panes log: the in-page selection stays in view across
/// many in-page moves on a page taller than the window; `n` (next page)
/// resets the in-page window to the top with the selection on the first
/// entry.
#[test]
fn unit_flow_panes_log() {
    let repo = log_repo();
    let mut s = store_in(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("g"));
    s.key_event(key("l"));
    assert_eq!(s.top_view(), ViewId::Log, "log open");
    let entries = s.log.as_ref().map(|l| l.entries.len()).unwrap_or(0);
    let (win0, _top, _total) = s.log_view_info();
    assert!(entries > win0.len(), "page ({entries}) taller than the window");
    let in_window = |s: &AppStore| -> bool {
        let (win, top, total) = s.log_view_info();
        let sel = s.log.as_ref().map(|l| l.selected).unwrap_or(0);
        !win.is_empty() && top <= sel && sel < top + win.len() && sel < total
    };
    let mut all_in = true;
    for _ in 0..20 {
        s.key_event(key("DOWN"));
        all_in &= in_window(&s);
    }
    assert!(all_in, "in-page selection in view across 20 down-moves");
    s.key_event(key("n")); // next page
    let paged = in_window(&s)
        && s
            .log
            .as_ref()
            .map(|l| (l.selected, l.offset))
            .unwrap_or((0, 0))
            == (0, 25);
    assert!(paged, "after next page: selection on the first entry, in view");
}

// ═══════════════ xref (drive_xref.py L1-L6) ════════════════════════════════

/// L1 same-file jump: M-. at the END of the `target_one` call jumps to the
/// SAME-FILE definition (struct+impl-style case), the window lands on the
/// definition, and M-, returns to the call site.
#[test]
fn unit_flow_xref_l1() {
    let repo = xref_repo();
    let mut s = store_in(repo.path());
    install_index(&mut s, repo.path());
    open_via_finder(&mut s, "main");
    goto_line(&mut s, 6); // "    target_one()"
    s.key_event(key("M-f")); // end of the `target_one` run
    s.key_event(key("M-."));
    let msg_ok = s.message == "jumped to src/main.rs: 1";
    let landed = s.point_line() == 0;
    let top_def = s
        .file_view_rows()
        .iter()
        .find(|r| !r.text.is_empty())
        .map(|r| r.text.contains("fn target_one"))
        .unwrap_or(false);
    s.key_event(key("M-,")); // jump-back (the landing recorded a jump)
    let back = s.point_line() == 5;
    let msg = s.message.clone();
    let frame = render80(s);
    let call_back = frame.contains("target_one();");
    assert!(
        msg_ok && landed && top_def && back && call_back,
        "msg={msg:?} landed={landed} top-def={top_def} back-line={back} call-back={call_back}\n{frame}"
    );
}

/// The same-file M-. origin-accuracy twin (user report: "when I jump to
/// definition in the SAME file, popping back can go to the wrong place, not
/// where my cursor was"): after a first same-file M-., the cursor moves by
/// PLAIN MOTION (no jump recorded — the jump stack's current-position slot
/// goes stale), then a second same-file M-. from the new point (with other
/// symbols around). M-, must restore BOTH the point line AND column at the
/// SECOND jump's origin (the stale slot held the first jump's destination —
/// the "wrong place") AND the window, recentered on that origin line.
#[test]
fn unit_flow_xref_l1b_same_file_origin() {
    let repo = xref_repo();
    let p = repo.path();
    // A tall same-file fixture: `alpha` (0-based line 22), `beta` (line 30,
    // the other symbol around), call sites (line 41: `    let x = alpha();`,
    // line 42: `    let y = beta();`; each symbol runs from col 12) — 50
    // lines total so the 21-row window recenter is discriminating.
    let mut body = String::new();
    for i in 0..22 {
        body.push_str(&format!("// filler {i}\n"));
    }
    body.push_str("pub fn alpha() -> u32 { 1 }\n");
    for i in 23..30 {
        body.push_str(&format!("// filler {i}\n"));
    }
    body.push_str("pub fn beta() -> u32 { 2 }\n");
    for i in 31..40 {
        body.push_str(&format!("// filler {i}\n"));
    }
    body.push_str("fn caller() {\n    let x = alpha();\n    let y = beta();\n}\n");
    for i in 44..50 {
        body.push_str(&format!("// filler {i}\n"));
    }
    std::fs::write(p.join("src/sf.rs"), body).unwrap();
    git(p, &["add", "src/sf.rs"]);

    let mut s = store_in(repo.path());
    install_index(&mut s, repo.path());
    open_via_finder(&mut s, "sf");
    goto_line(&mut s, 42); // 1-based 42 = 0-based 41: the alpha call site
    // Twelve char-forwards (C-f) from col 0 → col 12, inside the `alpha`
    // run. Char motion: deterministic, no word-skip.
    for _ in 0..12 {
        s.key_event(key("C-f"));
    }
    assert_eq!(s.point_line(), 41, "origin line (call site)");
    assert_eq!(s.point_col(), 12, "origin col (inside the alpha run)");
    let origin_top = s.scroll_top();

    s.key_event(key("M-.")); // same-file unique def → direct jump
    assert!(!s.picker_open(), "unique same-file: no picker");
    assert_eq!(s.point_line(), 22, "landed on the same-file alpha def");

    // Plain motion (goto-line + C-f, no jump recorded) to the BETA call
    // site: the jump stack's current-position slot now holds the alpha
    // landing (line 22), not the cursor (line 42).
    goto_line(&mut s, 43); // 1-based 43 = 0-based 42: the beta call site
    for _ in 0..12 {
        s.key_event(key("C-f"));
    }
    assert_eq!((s.point_line(), s.point_col()), (42, 12), "cursor at the beta call");

    s.key_event(key("M-.")); // second same-file jump, from the new point
    assert_eq!(s.point_line(), 30, "landed on the same-file beta def");

    s.key_event(key("M-,")); // jump-back
    assert_eq!(s.point_line(), 42, "M-, restores the origin LINE (not the stale alpha landing)");
    assert_eq!(s.point_col(), 12, "M-, restores the origin COLUMN");
    // The origin line is recentered in the 21-row window (store_in's PTY
    // viewport): (42 - 10) clamped to max_scroll = total - 21.
    let total = s.current_line_count();
    assert!(total >= 50, "tall fixture: {total} lines");
    let expected = (42u32 - 10).min((total - 21) as u32) as usize;
    assert_eq!(
        s.scroll_top(),
        expected,
        "M-, recenters the window on the origin line (pre-jump top {origin_top})"
    );
}

/// L2 cross-file jump: M-. on `target_lib` in a fresh src/leg.rs lands in
/// src/lib.rs on the definition; M-, returns to the leg.rs call site
/// (line AND column — the cross-file jump-back twin: the same-file
/// origin-accuracy fix must not degrade it).
#[test]
fn unit_flow_xref_l2() {
    let repo = xref_repo();
    let mut s = store_in(repo.path());
    install_index(&mut s, repo.path());
    open_via_finder(&mut s, "leg");
    goto_line(&mut s, 2); // "    target_lib();"
    s.key_event(key("M-f")); // end of `target_lib`
    let origin_line = s.point_line();
    let origin_col = s.point_col();
    s.key_event(key("M-."));
    // (jump-ambiguity) a cross-file UNIQUE candidate opens the picker
    // (best preselected); RET accepts the top guess.
    assert!(s.picker_open(), "cross-file unique: picker");
    s.key_event(key("RET"));
    let msg_ok = s.message == "jumped to src/lib.rs:1";
    let in_lib = s
        .buffers
        .current()
        .map(String::from)
        .map(|k| k.ends_with("src/lib.rs"))
        .unwrap_or(false);
    let top_def = s
        .file_view_rows()
        .iter()
        .find(|r| !r.text.is_empty())
        .map(|r| r.text.contains("pub fn target_lib"))
        .unwrap_or(false);
    s.key_event(key("M-,")); // jump-back (the landing recorded a jump)
    let back = s
        .buffers
        .current()
        .map(String::from)
        .map(|k| k.ends_with("src/leg.rs"))
        .unwrap_or(false)
        && (s.point_line(), s.point_col()) == (origin_line, origin_col);
    assert!(
        msg_ok && in_lib && top_def && back,
        "msg={:?} in-lib={in_lib} top-def={top_def} back={back}",
        s.message
    );
}

/// L3 bare miss: cursor at the end of `other` in `let other = 9;` — not in
/// the index (a let binding), no enclosing symbol → resolver fall-through
/// with the BARE name; the provider-side miss (the corpus's) lands through
/// the store's graceful-report path: the report names `other`, never the
/// path token.
#[test]
fn unit_flow_xref_l3() {
    let repo = xref_repo();
    let mut s = store_in(repo.path());
    install_index(&mut s, repo.path());
    open_via_finder(&mut s, "leg");
    goto_line(&mut s, 6); // "let other = 9;"
    s.key_event(key("M-f")); // end of `let`
    s.key_event(key("M-f")); // end of `other`
    let before_key = s.buffers.current().map(String::from);
    s.key_event(key("M-."));
    let bare = s.message.contains("no provider resolution for `other`")
        && !s.message.contains("`tokio`");
    assert!(bare, "fall-through carries the BARE symbol: {:?}", s.message);
    let before_line = s.point_line();
    miss_resolved(&mut s, "other", "no crate `other`");
    let graceful = s.message.starts_with("no provider resolution for `other`:")
        && s.message.contains("tried 1 provider(s): rust");
    assert!(graceful, "graceful resolver report: {:?}", s.message);
    assert_eq!(s.point_line(), before_line, "a miss never moves the point");
    assert_eq!(
        s.buffers.current().map(String::from),
        before_key,
        "buffer unchanged on a miss"
    );
}

/// L4 path token: cursor at the end of `tokio` in `tokio::spawn(f);` — the
/// resolver gets the raw PATH token `tokio::spawn` (the raw token under the
/// point), and the miss report names it.
#[test]
fn unit_flow_xref_l4() {
    let repo = xref_repo();
    let mut s = store_in(repo.path());
    install_index(&mut s, repo.path());
    open_via_finder(&mut s, "leg");
    goto_line(&mut s, 4); // "tokio::spawn(f);"
    s.key_event(key("M-f")); // end of `tokio`
    s.key_event(key("M-."));
    let raw_token = s
        .message
        .contains("no provider resolution for `tokio::spawn`");
    assert!(raw_token, "resolver gets the raw path token: {:?}", s.message);
    miss_resolved(&mut s, "tokio::spawn", "no crate `tokio`");
    let graceful = s.message.starts_with("no provider resolution for `tokio::spawn`:")
        && s.message.contains("tried 1 provider(s): rust");
    assert!(graceful, "graceful report names the path token: {:?}", s.message);
}

/// L5 ownership guard (006-02b item 1): a landing OUTSIDE the project root
/// arrives via the same external-landing path as a registry source
/// (read-only); C-x C-q AND C-x C-s are refused there. (The path-dep's own
/// resolution through cargo metadata is the resolver corpus's leg.)
#[test]
fn unit_flow_xref_l5() {
    let repo = xref_repo();
    let ext = ext_notes_setup().1; // crate root outside the project
    let target = ext.path().join("src/rope_builder.rs");
    let mut s = store_in(repo.path());
    install_index(&mut s, repo.path());
    open_via_finder(&mut s, "leg");
    goto_line(&mut s, 5); // "extdep::ext_target();"
    s.key_event(key("M-f")); // end of the `extdep` run
    s.key_event(key("M-.")); // fall-through (path dep outside the root)
    land_resolved(&mut s, &target, ext.path(), 1);
    // (jump-ambiguity) the tooling hit joins the Xref picker (the
    // `tooling`-marked row, preselected); RET accepts the top guess.
    assert!(s.picker_open(), "the tooling hit joins the picker");
    s.key_event(key("RET"));
    let abs = target.to_string_lossy().into_owned();
    let landed = s.message.contains(&format!("jumped to {abs}:1"));
    let read_only = s.buffer_mode_display() == "Read-only";
    assert!(landed && read_only, "msg={:?} ro={read_only}", s.message);
    s.key_event(key("C-x"));
    s.key_event(key("C-q"));
    let ref1 = s.message.contains("external buffer is read-only (not project-owned)");
    s.key_event(key("C-x"));
    s.key_event(key("C-s"));
    let ref2 = s.message.contains("external buffer is read-only (not project-owned)");
    assert!(
        ref1 && ref2 && s.buffer_mode_display() == "Read-only",
        "C-x C-q={ref1} C-x C-s={ref2} (msg={:?})",
        s.message
    );
}

/// L6 middle landing (plan 004 issue 07): M-. from a call to a definition
/// ~150 lines below lands the definition on the MIDDLE row of the content
/// area (the xref-after-jump-hook recenter), not the last content row.
#[test]
fn unit_flow_xref_l6() {
    let repo = xref_repo();
    let mut s = store_in(repo.path());
    install_index(&mut s, repo.path());
    open_via_finder(&mut s, "call");
    goto_line(&mut s, 2); // "    far_target();"
    s.key_event(key("M-f")); // end of the `far_target` run
    s.key_event(key("M-."));
    // (jump-ambiguity) a cross-file UNIQUE candidate opens the picker
    // (best preselected); RET accepts the top guess.
    assert!(s.picker_open(), "cross-file unique: picker");
    s.key_event(key("RET"));
    let msg_ok = s.message.contains("jumped to src/long.rs:160");
    let def = s
        .file_view_rows()
        .iter()
        .position(|r| r.text.contains("pub fn far_target"))
        .unwrap_or(usize::MAX);
    assert!(
        msg_ok && (8..=14).contains(&def),
        "msg={:?} def-row={def} (want mid-window 8..=14; the last content row is 20)",
        s.message
    );
    let frame = render80(s);
    assert!(frame.contains("pub fn far_target"), "definition mid-window: {frame}");
}

// ═══════ external notes (drive_external_notes.py E1-E4) ═══════════════════

/// E1 + E1b: the M-. landing on an external (registry) source — read-only,
/// the window on the source — and the ownership guard: C-x C-q AND C-x C-s
/// are refused on the external buffer.
#[test]
fn unit_flow_ext_notes_landing_guard() {
    let (repo, ext) = ext_notes_setup();
    let rope = ext.path().join("src/rope.rs");
    let mut s = store_in(repo.path());
    install_index(&mut s, repo.path());
    open_via_finder(&mut s, "probe");
    goto_line(&mut s, 4); // top-level `ropey::Rope::new();` probe
    s.key_event(key("M-f")); // end of the `ropey` run
    s.key_event(key("M-.")); // fall-through to the resolver
    land_resolved(&mut s, &rope, ext.path(), 3);
    // (jump-ambiguity) the tooling hit joins the Xref picker (the
    // `tooling`-marked row, preselected); RET accepts the top guess.
    assert!(s.picker_open(), "the tooling hit joins the picker");
    s.key_event(key("RET"));
    let abs = rope.to_string_lossy().into_owned();
    let reported = s.message.contains(&format!("jumped to {abs}:3"));
    let external = s.buffers.current().map(String::from) == Some(abs.clone());
    let source_view = s.buffer_text().contains("Rope");
    assert!(
        reported && external && source_view,
        "reported={reported} external={external} msg={:?}",
        s.message
    );
    // E1b: the deliberate edit-mode override must never turn a registry
    // source (a cache shared by every project on the machine) editable or
    // writable.
    s.key_event(key("C-x"));
    s.key_event(key("C-q"));
    let ref1 = s.message.contains("external buffer is read-only (not project-owned)");
    s.key_event(key("C-x"));
    s.key_event(key("C-s"));
    let ref2 = s.message.contains("external buffer is read-only (not project-owned)");
    assert!(
        ref1 && ref2 && s.buffer_mode_display() == "Read-only",
        "C-x C-q={ref1} C-x C-s={ref2} (msg={:?})",
        s.message
    );
}

/// E2: `A` on the external landing line → RET commits — the margin marker
/// AND the inline note row render on the external buffer, and the record is
/// keyed by the ABSOLUTE path in the project's .redline-notes.md.
#[test]
fn unit_flow_ext_notes_annotate() {
    let (repo, ext) = ext_notes_setup();
    let rope = ext.path().join("src/rope.rs");
    let abs = rope.to_string_lossy().into_owned();
    let mut s = store_in(repo.path());
    install_index(&mut s, repo.path());
    open_via_finder(&mut s, "probe");
    goto_line(&mut s, 4);
    s.key_event(key("M-f"));
    s.key_event(key("M-."));
    land_resolved(&mut s, &rope, ext.path(), 3); // point on the landing line
    // (jump-ambiguity) the tooling hit joins the Xref picker (the
    // `tooling`-marked row, preselected); RET accepts the top guess.
    assert!(s.picker_open(), "the tooling hit joins the picker");
    s.key_event(key("RET"));
    s.key_event(key("A"));
    assert!(s.note_prompt_active(), "annotation prompt active");
    for c in "external note".chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("RET"));
    let saved = s.message.contains("note saved") && s.message.contains(&format!("in {abs}"));
    let rows = s.file_view_rows();
    let code_idx = rows
        .iter()
        .position(|r| r.text.contains("RopeBuilder::new"));
    let marker = code_idx.map(|code| rows[code].annotated).unwrap_or(false);
    // annotations-render-fold: the note row sits directly ABOVE the code row.
    let above = code_idx
        .and_then(|code| rows.get(code - 1))
        .filter(|r| r.text.contains("external note"))
        .is_some();
    let disk = std::fs::read_to_string(repo.path().join(".redline-notes.md"))
        .expect("notes file created")
        .contains(&format!("path: {abs}"));
    let frame = render80(s);
    assert!(
        saved && marker && above && disk,
        "saved={saved} marker={marker} above={above} disk={disk}\n{frame}"
    );
}

/// E3: `d` removes the external-buffer record — echo, the note row is gone
/// from the buffer, and the disk record is gone.
#[test]
fn unit_flow_ext_notes_delete() {
    let (repo, ext) = ext_notes_setup();
    let rope = ext.path().join("src/rope.rs");
    let mut s = store_in(repo.path());
    install_index(&mut s, repo.path());
    open_via_finder(&mut s, "probe");
    goto_line(&mut s, 4);
    s.key_event(key("M-f"));
    s.key_event(key("M-."));
    land_resolved(&mut s, &rope, ext.path(), 3);
    // (jump-ambiguity) the tooling hit joins the Xref picker (the
    // `tooling`-marked row, preselected); RET accepts the top guess.
    assert!(s.picker_open(), "the tooling hit joins the picker");
    s.key_event(key("RET"));
    s.key_event(key("A"));
    for c in "external note".chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("RET"));
    s.key_event(key("d"));
    let echo = s.message.contains("deleted annotation: external note");
    let gone = !s
        .file_view_rows()
        .iter()
        .any(|r| r.text.contains("external note"));
    let disk_gone = !std::fs::read_to_string(repo.path().join(".redline-notes.md"))
        .unwrap()
        .contains("external note");
    assert!(
        echo && gone && disk_gone,
        "echo={echo} gone={gone} disk-gone={disk_gone} (msg={:?})",
        s.message
    );
}

/// E4: the quit dump carries the record's path VERBATIM (the absolute
/// external path) with the anchored code line, the NOTE text, and the
/// project-root header unchanged.
#[test]
fn unit_flow_ext_notes_dump() {
    let (repo, ext) = ext_notes_setup();
    let rope = ext.path().join("src/rope.rs");
    let abs = rope.to_string_lossy().into_owned();
    let mut s = store_in(repo.path());
    install_index(&mut s, repo.path());
    open_via_finder(&mut s, "probe");
    goto_line(&mut s, 4);
    s.key_event(key("M-f"));
    s.key_event(key("M-."));
    land_resolved(&mut s, &rope, ext.path(), 3);
    // (jump-ambiguity) the tooling hit joins the Xref picker (the
    // `tooling`-marked row, preselected); RET accepts the top guess.
    assert!(s.picker_open(), "the tooling hit joins the picker");
    s.key_event(key("RET"));
    s.key_event(key("A"));
    for c in "external note".chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("RET"));
    let items = s.annotations_for_dump();
    assert_eq!(items.len(), 1, "exactly one dump record: {items:?}");
    let root = repo.path().display().to_string();
    let dump = format_notes_dump(&items, &root, false);
    let header = dump.starts_with(&format!("# redline annotations \u{2014} {root}\n\n"));
    let verbatim = dump.contains(&format!("{abs}:3\n"));
    // The dump's anchored code line: 4-space dump indent + the source line
    // (which carries its own indent).
    let block = dump.contains(&format!(
        "{abs}:3\n        RopeBuilder::new();\n  NOTE: external note\n"
    ));
    assert!(
        header && verbatim && block,
        "header={header} verbatim={verbatim} block={block}\ndump={dump:?}"
    );
}

// ═══════ external crate (drive_external_crate.py L1-L7) ═══════════════════

/// L1 + L2 + L3: the external landing registers the crate for background
/// indexing — the `indexing crate …` indicator is up while the build is in
/// flight, and the build's final event clears it.
#[tokio::test]
async fn unit_flow_ext_crate_landing_indicator() {
    let (repo, ext) = ext_notes_setup();
    let rope = ext.path().join("src/rope.rs");
    let mut s = store_in(repo.path());
    install_index(&mut s, repo.path());
    let mut rx = s.crate_index_bus.subscribe();
    open_via_finder(&mut s, "probe");
    goto_line(&mut s, 4);
    s.key_event(key("M-f"));
    s.key_event(key("M-."));
    land_resolved(&mut s, &rope, ext.path(), 3);
    // (jump-ambiguity) the tooling hit joins the Xref picker (the
    // `tooling`-marked row, preselected); RET accepts the top guess.
    assert!(s.picker_open(), "the tooling hit joins the picker");
    s.key_event(key("RET"));
    // L1: the landing report, the external read-only buffer, the window on
    // the source.
    let abs = rope.to_string_lossy().into_owned();
    let reported = s.message.contains(&format!("jumped to {abs}:3"));
    let source_view = s.buffer_text().contains("Rope");
    // L2: the in-flight `indexing crate …` indicator (the indicator only
    // clears when the final event is APPLIED — it has not been drained yet).
    let indicator = s.crate_indexing_display();
    let in_flight = indicator.starts_with("indexing crate ")
        && indicator.ends_with("/2)\u{2026}");
    assert!(
        reported && source_view && in_flight,
        "reported={reported} indicator={indicator:?}"
    );
    // L3: the build's final event lands → the indicator clears, the index
    // is installed (crate-relative keys).
    let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
        .await
        .expect("crate index event within 30s");
    let event = rx.borrow_and_update().clone();
    assert_eq!(event.source_root, ext.path().to_path_buf());
    s.apply_crate_index_event(&event);
    assert_eq!(s.crate_indexing_display(), "", "indicator clears on the final event");
}

/// L4 + L6 + L7: after the crate index lands, M-. on `RopeBuilder` jumps
/// cross-file INSIDE the crate (the report is crate-relative, not the
/// absolute registry path); M-, walks back through the in-crate origin to
/// the project file.
#[test]
fn unit_flow_ext_crate_in_crate_mdot() {
    let (repo, ext) = ext_notes_setup();
    let rope = ext.path().join("src/rope.rs");
    let mut s = store_in(repo.path());
    install_index(&mut s, repo.path());
    // The crate index (the L1 event's store-side state).
    let files: Vec<String> = vec!["src/rope.rs".into(), "src/rope_builder.rs".into()];
    let idx = crate::nav::index::build_index(ext.path(), &files, None);
    s.apply_crate_index_event(&CrateIndexEvent {
        source_root: ext.path().to_path_buf(),
        index: idx,
    });
    open_via_finder(&mut s, "probe");
    goto_line(&mut s, 4);
    s.key_event(key("M-f"));
    s.key_event(key("M-."));
    land_resolved(&mut s, &rope, ext.path(), 3); // lands on the RopeBuilder call line
    // (jump-ambiguity) the tooling hit joins the Xref picker (the
    // `tooling`-marked row, preselected); RET accepts the top guess.
    assert!(s.picker_open(), "the tooling hit joins the picker");
    s.key_event(key("RET"));
    // L4: M-. WITHIN the crate — cross-file, crate-relative report.
    // (jump-ambiguity) cross-file unique → the picker; RET accepts.
    goto_line(&mut s, 3); // "    RopeBuilder::new();"
    s.key_event(key("M-f")); // end of the `RopeBuilder` run
    s.key_event(key("M-."));
    assert!(s.picker_open(), "cross-file unique: picker");
    s.key_event(key("RET"));
    let rel_msg = s.message.contains("jumped to src/rope_builder.rs:1");
    let struct_visible = s.buffer_text().contains("pub struct RopeBuilder");
    let in_crate = s.buffers.current().map(String::from)
        == Some(ext.path().join("src/rope_builder.rs").to_string_lossy().into_owned());
    assert!(
        rel_msg && struct_visible && in_crate,
        "rel-msg={rel_msg} struct={struct_visible} in-crate={in_crate} (msg={:?})",
        s.message
    );
    // L6/L7: M-, walks the jump stack back.
    s.key_event(key("M-,"));
    let origin = s.buffers.current().map(String::from)
        == Some(rope.to_string_lossy().into_owned());
    s.key_event(key("M-,"));
    let project_back = s.buffers.current().map(String::from)
        .map(|k| k.ends_with("probe.rs"))
        .unwrap_or(false);
    assert!(origin && project_back, "origin={origin} project-back={project_back}");
}

/// L5: imenu on the external buffer lists the crate file's symbols from
/// the crate index (not a refusal).
#[test]
fn unit_flow_ext_crate_imenu() {
    let (repo, ext) = ext_notes_setup();
    let rope = ext.path().join("src/rope.rs");
    let mut s = store_in(repo.path());
    install_index(&mut s, repo.path());
    let files: Vec<String> = vec!["src/rope.rs".into(), "src/rope_builder.rs".into()];
    let idx = crate::nav::index::build_index(ext.path(), &files, None);
    s.apply_crate_index_event(&CrateIndexEvent {
        source_root: ext.path().to_path_buf(),
        index: idx,
    });
    open_via_finder(&mut s, "probe");
    goto_line(&mut s, 4);
    s.key_event(key("M-f"));
    s.key_event(key("M-."));
    land_resolved(&mut s, &rope, ext.path(), 3);
    // (jump-ambiguity) the tooling hit joins the Xref picker (the
    // `tooling`-marked row, preselected); RET accepts the top guess.
    assert!(s.picker_open(), "the tooling hit joins the picker");
    s.key_event(key("RET"));
    // L4's landing first (the PTY leg order): the current file becomes the
    // crate's rope_builder.rs, whose outline imenu must list.
    // (jump-ambiguity) cross-file unique → the picker; RET accepts.
    goto_line(&mut s, 3); // "    RopeBuilder::new();"
    s.key_event(key("M-f")); // end of the `RopeBuilder` run
    s.key_event(key("M-."));
    assert!(s.picker_open(), "cross-file unique: picker");
    s.key_event(key("RET"));
    let in_crate = s.message.contains("jumped to src/rope_builder.rs:1");
    s.key_event(key("M-i"));
    let open = s.picker_open() && s.picker_kind() == Some(PickerKind::Imenu);
    let lists = s
        .picker_filtered()
        .iter()
        .any(|(c, _)| c.name.contains("RopeBuilder"));
    let no_refusal = !s.message.contains("not in project");
    assert!(
        open && lists && no_refusal && in_crate,
        "open={open} lists-RopeBuilder={lists} refusal={no_refusal} in-crate={in_crate} (msg={:?})",
        s.message
    );
}

// ═══════ external use (drive_external_use.py L1-L2) ═══════════════════════

/// L1: a BARE use-imported symbol falls through with the bare name (the
/// scope hint is the store-side supply, 007-03; the serde lookup itself is
/// the resolver corpus's) — the landing lands read-only in the (external)
/// trait source and M-, returns to the project file.
#[test]
fn unit_flow_ext_use_l1() {
    let repo = ext_use_repo();
    let serde = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(serde.path().join("src")).unwrap();
    let trait_file = serde.path().join("src/trait.rs");
    std::fs::write(&trait_file, "pub trait Deserialize {}\n").unwrap();
    let mut s = store_in(repo.path());
    install_index(&mut s, repo.path());
    open_via_finder(&mut s, "main");
    goto_line(&mut s, 2); // "let d = Deserialize;"
    s.key_event(key("M-f")); // end of `let`
    s.key_event(key("M-f")); // end of `d`
    s.key_event(key("M-f")); // end of the `Deserialize` run
    s.key_event(key("M-."));
    let bare = s.message.contains("no provider resolution for `Deserialize`")
        && !s.message.contains("serde::Deserialize");
    assert!(bare, "fall-through carries the BARE symbol: {:?}", s.message);
    land_resolved(&mut s, &trait_file, serde.path(), 1);
    // (jump-ambiguity) the tooling hit joins the Xref picker (the
    // `tooling`-marked row, preselected); RET accepts the top guess.
    assert!(s.picker_open(), "the tooling hit joins the picker");
    s.key_event(key("RET"));
    let abs = trait_file.to_string_lossy().into_owned();
    let msg = s.message.contains(&format!("jumped to {abs}:1"));
    let trait_view = s.buffer_text().contains("trait Deserialize");
    s.key_event(key("M-,"));
    let back = s
        .buffers
        .current()
        .map(String::from)
        .map(|k| k.ends_with("main.rs"))
        .unwrap_or(false);
    assert!(
        msg && trait_view && back,
        "msg={:?} trait-view={trait_view} back={back}",
        s.message
    );
}

/// L2 degradation pin: a BARE symbol with NO `use` still misses with the
/// graceful "no provider resolution for `other`" report — no hint, no
/// guessing, and it never jumps.
#[test]
fn unit_flow_ext_use_l2() {
    let repo = ext_use_repo();
    let mut s = store_in(repo.path());
    install_index(&mut s, repo.path());
    open_via_finder(&mut s, "main");
    goto_line(&mut s, 3); // "let other = 9;"
    s.key_event(key("M-f")); // end of `let`
    s.key_event(key("M-f")); // end of `other`
    s.key_event(key("M-."));
    let named = s.message.contains("no provider resolution for `other`");
    miss_resolved(&mut s, "other", "no crate `other`");
    let graceful = s.message.starts_with("no provider resolution for `other`:")
        && s.message.contains("tried 1 provider(s): rust");
    assert!(
        named && graceful && !s.message.contains("jumped to"),
        "named={named} graceful={graceful} (msg={:?})",
        s.message
    );
}

// ═══════ syntax notes (drive_syntax_notes.py S0-S7) ═══════════════════════

/// S1-S3: A on the function name commits a note; the on-disk record carries
/// the syntax keys (`syntax_kind: identifier` / `syntax_name: target_one`)
/// plus the line-0 text anchor.
#[test]
fn unit_flow_synleg_anchor_commit() {
    let (repo, _leg) = synleg_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "synleg");
    // Point ON the function name: buffer start, then 3 char-forwards
    // (`f`, `n`, ` `, then the `t` of `target_one`).
    s.key_event(key("M-<"));
    for _ in 0..3 {
        s.key_event(key("C-f"));
    }
    s.key_event(key("A"));
    let prompt = s.note_prompt_active();
    for c in "follow fn".chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("RET"));
    let saved = s.message.contains("note saved");
    let disk = std::fs::read_to_string(repo.path().join(".redline-notes.md"))
        .expect("notes file created");
    let keys = disk.contains("syntax_kind: identifier")
        && disk.contains("syntax_name: target_one")
        && disk.contains("line: 0")
        && disk.contains("anchor: fn target_one() {");
    assert!(prompt && saved && keys, "prompt={prompt} saved={saved}\ndisk={disk}");
}

/// S4 through S7: an out-of-band rewrite (a 100-line insertion on top AND a
/// reformat of the signature — the ±25-line text path has nothing to match)
/// plus `g` force-reload runs the re-anchor pass. Asserts: the marker +
/// note row follow the function to its new line; no `(orphaned)` tag
/// renders; the record re-anchored to line 100, not orphaned.
#[test]
fn unit_flow_synleg_reanchor() {
    let (repo, leg) = synleg_repo();
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "synleg");
    s.key_event(key("M-<"));
    for _ in 0..3 {
        s.key_event(key("C-f"));
    }
    s.key_event(key("A"));
    for c in "follow fn".chars() {
        s.key_event(key_char(c));
    }
    s.key_event(key("RET"));
    let mut rewritten = String::new();
    for i in 0..100 {
        rewritten.push_str(&format!("// filler {i}\n"));
    }
    rewritten.push_str("fn target_one()\n{\n    let x = 1;\n    x\n}\n");
    std::fs::write(&leg, rewritten).unwrap();
    s.key_event(key("g")); // force reload → the re-anchor pass
    let reloaded = s.message.contains("reloaded");
    // Jump to the end so the function (now at line 100) is in view
    // (the PTY leg does the same before its S5 assert).
    // (jump-ambiguity: point-buffer-end moved M-> → M-END.)
    s.key_event(key("M-END"));
    let rows = s.file_view_rows();
    let fn_row = rows.iter().position(|r| r.text == "fn target_one()");
    let marker = fn_row.map(|i| rows[i].annotated).unwrap_or(false);
    // annotations-render-fold: the note row renders directly ABOVE the
    // re-anchored function header.
    let note_above = fn_row
        .and_then(|i| rows.get(i - 1))
        .filter(|r| r.text.contains("follow fn"))
        .is_some();
    let no_orphan_tag = !rows.iter().any(|r| r.text.contains("(orphaned)"));
    let disk = std::fs::read_to_string(repo.path().join(".redline-notes.md")).unwrap();
    let reanchored = disk.contains("line: 100")
        && disk.contains("orphaned: false")
        && disk.contains("syntax_name: target_one");
    let frame = render80(s);
    assert!(
        reloaded && marker && note_above && no_orphan_tag && reanchored,
        "reloaded={reloaded} marker={marker} note-above={note_above} \n         no-orphan-tag={no_orphan_tag} reanchored={reanchored}\ndisk={disk}\n{frame}"
    );
}

// ═══════ ux_sweep (keymap-coverage derivation) ════════════════════════════

/// The store-side twin of one ux_sweep leg: `setup` opens the view (a fresh
/// store, the PTY's fresh App per drive), then every key is pressed and
/// checked — no "unbound key" echo (EXCEPT the `known_unbound` parity keys,
/// which MUST echo — no keys remain in that set: the window-split keys were
/// the last three and are now bound on the buffer view, see
/// `unit_flow_window_splits_*`), no quit, and the live view stays coherent.
/// Returns
/// the per-key ok flags + the store (for the end-of-leg render check).
fn ux_sweep_leg(
    repo: &std::path::Path,
    setup: &[&str],
    keys: &[&str],
    known_unbound: &[&str],
) -> (Vec<bool>, AppStore) {
    let mut s = store_in(repo);
    for tok in setup {
        drive_token(&mut s, tok);
    }
    let mut oks = Vec::new();
    for tok in keys {
        let msg_before = s.message.clone();
        s.key_event(key(tok));
        // A NEW unbound echo (the minibuffer holds the last message — a
        // stale "unbound key: …" from a previous key is not a finding).
        let echo = s.message.contains("unbound key") && s.message != msg_before;
        let expected_echo = known_unbound.contains(tok);
        oks.push(echo == expected_echo && !s.quit && ux_view_alive(&s));
    }
    (oks, s)
}

/// Drive one token: key tokens through `parse_key`, anything else
/// ("lib.rs", "toggle-tree") as typed characters — the PTY's type path.
fn drive_token(s: &mut AppStore, tok: &str) {
    match parse_key(tok) {
        Ok(k) => s.key_event(k),
        Err(_) => {
            for c in tok.chars() {
                s.key_event(key_char(c));
            }
        }
    }
}

/// The state half of ux_sweep's anomaly scan: no unprompted changed-on-disk
/// marker, and the live view still holds content with a coherent cursor
/// (the "blank frame while a view is open" + "cursor parked off-screen"
/// twins; the raw-pixel / hardware-cursor halves stay in the thin PTY tier).
fn ux_view_alive(s: &AppStore) -> bool {
    if s.current_buffer_changed_on_disk() {
        return false;
    }
    match s.top_view() {
        ViewId::MagitStatus | ViewId::Log | ViewId::Blame => {
            let rows = match s.top_view() {
                ViewId::MagitStatus => s.magit_rows(),
                ViewId::Log => s.log_rows(),
                _ => s.blame_rows(),
            };
            !rows.is_empty() && rows.iter().any(|r| r.selected)
        }
        ViewId::CommitDiff => !s.commit_diff_rows().is_empty(),
        ViewId::BufferList => {
            // The boot table is EMPTY by design (06a: no scratch) — an empty
            // list is a coherent state; a non-empty one keeps its selection
            // in range.
            s.buffer_rows().is_empty() || s.buffer_list_selected() < s.buffer_rows().len()
        }
        ViewId::Search => !s.search_view_info().0.is_empty(),
        ViewId::Home => !s.home_body_rows().is_empty(),
        ViewId::Buffer => s
            .buffers
            .current_buffer()
            .map(|b| s.point_line() < b.line_count())
            .unwrap_or(true),
        _ => true,
    }
}

/// One ux_sweep leg: (name, setup tokens, key tokens, known-unbound keys).
type UxLeg = (
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
    &'static [&'static str],
);

/// ux_sweep: keymap coverage across every view the PTY sweep drives — every
/// key is bound where the sweep expects (no "unbound key" echo), nothing
/// quits the app, every view stays coherent, and the narrow-40 frame is
/// intact at the width-bounded static render. (The terminal-tier residue —
/// real input encoding, process death, hardware cursor, raw pixels — stays
/// in the thin PTY tier.)
#[test]
fn unit_flow_ux_keymap_coverage() {
    let repo = fixture_repo();
    let legs: [UxLeg; 11] = [
        (
            "buffer-view",
            &["C-x", "C-f", "lib.rs", "RET"],
            // (jump-ambiguity: point-buffer-end moved M-> -> M-END; M->
            // now forces the Xref candidate list — the C-g after it
            // closes the picker before the remaining buffer-view keys
            // sweep as before.) 015-03: `C-d` is freed from half-page
            // scroll (delete-char in accurate mode), so in this read-only
            // buffer view it now echoes unbound — moved to known_unbound.
            &["C-n", "C-p", "C-f", "C-b", "C-a", "C-e", "M-f", "M-b", "M-<",
              "M-END", "M->", "C-g", "C-v", "M-v", "C-u", "C-l",
              "j", "k"],
            &["C-d"],
        ),
        ("magit", &["C-x", "g"], &["n", "p", "n", "n", "TAB", "TAB", "s", "u", "g", "q"], &[]),
        ("log", &["C-x", "g", "l"], &["n", "p", "RET", "q"], &[]),
        ("blame", &["C-x", "g", "b"], &["n", "p", "q"], &[]),
        ("find-file", &["C-x", "C-f"], &["x", "y", "z", "C-g"], &[]),
        ("buffer-list", &["C-x", "C-b"], &["n", "p", "C-g"], &[]),
        ("transient-menu", &[], &["C-x", "C-g", "?", "C-g"], &[]),
        ("search", &["C-c", "p", "s", "s"], &["C-g", "C-g"], &[]),
        ("notes-edit", &["C-x", "n"], &["a", "b", "C-h", "C-g"], &[]),
        (
            "tree",
            &["C-x", "C-f", "lib.rs", "RET", "M-x", "toggle-tree", "RET"],
            &["C-n", "C-p", "RET", "C-g"],
            &[],
        ),
        (
            "window-splits",
            &["C-x", "C-f", "lib.rs", "RET"],
            // The terminal sends the digit PLAIN after the C-x prefix
            // (encode_key's literal-char path). C-x 2 / C-x 1 / C-x 0 are
            // bound on the buffer view (view-stack-degraded semantics —
            // `unit_flow_window_splits_*` below), so NO key in this leg
            // echoes; C-x o (open-scratch) is bound too.
            &["C-x", "2", "C-x", "o", "C-x", "o", "C-x", "1", "C-x", "0"],
            &[],
        ),
    ];
    for (name, setup, keys, known_unbound) in legs {
        let (oks, s) = ux_sweep_leg(repo.path(), setup, keys, known_unbound);
        assert!(
            oks.iter().all(|ok| *ok),
            "leg {name}: per-key ok={oks:?}\n{frame}",
            frame = render80(s)
        );
    }
    // The narrow-terminal stress leg: the 40-col frame stays intact.
    let (oks, s) = ux_sweep_leg(
        repo.path(),
        &["C-x", "C-f", "lib.rs", "RET"],
        &["C-n", "C-e", "M->", "C-l", "?", "C-g"],
        &[],
    );
    let frame40 = crate::ui::root::render_at_width(s, 40);
    let non_blank = frame40.lines().any(|l| !l.trim().is_empty());
    assert!(
        oks.iter().all(|ok| *ok) && non_blank,
        "narrow-40: per-key ok={oks:?} frame-non-blank={non_blank}\n{frame40}"
    );
}

// ═══════ window splits (C-x 2 / C-x 1 / C-x 0 — the 3 pre-existing
// ux_sweep findings, now bound with view-stack-degraded semantics) ═══════

/// Open lib.rs in the buffer view (the window-splits setup).
fn window_splits_buffer_store(repo: &std::path::Path) -> AppStore {
    let mut s = store_in(repo);
    for tok in ["C-x", "C-f", "lib.rs", "RET"] {
        drive_token(&mut s, tok);
    }
    assert_eq!(s.top_view(), ViewId::Buffer, "setup must land in the buffer view");
    s
}

/// C-x 0 (close current view) on the buffer view: bound (no unbound echo),
/// and a no-op on the last view — the buffer view is the only view and
/// close-view cannot close it (the home root never dies, 06a). Home has no
/// view-local bindings at all (06a), so C-x 0 there dead-ends to the
/// unbound-key echo. The stacked top-view leg (the binding really
/// dispatches `close-view`) uses a test-constructed `[Buffer, MagitStatus,
/// Buffer]` — production entry points never push Buffer over another view,
/// but the state is legal to the stack model.
#[test]
fn unit_flow_window_splits_c_x_0_close_current() {
    let repo = fixture_repo();
    let mut s = window_splits_buffer_store(repo.path());
    assert_eq!(s.view_stack, vec![ViewId::Buffer]);

    // Degradation: C-x 0 on the last view — no-op, no echo.
    s.key_event(key("C-x"));
    assert_eq!(s.pending_display(), "C-x", "the prefix must arm, not echo");
    s.key_event(key("0"));
    assert!(
        !s.message.contains("unbound key"),
        "C-x 0 must be bound on the buffer view: {}",
        s.message
    );
    assert_eq!(s.view_stack, vec![ViewId::Buffer], "last view must survive");
    let frame = render80(s);
    assert!(
        frame.contains("target_lib") && frame.contains("src/lib.rs"),
        "buffer view still renders:\n{frame}"
    );

    // Home (06a: no view-local bindings): C-x 0 echoes unbound.
    let mut h = store_in(repo.path());
    h.key_event(key("C-x"));
    h.key_event(key("0"));
    assert!(
        h.message.contains("unbound key: 0"),
        "home has no C-x 0 (06a): {}",
        h.message
    );

    // Stacked top: the binding dispatches close-view and pops the top.
    let mut s = window_splits_buffer_store(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("g")); // [Buffer, MagitStatus]
    s.push_view(ViewId::Buffer); // [Buffer, MagitStatus, Buffer]
    s.key_event(key("C-x"));
    s.key_event(key("0"));
    assert_eq!(s.top_view(), ViewId::MagitStatus);
    assert_eq!(
        s.view_stack,
        vec![ViewId::Buffer, ViewId::MagitStatus],
        "C-x 0 must pop exactly the top view"
    );
}

/// C-x 1 (only-this-window, view-stack degraded): closes every view except
/// the current buffer view — the stack collapses to `[Buffer]`. Degradation:
/// with a single view it is already "only this window", so a no-op (like
/// emacs's C-x 1 on a single window).
#[test]
fn unit_flow_window_splits_c_x_1_only_this_view() {
    let repo = fixture_repo();
    let mut s = window_splits_buffer_store(repo.path());

    // Degradation: one view — no-op, no echo, no message.
    s.key_event(key("C-x"));
    s.key_event(key("1"));
    assert!(!s.message.contains("unbound key"), "{}", s.message);
    assert!(s.message.is_empty(), "no-op leaves the minibuffer: {}", s.message);
    assert_eq!(s.view_stack, vec![ViewId::Buffer]);

    // Stacked: [Buffer, MagitStatus, Buffer] truncates to [Buffer] (the
    // test-constructed state, as in the C-x 0 leg).
    let mut s = window_splits_buffer_store(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("g"));
    s.push_view(ViewId::Buffer);
    s.key_event(key("C-x"));
    s.key_event(key("1"));
    assert_eq!(s.view_stack, vec![ViewId::Buffer], "C-x 1 keeps only the buffer view");
    assert_eq!(s.top_view(), ViewId::Buffer);
    let frame = render80(s);
    assert!(
        frame.contains("target_lib") && frame.contains("src/lib.rs"),
        "the current buffer must survive the truncation:\n{frame}"
    );
}

/// C-x 2 (split-window-vertically) on the single-pane model: the key is
/// bound (no unbound echo — the pre-existing finding cleared) but reports
/// instead of splitting: the minibuffer says so, and the buffer, point, and
/// view stack are all untouched. Degradation: in a non-file view (magit
/// status — and home, 06a) the key is NOT bound (the split keys live only
/// on the buffer view), so it dead-ends to the unbound-key echo with no
/// state change.
#[test]
fn unit_flow_window_splits_c_x_2_single_pane_report() {
    let repo = fixture_repo();
    let mut s = window_splits_buffer_store(repo.path());
    let before = s.point_line();
    s.key_event(key("C-x"));
    s.key_event(key("2"));
    assert!(!s.message.contains("unbound key"), "{}", s.message);
    assert!(
        s.message.contains("single pane"),
        "C-x 2 must report the single-pane model: {}",
        s.message
    );
    assert_eq!(s.view_stack, vec![ViewId::Buffer], "no view-state change");
    assert_eq!(s.point_line(), before, "no point change");
    // render80: the buffer still renders + the note is in the minibuffer
    // row (the report must be visible, not just state).
    let frame = render80(s);
    assert!(
        frame.contains("target_lib") && frame.contains("single pane"),
        "buffer still renders + the note is in the minibuffer row:\n{frame}"
    );

    // Non-file view degradation: magit status has no C-x 2 binding.
    let mut s = window_splits_buffer_store(repo.path());
    s.key_event(key("C-x"));
    s.key_event(key("g")); // [Buffer, MagitStatus]
    s.key_event(key("C-x"));
    s.key_event(key("2"));
    assert!(
        s.message.contains("unbound key: 2"),
        "C-x 2 is buffer-view-only: {}",
        s.message
    );
    assert_eq!(
        s.view_stack,
        vec![ViewId::Buffer, ViewId::MagitStatus],
        "no state change on the echo"
    );
    // render80: the magit view still renders after the dead end.
    let frame = render80(s);
    assert!(
        frame.lines().any(|l| !l.trim().is_empty()) && !frame.contains("single pane"),
        "magit view intact, no split report:\n{frame}"
    );

    // Home (06a): the split keys are not bound there either.
    let mut h = store_in(repo.path());
    h.key_event(key("C-x"));
    h.key_event(key("2"));
    assert!(h.message.contains("unbound key: 2"), "{}", h.message);
    assert_eq!(h.view_stack, vec![ViewId::Home]);
}

// ── watchlist-fixes lane: the U-K items' unit twins ───────────────────────

/// U-K item 1 (U-D3): M-. on a type/constant name — the extraction has no
/// case filter (006-02b); the twin drives the real key (alt-dot) and
/// asserts the cross-file type landing (state) + the definition renders
/// (render80).
#[test]
fn unit_flow_xref_uppercase_type_m_dot() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(p.join("Cargo.toml"), "[package]\n").unwrap();
    std::fs::write(
        p.join("src/main.rs"),
        "mod widget;\nfn use_it() {\n    let w = Widget { x: 1 };\n}\n",
    )
    .unwrap();
    std::fs::write(p.join("src/widget.rs"), "pub struct Widget { pub x: i32 }\n").unwrap();
    let mut s = store_in(p);
    install_index(&mut s, p);
    s.open_path("src/main.rs");
    // Point at line 2, inside `Widget` (line "    let w = Widget {…"):
    // C-n ×2 → line 2; C-f ×12 → the `W` column.
    for _ in 0..2 {
        s.key_event(key("C-n"));
    }
    for _ in 0..12 {
        s.key_event(key("C-f"));
    }
    s.key_event(key("M-."));
    // (jump-ambiguity) a cross-file UNIQUE candidate opens the picker
    // (best preselected); RET accepts the top guess.
    assert!(s.picker_open(), "cross-file unique: picker");
    s.key_event(key("RET"));
    assert_eq!(s.top_view(), ViewId::Buffer);
    assert_eq!(s.view_name_display(), "src/widget.rs", "the CamelCase type jumps cross-file");
    assert_eq!(s.point_line(), 0, "on the struct's definition line");
    let frame = render80(s);
    assert!(
        frame.contains("pub struct Widget"),
        "the type's definition renders: {frame}"
    );
}

/// U-K item 2 (imenu flat, no impl-parent nesting): M-i groups the Rust
/// impl methods under the impl's type — state on the name-first row
/// (label = the indented name, detail = the kind tag) + render80 on the
/// indented list row.
#[test]
fn unit_flow_imenu_impl_parent_grouping() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(p.join("Cargo.toml"), "[package]\n").unwrap();
    std::fs::write(
        p.join("src/lib.rs"),
        "pub struct Foo { a: i32 }\nimpl Foo {\n    pub fn new() -> Self { Self { a: 0 } }\n}\npub fn free() {}\n",
    )
    .unwrap();
    let mut s = store_in(p);
    install_index(&mut s, p);
    s.open_path("src/lib.rs");
    s.key_event(key("M-i"));
    assert!(s.picker_open(), "M-i opens the imenu picker");
    assert_eq!(s.picker_kind(), Some(crate::app::store::PickerKind::Imenu));
    // picker-density re-pin (was: exact `display` == "  new  [fn]"): the
    // row is now name-first — the label carries the (indented) name and
    // the detail carries the kind tag. Requirement-level intent kept:
    // imenu rows still show the kind tag, and the impl method stays
    // grouped/indented under its struct (the free fn does not).
    let rows: Vec<(String, String, String)> = s
        .picker_filtered()
        .iter()
        .map(|(c, _)| (c.name.clone(), c.label.clone(), c.detail.clone()))
        .collect();
    let new = rows.iter().find(|(n, _, _)| n == "new:3").unwrap();
    assert_eq!(new.1, "  new", "impl method grouped/indented under the struct: {rows:?}");
    assert_eq!(new.2, "[fn]", "imenu rows still show the kind tag: {rows:?}");
    let free = rows.iter().find(|(n, _, _)| n == "free:5").unwrap();
    assert_eq!(free.1, "free", "the free fn stays flat: {rows:?}");
    assert_eq!(free.2, "[fn]", "the kind tag on the flat row: {rows:?}");
    // render80 re-pin (was: contiguous "  new  [fn]"): the kind tag is now
    // a right-aligned detail column, so pin the row shape — the method
    // row carries its 2-space indent AND ends with its tag; the free fn
    // row renders flat.
    let frame = render80(s);
    // The preview pane shares the row, so pin the row shape in-row: the
    // method row carries its 2-space indent, its name left, and the kind
    // tag after the name on the same row.
    let new_row = frame
        .lines()
        .find(|l| l.trim_start().starts_with("new") && l.contains("[fn]"))
        .unwrap_or("");
    assert!(
        new_row.contains("  new"),
        "the grouped row renders indented with its kind tag: {frame}"
    );
    let free_row = frame
        .lines()
        .find(|l| l.trim_start().starts_with("free") && l.contains("[fn]"))
        .unwrap_or("");
    assert!(
        !free_row.contains("  free"),
        "the free fn renders flat (no indent): {frame}"
    );
}

/// picker-density follow-up: the remaining pickers render NAME-FIRST —
/// find-file and the buffer switcher rows carry the name left and the
/// path right-aligned on the SAME row (the scannable right column the
/// Xref/Symbols conversion established; the state-level label/detail are
/// pinned in the store tests, this pins the rendered row shape).
#[test]
fn unit_flow_picker_name_first_rows_find_file_and_buffers() {
    // find-file: the file name left, the path right, on the same row.
    {
        let repo = fixture_repo();
        let mut s = store_in(repo.path());
        open_via_finder(&mut s, "lib");
        s.key_event(key("C-x"));
        s.key_event(key("C-f"));
        assert!(s.picker_open(), "find-file opens");
        let c = s
            .picker_filtered()
            .iter()
            .find(|(c, _)| c.name == "src/lib.rs")
            .map(|(c, _)| c.clone())
            .unwrap();
        assert_eq!(c.label, "lib.rs", "the file name is the label: {c:?}");
        assert_eq!(c.detail, "src/lib.rs", "the path is the detail: {c:?}");
        let frame = render80(s);
        // The preview pane shares the row, so pin in-row: the label left,
        // the detail path after it on the same row.
        let row = frame
            .lines()
            .find(|l| l.trim_start().starts_with("lib.rs") && l.contains("src/lib.rs"))
            .unwrap_or("");
        assert!(!row.is_empty(), "the find-file row is name-first (name left, path right): {frame}");
    }
    // switch-buffer: the buffer name left, the absolute path right.
    {
        let repo = fixture_repo();
        let mut s = store_in(repo.path());
        open_via_finder(&mut s, "lib");
        s.key_event(key("C-x"));
        s.key_event(key("b"));
        assert!(s.picker_open(), "switch-buffer opens");
        let key = s.buffers.current().map(String::from).unwrap();
        let c = s
            .picker_filtered()
            .iter()
            .find(|(c, _)| c.name == key)
            .map(|(c, _)| c.clone())
            .unwrap();
        assert_eq!(c.label, "src/lib.rs", "the buffer name is the label: {c:?}");
        assert_eq!(c.detail, key, "the absolute path is the detail: {c:?}");
        let frame = render80(s);
        let row = frame
            .lines()
            .find(|l| l.trim_start().starts_with("src/lib.rs") && l.contains(&key))
            .unwrap_or("");
        assert!(!row.is_empty(), "the buffer row is name-first (name left, path right): {frame}");
    }
}

/// U-K items 3 + 4 (search_jump + M-, under Search): state + render80.
/// Leg A — a RET whose hit file cannot be opened (deleted after the walk)
/// keeps the results view open and reports: the jump did not happen.
/// Leg B — M-, under the results view pops through the sentinel in one
/// step to the pre-search position (the view closes with the landing).
#[test]
fn unit_flow_search_ret_and_mcomma_under_search() {
    // Leg A: the failed open.
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(p.join("Cargo.toml"), "[package]\n").unwrap();
    std::fs::write(
        p.join("src/main.rs"),
        "fn target() {}\nfn main() { target(); }\n",
    )
    .unwrap();
    std::fs::write(p.join("src/lib.rs"), "pub fn target() {}\n").unwrap();
    let mut s = store_in(p);
    s.open_path("src/main.rs");
    let main_key = s.buffers.current().unwrap().to_string();
    let mut rx = s.search_rx().unwrap();
    s.start_project_search("target".into());
    drain_search_finished(&mut s, &mut rx);
    assert!(s.search_view_info().2 > 0, "hits arrived (lib.rs is the first file)");
    std::fs::remove_file(p.join("src/lib.rs")).unwrap();
    s.key_event(key("RET"));
    assert_eq!(
        s.top_view(),
        ViewId::Search,
        "the failed open keeps the results view open"
    );
    assert_eq!(
        s.buffers.current().map(String::from),
        Some(main_key),
        "the current buffer is unchanged"
    );
    assert!(s.message.contains("cannot open"), "{}", s.message);
    assert!(s.message.contains("the jump did not happen"), "{}", s.message);
    let frame = render80(s);
    assert!(
        frame.contains("cannot open"),
        "the report renders on the results view: {frame}"
    );

    // Leg B: the one-step sentinel pop to the pre-search position.
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(p.join("Cargo.toml"), "[package]\n").unwrap();
    std::fs::write(
        p.join("src/main.rs"),
        "fn target() {}\nfn main() { target(); }\ntarget();\n",
    )
    .unwrap();
    std::fs::write(p.join("src/lib.rs"), "pub fn target() {}\n").unwrap();
    let mut s = store_in(p);
    s.open_path("src/main.rs");
    s.key_event(key("C-n")); // line 1
    s.key_event(key("C-n")); // line 2 — the pre-search position
    let mut rx = s.search_rx().unwrap();
    s.start_project_search("target".into());
    drain_search_finished(&mut s, &mut rx);
    s.key_event(key("RET"));
    assert_eq!(s.top_view(), ViewId::Buffer, "RET lands the hit's file");
    assert_eq!(s.view_name_display(), "src/lib.rs");
    s.key_event(key("M-,"));
    assert_eq!(s.top_view(), ViewId::Search, "the first M-, returns to the results");
    // The watchlist fix: M-, UNDER the results view is bound and pops
    // through the sentinel to the pre-search position in one step.
    s.key_event(key("M-,"));
    assert_eq!(
        s.top_view(),
        ViewId::Buffer,
        "the second M-, closes the results with the landing"
    );
    assert_eq!(s.view_name_display(), "src/main.rs", "the pre-search buffer");
    assert_eq!(s.point_line(), 2, "the pre-search line");
    let frame = render80(s);
    assert!(
        frame.contains("fn main"),
        "the pre-search file renders: {frame}"
    );
}
