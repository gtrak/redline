# Tree-sitter runtime bump 0.24.7 → 0.25.10 — compatibility matrix

Decision-free evidence for the runtime bump task
(`.agents/tasks/issue-runtime-bump.md`). Every requirement below was
read from the local crates.io registry index cache
(`~/.cargo/registry/index/…/.cache/tr/ee/<crate>`) — no network. The
empirical confirmation is the ABI guard
`all_grammars_set_language_succeeds` (registry.rs), run over every
grammar-bearing row of the descriptor table's `grammar` column
(`crates/redline-syntax/src/language.rs`) — the single grammar pin every consumer
reads — against the bumped lock.

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

## Recorded optional follow-up: 0.25-gen grammar releases (RESOLVED by the grammar-bumps lane)

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
That lane is the 0.25-generation grammar-bump lane; its per-grammar
verdict table (the outcome of every candidate below) is at the bottom
of this file.

## Behavioral neutrality claim

Grammar binaries are UNCHANGED (all 16 pins kept byte-for-byte), so
every parse, highlight event, node predicate, and golden is
byte-identical to the 0.24.7 tree — the only new code path is the
0.25.10 runtime itself. The full workspace test suite +
`tools/gate.sh full` battery is run to confirm; any drift is reported
per grammar, not absorbed.

## Bump verdict table (evidence gathered during the provider outage, orchestrator)

The provider outage interrupted the lane mid-table; the evidence below was
measured directly in this worktree at `7505583` (6 committed bumps + the
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
| tree-sitter-c-sharp | 0.23.1 | 0.23.5 | bumped — no drift (vendored highlights re-copied + re-pinned) | per-bump suite below; final gate below |
| tree-sitter-md | 0.3.2 | 0.5.1 | bumped — no drift (the recorded known-bad straggler is now verified GOOD) | per-bump suite below; final gate below |
| tree-sitter-cpp | 0.23.4 | 0.23.4 | not bumped — no newer release (0.23.4 is the newest in the index) | index cache |
| tree-sitter-java | 0.23.5 | 0.23.5 | not bumped — no newer release | index cache |
| tree-sitter-ruby | 0.23.1 | 0.23.1 | not bumped — no newer release | index cache |
| tree-sitter-scheme | 0.24.7 | 0.24.7 | not bumped — no newer release (0.24.7-1 is a pre-release) | index cache |
| tree-sitter-json | 0.24.8 | 0.24.8 | not bumped — no newer release | index cache |
| tree-sitter-toml-ng | 0.7.0 | 0.7.0 | not bumped — no newer release | index cache |
| tree-sitter-typescript | 0.23.2 | 0.23.2 | not bumped — no newer release | index cache |
| runtime | 0.25.10 | 0.25.10 | unchanged (the bump lane's base) | — |

Measured gate at this state: `cargo test --workspace` **959 passed / 0
failed** (incl. the highlight byte-identity suite and every corpus golden
driver), `cargo clippy --workspace --all-targets -- -D warnings` clean,
`tools/pool.py runall` **12/12 suites, 87 s**. No golden flipped; no
extraction/highlight test changed.

### The resumed candidates (c-sharp, md) — measured per-bump on the new pins

Each of the two remaining candidates was bumped one at a time (exact pin)
and re-measured with the same standard — the full `cargo test --workspace`
suite (959 tests: extraction, highlight byte-identity, node predicates,
corpus goldens, and the `all_grammars_set_language_succeeds` ABI guard):

- **tree-sitter-c-sharp 0.23.1 → 0.23.5** — suite green (959 / 0 failed);
  lock moved exactly one entry. The 0.23.5 crate's `queries/highlights.scm`
  differs from the 0.23.1 vendored copy by exactly one line: the `..`
  range operator joins the `@operator` list. Per the C#/Clojure vendored
  pattern the new file was re-copied VERBATIM to
  `third_party/tree-sitter-c-sharp-0.23.5/highlights.scm` (sha256
  `ab8a9930aeeee70fa2dbfde82e4763170b7e826bc642338ad0683772c20c060f`,
  byte-verified) and `C_SHARP_HIGHLIGHTS` re-pinned (doc comment +
  `include_str!`, `crates/redline-syntax/src/queries.rs`); the stale 0.23.1 copy was
  removed and the `language-coverage.md` CSharp row path updated. No C#
  extraction/predicate/golden flipped — no fixture exercises `..`, so the
  query content change is latent (a highlight-face gain, not observed
drift).
- **tree-sitter-md 0.3.2 → 0.5.1** — the 011-resolver-parity note recorded
  0.5.1 as a known-bad straggler. Re-verified honestly: its `tree-sitter
  ^0.24` NORMAL dependency is OPTIONAL and feature-gated (registry-index
  metadata — same shape as 0.3.2's optional `^0.23`), and with the feature
  disabled it pulls nothing into the lock: the lock keeps exactly ONE
  `tree-sitter` runtime (=0.25.10), so the historical second-runtime /
  links failure mode does not reproduce. The 0.5.1 grammar ABI is inside
  the 13..=15 window (guard test green) and the full suite is green on
  the new pin (959 / 0 failed): md extraction, highlight byte-identity
  (block + inline), node predicates, and the markdown goldens unchanged.
  Verdict: the straggler status is stale — 0.5.1 is GOOD. 0.5.2+ (normal
  `^0.26`) remain outside the ABI window and are NOT bumped.

### Final gate (tip of the lane, all 9 bumped grammars in place)

`cargo test --workspace` green, `cargo clippy --workspace --all-targets
-- -D warnings` clean, and `tools/gate.sh full` **OK (full)** — build +
clippy + test + all 18 PTY/Python battery stages (sweep, drive_all, windowing,
cursor stream, xref, externals, 011 legs, sweep_flows) passing. (The
lane's first `gate.sh full` attempt FAILED at `check_cursor_stream` +
`sweep_flows` purely on cross-lane PTY-fixture contention — a concurrent
jumpback-lane `gate.sh full` held the shared fixture, which the driver's
flock surfaced as "busy" refusals; with the lanes clear the identical gate
passes end-to-end. No grammar-related failure at any point.) No golden
flipped across the whole lane; no extraction/highlight test changed.
