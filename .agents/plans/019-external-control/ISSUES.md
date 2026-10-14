# Plan 019 — the feature's OWN issues (the 019 workstream)

These are **part of building plan 019** — bugs, gaps, and decisions the new
external-control feature itself introduced or exposed. They are fixed within
the 019 workstream (by the issue that owns them), **not** deferred to the
pre-existing backlog. Escalate to the user only when a fix needs a product
decision.

Stable IDs (`W*`) are kept from the original combined list so cross-references
stay valid. Genuine pre-existing warts live in `WARTS.md`; the pure
orchestration lesson is `P1` at the bottom.

| # | Issue | Where (evidence) | Disposition | Owner |
|---|-------|------------------|-------------|-------|
| W3 | picker confirm/accept is key-only — no named `select`/`confirm`; `PickerKind` covers 13 flavors, driven only through `key_event` (RET/arrows) | `src/app/store/picker.rs`, `keys.rs` modal chain | the CLI needs a first-class `select`/`confirm`; add real picker ops (not a key replay) | 019-03 |
| W4 | `state` observable gaps — no public accessor for the current buffer KEY (only `buffer_display`/`buffer_text`), the search-prompt activation state (`search_prompt` is private), the notes-doc dirty flag | `src/app/store/{buffers,search,notes}.rs` | **RECON 2026-10-02: no current consumer** — 019-02/03 did not need them; **leave**, add an accessor only when a real caller appears | n/a |
| W7 | the static frame render was `#[cfg(test)]`-gated though it is the only faithful way to read a frame as text | `src/ui/root/render.rs::render_at_width` | **DONE** — lifted to production as `render_handle_text` | 019-01b |
| W8 | NO registry command consumes its `arg` (120/120 ignore it) — `do find-file <path>` opens the *picker* and drops the path | `grep -c 'store, _arg|' src/app/command.rs` = 120 | the sugar verbs must call REAL store ops (`open_path_at`), not revive registry args | 019-03 |
| W9 | `text` is mode-dependent (a read-only buffer refuses; an annotation-mode buffer appends at point) — the 01a test asserts only equality | `src/session.rs` tests | **DONE** for the test's purpose; the CLI docs must state the mode-dependence | 019-03 (docs) |
| W10 | `-c 'state --json'` parses `rest=" --json"` and the `state` handler silently ignores it, printing TEXT | `src/main.rs` `perform` | reject an unknown trailing token on a no-arg verb (exit 2), or accept it — do NOT ignore it | 019-03 |
| W11 | a second positional is silently dropped (`redline state state` runs `state`, discards the extra) | `src/main.rs` `sub_args` | error on extra positionals for subcommands that take none (exit 2) | 019-03 |
| W12 | `perform(Quit)` calls `std::process::exit(0)` deep inside the match — bypasses normal return, untestable in-process | `src/main.rs` | return a quit signal up through `run_headless`/`run_attached` | 019-03 |
| W14 | the control drive asserts a client's `quote` OUTPUT, not the human's SCREEN — a broken repaint would still pass | `tools/drive_control.py` | add a leg that reads the live PYTE SCREEN after a remote mutation and asserts the pane changed (kept the quote leg too) | 019-03 |
| W15 | `--attached` is documented but not implemented (no flag in `parse_cli`; the semantics are close by accident) | `src/main.rs` | add the flag (or drop it from docs — do not document a flag that does not exist) | 019-03 |
| W16 | the bound socket file is left on disk after a clean exit | observed: `srwxrwxr-x` entry survives the process | remove the socket file on exit (RAII guard or explicit `remove_file`) | 019-03 |
| W17 | remote `quit` prints a ~25-line state block (the "nothing printed → state" fallback fires) | `src/main.rs` `run_attached` | make `quit` a non-printing terminal verb | 019-03 |
| W18 | `handle_connection` reads a line with no size cap | `src/control.rs` | **leave** (local, same-uid) — note only; a cap is cheap if the surface widens | 019-03 (note) |
| W19 | `quote` rendered at a fixed 80; the live TUI renders at the terminal's real size | `src/control.rs` hardcoded `80u16, 24u16`; `TerminalEvent::Resize` discarded the width | **DONE** `3fdf73f` — `AppStore::terminal_size` recorded at startup + on resize; the socket reads it, so an attached `quote` matches the human's width. Test proven to discriminate; gate OK (full) | n/a |

## Process note (orchestration)

### P1 — a worker that reads forever and never writes
The first 019-01 subagent (`a2bc1661`) made **65 tool calls: 65 reads/greps,
0 writes/edits** over its whole 30-minute budget, producing no code and no
commit. It was stuck on *exploration*, not on a hard problem: the brief said
"read first" and listed 8 files, and the child kept widening. The fix that
worked: split the issue into 01a/01b, put the **complete code skeleton and
the exact accessor list in the spec**, fence the reads to five named files,
and state the anti-pattern. A second failure mode appeared in 019-02: a
`#[tokio::test]` single-thread deadlock (blocking std client starving the
spawned server task) hung `cargo test` for 4+ minutes — found by reading
`/proc/<pid>` thread states (all futex) and the test log's last line, fixed
with `flavor = "multi_thread"`. **Process rules for the rest of plan 019:**
worker briefs carry the code, not just the contract; every `cargo test` run
is bounded (a hang reports, never waits).

### W20 — a positional sugar verb swallows a following positional printer
`redline --project . open Cargo.toml:5 state` passes `"Cargo.toml:5 state"`
as the `open` argument (`sub_args.join(" ")`), so the open fails
(`cannot open Cargo.toml:5 state`) and no state prints. The `-c` form is
unaffected (`-c 'open Cargo.toml:5' state` works — that is what the drive
uses). The spec's own first smoke line used the positional form, so this is a
real usability gap: for an arg-taking verb, the NEXT positional is the
argument and any FURTHER token that names a known subcommand should be the
printer. **FIXED** (019-03 follow-up): a single-token-arg verb
(`open`/`goto`/`jump`/`search`/`isearch`/`select`) now takes exactly one
argument and REJECTS a further positional with a usage error (exit 2) rather
than swallowing it (`open Cargo.toml:5 state` -> "`open` takes one argument").
Chain commands with `-c` (`-c 'open f:5' state`), and a lone non-printing
verb already emits `state`. `text`/`annotate` stay greedy (free text).
Verified: the rejected form exits 2, `open Cargo.toml:5` alone lands + prints
state, and the `-c` chain is unaffected.
