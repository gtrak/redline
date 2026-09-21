# Task: split `ui/root.rs` (plan 012 A4) — the last oversized file

## Context
`src/ui/root.rs` is ~1,752 lines and is not just the `Root` component: it is also the
iocraft→app key translation, the cursor/click geometry, the `Snapshot` read model, the
public static-render seam, and two widgets, plus ~789 lines of tests. It is the last file
above the plan's size target. Read the file first and derive the split from what is there
(the design sketch below is from an audit, not a fresh measurement).

## Target layout (adjust if reading suggests better boundaries, and say so)
`src/ui/root/{mod.rs, input.rs, geometry.rs, snapshot.rs, render.rs, widgets.rs}`:
- **`input.rs`** — the iocraft→app key translation (`code_to_app_code`); it maps onto
  `app::keymap::KeyCode`, which is fine (ui may depend on app).
- **`geometry.rs`** — `cursor_cell`, `click_pane` (pure cursor/click math).
- **`snapshot.rs`** — the `Snapshot` read model.
- **`render.rs`** — `StaticRenderWidth` and `render_at_width`.
- **`widgets.rs`** — `Minibuffer`, `StatusLine`.
- **`mod.rs`** — the `Root` component + module declarations + re-exports.
Confirm the real item boundaries by reading; report the actual layout.

## Hard constraints
1. **`crate::ui::root::render_at_width` must keep working unchanged** — app-side tests
   (`src/app/flow_tests.rs`) import it by that path. If it moves to `render.rs`, re-export
   it from `root/mod.rs` so the path is stable. **This is the one external contract.**
2. **`Snapshot`'s fields are read inside `Root`.** They live in a *sibling* submodule now,
   so the compiler will demand visibility — add `pub(super)` (not `pub`/`pub(crate)`) only
   where it is needed, and report the count. (Precedent: the `AppStore`/`GitRepo` splits —
   a child module reaches private items, a sibling does not.)
3. **iocraft components**: `#[component]` functions must still compile after the move
   (macros/imports come along). If a component cannot move cleanly, leave it in `mod.rs`
   and say why.
4. **Pure move, behaviour-preserving.** No logic edits, renames, reordering, or
   "while I'm here". **Prove it faithful**: extract the pre-move file and check every item
   body appears exactly once across the new files with only visibility/`use`/`mod` deltas
   (a comment-stripped multiset partition; state your method and item counts).
5. **Tests**: ~789 lines of tests live in the file. Decide where they go (with their
   subject, or staying in `mod.rs` if they are black-box tests of the component) and say
   which — **no test may be lost, renamed, or weakened**.
6. Fence: `src/ui/root.rs` → `src/ui/root/*` (+ `src/ui/mod.rs` if the declaration needs
   it). **Nothing else** — in particular do not touch `src/app/store/*` or
   `crates/redline-resolve/*`.
7. Honest-stop at half budget: a partial split that is green is a valid landing.

## Gate
`cargo build` first, then `cargo test --workspace`, `cargo clippy --workspace
--all-targets` (read `${PIPESTATUS[0]}`), and `timeout 900 tools/gate.sh full`.
**This file is PTY-visible** (it renders every frame), so the battery matters here — run it
if swap has headroom.
**Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` **and swap**. The box has
been swap-exhausted with unattributed SIGTERM kills of rustc/test harnesses under that
pressure; if swap is exhausted, run `cargo test --workspace` + the render/windowing drives
and report the battery deferred (do not sleep-wait). If a gate run fails, re-run once,
capture the full log, and name the killed/flaking stage. NOTE: `check_cursor_stream.py` is
a **known** loud failure under load (an app-side cursor race, `b47e03c`) — if that is the
only failing suite, say so and do not treat it as your regression.
Budget ~50 tool calls. Report: layout + line counts, faithfulness method + item counts,
the `pub(super)` count, where the tests went, the `render_at_width` path check, gate counts.
