# Task: `nav/index.rs` split + the stale dead-code cluster (M1/M2 + S4)

## Why
`src/nav/index.rs` (1,075 lines) conflates four concerns, and six of its items carry a
stale `#[allow(dead_code)] // used by the background index thread (future wiring)`
comment on code that **has been wired for a long time**. A contributor reading those
notes concludes the index path is unwired/tested-only. This lane fixes both.

## Item 1 — delete the stale attributes and the lying comments (M1)
Verified live callers (the attribute is NOT load-bearing — `cargo clippy -D warnings`
passes without it):
- `IndexProgress` / its impl — used at `src/app/store.rs` (`IndexProgress::new`,
  `Arc<IndexProgress>`);
- `extract_file`, `build_index`, `refresh_in_place` — production callers in
  `src/app/store.rs` (the background index job);
- `IndexBus::send` — called internally when progress is published;
- also `with_publisher` if it carries the same stale note.
Delete the attribute **and** the obsolete "(future wiring)" comment on each. If a
particular item turns out to be genuinely test-only, say so and leave the attribute
with an accurate reason (`// test-only seam`) — do not delete a load-bearing one.

## Item 2 — delete the genuinely dead method (M2)
`IndexProgress::mark_done` has **zero references** anywhere in the repo (verified).
Delete it. If you find a caller, stop and report instead.

## Item 3 — split the module (S4)
Target layout under `src/nav/index/`:
- `symbol_index.rs` — `Location`, `TraitImpl`, `TraitImplLocation`, `SymbolIndex`
  (+ its inherent impl and accessors);
- `progress.rs` — `IndexProgress`, `PROGRESS_STEP`, `IndexEvent`, `IndexBus`;
- `builder.rs` — `extract_file`, `build_index`, `refresh_in_place`,
  `enclosing_symbol`, and `set_file_tables` as a second `impl SymbolIndex` block
  (legal in-crate; it is currently a ~100-line method);
- `mod.rs` — the module root with `pub use` re-exports so every existing
  `nav::index::X` path keeps working unchanged.
Confirm the exact boundaries by reading (do not trust remembered line numbers).
`src/nav/xref.rs`'s `Xref` trait is **out of scope** (a separate decision).

## Rules
- **Behavior-preserving only** — pure moves, no logic edits, no renames.
- `pub use` re-exports are mandatory: external paths (`nav::index::SymbolIndex`,
  `IndexBus`, `IndexEvent`, …) must not change; check `src/nav/mod.rs` and every
  caller before/after.
- Fence: `src/nav/index.rs` (→ `src/nav/index/*`), `src/nav/mod.rs`. Nothing else.
  If a needed change falls outside, stop and report.
- Honest-stop at half budget: the attribute deletions (Items 1–2) are a valid
  landing on their own if the split is not finished.

## Gate
`cargo build` first, then `cargo test --workspace`, `cargo clippy --workspace --all-targets`
(read `${PIPESTATUS[0]}`), `tools/gate.sh full`.
Budget ~40 tool calls. Report: which attributes you deleted vs kept (with the reason),
the split layout, re-export handling, gate counts.
