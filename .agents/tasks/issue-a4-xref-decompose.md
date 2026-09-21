# Task: decompose `xref_find_definitions` (177 lines) along its own phase labels

`src/app/store/navigation/definitions.rs:33` — `pub fn xref_find_definitions(&mut self)`
is **177 lines, the longest production function in the tree** after A1 (`Root` 504→66)
and A3 (`key_event` 346→109) cleared their targets. It was *moved* by A7 (organized by
file) but never *decomposed* (organized by logic) — which is exactly the distinction the
user's maintainability criterion draws.

Measured with `tools/fn_survey.py` (locations verified against `rg`).

## The structure — the author already labelled the phases

| Lines | Job | The author's own label |
|---|---|---|
| 35–80 | **preamble**: buffer guard, buffer lookup, OWNED path, project guard, `rel` via `strip_prefix` (with the 006-03 external-buffer branch at 56–69), `rel` string, `resolve_generation += 1` (77), `line` (79), `line_text` (80) | — |
| 81–141 | **symbol under the point** → `lang` (86), `at = symbol_at_point(...)` (87), the index lookup (~89–140), `defs.unwrap_or_default()` (141) | `// (1) The symbol under the point: identifier run around the point's column…` |
| 143–165 | **fallback chain**: `if !defs.is_empty() { … } else { … }` | `// (2) Symbol-at-point with definitions…` / `// (3) Enclosing-symbol fallback…` |
| 167–176 | **nothing known** | `// (4) Nothing the workspace knows about under/near the point…` |
| 178–~209 | **dispatch**: `if defs.len() == 1 { jump } else { picker }` | — |

**Goal:** `xref_find_definitions` becomes ~30–40 lines of orchestration that call named
helpers, and each phase becomes a function named for its job. **Use the author's
phase names** — they are already the right names.

## Key decisions

- **This is a pure extraction** — no logic rewritten, no condition inverted, no
  reordering. Every comment survives (this function carries a lot of decision history:
  the 006-03 external-buffer semantics, the 006-02b generation/supersede rationale,
  the 011-06 language-aware token extraction).
- **BORROW DISCIPLINE IS THE HAZARD.** Line 43's comment is load-bearing: *"An OWNED
  path: the `buf` borrow must not span the `&mut self` calls below (the external-buffer
  navigation, 006-03)."* Any helper you extract must not hold a `buf`/`path` borrow
  across a `&mut self` call. If a helper's signature would force that, return owned
  data instead — and say so.
- **The guard preamble is a judgement call — argue it.** The early returns (35–69) emit
  user-visible minibuffer messages ("no buffer", "no file (scratch buffer)", "no
  project") and are the function's *entry contract*. Two defensible shapes: (a) keep the
  guards in the orchestrator and extract only the computation after them, or (b) extract
  a `fn definition_context(&mut self) -> Option<…>` returning `None` after messaging.
  Pick one, and say why. (b) risks moving the messages away from the entry point; (a)
  leaves a ~40-line preamble. **Do not** invent a `Result` error type for this — the
  messages are already the contract.
- **`self.resolve_generation += 1` (77) must stay ordered** relative to the resolution
  work (the 006-02b rationale: a stale event must not clobber a newer request). Preserve
  its position exactly.
- **Phase 4 and the dispatch interact**: `if defs.is_empty() { …(4)… }` is followed by
  the `defs.len() == 1` / else dispatch. Make sure the extraction keeps the
  empty/multiple/single cases reachable in the same order (an early `return` added or
  moved here would change behaviour).
- **Do not touch** `symbol_at_point`, `start_symbol_resolution`, `find_implementations`,
  or the resolver machinery — only this function's body.

## Files

`src/app/store/navigation/definitions.rs` (the fence). A `definitions/` submodule split
is *allowed* if the extracted helpers make the file unwieldy, but **not required** —
the criterion is logic organization, not file size.

## Verification

- `cargo build`; `cargo test --workspace` — reconcile against the **current** baseline
  (**867** passed / 0 failed / 2 ignored redline — the column-landings lane added 8 to
  859; measure it yourself, do not copy my number), resolver 123/0/4, integration
  7,2,3,1,1, doctests 0; `cargo clippy --workspace --all-targets` (read
  `${PIPESTATUS[0]}`); **`timeout 900 tools/gate.sh full`** — `M-.` is exercised by
  `drive_xref.py`, `drive_issue_011_0*`, and the PTY flows, so the battery is the real
  check. If it fails for swap, run the workspace suite + both windowing drives +
  `drive_xref.py` + `check_cursor_stream.py` and report the battery DEFERRED.
- Report: the before/after line count of `xref_find_definitions`, the helper list with
  sizes, the borrow-discipline analysis, the preamble shape you chose and why, comment
  accounting, and the before/after `tools/fn_survey.py --min 130` output.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.
