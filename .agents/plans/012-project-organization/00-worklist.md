# 012-00 — Cleanup work list (refactor-consolidate Phase 1 output)

Method: the `refactor-consolidate` skill. Four **read-only** scans (code
duplication · test redundancy · boilerplate & ceremony · structural issues)
over `src/` + `crates/redline-resolve/`, each reporting
`Location / Current / Issue / Suggested / Risk` + a mechanical|structural|tests
tag. Supervisor then **verified** every headline claim by grep (see
§Verification) before it entered this list.

Baseline facts (all grep-verified, HEAD `acd974e`):
`src/app/store.rs` 22,992 lines / 869 methods / 324 `pub fn` / **349 tests**
(one 11k-line test module) · `flow_tests.rs` 3,624 lines / 88 twins ·
`allow|expect(dead_code)` **50 occurrences** · `seed()` 694 lines ·
`tempfile::tempdir()` 205 sites · `Cargo.toml` scaffolds 69 sites.

## Verification notes (supervisor re-verified every headline claim)

- Attribute inventory: **50** `allow|expect(dead_code)` attributes; 52 lines
  mention `dead_code` (2 are prose). Both scan numbers were defensible; the
  attribute count is 50.
- `store.rs` test count: **371 test attributes** = 349 `#[test]` + 22
  `#[tokio::test]`. (My first re-count said 349 — that was the narrower count;
  the scan was right.)
- **Scan-methodology caveat**: `.agents/worktrees/{1,2}` are full-tree copies;
  a repo-root `grep -r` sees 3× every hit (verified: `fn git(` = 18 root-wide vs
  6 in `src`). Future scans must exclude `.agents/`. The four scans below did
  not appear to double-count (their totals match main-only counts).
- `LanguageId::name` has no `LanguageId::name(` call site but IS live via
  `.name()` (11 hits, incl. `store.rs:8562`) — the stale attribute holds.
- Findings the supervisor REFUTED or downgraded: none (all headline claims held).
- Layer audit came back **clean** (no `app→ui`, `syntax→app`, `nav→app`; the
  resolver crate is app-free). The debt is module-mixing + duplication + docs,
  **not** architectural violation — so almost everything here is safe mechanical work.
- **Coverage caveat (measured, not assumed)**: the four scans each ran 21–37
  repo-wide `bash` greps and **0–1 `read` calls**, i.e. they are whole-tree
  *textual* scans with sampled reading, not end-to-end file review. Findings
  are reliable where a grep can prove them (all headline claims re-verified by
  the supervisor); they can miss duplication that requires reading a file to
  notice. A deterministic block-duplication pass is the cross-check (see
  §Deterministic cross-check).

---

## Tier 1 — Mechanical, ready to delegate (no behavior change, verified)

| ID | Item | Sites | Risk |
|----|------|-------|------|
| **M1** | Delete **7 stale `#[allow(dead_code)]`** + obsolete "future wiring" comments (6 in `nav/index.rs:425–650`: `IndexProgress`, `with_publisher`, `extract_file`, `build_index`, `refresh_in_place`, `IndexBus::send`; + `syntax/registry.rs:41` `LanguageId::name`). Prod callers verified. | 7 | low |
| **M2** | Delete `IndexProgress::mark_done` (`nav/index.rs:458`) — **zero callers** (verified) | 1 | low |
| **M3** | Shared `src/git/test_support.rs` for the hermetic-git test harness. **Corrected by deterministic pass: ~19 helpers in 8 files**, not 6 (the scans grepped `fn git(` and missed the rest): 6× `fn git() -> String`, 1× `git_out()`, 11× `fn git_cli()` (`store.rs` ×9, `ui/magit_status.rs`, `ui/rows_view.rs`), 1× `git_test_cli()`; plus 5 `init_repo` copies | ~19 in 8 files | low |
| **M4** | Shared `ui::text_style(fg, invert, bold)` (+ bold/italic wrappers) next to `color()`; unify `picker.rs:140`, `transient_menu.rs:144`, `file_view.rs:256` | 3 | low |
| **M5** | One `is_word_char(c: char)` helper (`store.rs:5261`, `rg.rs:593`, 3 inline closures) | 5 | low |
| **M6** | Split `command.rs::seed()` (694 lines, 108 commands) into `register_{navigation,region,git,search,…}`; keep the 108-count assertion | 1 fn | low |
| **M7** | Free fns → methods for type-owned helpers (`nav/index` build fns, `ui/root` `cursor_cell`/`click_pane`→`Snapshot`, `model/sections` render/fold, `syntax/queries` `kind_of`, `app/keymap` `collect_command_pairs`) | ~12 | low |
| **M8** | Delete genuinely-dead serde-only fields (`cargo.rs` `workspace_root`, `Target.name`/`kind`; `js_provider` `types`) — **check `deny_unknown_fields` first** | 4 | med |
| **M9** | Narrow `pub`→`pub(crate)` where no `crates/*` consumer (binary crate ⇒ safe); incl. `command.rs:95 dispatch_by_name` (test-only) | ~many | low |
| **M10** | `current_key()` (24 sites) + `project_root()` (10 sites) guard helpers — **but both live in `store.rs`; fold into the store split stage, don't do as a separate pass** | 34 | low |
| **M11** | Shared render preamble for the `element! { … }` block repeated in `ui/{file_view,magit_status,results_view,rows_view,views/buffer}.rs` (found by the deterministic pass; not in any scan) | 5 | low-med |
| **M12** | Delete **4 dead public constructors with zero in-repo references** (incl. tests/docs): `cargo.rs::with_cargo_bin`, `go_provider::with_go_bin`, `js_provider::with_npm_bin`, `lib.rs::with_providers` (`tools/cleanup_scan.py deadpub`) | 4 | low |
| **M13** | **Correctness-adjacent**: `store.rs:2627 is_syntax_anchor_kind` and `syntax/node.rs:357 is_rust_identifier_kind` have **byte-identical bodies** — the Rust-identifier-kind list exists twice, in production code, in two modules, so a grammar bump can silently desync them (and `store.rs` carrying syntax knowledge is itself a layering smell). Single source in `syntax/` | 2 | med |
| **M14** | Identical test helper `fn file(path, content)` in `model/files.rs:145` + `model/project.rs:214` | 2 | low |

## Tier 2 — Structural (each needs one stated design call)

| ID | Item | Design call | Risk |
|----|------|-------------|------|
| **S1** | `store.rs` split (plan 012 phase 2) | already decided; `012-01` inventory in flight | med |
| **S2** | `queries.rs`: 245 lines of per-language consts → `queries/lang.rs`; engine stays | none — pure move | low |
| **S3** | `node.rs`: 3 parallel per-language dispatch tables (`is_*_identifier_kind`, `*_scope_path`, `is_path_segment`) → one table/trait | **D1** | med |
| **S4** | `nav/index.rs` → `index.rs` (SymbolIndex) + `build.rs` (build/refresh/extract) + `progress.rs` (IndexProgress/IndexEvent/IndexBus) | none — pure move (2nd `impl SymbolIndex` block) | low |
| **S5** | `ui/root.rs` → `root.rs` (components) + `input.rs` (`to_app_key`) + `cursor.rs` (`cursor_cell`/`click_pane`/`Snapshot`) | none | low |
| **S6** | `git/repo.rs` → `repo.rs` (queries) + `index_ops.rs` (stage/unstage/discard) + `hunk.rs` (pure byte math) | field visibility `pub(crate)` | med |
| **S7** | resolver: `providers/scan.rs` shared `walk_files(exts)` + line-defines driver; providers keep only their predicates | none | med |
| **S8** | provider boilerplate (name/languages/new/offline ×4) → macro or default trait | none | low |
| **S9** | `search/rg.rs::run` (170 lines) → spawn/parse/emit; `model/sections.rs` model vs view helpers | none | low |
| **S10** | `ui/root.rs` 4× duplicated async bus-drain → generic helper | iocraft hook ergonomics | med |
| **S11** | `docs/` index + process-log separation (`ux-testing-plan.md` 760 lines is half process log; `emacs-parity-log.md` has duplicated headings) | none | low |

## Tier 3 — Test consolidation (assertions are spec; per-pair diff REQUIRED)

| ID | Item | Rule | Risk |
|----|------|------|------|
| **T1** | Golden-suite shared helpers (`golden_{go,js,python,rust}.rs`): `assert_bless_stopped` is **byte-identical** (md5-verified), plus `normalize`/`check_golden`/`summarize`/`copy_tree` → `tests/common/golden_support.rs` | pure move; corpora+goldens stay | low |
| **T2** | store-test vs flow-twin pairs (quit-prompt, buffer-list, notes, xref, mark/kill) | **diff each pair**; keep the tier carrying the unique assertion (twin usually adds `render80`/key-path); never blanket-delete | med |
| **T3** | `crate_index_builds_for_<lang>_*` ×10 → table-driven case list (keep per-language assertions) | keep setup consolidation only | med |
| **T4** | Unify index install (`store_with_index` vs `install_index`) + `notes_store` fixtures | setup only | low |
| **T5** | `project_dir(files) -> TempDir` scaffold helper (69 `Cargo.toml` sites / 205 tempdirs) | setup only | med |
| **T6** | Provider `line_defines_*` / `bare_symbol_*` → per-file tables | per-language truth tables stay | med |
| **T7** | Strengthen non-discriminating tests: bare `.is_ok()` with no payload/state check (`store.rs:10577`, `:16949`, `:3645`); audit twins for vacuous absence-only assertions post `PTY_QUIET` shrink | strengthen, never delete | low |
| **T8** | PTY battery drivers `drive_{emacs,redline}_battery2/3` → parameterized driver | lower priority than Rust duplication | low |

## Round 2 deltas (read-verified, local model)

Round 2 *read the code*. It verified both ground-truth items and added seven
semantic-duplication findings round 1 could not see. New IDs here supersede or
extend the tiers above; `fix-by` records whether the fix is deterministic
(mechanical, referee-checkable) or needs judgment.

### Corrections to earlier rows

- **M3 (git harness) — IMPROVED**: it is **19 helpers in 8 files**, and the
  copies are **not all equivalent**:
  - `git/commit.rs:68` has an extra `with_author: bool` and reordered env;
  - **`app/flow_tests.rs:91` drops ALL SIX hermetic env vars**
    (`GIT_CONFIG_GLOBAL=/dev/null` etc.) — that copy is *less* hermetic than the
    rest, a latent host-identity flake, not a style difference;
  - `store.rs:14593` and `store.rs:20102` use short env values (`"T"/"t@e.com"`).
  Consolidation must therefore decide flow_tests' env shape explicitly
  (aligning it is a **behavior change to a test**, allowed only with a stated reason).
- **S3 (language tables) — WIDENED**: it is **six 18–19-arm dispatch tables
  across 3 files** (`registry::{name, build, highlight_query_for}`,
  `queries::{query_for, language_for}`, plus the supported-language gate in
  `node::parse_source`) — **~110 hand-synced arms**. Adding a language means
  touching >=6 match sites in 3 files.
- **S10 (bus drains) — CONFIRMED as the highest-leverage *production* dedup**:
  four ~12-line `watch` drain loops in `root.rs` are verbatim except identifiers,
  plus a 5th structurally different mpsc variant for search.
  *Tooling note*: `cleanup_scan.py dup`'s fn-level mode **cannot** see these (they
  are closures inside `Root`, not `fn`s) — its window mode can. Recorded so nobody
  reads "only 10 identical fn bodies" as "little duplication".

### New findings

| ID | Finding | Location | fix-by | Risk |
|----|---------|----------|--------|------|
| **R1** | UI-test fixture + "title not overprinted" assertion duplicated 4–5×, with **drift** (magit hardcodes `*magit-status*`, rows_view uses `store.log_title()`, buffer `*list-buffers*`) | `ui/{magit_status,rows_view,file_view,results_view,views/buffer}.rs` | deterministic (`ui/test_util.rs`: `render_app`, `fixture_project`, `assert_title_not_overprinted`) | low |
| **R2** | `repo.rs` blob-newline twins (`head_`/`index_blob_ends_with_newline`, 10-line bodies differing only by `DiffSide`) + hunk-lookup sequence ×4 | `git/repo.rs:369-402`, `:297,:422,:521,:536` | deterministic | low |
| **R3** | **`files.rs` vs `rg.rs`: gitignore ancestor-chain semantics implemented twice** — documented as intentional (threading), but they *must* agree (finder vs search disagreement is user-visible) and **no test asserts they agree** | `model/files.rs:108-147`, `search/rg.rs` | judgment: keep 2 engines, share the match helper, **add an agreement test** | med |
| **R4** | Provider `walk_*_files` ×2 identical + locate/scan triad ×3 (`line_defines_*` predicates stay per-language) | `crates/redline-resolve/src/providers/*` | deterministic (`walk_files`, `find_def_line`) | low |
| **R5** | `MagitStatusView` ≡ `MagitRowsView` minus props — 60 lines re-copied instead of reusing; log/blame/commit-editor already reuse `MagitRowsView` | `ui/magit_status.rs:114-174`, `ui/rows_view.rs:111-169` | judgment (small): delete the body, render `MagitRowsView(title:…)` | low |
| **R6** | `store.rs`: **4 `*_view_info`** + **3 cursor/scroll families (~18 handlers)** clone the windowing *call pattern* (only the `window_slice`/`keep_cursor_visible` primitives are shared — the 003-02 "shared windowing" comment overstates what is shared); the 4 view_infos are not even uniform (`magit_window()` vs `pane_window()`) | `store.rs:6134,7237,7306,7388`, `:7245-7450` | judgment (state-shape refactor of the hottest file; pinned by windowing tests at `:20078+`) | med |
| **R7** | `StatusLine` indicator chain: ~12× `if !props.X.is_empty() { push_str(&format!("  *{X}")) }` — fold over a `[(label, value)]` list | `ui/root.rs` | deterministic | low |

Additional areas round 2 flags for a later pass: `store.rs` `apply_*_event` halves
(the `&mut self` side of S10) may share a stale-generation-check + reset + message
shape; `switch_project_root` vs `open_project_path` share a close/reset/re-walk/prime
sequence; `model/sections.rs`/`tree_layout.rs` padding math vs the blame/log row
formatters.

**Sequencing note**: R7 and S10 both edit `ui/root.rs`, which the **live
picker-density lane also edits** — both must wait for that lane to land (same rule
as M4).

## Design decisions needed (supervisor)

- **D1 — the per-language descriptor table (highest payoff).** `LanguageId`
  is enumerated in **six** places that must stay in sync: `registry.ext_map`,
  `registry.build`, `queries.query_for`, `queries.language_for`,
  `highlight.{supports_reuse,reuse_language}`, `node.is_identifier_kind` (+15
  per-language predicates). One `LanguageSpec { id, exts, grammar, query,
  reuse, identifier_kinds, scope_path }` table collapses them and makes adding
  a language a one-edit change (today: a six-file sweep — exactly what the
  19-language expansion cost). **Recommendation: yes, land it, but as its own
  lane after S1**, because it touches `node.rs`+`queries.rs`+`registry.rs`
  (file-disjoint from the store split ⇒ parallel-safe).
- **D2 — `Xref` trait** (one implementor, only test-side `dyn` use): **keep** —
  it is the deliberate LSP-backend seam and the reason no LSP crept in.
- **D3 — `theme::Color` enum-level allow + `node.rs` module-level
  `#![allow(dead_code)]`**: narrow to real items while doing S3; do not
  keep file-level suppression.

## Deferred (with trigger)

- `AppStore` `pub`-surface narrowing → after S1 (the split moves the surface).
- `check_cursor_stream.py` (1,103 lines, one-off) → only if PTY work resumes.
- `store.rs` `#[cfg(test)]` module split → phase 3 (012-08), after the code moves.

## Metrics (duplication identified)

- ~200+ duplicated test-helper lines (goldens), ~120 lines (6 git harnesses),
  ~700 lines freed by `seed()` split, 34 guard sites, 50 attributes audited
  (7 stale + ~25 mislabeled test seams + ~5 truly dead), 10–12 table-drivable
  test bodies, 6 language-sync points → 1 table.

## Proposed staging (maps Tier 1–3 onto plan 012's phases)

1. **012-01** inventory (in flight) → **012-02** store split pattern.
2. **012-10 (new) "mechanical sweep"** — M1–M9 live in files the store split
   does NOT own (`nav/index.rs`, `syntax/registry.rs`, `src/git/*`, `ui/mod.rs`,
   `app/command.rs`) ⇒ **can run as a parallel lane**, except **M4 must wait
   for the picker lane to land** (it edits `ui/picker.rs`).
3. **012-11 (new) "language descriptor table" (D1/S3)** — parallel-safe with the
   store split; sequence after M1 (same file, `nav/index.rs` is disjoint from
   `syntax/*` so actually fine, but `M1`+`S3` both touch `syntax/registry.rs` ⇒ serial).
4. **012-12 (new) "test consolidation"** — T1 first (cheap, proven byte-identical),
   then T4/T2, then T3/T5/T6.
5. **012-09** other oversized files + docs index (S4–S9, S11).

## Deterministic cross-check (no LLM)

**Tool: `tools/cleanup_scan.py`** (`dup` | `deadpub` | `all`) — committed so this is
repeatable rather than ad-hoc. It runs (a) normalized 6-line-window duplicates,
(b) brace-matched **whole-function** duplicates, (c) pub fns with zero in-repo
references. Excludes `.agents/` and `target/`. This is the referee for inventory
questions: round-2 LLM verdicts get cross-checked against it, and "did the scan
cover the project?" is answered by running it, not by trusting prose.

### Results (78 files, 2,220 functions parsed)

Whole-function duplicate groups (cross-file): **10**, of which the notable ones:
- the git harness family (4× `git()`, 5× `git_cli()`, 3× `init_repo()`) — see M3;
- golden-suite helpers (`sub_path`, `assert_bless_stopped`, `copy_tree`, `bless_*`) — see T1;
- `text_style` (M4);
- **`is_syntax_anchor_kind` ≡ `is_rust_identifier_kind`** (M13) — missed by all four round-1 scans;
- identical test helper `file()` in `model/{files,project}.rs` (M14).

Dead pub surface: **7** zero-reference pub fns, of which 4 are real dead code (M12)
and 3 are corpus fixtures (expected).

Window duplicates: 176 cross-file windows (77 prod-involving across 22 files,
99 test-only).


A normalized 6-line-window hash pass over every `.rs` file in `src/` +
`crates/` (comments/whitespace stripped, cross-file repeats only, 78 files
scanned): **176 cross-file duplicate windows** (77 prod-involving across 22
files, 99 test-only).

What it corroborated (scan was right): the golden-suite helper duplication
(`golden_go`↔`golden_js`↔`golden_python`↔`golden_rust`, 3+ files) and the
hermetic-git harness.

What it **corrected or added** (scan missed it):

1. **The git harness is ~19 copies in 8 files, not 6.** The scans grepped
   `fn git(` and so missed `git_out`/`git_cli`/`git_test_cli` (11+ sites) and
   the two UI files. `GIT_AUTHOR_NAME` appears in `store.rs`,
   `ui/magit_status.rs`, `ui/rows_view.rs` + all 5 `git/*.rs`.
2. **`element! { … }` render preamble duplicated across 5 UI files**
   (`file_view`, `magit_status`, `results_view`, `rows_view`, `views/buffer`)
   — a duplication class no scan reported.

**Methodology lesson for future scans**: grep-shape assumptions ("the helper
is called `git`") silently set the coverage ceiling. Pattern-free mechanical
passes (this one) are the cross-check; LLM scans should be used for the
judgment calls (e.g. T2's keep-or-merge per pair), not as the inventory.

## Start here

`nav/index.rs:425–650` (M1+M2, one contiguous verified batch delete), then
`crates/redline-resolve/tests/golden_go.rs` ↔ `golden_python.rs` (T1).
