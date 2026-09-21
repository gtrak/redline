# issue-index-file-budget — no single file may own startup

## Why

The user's profiler run on their real project (10,150 files, release, 32 cores)
measured a cold index of **56,220 ms**, of which **55,351 ms (98.5%) was ONE
file**: an 11.3 MB `.cpp` that produced **0 symbols**. Strip that file and the
whole project indexes in **869 ms**.

Two consequences follow, and both were misleading on first reading:

* The profiler's `parallelism: 1.8x of 32 — LOW` verdict is a **symptom**, not
  a fan-out bug. One rayon thread was stuck for 55 s while the other 31
  finished and idled (Amdahl). Without the outlier the CPU/wall ratio is ~32x
  — the pool saturates. The report should say so itself.
* There is **no per-file cap anywhere in the extract path**. Every parse in the
  workspace is an unbounded `parser.parse(source, None)`. So the worst case is
  unbounded: a project's startup is at the mercy of its worst single file.

Per-file timings are otherwise healthy: `extract_ms` p50 = 0.653 ms,
p90 = 5.412 ms, p99 = 28.507 ms.

## RESOLVED CONTEXT (do not be misled by the paragraph above)

The pathological file was **not a legitimate project file**. The user's
`node_modules` was not in the project's `.gitignore`, so the walk indexed the
entire dependency tree (3,664 `.cpp`/`.hpp`, 698 `.c`/`.h`, one generated
11.3 MB C++ blob). With `node_modules` excluded, the same project indexes in
**368 ms** — 153x faster — walking 3,596 files, and its **legitimate** per-file
distribution is: `extract_ms` p50 = 0.541, p90 = 5.512, p99 = 32.686,
**max = 98.049 ms** (largest file 1.5 MB), with parallelism 29.4x of 32 cores.

So the constants in item 2 must be sized as a **safety net that never fires on
a legitimate file** — generously, such that a legitimate ~10 MB source file
passes — with the goal of bounding the worst case, not of optimising any
particular project. Do NOT tune them to the 55-second outlier. The disclosure
of the *cause* of that outlier is a separate issue
(`issue-dependency-dir-guard.md`).

## Required

1. **A per-file deadline covering BOTH parse and query execution.** Query
   execution was the larger half in aggregate (54,632 ms) vs parse
   (25,560 ms), so a parse-only budget would leave the bigger exposure
   uncovered. Both seams exist in the pinned `tree-sitter = 0.25.10` and were
   verified in its source (`binding_rust/lib.rs`):
   * `Parser::parse_with_options(text, old_tree, Some(ParseOptions::new()
     .progress_callback(&mut cb)))` — `cb: FnMut(&ParseState) -> bool`;
     returning `true` cancels and `parse` yields `None`.
   * `QueryCursor::matches_with_options` / `captures_with_options` with
     `QueryCursorOptions::new().progress_callback(&mut cb)` —
     `cb: FnMut(&QueryCursorState) -> bool`.
   The budget is **one deadline per file**, shared across parse + queries, so
   a file cannot spend its allowance twice.

2. **Size-aware default, with the constants justified in the commit message.**
   A flat budget is too blunt: the 8.7 MB JavaScript file is *legitimate*
   (12,249 symbols in 1,019 ms), while the 11.3 MB `.cpp` is pathological
   (55,351 ms, 0 symbols) — both ~10 MB. Observed legitimate rates top out near
   0.4 ms/KB (cpp) and 0.12 ms/KB (js); the pathological file is 4.9 ms/KB,
   ~12x the worst legitimate rate. Choose a `base + rate * size_kb` allowance
   that keeps every legitimate file measured above and aborts the pathological
   one, and state the reasoning.

3. **An aborted file is a first-class, VISIBLE outcome** — never a silent
   zero-symbol file. The extraction report must distinguish `aborted` from
   `zero-symbol`, and the profiler must print the count alongside the budget
   that produced it, so the user can tell that symbols are missing and why.

4. **Do NOT apply the budget to the interactive paths.** Highlighting/token
   extraction for a file the user opened may take as long as it takes —
   silently handing them a half-parsed buffer is worse than a slow paint.
   Scope: the index extraction path (`extract_all` / `extract_all_timed`) and
   its callers.

5. **Diagnostics that localize the next fix.** Keep the aggregate
   parse/query/compile split, and add:
   * **per-file parse-vs-query columns for the top-N** — the user's
     pathological file needs these to know WHICH half owns its 55 s (that
     decides whether the next fix is a smarter skip, e.g. not running
     definition queries over a tree riddled with ERROR nodes, or purely the
     budget);
   * a **dominance** figure (`max extract_ms` as a share of total wall), so the
     LOW-parallelism verdict explains itself instead of reading like a fan-out
     bug.

## Acceptance

* A test proving cancellation works end to end with a **tiny** budget: a
  ~0-1 ms budget on a non-trivial generated source must yield an aborted file
  (no tree / 0 symbols), must not panic or hang, and the surrounding
  extraction must complete normally.
* A test proving a **normal** file is unaffected by the default budget (it
  still yields its usual symbols).
* A test that the budget is **shared**, not per-phase — a file cannot get 2x
  its allowance by splitting across parse and queries — or, if the
  implementation makes that structurally impossible, say so and pin it the
  cheapest way that actually discriminates.
* The profiler prints the aborted count, the effective budget, the per-file
  parse/query columns for the top-N, and the dominance figure.
* `cargo test --workspace` + clippy `-- -D warnings` clean; `tools/gate.sh
  full` green.

## Fence

* `crates/redline-syntax/src/queries.rs` (the extraction engine +
  `extract_all_timed`), `crates/redline-syntax/src/registry.rs` (or wherever
  the per-language handles live, if a signature must change),
  `src/index_profile.rs` (reporting), and the minimal wiring in
  `src/app/store/index_wiring.rs` / `src/main.rs` if the budget must reach the
  extraction call.
* `src/ui/` and the interactive highlight/token paths are OUT of fence.
* Disclose any file touched outside the fence with before/after.

## Honest stop

At half budget: land the budget + aborted reporting + their tests, and leave
the profiler diagnostics (item 5) for a follow-up — say so rather than
half-doing both.

## NOT this lane

The profiler's `total - walk` line currently reads "what a WARM PERSISTED INDEX
would cost to start", which is **backwards** (`total - walk` = parse +
assembly = exactly what persistence would SAVE). The orchestrator is fixing
that label directly. Do not touch it.

## Follow-ups from the gate (all disclosure-class, no behaviour change)

- **P2-3 — `aborted` is invisible in the index.** It is only observable when
  `timings.is_some()`, and the production indexer (`extract_all`) passes `None`, so in the
  store an aborted file is **indistinguishable from a zero-symbol file**. The
  "first-class outcome" requirement holds in the profiler report but not where it matters
  most. Fix: count aborted files in `index_wiring` and surface the count (the same
  disclosure surface `issue-dependency-dir-guard.md` needs — consider doing both in one
  place, since "we silently dropped symbols" and "we silently indexed a dependency tree"
  are the same class of surprise).
- **P2-4 — the printed budget is the formula, not the per-file value.**
  `ExtractTimings::budget` is recorded but never printed, so the report says
  `base 500 ms + 2.0 ms/KB` when the file actually got `base + 2.0 * its_own_kb`. The
  effective value is derivable from the row's size column, but printing
  `r.stages.budget` on the aborted row is strictly better.
- **Latent (not a gate finding) — `extract_symbols` inherited the budget.**
  `extract_symbols` is `extract_all(...).0`, so it is budgeted by default. It has **no
  production caller today** (only `src/app/store/tests/index_wiring.rs`), so there is no
  regression — but a future *interactive* caller of that convenience wrapper would
  silently become budgeted, which spec item 4 forbids. Either make the budget explicit in
  its signature or note the hazard on the wrapper.

## What the budget does NOT do (state this whenever it is discussed)

It **bounds** the pathological case; it does not **fix** it. The 11.3 MB outlier stops at
23.6 s, so the original cold index becomes ~24.5 s instead of 56.2 s — still ~66x the
368 ms the same project takes once `node_modules` is excluded. The cause was a
non-gitignored dependency tree; the real fix is `issue-dependency-dir-guard.md`. On a
legitimate project the budget never fires (largest legitimate file 98 ms against a 3.4 s
allowance; 0 aborted expected of 3,596), which is the property that matters.
