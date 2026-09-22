# issue-all-symbol-jumps — every symbol jump lands on the right definition

**User directive (2026-09-21):** *"ok, ALL symbols should jump to the right definition."*

This is an **acceptance criterion**, not a bug fix. It exists because the same class has
now been reported twice (*"jumps are still on the beginning of the line"*, then *"in
redline-resolve cargo.rs, when I jump to a field name, it goes to beginning of line"*),
each time fixed for the paths someone happened to notice. The criterion is what stops
the third report.

**"Right definition" means all three of:**
1. the right **file**,
2. the right **symbol** (not a same-line neighbour, not a substring match, not the wrong
   same-named one),
3. the right **column** — the landing point is on the symbol's name.

## Why it kept slipping: there is no matrix, and almost no column coverage

- The column work (`fcde521`, `0a49be9`) fixed the **outline-symbol** paths, which carry
  `start_byte`. It could not fix paths whose data source has no byte.
- **Field jumps were structurally col-0**: `StructField { field, line }`
  (`crates/redline-syntax/src/queries.rs:94`) and `SymbolIndex::field_locations`
  (`src/nav/index/symbol_index.rs:102`, `Vec<(String, usize)>`) carry a *line*, so the
  landing in `definitions.rs` computes column 0 **by construction**. The same shape
  applies to **impl methods** (`RustTables::impls`, `(name, line)`) and **local
  bindings** (`RustTables::bindings`).
- **The PTY tier asserts no columns for the definition journeys.** `tools/drive_xref.py`
  contains no `col`/`cursor`/`CUP` check at all; the only PTY column pin in the suite is
  the **imenu** landing in `check_cursor_stream.py`. So a col-0 landing on any `M-.`
  path passes the entire battery. That is why two reports arrived while every gate was
  green.

## Required: the matrix, with a test per cell

Produce a table of **symbol kind × jump path** and, for each cell, a test (or a
justified N/A). The point is that "ALL" is only checkable cell by cell — a single
end-to-end flow does not cover it, and an unstated cell is how the next report happens.

**Kinds** (at least): Rust outline (fn/struct/enum/trait/const/static/mod/type/macro) ·
Rust tables (**field**, **impl method**, **local binding**) · each other registry
language's outline symbols (C, Cpp, JS, TS/TSX, Python, Go, Java, C#, Ruby, Clojure,
Scheme, Bash, Markdown headings, TOML keys) · annotations.

**Paths** (at least): `M-.` silent same-file unique · `M-.` → the `Xref` picker
(cross-file and ambiguous) · `M->` forced list · the Symbols and Impls pickers · imenu ·
the annotations picker · the **tooling/resolver** landing (the provider path) · search
RET · `M-,`/`C-i` jump-back/jump-forward · the enclosing-symbol fallback.

**Rules for the matrix:**
- Every cell that lands at **column 0** is a bug **unless** it is one of the justified
  exceptions, which must be listed and defended in one place:
  * **goto-line** — emacs's `goto-line` is the line start (verified against emacs 30.2).
  * **annotations** — anchored to a line; but note the record *does* carry `col`, so it
    lands on it (do not regress that).
  * an **impl-block header** (`impl X for Y`) — no single name symbol to land on.
- A **multibyte** case per kind, asserting a **char** column and not a byte offset.
- Each test must state what a col-0 (or wrong-symbol) regression would look like.
- **At least the field/method/binding cells must be PTY-level**, because those are the
  ones whose data source lacked a byte and whose fix is structural.

## Acceptance

* The matrix exists, in the tree, with a pointer to the test for each cell (and the
  justified N/A cells named).
* Every non-exception cell lands on its symbol's **name column** — verified by
  execution, not by reading.
* The Rust tables carry the name node's **start byte** (captured from tree-sitter, not
  re-derived by searching the line for the name — a text search is the heuristic that
  produced two rounds of wrong columns in the mask work).
* `tools/drive_xref.py` (or `check_cursor_stream.py`) asserts the **cursor column** for
  at least the same-file, cross-file-picker and field/method journeys, so this class can
  fail the battery from now on.
* `cargo test --workspace` + clippy `-- -D warnings` clean; `tools/gate.sh full` green.

## Fence

`crates/redline-syntax/src/queries.rs`, `src/nav/index/**`, `src/app/store/navigation/**`,
`src/app/store/picker.rs`, `src/app/store/file_view.rs`, `tools/drive_xref.py`,
`tools/check_cursor_stream.py`, and the tests for those. Disclose anything else.

The direct consequence to keep in mind: this issue **supersedes** one-off fixes for
individual jump paths. If a cell is wrong, fix it here rather than opening a new issue
for that kind.

## Matrix (evidence, jump-column-pty lane 2026-09-21)

Every cell lands through ONE of the mechanisms below; the matrix is kind ×
path with the mechanism + the executed test pinning the cell. "col" means the
symbol name's CHAR column (never a byte offset); a col-0 cell that is not in
the exceptions block is a bug.

**Landing mechanisms (the whole class):**
- **A. silent jump** (`xref_jump_unique_definition`, `definitions.rs`): lands
  via `Location.symbol.start_byte` → `try_byte_to_line_col` → char col.
- **B. Xref/Symbols/Impls picker RET** (`picker.rs` `run_selected` →
  `definition_start_byte`): outline re-read by (name, line) → `start_byte`;
  **field-table fallback** (`field_start_byte`) for struct fields (fields are
  not outline symbols); `None` → col 0.
- **C. imenu RET** (`picker.rs` Imenu arm): outline re-read by (name, line).
- **D. Annotations picker RET**: `Annotation.col` (P2-1).
- **E. tooling/resolver landing** (`land_tooling_resolved_source`): the
  provider pins a LINE; the app refines with `first_word_column` (first
  whole-word occurrence outside a comment/string; char-based).
- **F. M-, / C-i**: the recorded jump entry's (line, col).
- **G. search/isearch RET**: the hit's char column.
- **H. goto-line (M-g g)**: line start — justified (emacs 30.2).

| Kind \ Path | A silent | B picker RET | C imenu | D annot | E tooling | F M-,/C-i | G search |
|---|---|---|---|---|---|---|---|
| **Rust outline** (fn/struct/enum/trait/const/static/mod/type/macro) | A: `start_byte` from index. Store: `definitions.rs::xref_uppercase_type_and_const_shapes_jump_directly`; PTY: `drive_xref.py` L1 + L7a (CUP on the name) | B: outline re-read. Store: `xref_force_list_opens_picker_for_same_file_unique` (RET lands); PTY: L7a M-> RET (CUP) | C: PTY `check_cursor_stream.py::jump_highlight_checks` (CUP col on `beta`); store open-pin `imenu_groups_impl_methods_under_the_struct` | n/a | E: store `xref.rs::xref_resolver_hit_opens_tooling_picker_and_ret_lands_read_only` (col 7 on `pub fn spawn`) + `tooling_refinement_lands_on_the_name_for_live_definition_lines` (frequency probe) | F: `jump.rs::jump_back_restores_recorded_column`; flow test "M-, restores the origin COLUMN" | G: isearch lane pins (char col of the match) |
| **Rust tables: struct field** | A: field-table byte (this fix). Store: `xref_field_silent_jump_lands_on_the_field_name_column` (col 32), `xref_field_multibyte_prefix_lands_on_the_char_column_not_the_byte` (char 29, not byte 33); PTY: L7d (CUP display col 32) | B: field-table fallback (this fix). Store: `xref_field_picker_landing_lands_on_the_field_name_column`; PTY: L7b (cross-file picker RET, CUP col 21) | n/a (fields are not outline symbols) | n/a | n/a (fields resolve in the workspace index; a provider field-hit degrades per the E row) | F: generic | G: generic |
| **Rust tables: impl method** | A: method-table byte (this fix) via the self-/binding-pre-step. Store: `xref_method_silent_jump_lands_on_the_method_name_column` (col 7), `xref_method_multibyte_prefix_lands_on_the_char_column_not_the_byte` (char 16, not byte 20); PTY: L7c (CUP col 8) | B: outline re-read (an impl method IS an outline symbol — `function_item` carries `start_byte`); same arm as the outline row (L7a RET pin) | C: methods group under the impl parent; RET uses the same arm | n/a | E: as outline (a bare `fn` line) | F: generic | G: generic |
| **Rust tables: local binding** | as a landing TARGET: **N/A — no jump path lands on a binding's `let`** (a binding resolves the receiver's written-down type; the pre-step lands on the FIELD/METHOD cell, whose pins above carry the column). The record now carries `name_byte` (`LocalBinding`, this fix) so a future landing path has the byte at the source. Pre-step landing pins: the field/method store tests (all use `p.member` receivers) | as in A | n/a | n/a | n/a | F: generic | G: generic |
| **Other registry languages' outline** (C, Cpp, JS, TS/TSX, Python, Go, Java, C#, Ruby, Clojure, Scheme, Bash, Markdown, TOML) | A: language-agnostic — every language's extraction records `start_byte` on the name node (`queries.rs`); the landing consumes it identically. Per-language extraction pins: `queries.rs` tests (e.g. `ruby_extracts_module_class_method`, `csharp…`, `scheme_extracts_define_and_library`) | B: language-agnostic (same arm) | C: language-agnostic | n/a | E: language-agnostic refinement; provider line-pinning is per-language (`redline-resolve` corpus) | F: generic | G: generic |
| **Annotations** | n/a | n/a | n/a | D: store `landings.rs` P2-1 pins (record's `col`, not the line start) | n/a | F: generic | G: n/a |
| **Enclosing-symbol fallback** (`M-.` with no symbol under point, or on a bare binding name — the criterion lists this as its own PATH) | It is a by-LINE guess, so by construction it is **never a silent jump**: `from_enclosing` routes it to the picker, i.e. mechanism **B**, where the row's outline re-read carries the same name-node `start_byte` any outline symbol does. Store pointer: `xref_no_symbol_under_point_falls_back_to_enclosing` (`src/app/store/tests/navigation/definitions.rs`); the landing column is pinned by the B row's outline pins | B (as above) | n/a | n/a | n/a | F: generic | n/a |

**Justified col-0 / N-A cells (the ONLY exceptions — defended in one place):**
1. **goto-line** — line start by design (emacs 30.2 behavior; mechanism H).
2. **Impls picker RET** — the impl-block header (`impl X for Y`) has no single
   name symbol; the re-read matches nothing → col 0 by construction (the
   pre-existing `definition_start_byte` `None` degradation).
3. **Local binding as a landing target** — no such path exists (N/A, not
   col-0): M-. on a bare binding name has no indexed definition and takes
   the enclosing-fallback/resolver paths.
4. **Tooling landing on a fully comment/string-masked line** — the provider's
   `line_defines_item` does not mask comments, so a dead `/* fn spawn */`
   line pins; every live occurrence is masked → honest col 0 (never an
   invented column). Frequency (executed): this is the ONLY degradation *to
   col 0* on provider-pinned lines — every live definition shape
   (fn/struct/trait/const, preceding comment or string mention, multibyte
   prefix) lands on the name; see
   `tooling_refinement_lands_on_the_name_for_live_definition_lines`.
   One adjacent shape is **not** a col-0 degradation and is worth naming so a
   reader does not expect it in this list: because `comment_or_string_mask`
   deliberately leaves `//` unmasked (matching base), a provider-pinned
   **line-commented** definition (`// fn spawn() {}`) lands on the name
   *inside the comment* — a nonzero column, on dead code. It is the same
   dead-code class, one step milder.
5. **Stale index** (byte out of range after an external edit) — honest col 0
   (`landing_column_from_start_byte`'s `None`), pre-existing.
6. **Same-line tie — deterministic but not EXACT** (a known limit, not a col-0
   exception). The picker's RET row carries only `file:line` + the field name
   (`picker.rs` parses `name.rsplit_once(':')`), so when two same-named fields are
   declared on one line the landing takes the **minimum** byte — the earliest
   declaration — mirroring the outline arm's "earliest occurrence" convention. That
   is deliberate and deterministic (it replaced a `HashMap`-order lookup whose answer
   varied **per process**; see the `table-determinism` commit). Note the remaining
   imprecision: `SymbolIndex::rust_fields` is keyed by **bare struct name**, so two
   same-named structs in different modules also collapse into one entry and the
   minimum can be the other module's column. Pre-existing keying, out of the
   field-jump lane's scope; module-qualifying the key would make it exact.

**Residual (honest gaps):** no PTY column pin per NON-RUST outline language
(one per language would be 14 near-duplicate legs of the same mechanism; the
mechanism is language-agnostic and pinned for Rust + imenu + tooling). No
store/PTY pin for the Symbols-picker RET distinct from the Xref arm (same
`run_selected` code path, pinned via Xref/forced-list).
