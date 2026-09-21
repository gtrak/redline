# Task: A7 — split `store/navigation.rs` (and its test file) by concern

Plan 012's stated success criterion is **"No `src/` file over ~1,500 lines"**; an
audit after the 17 landed lanes showed it is only PARTIALLY met
(`.agents/plans/012-project-organization/00-worklist.md` § A7). The one genuine
grab-bag is `src/app/store/navigation.rs`: **2,394 lines, ONE `impl AppStore`,
48 methods** — three unrelated concerns sharing a vague label, plus a test file
(`src/app/store/tests/navigation.rs`, **3,074 lines, 100 tests**) that mirrors it.

This is a **PURE MOVE** (behaviour-preserving), the same shape as the landed
`store-concerns`, `node-split`, `repo-split`, and `root-split` lanes. Two commits:
one for the production file, one for its test file.

## Read first

- `.agents/plans/012-project-organization/00-worklist.md` § **A7** (the seam evidence).
- `.agents/plans/012-project-organization/PLAN.md` § Success criteria.
- `docs/architecture.md` (the module map the split must update).
- `.agents/skills/plan-process/SKILL.md` § "Test authority".
- The landed precedent: `git show 0bc74c3` (store-concerns) and `git show 7be095e`
  (root-split) — both are the same pattern, including the faithfulness proof.

## The seams (I measured these; **re-derive them before relying on them**)

`src/app/store/navigation.rs`, method → line, in source order:

| Range | Methods | Concern |
|---|---|---|
| 1–191 | `open_external_path` 11, `open_resolved_source` 34, `current_jump_entry` 79, `record_jump` 93, `jump_back` 106, `jump_forward` 115, `navigate_to_entry` 127 | **jump history** (7) |
| 192–912 | `xref_find_definitions` 192, `find_implementations` 391, `xref_definition_candidates` 489, `self_receiver_candidates` 531, `local_binding_candidates` 560, `type_member_candidates` 586, `rust_dotted_receiver` 656, `start_symbol_resolution` 720, `apply_resolve_event` 776, `resolving_display` 806, `resolution_language` 819, `resolver_scope` 844, `resolver_scope_for` 860 | **definitions + resolution lifecycle** (13) |
| 913–1806 | `use_path_for_symbol` 913, `use_decl_path` 985, `use_group_entries` 1036, `import_segments` 1112, `node_text` 1132, `is_ascii_identifier` 1139, `js_ts_scope_for` 1149 … `js_ts_namespace_member_path` 1431 (8 `js_ts_*`), `python_scope_for` 1475 … `python_dotted_segments` 1658 (4 `python_*`), `go_scope_for` 1682 … `go_string_content` 1752 (4 `go_*`), `resolver_from_file` 1773 | **use-path extraction + per-language import parsing** (24) |
| 1807–2394 | `xref_in_external_buffer` 1807, `crate_xref_outcome` 1897, `symbol_at_point` 2028, `dotted_path_container` 2195, `open_imenu` 2230, `current_buffer_outline` 2273, `current_buffer_rust_tables` 2306, `open_imenu_picker` 2328, `open_symbol_picker` 2341, `which_function` 2371 | **external/crate xref + symbol-at-point + imenu/outline** (10) |

**The tell is range 3**: ~20 language-specific import-parsing free functions
(`js_ts_*`/`python_*`/`go_*`) are "navigation" only by accident — they are really
*import resolution for a language*, and they are the reason this file is a
grab-bag rather than a big cohesive module.

## What to build

Split by **contiguous range** (a pure move — do not reorder methods, do not
rewrite bodies). Proposed shape, ~4 files, each well under the bar:

```
src/app/store/navigation/
  mod.rs          # jump history (1–191) + the `impl AppStore {` wrapper + mod decls
  definitions.rs  # 192–912
  imports.rs      # 913–1806   <- the concern that does not belong
  xref.rs         # 1807–2394
```

Judge the boundaries yourself: the requirement is **no resulting file over
~1,500 lines and no reordering**, not the exact file names. If `1807–2394` reads
as two concerns (external/crate xref vs imenu/outline), split it further — 5 files
is fine. If a boundary needs to move by a few methods to keep a helper with its
caller, that is acceptable **as long as you say so** and the partition proof still
balances.

Then split `src/app/store/tests/navigation.rs` **along the same seams** (separate
commit): its test names already cluster that way — `xref` 40, `resolver` 28,
`symbol` 10, `jump` 8, `imenu` 3, `which` 1. Target
`src/app/store/tests/navigation/{mod,jump,definitions,imports,xref,imenu}.rs` or
similar; `tests/mod.rs` keeps the shared fixtures (all concern files do
`use super::*`). Mind the `#[path]` rule below.

## Mechanics (compiler-verified in earlier lanes — do not re-learn these)

- **`E0624`**: a private method called across sibling submodules needs
  `pub(super)`. Count and report how many you add. Precedent: store-concerns added
  **73** (measured, not 168).
- **`E0583`**: `#[path = "..."]` resolves **relative to the containing file**, so a
  child of `tests/navigation/mod.rs` referencing `../..`-style paths needs care.
- **`E0364`**: `pub use` cannot re-export a less-visible item → use `pub(crate) use`.
- A **child-module `impl` block reaches private fields with no field-visibility
  change** (validated by `repo-split`): splitting `impl AppStore` does not require
  touching `AppStore`'s field visibility. Report if you find otherwise.
- `src/app/store/mod.rs` needs only its `mod` declaration changed. Do not touch
  `store/mod.rs`'s 2,445 lines otherwise — it is cohesive (struct + 18 core
  methods) and staying over the bar is an accepted decision, not an oversight.

## Faithfulness proof (required in the report)

The landed pattern, and the reason these moves are trustworthy: extract the
original with `git show <base>:src/app/store/navigation.rs`, strip comments
(string/char-aware) and collapse whitespace, chunk top-level items (and, for the
test file, test items) at the right brace depth, and compare old vs new as a
**multiset**. Report: items paired N/N, zero residue, zero duplication, and the
exact delta set (expected: `mod` decls, per-file `use` blocks, `pub(super)`
insertions, any re-export). **Any changed body is a P1.** State the
comment-line count before/after and explain any missing comment line.

## Fence

`src/app/store/navigation.rs` → `src/app/store/navigation/*`,
`src/app/store/tests/navigation.rs` → `src/app/store/tests/navigation/*`,
`src/app/store/mod.rs` + `src/app/store/tests/mod.rs` (**`mod` declarations only**),
`docs/architecture.md` (the module map). Nothing else.

## Gate

`cargo build`; `cargo test --workspace`; `cargo clippy --workspace --all-targets`
(read `${PIPESTATUS[0]}`); `timeout 900 tools/gate.sh full`.
**Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` **and swap**
before the battery; if swap is exhausted, run `cargo test --workspace` + the two
windowing drives and report the PTY battery as **deferred** (the session's
final-pass pattern), never as a failure.
`check_cursor_stream.py` is a known failure **only under concurrent build load**
(80/80 idle, verified) — if it is the only failing suite, say so.

## Report format

Per commit: the file list with line counts, the item partition (N/N, residue,
delta set), the `pub(super)` count, comment-line accounting, gate commands with
observed output, the workspace `test result:` lines reconciled against the
pre-lane baseline, and anything you judged differently from the proposal above
(with the reason). Budget ~60 tool calls; honest-stop at half budget with the
production-file commit landed if the test split is incomplete.
