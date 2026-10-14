# Task spec — issue-019-01b: headless CLI subcommand mode + quote()

## Do this first (mandatory — do NOT explore)

READ ONLY these files, in this order, then WRITE CODE. A previous attempt on
this task read for 30 minutes and wrote nothing; 01a succeeded only because
the spec carried the code. Follow the same discipline: after these reads,
write the code, compile, and fix only the errors the compiler names.

1. `src/main.rs` — the WHOLE file (you are editing `CliArgs`, `parse_cli`,
   `parse_args`, `usage`, and `main`; adding `mod session;` is already done).
2. `src/session.rs` — already written by 01a. You call
   `Session::{at, do_command, key, keys, text, snapshot, buffer_text, notes,
   quit_requested}`; `Snapshot::to_text()` exists. `quote()` is a stub.
3. `src/ui/root/render.rs` lines 290–309 (`render_at_width`, currently
   `#[cfg(test)]`) — you will lift it to production-callable.
4. `src/ui/root/mod.rs` lines 1–40 and 95–110 (the `Root` element wiring and
   the `StaticRenderWidth` context).

## Part A — lift the static frame render to production (ownership already solved)

In `src/ui/root/render.rs`, `render_at_width(store: AppStore, width: usize)`
is `#[cfg(test)]` and consumes an owned `AppStore`. `AppStore` is **NOT
`Clone`**, so the `Session` cannot hand its store over. The fix is already
decided for you: add a production sibling that takes the **`Arc<Mutex<AppStore>>`**
the session already owns — no ownership transfer, no `mem::replace`:

```rust
/// Render the current frame to plain text at a pinned width. The external
/// CLI's `quote` reads the frame a human would see. Takes the SAME
/// `Arc<Mutex<AppStore>>` handle the render loop uses, so the caller keeps
/// its store (`AppStore` is not `Clone`).
pub fn render_handle_text(store: Arc<Mutex<AppStore>>, width: usize) -> String {
    let mut app = element! {
        ContextProvider(value: Context::owned(StaticRenderWidth(width as u16))) {
            ContextProvider(value: Context::owned(store)) {
                Root
            }
        }
    };
    let canvas = app.render(Some(width));
    canvas.get_text(0, 0, width, canvas.height())
}
```

- Make `StaticRenderWidth` production-visible (drop its `#[cfg(test)]`), and
  add whatever production `use`s are needed (`Arc`, `Mutex`, `AppStore`,
  `Root`) — they are currently test-gated. The production build must compile.
- Keep the existing `#[cfg(test)] render_at_width` (it may stay; it takes an
  owned store for the tests). Do **not** change what either renders.
- `AppStore::at`/`Session` already hold the `Arc` — expose a
  `pub(crate) fn render_handle_text` and call it from `session.rs`.

## Part B — `Session::quote()`

Implement `Session::quote(&mut self) -> String` as:

```rust
pub fn quote(&mut self) -> String {
    crate::ui::root::render_handle_text(Arc::clone(&self.store), self.width as usize)
}
```

`ui::root` must re-export `render_handle_text` (add it to the `pub use`
line already there for `render_at_width`). No lock is held across the call
(the render takes the lock itself). Document the one invariant: the frame is
rendered at the session width (`self.width`), which defaults to 80.

## Part C — CLI subcommand mode (in `src/main.rs`)

Extend the existing hand-rolled parser. Requirements:

- **Bare invocation is unchanged.** `redline`, `redline --notes=plain` must
  parse to the exact same `CliArgs` and take the exact same path as today
  (existing `cli_tests` must pass unmodified).
- New: detect a subcommand (the first non-flag arg) from:
  `state`, `quote`, `buffer`, `notes`, `do`, `key`, `keys`, `text`, `quit`.
- Flags: `--project DIR` (default `.`), `--json`, `--width N` (default 80),
  `--height N` (default 24), `-c CMD[:ARG]` (repeatable), `--socket PATH`
  (parsed, unused, TODO(019-02)), plus the existing `--notes=plain` and
  `--index-profile*`.
- `redline do <name> [arg]` and `redline -c 'do <name> [arg]'` both work.
- **`-c` grammar (simple): each `-c` value is one command LINE** — split on
  the first run of whitespace into `verb` + `rest`. `verb` is one of the
  subcommand names; `rest` is the argument (`do` → command name + optional
  arg; `key`/`keys`/`text` → the token/sequence/literal; `state`/`quote`
  ignore `rest`). Examples that MUST work:
  `-c 'do find-file' -c 'text Cargo.toml' -c 'key RET' -c 'state'` and
  `-c 'do goto-line' -c 'text 5' -c 'key RET'`.
  (No `:` splitting, no quoting inside `rest` beyond what the shell strips.)
- `-c` entries run **in order**. The final action is the last `-c` line whose
  verb prints, or the trailing positional subcommand; if none prints, default
  to `state`. Concretely: run every `-c` line in order except that a
  non-final `state`/`quote` is skipped; the LAST printing verb is emitted.
  Simplest correct rule: run all mutation `-c` lines, then run the positional
  subcommand if given, else the last `-c` line if it is `state`/`quote`, else
  `state`. Document the rule you implement.
- Headless path runs **before** any tty/terminal setup (the `--index-profile`
  precedent — put it next to that branch in `main`), prints `state` (text by
  default; JSON with `--json`) or the subcommand's output to stdout, exits 0.
- Unknown subcommand → usage on stderr, exit 2 (the existing convention).
- `quit` sets the quit request and exits 0 (no TUI).

Dispatch table (map subcommand → Session call):

| subcommand | action |
|---|---|
| `state` | `println!("{}", snap.to_text())` or JSON |
| `quote` | `println!("{}", s.quote())` |
| `buffer` | `print!("{}", s.buffer_text(range_from_args))` (optional `START..END`) |
| `notes` | `print!("{}", s.notes(plain))` |
| `do` | `s.do_command(name, arg)` |
| `key` | `s.key(token)` |
| `keys` | `s.keys(seq)` |
| `text` | `s.text(literal)` |
| `quit` | set quit, exit 0 |

## Part D — tests (in `src/main.rs` `cli_tests`, and/or `src/session.rs`)

1. `bare_invocation_unchanged` — `parse_cli([])` and `parse_cli(["--notes=plain"])`
   behave exactly as the existing pins assert (do NOT modify the existing
   tests; add a new one asserting a subcommand parses).
2. `subcommand_parses` — `parse_cli(["state"])`, `["--json","state"]`,
   `["--project",".","-c","open:src/main.rs","state"]` parse to the expected
   struct/fields.
3. `unknown_subcommand_is_usage_error` — `parse_cli(["bogus"])` is `Err`.
4. `quote_renders_opened_file` — `Session` at a temp project, open the file
   (via the picker: `find-file` + `text(path+"\n")`), then `quote()` contains
   a known line from that file.
5. `headless_state_json` — serialize a real session snapshot to JSON and
   assert the `view`/`point` keys (if not already covered in 01a, this is the
   end-to-end glue).

## Verification

```
cargo build --workspace > /tmp/b1b.log 2>&1; echo BUILD_EXIT=$?
cargo clippy --workspace --all-targets -- -D warnings > /tmp/c1b.log 2>&1; echo CLIPPY_EXIT=$?
cargo test --bin redline > /tmp/t1b.log 2>&1; echo TEST_EXIT=$?
target/debug/redline --project /home/gary/dev/red -c 'open Cargo.toml' -c 'goto 5' state > /tmp/smoke.out 2>/tmp/smoke.err; echo SMOKE_EXIT=$?
head -30 /tmp/smoke.out
target/debug/redline --project /home/gary/dev/red -c 'open Cargo.toml' -c 'goto 5' state --json > /tmp/smoke.json; python3 -c 'import json;print(json.load(open("/tmp/smoke.json"))["point"])'
```
The smoke must prove a REAL landing through the primitive verbs (no registry
command reads its `arg` yet — see W8; `open`/`goto` sugar is 019-03). Use:

```
target/debug/redline --project /home/gary/dev/red \
  -c 'do find-file' -c 'text Cargo.toml' -c 'key RET' \
  -c 'do goto-line' -c 'text 5' -c 'key RET' state > /tmp/smoke.out 2>/tmp/smoke.err; echo SMOKE_EXIT=$?
head -30 /tmp/smoke.out
```

Assert the output's `buffer` names `Cargo.toml` and `point.line` is 4
(0-based for goto-line 5). Report the exact sequence you used.

## Report format
Files changed; the `quote` ownership decision (Part B); gate outputs with
exit codes + counts; new test names + what each would do on absent code;
deviations; anything not done.
