# Redline UX Test Plan

Living document. Complements unit tests and review gates with *human-in-the-
loop* verification of the interactive surface. The frozen-first-frame bug
(found by the first real user, invisible to 227 unit tests) is the reason
this file exists: **nothing counts as UX-verified until a human or a scripted
PTY has driven it in a live terminal.**

- Flow IDs (`U-A1`, `U-C3`, …) are stable — reference them in findings.
- Check items off per environment; log results in the Findings Log at the
  bottom. New flows get appended; fixed findings get a ✓ and stay as
  regression-watch items.

---

## How to run a pass

```bash
# isolated environment (never pollutes real recents/registry)
export XDG_CACHE_HOME=/tmp/rl-ux-cache
cargo build && ./target/debug/redline    # in the repo you want to browse
```

Two harness levels:

1. **Scripted PTY** (reproducible, greppable):
   ```bash
   { sleep 2; printf '\030\006'; sleep 1; printf 'ma'; sleep 1; printf '\r'; sleep 1; printf 'q'; sleep 1; } \
     | script -qec "timeout 12 ./target/debug/redline" /tmp/rl-cap >/dev/null
   echo "exit=$?"          # 124 = app never quit (bug); 0 = clean
   grep -a "Find file" /tmp/rl-cap   # frame content assertions
   ```
   Key bytes: `C-x`=`\030`, `C-f`=`\006`, `C-g`=`\007`, `RET`=`\r`, `ESC x`=
   `M-x`, `C-s`=`\023`, `C-r`=`\022`, `TAB`=`\t`, `C-c C-c`=`\003\003`.
2. **Real terminal, human hands**: everything in this file, ultimately, in
   your daily terminal. Scripted PTY proves the wiring; hands prove the feel.

Diagnostics: `~/.cache/redline/redline.log` (panics, watcher, exits).
Reproduce frame-freeze suspects with the PTY harness *first* — a screenshot
of nothing is not a report.

## Bug report template

```
Flow: U-C2   Env: wezterm/truecolor/140x40   Commit: <hash>
Keys: C-x C-f → "src" → RET
Expected: main.rs opens highlighted
Actual: frame never repainted; log shows key_event returned
Capture: /tmp/rl-cap ; log tail attached
```

## Environment matrix (run each pass in at least the bold cells)

- **Terminals**: **your daily driver**, tmux pane, gnome-terminal, xterm
  (256-color), a truecolor terminal with `COLORTERM` unset
- **Sizes**: **80×24**, 140×40, very wide/short (200×15), resize mid-session
  (every view mode)
- **Repos**: **this repo (red)**, a 10k+-file repo, a non-git folder,
  a repo with CRLF files / no-trailing-newline files / binaries / multibyte
  names
- **Input**: real keyboard, IME/multibyte typing, rapid key mashing, ssh
  session (latency)

---

## Reference: bindings at time of writing (authoritative: `M-x` palette)

Global: `M-x` palette · `C-g` cancel · `C-x C-f` find-file · `C-x C-b` buffer
list · `C-x b` switch-buffer · `C-x k` kill-buffer · `C-x C-c` quit ·
`C-x g` magit-status · `C-s`/`C-r` isearch · `M-.` definitions · `M-,` jump
back · `C-i`/`Tab` jump forward · `M-i` imenu · `M-?` references ·
`M-s o` occur · `C-c p p` projects · `C-c p f` find-file · `C-c p e` recents ·
`C-c p s` symbols · `C-c p s s` project search.

Buffer view: `C-n/C-p/j/k` line · `C-d/C-u` half-page · `C-v/M-v` page ·
`g` reload · `G` bottom · `M-<`/`M->` top/bottom · `M-g g` goto-line.
Magit: `s`/`u` stage/unstage · `TAB` fold · `RET` visit · `g` refresh ·
`n`/`p` cursor. Search: `n`/`p` match · `RET` jump · `g` re-run · `C-g`
cancel · `q`/`ESC` close. Pending: issue 08 (`l`/`b`/`c`/`y`/`z`).

---

## U-A · Startup & shell

- [ ] **U-A1 Cold start**: launches <1s feel; `*scratch*` frame + status line
      (project name, branch, dirty counts, view name); log file created.
      *Watch*: current known gap — no welcome/recents view (issue 09).
- [ ] **U-A2 Startup outside a git repo** (plain folder with marker file, and
      a bare folder): graceful fallback, no panic.
- [ ] **U-A3 Startup in a 10k-file repo**: status line shows indexing
      progress; UI stays responsive while indexing (keys repaint immediately).
- [ ] **U-A3b Index progress honesty**: counter advances during indexing; clears on completion; incremental refresh shows `indexing…` not fake totals.
- [ ] **U-A4 Second instance** while first is running: no cache corruption
      (recents/registry still load in both).

## U-B · File finding & buffers

- [ ] **U-B1 `C-x C-f` flow**: prompt appears; typing filters with no lag on
      10k files; preview follows selection; `RET` opens highlighted file.
- [ ] **U-B2 Ignored paths** (`target/`, `node_modules/`, `.git/`) never
      appear in candidates.
- [ ] **U-B3 Backspace edits the query**; empty query lists everything;
      no match shows a clean empty state (not a crash).
- [ ] **U-B4 `C-x C-b` buffer list**: open buffers listed, `*scratch*`
      present; `n/p` move; `RET` switches; `d`… (kill via `C-x k` on a
      buffer: gone from list; killing the last file buffer lands sanely).
- [ ] **U-B5 `C-c p e` recents** populate as files are visited and survive a
      restart (with isolated cache: survive a clean restart, not across
      XDG changes).
- [ ] **U-B6 Deleted-file edge**: result/recents entry for a file deleted
      on disk — clean error or skip, never a panic; view stays usable.

## U-C · Reading & scrolling (FileView)

- [ ] **U-C1 Motion**: `C-n/C-p`, `C-d/C-u`, `C-v/M-v` — repaint per press,
      scroll indicators correct at top/bottom.
- [ ] **U-C2 Syntax colors**: Rust/TS/Python/Go files show faces; theme
      plausible; a file with tabs/long lines/multibyte (é, 中) aligns.
- [ ] **U-C3 Large file (50k lines)**: scroll stays instant; jump to
      `M-g g 40000 RET` lands near line 40000 (1-based convention check!).
- [ ] **U-C4 Huge file (>10MB)**: opens as plain text, never hangs.
- [ ] **U-C5 Nasty files**: CRLF, no trailing newline, empty file, binary —
      no blank lines mid-file (ropey chunk bug class), no garbage.
- [ ] **U-C6 `M-<`/`M->`/`G`**: top/bottom; `G` then `M-<` round trip.
- [ ] **U-C7 Resize during FileView**: viewport reflows, position kept
      sensibly, indicators correct at every size down to 80×24.

## U-D · Navigation

- [ ] **U-D1 `M-.` on a function call** in a multi-file repo: lands on the
      definition; status line which-function updates.
- [ ] **U-D2 `M-.` on an ambiguous name** (same symbol, 2 files): picker
      with both definitions + preview; `RET` jumps.
- [ ] **U-D3 `M-.` on a type/constant name** (uppercase): currently falls
      back to enclosing symbol (known gap — verify behavior, log if fixed).
- [ ] **U-D4 `M-,` returns to the exact origin line**; `C-i` forward again;
      mash `M-,` at stack bottom — clean no-op, no panic.
- [ ] **U-D5 `M-i` imenu**: all symbols of the file (nested present; flat
      display is a known gap); `RET` jumps; preview sensible.
- [ ] **U-D6 Symbol picker** (`C-c p s`): fuzzy over project; preview shows
      definition context; jump records onto the stack.
- [ ] **U-D7 Which-function** while scrolling through nested fns/methods:
      updates per line, shows innermost enclosing symbol.

## U-E · Search & references

- [ ] **U-E1 `C-c p s s`**: prompt → type → **first hits appear while the
      walk continues** (do not require completion); per-file counts tick up;
      title shows running→finished.
- [ ] **U-E2 Cancel paths mid-search**: `ESC`, `q`, and `C-g` — search stops
      promptly, partial results stay consistent, no hang, store state clean.
- [ ] **U-E3 Results navigation**: `n`/`p`/arrows move; `RET` jumps to the
      exact line; `M-,` returns to results with selection restored; `g`
      re-runs.
- [ ] **U-E4 Counts**: hand-verify a query's per-file counts against `grep`
      in a scratch repo (equals `rg -c` semantics).
- [ ] **U-E5 `M-?` references** on an identifier used in code + comment +
      string (Rust file): comment/string hits dropped. Same on a `.txt`:
      all kept (honest fallback).
- [ ] **U-E6 `M-s o` occur** on current buffer: all matches, counts; jump
      works; on `*scratch*` does not crash (known edge).
- [ ] **U-E7 Rapid re-search**: start search, immediately start another —
      old job cancelled, no interleaved results, generation semantics hold.

## U-F · Git surface

- [ ] **U-F1 `C-x g` status**: sections Staged/Unstaged/Untracked; dirty
      counts in status line match `git status --porcelain | wc` reality.
- [ ] **U-F2 Stage/unstage**: file-level `s`/`u`; hunk-level on a multi-hunk
      file; `git diff --cached` agrees afterwards; index-only (workdir
      untouched).
- [ ] **U-F3 Fold/unfold + `RET` visit**: sections collapse; visit opens the
      file at the right place (file-level while hunk-offset pending 03-seam).
- [ ] **U-F4 Live status**: edit a file in another pane → status + counts
      update within ~1s without keypress (watcher → magit refresh).
- [ ] **U-F5 Commit flow** *(after issue 08)*: stage → `c` → editor pre-filled
      → type message → `C-c C-c` → commit in `git log`, status clean; `C-c
      C-k` aborts leaving repo byte-identical.
- [ ] **U-F6 Log** *(after 08)*: `l` lists commits, paging, `RET` diff.
- [ ] **U-F7 Blame** *(after 08)*: `b` matches `git blame` on sample files.
- [ ] **U-F8 Branch/stash** *(after 08)*: `y` checkout/create; `z` pop/drop;
      checkout triggers reindex; status line branch updates.

## U-G · Live repo (watcher)

- [ ] **U-G1 Edit a viewed file in another pane**: view repaints within ~1s,
      **scroll line preserved** when the line still exists; clamp when the
      file shrinks below the anchor.
- [ ] **U-G2 Agent churn**: 10 rapid saves → no event storm, no freeze; final
      content shown.
- [ ] **U-G3 Conflict path**: a locally-owned/edited buffer shows the
      "changed on disk" marker instead of clobbering; `g` force-reloads and
      clears it. (Today reachable via scratch/tests; full UX once editing
      exists — keep on the list.)
- [ ] **U-G4 Noise immunity**: `.git/` internal churn, log file writes → no
      reload flicker.
- [ ] **U-G5 Project switch** (`C-c p p`): watcher swaps (old root stops),
      symbol index rebuilds for the new project (**watch: stale-index race —
      fixed in a617b72+; verify no foreign symbols**), status line updates.
- [ ] **U-G6 Suspend**: `M-x toggle-watcher` → disk edits do nothing; toggle
      again → live again. With `auto_reload = false` in config, the resume
      message should not claim watching resumed (known cosmetic gap).

## U-H · Modes, prefixes, cancel discipline

- [ ] **U-H1 Pending prefixes**: `C-x` shows pending in status line; `C-c p`
      pending; unmatchable key after a prefix cancels cleanly with echo.
- [ ] **U-H2 `C-g` matrix**: picker open (closes picker), search running
      (cancels search, view stays), isearch active (exits isearch), pending
      prefix (clears it), idle (no-op/message). Each state independently.
- [ ] **U-H3 Unknown keys echo** in the minibuffer in every view; unprintable
      keys (F-keys, arrows in Buffer view) do nothing harmful.
- [ ] **U-H4 Key mashing**: 20 random keys fast — no pending-state corruption,
      no panic, `C-g` recovers.
- [ ] **U-H5 Mode overlap**: picker open + `C-s` (should not latch isearch
      behind the picker — known gap, verify current behavior); isearch inside
      magit/search views (bindings should be view-appropriate).

## U-I · Config & persistence

- [ ] **U-I1 Missing config**: starts with defaults; `bindings=0` in log.
- [ ] **U-I2 Key overrides**: rebind a command via `[key-bindings]` → works;
      an override that shadows a view binding behaves per docs; malformed
      TOML → tolerated or clean error, never a panic.
- [ ] **U-I3 `auto_reload = false`**: no watcher reloads; toggle command
      messaging accurate.
- [ ] **U-I4 Theme selection**: config theme switch → cache invalidates,
      colors change on next render.
- [ ] **U-I5 Persistence**: recents + project registry survive restart;
      corrupt cache files (truncate one) → tolerated, rebuilt.

## U-J · Terminal & rendering edge cases

- [ ] **U-J1 Terminal restore on every exit path**: `q`, `C-x C-c`, Ctrl+C,
      and a forced kill — cursor visible, colors/scrollback sane, alternate
      buffer exited.
- [ ] **U-J2 Panic path**: (dev-only) trigger a panic → terminal restored +
      `redline panicked` in log (issue-03-era unwrap panics must stay fixed).
- [ ] **U-J3 80×24 everywhere**: every view usable; status line never wraps.
- [ ] **U-J4 Resize storm**: drag-resize continuously → no crash, final frame
      correct.
- [ ] **U-J5 No-truecolor / 256-color**: colors degrade, nothing unreadable.
- [ ] **U-J6 Multibyte & wide chars**: filenames and content render aligned;
      isearch over multibyte safe (fixed in 03 — regression watch).
- [ ] **U-J8 Idle stability**: with no input and no file changes, zero repaints over 5 idle seconds (render counter/log).
- [ ] **U-J9 Burst coalescing**: 50 rapid file writes -> bounded repaints, no sustained flashing; pane fills the terminal exactly at 100x30 and on resize.
- [ ] **U-J7 Mouse**: click/scroll in fullscreen — either functional or
      silently ignored (document which); no escape-sequence garbage on screen.

## U-K · Known-issue watchlist (regression checks from reviews)

- [ ] Uppercase-initial `M-.` misses types/constants (review 05 non-blocking)
- [ ] imenu flat, no impl-parent nesting (review 05)
- [ ] `search_jump` closes view when open fails; occur-on-scratch RET error
- [ ] `M-,` under Search view: no-op until view closed
- [ ] Page scroll has no 2-line overlap (emacs `next-screen-context-lines`)
- [ ] `C-s`/`C-r` during isearch don't repeat (swallowed); `C-s` behind an
      open picker latches isearch
- [ ] isearch `n`/`N` can never appear in a query (e.g. "main")
- [ ] EOFNL "(no newline)" cue not rendered; `unstage_hunk` on a fully
      staged-added file writes an empty blob instead of removing the entry
- [ ] goto-line 0-based vs its 1-based error message
- [ ] `Theme::name()` hardcoded "default" (cache key constant)
- [ ] Watcher late-publish race after stop/replace (benign, revisit with UI)

---

## Findings log

| Date | Flow | Env | Commit | Result | Notes / issue link |
|---|---|---|---|---|---|
| 2026-09-16 | U-A1/U-B1/U-K(all keys) | user terminal | pre-a617b72 tree | **FAIL** | App first-frame-frozen: no repaint ever; fixed by revision-tick (fix-live-input lane), pending review |
| 2026-09-16 | U-A1/U-J3/U-C1/D(all) | user terminal | 413f843 | **FAIL (4 findings)** |
| 2026-09-16 | U-H2/U-B1/U-A3b/U-J8 | sized-pty (pyte) | 148aec1 | PARTIAL FIX |
| 2026-09-16 | U-F1-U-F5/C(all) | user terminal | f2a3eec | **FEEDBACK** |
| 2026-09-16 | U-F1-F4/H(all) | sized-pty (pyte attributes) | 2c0faf0 | **VERIFIED + 1 GAP** | Transient menu live: ? opens grouped menu, C-c p submenu descends, C-g closes, anti-drift derivation renders from live engine. Magit readable post-layout-fix. GAP: magit cursor bar does not render after n/p movement (no attribute change on any row across 8 cursor moves) -- plan 002 issue 02 confirmed as the fix target. Discard-by-cursor blocked on cursor visibility (store logic unit-tested). | (1) incremental 'indexing…' flashing per churn batch — FIXED (silent incremental, f2a3eec). (2) magit not fleshed out — plan 002 issue 01. (3) cursor not visible — plan 002 issue 02 (cursor & rendering audit). Plan 001 archived. | C-c family unbound from iocraft (ignore_ctrl_c) -- C-c p f live-verified, C-c no longer quits; index race fixed (receiver pre-subscribed, indicator clears); tree first-open populates walk. OPEN: tree/palette frames overprint (stale cells at layout shifts) in pyte reconstruction -- likely the user-visible flashing; needs real-terminal pass + possible iocraft renderer investigation. | (1) indexing indicator frozen at 0/N for whole build; (2) pane does not fill screen (no width on root View); (3) quick black flashing on churn; (4) arrows unbound in nav views, cursor visibility weak. All -> issue 09 PART A (spec .agents/tasks/issue-09-impl.md). Navigation directive: cursor visible, hunk/line oriented. |
| 2026-09-16 | U-A1/B1/H2/K | sized-pty + user | 788b25f5 review | FIXED | Frozen-frame root cause: store mutations invisible to render loop; revision-tick + read + definite heights. Picker renders, C-g→q exits 0, watcher repaints. Search hits now deterministic (path,line,col). |
| 2026-09-17 | U-sweep (39 flows + overprint matrix) | sized-pty 80×24 (pyte) | 5db68f6 + working tree (B.2 pin, perf-harness ignore; round 3 enumeration must-fixes) | **ALL PASS** | Plan 002 issue 03 sweep (fix round 3 — the final review's four enumeration must-fixes): 39 flows PASS (A1; B1-B6 [B5 populate leg: visiting src/main.rs then C-c p e lists it — restart leg not PTY-driven, recents-across-restart is unit-tested (recent_files_persist_across_restart); B6: src/lib.rs deleted on disk → the recents picker skips it, the surviving recent is still listed, the picker stays usable]; C1; C6 [on a 50-line file: M-> scrolls to bottom, M-< round-trips to the top, G scrolls, G→M-< round-trips]; E1; E3; E2 cancel legs [ESC closes the results view; q closes it; the C-g cancel-without-closing leg is H2-search]; F1; F2; H1; H2×5 [idle / picker open / pending prefix / isearch active / search running — each state pyte-asserted; the search-running cancel is driven on a 6000-file repo so the walk is in flight when C-g lands]; H3; q-quit; notes; palette; editable-keys [C-x g dispatches while typing in notes; self-insert intact, prefix tail not typed]; graft-pollution [graft/ subtree pruned from the file walk; finder count line reads 0-of-N on the 'graft' query]; G3 [a locally-owned buffer — the notes buffer, the only key-editable one — shows the '⚠ changed on disk' marker after a keystroke (watcher Access self-sustain) plus an external disk append; reload-buffer, the command `g` dispatches in file views, supersedes it: marker cleared, disk line shown, typed char gone; the plain `g` key self-inserts on an editable buffer by design, so the reload leg is driven via M-x]; J3 [80×24 no-wrap: the status line renders on exactly one row — the bottom row — in both the buffer and magit views, with no spill onto the row above; the dedicated no-wrap check on top of every flow already running at 80×24]; F3 [TAB fold reveals the hunk rows under the file section (file sections start folded), TAB again hides them, RET on the file row opens that file in the buffer view]; F4 [an external disk edit to a FILE buffer is picked up by the watcher and the file view repaints within the debounce with no keypress; a plain non-locally-owned file buffer auto-reloads rather than raising the marker (the marker is the locally-owned path, G3); `g` force-reloads]; F5 [stage the unstaged file, open the commit editor (`c`), type a message, commit (`C-c C-c`) — verified by `git log` in a dedicated throwaway repo (REPO5) and the status buffer going clean, so REPO's baseline/history stay pristine]; F6 [`l` lists commits, an in-page key moves the selection to exactly one cursor row, RET opens the selected commit's diff]; F7 [`b` blames the current buffer's file — per-line rows with a hash/author/age prefix and exactly one cursor row; find-file sets the current buffer (not the top view), so the current-file check reads the status line's which-function]; F8 [`y` opens the branch picker (lists the local branch), `z` with no stashes shows the empty state `no stashes`]; G1 [an external append at the bottom of a viewed file repaints the view (watcher) and preserves the scroll anchor — the top line stays put; the shrink-clamp half is unit-tested (reload_anchor)]; G2 [10 rapid disk writes → the view shows the final content and the app stays responsive (a keypress repaints, no freeze); the bounded-repaints / no-event-storm property is the debounce coalescing, unit-tested]; G5 [a second project registered in an isolated cache (`XDG_CACHE_HOME` redirected, the real registry untouched) → `C-c p p` switches to it: the status line + find-file land on the new project; a real two-project switch is drivable because the harness runs sequential apps]; G6 [`M-x toggle-watcher` OFF → a disk edit produces no reload or marker; ON again → the next edit lands]). Overprint matrix 14/14 transitions clean — now including the contract-named palette open/close and picker open/close, and the tree ON/OFF shift as stale/mismatch reference-diffs (replacing the old presence heuristics). **cargo test: 346 passed / 0 failed / 2 ignored** (clippy `--all-targets -- -D warnings` clean) — the 2 ignored are the pre-existing doc-placeholder test (src/ui/root.rs:808 pty_live_verification_documented) and the src/perf.rs perf harness (perf_remeasure_500_file_repo, #[ignore], run explicitly with --include-ignored; both pass under -- --ignored); the B.2 tree pin strengthened the existing tree_title_and_rows_on_separate_lines test in place — no +1, no test-count change. **Verified fixed from plans 001/002**: cursor (drive_magit/log/search/tree/windowing all green), layout collapse (overprint clean), notes conflict (no false marker on open; see the new finding below for the editing form), demo commands (palette clean), q-quit (bare q does not quit), windowing (cursor stays in view), overprint (no stale/mismatched rows at any transition, incl. palette/picker open/close and tree ON/OFF), **editable keys** (C-x g / C-c p f dispatch while editing notes; printable self-inserts), **graft pollution** (graft cache cards absent from the finder). PERF (Part C; method: `src/perf.rs` store-level microbench, 500-file Rust repo, cold cache, median of 5): cold start ~310/~75 ms debug/release (prior table ~50/~15 — >20% movement, table updated), index ~110/~33 ms ≈ ~4500/~15000 files/s (prior ~200/~50 — table updated), search first-hit ~6/~1.1 ms (within 20% of prior ~5/~1 — left, noted). Tools: `tools/sweep.py` (overprint), `tools/sweep_flows.py` (U-* flows), `src/perf.rs` (perf harness). |
| 2026-09-17 | U-G3/notes-edit (NEW) | sized-pty 80×24 (pyte) + redline.log | 5db68f6 | **NEW FINDING (unfixed)** | The watcher treats its own ACCESS (read) events as project changes: after the notes file is created, debounced Access(Open) batches self-sustain at ~500 ms cadence (each batch reloads the open buffer; the reload's own read produces the next batch; the re-index reads likewise). Consequence: once the buffer is locally-owned (first keystroke sets locally_modified), the loop's next batch sets a FALSE "changed on disk" marker ~0.5 s after typing — the issue-05 notes-conflict symptom in its editing form (the open-without-editing form stays verified fixed: no marker after 6 s idle, file mtime unchanged). Evidence: pyte frame shows "⚠ changed on disk — press g to reload" ~0.5 s after the first keystroke while the file's mtime is unchanged and atime advances every cycle; redline.log shows an Access(Open(Any)) batch every ~500 ms. NOT fixed here (Part B scope is the overprint fix + tree pin only). Fix direction: drop pure-access events in watcher summarize / apply_project_change, or mtime-guard reload_buffer. |

## Backlog (plan-003 candidates)

| # | Item | Source | Notes |
|---|------|--------|-------|
| 1 | **Scrollability sweep** (user-requested): make every long-content pane cursor-following like magit status — commit-diff/`git show` pane (RET on a log commit), blame view (one row per file line), notes/scratch editable buffers (long rope content), log within-page selection stranding below the fold. Generalize `magit_scroll` (store.rs, issue 02) into a shared windowing helper and apply to all four. | User directive 2026-09-17; issue-04 review follow-up | Search results + file view already window; magit status done in 002-02. |
| 2 | **Watcher Access-event fix** (see the NEW FINDING row above): drop pure-access events in watcher summarize / apply_project_change, or mtime-guard reload_buffer. Explains the false "changed on disk" ~0.5 s after typing in notes and the self-sustaining ~500 ms reload/reindex loop. | Issue-03 sweep | Mechanism confirmed in source; G3 flow drives the symptom reproducibly. |
| 3 | **Banner nuance**: "⚠ changed on disk — press g to reload" is misleading on editable buffers where plain `g` self-inserts by design (issue-05 data-loss protection) — the reachable path is M-x reload-buffer. | Final review, issue 03 | Small copy/behavior fix; pair with #2. |
| 4 | Carried review non-blockings: F4 tracked-path magit-refresh unit test (watcher → dirty counts); flow_c6 bottom-anchor assertion (last row reads line 50 after M->/G); flow_g3 typed-char precondition assert; flow_b3 partial-leg note (no-match leg only); D-group N/A clause overbroad re: imenu/symbol pickers; menu overflow "+N more" indicator; armed-discard TOCTOU re-validation; misleading "nothing staged" error when git_status errors; tree.rs test doc-comment says "moved" (store is Arc'd). | Review rounds, plans 001/002 | None block; opportune during #1/#2. |
