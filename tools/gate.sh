#!/usr/bin/env bash
# Redline gate runner — tiered so the inner dev loop doesn't pay for the
# full PTY battery on every iteration (it is a RELEASE gate).
#
#   tools/gate.sh fast      Rust-only: build + clippy + unit tests      (~30 s)
#   tools/gate.sh smoke     fast + one PTY chain (drive_all)            (~90 s)
#   tools/gate.sh full      everything at the safe 0.2 quiet (~3 min)
#   tools/gate.sh full-fast everything at 0.06 quiet (~2 min; see caveat)
#   tools/gate.sh equiv     A/B the quiet window across the full battery
#
# Latency knobs (see tools/pyte_driver.py):
#   REDLINE_PTY_QUIET  read-quiet window; default 0.2 = old behavior.
#                      Set 0.06 in the fast loop (~2.9x on PTY suites,
#                      verdict-verified equivalent — tools/gate.sh equiv).
#   REDLINE_BIN        binary under test (default target/debug/redline).
#
# Every PTY invocation is under `timeout` and the shared PTY flock still
# refuses to run two fixture-touching suites at once (backlog #13).
set -u
cd "$(dirname "$0")/.."

TIER="${1:-full}"
# Latency policy is PER TIER, because the saving is not free everywhere:
#   - Rust-only tiers: irrelevant.
#   - smoke (drive_all): safe at 0.06 (verified verdict-identical).
#   - full: sweep_flows has fixed-sleep orderings that assume the app has
#     settled; a short quiet window exposes two latent races (U-BHN banner
#     debounce, ann-delete message) and gives 64/65 instead of 65/65. So
#     `full` runs at the proven 0.2 default; use `full-fast` to opt into
#     0.06 for everything and accept the known sweep_flows caveat.
# An explicit REDLINE_PTY_QUIET from the caller always wins.
case "$TIER" in
  full|equiv) _default_quiet=0.2 ;;
  *)          _default_quiet=0.06 ;;
esac
export REDLINE_PTY_QUIET="${REDLINE_PTY_QUIET:-$_default_quiet}"

# Shared-fixture suites: MUST run one at a time (the driver's flock enforces
# this and exits 3 if a rival is live). Ordered cheapest-first for a fast fail.
SHARED_SUITES=(
  sweep.py
  drive_all.py          # 8 scenarios incl. the per-repo external children
  drive_windowing.py
  drive_windowing_panes.py
  check_cursor_stream.py
  ux_sweep.py
  probe_notes_dump.py
  drive_xref.py
  drive_external_notes.py
  drive_external_crate.py
)
# sweep_flows is the heavyweight (29 App launches); keep it last so the
# common failure surfaces before it.
HEAVY_SUITES=(sweep_flows.py)

fail=0
run() {  # run <label> <cmd...>
  local label="$1"; shift
  printf '\n=== %s ===\n' "$label"
  local t0; t0=$(date +%s)
  # Progress goes to STDERR, unbuffered, BEFORE the command runs. Without
  # this a `gate.sh full | tail -N` invocation emits nothing for ~3 min
  # and looks hung (it tripped a false long-running watchdog alarm in the
  # orchestrator during 006-03b). stderr is line-buffered to a terminal
  # and never swallowed by `| tail`.
  printf '\n=== %s ... (t=0s) ===\n' "$label" >&2
  if "$@"; then
    printf '=== %s: OK (%ss) ===\n' "$label" "$(( $(date +%s) - t0 ))"
  else
    printf '=== %s: FAIL (%ss) ===\n' "$label" "$(( $(date +%s) - t0 ))"
    fail=1
  fi
}

cargo_build()  { cargo build; }
cargo_lint()   { cargo clippy --all-targets -- -D warnings; }
cargo_tests()  { cargo test --quiet; }
pty()          { timeout 900 python3 "tools/$1"; }

case "$TIER" in
  fast)
    run "build"  cargo_build
    run "clippy" cargo_lint
    run "test"   cargo_tests
    ;;
  smoke)
    run "build"  cargo_build
    run "clippy" cargo_lint
    run "test"   cargo_tests
    run "drive_all" pty drive_all.py
    ;;
  equiv)
    # A/B the read-quiet setting on the full battery: proves the fast
    # window is verdict-identical before it is trusted. Run once per
    # change to the driver, not per iteration.
    for q in 0.2 0.06; do
      printf '\n######## REDLINE_PTY_QUIET=%s ########\n' "$q"
      REDLINE_PTY_QUIET="$q" bash "$0" full || fail=1
    done
    ;;
  full-fast)
    # Accelerated full battery: same suites, 0.06 quiet. Known to drop
    # sweep_flows to 64/65 (timing-sensitive flows, NOT regressions — the
    # old driver on the same binary gives 65/65). Use for the inner loop;
    # gate releases on `full`.
    REDLINE_PTY_QUIET=0.06 bash "$0" full || fail=1
    ;;
  full)
    run "build"  cargo_build
    run "clippy" cargo_lint
    run "test"   cargo_tests
    for s in "${SHARED_SUITES[@]}" "${HEAVY_SUITES[@]}"; do
      run "$s" pty "$s"
    done
    ;;
  *) 
    echo "usage: tools/gate.sh {fast|smoke|full|full-fast|equiv}" >&2
    exit 2
    ;;
esac

if [ "$fail" -ne 0 ]; then
  printf '\nGATE RESULT: FAIL\n'
  exit 1
fi
printf '\nGATE RESULT: OK (%s)\n' "$TIER"
