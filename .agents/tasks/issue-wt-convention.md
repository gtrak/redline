# Task: lane-worktree convention in the plan-process skill (docs)

You are the implementation worker. Repo root is your cwd. Self-contained.

## Origin (user directive, 2026-09-20)

A session OOM'd the build box because agent lane worktrees accumulated in
/tmp (which filled up). The user's decision: **lane worktrees live in
`.agents/worktrees/` as NUMBERED SLOTS**, they are **removed once merged**,
and slots are **not tied to a task** (they are reusable isolation
resources). Document the convention in
`.agents/skills/plan-process/SKILL.md` (the skill that owns locations and
artifacts) and make the repo's `.gitignore` reflect it (already committed
on main by the orchestrator: `/.agents/worktrees/`).

## What to write (a new section in the skill, after "Locations")

**Lane worktrees (numbered slots)**:
- **Location**: `.agents/worktrees/<N>` for N = 1..**3** (the cap is
  memory-driven: each lane builds its own `target/`; three concurrent
  Rust workspace builds is what exhausted the box — raise only for
  read-only lanes or non-building lanes). The path is deliberately FIXED
  length and task-agnostic; `.gitignore` covers `/.agents/worktrees/`;
  the project index skips hidden dirs so slots are never self-indexed.
- **Slot ↔ lane mapping**: each occupied slot carries a `.lane` marker
  file (branch, purpose/lane name, session id, state: active|parked) so a
  stale slot is never anonymous. The plan's lane board records the slot
  path before the first mutation (the existing rule).
- **Lifecycle**:
  - Acquire: pick the lowest free slot; verify it is empty/clean/prunable
    (`git worktree list`, `git worktree prune`); if occupied, that is a
    leak signal — resolve it, never clobber.
  - Release: a slot may be freed when its branch has **no uncommitted
    changes**. Merged → remove the worktree AND delete the branch. Unmerged
    (abandoned/interrupted) → keep the branch, free the slot, record why in
    the lane board/task file; WIP must be committed first (the checkpoint
    pattern).
  - Deleted on release: the slot's `target/` goes with the checkout (or is
    removed explicitly) — never a shared `CARGO_TARGET_DIR`, which would
    serialize parallel builds on cargo's lock.
- **Session-boundary audits** (the process that keeps this honest):
  - Start of session: `git worktree list` + `git worktree prune`; decide
    every leftover slot (resume, park, or release).
  - End of mission: every slot is terminal — merged+released, or parked
    with a named owner and next action.
- **Why slots, not branch-named paths**: fixed path length (the pool/fixture
  suites have path-length and per-lane-fixture constraints), trivial reuse,
  and the slot count IS the concurrency cap.

## Constraints

- Docs only: `.agents/skills/plan-process/SKILL.md` (the new section; keep
  the skill's existing voice/format — it is terse and imperative).
- Optionally one cross-reference line in the "Cross-references" section
  pointing orchestration mechanics at the subagent-orchestration skill
  (unchanged).
- Budget ~10 tool calls. Commit on your branch. Report: the section text
  summary, any skill-correction notes, deviations.
