# Pre-existing warts & tech debt (NOT part of plan 019)

Greenfield project: existing details are fair game to change case by case.
These are issues that **predate or sit adjacent to** the external-control
feature — they are not the feature's own bugs (those live in `ISSUES.md`).
Resolve one by one **after** the session CLI ships (plan 019 complete).

Each entry: what, where (evidence), why it matters, disposition
(fix / change / leave).

## Pre-existing warts found while building plan 019

### W1 — `command.rs` is a 1,039-line `seed()` chain with a stale doc comment
`src/app/command.rs` says the store owns the registry and dispatch extracts a
clone; the actual `seed()` is a long chain of `reg.register_*()` calls. The
header implies a category system, but categories are ad-hoc string literals.
Matters: the external CLI now surfaces these names as an API, so the doc/name
surface should be trustworthy. **Recon (2026-10-02): FALSE ALARM.** The registry uses 120 `Command::new`
entries with only **11 distinct categories** (git 28, motion 26, navigation
16, buffers 11, search 11, editing 9, region 6, files 5, annotations 5,
watching 2, project 1) — a real, consistent system, and the header doc is
accurate. Disposition: **leave** (no action).

### W2 — `dispatch_by_name` is dead — **DONE `6156748`**
`src/app/command.rs:95` — unit-tested but unused in production (in-app
dispatch is `AppStore::dispatch`, which clones out of the store's own
registry); it also skipped the invariants `AppStore::dispatch` applies
(`self_insert_run`/`jump_highlight`/`recenter_cycle` clears). **Done:**
deleted; its 3 tests migrated to `store.dispatch` with every assertion
preserved (`git grep dispatch_by_name` = 0).

### W5 — `quit` is a public bool field mutated in four places — **DONE `6156748`**
`AppStore.quit` (`src/app/store/mod.rs:1925`) is `pub` and set `true` in
three different `keys.rs` sites (plus the control socket's `do_command`).
**Done:** added `AppStore::request_quit()`; all 3 mutation sites in
`keys.rs` route through it. The field stays `pub` (the read side spans
Snapshot/Root/Session). Only `request_quit` sets it now.

### W6 — headless store construction is duplicated
`AppStore::at(start, base)` exists; `--index-profile` builds its own headless
path without a store; `main.rs`'s TUI path builds `AppStore::new()` + config +
watcher + indexing inline; `Session::at` is now a fourth-ish site.
**Recon (2026-10-02): the sites legitimately differ** (the TUI startup runs
config + watcher + indexing inline on a live store; headless `Session::at`
builds a throwaway store; `--index-profile` builds none). No dedup without
adding a mode flag. Disposition: **leave** (do not add a fifth path).

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
file view's title row. It reclaims a row but shifts every file-view row index
by one: `cursor_cell`, `mouse_click_position`/`click_target_line`, the
note-row offsets, the static-render test that asserts `code_idx >= 3` ("title
+ line 0 code + 2 notes"), and every PTY drive that maps terminal rows to
buffer lines (the `row = terminal_row - 1` title offset in
`src/ui/root/hooks.rs`). Do it as its OWN issue in the post-CLI sweep, with
the offset fix and drive updates in one commit. **Also check `Blame`/`Log`/
`Search`** — the same "title row + status line" duplication may hold there;
the directive is stated for the file view, apply consistently where it holds.

**W13 status: DONE** (`3d36c3d`). The file-view title row is gone; the file view's row 0 is now the first code line and the status line still carries the name. `cursor_cell` Buffer arm `1 + banner + content_row` -> `banner + content_row`; both mouse sites subtract only the banner; test/drive row maps shifted -1; `drive_magit_narrow`'s row-0 assertion re-expressed STRONGER (asserts the new content row IS lib.rs AND the status line names it). Scoped to the FILE VIEW (magit/search/log/blame titles untouched). **Orchestrator-verified independently on a fresh root: build/clippy/test clean (1119), `gate.sh full` GATE RESULT: OK (full), zero failures.**

### W21 — the shared PTY fixture `/tmp/redline_pyte_repo` is UNREPRODUCIBLE (pre-existing infra)
**Found by the 019 plan-level gate (2026-10-02)**: `tools/gate.sh full` failed
~12 suites at SETUP (0–1s each) with
`CalledProcessError: git -C /tmp/fx<pid>/redline_pyte_repo checkout -- .`
returned 128. Cause: the fixture repo **does not exist** and NO committed tool
creates it:
- `tools/fixture.py::reset()` only RESETS an existing repo (`git checkout`,
  marker re-apply) — it has no `git init`/`mkdir` path;
- `grep` across every `tools/*.py` shows the suites ASSUME the path
  (`repo("redline_pyte_repo")`) but nothing builds it;
- it was never in git (`git ls-files | grep pyte_repo` = 0 commits).
It was a **developer-machine `/tmp` artifact**; the `/tmp` cleanup (documented
incident: "PTY fixtures are wiped with /tmp: rebuild (`tools/fixture.py`…)",
`docs/ux-testing-plan.md`) took it, and the doc's "rebuild via fixture.py"
instruction is FALSE for a from-scratch machine — fixture.py cannot create it.
Consequence: **a fresh clone cannot run the PTY battery at all**; on this box
every shared-fixture suite (sweep, drive_all, drive_*_narrow, windowing,
check_cursor_stream, drive_xref, …) fails at setup while suites with their own
fixture (drive_external_*, drive_issue_011_*, drive_control) pass. NOT caused
by plan 019 (the failure is at fixture setup, before any app code runs).
Disposition: **fix** (pre-existing infra, own issue) — commit a real fixture
BUILDER (a `tools/fixture.py::create()` or a `tools/make_fixture.sh`) that
materializes `redline_pyte_repo` from scratch (3 tracked files + the 5
de-strays + a git repo + the baseline staged/unstaged markers), so the battery
is reproducible on a clean machine. Until then the fixture is reconstructed
per-machine and the battery is not portable.

**W21 status: FIXED** (`40c1ffa`). `tools/fixture.py` gained a committed
content spec + `create()` (idempotent; builds `redline_pyte_repo` — 6-commit
log, staged `src/lib.rs`, unstaged `README.md`, 5 strays — and
`redline_tall_repo`); `tools/gate.sh` builds the fixtures before the battery.
**Orchestrator-verified independently on a fresh root (`/tmp/rvfx`): fixture
builds correctly (exact search counts 2/1/5, correct status) and
`gate.sh full` = 38/38 suites OK, GATE RESULT: OK (full)** — every previously
broken shared-fixture suite now passes, with ZERO drive assertions modified.
