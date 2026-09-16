# Task: Implement issue 09 — Polish, packaging & field feedback (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained: read it, then execute it in order. The library references in
`.agents/skills/` are authoritative ground truth.

Context: issues 01–08 are implemented, reviewed, and committed. This is the
final issue before archive. It has TWO halves: **(A) field-feedback fixes**
from the first real user sessions — these are must-fix and come FIRST — and
**(B) the original polish scope** (tree, themes, notes, mouse, perf, docs).

## Working agreement (overrides any caution)

- **Skills are truth.** Work straight from `.agents/skills/*.md`. Do NOT read
  dependency sources under `~/.cargo/registry`, do NOT browse docs.rs, do NOT
  fetch anything. If a skill lacks an API detail you need, write the most
  reasonable call consistent with the skill and keep moving.
- **Write-first.** Create all new modules in your first handful of tool calls,
  then run `cargo build` early and iterate on specific errors. No front-loaded
  research.
- **Skill corrections.** If running code (compiler errors, runtime behavior)
  contradicts a skill file: code reality wins for the implementation, AND you
  make a minimal, factual correction to the relevant skill file so future
  agents are not misled. Never rewrite a skill file wholesale. List every
  skill-file edit in your report under "skill corrections".
- **graft is available** (CLI on PATH, graph in `graft/`, kept fresh by the
  orchestrator): `graft map`, `graft ask "<query>"`, `graft callers <symbol>`,
  `graft skeleton <file>`, `graft grep <pattern>` — for cross-file
  orientation; never a replacement for the read-first list.

## Read first (in this order)

1. `docs/plans/001-redline-code-browser/PLAN.md` — success criteria + the
   "Out of scope" list (do not creep past it).
2. `docs/plans/001-redline-code-browser/09-polish.md` — THIS issue.
3. `docs/ux-testing-plan.md` — the UX flows and the findings log; your
   work is verified against these flows.
4. `.agents/skills/iocraft/SKILL.md` — especially use_terminal_size,
   ScrollView, mouse events, TextInput (notes editing).
5. `.agents/skills/tree-sitter/SKILL.md` + existing `src/syntax/` — tree
   sidebar needs the file walk, not parsing.
6. Existing code: `src/ui/root.rs` (root View sizing + tick), `src/ui/magit_status.rs`
   (selected-row face pattern), `src/app/store.rs` (keymap seeds,
   `apply_project_change`, `indexing_display`, `IndexProgress`),
   `src/nav/index.rs` (build_index progress), `src/model/files.rs` (walk),
   `src/model/buffer.rs` (editable path for notes).

## PART A — field-feedback fixes (must-fix, do first)

These came from real user sessions. Each has a diagnosis attached; verify the
diagnosis in code, fix, and prove with the listed verification.

1. **Pane does not fill the screen.** The root View sets `height` from
   `use_terminal_size` but **no width** — the pane is content-sized
   horizontally. Fix: set both dimensions explicitly from the terminal size
   (and re-read on resize — the tick already fires on Resize). Verify in a
   sized pty (e.g. 100x30): the rendered frame's status line spans the full
   width, and a file view's content area fills all rows.
2. **Indexing indicator reads "forever".** `IndexEvent` publishes only once
   per job, so `indexing 0/N` freezes for the whole build, and per-batch
   refreshes keep it alive nearly constantly in busy repos. Fix (two parts):
   (a) publish honest coarse progress — update the shared `IndexProgress` and
   have `build_index` push a progress event every K files (K≈25) or on a
   ~200ms cadence, with the FINAL event unchanged (same generation contract);
   (b) `indexing_display` shows `indexing N/M` truthfully and, when a job is
   incremental, `indexing…` without misleading totals. Verify: in a pty
   against a repo with a few hundred files, the counter visibly advances.
3. **Flashing on churn.** Every watcher batch → reload + git2 refresh +
   reparse + tick bump → full repaint; in a busy repo that is constant
   flashing. Fix: coalesce the drain loops — after each `recv()` wake, drain
   ALL immediately-available events from the channel/before bumping the tick
   once (one repaint per burst, not per event); and skip the git2 refresh
   when the change touches no tracked file the status view shows (cheap
   pre-filter before `refresh_magit`). Verify: 50 rapid file writes →
   bounded repaint count (assert via a render counter in a store-level test
   where possible; pty-check no continuous flashing while idle).
4. **Navigation keys & cursor (user directive: "navigation should have a
   cursor and otherwise be hunk and line oriented").**
   - Bind `Up`/`Down` arrows (plus `PageUp`/`PageDown`) alongside existing
     bindings in EVERY list/navigation view: Buffer/FileView (line scroll),
     MagitStatus, Search results, BufferList, and pickers (arrows already
     move there — verify).
   - Make the cursor VISIBLE in every such view: verify the magit
     selected-row face actually renders distinctly (contrast, not just
     weight); apply the same treatment to the search results view if its
     selection is not obvious. A cursor the user cannot see is a bug.
   - Magit status stays hunk/line-oriented: `n`/`p` move by row,
     section-level ops stay bound as they are; do NOT add mouse-first or
     menu-first navigation.
5. **Review watchlist quick wins** (all previously recorded; fix now):
   - `M-.` identifier scan skips uppercase-initial names (types/constants) —
     include them; add tests.
   - `C-s`/`C-r` while isearch is active repeat the search (next/prev match).
   - `C-s`/`M-g g` must not latch behind an open picker (gate on
     `picker.is_none()`).
   - Page scroll keeps a 2-line overlap (emacs `next-screen-context-lines`).
   - `M-g g` accepts 1-based input (matching its own error message and emacs).
   - `unstage_hunk` on a fully-staged-added file removes the index entry
     instead of writing an empty blob.
   - imenu: indent nested symbols by enclosing extent (visible outline).
   Leave recorded-only (no fix): EOFNL display cue, `Theme::name()`
   hardcoding, `JumpEntry.col`.

## PART B — original polish scope

6. **Tree sidebar** (`src/ui/tree.rs`): toggle command (bind per helm/projectile
   conventions, e.g. `C-c p t` — check the skill quick-references), ignore-
   aware file tree from the existing walk, cursor navigation, `RET` opens,
   optional buffer-follow (off by default). Rendered as a left column; must
   respect terminal size (PART A fix).
7. **Themes**: 2–3 built-in themes in `src/theme.rs` + config selection
   (existing plumbing); `Theme::name()` becomes config-derived (fixes the
   cache-key constant); document the faces.
8. **Notes/scratch**: per-project notes file (e.g. `.redline-notes.md` or
   under the project) opened as an editable ropey buffer with explicit save
   (`C-x C-s` following emacs convention — verify against the emacs-ux
   skill) — reuses the commit-editor save path; conflict rules identical to
   issue 04 (locally-owned).
9. **Mouse**: wheel scroll in Buffer/FileView + list views; click-to-position
   in FileView (line) — best-effort per the iocraft mouse API; document
   limitations in the report.
10. **Performance pass**: record cold-start, index throughput (files/s),
    search latency on a large repo; fix anything egregious the numbers expose
    (candidate: per-keystroke candidate cloning in the picker — only if
    numbers say so). Record numbers in the report.
11. **Docs**: `README.md` (install, keymap cheat sheet, screenshots-lite via
    asciinema-style description), `redline.1` man page, keymap table
    generated from the registry (a test asserts docs match registered
    bindings — cheap and prevents rot).

**Deferred (do NOT implement)**: Hunk handoff (`H`) — stretch goal, no stable
surface verified; leave a TODO. Undo. Splits. LSP.

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green (baseline 267).
- **Sized-PTY checks (mandatory for PART A + interactive PART B)** — extend
  the established harness pattern:
  - 100x30 frame: status line spans full width; tree sidebar toggle renders
    a left column; file view fills the pane.
  - Index progress advances on a multi-hundred-file temp repo.
  - Idle stability: with no input and no file changes, NO renders occur over
    5 idle seconds (render counter via log or test).
  - Burst coalescing: 50 rapid writes → bounded repaints (assert counter).
  - Arrow keys move in every navigation view; cursor visibly highlighted
    (assert distinct escape/face in the frame capture for the selected row).
  - isearch repeat via `C-s`/`C-r`; page scroll overlap; 1-based `M-g g`.
- Unit tests: tree walk snapshot, notes save/load + conflict, theme
  switching invalidates the highlight cache, keymap docs test (docs match
  registry), uppercase `M-.` fix, unstage empty-blob fix, page-scroll overlap
  math.
- Perf numbers recorded (not just asserted): cold start (debug + release),
  index files/s on the temp repo, search latency first-hit.
- Declare in your report exactly which behaviors are covered by tests, which
  by PTY checks, and which remain manual-only.

## Report format

- **Module map**: file + one-line purpose (new + modified).
- **Field-feedback section**: per PART A item — diagnosis confirmed/adjusted,
  what changed (file:line), how verified.
- **Verification**: exact commands + pass/fail + counts; PTY checks list.
- **Skill corrections**: every skill-file edit (or "none").
- **Deviations**; known gaps; perf numbers.
