# Task: plan 004 issue 06 — discovery home (drop *scratch*)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained. `.agents/skills/*.md` are authoritative ground truth.

Issue contract (user directive 2026-09-17): "I think we also need a
top-level hydra menu when no buffer is open to discover all the
functionality. We don't need *scratch*."

## Working agreement

- Skills are truth (iocraft SKILL.md for render; no registry reads, no
  docs.rs, no fetch). Write-first; compile early, iterate on named errors.
  Minimal skill corrections, listed. `graft` available.

## Read first

1. `src/app/store.rs` — the scratch auto-create at boot (grep `scratch`),
   the buffer stack / empty-state render logic, `open_scratch`.
2. `src/ui/transient_menu.rs` — the derived transient-menu engine from
   plan 002 issue 01 (keymap × command registry → grouped menu). This is
   the anti-drift machinery you MUST reuse; do not hand-write home content.
3. `src/ui/root.rs` — the root render switch (where the current buffer is
   rendered) — home is a new arm, not a buffer.
4. `.agents/skills/emacs-ux/SKILL.md` — splash/startup (we deliberately
   diverge; note it).

## What to build

1. **Remove the default `*scratch*` buffer.** A fresh session (and any
   state where the buffer stack/list is empty) renders a HOME view. Home
   is NOT a buffer: it never enters the buffer list or recents, and it has
   no `locally_modified` state.
2. **Home content is derived, anti-drift**, from the live keymap ×
   registry (reuse/extend the transient-menu renderer for an all-groups
   layout): every top-level group at once — single-key commands, prefix
   groups with their members (e.g. `C-x`, `C-c p`, `M-x`), and chords.
   Never a hand-maintained list.
3. **Header**: project name + dirty counts + "redline". **Footer**: the
   standard help line (`C-x C-c` quits, `?` opens the descendable menu).
4. **Opening anything replaces home**: file (`C-x C-f`), magit (`C-x g`),
   search, notes (`C-x n`), project-find, etc. — every existing entry
   point works from home.
5. **Keys on home**: `q` is UNBOUND (no buffer to close); `C-x C-c` quits
   immediately (nothing to save with no buffers — consistent with 004-04).
   `?` opens the descendable transient menu.
6. Its own parity-log row: divergence from the emacs splash is deliberate
   (KEEP) — append to `docs/emacs-parity-log.md`.

## Orchestrator pre-check (refreshed 2026-09-18 late, against HEAD 88e245f)

- **Derive machinery already exists — reuse, never hand-write**: `store.rs`
  has `menu_entries_for_path(&KeySeq)`, `menu_entries()`, `menu_rows()`,
  `menu_height()`, `menu_bindings()` turning the live keymap × command
  registry into grouped rows (prefixes first, then leaf commands by
  registry category, headers per group). Home = the all-groups view of
  exactly this; add an "all top-level groups" query rather than a new
  formatter. (grep these names; they have drifted from any old line range.)
- **`open_scratch` call sites to change** (refreshed): the explicit
  command `command.rs:156` (`open-scratch` — may stay, but should now open
  HOME instead of a buffer), `store.rs:3976`, `store.rs:7081` (auto-create
  fallbacks → HOME), and `store.rs:2024` is the definition. Tests call
  `store.open_scratch()` directly (grep `open_scratch` in tests) — expect
  those to be reworked to the home state.
- **`ViewId` has NO `Home` variant today** (`store.rs:173-205`): add
  `ViewId::Home` (and its `name()` arm) — home is a view, not a buffer.
- **`q` is not dangerous to inherit**: on `ViewId::Buffer`, bare `q` is
  bound to `close-view` (an explicit no-op when it is the only view — see
  the comment at the `ViewId::Buffer` keymap arm), NOT to `quit`. So home
  must simply NOT bind `q` at all (or bind nothing that closes/quits);
  `C-x C-c` remains the only quit. Verify the home keymap has no `q`.
- **`*scratch*` appears in existing assertions**: it is a buffer key in
  `BufferTable` (`src/model/buffer.rs:32` `SCRATCH_NAME`); tests
  `new_table_has_current_scratch` (`buffer.rs:~328`), the buffer-order
  tests (`:~354`), and root render tests (`root.rs:939`, `:1029`, `:1118`)
  assert it. Those move to the home state (no scratch entry, so any
  buffer-list counts change).
- **THE REAL COST (found 2026-09-18, do not underestimate)**: `BufferTable::new()`
  (`src/model/buffer.rs:188`) itself inserts a fresh `*scratch*` and sets it
  `current`. So "drop *scratch*" is NOT merely removing the `open_scratch`
  call sites: `BufferTable::new()` must start EMPTY, `Buffers::current`
  becomes `None` at boot, and the UI must render HOME for that state.
  There are **37** `current_buffer()` uses in store.rs and many `None`-guarded
  paths already exist (`unwrap_or(0)`, `unwrap_or(false)`, `unwrap_or_default`)
  — but each boot-path consumer must be checked for a `None` that used to
  be impossible. Suggested shape: keep `SCRATCH_NAME` only as a legacy
  key (do not create it), add an explicit `ViewId::Home`, and have the
  root render switch on "no current buffer ⇒ Home". Tests that assert
  `*scratch*` in the initial frame must be reworked to the home state.
  If this proves larger than a single issue, STOP and report — do not
  silently leave half the invariants; I will split it (e.g. "empty table +
  Home view render" then "remove scratch affordances/commands").
- `?` already opens the descendable transient menu and works; home's `?`
  should do the same.

## Constraints

- Scope fence: `src/app/store.rs` (drop scratch create; empty-state/home
  render state; expose derived groups), a new home view module under
  `src/ui/` OR a root arm in `src/ui/root.rs`,
  `src/ui/transient_menu.rs` (reuse/extend renderer), tests, `tools/`
  flows, `docs/`. No dependency changes; no undo. Do not alter the 004-05b
  point motion, kill/yank, or 004-04 quit machinery.
- All suites green (counts change where scratch tests were updated):
  `cargo test`, `sweep.py` 14/14, `sweep_flows.py` (boot-state flow
  updated), `drive_all` 8/8, `drive_windowing` 28/28,
  `drive_windowing_panes` 4/4, `check_cursor_stream` 80/80, and the rest
  of the battery via `tools/gate.sh full`. NOTE: `tools/gate.sh` is the
  current gate runner (fast = rust-only, full = whole battery at the safe
  read-quiet).

## Verification (iterate until ALL pass)

- Gates: build / `clippy --all-targets -- -D warnings` / cargo test green.
- Anti-drift test: mutate the registry (or add a temp command) in a test
  and assert home's derived groups change accordingly — proving home is
  generated, not hard-coded.
- PTY flow(s): boot renders home with derived groups (assert a sample of
  real group labels/keys are present exactly once); from home, `C-x C-f`
  opens a file (home replaced); `C-x g` opens magit; `q` on home is a
  no-op (app alive); `C-x C-c` from home exits immediately with NO prompt;
  `?` opens the descendable menu. Update the existing boot-state flow(s)
  that assumed `*scratch*`.
- Confirm `*scratch*` no longer appears anywhere at boot (grep the frames).

## Report format

Home render design (data flow from keymap×registry). Diff of removed
scratch machinery. Per-key table on home. Gate outputs (exact counts).
Skill corrections (or none). Deviations; known gaps.
