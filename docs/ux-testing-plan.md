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

## Pooled parallel sweep

The release gate (`tools/gate.sh full`) runs the 12-PTY-suite battery
**serially**: every suite hardcodes the same fixture paths (`/tmp/redline_*`),
and the driver's per-repo `flock` (keyed on the repo **abspath**, backlog #13)
refuses to run two fixture-touching suites at once. That lock is the whole
reason the battery serializes — the suites are independent processes. Serial
cost: **~399 s** of PTY time.

`tools/pool.py` removes the serialization by giving each concurrently-running
suite a **private lane** instead of a shared fixture:

- `setup N` copies every `/tmp/redline_*` repo into `N` lanes under
  `REDLINE_POOL_ROOT` (default `/tmp/rl/<i>/`), and into each lane a
  **rewritten copy of `tools/`** whose fixture literals point at the lane's own
  copies. Each lane also gets a private `XDG_CACHE_HOME` (`<lane>/xdg`).
- Because each lane's repo has a **different abspath**, its `flock` never
  contends with another lane's → real parallelism. A lane crash cannot corrupt
  a sibling lane.
- A lane is an **exclusive** resource: `run`/`runall` draw lanes from a queue,
  one suite per lane, so jobs > lanes queue instead of colliding (the
  corruption class of backlog #13).
- `tools/gate.sh pooled` = the Rust gate + the battery run this way
  (`pool.py setup` then `pool.py runall`), on the fast 0.06 quiet window
  (loop-02; `REDLINE_PTY_QUIET=0.2` for the old conservative window).
  Lane count is `REDLINE_POOL_LANES` (default 4; 4–6 measured well on a
  24-core box). `full` stays the sequential, always-works fallback.

**Measured, same binary** (4 lanes, 0.2 quiet): serial `full` ≈ **399 s** PTY
(425 s incl. build/clippy/test); pooled ≈ **220 s** (−45%), **12/12 suites
identical verdicts** — sweep 14/14, drive_all 8/8, windowing 28/28, panes 4/4,
cursor-stream 80/80, notes 17/17, syntax-notes 8/8, xref 10/10,
external-notes 16/16, external-crate 12/12, sweep_flows 65/65, ux_sweep 3
pre-existing findings.

**Measured at the fast 0.06 default (loop-02)**: serial battery ≈ **254 s**
(at 0.2 the same suites measured ≈ 415 s; sweep_flows alone 170 s → 85 s,
check_cursor_stream's raw-stream timing is quiet-independent and unchanged)
and pooled ≈ **117 s** (12/12 suites, sweep_flows 65/65, 3x stable). The
quiet window only speeds up the quiet-bounded suites; the raw-stream and
fixed-settle legs dominate the residual.

Two hard-won constraints (they are not obvious):
1. **Keep lane paths short.** The status line shows the project *name* (the
   basename). A path long enough to overflow 80 cols used to wrap the status
   line onto a second row and shift every content row (U-J3 asserts the status
   line is exactly one row; several suites read a hardcoded minibuffer row,
   `MINI=22`). The status-line `NoWrap` + hidden-overflow fix (see
   `src/ui/root.rs` `StatusLine`, pinned by
   `status_line_long_text_stays_one_row`) mitigates this, but the pool root is
   kept tiny (`/tmp/rl`) so lane paths stay short regardless.
2. **Preserve fixture BASENAMES** (`redline_pyte_repo`, …): suites assert those
   literals (mode-line project name, U-A1/U-J3/U-G5). Only the *parent* dir is
   swapped for the short lane dir; the basename is copied verbatim.

Cleanup: `tools/pool.py clean` removes the whole pool root (see backlog #17
for the disk-use note).

## Harness note: the gate must cover the whole WORKSPACE

`Cargo.toml` declares `[workspace] members = ["crates/*"]` but no
`default-members`, so a bare `cargo test` / `cargo clippy` operates on the
ROOT PACKAGE ONLY. Every gate run before 2026-09-19 therefore silently
skipped `crates/redline-resolve`: **92 tests (85 lib + 7 integration) and all
of its clippy lints never ran**, while the plan worked on that crate
(006-01/02/03, 007-03). Found when 007-03 added a public `scope` field to the
crate and the worker observed only "0 tests" for `-p redline-resolve` in the
default `cargo test` output.

- `tools/gate.sh` now uses `--workspace` on build, clippy, and test
  (4 test binaries, 648 tests total). Do not drop it.
- **Lesson**: when a workspace gains a member, verify the gate actually
  compiles and tests it — `cargo test -p <member>` should show a non-zero
  test count. A gate that cannot see a crate is worse than no gate, because it
  reports green.
- Separately observed once (1 in 6 on a cold-compile run, 8/8 clean under
  deliberate parallel load): `git::repo::tests::stage_file_then_unstage_matches_cli`
  failed during a concurrent workspace test run, then passed 5/5 in isolation
  and 8/8 under load. Not reproducible; the resolver integration tests use
  fresh tempdirs + isolated `CARGO_HOME` and do not touch the repo, so the
  mechanism is unexplained. Treat a single such failure as re-run-before-believe
  (the counts are otherwise deterministic).

## Harness note: SPLIT the largest issue before dispatching it

004-06 (discovery home / drop `*scratch*`) sat in the queue as the largest and
riskiest item for hours. Its own spec said "if this proves larger than a single
issue, STOP and report — I will split it", which is the right instinct but a
waste of a worker's budget: the worker discovers the size AFTER being
dispatched.

Do the pre-check read-only and split FIRST. For 004-06 the orchestrator
pre-check (no build cost, ~1 minute of greps) found:

- `BufferTable::new()` inserts a fresh `*scratch*` at boot (`buffer.rs:188`);
- 37 `current_buffer()` uses vs 40 existing `None` guards in `store.rs` — the
  empty state is partly prepared but boot-path consumers need auditing;
- 96 `ViewId` references (adding `Home` touches name/keymap/render);
- existing tests + flows assert `*scratch*` at boot, so counts change.

That was enough to split into 06a (empty table + Home render — the risky,
invariant-carrying half) and 06b (retire the remaining scratch affordances and
update flow expectations), each independently verifiable, with the split
boundary written into both specs. A spec that says "split if too big" is a
prompt to split, not a licence to hand a worker a coin flip.

## Harness note: parallel lanes need DISJOINT files, and a worktree

Running two workers at once is only safe with one writer per tree. The pattern
that works (used 2026-09-19 for 011-01 + loop-02):

- `git worktree add -b <branch> /tmp/redline-<name> HEAD` gives the second lane
  its own checkout, its own `target/`, and its own binary
  (`REDLINE_BIN=/tmp/redline-<name>/target/debug/redline`). Never let two lanes
  edit the same file, and never let a lane touch the other's worktree.
- Pick lanes that are FILE-DISJOINT (e.g. resolver crate + store.rs vs
  tools/ + docs/). Overlapping scope serializes instead — say so in the spec.
- **The PTY fixtures are still a single shared resource** even across
  worktrees: the recomposed tools copy the same `/tmp/redline_*` repos. Two
  lanes both running PTY suites therefore contend on the driver's flock (by
  design — it exits 3 rather than corrupt state). Give a lane its own
  `REDLINE_POOL_ROOT` so `tools/pool.py` copies fixtures into a private lane
  set instead of sharing.
- Merge order matters only for overlapping hunks; verify with
  `git merge-tree --write-tree <main> <branch>` (read-only) before landing.

## Harness note: bound the WORKER, not just the probe

Two lanes on 2026-09-19 burned far past a reasonable budget for their task
size (140 turns / 21M tokens; 98 turns / 7.7M tokens). Neither was stuck in
the end — one was a genuine multi-file feature, the other was thrashing on
reference-tool probes — but by the time it showed, the tokens were spent.

Rules this produced:
- **Every worker spec states an explicit budget expectation** ("land the
  implementation within N tool calls; do not start new investigations after
  the implementation compiles") — cheap to add to `.agents/tasks/*.md`, and it
  gives the worker a reason to stop polishing.
- **Cap reference-tool probing.** If a spec needs a parity answer from emacs
  (or any external tool), state the answer in the spec if it is already known,
  or bound the probe to a handful of calls. Do not leave "probe the reference
  tool" open-ended — that is where the unbounded loops come from.
- **Watch the turn count, not just the clock.** "Active but long-running"
  with a high turn count means thrashing even when it is technically making
  progress; steer it to the write-up with the facts it already has.

## Harness note: never leave a PTY probe un-bounded

A worker probing emacs recenter geometry wrote an elisp file via a mangled
heredoc, then ran `emacs ... -l probe.el` under a `pty.fork()` +
`os.waitpid(pid, 0)` with **no timeout**. Elisp failed to load, emacs never
exited, and `waitpid` blocked forever — the worker's `bash` tool stayed open
for 6+ minutes, no activity, and a steering message could not be delivered
until the orphan emacs was killed externally (2026-09-19).

Rules this produced:
- **Every interactive reference-tool invocation needs a hard bound.** Wrap
  the whole probe in `timeout N ...` (and give `waitpid` a deadline with a
  `kill -9` fallback) — a `pty.fork` + blocking `waitpid` has no natural
  timeout, unlike subprocess with `timeout=`.
- Prefer running a reference tool (`emacs -Q -nw`) through the existing
  `tools/drive_emacs.py` `EmacsSession` harness, which already bounds its
  pumps, instead of ad-hoc `pty.fork` scripts.
- When a subagent shows "no activity" + a long-open shell tool, check for a
  stuck child process (`ps`) before assuming it is thinking — kill the child
  and the queued steer will deliver.

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

## U-M · Mark, region, kill/yank (plan 004 issue 03)

- [ ] **U-M1 set-mark**: `C-SPC` (NUL byte) sets the mark at the current point;
      minibuffer echoes "Mark set"; region is active until C-g or a command
      that clears it.
- [ ] **U-M2 region face visible**: after set-mark + movement (C-n), the region
      face (background) is visible at the attribute level on the marked lines
      (multi-line range: first and last region lines show the background).
- [ ] **U-M3 C-w kill region**: with a region active in an editable buffer,
      `C-w` removes the region text and pushes it to the kill ring; minibuffer
      echoes the byte count; mark is cleared.
- [ ] **U-M4 C-y yank**: after C-w, `C-y` restores the exact text at the
      insertion point (the current top line's start); buffer content matches
      the pre-kill state.
- [ ] **U-M5 M-y yank-pop**: after C-y, `M-y` cycles backward through the kill
      ring, replacing the last yanked text with the previous entry.
- [ ] **U-M6 M-w copy region (read-only)**: in a read-only file view, `C-SPC`
      + movement + `M-w` copies the region to the kill ring without modifying
      the buffer; then `C-y` in the notes buffer yanks the copied text
      (cross-buffer kill ring).
- [ ] **U-M7 C-g clears region**: with a mark set, `C-g` clears the mark and
      region (the region face disappears from the file view).
- [ ] **U-M8 C-x C-x exchange**: with a mark set, `C-x C-x` swaps point and
      mark: the cursor moves to where the mark was, and the mark is set where
      the point was.
- [ ] **U-M9 mark persists across movement**: after C-SPC, scrolling (C-n/C-p)
      does not clear the mark; the region extends from mark to current point.

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
| 2026-09-17 | U-G3/notes-edit (NEW) | sized-pty 80×24 (pyte) + redline.log | 5db68f6 → plan-003-01 | **FIXED** | The watcher treated its own ACCESS (read) events as project changes: debounced Access(Open) batches self-sustained at ~500 ms cadence, and once the buffer was locally-owned the loop's next batch set a FALSE "changed on disk" marker ~0.5 s after typing. Fix: `summarize` in `watcher.rs` now drops events whose notify kind is not create/modify/remove (i.e. access events), so access-only batches publish nothing and mixed batches carry only the real kinds. Evidence: (1) unit tests `summarize_access_only_batch_is_empty` and `summarize_mixed_batch_drops_access_keeps_real_kinds` (pure function); (2) live-watcher test `access_events_produce_no_event` (read a file → no bus publish; write → publish, positive control); (3) store test `access_only_batch_does_not_set_changed_on_disk` (locally-owned buffer + access-only batch through `summarize` → no `changed_on_disk` marker); (4) PTY flow G3 reworked: type-then-idle 3 s → no false marker; real external append → marker appears; M-x reload-buffer → marker cleared, disk line shown, typed char superseded. All 39 sweep flows PASS. |
| 2026-09-17 | U-sweep round 2 (43 flows) + carried items | sized-pty 80×24 (pyte) + cargo | plan-003-03 | **ALL PASS** | Plan 003 issue 03 (FINAL — sweep round & carried items): (1) **Banner hint per kind** (the only src behavior change): the "⚠ changed on disk" banner now renders the accurate key per buffer kind — editable → "press M-x reload-buffer to reload" (plain `g` self-inserts by design), plain file → "press g to reload". New store accessor `current_buffer_editable` drives the per-kind hint; both texts unit-tested (`changed_on_disk_hint_plain_says_g` / `changed_on_disk_hint_editable_says_reload_buffer`); the reachable editable (notes) case is PTY-driven (U-BHN: type in notes → external append → banner says "M-x reload-buffer", never "press g to reload"). The plain-file "g" hint is unit-only: plain file buffers are read-only and never locally-owned, so they auto-reload and never render the banner — not PTY-reachable (flow_f4's no-marker leg). (2) **Carried non-blockings** (each with a test/note): F4 tracked-path magit-refresh unit test (`apply_project_change_tracked_path_refreshes_magit_counts`: tracked-path change → dirty counts updated; the classifier `any_tracked` was already tested, this drives the refresh leg); flow_c6 bottom-anchor assert (last content row reads "line 50" after M-> AND after G, not just "the first row changed"); flow_g3 typed-char precondition assert (the typed 'x' is present in the notes buffer BEFORE the reload supersedes it); flow_b3 partial-leg mark (recorded as "U-B3 (no-match leg only)" — the Backspace-edit + empty-query legs are not PTY-driven, per the log's own partial-leg convention); D-group N/A clause trimmed (dropped the overbroad "not a single-PTY pyte assertion" clause for imenu/symbol pickers, kept the true unit-test claim); tree.rs test doc-comment precision ("before the store is moved" → "before the store is wrapped in Arc<Mutex<_>> for the render context"). (3) **New sweep flows** (thin legs; deep drives live in `tools/drive_windowing_panes.py` and are cited): U-CDS commit-diff scroll (M-> lands on the last page, M-< round-trips); U-BLW blame windowing (cursor stays in view across C-n moves past the bottom + M-> to the last line); U-NSL notes-scroll (typing near the bottom scrolls the active row into view); U-BHN banner-hint check per kind (above). (4) **Backlog**: banner (#3) + the carried items (#4) flipped to resolved/closed; two new candidates added — #5 `search_keep_visible` repoint-to-helper and #6 commit-editor unwindowed (both from the issue-02 review). **cargo test: 357 passed / 0 failed / 2 ignored** (+3 new over the 354 baseline: the 2 banner-hint unit tests + `apply_project_change_tracked_path_refreshes_magit_counts`) — clippy `--all-targets -- -D warnings` clean. **sweep_flows.py: 43/43 PASS** (39 baseline + U-CDS / U-BLW / U-NSL / U-BHN); **sweep.py: 14/14; drive_windowing.py: 28/28; drive_all.py: 6/6; drive_windowing_panes.py: 4/4**. **Coverage split**: test-covered — both banner-hint texts + the F4 magit-refresh leg + all carried item assertions; PTY-covered — U-CDS / U-BLW / U-NSL / U-BHN (banner hint on the real render) + the baseline 39; manual-only — none (the deferred items — menu overflow "+N more", armed-discard TOCTOU, misleading "nothing staged" error, insertion-point cue — remain open in the backlog, not driven here). |
| 2026-09-19 | edit-mode legs (NEW ×4) | sized-pty 80×24 (pyte) + cargo | plan-005-01 working tree | **ALL PASS** | Plan 005 issue 01 (file edit mode): C-x C-q toggles the current file buffer between Read-only and Edit (status-line mode word); C-x C-s saves in place. New PTY legs (dedicated src/edit.rs, own App, removed after — fixture baseline untouched): **edit-toggle** (open → status `Read-only`; C-x C-q → `Edit` + "editable" message; no-edit toggle-back is immediate, no confirm; C-x C-q → `Edit` again); **edit-save** (type `zz` → lands in the buffer; C-x C-s → "wrote" message AND the on-disk file ends in `zz` (bytes asserted); idle 1.5 s past the 400 ms debounce → NO false "changed on disk" marker — the new saved-path self-write suppression with the expected-mtime check); **edit-confirm** (type `qq` → C-x C-q arms the "Discard unsaved edits in src/edit.rs to make it read-only? (y or n)" confirm, flip deferred; C-g cancels → edit mode + `zzqq` text kept; re-arm → `y` accepts → `Read-only`, `zzqq` gone (buffer re-reads disk), disk never written); **edit-conflict** (contrast: read-only file buffer auto-reloads an out-of-band append with no marker — pre-005 behavior preserved; then edit mode + type + out-of-band append → "changed on disk" marker, the edit text intact, NO auto-clobber). Unit tests (store): toggle on/off, scratch/non-buffer-view no-ops, typing in edit mode sets locally_modified, save clears it + writes disk, read-only save refuses, confirm arm/cancel(n, C-g, ESC)/accept, saved-path suppression (own event suppressed, repeat event conflicts), genuine external write after save still conflicts, read-only auto-reload unchanged, edit-mode-without-edits locally owned. Model test: `is_locally_owned` edit-mode leg. UI test: status-line mode word. **Gates (actual harness output)**: cargo build ok; clippy `--all-targets -- -D warnings` clean; **cargo test: 465 passed / 0 failed / 2 ignored** (2 ignored = pre-existing doc-placeholder + perf harness); sweep.py 14/14; **sweep_flows.py: 57/57** (53-flow tree baseline + 4 new — the spec's "46/46" predates the 004-04/05h legs); drive_all.py 6/6; drive_windowing.py 28/28; drive_windowing_panes.py 4/4; check_cursor_stream.py 68/68; ux_sweep.py: 3 findings, all pre-existing (window-split keys C-x 2/1/0 unbound by design — parity log row 10 "KEEP"); palette count 101 → 102 (`toggle-read-only` registered). |

## Pooled parallel sweep (tools/pool.py)

The 12 PTY suites share fixture repos, and `pyte_driver` takes an exclusive
flock keyed on the repo **abspath** to prevent two suites corrupting each
other (backlog #13). That made the battery strictly serial: sum(), not max(),
~400s of wall time.

`tools/pool.py` gives each concurrently-running suite a private **lane**
(copied fixtures + a rewritten `tools/` copy pointing at them + a per-lane
`XDG_CACHE_HOME`). Lane copies have different abspaths, so the flock never
contends and the suites run for real. Measured on the same binary:
**12/12 suites, verdicts identical to serial, 219s vs 425s serial (-48%)**
at 4 lanes (a 24-core box measured fine at 4-6).

```bash
tools/gate.sh pooled          # fast + the battery pooled (~3.8 min)
tools/gate.sh full            # the sequential fallback, always works
python3 tools/pool.py clean   # drop the lanes (~105 MB at 4 lanes)
```

Two constraints, both learned the hard way:
- **Lane fixture paths must not be LONGER than main's.** A longer path
  overflows the 80-col status line and wraps it onto a second row, shifting
  every content row and breaking suites that assert a fixed row (the
  hardcoded minibuffer row, U-J3). `pool.py` uses `/tmp/rl/<i>/` for that
  reason. (This also drove a real app fix: the status line is now
  `TextWrap::NoWrap` + hidden overflow, so a deep project path clips instead
  of wrapping — pinned by `status_line_long_text_stays_one_row`.)
- **Preserve fixture basenames** (`redline_pyte_repo`, ...): suites assert
  those literals in the mode line and in U-A1/U-J3/U-G5.

## Backlog (plan-003 candidates)

| # | Item | Source | Notes |
|---|------|--------|-------|
| 1 | **Scrollability sweep** (user-requested): make every long-content pane cursor-following like magit status — commit-diff/`git show` pane (RET on a log commit), blame view (one row per file line), notes/scratch editable buffers (long rope content), log within-page selection stranding below the fold. Generalize `magit_scroll` (store.rs, issue 02) into a shared windowing helper and apply to all four. | User directive 2026-09-17; issue-04 review follow-up | **RESOLVED (002-02 + 003-03)**: shared windowing helper landed in 002-02 (commit-diff/blame/log/editable panes); the sweep now drives all four + a banner-hint check — U-CDS / U-BLW / U-NSL / U-BHN in `tools/sweep_flows.py` (deep drives: `tools/drive_windowing_panes.py`). |
| 2 | **Watcher Access-event fix** (see the NEW FINDING row above): drop pure-access events in watcher summarize / apply_project_change, or mtime-guard reload_buffer. Explains the false "changed on disk" ~0.5 s after typing in notes and the self-sustaining ~500 ms reload/reindex loop. | Issue-03 sweep | **RESOLVED (003-01)**: `summarize` in `watcher.rs` drops non-create/modify/remove (access) events; see the U-G3/notes-edit NEW FINDING row (FIXED) + `access_only_batch_does_not_set_changed_on_disk`. |
| 3 | **Banner nuance**: "⚠ changed on disk — press g to reload" is misleading on editable buffers where plain `g` self-inserts by design (issue-05 data-loss protection) — the reachable path is M-x reload-buffer. | Final review, issue 03 | **FIXED (003-03)**: the banner hint is now per-kind — editable buffer → "press M-x reload-buffer to reload"; plain file buffer → "press g to reload". Both texts unit-tested (`changed_on_disk_hint_plain_says_g` / `changed_on_disk_hint_editable_says_reload_buffer`); the reachable editable (notes) case is PTY-driven (U-BHN). The plain-file "g" hint is unit-only: plain file buffers auto-reload (never locally-owned → no banner), so that case is not PTY-reachable (flow_f4's no-marker leg). |
| 4 | Carried review non-blockings (see 003-03). | Review rounds, plans 001/002 | **Closed by 003-03**: F4 tracked-path magit-refresh unit test (`apply_project_change_tracked_path_refreshes_magit_counts`); flow_c6 bottom-anchor assert (last content row reads "line 50" after M->/G); flow_g3 typed-char precondition assert; flow_b3 partial-leg mark (no-match leg only); D-group N/A clause trimmed (kept the true unit-test claim); tree.rs test doc-comment precision (store is Arc-wrapped). **Still open (deferred)**: menu overflow "+N more" indicator; armed-discard TOCTOU re-validation; misleading "nothing staged" error when git_status errors. |
| 5 | **search_keep_visible repoint-to-helper**: route the search-results keep-visible logic through the shared windowing helper (the other panes' `*_keep_visible`). | Issue-02 review | Candidate; not done (search already windows, so this is a refactor/consistency item, not a behavior fix). |
| 6 | **Commit-editor unwindowed**: the inline commit-message editor buffer is not cursor-following / windowed like the other long-content panes. | Issue-02 review | Candidate; not done (short in practice, but a long commit message could strand the cursor below the fold). |
| 9 | **05d carries (review P2s)**: (a) pure width helpers live in `src/ui/file_view.rs` but are called from `src/app/store.rs`, inverting app→ui layering (move to a non-UI module); (b) `display_col_to_char_index` cannot address a LEADING zero-width combining char (col 0 ⇒ char idx 1), so a leftmost-cell click skips it. | 05d review | (a) **DONE (05e)**: the pure width helpers moved to `src/model/text_width.rs` (headless; same tests); (b) candidate, obscure (documented combining allowance), not scheduled. |
| 10 | **Tree sidebar fixed 34-column width** (`TREE_WIDTH` in `src/ui/tree.rs`): a layout-hardcoded width — at an 80-col terminal it takes 42% of the width and on narrower terminals the code pane shrinks to a sliver (or overflows). 05e deliberately only SHARED the constant with the click offset; the width policy (proportional sizing, resize key, or fit-to-content) is a separate layout decision. | 05e spec (plan 004) | Candidate; layout question, not a bug. No width change in 05e (per its spec). |
| 11 | **Click row ignores the banner row**: `mouse_click_position`'s `row` assumes the file view's title occupies terminal row 0 (no banner); when the "changed on disk" banner is shown the content shifts down one row, so a click is off by one. Pre-existing 05c behavior, not reachable as a regression from 05e. | 05e review (P2) | Candidate; small (subtract the banner row when visible), needs a PTY leg with the banner shown. |
| 14 | **Dense-annotation canvas under-fill**: when most/all in-window lines are annotated, the note-row budget floors the code span at 1, so the canvas shows very few rows (measured: 3 of 21 for a 25-line file with all lines annotated) instead of filling via the largest span s with `s + notes_in_window(s) <= viewport`. The blank-view P1 is fixed (0bd99ad); this is the residual UX question. | 02b re-review round 2 | **FIXED** (eb5be7c, 02c/02d converged): largest-span downward scan (monotone `s + notes_in_window(s)`), floor at 1 kept as the blank-view guarantee; all-annotated repro 3 → 20 of 21 content rows (10 code + 10 note). Review PASS (hand-traced 4 legs; deviations judged in-scope); gates re-run by orchestrator: 479 tests, 80/80 cursor-stream (new all-annotated fill leg), sweep 14/14, sweep_flows 65/65, drive_all 6/6, windowing 28/28, panes 4/4. |
| 15 | **Watcher create-events don't invalidate the file walk**: `ensure_files()` caches `FileList` per root and never rebuilds (`store.rs:2840-2857`), so a file created after session start is invisible to `C-x C-f` until an explicit `C-c p i` re-walk. Found by the 02c worker when the new PTY leg's fixture file wasn't pickable (same trick as sweep_flows' big.txt). | 02c deviation 2 / review | Candidate; small (invalidate the walk on watcher create events), but touches the auto-reload/watch path — needs care around the mtime suppression. |
| 13 | **Concurrent PTY suites corrupt each other (phantom failures)**: all 14 tools drive the SAME fixture repo, and flow legs mutate it, so two concurrent suites produce wrong results (observed live: sweep.py 10/14 vs the worker's 13/14 on the same commit). Fixed in tools/pyte_driver.py: a non-blocking exclusive flock keyed by a stable sha1 of the repo path, held for the suite lifetime; a second suite exits 3 with a clear message. NOTE: the first attempt used builtin `hash()`, which is randomized per process — it silently never contended. | Orchestrator live verify (005-02b) | **FIXED** (8f0375e). Opt out with REDLINE_NO_PTY_LOCK=1. |
| 12 | **Fixture drift (found while verifying 005-01 edit mode)**: an edit-mode PTY leg or manual probe that SAVES real edits into a tracked fixture file (`src/main.rs`) leaves it modified forever — `fixture.py` did not restore tracked files, so every later flow rendered mutated content (I hit this live: main.rs ended with `line 3/4/5` and a marker). Fixed: `reset()` now `git checkout -- .` first, then re-applies the markers, and also removes the stray leg files from backlog #8. | Orchestrator live drive | **FIXED** in tools/fixture.py (this commit); all 57 flows green after. |
| 8 | **Harness hygiene**: `tools/check_cursor_stream.py` writes `src/leg.rs`, `src/wordleg.rs`, `src/cursorleg.rs`, `src/whichfn.rs` into the shared fixture repo and never removes them, so they accumulate as untracked files across runs. No flow currently asserts untracked-file counts (so this is latent, not active flakiness), but it pollutes the magit-status fixture and could make a future untracked-count assertion order-dependent. | Orchestrator UX sweep | Candidate; tools-only cleanup (delete the leg files in a `finally`, or add them to `fixture.py`'s reset). NOT edited while 004-05d holds that file. |
| 16 | **Timing-fragile flows under a faster PTY read-quiet window**: reducing `REDLINE_PTY_QUIET` from 0.2 to 0.06 (tools/gate.sh fast modes) speeds suites ~2x but drops `sweep_flows` to 64/65 — U-BHN (waits on the watcher's ~500 ms banner debounce) and ann-delete (waits on a transient message) assume fixed sleeps, not the app's actual settle. NOT a regression: the old driver on the same binary is 65/65. Fix = replace those flows' fixed `wait()`s with `wait_for(pred)` polls; then the fast window can be the default everywhere. | Loop-speedup work | **RESOLVED (loop-02)**: (1) the two flaky flows are fixed at their root — U-BHN's flake was the app's `created_paths` guard consuming the notes-file CREATION event: when the debouncer coalesces that creation with the test's first rapid append into one batch, the banner legitimately does not fire for that batch, so the flow now does a bounded second external append (the discriminating assertion; a genuinely broken banner still FAILs after the retry budget) and ann-delete's two transient echoes are now `wait_for` polls instead of fixed settles; (2) the absence-style assertion audit: every `not in` / `== 0` / `unchanged` site in `sweep_flows.py` verified gated on a positive completion signal (prompt + count line for finder absence checks, the cancel-echo for the C-g legs, process-exit/disk-state for quit legs, the watcher-debounce wall-clock `pump()` for the "no false marker" legs — which were already wall-clock and stayed wall-clock by design); (3) 0.06 is now the DEFAULT `REDLINE_PTY_QUIET` for every gate tier (pyte_driver default + gate.sh) with `REDLINE_PTY_QUIET=0.2` as the escape hatch; `full-fast` is a redundant alias. Battery proven verdict-identical at 0.06, sweep_flows 65/65, 3x stable; discrimination proven (broken banner-hint text and broken no-annotation message each FAIL their flow). |
| 16 | loop-02 merged (`cf00d7f`): the 0.06 window is now the DEFAULT (escape hatch REDLINE_PTY_QUIET=0.2); root causes were NOT the window — U-BHN is the app's created_paths guard + debouncer coalescing the notes-file creation with a rapid append (fires at 0.2 under load too), fixed with a bounded second append; ann-delete was a late repaint overwriting the transient echo, fixed with wait_for + one idempotent re-arm. Discrimination proven (broke the banner-hint text and the no-annotation message: both flows FAIL). Stable 3x; pooled 0.06 = ~117s; serial 0.06 ~= 254s vs 415s at 0.2 (1.6x overall; sweep_flows 2.0x). New pool-lane artifact fixed: longer lane paths wrap the quit-save prompt at 80 cols — wrap-tolerant flat_text() matcher. |
| 17 | **Pool-lane disk use**: `tools/gate.sh pooled` copies every `/tmp/redline_*` fixture into each lane, so the pool root (`REDLINE_POOL_ROOT`, default `/tmp/rl`) is ~27 MB per lane — **~105 MB for 4 lanes** (the 24 MB `redline_sweep_slow_repo` dominates). It is `/tmp`-ephemeral and never committed, but a 6-lane run grows to ~160 MB. `tools/pool.py clean` removes the whole root; `setup` is idempotent (rebuilds the lanes). | loop-01 (pooled sweep) | **DONE (loop-01)**: documented in the "Pooled parallel sweep" section above; lanes are disposable and cleaned with `pool.py clean`. Candidate to slim further: copy only the fixtures each lane's suite actually reads (the battery uses a subset of all `/tmp/redline_*`), or hardlink the large `sweep_slow_repo` instead of `cp -r`. |
| 7 | Conflict **minibuffer message** at store.rs:4396 ("changed on disk — press g to reload") fires only in the locally-owned (editable) case where plain `g` self-inserts — repoint to the per-kind helper (M-x reload-buffer wording). Same family as backlog #3; found by the issue-03 review. | Issue-03 review (plan 003) | One-line src fix + message-text test. |

## loop-03 — Demote the test pyramid (kept / converted ledger)

**Shape (plan):** PTY → unit → fuzz. This iteration lands the PTY → unit
demotion for `tools/sweep_flows.py`: 50 of its 65 driven flows moved to
store-level unit twins in `src/app/flow_tests.rs` (driven through the same
entry points the live app uses — `AppStore::key_event`,
`apply_project_change`, the search bus — and asserting the same positive
signals on state + `render_at_width(80)`), and the PTY file keeps only the
thin tier: input encoding, repaint-race classes, process lifecycle, and
one end-to-end smoke per drive family. Nothing is untested — the ledger
below is the kept/converted map; "no judgment was relaxed" (a kept flow's
PTY assertions are the live-app proof for what the twin pins at store
level; a converted flow's twin asserts the same signals it did, at the
level where they live).

### Kept in the PTY tier (tools/sweep_flows.py — 15 records, was 65)

| Kept flow | Why it needs a live app | Unit twin (state half) |
|---|---|---|
| U-A1 | boot/frame smoke: launch → home render (drive-family smoke) | unit_flow_a1 (boot smoke twin; the PTY record additionally proves the real launch path) |
| editable-keys | input encoding: multi-key sequences + self-insert through the REAL terminal encoder (C-x g dispatch vs self-insert) | `unit_flow_editable_keys` |
| U-H2 search | in-flight cancel: C-g must land while a real rg walk over 6000 files is in flight (wall-clock race) | `search_c_g_cancels_without_closing (store.rs — the state half; the timing half stays in the kept PTY record)` |
| U-BHN | banner debounce: the hint only lands after the watcher cadence (repaint-race class) | `changed_on_disk_hint_plain_says_g / changed_on_disk_hint_editable_says_reload_buffer (src/ui/file_view.rs — the state half; the timing half is the kept PTY record)` |
| annotation suite (create/toggle/crossing/notes-editable/drift/orphan/delete/cu-scroll) | ann-delete's transient-echo / repaint race + the raw space-byte encoding through the PTY encoder | `unit_flow_ann_*` (8 twins) |
| U-G6 | watcher SUSPENDED: a disk edit produces no reload — the gate is at the watcher source, which the store-level apply path deliberately bypasses | `unit_flow_g6` |
| U-G1 | watcher delivery smoke: disk append → repaint + scroll anchor (one end-to-end watcher leg) | `unit_flow_g1` |
| quit-prompt-y | quit lifecycle: prompt → y → on-disk write → exit 0 (process lifecycle) | `unit_flow_quit_prompt_*` (5 twins) |

### Converted (PTY → unit; `src/app/flow_tests.rs`, 60 twins total)

| PTY flow | Unit twin(s) |
|---|---|
| U-B1–B6 (recents/finder) | `unit_flow_b1..b6` (+ pre-existing `recent_files_*` store units) |
| U-C1/C6 (windowing motion) | `unit_flow_c1`, `unit_flow_c6` |
| U-E1–E3 (isearch) | `unit_flow_e1..e3` |
| U-F1–F8 (magit) | `unit_flow_f1..f8` |
| U-G2 (debounce storm) | `unit_flow_g2` |
| U-G3 (local marker) | `unit_flow_g3` |
| U-G5 (project switch) | `unit_flow_g5` |
| U-H1 (mode hints) | `unit_flow_h1` |
| H2 (home states) | `unit_flow_h2_*` |
| U-H3 (isearch prefix) | `unit_flow_h3` |
| q-quit | `unit_flow_qquit` |
| notes (no false marker) | `unit_flow_notes_no_false_marker` |
| palette | `unit_flow_palette_no_demo` |
| graft | `unit_flow_graft` |
| U-CDS / U-BLW / U-NSL | `unit_flow_cds` / `unit_flow_blw` / `unit_flow_nsl` |
| buffer-list NP | `unit_flow_buffer_list_np` |
| J3 (no-wrap) | `unit_flow_j3` (via `render_at_width(80)`) |
| edit-mode legs (×4) | `unit_flow_edit_toggle/save/confirm/conflict` |
| U-M1–M8 marks + cross-buffer | `unit_flow_mark_kill_yank`, `unit_flow_mark_exchange` (U-M6 cross-buffer leg is the kill-ring leg of the first twin; `quit_prompt_bang` is covered by the pre-existing `quit_prompt_bang_saves_all_remaining_then_quits`) |

### Other suites — evaluated (contract item 3)

| Suite | Verdict | Rationale |
|---|---|---|
| `tools/sweep.py` (transition overprint matrix) | **keep** (candidate for later) | Stale/mismatch row detection over live terminal pixels — terminal-tier by nature; the state halves (view stacks) are store-testable, a follow-up iteration could split it. |
| `tools/drive_windowing.py` / `_panes.py` | **demote** (follow-up) | View-stack/scroll state = store level (the C1/C6/BLW/CDS twins prove the pattern); only the "cursor pixel on-screen" half is terminal. |
| `tools/drive_xref.py` / `drive_external_notes.py` / `drive_external_crate.py` | **demote** (follow-up) | Resolution + landing are store-level (open_file/symbol-index APIs); the PTY legs duplicate unit coverage. |
| `tools/probe_notes_dump.py` | **keep** | Process-lifecycle dump probe (exit behavior + dump file) — the thin tier's lifecycle family. |
| `tools/ux_sweep.py` | **demote** (follow-up) | Keymap-derivation checks are pure store/keymap state. |
| `tools/drive_all.py` / `drive_syntax_notes.py` | **keep** | Drive-family boot smoke + syntax-face plausibility (human-judgment color tier). |

### Harness notes (loop-03)

1. **Stale-binary trap (the gate-run lesson):** the pooled lanes test the
   binary `tools/pool.py` builds ONCE from the checkout it was started in.
   If a lane's worktree is checked out at another lane's commit, the pooled
   gate silently validates the wrong code and reports green. Rule: run the
   pool from the tree under test, and pin the binary explicitly
   (`REDLINE_BIN=$(pwd)/target/debug/redline tools/gate.sh pooled`); the
   lane checkouts must match that tree.
2. **Width-bounded static render:** `render_at_width(store, 80)` (src/ui/root.rs)
   pins the root View to a terminal width in static renders — without it the
   root is content-sized and df95113's width pin is a no-op (the picker count
   line renders off-screen). Regression pin:
   `render_at_width_catches_offscreen_picker_count_line` — proven
   discriminating (removing the df95113 width pins makes it FAIL).
3. **Measurements (this box, 0.06 quiet):** sweep_flows 65 flows / ~87 s →
   15 records / ~24 s; 60 unit twins run in ~2.5 s. Post-merge full gate
   (build + clippy + workspace tests + pooled 12-suite PTY battery,
   lanes=4, REDLINE_BIN pinned to this tree): pooled battery **70.5 s**
   (pre-loop-03: ~117 s); sweep_flows leg 23.7 s; 12/12 suites OK —
   inside the spec's ~60–90 s battery target.


## loop-04 — Demote the remaining suites (kept / converted ledger)

**Shape (plan):** loop-03 demoted `sweep_flows`; loop-04 demotes the rest of
the "other" suites — `drive_windowing`, `drive_windowing_panes`, `drive_xref`,
`drive_external_notes`, `drive_external_crate`, `drive_external_use`,
`drive_syntax_notes`, `ux_sweep`. Same discipline: the state half moves to
store-level twins in `src/app/flow_tests.rs` (the batch-4 section; 20 new
twins, module 60 → 80, all green), the Python file keeps the thin terminal
tier — input encoding through the REAL PTY encoder, process liveness,
hardware-cursor placement, raw pixels — and one end-to-end smoke per drive
family. Provider-level resolution (real `cargo metadata` against the
registry) stays proven by the `redline-resolve` corpus; the store twins use
synthetic `ResolveEvent`/`CrateIndexEvent` for the landing/miss halves.

### Converted (suite file → thin tier + twins)

| Suite | Kept in PTY (thin tier) | Unit twin(s) (state half) |
|---|---|---|
| `drive_windowing.py` (28 steps → 21) | magit cursor-follow smoke: exactly-one-blue, help pinned, cursor in-window, window scroll observed | `unit_flow_win_magit_follow` (27-step store drive) |
| `drive_windowing_panes.py` (4 scenarios → 1) | commit-diff scenario (M-\>/M-< + C-n/C-p through the real terminal) | `unit_flow_panes_diff`, `unit_flow_panes_log` |
| `drive_xref.py` (6 legs → 1) | L1 same-file M-. jump (live-app proof of the jump + back-trail) | `unit_flow_xref_l1..l6` (L2 miss-report, L3 cross-file, L4 in-crate, L5/L6 fallbacks) |
| `drive_external_notes.py` (E1–E4 → E1+E1b) | landing into the registry source + the ownership guard (C-x C-q / C-x C-s refused on the external buffer) | `unit_flow_ext_notes_landing_guard`, `_annotate`, `_delete`, `_dump` |
| `drive_external_crate.py` (L1–L7 → L1) | registry landing (M-. on the top-level `ropey::Rope::new` probe) | `unit_flow_ext_crate_in_crate_mdot`, `unit_flow_ext_crate_imenu` |
| `drive_external_use.py` (→ L1) | the use-imported bare-symbol landing (`use serde::Deserialize;` scope hint, 007-03) | `unit_flow_ext_use_l1`, `unit_flow_ext_use_l2` (the bare-symbol-no-use degradation pin) |
| `drive_syntax_notes.py` (S0–S7 → S0–S3) | the anchor COMMIT path: open → point on the name → `A` → the on-disk record carries the syntax keys | `unit_flow_synleg_anchor_commit`, `unit_flow_synleg_reanchor` (S4–S7: the 100-line insertion + signature reformat re-anchor pass) |
| `ux_sweep.py` (13 fresh Apps → 2) | the per-key anomaly scan (unbound echoes, blank frames, overflow rows, off-screen cursor) in one 80-col session + one 40-col narrow session | `unit_flow_ux_keymap_coverage` (every key of every leg through `key_event`; the known-unbound C-x 2/1/0 parity-pinned) |

### Kept as-is (loop-04 scope fence)

`tools/sweep.py`, `probe_notes_dump.py`, `sweep_flows.py` (loop-03's thin
tier), `drive_all.py` (drives the other suites as subprocesses — picks up
their new shape automatically), `check_cursor_stream.py` (raw-stream pixel
class — terminal-tier by nature). NO changes to `store.rs`, `pool.py`, or
the providers in this loop.

### Measurements (this box, 0.06 quiet, REDLINE_BIN pinned, lanes=4)

- `cargo test --workspace`: all green (main bin 717 + redline-resolve 110
  + integration suites, 0 failed).
- Pooled 12-suite battery: **12/12 in 82.7 s** (pre-loop-04: ~96 s per the
  task spec; the first post-conversion run was 106 s with the external-crate
  bug below — fixed same session); every converted thin tier passes
  (drive_windowing 5.4 s, panes 3.6 s, xref 4.8 s, syntax-notes 3.9 s,
  external-notes 4.7 s, external-crate 3.8 s, ux_sweep 20.7 s).

### Resolved mid-loop (was the 2 BADs)

`drive_external_crate.py` (and the same leg inside `drive_all.py`) failed
1/3: the in-app "resolve job" appeared to hang 60–170 s with an empty
minibuffer, while the identical resolve completed in ~0.9 s standalone.
Root cause (found by A/B against the branch-base suite): **the thin-tier
rewrite dropped the `app.key("M-.", 0.5)` keypress** — after `M-f` it went
straight to the poll loop, so nothing was ever triggered (the window never
jumped; the L1 "Rope in view" check false-positived because the probe file
itself contains `Rope`). It looked like an in-app provider hang because the
status line stayed idle and the weak third check passed. Fix: restore the
single `M-.` keypress; battery 10/12 → 12/12 (82.7 s).

## Merge gate discipline (2026-09-20)

After EVERY merge: `cargo build` + `cargo test --workspace` +
`cargo clippy --workspace --all-targets -- -D warnings` — test-only
verification let a go-lane dead-code lint go red on main between merges
(caught by the setext-fix lane's gate; fixed `b2eb424`). Always
`cargo build` before a PTY battery (`pool.py` does not rebuild; a stale
binary bit the session twice).


## loop-03 (2026-09-20): demote the test pyramid — DONE

- `render_at_width(store, 80)` + `StaticRenderWidth` (root.rs): the layout-correctness class is statically catchable (`render_at_width_catches_offscreen_picker_count_line`, proven by removing df95113's pins).
- 60 store-level unit twins in `src/app/flow_tests.rs` (child module); all converted twins keep the positive signals; no absence-style weakening (review-sampled 20+).
- sweep_flows reduced to a 15-record thin PTY tier: 15/15 in 23.7s (was 65/65 ~87s). Kept: input encoding, repaint races (ann-delete class, U-BHN), quit lifecycle, boot smoke, watcher smokes, in-flight search cancel.
- Pooled battery: 70.5s (was ~117s; serial 0.2-era was 524s at session start). Reviewer: PASS, no verdict loss on any sampled twin.
- Deferred follow-up (scheduled): demote drive_windowing/panes, drive_xref/external suites, ux_sweep — all state+text, still run in the battery.

## Cross-lane PTY contention (2026-09-20, open)

Two lanes running serial `gate.sh full` concurrently can bleed renders
through the SHARED `/tmp/redline_pyte_repo` fixture (hint-rel saw another
lane's content in mismatched renders; py-roots hit exit-3 refusals +
backlog-#12 stray-file residue). The flock serializes suites, but two
concurrent GATES each run their own suite sequence against the same
fixtures — the quiet-window wait is the only documented mitigation.
Gap: every battery (not just pooled) should use private per-lane
fixtures (`REDLINE_POOL_ROOT`) or the fixture repos need per-lane copies
at the driver level. Spec'd for the next harness lane.
