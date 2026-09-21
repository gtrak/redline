# 013-02 — vendored iocraft patch (contingent on 013-01)

**Objective.** If the upstream seam is not available when it is wanted, own the minimal
patch locally and delete redline's 12 ms race.

**This issue is contingent:** do not start it unless the user chooses option B (§5 of
`PLAN.md`).

## Key decisions

- **Patch the smallest possible surface.** In `write_canvas`'s fullscreen branch, right
  after the park (`iocraft-0.9.1/src/backend/crossterm.rs:583-586`), queue
  `Show` + `MoveTo(col, row)` from a cursor position the backend holds. Add one public
  setter (or the `use_cursor_position` hook if 013-01 landed a shape worth matching).
  Expect ~20-30 lines.
- **Keep the emit point inside the region.** It must be after the park and before
  `end_frame` closes `?2026l` — that is the fix. Getting this wrong reproduces the
  original clobber.
- **Pin explicitly.** `[patch.crates-io] iocraft = { git = "…", rev = "…" }` or a vendored
  path dep — and record the **upgrade liability** in the commit message: every future
  iocraft bump must re-apply or drop this patch. The pin is 0.9.1 today.
- **Delete the workaround, don't layer on it.** The redline-side change is the *point* of
  the issue: remove the `tokio::spawn` + `sleep(12ms)` (`src/ui/root/mod.rs:381-394`) and
  the raw `crossterm::execute!` escape. If the fix lands but the sleep stays, nothing was
  fixed.
- **No behaviour change beyond the cursor.** The frame content path must be untouched.

## Files

| File | Change |
|---|---|
| vendored/forked iocraft | the patch (park → emit cursor) |
| `Cargo.toml` | `[patch.crates-io]` / path dep |
| `src/ui/root/mod.rs` | delete the spawn+sleep; call the new API |

## Steps

1. Fork/pin iocraft; apply the patch; confirm the patch alone changes nothing when the
   cursor position is unset.
2. Replace the redline workaround with the new API.
3. Verify per §Verification.

## Verification

- `check_cursor_stream.py` passes **under induced concurrent load** — the only condition
  that discriminates (it is already 80/80 idle, per `PLAN.md` §3).
- The `sleep(12ms)` and the detached `spawn` are gone: `rg 'from_millis\(12\)' src/` is
  empty, and no app code calls `crossterm::cursor::{Show,MoveTo}`.
- `cargo test --workspace` + `clippy --workspace --all-targets` clean; `gate.sh full` OK.
- The patch's upgrade liability is written into the commit message and `PLAN.md`.
