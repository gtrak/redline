---
name: plan-process
description: >-
  Planning pattern for feature development: active plan structure, per-issue
  task specs, issue file format, and archival process when a plan is
  implemented.
---

# Plan Process

The planning workflow for feature development. Each plan lives in a numbered
folder with a `PLAN.md` overview and broken-down issue files, then gets
archived once implemented. Each issue is executed by a worker subagent from a
**task spec** — a self-contained, repo-side file that operationalizes the
issue.

## Locations

- **New plans**: `.agents/plans/NNN-short-name/` (since plan 001; older plans
  may remain in `docs/plans/` — numbering scans both plus both archives).
- **Task specs**: `.agents/tasks/` — one `issue-NNN-impl.md` per issue plus
  the shared `review-gate.md`.
- **Archives**: `.agents/plans/archive/NNN-short-name.md` (legacy:
  `docs/plans/archive/`).

### Plan numbering

Scan `.agents/plans/`, `docs/plans/`, and both `archive/` folders for all
existing numbers, take the highest `NNN`, and add 1:

```bash
ls .agents/plans/ docs/plans/ 2>/dev/null | grep -E '^[0-9]+' | sed 's/-.*//' | sort -n | tail -1
```

## Lane worktrees (numbered slots)

Reusable isolation slots for parallel agent sessions. Fixed numbered paths
— not branch-named — so pool/fixture constraints (path length, per-lane
fixtures) stay deterministic and the slot count **is** the concurrency cap.

### Location

`.agents/worktrees/<N>` for N = 1..**3**. The cap is memory-driven: each
lane builds its own `target/`; three concurrent Rust workspace builds is
what exhausted the box. Raise only for read-only or non-building lanes.
`.gitignore` covers `/.agents/worktrees/`; the project index skips hidden
dirs so slots are never self-indexed.

### Slot ↔ lane mapping

Each occupied slot carries a **`.lane`** marker file (branch,
purpose/lane name, session id, state: `active`|`parked`) so a stale slot
is never anonymous. Record the slot path in the lane board before the
first mutation (the existing rule).

### Lifecycle

**Acquire** — pick the lowest free slot. Verify empty/clean/prunable
(`git worktree list`, `git worktree prune`). If occupied, that is a leak
signal — resolve it, never clobber.

**Release** — a slot may be freed when its branch has **no uncommitted
changes**.

- Merged → remove the worktree **and** delete the branch.
- Unmerged (abandoned/interrupted) → keep the branch, free the slot,
  record why in the lane board/task file. WIP must be committed first
  (the checkpoint pattern).

On release the slot's `target/` goes with the checkout (or is removed
explicitly) — **never** a shared `CARGO_TARGET_DIR`, which would
serialize parallel builds on cargo's lock.

### Landing (squash, no merge commits)

Land a lane by SQUASHING its branch into ONE commit on main — no merge
commits (user directive, 2026-09-20):

```bash
git merge --squash <branch> && git commit   # then: git branch -d <branch>
```

The squashed commit message carries the lane's story (what changed, the
gate evidence, the deviations). Consequences to plan for:
- The lane's intermediate commits (WIP checkpoints, review-fix rounds) drop
  off main's history; keep the branch (do not delete) when the work is
  likely to be reverted/re-landed, e.g. an experiment the user asked to
  park. Otherwise delete it per the release rule.
- Reverting a landed lane is `git revert <squash-sha>` (no `-m 1`
  mechanics), and re-landing is reverting the revert.

### Session-boundary audits

- **Start of session**: `git worktree list` + `git worktree prune`;
  decide every leftover slot (resume, park, or release).
- **End of mission**: every slot is terminal — merged+released, or parked
  with a named owner and next action.

## Test authority (user directive, 2026-09)

**This is greenfield. A test never vetoes a requirement.**

Tests are evidence and a thinking cue, not an authority. When a test blocks a
change, decide what the test actually encodes and act accordingly:

1. **Implementation-level assertion** (a contiguous rendered substring, an
   internal field, a helper's exact wording, a specific line number): the
   requirement wins — update the assertion and say so in the report. Do not
   report it as a blocker, and do not ship a degraded deliverable because
   "the test pins it".
2. **Requirement-level assertion** (a behaviour the user asked for, a
   safety/ownership guard, a data-integrity invariant): keep it. If the
   requirement genuinely conflicts with it, **escalate to the user** — that is
   the one case where a decision belongs above the agent.
3. **Genuinely unsure**: escalate with the specific assertion quoted and a
   recommendation, rather than silently degrading the deliverable or silently
   rewriting the test.

Practical consequences for lanes:

- **Dispatch gates to `gate-reviewer`, not builtin `reviewer`.** The builtin
  reviewer is granted only `read, grep, find, ls` — it has **no `bash`**, so it
  cannot run `cargo test`, clippy, or `tools/gate.sh`, and its numeric verdicts
  are read-only inference. A gate that cannot execute is not a gate. The
  `gate-reviewer` agent (project `.pi/agents/`, user `~/.pi/agent/agents/`) adds
  `bash` and must report the commands it ran with observed output. Read-only
  review is fine for docs-only deliverables; anything touching code needs
  execution.
- A lane whose fence excludes a test file **may ask for the fence to be
  widened** instead of shipping a partial result; a fence is an implementation
  detail of the task spec, not a requirement.
- Reviewers must distinguish "the test pins this" from "the requirement needs
  this", and must not raise a P1 merely because an implementation-detail
  assertion would need updating.
- When a change makes an old assertion describe the wrong thing, re-pin it on
  the *requirement* (the minibuffer message, the invariant), never on a screen
  substring the fixture itself contains.

### Maintainability criterion (user directive, 2026-09)

**Test file size is not a criterion. The criterion is: is complex logic organized
for maintainability?**

A line count is a *proxy*, and it is the wrong proxy for this codebase: a 3,000-line
file of flat, cohesive tests is fine, and a 150-line function with 8 levels of
nesting is not. Judge by:

- **Function shape** — length, nesting depth, argument count, and whether the name
  still describes what the body does (a 151-line predicate named `is_path_segment`
  is a naming failure, not a size failure).
- **Separation of jobs** — a function that extracts state *and* runs an effect *and*
  dispatches *and* assembles output is four functions wearing one name.
- **Data expressed as code** — hundreds of imperative `bind(...)` statements inside
  a constructor are configuration that wants to be a table (the `LanguageSpec`
  precedent), because adding an entry should be a one-line data edit.
- **Shared shape vs. coincidence** — per-language logic that repeats a structure is
  a candidate for a trait/table with per-language hooks; logic that genuinely
  differs is not.

Consequences: a plan's "no file over N lines" criterion applies to **production
logic**, not to test files; a large test file is not a finding. When a lane reports
a big test file as a judgement item, the answer is "not a criterion" unless the
tests themselves are hard to navigate.

### Numbers in briefs are claims, not facts
When relaying a reviewer's arithmetic (counts, line spans, test totals) into a
worker's brief, label it **"reviewer-reported — re-derive before relying on it"**.
A propagated wrong number is worse than no number: it becomes the spec. Observed
in 012-01, where a review's method/test/line figures were passed into a fix brief
as fact and **six of them were wrong** — the fixer caught them only because the
brief also demanded an independent re-derivation. Corollary: a reviewer's
*findings* (a missing assignment, a non-disjoint range, a broken `#[path]`) are
high-value and should be relayed; its *arithmetic* must be re-measured.

### The orchestrator's own counts are the most error-prone numbers of all

Three times in one session a count in a task spec — *my* number, not a worker's —
was wrong, and each time a lane caught it only because the spec ordered a
re-derivation first:
- "48 methods / 8 `js_ts_*` / 4 `go_*`" in the A7 brief → really **51 / 7 / 3**;
- "79 distinct commands" in the A2 brief → really **99**;
- a "no file over 1,500 lines" criterion applied to files the criterion never meant.

The mechanism is always the same: an ad-hoc `grep`/`awk` scoped to a **line range**
or a **single function** (`awk 'NR>=237 && NR<=521'`) silently excludes the rest of
the artifact — in the A2 case `AppStore::at`'s 22 global binds, which is exactly
where the missing 20 commands lived. The fix is procedural, not diligence:

1. Measure the **whole artifact** (the file, the module, the tree) — never a line
   range you chose by eye.
2. Prefer a **deterministic tool** (`tools/cleanup_scan.py`) or a compiler over a
   hand-rolled regex; brace-matching beats line windows.
3. Always label spec numbers **"re-derive before relying on it"**, and require the
   re-derivation *first*, before the lane acts on anything.
4. Treat a spec's counts as claims even when the spec's *map* (which symbol is
   where) is exact — a correct map with wrong totals is the observed failure mode.
5. **A measurement tool that strips code must preserve line numbers.** The first
   version of the function survey stripped comments and string literals for brace
   matching *and deleted the newlines inside them*, so every reported `file:line`
   drifted earlier by that count — in `src/syntax/queries.rs` (test fixtures full of
   multi-line raw strings, 227 newlines inside literals) by ~115 lines: it claimed
   `extract_all` was at `:360` when it is at `:475`. **Both the locations AND the sizes
   were wrong** — I first wrote "sizes were right, locations were wrong", and that was
   itself wrong: a function whose body contains a multi-line string also measures
   *shorter*, because those newlines were deleted. `xref_find_definitions` measured 177
   when it is 189 (the worker's independent count agreed with 189, which is what
   exposed the error). A spec that says "this function is N lines at file:line" must not
   be wrong about either number. Fixed and
   promoted to a real tool: `tools/fn_survey.py` (newlines preserved, `#[cfg(test)]`
   and `tests/` excluded, `--min`/`--nest` modes). Use the tool, not an ad-hoc heredoc;
   it is the deterministic measurement behind the logic-organization agenda.
6. **A baseline copied into a spec goes stale the moment another lane lands.** A spec
   written before lane X landed carries X's *pre*-X test counts; the next lane then
   reports a "delta" that is really the spec being out of date (observed: A2 added one
   test, so the A1 spec's "854" was already 855). Either quote the baseline as
   "current at spec time" and tell the lane to re-read it, or leave the number out and
   require the lane to state the baseline it measured.

### A line number is not evidence of which function it is in

Distinct from a miscounted number: a spec claimed a defect from a `rg` hit plus an
*earlier, unrelated* read. The evidence was real (`navigation/mod.rs:73` does call
`set_point_line`), but the spec asserted that line was inside `navigate_to_entry` and
therefore that "the recorded column is never restored" — complete with a doc it
claimed was contradicted. It was inside `open_resolved_source` (whose target has no
column, so col 0 is correct), and `navigate_to_entry` had restored the column since a
much earlier commit. The lane caught it with `git log -L` / `git log -S`, i.e. it
checked whether the defect had *ever* existed rather than reasoning about the
present.

Rules that follow:
1. **Read the enclosing item.** A line number from `rg` tells you nothing about the
   function, impl, or block it sits in; open the file at that line before attributing
   behaviour to it.
2. **To claim "X is contradicted by the code", quote the code that contradicts it** —
   not a line number that plausibly does.
3. **Use history to test a defect hypothesis**: `git log -L <fn>:<file>` and
   `git log -S <string>` answer "did this ever behave differently?" far more cheaply
   than reasoning from the current snapshot. A hypothesis that the code never had the
   bug is a hypothesis worth one command.
4. A claim of the form "and the same defect affects Y" **doubles** the blast radius of
   a mistake. Either verify Y the same way as X, or label it explicitly as unverified.

### A pin must read the same source of truth as the code it guards

A pin that asserts an invariant via a *copy* of the fact gives **false
assurance** — worse than no pin, because it looks like coverage. Observed in
012's C13: the invariant ("a reuse language must have no locals query") was pinned
against a hand-maintained `has_locals_queries` list, while the registry carried the
same fact *positionally* in its per-language `build()` arms. The drift the pin
existed to catch — editing `build()`'s arm — would not have failed the test.
Corollary for spec authors: when you ask for a pin, ask for the **single source of
truth** to be extracted first (one accessor that both the production code and the
test call), and require the discriminating experiment to go through **that** path.

## Pattern

### Active plan

```
.agents/plans/NNN-short-name/
  PLAN.md            # Overview: why, what, approach, scope, task order
  01-first-step.md   # Issue: objective, files, steps, verification
  02-next-step.md    # ...
```

**PLAN.md** — High-level design document covering:
- **Why** — problem statement and motivation
- **What** — technical approach and scope
- **Success criteria** — how we know it's done
- **Task order** — dependency graph for the subtasks

**Numbered issue files** — the *contract*: what the issue must accomplish.
- **Objective** — what this step accomplishes
- **Key decisions** — constraints and approach for this step
- **Files** — table of affected files and changes
- **Steps** — ordered implementation steps
- **Verification** — how to confirm correctness

Issue files stay stable during implementation; refinements go into the task
spec, not the issue file (the issue file is what the reviewer judges against).

### Task specs (.agents/tasks/)

Each issue gets a worker-facing **task spec** at
`.agents/tasks/issue-NNN-impl.md` — self-contained so a fresh-context worker
needs nothing else. It translates the issue file into execution terms and
always contains, in order:

1. **Context header** — which issues are committed vs in-flight; what to read
   from the existing codebase.
2. **Working agreement** (boilerplate, keep identical across specs):
   - *Skills are truth*: `.agents/skills/*.md` are authoritative — no
     `~/.cargo/registry`, no docs.rs, no fetching; a missing API detail gets
     a best-guess call consistent with the skill + a noted gap.
   - *Write-first*: scaffold all new modules in the first handful of tool
     calls, compile early, iterate on named errors.
   - *Skill corrections*: if code reality contradicts a skill file, code wins
     and the worker makes a minimal, factual skill-file correction, listed in
     the report under "skill corrections".
   - *graft available*: the graft CLI (`graft map/ask/callers/skeleton/grep`)
     is on PATH with a fresh graph — use for cross-file orientation; never a
     replacement for the read-first list.
3. **Read first** — ordered reading list: plan, issue file, relevant skill
   files, existing code to build on.
4. **Constraints** — dependency guardrails, layering rules, scope fences,
   explicitly deferred items (with the seam to leave behind).
5. **What to build** — per-file requirements, concretely.
6. **Verification** — gates (`cargo build`, `clippy -D warnings`,
   `cargo test`) plus per-feature tests; for anything interactive, scripted-
   PTY checks (open-frame, quit-exit-0, repaint-on-external-event); perf
   sanity stated as reasoning, not benchmark theater.
7. **Report format** — module map, design summary, gate outputs, skill
   corrections, deviations, known gaps.

The shared **`review-gate.md`** holds the reviewer protocol: input (issue
file = contract, skills authoritative, implementer report to sanity-check),
the 10-point checklist, and the verdict contract — non-blocking suggestions
first, then exactly one final line `VERDICT: PASS` or
`VERDICT: BLOCKING -- <numbered must-fix list, file:line>`.

Write the task spec when the issue is picked up (or just before), not at plan
time — it absorbs what earlier issues taught (accepted deviations, new
guardrails, carry-over review notes).

### Cross-references

- UX flows: anything interactive gets flow IDs in `docs/ux-testing-plan.md`;
  the task spec's verification should map to them.
- Orchestration (dispatching, budgets, review loop) lives in the
  `subagent-orchestration` skill — this skill only defines the artifacts.

### Completion

When a plan is fully implemented, consolidate `PLAN.md` + all issue files
into a single short summary (~20-30 lines), write it to
`.agents/plans/archive/NNN-short-name.md`, then remove the original folder.
The archive is a flat list of `.md` files — one per completed plan. Full
implementation details remain in git history.

## A fixture must contain the property that triggers the bug

**Learned twice in one session, from two different lanes, both times found by a gate.**
A test whose fixture lacks the very property the bug depends on will pass on the broken code
and keep passing after the fix — it is not evidence, it is decoration.

- The **gitignore agreement test** (`finder_and_search_agree_on_gitignore`) used a fixture
  with **no `.git`**. The walker branches on `root.join(".git").exists()`: without it the walk
  and the incremental filter share the *same helper*, so the test asserted agreement that was
  **structural** rather than behavioural — and could not see that in a real git repo the two
  paths disagree (the walk honours `.git/info/exclude` and global excludes; the filter did not).
- The **crate-source-files test** (`crate_source_files_node_modules_boundary`) used a non-git
  tempdir. `ignore::Walk`'s defaults include `require_git: true`, so with no `.git` the
  gitignore rules are inert; `node_modules` is also not hidden. So the test could not see that
  in a real git repo the project's `node_modules/` rule excludes the landed dependency entirely
  (via `parents: true`), nor that a hidden `.venv` is skipped by `hidden: true`.

**The check to run before believing a passing test:** name the property the bug needs (a `.git`
dir, a hidden component, a nested `.gitignore`, a non-ASCII byte, an ignored file that *changes*,
a batch that coalesces) and confirm the fixture actually has it. If the fixture would pass with
the bug present, say so and fix the fixture — **or write the test so it fails on the current
code first and prove that**, which is the only way to know it discriminates.

**Corollary for specs:** when a lane reports "N tests added", that is a count, not evidence.
Ask what each test would do on the unfixed code. A lane that says "the test fails on the old
code with `left: 0, right: 3`" has proved discrimination; a lane that says "5 new tests, all
passing" has not.
