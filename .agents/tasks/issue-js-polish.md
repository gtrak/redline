# Task: js_provider polish + matrix refresh (queued review P2s)

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `.agents/skills/*.md` as ground truth.

## Items (all small, from landed review P2s)

1. **`.d.ts` omission in the relative walk** (fix-jsrel review P2-1
   remainder): `resolve_relative_file`'s JS_EXT walk omits `.d.ts` while
   the package-side `walk_js_files` scans it — `import type {X} from
   "./types"` with only `types.d.ts` present bails. Decide: add `d.ts`
   to the relative walk's tail (order: where? Node has no `.d.ts`
   convention for runtime imports, but go-to-definition DOES want it) —
   or pin the honest bail. State the decision; pin it either way.
2. **Symlink-escape test** (fix-jsrel review P2-6): a `#[cfg(unix)]` unit
   test — a workspace symlink pointing outside must bail (canonicalize-
   before-contains neutralizes it at runtime; pin the claim).
3. **`find_local_path_dep` ancestor-walk quirk** (pre-existing, flagged
   by the fix-jsrel reviewer): `read_pkg_json(&dir)?` INSIDE the loop
   stops the walk at the first ancestor lacking a package.json instead
   of skipping it. Fix: continue the walk on a missing package.json,
   keep bailing on a malformed one. Add a discriminating unit test
   (a `file:` dep two levels up with a packageless intermediate dir).
4. **Matrix refresh** (fix-alias reviewer P2-2 + stragglers): go unit
   count 39 → current (41+); the python/go path-shaped cells gain the
   alias-rewrite note; the python workspace-resolution honesty note per
   the py-roots lane's outcome IF it has landed by your commit (else
   leave that one row); verify no stale counts remain.
5. **Small doc/lint stragglers**: the setext e2e false-positive README
   note (reviewer P2: the e2e alone wouldn't catch a loose-paragraph
   regression — one clause in docs/provider-matrix.md's Markdown section
   or the corpus README).

## Constraints

- Gate: `cargo test --workspace` + clippy (the go-lane lint discipline).
  Budget ~30 tool calls; honest-stop provision.
- Scope fence: `crates/redline-resolve/src/providers/js_provider.rs`
  (+ unit tests), `crates/redline-resolve/tests/corpus/js/README.md`,
  `docs/provider-matrix.md`. NOTHING else. A parallel lane owns
  `src/app/flow_tests.rs` and `src/app/store.rs` — do not touch either.

## Post-merge follow-up (review P2s, queued)

- malformed-package.json bail half: unit test (malformed intermediate + valid above → walk bails, not climbs past).
- matrix "41 unit tests" wording: 3 are #[ignore] (loud-skip, not pass) — precision fix for a future matrix pass.
