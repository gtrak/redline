# Tree-sitter runtime bump 0.24.7 → 0.25.10 — compatibility matrix

Decision-free evidence for the runtime bump task
(`.agents/tasks/issue-runtime-bump.md`). Every requirement below was
read from the local crates.io registry index cache
(`~/.cargo/registry/index/…/.cache/tr/ee/<crate>`) — no network. The
empirical confirmation is the ABI guard
`all_grammars_set_language_succeeds` (registry.rs), run over ALL
`LanguageId::ALL` against the bumped lock.

## The mechanism (why the matrix looks the way it does)

Since the grammar crates' `tree-sitter-language` split (≥0.23-era
releases), a grammar crate's **normal** dependency is only
`tree-sitter-language = "^0.1"` (the `LanguageFn` ABI bridge; its
`links = "tree-sitter-language"` keeps one ABI-bridge version in the
graph). The grammar's `tree-sitter` (RUNTIME) dependency is a
**dev-dependency** in every pinned crate — dev deps are NOT resolved
when the grammar is consumed as a dependency, so they impose NO
constraint on which runtime the redline build links. What constrains a
grammar at link time is the runtime's ABI window:
`set_language` succeeds iff `MIN_COMPATIBLE_LANGUAGE_VERSION ≤
grammar ABI ≤ LANGUAGE_VERSION`.

- pinned 0.24.7 runtime: window 13..=14 (verified in its source,
  `binding_rust/bindings.rs`).
- 0.25.10 runtime: window 13..=15 (verified in its source after fetch).
- all 16 pinned grammar crates are ABI 13 or 14 today (that is why
  they load under 0.24.7 — the guard test) → ALL stay in-window under
  0.25.10.

## The matrix

| Crate (current pin) | runtime req (normal) | dev-only runtime req | loads on 0.25.10 (ABI window 13..=15)? | 0.25-gen release exists? | action |
|---|---|---|---|---|---|
| tree-sitter-rust `0.23.3` | tree-sitter-language ^0.1 | ^0.24 | yes (ABI 14) | 0.24.1 / 0.24.2 (dev ^0.25) | KEEP 0.23.3 |
| tree-sitter-javascript `0.23.1` | tree-sitter-language ^0.1 | ^0.24 | yes (ABI 14) | 0.25.0 (dev ^0.25.8) | KEEP 0.23.1 |
| tree-sitter-typescript `0.23.2` | tree-sitter-language ^0.1 | ^0.24 | yes (ABI 14) | no (0.23.2 is newest) | KEEP 0.23.2 |
| tree-sitter-python `0.23.6` | tree-sitter-language ^0.1 | ^0.24 | yes (ABI 14) | 0.25.0 (dev ^0.25.8) | KEEP 0.23.6 |
| tree-sitter-go `0.23.4` | tree-sitter-language ^0.1 | ^0.24 | yes (ABI 14) | 0.25.0 (dev ^0.25.8) | KEEP 0.23.4 |
| tree-sitter-c `0.23.4` | tree-sitter-language ^0.1 | ^0.24 | yes (ABI 14) | 0.24.0–0.24.2 (dev ^0.25.4) | KEEP 0.23.4 |
| tree-sitter-cpp `0.23.4` | tree-sitter-language ^0.1 | ^0.24 | yes (ABI 14) | no (0.23.4 is newest) | KEEP 0.23.4 |
| tree-sitter-bash `0.23.3` | tree-sitter-language ^0.1 | ^0.24 | yes (ABI 14) | 0.25.0 / 0.25.1 (dev ^0.25) | KEEP 0.23.3 |
| tree-sitter-json `0.24.8` | tree-sitter-language ^0.1 | ^0.24 | yes (ABI 14) | no (0.24.8 is newest) | KEEP 0.24.8 |
| tree-sitter-yaml `=0.7.0` | tree-sitter-language ^0.1 | ^0.24 | yes (ABI 14) | 0.7.1 / 0.7.2 (dev ^0.25.4) | KEEP 0.7.0 |
| tree-sitter-md `0.3.2` | tree-sitter-language ^0.1 (+ OPTIONAL tree-sitter ^0.23, feature-gated, not enabled) | ^0.23 | yes (ABI 13/14) | 0.5.1 (optional ^0.24; 0.5.2+ want ^0.26) | KEEP 0.3.2 |
| tree-sitter-toml-ng `0.7.0` | tree-sitter-language ^0.1 | ^0.24 | yes (ABI 14) | no (0.7.0 is newest) | KEEP 0.7.0 |
| tree-sitter-java `=0.23.5` | tree-sitter-language ^0.1 | ^0.24 | yes (ABI 14) | no (0.23.5 is newest) | KEEP 0.23.5 |
| tree-sitter-c-sharp `=0.23.1` | tree-sitter-language ^0.1 | ^0.24 | yes (ABI 14) | 0.23.5 (dev ^0.25) | KEEP 0.23.1 |
| tree-sitter-ruby `=0.23.1` | tree-sitter-language ^0.1 | ^0.24 | yes (ABI 14) | no (0.23.1 is newest) | KEEP 0.23.1 |
| tree-sitter-scheme `=0.24.7` | tree-sitter-language ^0.1 | ^0.24.7 | yes (ABI 14) | no (0.24.7 is newest) | KEEP 0.24.7 |
| tree-sitter-clojure (new) | **tree-sitter ^0.25.6 (NORMAL)** + tree-sitter-language ^0.1.5 | — | 0.25.10 only (the whole point of the bump) | 0.1.0 is the only release | ADD `=0.1.0` |

## Classification

- **already 0.25-compatible: ALL 16 pinned grammars** — no crate has a
  normal (non-dev) runtime requirement, and every pinned grammar is
  ABI 13/14, inside the 0.25.10 window 13..=15.
- **needs a crate version bump: NONE.** No grammar is forced to move
  to stay on 0.25.10. (The "0.25-gen release exists" column records the
  optional follow-up lane — bumping e.g. javascript to 0.25.0 or
  c-sharp to 0.23.5 would change grammar binaries and therefore parses;
  that is a separate decision with per-grammar drift review, not part
  of this behavior-neutral runtime bump.)
- **CANNOT follow (lost language): NONE.** No BLOCKING-report needed:
  every current language survives the bump.
- **clojure 0.1.0**: hard normal dep `tree-sitter ^0.25.6` — resolvable
  exactly when the runtime is ≥0.25.6. This is the unblock.

## The chosen runtime version

`tree-sitter = "=0.25.10"` — the newest 0.25.x in the index, and the
version that simultaneously satisfies:

- `tree-sitter-highlight =0.25.10` (its `tree-sitter` req is exactly
  `^0.25.10` — the highlight crate family tracks the runtime 1:1;
  0.25.9's req is `^0.25.9` and would strand the lock one below the
  newest),
- `tree-sitter-clojure =0.1.0` (`^0.25.6`),
- every pinned grammar (no runtime constraint at all).

One runtime in the lock (the ABI-pinning rule, 001/007 lesson):
`cargo update` must move exactly `tree-sitter`, `tree-sitter-highlight`
(and, after the Clojure landing, `tree-sitter-clojure` + its build
deps) — no second runtime, no fork.

## Recorded optional follow-up: 0.25-gen grammar releases (deliberately NOT taken)

The "0.25-gen release exists?" column above records, per pinned grammar,
the newer releases that exist: rust 0.24.1 / 0.24.2, javascript 0.25.0,
python 0.25.0, go 0.25.0, c 0.24.0–0.24.2, bash 0.25.0 / 0.25.1, yaml
0.7.1 / 0.7.2, c-sharp 0.23.5 (plus md 0.5.1 — optional `^0.24`, 0.5.2+
want `^0.26`). The ts-bump deliberately took **none** of them: each such
bump changes the grammar *binary*, and therefore parses, highlights,
and goldens. That is a per-grammar drift decision requiring its own
evidence + review (per-grammar drift decision), not a behavior-neutral
runtime change. Recorded here (with the Classification note above) so it
is not re-filed as an oversight — if a lane wants one, it is its own
task with per-grammar drift gates, not a ride-along on a runtime bump.

## Behavioral neutrality claim

Grammar binaries are UNCHANGED (all 16 pins kept byte-for-byte), so
every parse, highlight event, node predicate, and golden is
byte-identical to the 0.24.7 tree — the only new code path is the
0.25.10 runtime itself. The full workspace test suite +
`tools/gate.sh full` battery is run to confirm; any drift is reported
per grammar, not absorbed.

## Bump verdict table (evidence gathered during the provider outage, orchestrator)

The provider outage interrupted the lane mid-table; the evidence below was
measured directly in this worktree at `7505583` (5 committed bumps + the
yaml WIP lock state). **Zero drift observed across every bump.**

| Grammar | Was | Now | Verdict | Evidence |
|---|---|---|---|---|
| tree-sitter-rust | 0.23.3 | 0.24.2 | bumped — no drift | full gate below |
| tree-sitter-javascript | 0.23.1 | 0.25.0 | bumped — no drift | full gate below |
| tree-sitter-python | 0.23.6 | 0.25.0 | bumped — no drift | full gate below |
| tree-sitter-go | 0.23.4 | 0.25.0 | bumped — no drift | full gate below |
| tree-sitter-c | 0.23.4 | 0.24.2 | bumped — no drift | full gate below |
| tree-sitter-bash | 0.23.3 | 0.25.1 | bumped — no drift | full gate below |
| tree-sitter-yaml | 0.7.0 | 0.7.2 | bumped — no drift | full gate below |
| runtime | 0.25.10 | 0.25.10 | unchanged (the bump lane's base) | — |

Measured gate at this state: `cargo test --workspace` **959 passed / 0
failed** (incl. the highlight byte-identity suite and every corpus golden
driver), `cargo clippy --workspace --all-targets -- -D warnings` clean,
`tools/pool.py runall` **12/12 suites, 87 s**. No golden flipped; no
extraction/highlight test changed.

Remaining candidates NOT in this state (verify when the provider returns):
c-sharp 0.23.5, md 0.5.1 (the recorded known-bad straggler), and any newer
release for cpp/java/ruby/scheme/json/toml-ng/typescript.
