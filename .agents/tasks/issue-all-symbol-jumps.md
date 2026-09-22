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
