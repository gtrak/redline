# Task: A1 — decompose `Root` (504 lines, six jobs in one function)

`src/ui/root/mod.rs:42` — `pub fn Root(mut hooks: Hooks) -> impl Into<AnyElement<'static>>`
is **504 lines with nesting depth 7**, and it does six unrelated jobs. This is the
largest single piece of unorganized logic in the tree, measured under the user's
criterion (*complex logic organized for maintainability*, not file size — see
`.agents/skills/plan-process/SKILL.md` § Maintainability criterion).

## The measured structure (re-derive before relying on it)

| Lines | Job | Candidate home |
|---|---|---|
| 42–83 | contexts (`store_handle`, `store`, `system`), terminal size + width resolution, `tick` state | stays in `Root` |
| 84–143 | the **terminal event handler** closure (~60 lines) | `use_terminal_events(...)` helper |
| 152–266 | **five `use_future` blocks** (~115 lines: watcher, index, search, …) | `use_app_futures(...)` helper(s) |
| 268–359 | the **`Snapshot { … }` literal** (~92 lines, 66 fields) | `snapshot.rs`: a builder fn |
| 381–394 | the **cursor effect** | `use_cursor_effect(...)` helper |
| 398–400 | quit | stays in `Root` |
| 402–463 | the **view dispatch** match (9 `ViewId` branches) | `render.rs`: `render_view(&Snapshot)` |
| 464–546 | the **frame assembly** (~82 lines: outer `View`, row/column layout, `Minibuffer` + `StatusLine` props) | `render.rs`: `render_frame(...)` |

**Goal:** `Root` becomes ~40–60 lines that compose these, and each job is a named
function in a file named for it.

## Key decisions

- **This is a pure extraction — no behaviour change.** Statements move into named
  functions; no logic is rewritten, no condition inverted, no ordering changed.
- **Hook order is the critical risk.** iocraft (like React) requires hooks to be
  called **unconditionally and in the same order on every render**. When you lift
  `use_terminal_events` / `use_future` / `use_effect` calls into helpers, the
  *call sites* in `Root` must keep their relative order, and no extracted helper may
  become conditional. State in your report how you verified this.
- **This is NOT the rejected `Snapshot::from_store` change.** That proposal was to
  make the 66 fields *private* via a constructor + ~60 accessors, and the gate
  verified it is a bad trade (the fields are read from four sibling sites; the
  render path needs `&mut` store accessors). **This task only moves the
  literal-building block into a function** — the fields stay `pub(super)` exactly as
  they are, and no accessor is added. If you find yourself widening or narrowing
  visibility, you have drifted; stop and report.
- **Name each helper for its job**, not for its position (`render_view`, not
  `root_part_2`).
- **`render_view`/`render_frame` return types.** The view match yields
  `Option<AnyElement<'static>>`; the frame yields the outer `element!`. Keep the
  exact types — `element!` expands to a type that must be returned as
  `AnyElement<'static>`; do not "simplify" the signatures.
- **Preserve every comment.** This function carries a lot of decision history (the
  width/`tw_raw` reasoning, the cursor-effect rationale, the `ViewId` branch notes).
  A mechanical extraction that drops comments is a P1 — report the comment-line
  count before and after.
- **Do not touch the other root modules' logic** (`geometry.rs`, `input.rs`,
  `widgets.rs`, `render.rs`'s existing `render_at_width`). You may add to
  `render.rs` and `snapshot.rs`.

## Files

| File | Change |
|---|---|
| `src/ui/root/mod.rs` | `Root` shrinks to composition; helpers called in the same order |
| `src/ui/root/snapshot.rs` | + the snapshot builder fn |
| `src/ui/root/render.rs` | + `render_view`, `render_frame` |
| `src/ui/root/hooks.rs` (new, or a name you justify) | + the effect/future/event helpers |

Fence: `src/ui/root/*` only.

## Verification

- `cargo build`; `cargo test --workspace` (reconcile the `test result:` lines against
  the baseline: redline 854 passed / 0 failed / 2 ignored, resolver 123/0/4,
  integration 7,2,3,1,1, doctests 0 — **no count may change**);
  `cargo clippy --workspace --all-targets` (read `${PIPESTATUS[0]}`);
  **`timeout 900 tools/gate.sh full`** — the rendering path is exactly what this
  touches, so the PTY battery is the real check, not the unit tests. If swap is
  exhausted, run `cargo test --workspace` + `drive_windowing.py` +
  `drive_windowing_panes.py` and report the battery as DEFERRED (never as a
  failure). `check_cursor_stream.py` fails only under concurrent build load.
- Report: the new line count of `Root`, each helper's location and size, the
  hook-order verification, comment-line accounting, and the before/after function
  list from a brace-matching survey (`Root` should no longer appear in the
  "longest functions" list).
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.
