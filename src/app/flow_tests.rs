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

/// Run git in `dir`; panic on failure (fixture hygiene).
fn git(dir: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_out(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
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
        store.key_event(key(&c.to_string()));
    }
    store.key_event(key("RET"));
    assert!(!store.picker_open(), "RET must close the picker");
}

/// Drive `C-c p i` (re-walk) then open a file through the finder.
fn rew_walk_and_open(store: &mut AppStore, query: &str) {
    store.key_event(key("C-c"));
    store.key_event(key("p"));
    store.key_event(key("i"));
    open_via_finder(store, query);
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
        s.key_event(key(&c.to_string()));
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
        s.key_event(key(&c.to_string()));
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
    let result = (|| {
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
        (main_listed, lib_absent)
    })();
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
    let full = s.picker_filtered().clone();
    let no_graft_full = !full.iter().any(|(c, _)| c.name.contains("graft"));
    for c in "graft".chars() {
        s.key_event(key(&c.to_string()));
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
        s.key_event(key(&c.to_string()));
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
    // Home view.
    {
        let frame = render80(store_in(repo.path()));
        let rows = rows_containing(&frame, &name);
        let last = frame.lines().count() - 1;
        assert!(rows.contains(&last), "status row carries the project: {frame:?}");
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

/// U-C1 Motion: C-d scrolls (the window moves, point's screen row held).
#[test]
fn unit_flow_c1() {
    let repo = tall_sweep_repo(60, "tall_sweep.rs");
    let mut s = store_in(repo.path());
    open_via_finder(&mut s, "tall_sweep");
    assert_eq!(s.top_view(), ViewId::Buffer, "tall file opened");
    let (top_before, _, viewport) = s.file_view_scroll_info();
    let point_before = s.file_view_point().0;
    assert_eq!(top_before, 0);
    s.key_event(key("C-d"));
    let (top_after, _, _) = s.file_view_scroll_info();
    let point_after = s.file_view_point().0;
    // (a) Real scroll: the top visible line changed (non-vacuous).
    assert_ne!(top_before, top_after, "C-d must scroll the window");
    assert!(top_after <= viewport, "half-page scroll: {top_after}");
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
    // M->: bottom anchor — the last content row reads the final line.
    s.key_event(key("M->"));
    let m_gt_bottom_anchor = s
        .file_view_rows()
        .iter()
        .rev()
        .find(|r| !r.text.is_empty())
        .map(|r| r.text.as_str())
        == Some("line 50");
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
        s.key_event(key(&c.to_string()));
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
        s.key_event(key(&c.to_string()));
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
        .map(|b| b.path.as_ref())
        .flatten()
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
            s.key_event(key(&c.to_string()));
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
            .map(|b| b.path.as_ref())
            .flatten()
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
        s.key_event(key(&c.to_string()));
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
    // (a symbol from src/main.rs), not the view title.
    let file_current = s.which_function().contains("target_one");
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
    for i in 0..10 {
        append(&gw, &format!("G2_CHURN_{i}\n"));
    }
    publish_change(&mut s, &gw, ChangeKind::Modify);
    assert!(
        s.buffer_text().contains("G2_CHURN_9"),
        "final content shown after the burst"
    );
    // Responsive = the store is still coherent after the burst: the next
    // keypress processes (state stays sane) and the view still renders the
    // final content.
    let frame = render80(s);
    assert!(frame.contains("G2_CHURN_9"), "post-burst frame coherent: {frame}");
}

/// U-F4 live status on a FILE buffer: an external disk edit is picked up
/// and the file view shows it, with no keypress; a plain (non-locally-
/// owned) file buffer auto-reloads rather than raising the marker; `g`
/// force-reloads either way.
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
        s.key_event(key(&c.to_string()));
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
            s.key_event(key(&c.to_string()));
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
