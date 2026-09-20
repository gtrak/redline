# Task: fix the five known-issue watchlist items (U-K)

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `docs/ux-testing-plan.md`'s U-K section (the list, with its review
provenance) and the landed machinery each touches.

## The five items

1. **Uppercase-initial `M-.` misses types/constants**: `symbol_at_point`'s
   token extension misses `Type`/`CONST`-shaped names (review 05) — hunt
   the actual heuristic gap in the extraction (why does an
   uppercase-initial identifier not extend?) and fix; pin per shape
   (types, SCREAMING consts, mixed CamelCase).
2. **imenu flat, no impl-parent nesting**: the imenu list doesn't group a
   struct's methods under the struct — decide the grouping honestly
   (Rust: methods group under their impl's type via the Rung 1 tables —
   the data exists; other languages stay flat unless cheap). The imenu
   list is the existing `xref_candidates`/Symbols picker — judge the
   cheapest honest grouping.
3. **`search_jump` closes the results view when the open fails** (the
   known pre-011 artifact): on open failure, keep the results view open
   and report — the jump didn't happen. ALSO the occur-on-scratch RET
   error in the same review note — reproduce and fix or report as
   separate.
4. **`M-,` under the Search view is a no-op until the view is closed**:
   jump-back should pop through the sentinel directly (the sentinel
   entry's navigation opens the results view — make M-, land the user
   back at the pre-search position in one step, like emacs's
   `xref-pop-marker-stack`).
5. **Page scroll has no 2-line overlap**: add emacs
   `next-screen-context-lines` semantics (keep 2 context lines on C-v/
   M-v) — judge where the scroll offset logic lives and pin.

## Constraints

- Each fix keeps its existing behavior byte-for-byte where the review
  says unchanged (degradation discipline); every fix gets a
  discriminating unit twin (loop-03 discipline: state + render80).
- Gate: `cargo test --workspace` + clippy (PIPESTATUS exit) +
  `tools/gate.sh full` (flock, cargo build first). Budget ~45 tool
  calls; honest-stop at half.
- Scope fence: `src/app/store.rs` (symbol_at_point, imenu/xref
  candidates, search flow, scroll logic), `src/app/flow_tests.rs`
  (twins), `docs/ux-testing-plan.md` (the U-K rows flip to fixed).
  NO syntax/provider changes. Parallel lane: ts-bump owns Cargo.toml +
  syntax files — do not touch them.
