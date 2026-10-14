# Issue 019-01 — Session core + headless CLI

**Objective:** give redline a scriptable, non-TUI command surface: a
`Session` façade over `AppStore` for command/key/text dispatch and state
readout, and a CLI mode that runs it headlessly and prints state.

**Why now:** it is the foundation for the attached control socket (019-02)
and the sugar verbs (019-03). Everything the socket will expose is defined
here.

## Key decisions

- **One dispatch path, two frontends.** `Session` is a thin façade over
  `AppStore`'s existing `pub fn`s. Commands go through `AppStore::dispatch`
  (the registry); keys go through `AppStore::key_event` (the modal routing
  the terminal uses). No new mutation paths are invented.
- **No new dependencies.** A hand-rolled subcommand parser in `main.rs`
  (the existing arg loop is already hand-rolled); no clap.
- **`quote` = the static render the tests already use.** The frame is
  rendered with the same `render_at_width` path (pin the width, set
  `viewport_lines = height - 3`), so the text the agent reads matches a real
  80x24 frame. This requires lifting that helper out of `#[cfg(test)]`
  (production-callable), without changing what it renders.
- **The TUI path is untouched.** With no subcommand the binary behaves
  exactly as today. Subcommands and `-c` are the only new entry points.
- **`state` reads only existing accessors.** Where an observable already has
  a `pub fn` accessor it is included; where it does not, the field is omitted
  and listed in the report as a follow-up (do not add speculative accessors).

## Files

| File | Change |
|------|--------|
| `src/session.rs` | **new** — `Session`, command dispatch, key/text feeding, `snapshot()` + text/JSON serialization, `quit`. |
| `src/main.rs` | extend `CliArgs`/`parse_cli` with the subcommand mode; add the headless run path; keep `--notes`/`--index-profile` working. |
| `src/ui/root/render.rs` | make the static frame render production-callable (currently `#[cfg(test)]`); no rendering change. `src/ui/root/mod.rs` re-exports it. |
| `src/app/store/…` | only if a truly required observable has no accessor — add the smallest `pub fn`; **disclose each in the report**. |

## Steps

1. `Session::{new, at}` — build an `AppStore` rooted at a project dir with a
   persistence base (so headless runs never touch the real cache dir), hold it
   as `Arc<Mutex<AppStore>>`, expose `root`, `width`, `height`.
2. `Session::do_command(name, arg)` → `AppStore::dispatch`; map
   `RegistryError` to a structured error.
3. `Session::key(token)` → `keymap::parse_key` → `AppStore::key_event`;
   `Session::keys(seq)` → `parse_sequence` → feed each; `Session::text(s)` →
   feed each char through `key_event` (`'\n'` → Enter).
4. `Session::snapshot()` → the frozen JSON schema below (serde).
5. Text rendering of the snapshot (stable, grep-friendly) and JSON rendering.
6. `Session::quote(width)` → the static frame text.
7. `Session::buffer_text(range)` and `Session::notes(plain)`.
8. `main.rs`: subcommand mode (`state`, `quote`, `buffer`, `notes`, `do`,
   `key`, `keys`, `text`, `quit`), `-c CMD[:ARG]` (repeatable, run in order),
   `--project DIR`, `--json`, `--width N`, `--socket` (parsed, used only by
   019-02). A bare invocation is the TUI.

## Verification

- `cargo build`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --bin redline` all green (TUI regression net).
- New tests in `src/session.rs` / `src/main.rs`: parse the new CLI forms
  (including rejection of unknown subcommands); `Session` dispatch →
  `snapshot` round-trip (open a file, goto a line, assert point/scroll/view);
  `state --json` parses back as JSON; `quote` contains an opened file's line;
  `text` inserts in an editable buffer and the following `state` shows it.
- One manual smoke run: `redline --project <this repo> -c 'open Cargo.toml' -c 'goto 5' state`
  prints state and exits 0 with stdout not a tty.
