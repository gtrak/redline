# Task: plan 011 issue 06 — per-language path-token extraction (activate the dotted providers)

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth.

## Origin

The 011 matrix (`docs/provider-matrix.md`) recorded the honest end state:
non-Rust path-shaped use sites BAIL at M-. because `symbol_at_point`
(`src/app/store.rs:9121`) extends the token only across `::`. The js/python/go
providers' dotted handling (`js_provider.rs`/`python_provider.rs`/
`go_provider.rs` resolve `pkg.member` directly) is fully unit-tested but
**unreachable from M-.** — the app never feeds them a dotted token. 011-03's
whole-path machinery in `src/syntax/node.rs` already returns the per-language
path containers (`member_expression`, `attribute`, `selector_expression`/
`qualified_type`). This issue connects them: the providers become reachable
for real use sites.

## Prerequisite

011-01..05 in HEAD. Check `git log --oneline -8`; STOP and report if absent.
Also: the 007-04 (tree retention) and 004-06a (home) lanes may have merged —
re-run the gate before starting if main moved.

## What to build

1. **Language-aware `symbol_at_point`** (or a sibling the two M-. call sites
   use): in a non-Rust buffer, use the 007-01/011-03 machinery
   (`syntax::node::node_at` with the buffer's `LanguageId`) so the PATH TOKEN
   for M-. becomes the whole dotted path (`json.dumps`, `ns.member`,
   `p.a.b`, `os.path.join`) when the point is inside a path container — and
   the bare identifier otherwise (exact current behavior). Rust keeps `::`
   byte-for-byte. Bound it: one parse at the point (007-01's discipline), and
   if `node_at` returns None (no tree / unsupported shape), fall back to the
   current bare extraction — never guess.
   - The Python leg of the matrix documents `plain import json` + `json.dumps`
     as bailing. After this change, `json.dumps` at a use site is a DOTTED
     token → the python provider's dotted handling resolves it. Same for JS
     `ns.member`/`pkg.fn` and Go `pkg.Fn`.
   - Mind the 011-02 interplay: a dotted token reaches the provider and
     resolves on its OWN path (`scope_qualified` returns None for symbols
     containing the separator — the symbol's own path wins; the import-walk
     hint is only for BARE symbols). Verify no double-application.
2. **Per-language unit legs — NO PTY where a lower level suffices.** The
   providers' dotted handling is already unit-tested; what is new is the
   APP feeding dotted tokens. Test the seam at the lowest reliable level:
   app-level unit tests on the store (real fixtures + `symbol_at_point` →
   M-. selection path → resolver context carries the dotted token → the
   provider's own machinery resolves it) — real toolchain shell-outs at the
   provider level (the existing `npm`/`python3` test pattern) where they
   are reliable, and NOT via PTY. PTY legs are for what ONLY a live app can
   prove (the matrix cells that change and any user-visible indicator);
   keep those minimal — one python leg + one js leg max, loud-skip if the
   runtime is absent.
3. **Update `docs/provider-matrix.md`**: the path-shaped cells change from
   "degrades to the bail" to the new truth, per language, with verification
   status. Keep the capability-terms discipline (cells cannot be misread).
4. **Degradation is byte-for-byte**: no parse / unsupported shape → the exact
   current bare-extraction behavior (a bare symbol with no import hint still
   bails with the exact existing message).

## Explicit non-goals

- No changes to provider internals (their dotted handling is already
  unit-tested — this issue FEEDS them; do not rewrite them).
- No changes to `resolver_scope_for`'s bare-symbol walks (011-02).
- No Rust `::` behavior change. No LSP. No new dependencies.

## Constraints

- Skills are truth. Write-first; compile early.
- Gate: `tools/gate.sh fast` inner loop, `tools/gate.sh full` final
  (`--workspace`; do not regress resolver coverage). Progress streams to
  stderr; do NOT pipe stdout through `tail`.
- PTY flock: "shared PTY fixture is busy" + exit 3 ⇒ wait and retry; NEVER two
  suites concurrently; wrap EVERY python PTY invocation in `timeout`.
- Budget: land within ~50 tool calls; no new investigations after it compiles.
- Scope fence: `src/app/store.rs` (`symbol_at_point` + its call sites +
  tests), `docs/provider-matrix.md`, the new drive, `tools/gate.sh`
  (SHARED_SUITES line). Nothing else. `src/syntax/node.rs` must NOT change
  (011-03's API is the prerequisite, already merged).
- Plain `git commit`; do not `git add -A` other sessions' files.

## Verification (iterate until ALL pass)

- `tools/gate.sh full` green with HONEST counts; all existing suites unchanged.
- The matrix cells updated match the live legs (a cell you could not verify
  live says so).
- Existing pins: `symbol_at_point_identifier_and_path_token` (Rust `::`) and
  `symbol_at_point_field_access_stays_bare` — the latter's CONTRACT may need
  updating (a Rust `.` field access still stays bare; a JS/Python `.` path
  now extends). Decide per language and state it; Rust `.` field access
  stays bare (fields are not in the index).

## Report

Files changed; per-language verification status (live vs unit vs loud-skip);
matrix cell deltas; tests added (discriminating vs pin); gate counts from
actual harness output; deviations with reasons.
