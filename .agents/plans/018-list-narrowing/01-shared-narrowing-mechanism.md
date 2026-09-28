# 01 — Extract the shared narrowing mechanism (byte-for-byte picker migration)

**Status:** OPEN

## Objective

Move the ONE narrowing engine out of `Picker`'s private method and into a
shared, store-side seam, so that every later narrowable surface implements
against a proven core instead of copying `Picker::recompute`. This issue
changes **no user-visible behaviour**: the picker's observable behaviour
(query parse, scores, order, selection clamp, preview refresh) is
byte-for-byte, and its entire existing test suite is the acceptance.

## Key decisions

- The core is a pure function over **display strings and row indices** —
  semantically identical to `Picker::recompute` (`src/app/store/mod.rs:2944`):
  - `narrow(query: &str, displays: &[&str], matcher: &mut Matcher) -> Vec<(usize, u32)>`
    best-first, `sort_by_key(Reverse(score))` (stable — ties keep source
    order, exactly today); empty query → every row at `u32::MAX`
    (`mod.rs:2948-2952`); one `Pattern::parse(query, CaseMatching::Ignore,
    Normalization::Smart)` per call, one `Utf32Str` scratch buffer.
  - Lives in `src/app/store/narrowing.rs` (module) or `mod.rs` if the content
    proves too thin — decide by content, not tidiness. No new crate.
- The session is `NarrowSession { query: String, selected: usize, filtered:
  Vec<(usize, u32)> }` with `recompute(displays, matcher)` and the picker's
  selection clamp as the one rule:
  `selected = selected.min(filtered.len().saturating_sub(1))`
  (`src/app/store/picker.rs:583`, `set_picker_query`).
- The seam takes the **`Matcher` as a parameter** — the two existing
  instances (`file_matcher` vs `matcher`, `picker.rs:548,581`) stay as they
  are; the seam does not unify them.
- The seam deals in display strings and indices ONLY. No byte/char/column
  conversion enters it (PLAN §5.8).
- Picker migration: `Picker` keeps `kind`/`prompt`/`preview`/verb machinery;
  its `query`/`selected`/`filtered` triple becomes a `NarrowSession` over
  `display` projections (`|c| &c.display`). The per-kind verb keys (Stash
  `x`, Annotations `d` — `keys.rs:173,187`) stay in the surface's key handler,
  NOT in the session.
- **Not** "make every list produce `PickerCandidate`s" — the row type stays
  per-surface (PLAN §2.2a, with the costs named both ways).

## Files

| File | Change |
|---|---|
| `src/app/store/narrowing.rs` (new) or `mod.rs` | the `narrow` core + `NarrowSession` |
| `src/app/store/picker.rs` | `Picker`'s query/selected/filtered triple → `NarrowSession`; `open_picker`/`picker_query_char`/`picker_query_backspace`/`set_picker_query` route through it |
| `src/app/store/mod.rs` | `Picker::recompute` deleted (moved); `Picker` struct updated; `mod narrowing;` |
| `src/app/store/tests/picker.rs` | unchanged behaviour; ADD the discrimination pins below |

## Steps

1. Extract `recompute`'s body into the core, verbatim semantics (empty-query
   branch, `CaseMatching::Ignore`, `Normalization::Smart`, stable
   `Reverse(score)` sort, `u32::MAX` sentinel).
2. Build `NarrowSession` with the clamp rule; port `Picker` onto it. Keep
   `picker_filtered()`'s public shape (it returns
   `&[(PickerCandidate, u32)]` — `picker.rs:605`; callers in `keys.rs` and
   tests depend on the `(candidate, score)` pairing).
3. Discrimination proof (PLAN §5.7) — the acceptance, not a nicety:
   - Pin A: a mutation of the SHARED core's parse flags (e.g.
     `CaseMatching::Sensitive`) reddens an existing picker test — run it,
     quote the red test's name in the report.
   - Pin B: a mutation of the clamp (e.g. drop the `.min(...)`) reddens a
     test where the selection survives a shrinking filtered set (add one if
     none covers it).
   - Pin C (registry cross-check, PLAN §5.5, `0829ddd`): a test that
     enumerates all **13** `PickerKind`s and asserts each kind's
     `set_picker_query` path re-derives its rows through the shared core
     (e.g. by asserting the filtered order through the session for a
     discriminating query per kind, or by a code-shape check the reviewer can
     falsify). A new `PickerKind` landing without a shared-path row fails
     this test.
4. Verify byte-for-byte: the full picker test suite (`tests/picker.rs`,
   `tests/project.rs`, the Xref/Imenu/Annotations picker pins in
   `tests/navigation/*`) passes UNMODIFIED. Any test that needed editing is a
   finding to report, not a silent update.

## Verification

- `cargo build` + `cargo clippy -D warnings` + `cargo test` (bin + workspace).
- The three discrimination pins (A/B/C) with the mutation evidence quoted.
- No drive changes expected (no user-visible change); if any drive asserted
  picker internals that moved, disclose with before/after.
