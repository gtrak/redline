# Warts / tech debt found while building plan 019

Running list. Greenfield: existing details are fair game to change case by
case. Resolve one by one **after** the session CLI ships. Each entry: what,
where (evidence), why it matters, disposition (fix / change / leave).

## From recon (2026-10-02)

### W1 — `command.rs` is a 1,039-line function-of-functions with a stale doc comment
`src/app/command.rs` says the store owns the registry and dispatch extracts a
clone; the actual `seed()` is a huge chain of `reg.register_*()` calls. The
file header claims a category system but categories are ad-hoc string literals.
Matters: the external CLI will surface these names as an API — the doc/name
surface should be trustworthy. Disposition: **change** (small doc/consistency
pass when 019-03 freezes the command-name surface).

### W2 — `dispatch_by_name` is `#[allow(dead_code)]`
`src/app/command.rs:95` — unit-tested but unused in production because
`AppStore::dispatch` clones out of the store's own registry. The external
session wants standalone dispatch. Disposition: **change** — the Session can
use whatever is cleanest; revisit whether `dispatch_by_name` should become the
real entry point or be deleted.

### W3 — picker confirm/accept is key-only, no named command
There is no `picker-confirm`/`picker-accept` registry command: the picker is
driven through `AppStore::key_event` (RET/arrows) only, and
`PickerKind` covers 13 flavors. The external CLI wants a first-class
`select`/`confirm` that does not depend on key parity. Disposition: **change**
(019-03 sugar verbs may add named picker commands, or the Session exposes
picker ops directly — decide then).

### W4 — `state` observable gaps
There is no public accessor for: the current buffer key (only
`buffer_display`/`buffer_text`), the goto-line/search prompt activation state
(`goto_line_active` IS pub but `search_prompt` is private), and the notes-doc
dirty flag. The snapshot spec tells the worker to omit + list these rather than
add speculative accessors. Disposition: **fix** if a real consumer (019-02/03)
needs them.

### W5 — `quit` is a public bool field, mutated in four places
`AppStore.quit` (`src/app/store/mod.rs:1925`) is `pub` and set `true` in three
different `keys.rs` sites. The Session's `quit` verb should set it once through
a method. Disposition: **change** (small: add `AppStore::request_quit`).

### W6 — headless store construction is duplicated
`AppStore::at(start, base)` exists, and `--index-profile` builds its own
headless path without a store; `main.rs`'s TUI path builds `AppStore::new()`
+ config + watcher + indexing inline. The Session needs a third construction
site. Disposition: **fix** — one `Session` builder is the dedup; do not create
a fourth path.

### W7 — static frame render is `#[cfg(test)]`-gated
`src/ui/root/render.rs::render_at_width` is test-only, though it is the only
faithful way to read a frame as text. The agent-facing `quote` needs it in
production. Disposition: **fix** (019-01 lifts it to production-callable).

## Process notes (orchestration)

### P1 — a worker that reads forever and never writes
The first 019-01 subagent (`a2bc1661`, 30-min timeout) made **65 tool calls:
65 reads/greps, 0 writes/edits**, and produced no code and no commit. It was
not stuck on a hard problem — it was stuck on *exploration*: the brief said
"read first" and listed 8 files, and the child kept widening (44 turns,
grep-ing 13 picker kinds, reading search.rs/magit.rs twice). The fix that
worked: split the issue into 01a/01b, put the **complete code skeleton and the
exact accessor list in the spec**, fence the reads to five named files, and
state the anti-pattern ("a previous attempt read for 30 min and wrote
nothing; write src/session.rs immediately after the reads"). Disposition:
**process rule** for the rest of plan 019 — worker briefs carry the code, not
just the contract.

### W8 — NO registry command consumes its `arg` (120 of 120 ignore it)
`grep -c 'store, _arg|' src/app/command.rs` = **120**; there are zero
`|store, arg|` handlers. `Command::run(store, arg)` ships an
`Option<String>` that nothing reads, so `do_command("find-file", Some(path))`
silently opens the *picker* and ignores the path. The 01a worker worked
around it by typing into the picker (`text("src/main.rs\n")`), which is fine
for a test but is exactly the "sugar verb must be real" problem for 019-03:
`open <path>` cannot be `do find-file <path>`. Disposition: **change** — the
arg-aware open belongs in 019-03 as a real store op (`open_path` already
exists and takes `rel`), NOT a registry-arg revival. Also the dead
`Option<String>` on every handler is a wart in itself (W2-adjacent).

### W9 — 01a `quote()` and the read-only-text test encode assumptions
`text_refused_in_read_only_buffer` passes, but note the buffer's *mode* is
what makes it read-only, and `text()` routes per-char through `key_event`
(so a buffer in Annotation mode appends at point). The test's fixture checks
equality only; it does not assert *why* nothing changed. Accepted for 01a
(it is the intended contract), flagged so 019-03's `text` docs state the
mode-dependence rather than implying text is always refused.

### W10 — `-c 'state --json'` silently ignores `--json`
`--json` is a top-level flag (`redline --json -c 'state'` works); but the very
natural `redline -c 'state --json'` parses `rest = " --json"` and the `state`
handler ignores it, printing TEXT. A silently-ignored flag is the worst kind
of CLI wart. Disposition: **fix** in 019-03 (either accept trailing flags in
`-c` rest, or error "unknown argument to state: --json" rather than ignore).

### W11 — a second positional is silently ignored
`redline state state` runs `state` and drops the extra token (it becomes
`sub_args`). Should be a usage error (exit 2). Disposition: **fix** in 019-03
(cheap: reject `sub_args` for subcommands that take none).

### W12 — `quit` in `-c` calls `std::process::exit(0)` mid-`perform`
`perform(Subcommand::Quit)` calls `session.do_command("quit")` then
`std::process::exit(0)` — it bypasses the normal return. It works, but it
skips any cleanup the `Session` Drop might do and is untestable in-process.
Disposition: **fix** in 019-03 (return a "quit" signal up through
`run_headless` instead of exiting deep inside `perform`).

### W13 — the file-view title (buffer name) is duplicated: top row AND status line
`src/ui/file_view.rs:726` renders `props.title` (the buffer's display name) as
the pane's FIRST row; the status line at the bottom already shows it
(`src/ui/root/widgets.rs:79`: `* {project} *  {view}  ...`). Live frame proof:
```
Cargo.toml              <- file-view title, top row
[workspace]
members = ["crates/*"]
... (bottom) ...
* red *  Cargo.toml  Read-only  Top   <- status line, same name
```
**User directive (2026-10-02): "I don't want the file name at the top of the
window, it's at the bottom anyway."** Disposition: **change** — drop the
file view's title row (reclaim the row; it also shifts every file-view row
index by one, so `cursor_cell`, `mouse_click_position`/`click_target_line`,
the note-row offsets, and the static-render test that asserts `code_idx >= 3`
("title + line 0 code + 2 notes") all move with it). **This is a real
interaction with plan 019**: `quote` prints the frame, and every PTY drive
that maps terminal rows to buffer lines has the `row = terminal_row - 1`
title offset baked in (`src/ui/root/hooks.rs` mouse handlers, `drive_*.py`).
Sequence it AFTER 019-02 lands (so the control-socket drives are green first)
and treat it as its own issue: it is a layout/geometry change, not a
control-plane change. **Also check `Blame`/`Log`/`Search` views** — the same
"title row + status line" duplication may exist there; the directive is
stated for the file view, apply it consistently where it holds.
