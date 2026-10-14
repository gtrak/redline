# Task spec — issue-019-01: Session core + headless CLI

## Context header

- **Plan:** `.agents/plans/019-external-control/PLAN.md` (read §1–§3).
- **Issue (the contract):** `.agents/plans/019-external-control/01-session-core-and-headless-cli.md`.
- **Landed / in-flight:** this is the first issue of plan 019. Issues 02–03
  are specced later and must not be built now; leave the seam (a
  socket-path arg parsed but unused).
- **Repo:** `/home/gary/dev/red`, branch `main`, clean tree at HEAD
  `fe7c387`. Do not create a worktree; work in place.

## Working agreement

- **Skills are truth.** `.agents/skills/*.md` are authoritative (iocraft,
  nucleo, tree-sitter, tokio, tracing, support-crates, ropey). No
  `~/.cargo/registry` reads, no docs.rs, no fetching. A missing API detail
  gets a best-guess call consistent with the skill plus a noted gap.
- **Write-first.** Scaffold `src/session.rs` and the `main.rs` changes in the
  first handful of tool calls, compile early, iterate on named errors.
- **Skill corrections:** if code reality contradicts a skill file, code wins;
  make the minimal factual correction and list it under "skill corrections".
- **`graft` is on PATH** (`graft map/ask/callers/skeleton/grep`) with a fresh
  graph — use it for cross-file orientation, never as a replacement for the
  read-first list.
- **No new dependencies.** Do not add clap/serde additions beyond what is
  already in `Cargo.toml` (`serde`, `serde_json` are present).
- **Commit at the first green build**, then keep working (amend or add
  commits). The deliverable is the COMMIT. `cargo build`/`clippy`/`test`
  must be run with output redirected to a file, then read.
- **Never pipe a battery/probe to `tail`** (it returns tail's status).

## Read first (in order)

1. `.agents/plans/019-external-control/PLAN.md` and the issue file above.
2. `src/main.rs` — the existing arg loop (`CliArgs`, `parse_cli`,
   `parse_args`, `main`), the `--index-profile` headless precedent, and the
   dump path.
3. `src/app/command.rs` — `CommandRegistry` / `Command::run` /
   `dispatch_by_name` and the category/name surface.
4. `src/app/store/keys.rs` — `pub fn dispatch`, `pub fn key_event`, and the
   modal priority chain (this is what `key`/`text` ride).
5. `src/app/keymap.rs` — `parse_key`, `parse_sequence`, `Key` (token syntax:
   `C-x`, `M-x`, `RET`, `TAB`, `SPC`, `ESC`, a single char).
6. `src/ui/root/render.rs` — `render_at_width` (the static frame helper,
   currently `#[cfg(test)]`) and `src/ui/root/mod.rs` — `Root`, the
   `ContextProvider<Arc<Mutex<AppStore>>>` wiring, `StaticRenderWidth`.
7. `src/app/store/mod.rs` — `AppStore::at(start, base)`, `new`, and the
   accessors used by `snapshot()` (list below).
8. `src/app/store/minibuffer.rs` — `minibuffer_message`; `message` is `pub`.

## Constraints / guardrails

- **TUI bytes unchanged** with no subcommand: `parse_cli` must return the
  same `CliArgs` for a bare invocation and for `--notes=plain`, and `main`'s
  existing path must be reached identically. Existing `cli_tests` must pass
  unmodified.
- **Headless must not touch real user state.** Build the store with
  `AppStore::at(project, base)` where `base` is a throwaway dir under
  `std::env::temp_dir()` (never `dirs::cache_dir()`), so recents/registry/
  notes persistence is isolated.
- **Headless must not require a tty.** Do the headless run BEFORE the
  `init_dump_sink()` / `IsTerminal` / fullscreen setup, exactly like
  `--index-profile`.
- **No store lock across an await.** The headless path is synchronous; do not
  hold a `MutexGuard` across `.await`.
- **`quote` must match a real frame.** Set the store's viewport to
  `height - 3` before rendering (the live contract) and pin the width via the
  production-callable static render. Do not change what `render_at_width`
  renders — only its visibility.
- **Scope fence:** no socket code, no sugar verbs, no docs/README/`redline.1`
  changes in this issue. Leave `--socket`/`REDLINE_SOCKET` parsed but
  unused (a named TODO pointing at 019-02).

## What to build

### `src/session.rs`

```rust
pub struct Session {
    store: Arc<Mutex<AppStore>>,
    root: PathBuf,
    base: tempfile-free PathBuf,   // created under std::env::temp_dir()
    width: u16,                    // default 80
    height: u16,                   // default 24
}

impl Session {
    pub fn at(project: &Path, width: u16, height: u16) -> Self;
    pub fn store(&self) -> Arc<Mutex<AppStore>>;   // 019-02 uses this
    pub fn do_command(&mut self, name: &str, arg: Option<String>) -> Result<(), SessionError>;
    pub fn key(&mut self, token: &str) -> Result<(), SessionError>;
    pub fn keys(&mut self, seq: &str) -> Result<(), SessionError>;   // parse_sequence
    pub fn text(&mut self, s: &str) -> Result<(), SessionError>;      // per char; '\n'->Enter
    pub fn snapshot(&mut self) -> Snapshot;                          // owned, serde::Serialize
    pub fn quote(&mut self) -> String;                               // static frame text
    pub fn buffer_text(&mut self, range: Option<(usize, usize)>) -> String;
    pub fn notes(&mut self, plain: bool) -> String;                  // annotations_for_dump + format_notes_dump
    pub fn quit_requested(&self) -> bool;                            // store.quit
}
```

`SessionError` is a small enum (`UnknownCommand(String)`,
`BadKey(String)`) implementing `Display`.

### Snapshot (frozen JSON schema for v1)

Serialize with `serde` derive; snake_case fields; omit nothing that is
listed (use `null`/empty rather than skipping, for a stable schema):

```jsonc
{
  "project": "redline",
  "root": "/home/gary/dev/red",
  "view": "Buffer",                 // render_view() name
  "view_name": "src/main.rs",
  "buffer": "src/main.rs",
  "mode": "Read-only",              // buffer_mode_display()
  "message": "",                    // store.message
  "pending": "", "activity": "",    // pending_display()/activity_display()
  "point": { "line": 0, "col": 0 }, // file_view_point()
  "window": { "top_line": 0, "total_lines": 1, "viewport": 21 },   // file_view_scroll_info()
  "position": "Top",                // file_view_position_display()
  "which_function": "",
  "annotations": "",                // annotation_count_display()
  "dirty": { "staged": 0, "unstaged": 0, "untracked": 0 },  // dirty_counts() or null
  "picker": null,                   // or { prompt, query, selected, total, rows:[{name,label,detail}] }
  "search": null,                   // or { title, running, error, narrow_query, total, selected,
                                    //         rows:[{line?, ...}] }  // use search_view_info()
  "magit": null,                    // or { rows_total, narrow_query }
  "tree": null,                     // or { visible, query, rows_total, selected }
  "isearch": null,                  // or { active, selected, total }
  "buffer_list": null,              // or { query, total, selected }
  "indexing": "", "resolving": "", "crate_indexing": ""  // *_display()
}
```

> The exact row shapes are the store's existing row structs
> (`PickerCandidate`, `ResultRow`, `BufferRow`, `TreeRow`, `MagitRow`,
> `IsearchMatchRow`). Derive `Serialize` on the smallest wrapper you need, or
> map to plain serde structs in `session.rs` — do **not** add serde derives to
> store-internal types if a local mapping is cleaner. Keep it in `session.rs`.

### Text rendering of a snapshot

A stable, grep-friendly, human/agent-readable block (one line per field,
lists indented). JSON is the machine form; text is the default. Both come
from the same `Snapshot` value.

### CLI (in `src/main.rs`)

- A `Command`-like enum or a small match on `argv[1]`: `state`, `quote`,
  `buffer`, `notes`, `do`, `key`, `keys`, `text`, `quit`. Bare argv or a
  leading `--`-flag keeps the TUI.
- Flags: `--project DIR` (default `.`), `--json`, `--width N` (default 80),
  `--height N` (default 24), `-c CMD[:ARG]` (repeatable; `CMD` may be a
  subcommand name or `do <name>`, run in order before the final action),
  `--socket PATH` (parsed, unused), plus the existing `--notes=plain`.
- `redline do <name> [arg]` and `redline -c 'do <name> [arg]'` both work.
- Exit code 0 on success; 2 on usage error (unknown subcommand / bad args),
  matching the existing convention.
- Print the socket path to stderr at TUI start is **out of scope** (019-02).

## Verification (all must be run, output to a file, then read)

1. `cargo build --workspace > /tmp/b.log 2>&1; echo BUILD_EXIT=$?`
2. `cargo clippy --workspace --all-targets -- -D warnings > /tmp/c.log 2>&1; echo CLIPPY_EXIT=$?`
3. `cargo test --bin redline > /tmp/t.log 2>&1; echo TEST_EXIT=$?` (report the pass/fail counts)
4. Manual smoke (stdout redirected, no tty):
   `target/debug/redline --project /home/gary/dev/red -c 'open Cargo.toml' -c 'goto 5' state > /tmp/smoke.out 2>/tmp/smoke.err; echo EXIT=$?`
   then `head -30 /tmp/smoke.out`. Confirm `view`/`buffer` name `Cargo.toml`
   and the point line is 4 (0-based for goto-line 5).
5. `target/debug/redline --project /home/gary/dev/red -c 'open Cargo.toml' -c 'goto 5' state --json > /tmp/smoke.json; python3 -c 'import json,sys;print(json.load(open("/tmp/smoke.json"))["point"])'`
6. New unit tests: CLI parse forms (+ unknown subcommand = exit-2 error),
   `Session` dispatch→snapshot round-trip, `state --json` round-trips,
   `quote` contains an opened file line, `text` inserts in an editable buffer.

## Report format

Return, in this order:
1. **Files changed** (path + one-line purpose + line delta).
2. **Design summary** — how `Session` sits over `AppStore`; the snapshot
   mapping; how the CLI mode is selected.
3. **Gate outputs** — the four commands above with their redirections and
   exit codes and counts.
4. **New tests** — names + what each would do on the unfixed/absent code.
5. **Skill corrections** (if any).
6. **Deviations** from this spec (with reasons).
7. **Known gaps / accessors you could not include** (the follow-up list).
