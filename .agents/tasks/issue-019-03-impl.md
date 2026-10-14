# Task spec — issue-019-03: sugar verbs, docs, frozen surface, control-plane fixes

This is the LAST chunk of plan 019 (the session CLI). It makes the CLI
pleasant and correct, and freezes the API. It also fixes the feature's own
correctness issues the 019-02 review found (W10–W12, W14–W17, W19), listed
in `.agents/plans/019-external-control/ISSUES.md`. Those are part of the 019
workstream — fix them here, not in the pre-existing wart backlog. The
pre-existing/layout warts (`WARTS.md`, incl. W13) are deliberately OUT of
scope; they land in the post-CLI sweep.

## Do this first (mandatory — do NOT explore)

Read ONLY these, then WRITE CODE:
1. `src/main.rs` — the WHOLE file (you extend the parser + `perform` +
   `run_attached`/`run_headless`; the `-c` grammar helpers `split_cmd_line`,
   `split_do`, `parse_range`, `is_printing`).
2. `src/session.rs` — the `Session` API.
3. `src/control.rs` — `apply_request`, `dispatch`, `handle_connection`,
   `apply_request`'s per-command mapping.
4. `src/app/store/project.rs` lines 1–60 (`open_path`, `open_project_path`).
5. `src/app/store/file_view.rs` — `set_point_line` / `set_point` signatures.
6. `.agents/plans/019-external-control/ISSUES.md` (W10–W12, W14–W17, W19 — the plan-019 feature's OWN issues; fix them here)
   and `PLAN.md` § 2.3.
7. `tools/drive_control.py` — you extend it (W14).
8. `README.md` (the cheat-sheet table) and `redline.1` — you add sections.

## Part A — the missing store ops (make the sugar REAL)

**W8 (carried from 01a):** no registry command reads its `arg`. Do NOT revive
registry args. Add real methods to `AppStore`:

```rust
// src/app/store/project.rs (or file_view.rs for goto)
impl AppStore {
    /// Open a project-relative path (the `open` verb). `line`/`col` are
    /// 1-based when given; lands the point on them.
    pub fn open_path_at(&mut self, rel: &str, line: Option<usize>, col: Option<usize>) {
        self.open_path(rel);                 // existing: opens + pushes the view
        if let Some(l) = line { self.set_point_line(l.saturating_sub(1)); } // set_point_line is pub(super) — if so, add a small pub wrapper here
        // col: use the existing set_point(line, col, col) if reachable; else
        // unlock a `pub fn goto_line_col(&mut self, line0: usize, col0: usize)`.
    }
}
```

Check the actual visibility of `set_point`/`set_point_line` (they may be
`pub(super)`); if so, put the new methods in the SAME module (`file_view.rs`)
or add a `pub` wrapper in `mod.rs`. Keep it minimal.

- `goto <line>` → `Session::do`/direct call to land line (reuse
  `goto_line_start` + digits + RET through the key path is ALSO acceptable
  and even preferred — it exercises the real UI. Your call; document it).

## Part B — sugar verbs in the CLI

Add to the subcommand set (both local headless and over the wire):

| verb | semantics |
|---|---|
| `open PATH[:LINE[:COL]]` | `open_path_at`; `PATH` project-relative; `:LINE`/`:COL` 1-based |
| `goto LINE` | land the point on LINE (1-based) |
| `jump SYMBOL` | open the symbol picker filtered to SYMBOL, OR resolve it via `xref_find_definitions`; document which |
| `search QUERY` | start a project search (the real store op, not a keystroke replay) |
| `isearch QUERY` | start an in-buffer isearch |
| `select N` | picker: move the selection to row N (or `next`/`prev`) |
| `confirm` | picker: RET-equivalent accept |
| `cancel` | the global cancel (C-g equivalent) |
| `annotate TEXT` | annotate at point with TEXT |
| `magit` | open magit status |

Each must be expressible in ONE socket round trip (attached) — add a
`cmd` for each in `control::apply_request`, mapping to a `Session` method.
Prefer a `Session::verb(name, args)` dispatcher so local and wire share code.

**Keep the primitive verbs working unchanged.**

## Part C — W14: make the drive assert the SCREEN, not `quote`

The 019-02 drive's kill-shot is `"Cargo.toml" in <client quote output>` — a
client read of store state. **Add a leg that reads the live PYTE SCREEN**:
after the remote `do find-file`+`text`+`RET`, call `app._read(2.0, quiet=0.3)`
and assert the screen's **title row and status line** both name the file (the
human's pane actually moved). This is the assertion that would FAIL if the
repaint broke. Do NOT delete the existing quote leg (keep both: they prove
different things).

## Part D — the control-plane warts

- **W15 `--attached`:** add the flag to `parse_cli`; with it, a missing
  socket is a hard error (exit 1) even for an env-only socket. Without it,
  keep today's behaviour (explicit `--socket` missing = error; env-only
  missing = headless fallback). Add a parse test.
- **W16 socket cleanup:** remove the socket file on clean exit. Either a
  small RAII guard in `main` (after the TUI loop + watcher teardown) or a
  `remove_file` in the shutdown path. Verify: after remote `quit`, the
  socket file is gone.
- **W17 `quit` prints state:** make `quit` a non-printing terminal verb in
  BOTH `run_headless` and `run_attached` (print nothing, or one line
  `quit requested`). No state dump.
- **W10 `-c 'state --json'`:** the trailing `--json` is silently ignored.
  FIX: reject unknown trailing tokens on a verb that takes none
  (`state`/`quote`/`cancel`/`magit`/`confirm`) with a usage error (exit 2),
  OR accept `--json` as a trailing flag. Prefer REJECT (a silently ignored
  flag is the wart); document the rule.
- **W11 extra positional:** `redline state state` silently drops the second.
  FIX: a subcommand that takes no positional args errors on extra
  positionals (exit 2).
- **W19 docs:** `quote` renders at the session width (default 80); state in
  `docs/control-api.md` that it is an 80-col approximation unless `--width`
  matches the human's terminal.

## Part E — docs + the frozen surface

- `docs/control-api.md` — **new**: the JSON snapshot schema (every field +
  nested object), the verb table (primitives + sugar), the socket
  discovery/env/flag, and the dotted examples.
- `README.md` — a "Driving redline from outside" section (attached +
  headless examples) and a cheat-sheet row.
- `redline.1` — the verbs, `--socket`, `--attached`, `REDLINE_SOCKET`.
- **Schema pin test:** assert the top-level `Snapshot` JSON keys and each
  nested object's keys, so an accidental rename reddens. (A golden key-set
  list in `src/session.rs` tests.)

## Part F — extend the drive

Add one leg per sugar verb to `tools/drive_control.py` (registered already
in `gate.sh` SHARED_SUITES and `pool.py` BATTERY — confirm still registered):
`open <file>:<line>`, `goto`, `search`, `select`+`confirm` on a picker,
`annotate`, `magit`. Each asserted via a following `quote` **and**, for at
least the `open` leg, the live screen (W14).

## Verification

```
cargo build --workspace > /tmp/b3.log 2>&1; echo BUILD_EXIT=$?
cargo clippy --workspace --all-targets -- -D warnings > /tmp/c3.log 2>&1; echo CLIPPY_EXIT=$?
cargo test --bin redline > /tmp/t3.log 2>&1; echo TEST_EXIT=$?
python3 tools/drive_control.py > /tmp/d3.log 2>&1; echo DRIVE_EXIT=$?; tail -20 /tmp/d3.log
```
Plus a smoke proving a sugar verb: `redline --project <repo> -c 'open Cargo.toml:5' state` (the positional form `open X:5 state` is rejected on purpose — see W20)
(or the attached form). Report the exact command + observed point line.

## Report format
Files changed; the sugar-verb design (Session::verb? per-cmd?); each wart's
one-line disposition and how you verified it; gate outputs + counts; drive
result; new tests; deviations; anything not done.
