# Task: Implement plan 004 issue 03 — mark, select, kill/yank (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained. `.agents/skills/*.md` are authoritative ground truth.

Context: plan 004 issues 01/05/02 are committed (isearch, cursor, adopts).
User-approved row 14 with "mark/select" (docs/emacs-parity-log.md). The
app is read-focused; the user wants emacs kill/yank + mark/region.

## Working agreement (overrides any caution)

- **Skills are truth.** Work from `.agents/skills/*.md` (ropey, emacs-ux,
  iocraft). Do NOT read dependency sources under `~/.cargo/registry`, no
  docs.rs, no fetching.
- **Write-first.** Scaffold early, compile, iterate.
- **Skill corrections.** Code reality wins; minimal factual correction,
  listed under "skill corrections".
- **graft** CLI available.

## Read first

1. `.agents/plans/004-emacs-parity/03-mark-and-kill-yank.md` — the issue.
2. `docs/emacs-parity-log.md` — row 14 decision.
3. `src/model/buffer.rs` — buffer state (ropey), editable/locally_modified
   semantics, insertion-point handling.
4. `src/app/store.rs` — insert_text / notes-editable branch (plan-002-05
   interception order), file-view cursor/selection state, C-g matrix.
5. `.agents/skills/emacs-ux/SKILL.md` — kill/yank + mark/region semantics.

## What to build

1. **Mark & region**: `C-SPC` set-mark (transient per emacs: region
   active until C-g or a command that moves the point far; implement the
   simple model — region active from mark to current position; C-g clears
   mark AND region). Movement extends the region when a mark is set.
2. **Region face**: new `region` face (theme, both modes, high-contrast —
   e.g. background blend distinct from the cursor bar); rendered
   attribute-level in the file view over the marked range (inclusive start,
   exclusive end per emacs), across line boundaries.
3. **Kill ring (one shared, store-level, depth 60 per emacs)**:
   - `C-w` kill region (editable buffers: removes text to the ring;
     read-only views: ring gets the text, buffer unchanged — emacs
     read-only kill-ring-save behavior).
   - `M-w` copy region to ring (both kinds).
   - `C-y` yank (editable buffers; inserts at the insertion point; sets
     the point after the inserted text).
   - `M-y` yank-pop (cycles ring depth backward, replacing the last
     yank; only valid immediately after C-y).
   - `C-x C-x` exchange point and mark.
   - Kill ring is shared ACROSS buffers (kill in a read-only view, yank
     in notes — the cross-buffer test).
4. **Editing integration**: mark/region live per buffer; kill/yank only in
   editable buffers (insert_text path); the notes printable-interception
   order (004-01) applies: C-SPC/C-w/M-w/C-y/M-y/C-x C-x are chords, so
   they route normally; printables still self-insert while typing.
5. **Status display**: when a region is active, show region size in the
   status line (emacs: "Saved to register" no; the echo "Mark set" on
   C-SPC — minibuffer echo per emacs).

## Constraints

- No dependency changes. Undo is NOT in scope (row 15: "no undo yet") —
  kill/yank must be safe without undo: C-w from an editable buffer is
  irreversible except via yank; that is accepted (emacs users know).
- No regressions: C-g matrix, pending-prefix, isearch, discovery
  (004-06 pending — no), recenter, scroll overlap, cursor stream.
- Scope fence: src/model/buffer.rs, src/app/store.rs, src/app/command.rs,
  src/ui/file_view.rs (+ region face render), src/model/theme.rs (region
  face), tests, tools/ flows. Nothing else.

## Verification (iterate until ALL pass)

- Gates: build / clippy -D warnings / cargo test all green (372 / 2
  baseline + new, 0 failed).
- Suites: sweep.py 14/14; sweep_flows.py 43/43; drive_all 6/6;
  drive_windowing 28/28; drive_windowing_panes 4/4;
  check_cursor_stream 11/11.
- New pyte flows (tools/sweep_flows.py or a drive script): set-mark →
  region face visible (attribute-level, multi-line range); C-w removes +
  ring holds; C-y restores (exact text); M-y cycles; M-w in a read-only
  view → C-y in notes (cross-buffer); C-g clears region; C-x C-x swaps.
- Coverage split in the report.

## Report format

- Design summary (state model, ring semantics, region render path). Per-
  command disposition. Gate outputs (exact counts). Skill corrections (or
  none). Deviations; known gaps.
