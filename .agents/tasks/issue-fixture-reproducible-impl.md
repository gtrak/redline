# Task spec — infra: make the shared PTY fixtures reproducible (W21)

## Problem (measured)

`tools/gate.sh full` fails ~12 suites at SETUP because the shared fixture
`<REDLINE_FIXTURE_ROOT>/redline_pyte_repo` does not exist and **nothing
creates it**. `tools/fixture.py::reset()` only RESETS an existing repo
(`git checkout -- .` + marker re-apply); it has no `git init`/`mkdir`. The
fixture was a developer-machine `/tmp` artifact; the `/tmp` cleanup took it.
Same for `redline_tall_repo` (only `redline_win_diff_repo`,
`redline_sweep_slow_repo`, `redline_ext_*`, `redline_control_repo`,
`redline_018_02_isearch_repo` self-create). Consequence: **a fresh clone
cannot run the PTY battery.**

## Goal

Make BOTH missing fixtures reproducible from the committed repo, so
`tools/gate.sh full` is green on a clean machine — WITHOUT weakening any
drive assertion.

## Two fixtures to create

1. `<root>/redline_pyte_repo` — the main shared fixture (30 tool references).
2. `<root>/redline_tall_repo` — 30 changed files so the magit status buffer
   exceeds the 19-row window (`tools/drive_windowing.py`,
   `tools/drive_all.py::drive_windowing`).

## Deliverable

- `tools/fixture.py` gains a **`create()`** function (and
  `if __name__ == "__main__"` calls `create()` then `reset()` so
  `python3 tools/fixture.py` is the one-command rebuild). It must be
  **idempotent** (safe to run when the repo already exists — reset it) and
  must `git init` a fresh repo when absent.
- A committed **content spec** for the fixture inside `fixture.py` (the file
  contents + commit sequence + strays live as data in the module, so the
  fixture is defined by the repo, not by a machine).

## How to derive the fixture (THE method — do not guess)

**The drives are the frozen oracle.** Do NOT read the drives and hand-derive
values. Instead:

1. Write a first-cut `create()` from the source-of-truth you DO have:
   - the **unit twins** in `src/app/flow_tests.rs` build the tracked seed
     inline: `fixture_repo()` (README `# commit fixture`, `src/lib.rs`
     `pub fn target_lib() {}\npub fn other_lib() {}\n`, `src/main.rs` =
     `MAIN_RS`, `Cargo.toml` `[package]\n`) and `tall_magit_repo()` (30 ×
     `f00.txt..f29.txt` = `base\n`, committed, then `change\n` appended).
   - the strays' NAMES come from `tools/drive_tree_narrow.py`
     (`fieldtype.rs`, `indented_demo.rs`, `nestedtype.rs`, `sym_cjk.rs`,
     `sym_demo.rs`) and their letter PROPERTIES from the drive's query
     expectations (`lib` → only `lib.rs`; `ma` → only `main.rs`; `m` → 5 rows
     README + indented_demo + main + sym_cjk + sym_demo).
2. Run the drives. Each prints the EXACT expected value on mismatch
   (`left/right`, `got X want Y`, row sets). Iterate `create()` until the
   drives pass.
3. **NEVER edit a drive to make it pass.** If a drive's expectation is
   genuinely unsatisfiable, STOP and report it — do not relax it.

### Known requirements to satisfy (starting hints, verify by running)

- `redline_pyte_repo` git log: **6 commits** — `init`, `commit 1` .. `commit 5`
  (`tools/drive_log.py::expected_subjects`). The unit twin's 3-commit
  `fixture_repo()` is a SIMPLIFIED mirror, not the real log.
- Working-tree baseline: `src/lib.rs` **staged** (`git add`), `README.md`
  **unstaged**; markers `staged_change_marker` / `unstaged_change_marker`
  (`tools/fixture.py::reset` + the home dirty-count `+1 ~1 ?0`).
- Search `target`: **8 hits in 3 files — 2 README.md, 1 src/lib.rs,
  5 src/main.rs** (`tools/drive_results_narrow.py` → so README carries
  `target`-bearing text, e.g. `# target README ...`; `drive_search.py` and
  `drive_results_narrow.py` assert `"target README"`).
- `src/main.rs` = the unit twin's `MAIN_RS` (has `target_one`, `target`,
  `fn main`) — 5 `target` hits; verify the count.
- 8 tree rows in this ORDER: README.md, then `src/lib.rs`, `src/main.rs`,
  `src/fieldtype.rs`, `src/indented_demo.rs`, `src/nestedtype.rs`,
  `src/sym_cjk.rs`, `src/sym_demo.rs` (drive_tree_narrow rows 2..9 and the
  arrow-walk identity: last row = sym_demo, then sym_cjk, then main).
- `sym_cjk.rs` must carry a **CJK symbol** (drive_buffer_list_narrow opens
  it as the MRU-current buffer; the name is asserted).
- `redline_tall_repo` = `tall_magit_repo()` shape (30 files with a change),
  committed once so 30 files are dirty.

## Tools to also fix (same root cause)

- `tools/gate.sh` seeds a non-`/tmp` root by copying `/tmp/redline_*`
  baselines; if the baseline is absent, that copy is empty. Make the gate
  **ensure the fixture exists** (call the builder) OR document+enforce that
  the builder runs before the battery. Prefer: `gate.sh` calls
  `python3 tools/fixture.py` once before the shared suites (cheap, idempotent).

## Verification (all must be run; redirect to files, then read)

```
python3 tools/fixture.py > /tmp/fx.log 2>&1; echo FIX=$?          # rebuild
REDLINE_FIXTURE_ROOT=/tmp/fxw tools/fixture.py > /tmp/fxw.log 2>&1; echo FIXW=$?   # fresh root
cargo build --workspace > /tmp/b.log 2>&1; echo BUILD=$?
cargo test --bin redline > /tmp/t.log 2>&1; echo TEST=$?
REDLINE_FIXTURE_ROOT=/tmp/fxg timeout 1800 tools/gate.sh full > /tmp/gate.log 2>&1; echo GATE=$?
```
Read `/tmp/gate.log`: EVERY shared suite must be `OK`. Report the per-suite
OK/FAIL table and the suite counts.

## Constraints

- No new dependencies.
- Do NOT modify any `tools/drive_*.py` / `sweep*.py` / `check_cursor_stream.py`
  / `pyte_driver.py` assertion. Adding a fixture CREATOR is fine; changing an
  expectation is not. If you must touch a tool, disclose exactly what and why.
- Commit at first green; keep working.

## Report format
Files changed; the fixture content spec (files, commits, strays); the per-suite
gate table; any drive you could not satisfy (with the exact mismatch output);
deviations.
