# Task: app-side relative-specifier hint emission (011-08 js review P2-7 contract pin)

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `.agents/skills/*.md` as ground truth.

## Origin

The fix-jsrel merge (`0905a18`, merged `b213fdf`) gave the JS provider a
relative-specifier LANDING feature (`./x` resolves against from_file's
directory: exact file → JS_EXT walk → directory entry-point). The corpus
goldens pin it via a scope hint the app DOES NOT YET EMIT: the app's
`js_ts_scope_for` never hints relative/absolute/side-effect specifiers
(pinned negative: `resolver_scope_js_no_import_and_relative_are_not_guessed`,
src/app/store.rs ~20348) and the probes carry the hint only as a contract
pin. This issue makes the app emit it so LIVE M-. on a relative use site
lands in the sibling file.

## What to build

1. **App-side relative hint emission** in `js_ts_scope_for` (store.rs): a
   use site `const { legacyJoin } = require('./legacy-util')` (or
   `import { legacyJoin } from "./legacy-util"`) with a sibling file
   present must hint `["./legacy-util", "legacyJoin"]` (item-included —
   the `SymbolContext.scope` contract). Bounded, one parse, real
   specifier (verify the parse yields the specifier; the negative-shape
   pins stay: relative-with-no-file / absolute / side-effect still NOT
   hinted unless the feature actually lands — decide honestly which
   shapes hint, per the provider's landed semantics: relative→hint,
   absolute→no hint (the provider bails dedicated), side-effect file-ish
   with no hint → the provider bails... CAREFUL: read fix-jsrel's
   semantics (is_file_ish / relative branch) before choosing which
   shapes the app may hint. The provider contract is already golden-
   pinned; the app must emit hints that match it exactly.)
2. **LIVE landing**: with the hint emitted, M-. on a relative use site
   lands in the sibling file (external=false; a project file — check
   what open_resolved_source does with an in-workspace file: it opens
   EDITABLE via open_project_path; verify the landing is honest).
3. **Update the contract-pin notes**: the js corpus README's P2-7 note and
   probes.toml header change from forward-looking to emitted (cite the
   commit); update the negative-shape pin (`resolver_scope_js_no_import_
   and_relative_are_not_guessed`) to the new truth (relative WITH the
   sibling present now hints; the honest negatives stay negatives) and
   rename honestly.
4. **Matrix**: add the app-side note to the JS relative-import row.

## Constraints

- Gate: `cargo test --workspace` + `tools/gate.sh full` (the reduced
  15-record tier + suites; flock discipline; `cargo build` before
  batteries). Budget ~40 tool calls; honest-stop provision.
- Scope fence: `src/app/store.rs` (js_ts_scope_for + the pins + tests),
  `crates/redline-resolve/tests/corpus/js/README.md` + `probes.toml`
  notes only (NO golden flips — the provider behavior is unchanged; if a
  golden must change for any reason, STOP and report), `docs/provider-
  matrix.md` (js relative row). NO provider changes, no other languages.
- A parallel lane owns `src/app/flow_tests.rs` (loop-04) — do not edit it.

## Post-merge follow-up (review P2s, queued)

- P2-1: one probe line in `resolver_scope_js_relative_import_carries_sibling_path` pinning the `{ default as D }` → `["./m"]` entry-only emission (the one shape that deliberately drops the member).
- P2-2: corpus hygiene note — `relative-import-whole-bail` anchors at the dotted call site (legacy.legacyPad), the bare `legacy` symbol has no live bare use (pre-existing fix-jsrel artifact).
