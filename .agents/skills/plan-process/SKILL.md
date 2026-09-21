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

- A lane whose fence excludes a test file **may ask for the fence to be
  widened** instead of shipping a partial result; a fence is an implementation
  detail of the task spec, not a requirement.
- Reviewers must distinguish "the test pins this" from "the requirement needs
  this", and must not raise a P1 merely because an implementation-detail
  assertion would need updating.
- When a change makes an old assertion describe the wrong thing, re-pin it on
  the *requirement* (the minibuffer message, the invariant), never on a screen
  substring the fixture itself contains.

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
