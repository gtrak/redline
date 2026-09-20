# Task: investigate + fix — same-file M-. jump-back lands at the wrong place

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `.agents/skills/*.md` and the jump-stack machinery (007-03 era +
the watchlist lane's M-, changes).

## Origin (user report, live usage)

"When I jump to definition in the SAME file, popping back can go to the
wrong place, not where my cursor was." M-, after a same-file M-. should
land exactly at the point (line AND column) the jump started from.

## What to do

1. **Reproduce first**: trace the same-file M-. flow end-to-end —
   `symbol_at_point` → the unique-def path (`xref_find_definitions`'s
   same-file arm) → `record_jump` (origin captured via
   `current_jump_entry`) → landing (`set_point_line` +
   `recenter_landing`) → `M-,` → `navigate_to_entry` (line + col
   restore, recenter). Find where the origin's line/col can go wrong:
   candidates: (a) the origin captured AFTER the point moved (ordering
   of capture vs the navigation), (b) col restore lost (point_col vs
   the entry's col), (c) the 010-01/010-03 pre-steps consuming the
   token BEFORE origin capture (ordering changed by the recent lanes),
   (d) the scroll/point restore disagreement (recenter restores
   scroll; the point restore may be missing/wrong for the same-file
   case), (e) the watchlist lane's `navigate_to_entry` Search-close
   addition interfering.
2. **Write the discriminating regression test FIRST** (a twin that
   reproduces the report: same-file M-. from a known point with other
   symbols around, M-., assert M-, restores BOTH point line AND column
   AND scroll) — it must FAIL before the fix.
3. **Fix**; byte-for-byte degradation elsewhere (cross-file M-,
   unaffected — verify the cross-file twin still passes).
4. Also audit the OTHER jump entry points for the same origin-capture
   bug (imenu, xref picker, external landings, Rung 4's Impls picker —
   they share `record_jump`).

## Constraints

- Gate: `cargo test --workspace` + clippy (PIPESTATUS exit) +
  `tools/gate.sh full` (flock, cargo build first). Budget ~40 tool
  calls; honest-stop at half.
- Scope fence: `src/app/store.rs`, `src/app/flow_tests.rs`. NOTHING
  else. A parallel lane owns main.rs + the event loop — do not touch.

## Review P2s (recorded; reviewer said no fix required for merge)

1. **Sentinel clobber under a late async landing** (store.rs sync + capture sites for
   `open_resolved_source` / external Jump arm): if a pending resolver landing arrives
   while the user has M-, back to a search sentinel, the sync overwrites the sentinel
   with the true buffer point; a later M-, then lands at the file point instead of
   reopening the results view. Reachable only inside the resolver's in-flight window
   after search-RET + M-,; no crash; direction matches the fix's invariant (the old
   code dropped the file position and kept a stale view marker instead).
2. **`JumpEntry.label` is internal-only**: the sync rewrites `history[pos].label` to
   `""` in the no-motion case; unobservable because no production code reads `.label`
   (verified exhaustively). Document the field as internal, or label origins, if it is
   ever surfaced.
