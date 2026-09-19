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

use super::store::{AppStore, ViewId};
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
