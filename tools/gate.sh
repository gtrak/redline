#!/usr/bin/env bash
# Redline gate runner — tiered so the inner dev loop doesn't pay for the
# full PTY battery on every iteration (it is a RELEASE gate).
#
#   tools/gate.sh fast      Rust-only: build + clippy + unit tests      (~30 s)
#   tools/gate.sh smoke     fast + one PTY chain (drive_all)            (~90 s)
#   tools/gate.sh full      everything at the fast 0.06 quiet
#                           (PTY battery ~255 s, ~4.3 min wall; at the old
#                           0.2 window the battery measured ~415 s on the
#                           same suites — loop-02)
#   tools/gate.sh full-fast redundant alias for `full` since loop-02
#                           (the 0.06 window IS the default; kept for old scripts)
#   tools/gate.sh equiv     A/B the quiet window (0.2 vs 0.06) across the
#                           full battery — run once after changing the driver
#   tools/gate.sh pooled    fast + the PTY battery POOLED across private
#                           per-lane fixture copies (tools/pool.py): the
#                           12-suite battery in ~117 s (lanes=4, 0.06 quiet)
#                           vs full's ~255 s serial; same verdicts.
#                           Full stays the sequential, always-works fallback.
#
# Latency knobs (see tools/pyte_driver.py):
#   REDLINE_PTY_QUIET  read-quiet window; default 0.06 (fast, loop-02 — every
#                      timing-sensitive sweep_flows assertion is now
#                      positive-gated, and the battery is proven
#                      verdict-identical at 0.06). Escape hatch: run at the
#                      old safe window with REDLINE_PTY_QUIET=0.2.
#   REDLINE_BIN        binary under test (default target/debug/redline).
#   REDLINE_POOL_LANES lane count for the `pooled` tier (default 4; a 24-core
#                      box measured well at 4-6 lanes).
#
# Every PTY invocation is under `timeout` and the shared PTY flock still
# refuses to run two fixture-touching suites at once (backlog #13).
set -u
cd "$(dirname "$0")/.."

TIER="${1:-full}"
# Latency policy (loop-02): the FAST window is the default for every tier.
# sweep_flows' former fixed-sleep orderings (U-BHN banner debounce, ann-delete
# transient message, and the absence-style assertions the audit flagged) are
# now positive-gated (wait_for on the render-completion signal), so a short
# quiet window can no longer turn a missing repaint into a vacuous pass.
# An explicit REDLINE_PTY_QUIET from the caller always wins (escape hatch,
# e.g. REDLINE_PTY_QUIET=0.2 for the old conservative window).
_default_quiet=0.06
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
  drive_issue_011_01.py # language-dispatch leg (011-01): a python buffer
                        # attempts exactly ONE provider, cargo never probed
  drive_issue_011_02.py # per-language scope hints (011-02): a bare imported
                        # `dumps` lands in the json stdlib source; prelude
                        # `print` (no import) bails byte-for-byte
  drive_issue_011_04.py # per-language source index (011-04): the python
                        # stdlib tree IS indexed after landing (the
                        # `indexing crate` indicator appears), and a second
                        # M-. INSIDE the dependency jumps in-crate through
                        # the freshly built index
  drive_issue_011_05.py # resolver parity (011-05; L-P2/L-J1a re-pinned
                        # to the 011-06 truth — the dotted LANDINGS,
                        # superseding the pre-011-06 bail pins): python
                        # legs live (bare-import landing; json.dumps
                        # lands; external blame bails) + js legs live
                        # (namespace entry lands; ns.member lands on the
                        # member's own line; in-crate follow-up); go leg
                        # skips LOUD when the toolchain is absent
                        # (unit-covered only — never a silent pass)
  drive_issue_011_06.py # language-aware M-. tokens (011-06): the dotted
                        # use sites LAND live — python `json.dumps` in
                        # the stdlib json source, js `fakelib.apply` in
                        # the package entry file (the two changed
                        # matrix cells); loud skip per runtime when
                        # absent; go stays unit-covered (no leg)
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
    # Redundant since loop-02: the 0.06 window is now the DEFAULT for every
    # tier (sweep_flows' timing-sensitive flows are positive-gated; the
    # battery is proven verdict-identical at 0.06). Kept as a no-op alias
    # so older scripts keep working.
    REDLINE_PTY_QUIET=0.06 bash "$0" full || fail=1
    ;;
  pooled)
    # Fast full: Rust gate + the PTY battery run POOLED across private
    # per-lane fixture copies (tools/pool.py). Each concurrently-running
    # suite gets its own fixtures + a per-lane XDG_CACHE_HOME, so the
    # driver's abspath-keyed flock never serializes them. Same verdicts as
    # serial `full`: measured ~117 s vs ~255 s serial (lanes=4, 0.06 quiet,
    # loop-02). Lanes are exclusive (one suite per lane). `full` remains the
    # sequential, always-works fallback (pool lanes live under REDLINE_POOL_ROOT
    # [default /tmp/rl]; `tools/pool.py clean` removes them). Note: a MANUAL
    # `pool.py runall` without REDLINE_PTY_QUIET in the environment falls back
    # to pool.py's own conservative 0.2 default; this tier exports 0.06.
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
