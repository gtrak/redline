#!/usr/bin/env bash
# Redline gate runner — tiered so the inner dev loop doesn't pay for the
# full PTY battery on every iteration (it is a RELEASE gate).
#
#   tools/gate.sh fast      Rust-only: build + clippy + unit tests      (~30 s)
#   tools/gate.sh smoke     fast + one PTY chain (drive_all)            (~90 s)
#   tools/gate.sh full      everything at the fast 0.06 quiet
#                           (PTY battery ~150 s after loop-03 demoted ~50 of
#                           sweep_flows' 65 flows to unit twins — before:
#                           ~255 s; at the old 0.2 window ~415 s, loop-02.
#                           loop-04 demoted the other 7 suites to thin
#                           smokes + 20 unit twins: the converted legs are
#                           3-21 s each now, but the battery total is still
#                           dominated by the kept heavy suites)
#   tools/gate.sh full-fast redundant alias for `full` since loop-02
#                           (the 0.06 window IS the default; kept for old scripts)
#   tools/gate.sh equiv     A/B the quiet window (0.2 vs 0.06) across the
#                           full battery — run once after changing the driver
#   tools/gate.sh pooled    fast + the PTY battery POOLED across private
#                           per-lane fixture copies (tools/pool.py): the
#                           12-suite battery in ~83-110 s (lanes=4, 0.06 quiet,
#                           post-loop-04: 82.7 s measured at 12/12; the first
#                           post-conversion run was 106 s with a dropped
#                           M-. keypress in the thin drive_external_crate,
#                           fixed same session; pre-loop-03: ~117 s) vs
#                           full's ~150 s serial; same verdicts. Full stays
#                           the sequential, always-works fallback.
#
# Stale-binary trap (loop-03): the pooled lanes run the binary pool.py
# builds ONCE from the CURRENT checkout — if you `git checkout` another
# lane's tree without rebuilding, the lanes silently test the wrong code
# and the pooled gate reports green. Pin the tree under test explicitly:
# REDLINE_BIN=$(pwd)/target/debug/redline tools/gate.sh pooled (and the
# lane checkouts must match it — pool.py rebuilds from the checkout it
# was started in, so start the pool from the tree being tested).
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
#   REDLINE_FIXTURE_ROOT  fixture tree root for every PTY suite (default /
#                      per-invocation /tmp/fx<pid>, set below; an explicit
#                      value from the caller wins). Two concurrent gates get
#                      disjoint roots, so their suites can no longer collide
#                      on shared fixtures (cross-lane contention class).
#
# Every PTY invocation is under `timeout` and the shared PTY flock still
# refuses to run two fixture-touching suites at once within ONE root
# (backlog #13).
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

# Per-invocation fixture root: every hardcoded tools/ fixture path resolves
# as <root>/<basename> (tools/fixture.py repo()), so two concurrent gate.sh
# runs get disjoint fixture trees and cannot collide. Keep it SHORT: the
# fixture path lands on the 80-col status line. An explicit
# REDLINE_FIXTURE_ROOT from the caller always wins. A root other than /tmp
# is seeded once from the /tmp baseline (basenames are preserved — the
# suites assert on them).
# caller-supplied root is remembered as-is; gate.sh's own default gets a
# cleanup marker so a green run removes its /tmp/fx<pid> tree.
_fixture_root_defaulted=0
if [ -z "${REDLINE_FIXTURE_ROOT:-}" ]; then
  REDLINE_FIXTURE_ROOT="/tmp/fx$$"
  _fixture_root_defaulted=1
fi
export REDLINE_FIXTURE_ROOT
if [ "$REDLINE_FIXTURE_ROOT" != "/tmp" ] && [ "$TIER" != "pooled" ]; then
  # Seed any non-/tmp root (gate default OR caller-supplied) from the /tmp
  # baseline. (pooled tier: the gate root is unused — pool.py's lanes ARE
  # the fixture roots and setup copies their baselines itself.)
  mkdir -p "$REDLINE_FIXTURE_ROOT"
  for d in /tmp/redline_*; do
    [ -d "$d" ] || continue
    b="${d##*/}"
    case "$b" in redline_pty_*.lock) continue;; esac
    [ -e "$REDLINE_FIXTURE_ROOT/$b" ] && continue
    cp -a "$d" "$REDLINE_FIXTURE_ROOT/"
  done
fi

# Shared-fixture suites: MUST run one at a time (the driver's flock enforces
# this and exits 3 if a rival is live). Ordered cheapest-first for a fast fail.
SHARED_SUITES=(
  sweep.py
  drive_all.py          # 8 scenarios incl. the per-repo external children
  drive_buffer_list_narrow.py # 018-04: C-x C-b narrowing (prompt keys lead,
                              # live narrow, RET opens, d kills the selected
                              # narrowed row, C-g clears not closes, q closes)
  drive_results_narrow.py     # 018-03: C-c p s s results narrowing (prompt
                              # keys lead, live narrow, RET jump, M-, return
                              # with selection, C-g clears not cancels idle,
                              # g re-run clears the query, q closes)
  drive_windowing.py
  drive_windowing_panes.py
  check_cursor_stream.py
  ux_sweep.py
  probe_notes_dump.py
  probe_current_line_tint.py # issue-current-line-highlight: the point row
                        # carries the tint background per cell (232323),
                        # the neighbours the view background (000000), the
                        # tint follows the point on C-n, and the region
                        # face wins over the tint on the region's rows
  probe_current_line_tint_16.py # issue-current-line-highlight (follow-up):
                        # the 16-colour fallback (DarkGrey 48;5;8 -> 7f7f7f)
                        # when COLORTERM is not truecolor
  drive_syntax_notes.py
  drive_symbol_precise.py # symbol-precise annotation anchoring: the indicator
                        # sits before the annotated SYMBOL (the record's
                        # column, char->display converted), incl. wide-char
                        # and tab fixtures and the no-whitespace fallback.
                        # Registered by the gate (P2-3): the 27 legs were
                        # reproducible only by a manual run before this.
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
  drive_issue_017_f2.py # plan-017 finding F2: the provider-miss detail
                        # reaches the minibuffer — a DECLINED fetch names
                        # the refusal (the gate's own reason + the exact
                        # command NOT run), an ordinary miss leads with
                        # the provider's own reason, and a no-refusal
                        # multi-provider walk keeps the generic shape
                        # (no unrelated-provider noise)
  drive_redo_live.py # issue-redo-live-leg: the PTY leg for redo (C-x U /
                     # C-M-7). Type a run, C-x u reverts, C-x U (and the
                     # byte-based C-M-7, raw ESC 0x1F) restore it
                     # byte-identically, and the point lands at the START of
                     # the redone insertion (oracle-pinned, emacs 30.2).
                     # Registered by the gate (016-04 item 3): before this
                     # the new user-facing redo binding had no drive at all.
  # The four genuine registration gaps (issue-battery-drive-registration):
  # each carried real expectations but was in neither list, so its failures
  # were invisible to the battery. All four are hermetic (a redline App + the
  # shared fixture; no emacs, no network) and exit non-zero on a failed check.
  drive_log.py        # magit log view (MagitRowsView): exactly ONE blue
                     # selected row at every in-page step, and the cursor is
                     # on the commit log.selected points at (down/up).
  drive_tree.py       # file-tree sidebar (tree.selected): exactly ONE blue
                     # row at every step, the file under the cursor, as
                     # down/up walks the tree.
  drive_search.py     # search results view (search.selected): exactly ONE
                     # blue hit row at every n/p step; exits non-zero if the
                     # search never finishes (no match count).
  drive_isearch_list.py # plan 018 issue 02: isearch as a list (helm-occur
                     # shape): C-s opens the match rows (line number + line
                     # text + the match's column) in place of the buffer
                     # content, typing narrows (the query IS the search),
                     # C-s/C-r move the selection (the highlight band follows
                     # behind the list), RET confirms the SELECTED match's
                     # column (CUP, multibyte fixture), and C-g restores the
                     # pre-search line AND column with the highlight gone.
  drive_magit.py      # magit status view cursor trajectory: exactly ONE
                     # full-bar blue row per step; n/p advance/retreat the
                     # cursor one row through the sections.
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
cargo_build()  { iocraft_patch_check; cargo build --workspace; }
cargo_lint()   { cargo clippy --workspace --all-targets -- -D warnings; }
cargo_tests()  { cargo test --workspace --quiet; }
pty()          { timeout 900 python3 "tools/$1"; }

# Plan 013-02 (fork form): redline depends on a PATCHED iocraft, carried in
# github.com/gtrak/iocraft and pinned by rev in [patch.crates-io]. Two things
# must hold, and this guards the one the compiler cannot see:
#   1. the [patch.crates-io] stanza still points iocraft at that GIT source —
#      a deleted stanza, or a stray `cargo update` reverting the lock to
#      crates.io, would silently rebuild against unpatched iocraft;
#   2. the lock resolves iocraft to the same rev the manifest declares, so a
#      half-applied pin change cannot pass.
# The COMPILE is the second backstop and needs no check: redline calls
# `hooks.use_cursor_position`, which does not exist in pristine 0.9.1.
# WHY NOT the old vendor-tree check: it verified a generated tree the build
# no longer uses (a guard aimed at the wrong subject — the tree is now only a
# local convenience for tools/apply-iocraft-patch.sh, which keeps its own
# `check` mode for anyone who wants it).
iocraft_patch_check() {
  local stanza lock_rev manifest_rev
  stanza=$(grep -A1 '^\[patch\.crates-io\]' Cargo.toml | grep '^iocraft' || true)
  case "$stanza" in
    *gtrak/iocraft*) ;;
    *) echo "iocraft patch stanza is missing or no longer points at the fork:" >&2
       echo "  ${stanza:-<absent>}" >&2
       echo "  The build would silently use UNPATCHED iocraft (no use_cursor_position)." >&2
       return 1 ;;
  esac
  manifest_rev=$(printf '%s' "$stanza" | sed -n 's/.*rev *= *"\([0-9a-f]\{40\}\)".*/\1/p')
  lock_rev=$(awk '/^name = "iocraft"$/{f=1} f&&/^source = "git\+/{print; exit}' Cargo.lock \
              | sed -n 's/.*rev=\([0-9a-f]\{40\}\).*/\1/p')
  if [ -n "$manifest_rev" ] && [ "$manifest_rev" != "$lock_rev" ]; then
    echo "iocraft pin mismatch: manifest rev ${manifest_rev} vs Cargo.lock ${lock_rev:-<none>}" >&2
    echo "  Run: cargo update -p iocraft   (then re-run the gate)." >&2
    return 1
  fi
  return 0
}

# The tracker is the single authoritative "what is open?" record. A task id
# carrying BOTH an OPEN row and a LANDED row makes every audit wrong in both
# directions (it lists landed work as open AND hides real gaps behind a stale
# row). Observed 2026-09-27: seven ids did, because landing rows were appended
# instead of flipping the queue row. Cheap python, no cargo. NOTE: this was
# itself silently reverted once, by landing a branch cut before it existed -
# a guard on STATUS.md's CONTENT cannot see that the gate stopped RUNNING it.
tracker_check() { python3 tools/check_tracker.py; }
pool_setup()   { python3 tools/pool.py setup "$1"; }
pool_runall()  { timeout 900 python3 tools/pool.py runall --lanes "$1"; }

case "$TIER" in
  fast)
    run "tracker" tracker_check
    run "build"  cargo_build
    run "clippy" cargo_lint
    run "test"   cargo_tests
    ;;
  smoke)
    run "tracker" tracker_check
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
    run "tracker" tracker_check
    run "build"  cargo_build
    run "clippy" cargo_lint
    run "test"   cargo_tests
    run "pool-setup  (lanes=$LANES)" pool_setup "$LANES"
    run "pool-runall (lanes=$LANES)" pool_runall "$LANES"
    ;;
  full)
    run "tracker" tracker_check
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
# Per-invocation root cleanup: only when THIS invocation created the root
# (caller-supplied roots are the caller's to manage) and only on success —
# a failed gate keeps its tree for forensics.
if [ "$_fixture_root_defaulted" -eq 1 ]; then
  rm -rf "$REDLINE_FIXTURE_ROOT"
fi
printf '\nGATE RESULT: OK (%s)\n' "$TIER"
