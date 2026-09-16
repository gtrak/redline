---
name: knowledge-refinement
description: >-
  Goal-directed knowledge refinement system. Use when the user needs to
  investigate a technical domain, build a structured knowledge base, answer
  "can we safely migrate/deploy/change this?" with tracked confidence, or
  surface evidence gaps. Runs `kg investigate` which autonomously searches
  the repo corpus, audits, links evidence, refines claims, and compiles an
  assessment — all in-process with parallel subagents. Triggers: "investigate this domain",
  "assess this migration", "find evidence for/against", "compile an
  assessment", "what do we know about X".
allowed-tools: Bash(kg:*), Read, Write, Edit
---

# Knowledge Refinement Skill

Investigates a technical domain autonomously. Seeds a knowledge base from
analysis documents, then runs `kg investigate` — which dispatches in-process
agents to search a repo corpus, record observations with code-anchored
evidence, link evidence, audit claims, propose refinements, and compile a
confidence-tracked assessment. Long-running commands are budget-bounded and
resumable.

## Your role

You are the **orchestrator**. You do NOT search repos, observe findings, link
evidence, or audit claims yourself. `kg investigate` does all of that internally
by dispatching isolated subagents. Your job:

1. Write (or point to) a seed analysis document.
2. Initialize and seed the knowledge base.
3. Plan the goal.
4. Run `kg investigate`.
5. Read the compiled assessment.
6. Report to the user.

## Workflow

```bash
# 1. Seed
kg init                                    # if no knowledge base yet
kg ingest ./analysis/ --goal <goal-id>     # extract claims + observations
                                           #   --goal filters to the goal's
                                           #   desired_outcome + success_criteria

# 2. Define the goal (hand-author or use a template)
# knowledge/goals/<goal-id>.md with desired_outcome + success_criteria

# 3. Score claim relevance
kg plan <goal-id>

# 4. Run the autonomous investigation
kg investigate <goal-id> --max-rounds 5
#   concurrency is configured via [concurrency] max_concurrent
#   in .knowledge/config.toml (default 4)

# 5. Read the assessment
cat knowledge/artifacts/<goal-id>-assessment.md
```

## Repo corpus search (new in the investigate loop)

The investigation is **retrieval-first**: each dispatched subagent can call a
`search` tool that searches the configured repo corpus (ripgrep-backed) before
observing anything, and records **code-anchored evidence** (repo, path, line
range, git SHA) so findings are verifiable against the working tree.

- Point the corpus at your repos with `KG_CORPUS_ROOT` (a directory of cloned
  repos) or the `corpus` section in `kg.toml`.
- Search the corpus interactively with `kg search <query> [--regex]
  [--type <glob>] [--repo <name>] [--context N] [--max N]`.
- Check that code-anchored evidence still matches the working tree with
  `kg verify [<evidence-id>]` / `kg verify --all` — drift (moved lines,
  changed code) is flagged so stale evidence is not treated as fresh.

## Budgets and resumability

`ingest`, `plan`, `link_evidence`, and `investigate` are bounded:

- `--max-llm-calls N` / `--max-wall-secs S` — stop after N LLM calls or S
  wall-clock seconds (defaults 500 calls / 3600s from config).
- `--resume` — continue from the last progress marker instead of starting over.
- Environment overrides: `KG_BUDGET_MAX_LLM_CALLS`, `KG_BUDGET_MAX_WALL_SECS`,
  `KG_BUDGET_MAX_TOKENS`.

## When to intervene manually

The autonomous loop handles everything, but you can inspect/intervene:
- `kg status` — see knowledge base counts + stale plans
- `kg explain <claim-id>` — inspect one claim's state
- `kg weak --goal <goal-id> --json` — see what's still weak
- `kg audit --all` — re-run audit manually
  - `--goal <goal-id>` — rank gaps by goal relevance
  - `--summary` — collapsed gap classes (unverified vs contested, etc.) sorted
    by weight instead of per-claim detail
- `kg search <query>` — look directly in the repo corpus
- `kg verify --all` — check code-anchored evidence against the working tree
- `kg dedup [--write] [--threshold 0.85]` — dry-run near-duplicate claim merges
- `kg serve` — browse the knowledge base as an HTML site with a graph tree
- `kg history <claim-id>` — see how a claim evolved

These are read-only inspection commands (except `kg dedup --write` and
`kg audit --write`, which mutate). The orchestrator never needs to write
observations, link evidence, or audit inline.

## Flags

- `--max-rounds N` — hard cap on audit→dispatch→re-audit rounds (default 5)
- `--max-tool-calls N` — tool calls per subagent before it must return (default 10)
- `--breadth N` — max claims dispatched per investigation round (overrides `limits.investigate_breadth`)
- `--max-llm-calls N` — budget: stop after N LLM calls (default 500)
- `--max-wall-secs S` — budget: stop after S wall-clock seconds (default 3600)
- `--resume` — continue from the last progress marker
- `--no-tui` — structured stderr logs instead of interactive TUI (auto-detected from TTY)
- `--debug` — stream LLM tokens to stderr

## Environment

- `KG_LLM_ENDPOINTS` — comma-separated vLLM endpoints for round-robin across parallel subagents
- `KG_LLM_API_KEY`, `KG_LLM_MODEL`, `KG_LLM_BASE_URL` — single-endpoint fallback
- `KG_CORPUS_ROOT` — directory of cloned repos the `search` tool searches during investigation
- `KG_BUDGET_MAX_LLM_CALLS`, `KG_BUDGET_MAX_WALL_SECS`, `KG_BUDGET_MAX_TOKENS` — budget overrides
- `KG_DEDUP_THRESHOLD` — similarity threshold for `kg dedup` (default 0.85)
- `KG_LIMITS_*` — per-source limits (claims/observations per source, evidence per claim, search hits per step, investigate breadth/depth)
- `[concurrency] max_concurrent` — global concurrency for parallel LLM work across ingest extraction, investigation subagents, and link-evidence calls (default 4, in `.knowledge/config.toml`; the per-command `--parallel` flags were removed)