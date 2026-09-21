# Task: delete the `Xref` trait (D2)

## Why
`src/nav/xref.rs` defines a `pub trait Xref` with **exactly one implementor**
(`impl Xref for SymbolIndex`) and its only dynamic uses are in its own test module
(`&dyn Xref`, `Box<dyn Xref>`). The store calls `SymbolIndex`'s methods directly
(`definitions_of`, `outline`, `all_locations`, `file_count`). Its `#[allow(dead_code)]`
justification was "the LSP backend will use it later" — but **no LSP is planned** (a
standing user decision), so the seam is a pass-through to itself and the justification is
a lie. The cleanup audit's recommendation, and the supervisor's decision, is to **delete
it** and keep the inherent methods.

## Required outcome
- Delete the trait (and its `#[allow(dead_code)]` + the stale comment).
- Keep `SymbolIndex`'s inherent methods; if the trait carried any method the inherent
  impl does not have, move it over (there should be none — verify).
- Update the test module that used `dyn Xref`/`Box<dyn Xref>` to call the inherent
  methods instead. **Do not weaken those tests**: they should still exercise the same
  behaviour through the concrete type. If a test existed *only* to exercise the trait
  indirection (e.g. asserting a `Box<dyn Xref>` dispatches), say so and either drop it
  with that reason or re-express it against the concrete type — state which.
- Check for other references: `src/nav/mod.rs` re-exports, the store, `ui/`, docs. Update
  any doc that describes the LSP seam as if it exists.

## Fence
`src/nav/xref.rs`, `src/nav/mod.rs` (if it re-exports), and docs. Nothing else — in
particular do NOT touch `src/ui/root.rs` or `tools/check_cursor_stream.py` (another lane
owns them).

## Gate
`cargo build`; `cargo test --workspace`; `cargo clippy --workspace --all-targets` (read
`${PIPESTATUS[0]}`); `timeout 900 tools/gate.sh full` if swap has headroom (the box has
been swap-exhausted; if so, run the workspace tests and report the battery deferred).
Budget ~20 tool calls. Report: what was deleted, how the former `dyn` tests are expressed
now, any doc updated, gate counts.
