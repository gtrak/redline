#!/usr/bin/env bash
# Redline gate runner — tiered so the inner dev loop doesn't pay for the
# full PTY battery on every iteration (it is a RELEASE gate).
#
#   tools/gate.sh fast      Rust-only: build + clippy + unit tests      (~30 s)
#   tools/gate.sh smoke     fast + one PTY chain (drive_all)            (~90 s)
#   tools/gate.sh full      everything at the safe 0.2 quiet (~3 min)
#   tools/gate.sh full-fast everything at 0.06 quiet (~2 min; see caveat)
#   tools/gate.sh equiv     A/B the quiet window across the full battery
#   tools/gate.sh pooled    fast + the PTY battery POOLED across private
#                           per-lane fixture copies (tools/pool.py) ~2.3 min
#                           vs full's ~3 min serial; same verdicts.
#                           Full stays the sequential, always-works fallback.
#
# Latency knobs (see tools/pyte_driver.py):
#   REDLINE_PTY_QUIET  read-quiet window; default 0.2 = old behavior.
#                      Set 0.06 in the fast loop (~2.9x on PTY suites,
#                      verdict-verified equivalent — tools/gate.sh equiv).
#   REDLINE_BIN        binary under test (default target/debug/redline).
#   REDLINE_POOL_LANES lane count for the `pooled` tier (default 4; a 24-core
#                      box measured well at 4-6 lanes).
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
  full|equiv|pooled) _default_quiet=0.2 ;;
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
  drive_syntax_notes.py
  drive_xref.py
  drive_external_notes.py
  drive_external_crate.py
  drive_external_use.py
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

# `--workspace` is REQUIRED: the workspace's default-members is the root
# package only, so a bare `cargo test`/`cargo clippy` silently SKIPS the
# entire `redline-resolve` crate (~92 tests + all its lints). Found
# 2026-09-19 via 007-03 (the crate gained a public scope field while no gate
# ever compiled its tests). Do not drop these flags.
cargo_build()  { cargo build --workspace; }
cargo_lint()   { cargo clippy --workspace --all-targets -- -D warnings; }
cargo_tests()  { cargo test --workspace --quiet; }
pty()          { timeout 900 python3 "tools/$1"; }
pool_setup()   { python3 tools/pool.py setup "$1"; }
pool_runall()  { timeout 900 python3 tools/pool.py runall --lanes "$1"; }

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
  pooled)
    # Fast full: Rust gate + the PTY battery run POOLED across private
    # per-lane fixture copies (tools/pool.py). Each concurrently-running
    # suite gets its own fixtures + a per-lane XDG_CACHE_HOME, so the
    # driver's abspath-keyed flock never serializes them. Same verdicts as
    # serial `full`, ~2.3 min instead of ~3 min. Lanes are exclusive (one
    # suite per lane); the quiet window stays the safe 0.2. `full` remains
    # the sequential, always-works fallback (pool lanes live under REDLINE_POOL_ROOT
    # [default /tmp/rl]; `tools/pool.py clean` removes them).
    LANES="${REDLINE_POOL_LANES:-4}"
    run "build"  cargo_build
    run "clippy" cargo_lint
    run "test"   cargo_tests
    run "pool-setup  (lanes=$LANES)" pool_setup "$LANES"
    run "pool-runall (lanes=$LANES)" pool_runall "$LANES"
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
    echo "usage: tools/gate.sh {fast|smoke|full|full-fast|equiv|pooled}" >&2
    exit 2
    ;;
esac

if [ "$fail" -ne 0 ]; then
  printf '\nGATE RESULT: FAIL\n'
  exit 1
fi
printf '\nGATE RESULT: OK (%s)\n' "$TIER"
