# 006 — Redline: tooling-aware jump-to-definition (COMPLETE)

Origin: user directive 2026-09-17 — "I want jump-to-selection to be language
and tooling aware, eg resolve the real cargo source folder, pull it if
necessary by shelling out to those tools." Definitions in dependencies
(`tokio::spawn`, `serde::Deserialize`) previously went nowhere.

Approach: the repo became a Cargo workspace; the resolver lives in its own
app-free crate `crates/redline-resolve` (plain root+symbol in → plain
locations out; `cargo metadata` + `cargo fetch` as the sanctioned shell-outs,
status-line activity, graceful degradation — the workspace path always wins).
The app consumes it as a dependency.

## Issues (all PASS, reviewer-gated)

- **01 — resolver chain + Rust/cargo provider** (PASS): resolution order
  workspace tree-sitter Xref → language tooling provider; crate name+version
  → registry source dir, `cargo fetch` when absent.
- **02 — app wiring: M-. fall-through** (`d9e181b`, PASS): cursor-aware
  `symbol_at_point` + `::`-path token, same-file definitions first-class,
  `spawn_blocking` `ResolveBus` (watch, latest-wins) drained in Root,
  generation-gated, `*resolving …` indicator, read-only external landings
  (`open_external_path`), `drive_xref.py` 7/7.
- **02b — review follow-ups** (`e66c654`, PASS, 509 tests): **ownership
  guard** — `external_buffers` + `buffer_is_project_owned`; `C-x C-q` and
  `C-x C-s` refused on registry/tooling sources (closes a real corruption
  hole in the deliberate read-only override; project/scratch/previous-project/
  notes semantics intact); M-. supersede generation bump; drain-race
  indicator clear; six P3s + 008-01's P3s.
- **03 — navigate WITHIN external sources** (`7bab785`, PASS, 520 tests):
  user follow-up — "I want to follow other types once inside a library
  buffer." Background crate index (`CrateIndexBus`, `spawn_blocking`, LRU
  cap 3 keyed by `ResolvedSource.source_root`, `indexing crate …` indicator
  off the input path); M-./imenu inside a library buffer run the SAME
  selection rule against the crate index (crate-relative display, read-only
  landings); a miss keeps the resolver fall-through on the ORIGIN project's
  metadata; blame carries a reason. New `drive_external_crate.py` 12/12.
- **03b — review follow-ups** (`9b2dd1d`, PASS, 523 tests): cap-3 eviction
  keeps the crate you are IN MRU (bumped at the eviction point itself);
  resolver `from_file` crate-relative; `\`→`/` normalization at both lookup
  sites; no `unwrap` on a path relation; N/M counter (zero nav/index.rs edits).

## Success criteria — met

- M-. on `tokio::spawn` resolves through the provider chain; registry
  landing read-only.
- Live: M-. on `Rope` → `~/.cargo/registry/.../ropey-1.6.1/src/rope.rs:82`
  → M-. on `RopeBuilder` → `src/rope_builder.rs:42` (in-crate) → M-, back.
- Workspace symbol behavior unchanged; project registry/tree/watcher/edit
  mode never see external sources (008-01 semantics).

## Notes

- Gate at completion: `tools/gate.sh full` — 523 tests / 0 failed / 2
  ignored; sweep 14/14; drive_all 8/8; windowing 28/28; panes 4/4;
  cursor-stream 80/80; notes-dump 17/17; drive_xref 10/10; external-notes
  16/16; external-crate 12/12; sweep_flows 65/65; ux_sweep 3 pre-existing
  (unbound `C-x 2/1/0` window splits).
- Residuals (non-blocking, documented): an external path under a
  `$HOME`/`$CARGO_HOME`-rooted project classifies as owned (guard ordering
  tradeoff); a same-key external-then-project open can keep a stale guard
  marker (fail-safe only); cap-3 pins the current crate by design.
