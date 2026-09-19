# Task: plan 004 issue 07 — recenter after a jump (M-. landing geometry)

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth.

## Origin (user report, 2026-09-18)

"When jumping to a new file, the followed entity should not be at the bottom
of the viewport."

## Diagnosis (already done by the orchestrator — do not re-investigate)

- Redline's landing path is `set_point` → `keep_cursor_visible`
  (`src/app/store.rs`, the pure fn near `blame_line_display`), which is a
  **minimal-scroll** helper: `cursor >= scroll + window ⇒ scroll = cursor+1-window`.
  So a target BELOW the current window lands on the LAST visible row.
- **Emacs parity reference (verified live with `emacs -Q -nw` 30.2):**
  `xref-after-jump-hook` is `(recenter xref-pulse-momentarily)` and
  `recenter-positions` is `(middle top bottom)`. So **every M-. jumps to the
  middle of the window**, not the bottom. (Verified empirically: after `M-.`
  the cursor landed at the *top area* — row 4 of 28 — with the target
  rendered at the top row, i.e. the window was fully repositioned, not
  minimally scrolled.)
- Goto-line / isearch / plain navigation do NOT recenter in emacs — leave
  their minimal-scroll behavior alone. This change is for JUMP landings only.

## What to build

1. A dedicated landing helper, e.g.
   `fn recenter_point_in_window(&mut self)` (or `recenter_landing`), that
   positions the window so the current point's line sits at the **middle**
   row (emacs `recenter` default), clamped by `[0, total - vp]`. Reuse the
   arithmetic already in `recenter()` (`C-l`) but do NOT consume/advance
   `recenter_cycle` — a jump is not a `C-l`; it must not perturb the
   `recenter-top-bottom` cycle. (Factor a shared helper if that keeps both
   honest; the cycle index is the only thing that must differ.)
2. Call it on JUMP landings only. Find every site that lands a jump and
   decide, with a one-line justification in your report for each:
   - M-. same-file / cross-file unique (the `jumped to {file}` site in
     `xref_definition_candidates`/`xref_find_definitions`),
   - the resolver fall-through landing (`open_resolved_source`),
   - Xref picker **selection** (`run_selected`, Xref arm),
   - imenu selection,
   - `M-,` / `navigate_to_entry` (jump-back) — emacs also recenters these
     (same hook); include unless you find a concrete reason not to.
   NOT: `goto_line` (M-g g), isearch, search-RET, mouse click, `C-n/C-p`,
   `M-</M->`. If a site is ambiguous, check vanilla emacs and say so.
3. Do NOT break the "jump lands at column 0 / start of line" rule
   (`set_point_line`), and do NOT change `keep_cursor_visible` itself
   (many non-jump callers depend on minimal scroll).
4. Keep the change inside the existing versioning/one-way-door style: it is
   pure window math on a landing; no persistence, no new state.

## Constraints

- Skills are truth (`.agents/skills/*.md`; no registry/docs.rs/fetch).
  Write-first; compile early. iocraft is not involved (store-level math).
- Gate runner: `tools/gate.sh fast` (inner loop), `tools/gate.sh full`
  (final: whole PTY battery — App starts are ~0.6s now, battery ~6.7min).
  Progress streams to stderr — do NOT pipe stdout through `tail`.
- PTY flock: "shared PTY fixture is busy" + exit 3 ⇒ wait and retry; NEVER
  two suites concurrently; wrap EVERY python PTY invocation in `timeout`.
  Honest gate counts.
- Scope fence: `src/app/store.rs` (the helper + call sites + tests). A new
  PTY leg in `tools/` is allowed if useful. No `src/ui/`, no `src/syntax/`,
  no resolver crate, no dependencies.
- Plain `git commit` (identity already configured); do not `git add -A`
  other sessions' files. Commit on the current branch only.

## Verification (iterate until ALL pass)

- `tools/gate.sh full` green, counts honest; all existing suites unchanged
  (this alters only landing geometry — `drive_xref` 10/10, `drive_all` 8/8,
  `drive_windowing` 28/28 must stay green; if any existing flow asserted the
  old bottom-landing row, it is legitimately updating — call that out).
- Unit tests (discriminating — must FAIL against `keep_cursor_visible`):
  - jump DOWN to a line far below the window → point's screen row is the
    middle (not `vp-1`);
  - jump UP to a line far above → middle (not row 0);
  - a jump to a line in a file SHORTER than the viewport → clamped, no panic,
    whole file visible, top = 0;
  - a jump within the current window still recenters (emacs recenters
    unconditionally) OR is a no-op — pick the emacs-faithful behavior, prove
    which by checking emacs, and pin it;
  - the `C-l` cycle is unaffected (a jump between two `C-l`s does not reset
    or advance the middle→top→bottom cycle).
- A PTY leg (preferred): open a file with a definition ~200 lines below,
  M-. to it, and assert the definition row is near the middle of the content
  area (not the last row). Reuse the `tools/drive_xref.py` fixture pattern.
- Live check: in the real repo, M-. on a symbol far down a large file and
  report the landing row relative to the viewport.

## Report format

The helper + its arithmetic; the call-site table (site → recenter? → why);
the emacs check for any ambiguous site; unit + PTY test list; gate counts
(honest); deviations; live landing row.
