# Task: hygiene sweep — dead API, stale allows, stale references, twin helpers

Five independent, small, behaviour-preserving cleanups. **Land each as its own
commit** and honest-stop at half budget (each is a valid landing alone). This is
Tier 2 of the post-012 agenda: everything here was found by the scanners
(`tools/cleanup_scan.py dup|deadpub`) or by the round-1/round-2 scans, and none of
it has been actioned.

## Item 1 — delete the dead `pub` constructors (4)

`tools/cleanup_scan.py deadpub` reports zero in-repo references for:

| Symbol | File |
|---|---|
| `with_cargo_bin()` | `crates/redline-resolve/src/cargo.rs` |
| `with_go_bin()` | `crates/redline-resolve/src/providers/go_provider.rs` |
| `with_npm_bin()` | `crates/redline-resolve/src/providers/js_provider.rs` |
| `with_providers()` | `crates/redline-resolve/src/lib.rs` |

**Prove dead before deleting**: `rg` the whole workspace (including `tests/`, and
any string-literal use), and confirm `redline-resolve` is not a published crate
(version `0.1.0`, workspace member) so removing public API is safe. Report the
evidence. If any has a caller the scanner missed, keep it and say why.
Also re-check whether the `Default`/builder impls that construct them become dead
in turn (a cascade is fine — report the final state).

## Item 2 — audit the `#[allow(dead_code)]` set

`rg -c 'allow\(dead_code\)' src/ crates/` → 46 at spec time; **A2 (`6d52129`) added
two more**, so the count is now 48. Two of them are already adjudicated by a gate:
- **`Key::tab` (`src/app/keymap.rs:90`) — genuinely dead, delete it.** It has **zero
  callers in the entire tree, production AND tests**, yet carries an allow and a doc
  comment claiming "used by tests" — which is false for `tab`. (The sibling
  `Key::alt_char` is genuinely test-used, so it stays; add a one-line reason on its
  attribute to match the `store/mod.rs:684` pattern.)
- **`src/app/store/mod.rs`'s 10 keymap tables are `pub const`** — `pub(crate)` is the
  narrowest-correct visibility (consumers are `store/mod.rs` itself and `keymap`'s
  test module; bin-only crate). A7 precedent. Narrow them here. Round 1 found a whole **stale**
cluster in `nav/index.rs` (removed by the `nav-index` lane), so the class is real:
an allow outlives the reason it was added, and it silently hides the next dead item.

For **each** of the 46, decide and record:
- **(a) stale** — the item now has callers → **remove the allow** (clippy/rustc will
  verify; if it then warns, the allow was not stale after all — put it back and
  report the discrepancy).
- **(b) genuinely dead** → **delete the item** (and anything only it used).
- **(c) a deliberate seam** — e.g. `IndexProgress::finished()` is an accurate
  test-only seam — → **keep, with a one-line comment stating why**. An allow with
  no stated reason is the thing being audited.

Report the counts: removed / deleted / kept-with-reason, and the list of kept ones
with their reasons. **Do not blanket-remove** — a pin or a test-only seam is a
legitimate keeper.

## Item 3 — stale references

1. `src/syntax/queries.rs:8` says *"the M-. self-receiver consumption in
   `app/store.rs`"* — `store.rs` is now a directory. Point at the **module**
   (`src/app/store/` or the specific concern file) rather than a file path, so the
   reference cannot re-stale on the next split. (Note: plan 012's A7 lane is
   splitting `store/navigation.rs` — another reason to name the module.)
2. **Two insta snapshots** carry stale `source: src/app/store.rs` metadata:
   `src/app/store/tests/snapshots/redline__app__store__tests__commit__snapshot_log_entry_display.snap`
   and `..._snapshot_blame_line_display.snap`. Update the path; verify by running
   the owning tests (`cargo test --bin redline snapshot_log_entry_display
   snapshot_blame_line_display`) and confirm the snapshots still pass (insta
   rewrites `source:` on bless — do not bless a payload change; if a payload
   changes, stop and report).
3. `point_byte_offset` still lives in `src/app/store/mod.rs` while its siblings
   moved to concern files. Decide: move it to the concern it belongs to, or state
   why it is core. Report the decision either way.

## Item 4 — the `model::file()` twins

`src/model/files.rs:154` and `src/model/project.rs:214` have **identical ~7-line
`fn file()` bodies** (scanner: "identical whole-function bodies"). Unify them into
one shared function (or one method on the type they both build), and check both
call sites' expectations before choosing the name — `file()` is generic enough that
the two may be coincidentally identical rather than the same concept. **If they are
the same concept, unify; if they merely coincide, say so and leave them.** Report
which, with the bodies quoted.

## Item 5 (optional, only if budget allows) — the `flow_tests` `key()` twin

`src/app/flow_tests.rs:29` and `src/app/store/tests/mod.rs:53` both define `fn
key()` (plus `key_char`/`key_null` in the former). They are siblings under
`src/app/`, so a shared home exists (`src/app/test_support.rs`). Only do this if it
does not fight the **git-test-harness** lane, which is creating
`src/test_support.rs` — if that lane has landed, prefer extending its home over
creating a second one. Otherwise **skip and say so**; it is genuinely optional.

## Sequencing

**Do not start while A7 is in flight** — item 3 touches `src/app/store/mod.rs`,
which A7 also edits (mod declarations). If the git-test-harness lane has landed,
item 5's shared-home decision depends on it.

## Fence

`crates/redline-resolve/src/{cargo.rs,lib.rs,providers/*.rs}` (item 1),
`src/**` for `allow(dead_code)` (item 2 — report every file touched),
`src/syntax/queries.rs`, `src/app/store/mod.rs`,
`src/app/store/tests/snapshots/*.snap` (item 3),
`src/model/{files,project}.rs` (item 4), `src/app/{flow_tests.rs,test_support.rs}`
(item 5). **Nothing else** — in particular do not restructure any file you are only
passing through.

## Verification

`cargo build`; `cargo test --workspace` (reconcile the per-target `test result:`
lines against the baseline: redline 854 passed / 0 failed / 2 ignored, resolver
123/0/4, integration 7,2,3,1,1, doctests 0 — a **dropped test count is a P1**, since
item 2 deletes code); `cargo clippy --workspace --all-targets -- -D warnings` (read
`${PIPESTATUS[0]}`); `timeout 900 tools/gate.sh full`.
**Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first; if
swap is exhausted, run the workspace suite + the two windowing drives and report the
PTY battery as DEFERRED, not failed.
Report per item: what changed, the proof (rg output for deletions; the keep/delete
table for item 2; the bodies for item 4), the test-count reconciliation, and
anything you judged differently with the reason.
