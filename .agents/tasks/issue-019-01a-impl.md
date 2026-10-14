# Task spec — issue-019-01a: Session core (module + tests only)

## Do this first (mandatory)

Your FIRST tool calls must be reads of EXACTLY these files, in this order,
then you write code. Do not grep the repo further; do not read any other
file unless a compile error names it.

1. `src/main.rs` (the `mod` list + `CliArgs` — you will add `mod session;`)
2. `src/app/store/mod.rs` lines 1890–1960 (the `AppStore` struct head)
3. `src/app/store/keys.rs` lines 195–210 + 984–1010 (`key_event`, `dispatch`)
4. `src/app/keymap.rs` lines 200–360 (`parse_key`, `parse_sequence`, `Key`)
5. `src/app/command.rs` lines 1–110 (`CommandRegistry`, `Command`,
   `dispatch_by_name`, `RegistryError`)

Then **write `src/session.rs`** with the complete code below. Compile.
Iterate ONLY on named errors.

## The exact observable accessors (do not hunt for these)

All exist already; call them from `snapshot()`:

- `store.project_display() -> &str`
- `store.render_view() -> ViewId`, `store.view_name_display() -> String`
- `store.buffer_mode_display() -> String`
- `store.message` (pub field, `String`)
- `store.pending_display() -> String`, `store.activity_display() -> String`
- `store.file_view_point() -> (usize, usize)`   // (line, col)
- `store.file_view_scroll_info() -> (usize, usize, usize)` // (top, total, viewport)
- `store.file_view_position_display() -> String`
- `store.which_function() -> String`
- `store.annotation_count_display() -> String`
- `store.dirty_counts() -> Option<DirtyCounts>`  // DirtyCounts fields: staged/unstaged/untracked (usize) — CHECK the struct in mod.rs while reading step 2, adjust field names to reality
- `store.picker_open() -> bool`; when true: `picker_prompt() -> &str`,
  `picker_query() -> &str`, `picker_selected() -> usize`,
  `picker_count() -> (usize, usize)` (takes `&mut self`), and
  `picker_filtered() -> &[(PickerCandidate, u32)]`
  (`PickerCandidate { name, display, label, detail, docs, category, .. }`)
- `store.search_view_info() -> (Vec<ResultRow>, usize, usize, Option<usize>)`
  (takes `&mut self`) — (rows, top, total, selected); plus
  `search_title() -> String`, `search_running() -> bool`,
  `search_error() -> Option<String>`, `search_narrow_query() -> &str`
- `store.magit_view_info()` (`&mut self`) → `(rows, top, total)`;
  `magit_narrow_query() -> &str`
- `store.tree_visible() -> bool`, `tree_narrow_query() -> &str`,
  `tree_view_info()` (`&mut self`) → `(rows, selected)`
- `store.isearch_list() -> (Vec<IsearchMatchRow>, usize, bool)`
- `store.buffer_list_view_info()` (`&mut self`) → `(rows, scroll, total, selected)`
- `store.buffer_list_query() -> &str`
- `store.indexing_display() -> String`, `resolving_display() -> String`,
  `crate_indexing_display() -> String`
- `store.buffer_text() -> String`
- `store.annotations_for_dump()` (`&mut self`) → `Vec<DumpAnnotation>`;
  `crate::app::store::format_notes_dump(&items, &root, plain) -> String`
- `store.quit` (pub bool)

## What to build — `src/session.rs` (complete)

```rust
//! Scriptable session façade over `AppStore` (plan 019 issue 01a).
//!
//! Everything the external CLI/socket can do is expressed here against one
//! `AppStore`: dispatch a named registry command, feed keys/text through the
//! SAME modal routing the terminal uses, and read the observable state. No
//! new mutation paths are invented.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::app::keymap::{parse_key, parse_sequence};
use crate::app::store::{AppStore, RegistryError};

#[derive(Debug)]
pub enum SessionError {
    UnknownCommand(String),
    BadKey(String),
}

impl std::fmt::Display for SessionError { /* ... */ }
impl std::error::Error for SessionError {}

pub struct Session {
    store: Arc<Mutex<AppStore>>,
    root: PathBuf,
    width: u16,
    height: u16,
}

impl Session {
    /// Build a headless session rooted at `project`. The persistence base is
    /// a throwaway dir under the system temp dir so a headless run NEVER
    /// touches the real cache dir (recents/registry/notes).
    pub fn at(project: &Path, width: u16, height: u16) -> Self {
        let base = std::env::temp_dir().join(format!(
            "redline-session-{}-{}",
            std::process::id(),
            // a monotonic-ish suffix so two sessions differ
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let mut store = AppStore::at(project, base);
        store.set_viewport_lines(height.saturating_sub(3) as usize);
        Self { store: Arc::new(Mutex::new(store)), root: project.to_path_buf(), width, height }
    }

    pub fn store(&self) -> Arc<Mutex<AppStore>> { Arc::clone(&self.store) }
    pub fn root(&self) -> &Path { &self.root }

    pub fn do_command(&mut self, name: &str, arg: Option<String>) -> Result<(), SessionError> {
        let mut s = self.store.lock().unwrap();
        s.dispatch(name, arg)
            .map_err(|RegistryError::UnknownCommand(n)| SessionError::UnknownCommand(n))
    }

    pub fn key(&mut self, token: &str) -> Result<(), SessionError> {
        let k = parse_key(token).map_err(|e| SessionError::BadKey(e.to_string()))?;
        self.store.lock().unwrap().key_event(k);
        Ok(())
    }

    pub fn keys(&mut self, seq: &str) -> Result<(), SessionError> {
        let ks = parse_sequence(seq).map_err(|e| SessionError::BadKey(e.to_string()))?;
        let mut s = self.store.lock().unwrap();
        for k in ks.iter() { s.key_event(*k); }   // CHECK KeySeq iteration shape while reading keymap.rs
        Ok(())
    }

    /// Literal text into the current insert context: each char is one key
    /// event, `\n` is Enter. Read-only buffers refuse exactly as a human.
    pub fn text(&mut self, s: &str) -> Result<(), SessionError> {
        let mut st = self.store.lock().unwrap();
        for ch in s.chars() {
            let k = if ch == '\n' { crate::app::keymap::Key::enter() }
                    else { crate::app::keymap::Key::char(ch) };
            st.key_event(k);
        }
        Ok(())
    }

    pub fn snapshot(&mut self) -> Snapshot { /* build per the schema in 019-01 */ }
    pub fn quote(&mut self) -> String { /* ADD LATER — this is 01b; for 01a return String::new() with a TODO, OR lift render_at_width now if cheap */ }
    pub fn buffer_text(&mut self, range: Option<(usize, usize)>) -> String { /* 1-based inclusive line range over buffer_text() */ }
    pub fn notes(&mut self, plain: bool) -> String { /* annotations_for_dump + format_notes_dump */ }
    pub fn quit_requested(&self) -> bool { self.store.lock().unwrap().quit }
}
```

`Snapshot` is a `#[derive(Serialize)]` struct with the exact schema in
`.agents/plans/019-external-control/01-session-core-and-headless-cli.md`
§ "Snapshot". Map store row types to plain local serde structs (do NOT add
serde derives to store types). Nested options are `Option<...>`; absent
surfaces serialize as `null`.

Also provide `impl Snapshot { pub fn to_text(&self) -> String }` — a stable,
grep-friendly block (one `key: value` line per scalar; lists indented).

## Tests (in `src/session.rs`, `#[cfg(test)] mod tests`)

Use `tempfile::tempdir()` (already a dev-dep) as the project root; create
`Cargo.toml` + `src/main.rs` + `README.md` so project detection works.

1. `dispatch_open_then_snapshot_shows_buffer` — `do_command("find-file",
   Some("src/main.rs"))` → `snapshot().buffer` names the file, `view` ==
   "Buffer".
2. `goto_line_moves_point` — open, then `text("10\n")`? No — use the goto-line
   flow: `do_command("goto-line", None)` then `text("10\n")`, then assert
   `snapshot().point.line == 9`. (If `goto-line` is not a registry command
   name, find the real one in `command.rs` and use it; disclose the name.)
3. `key_unknown_token_is_bad_key` — `key("NOTAKEY")` is `Err(BadKey)`.
4. `unknown_command_is_error` — `do_command("no-such-cmd", None)` is `Err`.
5. `snapshot_json_round_trips` — `serde_json::to_string(&snap)` then
   `serde_json::from_str::<serde_json::Value>` succeeds and has the `view`
   and `point` keys.
6. `text_refused_in_read_only_buffer` — after opening a file (Read-only
   mode), `text("x")` leaves `buffer_text` unchanged.

## Verification

```
cargo build --workspace > /tmp/b1a.log 2>&1; echo BUILD_EXIT=$?
cargo clippy --workspace --all-targets -- -D warnings > /tmp/c1a.log 2>&1; echo CLIPPY_EXIT=$?
cargo test --bin redline session:: > /tmp/t1a.log 2>&1; echo TEST_EXIT=$?
```
Then read the logs. Report pass/fail counts. **Commit when green.**

## Out of scope (01b): `quote()`, the CLI subcommand mode, `render_at_width`
lifting. Leave `quote()` a stub that returns an empty string with a TODO
comment naming 01b.

## Report format

Files changed; the snapshot mapping (which store accessor fed which field);
gate outputs with exit codes and counts; new test names + what each would do
on absent code; deviations; anything you could not do.
