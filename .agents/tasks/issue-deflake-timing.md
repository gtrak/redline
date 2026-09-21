# Task: deflake the timing-sensitive gate legs

## Why (this is gate reliability, not cosmetics)
A gate that reports FAIL on a **frozen tree** destroys the meaning of "gate-green" —
every future failure has to be re-litigated. Two load-correlated flakes are now
characterised, both with a *self-contradictory* failure shape (which is what argues
"timing" rather than "logic"):

1. **`tools/check_cursor_stream.py`, the `M-b x5` leg** — failed once in a full gate run
   where only the **first** backward-word press read a `None` cursor; green in isolation
   and in the next full run. Mechanism: the leg reads the cursor before the first frame
   has been drawn (first-frame timing).
2. **`git::repo::tests::unstage_hunk_on_no_trailing_newline_file_keeps_index_exact`**
   (`src/git/repo.rs`) — failed **2 of ~11** full-suite runs on a branch (0/4 on main),
   always mid-suite under peak load, with a byte-exact index-blob assert **passing**
   while a later `git diff --cached` still showed the unstage target. 9 consecutive
   green runs afterwards.

Root cause hypothesis for the cluster: three lanes building + running PTY suites in
parallel create the peak load that exposes these races.

## Item 1 — `check_cursor_stream.py`
Read the `M-b x5` leg and the surrounding harness. Replace the implicit assumption
("the cursor is available immediately") with a **positive gate**: wait for a signal
that the frame has been drawn (or for the cursor to be non-`None`, bounded) before
reading it — the loop-03 doctrine: assert a positive signal, never merely absence, and
never a bare sleep as the only synchronisation. If the harness has an existing
"frame drawn"/quiet-settle helper, use it; say which.

## Item 2 — the `repo.rs` test
Determine which of these it is, and report:
- **(a) the test has an ordering/timing assumption** (e.g. asserting index state right
  after a mutation without letting git2 refresh) → make the assertion deterministic
  (poll/retry the *observation*, not the operation) and explain why that does not hide
  a real failure;
- **(b) the production code has a genuine race** in the index refresh/unstage path →
  that is a **bug**: stop, report it with the evidence, and do not paper over it with
  test retries.
The contradictory shape (a passing byte-exact assert + a still-stale `git diff
--cached`) is the key clue — explain it either way.

## Rules
- **Do not weaken any assertion and do not add blanket retries.** A retry that hides a
  real failure is worse than the flake. If you add a bounded wait, it must be on a
  *positive* condition.
- Fence: `tools/check_cursor_stream.py`, `src/git/repo.rs` (**its test module only**;
  production code changes only if you find a genuine bug, and then report rather than
  patch). Nothing else.
- Honest-stop at half budget; Item 1 alone is a valid landing.

## Verification (this is the point)
Demonstrate the deflaking under **load**: run the affected drive/test **repeatedly
(≥10×) while the machine is busy** (e.g. concurrently with a full gate or a cargo
build) and report the pass counts. A single green run proves nothing about a flake.
Then run the full battery once.
**Resource guard**: `export CARGO_BUILD_JOBS=4`; if `free -g` "available" < 8 GB, run
the targeted loops only and report the full gate as deferred.
Budget ~40 tool calls. Report: the mechanism for each flake, the fix, the ≥10×
under-load evidence, and whether you found (a) or (b) for Item 2.
