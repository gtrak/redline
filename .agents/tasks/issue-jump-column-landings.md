# issue-jump-column-landings — jumps still land at column 0 instead of on the symbol

**User-reported (hands-on, after the jump animations landed):** *"I'm noticing some jumps
are still on the beginning of the line and not at the symbol definition, like when I jump
to a function."*

## Why this is a second round

`issue-column-landings.md` (landed `f53c251`) fixed the four callers that **had a column
available and dropped it**: `C-x C-x` exchange, isearch cancel, project-search RET, and the
unique same-file `M-.`. These are different: they are landings that **never had a column**,
because the value travelling through them is a bare line number.

## The seven landings still using the col-0 `set_point_line`

| site | column available? |
|---|---|
| `src/app/store/picker.rs` — **Xref** accept arm (both the crate-relative and the in-project branch) | **YES.** The candidate is `Location { file, symbol }` (`src/nav/index/symbol_index.rs:13`) and `Symbol` carries `start_byte`; the picker offer only encodes the line. |
| `src/app/store/picker.rs` — **Imenu** accept arm | **YES**, but lost in the offer encoding: `name` is `"symbol:line"`. |
| `src/app/store/picker.rs` — **Annotations** accept arm | An annotation is anchored to a *line* (`detail` is `"path:line"`), so column 0 may be **correct** here. Decide and state it rather than assuming. |
| `src/app/store/navigation/mod.rs:75` — the **resolver/tooling** landing (`open_resolved_source`) | **NO.** `ResolvedSource.line` is `Option<u32>` with no column. **This is the path `M-.` on a function usually takes in a Rust project** (the cargo provider), so it is the most likely one the user hit. |
| `src/app/store/navigation/xref.rs:55` — `ExternalXrefOutcome::Jump { file, line }` (external/crate) | NO — line only. |
| `src/app/store/file_view.rs:999` | Determine what this landing is and whether a column exists. |
| the remaining picker/`set_point_line` callers | Enumerate them yourself and classify each. |

## The doc that lies

`crates/redline-resolve/src/lib.rs:101-104` documents `ResolvedSource.line` as the
*"Best-effort 1-based line … `None` when a file was located but no definition line could be
pinned down; **the app refines placement precisely later**."* The app does **not** refine
anything — it calls `set_point_line`, i.e. column 0. Either make that claim true or correct
it. (This is the same class of defect as the `JumpEntry.col` doc that claimed bytes while
holding chars.)

## Required

1. **Wherever the column exists, land on it.** For the Xref picker the data is already
   there (`Location.symbol.start_byte` → `try_byte_to_line_col` → `set_point(line, col,
   col)`); use it rather than re-deriving anything. For Imenu, carry the column through the
   offer (a struct, or extend the encoding) instead of parsing it back out of a display
   string.
2. **For the provider path, either emit a column or do the refinement the doc promises.**
   `ResolvedSource` has no column today, so pick one and justify it:
   * **(a)** add an optional column to `ResolvedSource` and have the providers fill it where
     they can (they parse real sources, so a definition's column is often known);
   * **(b)** do the **app-side refinement** the doc already claims: after landing on the
     line, locate the resolved symbol's token **on that line** and land on its first column
     — with an honest fallback to column 0 when the token is not found (never guess).
   State which you chose and why, and make the doc match.
3. **A landing with no column must degrade to column 0 silently-but-honestly** — not to a
   wrong column. Do not invent a column you do not have.
4. **Fix the byte/char arithmetic on the way through.** Every one of these is a
   byte→(line,col) conversion, which is this project's most recurring defect class. Use the
   existing `try_byte_to_line_col`, and pin a **multibyte** case.

## Acceptance

* A test per fixed landing asserting a **nonzero** column, with a fixture whose symbol is
  **not** at column 0 — the isearch lane's lesson was that its fixture's match sat at column
  0, so the test could not discriminate at all. Say for each test what it would have caught.
* A **multibyte** case (e.g. a definition on a line containing `café`), asserting the
  landing column is a **char** column, not a byte offset.
* A test that a landing with no discoverable column still lands at column 0 (the honest
  degradation).
* The `ResolvedSource.line` doc claim is either true or corrected.
* `cargo test --workspace` + clippy `-- -D warnings` clean; `tools/gate.sh full` green.

## Fence

`src/app/store/picker.rs`, `src/app/store/navigation/{definitions,xref,mod}.rs`,
`src/app/store/file_view.rs`, `crates/redline-resolve/src/lib.rs` (the doc, and a column
field if you choose (a)), and tests in `src/app/store/tests/navigation/`.

**Do NOT touch** `src/app/store/mod.rs`, `src/app/store/tests/picker.rs`,
`src/app/store/keys.rs`, `src/app/store/buffers.rs`, `src/app/store/command.rs`,
`src/app/store/keymap.rs`, `src/app/store/notes.rs`, `src/model/buffer.rs`,
`src/ui/root/mod.rs`, or `src/app/flow_tests.rs` — the `accurate-editing` repair pass holds
those. If the external/crate landing needs `ExternalXrefOutcome` (declared in
`src/app/store/mod.rs`) to carry a column, **report it as a follow-up** instead of making
the change; the in-project paths and the tooling refinement are the valuable part and do
not need it.
