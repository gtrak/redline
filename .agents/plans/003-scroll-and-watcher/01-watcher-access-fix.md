# 01 — Watcher Access fix

Phase 1 · Depends on: —

## Objective

The watcher's own reads no longer count as project changes: access-only
batches publish nothing, the self-sustaining ~500 ms reload loop is gone,
and typing in a locally-owned buffer never raises a false "changed on
disk" marker.

## Key decisions

- Fix in `watcher.rs` `summarize`: an event batch containing ONLY access
  events (notify kinds that map to `ChangeKind::Other` today — Access(Open)
  etc.) publishes no change. Mixed batches publish their non-access events
  as before.
- No mtime guards in the store; `created_paths` suppression (issue 05)
  stays as-is for the app's own creates.
- The existing noise filter (`.git/`, `redline.log`) is untouched; this
  change is about the notify-event KIND, not paths.

## Files

| File | Change |
|---|---|
| `src/app/watcher.rs` | `summarize`: access-only batches → no events; access events in mixed batches are dropped, real kinds kept. |
| `src/app/watcher.rs` (tests) | access-only batch → no publish; mixed batch → only the modify arrives; positive control (a live watcher proves liveness, per `git_and_log_noise_produce_no_event`'s pattern). |
| `tools/sweep_flows.py` | `flow_g3` reworked: marker raised ONLY by the real disk append; a type-then-idle leg asserts NO marker appears (the old drive leaned on the Access bug). |
| `docs/ux-testing-plan.md` | NEW FINDING row flipped to FIXED with evidence. |

## Steps

1. Classify notify event kinds in `summarize`; drop pure-access.
2. Unit tests at the watcher layer (with positive control).
3. Store test: locally-owned buffer + access-only batch → no marker.
4. Rework `flow_g3`; add the type-then-idle clean leg.

## Verification

- `cargo build` / `clippy -D warnings` / `cargo test` all green.
- `tools/sweep_flows.py` 39/39 (G3 reworked, all others untouched).
- PTY: type into notes, idle 5 s → no marker, no repeating batches in
  redline.log; then external disk append → marker appears, reload clears.
