# Task: Implement plan 002 issue 05 — Editable-buffer keys & content coherence (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained: read it, then execute it in order. The library references in
`.agents/skills/` are authoritative ground truth.

Context: plan 001 + plan 002 issues 01/04 are committed. A live sized-PTY
drive found four coherence gaps (diagnoses attached — verify each, then fix).

## Working agreement (overrides any caution)

- **Skills are truth.** Work straight from `.agents/skills/*.md`. Do NOT read
  dependency sources under `~/.cargo/registry`, do NOT browse docs.rs, do NOT
  fetch anything. If a skill lacks an API detail you need, write the most
  reasonable call consistent with the skill and keep moving.
- **Write-first.** Fix in your first handful of tool calls per finding,
  compile early, iterate. No front-loaded research.
- **Skill corrections.** If running code contradicts a skill file: code
  reality wins, AND you make a minimal, factual correction to the relevant
  skill file; list edits under "skill corrections".
- **graft is available** (CLI on PATH): `graft map/ask/callers/skeleton/grep`.

## Read first (in this order)

1. `.agents/plans/002-magit-depth-and-cursor/05-editable-keys-and-content-coherence.md`
   — THIS issue.
2. `.agents/plans/002-magit-depth-and-cursor/PLAN.md` — the live-UX audit
   findings (section "LIVE-UX AUDIT") that this issue fixes.
3. `src/app/store.rs` — `key_event` (the interception order: CommitEditor arm
   got this RIGHT — mirror it), `open_notes`, watcher apply path, seed
   registry, Buffer view keymap (`q` binding).
4. `src/model/files.rs` + the finder source (`menu`/picker candidates for
   find-file) — for the graft/ exclusion.
5. `.agents/skills/emacs-ux/SKILL.md` — q/C-x C-s conventions.

## Findings to fix (each with diagnosis)

1. **Editable buffers break multi-key sequences.** In the notes buffer,
   pressing `l` (plain printable) self-inserted while a `C-x` prefix was
   armed — the printable-interception runs BEFORE prefix continuation, so
   `C-x g`, `C-c p f` etc. are dead while editing. Fix: mirror the commit
   editor's interception order — when a key arrives and `pending` is
   non-empty OR the key would EXTEND a possible sequence (i.e.
   `engine.resolve(pending + key)` is Pending or Command), route through the
   keymap FIRST; only self-insert when the key cannot continue any sequence
   AND is printable. C-g keeps its global cancel semantics (clears pending;
   does NOT close the notes buffer).
2. **Notes false conflict.** Opening notes (`C-x n`) creates
   `.redline-notes.md` and the watcher immediately flags the buffer
   "changed on disk" (the app's own file creation). Fix: suppress the
   self-inflicted event — e.g. mark the buffer just-created and ignore the
   first matching watcher event for it (or exclude the notes path from the
   change application when the buffer is locally-owned and unmodified
   on-disk-vs-our-creation). Keep the genuine conflict path (external edit)
   working.
3. **graft cards pollute the file finder.** The repo's `.ignore` re-admits
   `graft/` for grep (intentional — search should see it), but the FILE
   finder and tree sidebar list graft cache cards (graft/src/main.md ranked
   ABOVE src/main.rs for "main"). Fix: exclude `graft/` from the file walk
   used by the finder/tree (a walk-level filter in `src/model/files.rs` —
   keep grep's access via `.ignore` untouched). Tree sidebar inherits the
   fix automatically (same walk).
4. **Demo commands pollute the palette.** `demo-message-1`, `demo-message-2`,
   `message-echo`, `insert-demo-text` are the first things a new user sees.
   Fix: remove them from the seed registry (and their demo wiring); update
   count-carrying tests; keep the palette count test authoritative.
5. **Bare `q` in the main Buffer view quits the entire app.** One stray `q`
   loses the session. Fix: in the Buffer/FileView keymap, `q` becomes
   close-view semantics consistent with list views (closes overlays/views;
   when only the main view remains, it does nothing or shows a hint) — `C-x
   C-c` remains the quit. Update tests that assert q-quits.

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green (baseline 334).
- Store tests: printable key inserts in notes AND `C-x g`/`C-c p f` dispatch
  while a notes buffer is current; C-g in notes clears pending only; notes
  creation does not set changed_on_disk (watcher-path test); external edit of
  notes still sets it; graft/ absent from finder candidates for a query
  matching a graft card; demo commands absent from the registry/palette;
  bare `q` in Buffer view does not set quit.
- Sized-PTY (mandatory): with a notes buffer open, `C-c p f` opens the file
  finder; typing inserts into notes; palette shows no demo commands; finder
  query "main" lists src/main.rs before/without graft cards; `q` in the main
  view leaves the app running.
- Declare in your report which behaviors are test-covered, PTY-covered,
  manual-only.

## Report format

- **Per-finding table**: finding | diagnosis confirmed? | fix | verification.
- **Verification**: exact commands + pass/fail + counts; PTY checks list.
- **Skill corrections** (or none). **Deviations**; known gaps.
