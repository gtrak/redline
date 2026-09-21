# 012 — Redline: project organization & cleanup (ARCHIVED)

Status: ARCHIVED 2026-09-21 — structurally complete: all 9 plan issues
(01–09) LANDED, each verified at landing (gate + review + evidence rows in
the 012 table in `.agents/plans/STATUS.md`), plus the post-012 cleanup/
organization lanes recorded in the same table (A1–A7, M-sweeps, T1, D1, …).

This file is the durable record: the full original folder, verbatim —
`PLAN.md` plus the three supporting files that lived alongside it:

- `00-worklist.md` — the refactor-consolidate work list (tiers, round-2
  deltas, C/R/M/T finding tables). Its **§ Gate reliability** and
  **§ What remains** sections are still live cross-reference targets
  (cited from `.agents/plans/STATUS.md` and the 013 plan).
- `01-inventory.md` — the 012-01 concern-inventory issue contract.
- `02-language-descriptor-design.md` — the full per-language descriptor
  table design (19-row inventory + 7-step migration order) that issue D1
  (`issue-descriptor-table`) implemented.

No text was edited or summarized; status lines inside the plan are as
written when true (e.g. the "869 methods" figure is superseded in
`PLAN.md` itself — the correction is part of the record).

---

## PLAN.md (full text)

# 012 — Redline: project organization & cleanup

Status: structurally complete — all nine issues landed (the store is split,
the tests are split, the other oversized files are decomposed, incl. the A7
navigation split); the § Success criteria line-bar exceptions are the
recorded audit judgments; remainder is tracked in `00-worklist.md`
§ What remains; archive pending (on request)
Phases: 4 · Issues: 01–09 (staged; each stage lands gate-green)
Depends on: nothing (behavior-preserving refactor + cleanup)

## Why

`src/app/store.rs` is **22,993 lines** — roughly 40% of the 56 k-line codebase —
with an `impl AppStore` of **421 methods** (253 `pub` / 168 private) spanning lines
1701–11588, plus 11 associated free functions. One impl block holds every concern:
buffers, views, file-view cursor/scroll, search, magit, log/blame/commit, notes,
navigation, index wiring, pickers, minibuffer, project/files, key dispatch.
(Two count corrections on the way here: the widely-quoted "869" was a bad grep
that swept in the 11 k-line test module, and "432" counted methods + free fns.) It mixes
buffers, views, the view stack, file-view cursor/scroll, search, magit
status/staging, log/blame/commit/diff, notes, annotations, navigation
(M-., jump stack, xref/imenu/impls pickers), index wiring, crate indexes,
minibuffer, project/files/recents, keymap dispatch — plus a very large test
module. Consequences observed this session:
- every lane that touches anything in the app needs `store.rs`, so lanes
  cannot run in parallel (file-disjoint autonomy collapses);
- reviews are expensive (`file:line` citations drift constantly — e.g. 4 of
  6 doc citations in one review round pointed at the wrong lines);
- readers (human and agent) pay a full-file scan to find one concern.

Adjacent offenders: `src/app/flow_tests.rs` 3,624 · `src/syntax/node.rs`
2,251 · `src/ui/root.rs` 1,752 · `src/git/repo.rs` 1,713 ·
`src/syntax/queries.rs` 1,697.

## What

**Phase 1 — plan the split on facts (no behavior change).**
Inventory the methods into coherent concerns with line ranges; publish
`docs/architecture.md` (module map + the split plan + the mechanical
constraints). Reviewer checks the inventory against the file.

**Phase 2 — split `store.rs`, one concern per stage.**

**Step 0 (mandatory, do it first):** the authoritative concern map is
`docs/architecture.md`, but its **line numbers are baseline-pinned** to the commit
they were measured at and `store.rs` has already moved once (+45/−1 from the
picker-density landing). Re-run the census against the current file, update the
map, and only then start moving code. The counts (421 methods = 253 `pub` +
168 private) are stable; the line numbers are not.
Target layout (names indicative; the inventory confirms):
`src/app/store/mod.rs` (the struct + state + shared helpers),
`buffers.rs`, `views.rs`, `file_view.rs`, `search.rs`, `magit.rs`,
`commit.rs` (log/blame/diff/editor), `notes.rs`, `navigation.rs`
(M-./jump/xref/imenu/impls), `index_wiring.rs`, `picker.rs`,
`minibuffer.rs`, `project.rs` (files/recents/tree), `keys.rs` (dispatch).
Mechanics (**compiler-verified**, in both directions): Rust allows `impl AppStore`
blocks in other modules of the same crate, and a module's **private items are
visible to its descendant modules** — verified with `rustc`: a child module reads
and mutates private *fields* with no visibility change. **But** a private
*method* defined in one submodule and called from a **sibling** submodule fails
with `E0624: method is private`; `pub(super)` on it compiles. So the split needs:
files placed under `app::store::*` (**children** of the module defining
`AppStore`, not siblings like `app::store_buffers`), **no field-visibility pass**,
and a **`pub(super)` pass on the private methods that cross concern
boundaries** (measured at **73** when the 13 concern moves landed — far below the
168 private methods the original estimate assumed, because most private methods
are called from within their own concern). Two further facts the review
established, both easy to get wrong: `#[path = "flow_tests.rs"]` must become
**`"../flow_tests.rs"`** (the attribute resolves relative to the containing
file's directory, so it would otherwise point at a nonexistent
`src/app/store/flow_tests.rs` → E0583), and `collect_syntax_anchor_nodes` (2602)
+ `is_syntax_anchor_kind` (2627) are **impl methods formatted at column 0**, so
indent-based tooling must not move them out of the impl. A third rule came from
splitting `syntax/node.rs`: **a `pub use` re-export cannot re-export an item less
visible than itself** (`pub use` of a `pub(super)`/private item → **E0364**); for
internal-only items use `pub(crate) use` (or widen the item to `pub(crate)`), which
keeps the visibility narrowing rather than adding public API.
A **fourth** rule came from A7 (`store/navigation.rs`, 2,394 lines → 4 files):
**`pub(super)` is not always the narrowest correct answer.** A child module's
`pub(super)` reaches only its *parent*, so an item that was visible at the
`app::store` level — called from `picker.rs` or `store::tests`, i.e. the parent's
*siblings* — needs **`pub(in crate::app::store)`** instead. A7 needed exactly **9**
of those plus **6** plain `pub(super)`; the task spec had asserted `pub(super)`
would suffice, which is wrong for any item the store itself exposes. `pub(in path)`
is *narrower* than `pub(crate)`, so it is the right tool rather than a fallback.
Also verified: **one `impl` block cannot span files** (the parse fails with an
unclosed delimiter), so each file carries its own `impl AppStore { … }` wrapper —
the same shape the 13 concern moves used.
**A numbers lesson from the same lane**: the A7 brief stated 48 methods, 8
`js_ts_*`, 4 `go_*`; the re-derivation found **51, 7, 3**. The brief's
method→line *map* was exactly right, but its *counts* were not — which is the
"numbers in briefs are claims" rule applied to the orchestrator. It was caught
only because the spec ordered an independent re-derivation first.
**Each stage is behavior-preserving and lands gate-green**; no stage mixes a
behavior change with a move. Warm-up stages first (`00-worklist.md` §Round-2
structural deltas): free helpers · notes doc · test module, then per-concern
`impl` moves. The authoritative concern/module map is `docs/architecture.md`
(from `012-01`).

**Phase 3 — split the tests.** `store.rs`'s test module → per-concern test
files alongside the new modules; `flow_tests.rs` split by the same concerns.

**Phase 4 — the other oversized files + general cleanup.**
`node.rs` per language, `queries.rs` per-language consts, `git/repo.rs`
(status/log/blame/commit); a dead-code/`allow(dead_code)` audit; duplicated
helpers (e.g. `truncate` exists in more than one ui module); stale comments
(several found this session); a `docs/README.md` index.

## Success criteria

- No `src/` file over ~1,500 lines; `store.rs` concerns split as planned.
  **SUPERSEDED as a criterion (user directive, 2026-09): test file size is not a
  criterion; production *logic* organization is** (see
  `.agents/skills/plan-process/SKILL.md` § Maintainability criterion). The line bar
  therefore applies to production modules only — the test-side `definitions.rs`
  (1,548) and `flow_tests.rs` (3,709) are **not** findings.
  **PARTIALLY MET (audited after all 17 lanes landed)**: `store.rs`'s concerns ARE split
  (403 methods into 13 concern files + an 18-method core), but six `src/` files still
  exceed 1,500 — three are test files (cohesive), and the production misses are
  `store/mod.rs` 2,445 (cohesive core), `syntax/queries.rs` 1,653 (cohesive), and
  **`store/navigation.rs` 2,394, which is a grab-bag and is staged as A7** (see
  `00-worklist.md` § A7).
- **Zero behavior change**: the full workspace suite + PTY gate stay green
  at every stage (829+ tests, 12/12 pooled, `gate.sh full` OK) with no test
  assertion weakened — moves are `git mv`-shaped, not rewrites.
- Two lanes can work on different app concerns file-disjointly.
- `docs/architecture.md` exists and matches the tree.

## Task order

| Phase | Issue | Depends on |
|---|---|---|
| 1 | 01 concern inventory + module map | — |
| 2 | 02 split pattern + buffers/views | 01 |
| 2 | 03 search | 02 |
| 2 | 04 magit + commit/log/blame | 02 |
| 2 | 05 notes + annotations | 02 |
| 2 | 06 navigation + index wiring | 02 |
| 2 | 07 picker + minibuffer + project/files | 02 |
| 3 | 08 tests split (store tests + flow_tests) | 02–07 |
| 4 | 09 other files + general cleanup | 08 |

Serialization: each store split stage owns `src/app/store*` alone (no
parallel app lanes), because the moves touch shared field visibility.
Stage 09's per-file work is file-disjoint and can fan out.

When complete, archive per plan-process.

## 00-worklist.md (full text)

# 012-00 — Cleanup work list (refactor-consolidate Phase 1 output)

Method: the `refactor-consolidate` skill. Four **read-only** scans (code
duplication · test redundancy · boilerplate & ceremony · structural issues)
over `src/` + `crates/redline-resolve/`, each reporting
`Location / Current / Issue / Suggested / Risk` + a mechanical|structural|tests
tag. Supervisor then **verified** every headline claim by grep (see
§Verification) before it entered this list.

Baseline facts (re-verified; several earlier figures were grep artifacts — see §Verification):
`src/app/store.rs` 22,992 lines; `impl AppStore` **432 methods** per the `012-01`
inventory (254 `pub` / 178 private). My own recount of the impl region gives 419, so
the exact figure is being settled by the `012-01` review gate. The widely-quoted
"869" was a bad grep that swept in the 11 k-line test module's fns. Tests in the
file: **349 `#[test]` + 22 `#[tokio::test]` = 371** (plus ~18 helpers = the 389 the
worker measured); `flow_tests.rs` 3,624 lines / 88 twins ·
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
| **S1** | `store.rs` split (plan 012 phase 2) | done (`012-01` inventory landed; the split landed in the concern lanes) | med |
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

## C5 RESOLVED — magit-consistent section movement (user directive: "be consistent with real magit")

Authority: **magit 4.7.1** source (`lisp/magit-section.el:805-831`, cloned
`github.com/magit/magit` @ `5059906`, 2026-09-20). Real magit **does not wrap**:

- `magit-section-forward` (`n`): at `eobp` → `(user-error "No next section")`; when no
  next sibling/parent-sibling exists → the same `user-error`. It **stays put and
  reports**, it does not wrap.
- `magit-section-backward` (`p`): at `bobp` → `(user-error "No previous section")`.
- So the boundary behaviour is **symmetric: no wrap + an echo-area message**.

Redline today is wrong in both halves (wraps on `n`, silently stops on `p`), and the
stale doc accidentally described magit. **Required change**: drop the wrap branch in
`move_down`, make both directions stop at the boundary, and emit the boundary
message through the minibuffer (redline's echo area) — `"No next section"` /
`"No previous section"` for parity. Pin with unit tests (no-wrap in both
directions + the message), since nothing pins it today.

**Deeper parity nuances found in the same source** (not part of C5; verify before
claiming section-movement parity):

1. `n` **descends into children**: if the current section is visible and point is not
   at its end, the next section is its **first child** (lines 812-815) — i.e. document
   order, not sibling-only.
2. `p` goes to the **beginning of the current section first** when point is inside one
   (lines 838-841), not straight to the previous row.
3. Root-level special case: a section with no parent moves one **line**
   (`magit-section-goto 1`, line 822).

If redline's `StatusTree` rows are one-per-section these collapse to document order
and C5 alone suffices; if sections span multiple rows, (1)-(2) are a real behavioural
gap. Worth an empirical magit probe — the source clone makes a **runnable magit
probe harness** feasible now (clone `transient`/`dash`/`s`/`compat`/`with-editor`,
run `-L` from the clones), which would let us pin magit behaviour the way
`tools/drive_emacs*.py` pins vanilla emacs.

## Round 2 structural deltas (read-verified, local model)

Full design preserved in-repo: **`02-language-descriptor-design.md`** (19-row
inventory, dispatch table, 7-step migration order).

### S1 CORRECTED — the store split IS mechanically viable (compiler-verified)

Round 2 concluded the 9,887-line `impl AppStore` is "not mechanically splittable
without traits/restructuring". **That is wrong, and the compiler settles it.**
Verified with a standalone `rustc --edition 2021` experiment (clean compile,
run output `8 16 x`): an `impl AppStore` block placed in a **child module** of the
module that defines `AppStore` reads and mutates **private fields with no
visibility change** — Rust privacy makes a module's private items visible to its
descendants:

```rust
mod app {
    pub struct AppStore { secret: u32, pub name: String }   // app/store/mod.rs
    pub mod buffers { impl super::AppStore { fn bump(&mut self) { self.secret += 1; } } }
    pub mod sibling { impl super::AppStore { fn x(&self) -> u32 { self.secret * 2 } } }
}
```

Constraint this imposes: the new files must be **children of the defining
module** (`app::store::buffers`), *not* siblings (`app::store_buffers`) — which is
exactly the layout plan 012 already proposed. So **no field-visibility pass is
needed** — but (compiler-verified, `E0624`) private *methods* called across
submodules DO need `pub(super)`. `012-01` counts **178 private methods** → that is
the real mechanical pass, additive and non-breaking. Keep round 2's
inventory: `impl` 1,701–11,588 (253 `pub fn`) · state types 56–1,700 (~40 structs,
`ViewId` alone 176–504) · free helpers 11,588–11,804 · tests 11,805–22,985
(**11,181 lines**).

Warm-up stages round 2 identified, in this order (each green before the next):
**S1a** free helpers → `app/store/helpers.rs` (`window_slice` 11,703,
`keep_cursor_visible` 11,715, `prefill_commit_message` 11,627,
`extract_commit_message` 11,648, `point_byte_offset` 11,798) · **S1b** notes doc →
`app/notes_doc.rs` (`NotesDoc` 871, `parse_notes` 895, `parse_notes_section` 930,
`parse_record_block` 958, `serialize_notes` 1,032 — already store-free) ·
**S1c** test module → `#[cfg(test)] #[path]` files by cluster (low risk).
Decomposing the impl into separate *types* (state-struct extraction) remains a
later, optional, high-risk project — it is **no longer a prerequisite**.

**Authoritative module map (from `012-01`, pending review):** `docs/architecture.md`
holds the measured 14-module concern table — `core/mod.rs` (23 methods) ·
`views.rs` 18 · `buffers.rs` 29 · `notes.rs` 30 · `file_view.rs` 52 · `search.rs` 39 ·
`magit.rs` 24 · `commit.rs` 61 · `navigation.rs` 51 · `index_wiring.rs` 34 ·
`picker.rs` 42 · `project.rs` 17 · `minibuffer.rs` 1 · `keys.rs` 11 — with exact
line runs per module. Its extra requirements for stage 4: `pub use` re-exports in
`mod.rs` for the UI-imported row types (`BufferRow`, `DirtyCounts`, `FileViewRow`,
`PickerCandidate`, `ResultRow`, `TransientMenuRow`, `TreeRow`, `ViewId`), and the
`#[path = "flow_tests.rs"]` wiring moves verbatim with `flow_tests` **staying a
descendant of `store`** (its private-field access depends on it) until phase 3.

### D1 CORRECTED — the sync sites are EIGHT, not six

| # | Site | What it pins |
|---|------|--------------|
| S1 | `registry::{name, ALL}` | identity |
| S2 | `registry::ext_map` | extensions (incl. quirks `h`→C, `mdx`→Md, `ini/conf`→Toml) |
| S3 | `registry::build` | grammar + highlights + injections + locals |
| S4 | `registry::highlight_query_for` | **duplicates S3's highlights arm-for-arm** (vendored arms twice) |
| S5 | `queries::{query_for, language_for}` | definition query + grammar |
| S6 | `highlight::{supports_reuse, reuse_language}` | reuse policy + **grammar pinned a third time** |
| S7 | `node.rs`: `parse_source` whitelist, `in_identifier_position`, `is_path_segment`, `is_identifier_kind` + 15 predicates, 13 scope walkers | node predicates |
| S8 | `tokens::token_class_query_for` | token-class queries (round 1 missed this one) |

Hazards worth separate issues, because they are **correctness** and not tidiness:
1. **Grammar pinned in 3 places** (S3/S5/S6); only S5's pin is covered by the ABI
   guard test ⇒ a miss in S3/S6 **silently falls back to plain text**.
2. **Highlights pinned in 2 places** (S3/S4), vendored `include_str!` arms twice.
3. `supports_reuse` is a **non-exhaustive `matches!`** while `reuse_language` is an
   exhaustive arm ⇒ a new language silently defaults to "no reuse" and the two can
   drift (the warm-up test only covers the languages it names).
4. `parse_source`'s 18-language whitelist is a **4th identity list** duplicating
   `LanguageId::ALL` ⇒ adding a language without it makes M-. dead with **zero
   compile error**.
5. TS/TSX aliasing is implicit (one definition query, two grammars, two rows).

### Verified split plans (A1–A7, full ranges in the design doc)

`queries.rs`→3 modules · `node.rs`→3 submodules · `nav/index.rs`→4 ·
`ui/root.rs`→5 (medium: iocraft component re-derivation + `Snapshot` visibility) ·
`command.rs::seed()`→per-category fns (**the 108 registrations already carry
category strings** — the split axis exists in the data) · `git/repo.rs`→free-fn
extraction only (`hunks.rs` 676–824, `index_ops.rs` 833–908); the 641-line impl
stays whole. `` `git/repo.rs`''s 805-line test file is mostly the pure hunk math.

### Explicitly CLEAN — do not churn

`model/{sections,buffer,project,files,text_width}.rs`, `search/{rg,occur,references}.rs`,
`syntax/cache.rs`, `app/{watcher,events,keymap}.rs` — read end-to-end, **no splits
justified**. Recording this prevents a future lane from "improving" them.

### New findings

- **F-5**: `ui/views/` is dead weight — `mod.rs` is 2 lines, the only child is
  `buffer.rs` (102 lines, one component whose test module sits *above* it). Inline it.
- **F-6**: the resolver providers are the repo's 2nd–4th largest files (js 1,642 /
  go 1,601 / python 1,033), each with its own locate/line-scan + ~400-line golden
  suite; a shared `src/locate.rs` needs a follow-up read of the three locate sections.
- **F-7**: `ui::root::render_at_width` is a `pub fn` that exists **solely** for
  app-side tests — a deliberate cross-layer test seam (acceptable; now named).
- **F-8**: the vendored highlight queries are pinned by "sha256 at copy time"
  **only in comments** — nothing verifies `third_party/*/highlights.scm` still
  matches, so a local edit to a vendored file is undetectable. Add a checksum test
  (it should move with the consts into `language.rs`). **PINNED** (lane
  `invariant-pins`, `c0421c2`). The test shells out to `sha256sum`; the gate judged
  that acceptable here (Linux gate box, no hash crate in the dep set, a hand-rolled
  sha256 would be worse), with `sha2` as a dev-dependency the alternative if
  cross-platform dev is ever anticipated. **Accepted as-is.**
- Round-1 corrections: the fn is `code_to_app_code` (not `to_app_key`); `node.rs`
  has **16** kind-predicates (not 17) and **14** scope walkers (not 16); Scheme and
  Clojure have no scope walkers at all (flat grammars — a silent default today).

## Round 2 test-ledger deltas (both sides read, local model)

### T2 is RESOLVED — and much smaller than scoped

Every pair in T2 was diffed by reading both bodies. **Only 3 tests can be
dropped without losing an assertion:**

| drop | reason |
|---|---|
| `flow_tests.rs:1498 unit_flow_quit_prompt_none` | same three asserts as `quit_prompt_unmodified_fast_path` (11906); only the fixture differs |
| `flow_tests.rs:708 unit_flow_buffer_list_np` | strict subset of `buffer_list_n_p_d_keys` (13075), which also asserts C-n/C-p equivalence, exact row names, kill echo, clamp, `q`→Home |
| `flow_tests.rs:1569 unit_flow_quit_prompt_cg` | subset of `quit_prompt_c_g_cancels_the_whole_quit` (12033), and its "intact" check is weaker (`contains("QZ")`) |

Everything else in the five pairs carries a **positive delta**: render80/flat
width pins, raw-key-path coverage (which the API-driven store tests skip by
construction), the watcher-driven reload seam, the ±25/±26 boundary math,
window-recenter after `M-,` (exists *only* in the twin), or exact cache-line semantics.

**Whole-file audit (88 tests, all read)**: ~24 add a render assertion, ~50 add a
key-path assertion (overlapping), ~12 are unique behaviour, **3 are safe drops**.
Honest conclusion: **`flow_tests.rs` is almost entirely additive** — the
positive-gated style means dropping anything else would lose real coverage. The
ledger belongs in `docs/ux-testing-plan.md` so this is not re-litigated.
Unverified (marked, not assumed duplicate): magit f5–f8/cds/blw, the edit-mode
pair, `panes_*`.

**Positive audit result**: **no flow twin asserts only absence** — every absence
check is composed in the same `assert!` as a positive signal. The `PTY_QUIET`
shrink did not create vacuous passes. One weak gate remains: `ux_view_alive`
(`flow_tests.rs:3196`) has a `_ => true` catch-all, so the transient-menu leg of
`unit_flow_ux_keymap_coverage` degrades to near-absence → handle `TransientMenu`
explicitly (assert menu rows non-empty) instead of the catch-all (T12).

### Correction to T7 — two of those "weak tests" are PRODUCTION code

- `store.rs:3645` (`open_resolved_source`) and `store.rs:10577` (search-jump
  path) are **not tests**: `(rel.clone(), self.open_project_path(..).is_ok())`
  and its twin **discard the `Err`**, so the report cannot say *why* the open
  failed ("cannot open {file}: the jump did not happen"). Fix is mechanical
  (`match` → include `e`), but the message text is PTY-pinned — grep the ledger
  for "cannot open" first, then extend `unit_flow_search_ret_and_mcomma` Leg A to
  assert the cause token (the discriminating assertion that today doesn't exist).
- `store.rs:16949`: `assert!(event.is_ok())` after `.expect("…")` on the same
  `timeout(...).await` is **vacuous** — it cannot fail once the expect passes;
  delete it (the payload asserts right after are the real spec).
- `config.rs:261`: bare `assert!(good.validate_bindings(&registry).is_ok())` — a
  validator that silently swallows a malformed binding still passes; assert the
  exact `Err` for a deliberately bad binding.

### Harness inconsistency (new, and it can explain serial-vs-pooled flake differences)

**T11**: `tools/pyte_driver.py:42` defaults `PTY_QUIET` to **0.06** while
`tools/pool.py:195` defaults the pool path to **0.2** — the two tiers run with
different quiet windows. Confirm which is intended before shrinking further.

### Test-cost / correctness foot-guns (new)

- **T13a**: `crate_index_refuses_oversized_tree` (`store.rs:17714`) materializes
  `EXT_INDEX_FILE_CAP + 1` = **2,001 real files** — the slowest test in the
  family; a smaller probe + boundary comment would do.
- **T13b**: both quit-prompt save-failure tests (`store.rs:12063`,
  `flow_tests.rs:1598`) force failure with `chmod 0o444`, which **root bypasses**
  — the test flips meaning if the suite ever runs as root. Guard with a geteuid
  skip or use a real failure seam.
- **T13c**: `land_resolved`/`miss_resolved` (synthetic events) and the store's
  resolver tests (ambient `~/.cargo`) pin the *same* report text from two seams —
  any reword must update both; a shared prefix `const` would de-risk the fanout.

## Round 2 coverage-gap deltas (read-verified, local model)

This lane read every module round 1 skipped. Its most valuable output is a class
the other lanes could not reach: **invariants that two sites must agree on, with
no test pinning the agreement** (C8, C13, C14, C15) — silent-divergence risks
rather than style debt. `fix-by` as before.

### Correctness / silent-divergence (highest value)

| ID | Finding | Evidence | fix-by |
|----|---------|----------|--------|
| **C8** | `model/files.rs` header claims the watcher is unshipped (it shipped) **and** that search uses a `.ignore`-based walk — `search/rg.rs` is **gitignore**-based and never reads `.ignore`. Worse: the file-list walk **prunes `graft/`** while the search walk **does not**; documented as intentional but **nothing tests it** | `files.rs:1-10`, `rg.rs` (read in full) | deterministic (fix comments) + **add 2 asymmetry tests** |
| **C13** | `highlight::supports_reuse` vs `registry::build`'s locals-queries: the incremental path is byte-identical to the full path **only** for languages whose registry config passes an empty locals query (JS/TS/TSX, Ruby pass `LOCALS_QUERY`). Two hand-maintained lists; **no test** tied them together, so adding a locals query to e.g. Python silently desyncs incremental vs full highlighting. **Corrected while pinning: the invariant is one-directional — `supports_reuse(id) ⟹ !has_locals_queries(id)`, NOT an equivalence** (Java/CSharp/Scheme/Clojure have empty locals queries but no reuse: they take the `Highlighter` path), so an equivalence test would have been unwritable | `highlight.rs`, `registry.rs` | **PINNED** (lane `invariant-pins`, `c0421c2`): registry accessor + the `LanguageId::ALL` implication test; discriminating (adding Python to the locals set makes it fail) |
| **C14** | **Four walks, no cross-pins**: file list (hidden+gitignore+`graft/` prune), `rg` (hidden+gitignore, no graft/target prune), `cargo::walk_rs_files` (target+dot dirs, no gitignore), `nav/index` (built from FileList). Differences are user-perceivable and documented, but untested; no "walk policy" table exists | all four read | deterministic: pin the documented asymmetry in one test each + a docs table |
| **C15** | **`is_word_char` families disagree on Unicode**: `store.rs:5261` and `references.rs` are Unicode-aware; `rg.rs`'s sink and `cargo.rs::is_ident_char` are ASCII. So `café` splits differently between the two M-? paths. `rg.rs` even carries a branch that exists only because of this class of mismatch | 4 of 5 sites read | deterministic: one shared predicate in `model/` + a multibyte end-to-end test |
| **C5** | `StatusTree::move_down` **wraps to the first section while its doc says "wraps nowhere"**, and `move_up` does not wrap — asymmetric `n`/`p`, doc contradicts code, no test pins either | `model/sections.rs` | **RESOLVED — magit-consistent, see below** |
| **C17c** | `Registry::save` / `Recents::save` write `projects.json`/`recents.json` non-atomically via `fs::write`; load tolerates corruption by returning empty ⇒ **a crash mid-write silently wipes the known-project list** | `model/project.rs` | deterministic: temp-file + rename |

### Dead API with wrong justifications (all read-verified)

| ID | Finding | fix-by |
|----|---------|--------|
| **C9** | `Buffer::{byte_count,line_to_byte,byte_to_line}` — 3 `#[allow(dead_code)] // spec-required byte↔line conversion`; **zero non-test callers**, and the codebase deliberately uses the `try_*` variants instead | delete all three (or link the spec) |
| **C10** | `nav/xref.rs`'s `Xref` trait: `#[allow(dead_code)] // …LSP backend will use it later`; no non-test use (`dyn Xref` only inside its own file); the store calls `SymbolIndex` directly | **supervisor decision** (see D2) |
| **C11** | `highlight.rs::highlight(rope, config, _lang_id)` — parameter never read | drop it |
| **C12** | `LanguageId::name`'s allow is unneeded (it IS used at `store.rs:8562`) **and** its justification is wrong (cache key is `(path,mtime,theme)`); `FileList::len` has no non-test caller | fix comment / delete `len` |
| **C17a** | `StatusTree::visible_ids()` and free `collect_all_visible()` are the same DFS twice | merge |
| **C17b** | `buffer.rs::is_locally_owned`'s `path.is_some()` disjunct is redundant | simplify |
| **C17d** | `cargo.rs` error text "…not yet provided by the app" — the app HAS provided scope hints since 007-03 | reword |

### Stale docs/comments (one batch sweep)

**C6** `keymap.rs` says "C-SPC is NUL"; its own test pins `Char(' ')`+ctrl (NUL only via `^@`) · **C7** `ui/tree.rs` says "reverse-video cursor" but implements the shared no-invert bar · **C8-claims** above · **C17e** `events.rs` subscriber doc attributes the index refresh to "issue 05's" consumer instead of `AppStore`. Round-1/2 already found the `(future wiring)` family; the coverage lane confirms **6 sites in `nav/index.rs`** plus **~13 more suspected in `store.rs`** needing a per-item caller check.

### Test-suite hygiene (C16, all read)

- `syntax/highlight.rs::measure_full_vs_incremental` runs a **3.2 MB corpus with a wall-clock `incr*3 <= full` assertion inside the normal unit suite** → `#[ignore]`/`perf` (it admits the margin is for "a loaded shared box").
- `watcher.rs`'s two "must not publish" tests assert absence after a fixed settle sleep — the flakiest asserts in the file; keep the control-write pattern, document the 500 ms assumption.
- `drain_until_finished` duplicated ×3 (`rg.rs`, `references.rs`, `occur.rs`) → one `search/mod.rs` test helper.
- `pipeline_cancel_stops_promptly` creates **2,000 dirs + files per run** — the battery's heaviest I/O test.
- `command.rs::new_store()` builds `AppStore::at(dir, base)` with a tempdir that drops at function end (comment admits writes are no-ops) — any future persisted-state assertion here will fail confusingly.

### Confirmed from earlier rows

C1 = M1 (the `nav/index.rs` stale-allow cluster, now with the production callers spelled out: `store.rs:41,7837-7840,1592`, `main.rs`) · C2 = R5 (`MagitStatusView` ≈ `MagitRowsView`, **both live**: `root.rs:691` vs `:707`) · C3 = M4/the truncate trio, now with the concrete bug (`PickerCanvas::draw` passes **cell** widths into a **char**-counting truncate ⇒ CJK overflow, unpinned by any test) · C4 = M4 + cursor-bar ×5 + scroll-indicator ×4.

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

1. **012-01** inventory (landed) → **012-02** store split pattern.
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

## Follow-ups from landed lanes (outside plan 012)

- **picker-density — LANDED** (`6d01393`, lane `feaeaf2`+`a4ae0f9`, gate-reviewed
twice with execution). Delivered: name-first rows where the **name owns the
space** and the detail truncates tail-keeping (cell-aware `truncate_left`);
canvas `min(candidates+2, viewport-1)`; no preview split when the preview is
empty; contiguous selected-row bar (gap-only paint — iocraft's `invert` is SGR 7,
so painting text cells too would be white-on-white). 6 new tests; 835/0/2; clippy
clean; `gate.sh full` OK 15/15; zero deleted assertions.
- **picker-density follow-up — LANDED** (`5d53a35`, lane `08c3bde`+`33a3d74`, gate-reviewed
  twice; the re-gate audited all 16 `PickerCandidate` constructions to prove nothing lost
  information). All eight remaining pickers converted; palette deliberately left
  display-only and pinned. The two implementation-level `flow_tests` substrings were
  re-pinned on the requirement (imenu kind tag, impl grouping/indent, free fn flat) —
  strictly stronger: one contiguous-display assert per row became two state asserts plus
  two render80 shape asserts. Two gate-found row quirks fixed with discriminating tests.
  (Original entry, kept for the record:)
  convert the remaining single-string pickers — **imenu, impls, palette,
  find-file, recents, buffers, project, branch, stash** — to name-first +
  right-aligned detail, and **re-pin** `src/app/flow_tests.rs:3531` (exact
  `display == "  new  [fn]"`) and `:3536` (`frame.contains("  new  [fn]")`).
  Both pins are **implementation-level** per the test-authority policy: they yield
  to the requirement and get updated, and the requirement-level content they
  protect (imenu shows kind tags; impl methods grouped/indented under the struct)
  must be re-expressed in the new shape rather than dropped.
- **Optional (gate observation, acceptable to omit)**: an ANSI/SGR-level assertion
  for the selected-row bar, which would pin the cross-crate iocraft interaction
  rather than the per-cell model. Justified as optional: iocraft is lock-pinned to
  0.9.1 and its own suite pins its SGR emission.

## Gate reliability (higher priority than the cosmetic items)

**Four distinct flake classes are now characterised — do not conflate them:**

1. **Cursor CUP race (app-side; root-caused; NOT fixed)** — the hardware-cursor write is
   emitted from a spawned task 12 ms after each frame, i.e. outside the synchronized-output
   region. The deflake lane's harness gate (`cup_settle`, landed `b47e03c`) makes a starved
   CUP a **loud protocol failure** instead of a silent mis-read, but it does **not** fix the
   race: re-measured under confirmed load the drive is **~50% failure (7/10, then 3/8)**,
   dominated by `cup=None` occurring *after* the full 5 s wait. A second sub-class was also
   observed: **pre-frame starvation** (`?25l=0` — the startup hide itself missed; plus
   stale-content failures), which the item-2 gate structurally cannot cover.
   **The app-side fix is verified impossible in-fence**: iocraft 0.9.1's `write_canvas`
   parks the cursor at the canvas bottom row inside the sync region after every row write,
   and there is **no post-canvas/post-frame seam** (`use_effect` fires before
   `write_canvas`; `use_output` writes above the canvas; the `Terminal` handle is owned
   inside the render-loop future). A true fix needs an iocraft-side hook → **upstream PR or
   vendored patch** (decision open with the user; with ~50% under load the case for it is
   now stronger than "accept the occasional lag").
   **→ written up as plan 013** (`.agents/plans/013-cursor-race/`): the mechanism with
   exact iocraft + redline anchors, what is NOT broken (position and content are correct;
   only the cursor *report* is late), the four options, and three issues — upstream PR
   (recommended) · vendored patch (contingent) · the pre-frame harness sub-class.
2. **`git::repo` assertion flake (environmental, correlated)** —
   `stage_file_then_unstage_matches_cli` fails under two concurrent full suites with swap
   exhausted, green in isolation; no timing/ordering assumption to pin. Swap is a
   **correlate, not a deterministic cause** (green runs observed under it).
3. **External SIGTERM of the test harness (unattributed)** — a `test` stage SIGTERM-killed
   at ~test 812/847 and a rustc killed during a build, no OOM-kill logged. **Four**
   mechanisms eliminated: `gate.sh` wraps only PTY steps; timing rules out a per-tool
   deadline; the PTY harness only `os.kill(self.pid, SIGKILL)`s its own children; and no
   oom manager (earlyoom/nohang/systemd-oomd) is installed.
**FINAL VERIFICATION (end of the 012 session): `tools/gate.sh full` on main passed —
`GATE RESULT: OK (full)`, all 21 stages, exit 0**, including `check_cursor_stream.py`
(71 s) and `sweep_flows` 15/15. This closed all seven per-lane deferrals in one serial pass
on the assembled 17-lane tree, and it **refines class 1**: the cursor drive is 80/80 on an
idle box and fails only **under concurrent build load**, so the race is *load-induced*, not
intermittent-at-rest. The iocraft-side fix is therefore less urgent than the ~50%-under-load
figure suggested — but the race is still real (a starved CUP leaves the cursor stale for a
frame), so the upstream-PR/vendored-patch decision stands as an open, evidenced item.

4. **Swap exhaustion as a load multiplier** — not a flake itself, but the condition under
   which (1), (2) and (3) all appear. Resource guards therefore check `free -g` **and swap**
   and defer the battery rather than produce an unattributable FAIL.

**Rules that follow**: auto-retry is wrong for (1)–(2) — it hides a real failure — but
legitimate for a **signal-kill** (cargo exit 143/137), which is not a test outcome at all;
any such retry must be signal-specific and logged, never triggered by exit 101.

## What remains (after the 17 landed lanes)

Everything plan 012 targeted structurally is done and the final full battery on main
passed (`GATE RESULT: OK`, 21/21 stages, incl. `check_cursor_stream` 71 s). The true
remainder:

| Item | Kind | State |
|---|---|---|
| cleanup tail: inline `ui/views/` (F-5) · dedup the 10 `git_cli` + fixture family in `store/tests` · optional `Snapshot::from_store` (removes 67 `pub(super)`) | hygiene | **LANDED** (`cleanup-tail`; the optional `Snapshot::from_store` is the CLOSED row below) |
| iocraft post-canvas hook (the cursor race's only real fix) | decision + upstream PR / vendored patch | **open — user decision** |
| clearing swap (`swapoff -a && swapon -a`) to stop load-induced flakes | ops | **open — user decision** |
| plan 012 archival | mechanical | ready on request |
| ~~`Snapshot::from_store`~~ | design | **CLOSED — recommendation SUPERSEDED.** The tail lane skipped it; the gate verified the skip and showed the earlier `root-split` suggestion ("a constructor would be materially better design") **under-counted the read sites**: the 66 fields are read from the render literal (~66 lines), ~20 `geometry.rs` reads, two full-literal test fixtures, **and post-construction field mutation in the geometry tests** (which a constructor cannot address without setters); the render path needs `&mut` store accessors so the signature cannot be `&AppStore`. Private fields would cost ~60 accessors + ~100 rewrites on the render hot path for zero behavioural gain. The 67 `pub(super)` are the correct end state (`a1c2e47`). |
| **A7 — split `store/navigation.rs`** (2,394 lines, ONE impl, ~70 methods) | structural (pure move) | **LANDED** — `navigation/{mod,definitions,imports,xref}.rs`, all under the 1,500 line bar (the audit below is what produced the split) |
| re-land input coalescing (`git revert 0ddaf46`) | UX (user asked to park it) | parked, recipe in the commit |
| Shape B (rust-analyzer as a library) | architecture | only if written-down-types ever bites |

## A7 — the criterion the plan set is NOT met (found by a post-landing audit)

`PLAN.md` § Success criteria says: **"No `src/` file over ~1,500 lines"**. Measured after
all 17 lanes landed, six `src/` files still exceed it — and the earlier claim that "every
oversized file is decomposed, the rest are cohesive" was **too generous**. The audit:

| File | Lines | Verdict |
|---|---|---|
| `src/app/flow_tests.rs` | 3,709 | test file, cohesive (long key-sequence flows) — twins deliberately kept |
| `src/app/store/tests/navigation.rs` | 3,074 | test file, cohesive but further splittable |
| `src/app/store/mod.rs` | 2,445 | struct + 18 core methods (the update/dispatch core) — cohesive, over the bar |
| **`src/app/store/navigation.rs`** | **2,394** | **GRAB-BAG — three unrelated concerns under a vague label** |
| `src/syntax/queries.rs` | 1,653 | cohesive (symbol extraction) — over the bar |
| `src/app/store/tests/notes.rs` | 1,550 | test file, cohesive |

`navigation.rs` is the real finding. It is one `impl AppStore` containing three concerns
whose line ranges are already **contiguous**, so it splits by range exactly like the
landed lanes (pure move, same multiset-partition proof):

- **jump history** — `open_resolved_source` (34), `navigate_to_entry` (127), `jump_back`/
  `jump_forward`, `record_jump`, `current_jump_entry`, `open_external_path`;
- **resolver + language-specific import paths** (the bulk) — `resolver_scope_for` (860),
  `use_path_for_symbol` (913), `use_decl_path` (985), the `js_ts_*`/`python_*`/`go_*`
  families (through `go_string_content` at 1752), plus `xref_*`/`find_implementations`/
  `start_symbol_resolution`/`apply_resolve_event`/`symbol_at_point`;
- **imenu / outline** — `open_imenu` (2230), `open_imenu_picker` (2328),
  `open_symbol_picker` (2341), `which_function` (2371).

The middle concern is the strongest signal that the label was wrong: ~20 language-specific
import-parsing free functions (js/ts/python/go/rust) are "navigation" only by accident.
Suggested split: `navigation/{mod (jump history), resolver, imports, imenu}.rs`, or
`navigation.rs` + `imports.rs` + `imenu.rs` if the resolver half stays whole. Note the
`store/tests/navigation.rs` (3,074) split should follow the same seams, so it may be a
separate stage.

## The next tier (post-012 agenda, measured)

After the 18 landed lanes the tree is clean (one `TODO`/`FIXME` in the whole
repo), so what remains is organization, not rot. Written up as:

| Doc | Scope |
|---|---|
| `.agents/plans/014-crate-extraction/PLAN.md` | **crate extraction** — measured from the import graph: `syntax` is a TRUE LEAF (its row is empty) and `git` is a production leaf whose only inbound edge is TEST-ONLY (`git/repo/tests.rs:8`), so both extract cleanly; `model` is contingent (it reads git types in production via `sections.rs`); **`ui` must NOT be extracted** (it depends on `app::store` — the edge is inverted). |
| `.agents/tasks/issue-git-test-harness.md` | the **second git-harness cluster** the store dedup couldn't see: 4 × `git()` + 3 × `init_repo()` + 2 × `git_cli()` in `git/{blame,log,refs,repo/tests}.rs` and `ui/{magit_status,rows_view}.rs`, all `#[cfg(test)]`. Nine copies of the hermetic env block = nine chances to forget it. |
| `.agents/tasks/issue-hygiene-sweep.md` | 4 dead `pub` constructors · the `#[allow(dead_code)]` audit (**48** at spec time, not 46 — A2 added two) · stale refs (`queries.rs:8`, 2 insta `.snap` sources, `point_byte_offset`) · the `model::file()` twins. |
| `.agents/tasks/issue-guardrails.md` | **the guardrails** (Tier 1, specced) — measured 8 `unsafe` sites with **zero** `SAFETY:` comments (6 production in `main.rs`'s tty reroute, 2 test-only `set_var` in `git/commit.rs`), no `[lints]` table, no crate-level lint attrs, no hook and no CI. The `set_var` sites get an **investigation, not a comment** (cargo's threaded harness may make them genuinely unsound). `gate.sh fast` verified PTY-free, so it is a legitimate pre-push payload. |

**User-requested features specced this session** (both from the hands-on UX loop):

| Doc | Scope |
|---|---|
| `.agents/tasks/issue-match-highlight.md` | **match highlighting in the buffer view** — all visible matches one colour, the selected one another ("the cursor is hard to see when I jump to a search result"). **LANDED** (the second-pass overlay is in `src/ui/file_view.rs`). `render_row` is segment-based and the store already pre-computes visible rows, so the overlay is a second pass over the segment list. Sources: isearch or the persisted search state after a jump. |
| `.agents/tasks/issue-index-gitignore.md` | **the indexer's incremental path must respect `.gitignore`** — diagnosed precisely: the *full* build already does (it consumes the gitignore-filtered `FileList`), but `refresh_in_place` `set_file()`s **any** changed path the watcher reports, so ignored files enter the index after the fact. The fix extracts one `is_gitignored` helper for the ancestor `.gitignore` chain and makes the indexer the **third walker** in R3's agreement invariant. **SUPERSEDED — LANDED**: the core fix landed (`ca4e743`); the follow-up was re-specced as `issue-ignore-predicate.md` (also LANDED). |

## The logic-organization round (measured, per the user's criterion)

Test file size is not a criterion; **production logic organization is**
(`.agents/skills/plan-process/SKILL.md` § Maintainability criterion). Measured with a
brace-matching survey of production functions (test modules excluded; 1,633 fns
total, 40 over 80 lines, 8 over 150):

| # | Target | Evidence | Verdict |
|---|---|---|---|
| A1 | **`Root`** | `src/ui/root/mod.rs:42` — **504 lines, nest 7**, six jobs in one fn (measured: contexts/size 42-83, event handler 84-143, five futures 152-266, the `Snapshot` literal 268-359, cursor effect 381-394, view dispatch 402-463, frame assembly 464-546) | **LANDED** (`issue-a1-root-decompose.md`) — named helpers; PTY-covered so low risk. Note: this is NOT the rejected `Snapshot::from_store` (fields stay `pub(super)`; only the literal-building *block* moves) |
| A2 | **the keymap is data written as code** | `AppStore::at` (`store/mod.rs:1557`, **200 lines**, 22 binds) + `ViewId::keymap` (`:237`, **284 lines**, 120 binds) = **142 imperative binds** (79 distinct commands) inside *constructors* | **LANDED** (`issue-a2-keymap-table.md`) — a `(sequence, command)` table. The design is already decided by the codebase: `keymap::parse_sequence` (emacs notation, tested at `keymap.rs:457-514`) **already exists and is already used in production** by `config.rs:128`'s `Config::validate_bindings`, so the user-config path speaks `"C-x C-f"` while the built-in keymap uses `Key::ctrl_char('x')` |
| B1 | `key_event` → **A3 candidate** | `store/keys.rs:16` — **345 lines, nest 5**. NOT a dispatch match: it is a **hand-rolled modal chain** — 7 early-return guards (`quit`, `quit_prompt_active`, `menu_open`, `discard_armed`, `toggle_ro_active`, `picker`, `top_view()==CommitEditor`) with the picker and editor blocks inline (~50 lines each), ending in `dispatch_key`. Some modals already route to named methods (`quit_prompt_key`, `menu_key_event`, `discard_key_event`) — so the pattern is half-applied | **LANDED** (`issue-a3-modal-chain.md`): 16 sequential guards, 5 already routing to named methods, **11 with inline bodies (~300 of the 345 lines)**. Step 1 (mandatory) extracts each inline body to a named method, leaving the chain explicit so `key_event` reads as a priority list of named modals. Step 2 (an ordered handler table) is explicitly OPTIONAL and must be argued — an `if` chain in priority order *is already* a readable priority list, unlike A2's ~480 lines of `bind()` data. **Behaviour-sensitive** — the PTY battery is mandatory |
| B2 | `is_path_segment` | `syntax/node/paths.rs:23` — **151 lines for a predicate** | naming/scoping failure, not a size failure |
| B3 | per-language logic | `resolve_go` 143, `resolve_js` 127, `extract_all` 139, `extract_rust_tables` 136 | the *config* is table-driven (`LanguageSpec`); ask whether the *logic* shares a shape worth a trait + per-language hooks |
| B4 | tuple parameters | `crate_xref_outcome(..., at: Option<&(String, String)>)` (`navigation.rs:1897`) | a named struct carries the meaning; also a **measurement caveat**: my param counter miscounted this as 9 args — it is 6 (tuple commas) |
| B5 | deep hand-written nesting | `js_ts_require_path` nest 8, `start_watch` nest 8, `set_file_tables` nest 7 | examine individually |

**NOT findings** (state these so nobody "fixes" them): the iocraft components report
nest 8–9 (`ResultsView` 9, `MagitStatusView`/`MagitRowsView`/`HomeView` 8) — that is
`element!` macro nesting, not tangled logic; `register_magit` (135) and
`register_motion` (125) are long but **nest 1** (flat registration data); and test
functions are excluded by the criterion above.

### Follow-ups from A2 (keymap table)

- **`config.rs` could share the loader.** Both paths now use `parse_sequence` +
  `KeyMap::bind`; the remaining duplication is the `Result`-returning table-loading
  loop. A shared `bind_table(&mut KeyMap, &[(&str, &str)]) -> Result<(), String>`
  would finish the unification the A2 lane started (its own report names this).
- **`Key::tab` / `Key::alt_char` now carry `#[allow(dead_code)]`** (production no
  longer calls them; the tables are strings, so only tests use them). The
  hygiene sweep's allow-audit must judge these: is `#[allow]` + a stated reason
  right, or should the constructor be deleted / `#[cfg(test)]`-gated? An allow with
  no stated reason is exactly what that audit exists to catch.

### Follow-ups from the isearch-column lane (user-reported bug + audit)

**Fixed:** isearch now lands the point on the match's **column**, not the line start.
`isearch_jump_to_current` used `try_byte_to_line` then `set_point_line` (col 0); it now
uses a new `Buffer::try_byte_to_line_col` (explicit byte→char: `try_byte_to_char` −
`try_line_to_char`) and lands with `set_point(line, col, col)`. Pinned by a
**multibyte** test (`"café omega"` → char 5, not byte 6) — the byte/char trap was real.
The `set_point_line` doc was corrected (it is the *col-0* landing, not a blanket rule).

**My spec was wrong about a second defect:** `M-,` was already correct
(`navigate_to_entry` has used `set_point(entry.line, entry.col, entry.col)` since
`0bc74c3`). The lane added `jump_back_restores_recorded_column` as a **regression pin**
(it would fail if the col were zeroed). See the skill rule "a line number is not
evidence of which function it is in".

**Column-availability audit (report-only; four callers still drop a column):**

| Caller | Column source | Notes |
|---|---|---|
| `buffers.rs:458` `C-x C-x` `exchange_point_and_mark` | mark is a byte offset | **most emacs-visible of the four**; the new `try_byte_to_line_col` is exactly the seam it needs — **LANDED** (`issue-column-landings.md`) |
| `search.rs:156` isearch **cancel** | not recorded — `IsearchState` stores only `pre_search_line` | needs a `pre_search_col` field to restore the original column on `C-g` |
| `search.rs:329` project-search RET | `Hit.col: Option<u64>` (byte col; `None` for regex) | |
| `definitions.rs:186` unique-def jump | `Symbol.start_byte` | derivable |

**One-line doc fix:** `JumpEntry.col` (`src/app/store/mod.rs:616`) is documented as
"byte offset within the line" but is actually a **char index** (recorded from
`point_col()`, consumed by `set_point`). Zero behaviour impact; wrong doc.

**Still out of scope:** a match beyond the pane width clamps at the edge (no hscroll).

**The four are now specced as one lane** (`.agents/tasks/issue-column-landings.md`,
`C-x C-x` first), together with the two doc fixes (`set_point_line`'s caller list,
`JumpEntry.col`'s byte-vs-char claim) and — folded in because that file is already in
scope — the `try_byte_to_line` `#[allow(dead_code)]` that the isearch gate flagged as
suppressing a *standing* warning with an inaccurate reason (it was routed here from the
hygiene sweep; the hygiene sweep's allow-audit will simply find one fewer).

## Known follow-ups from gate findings (low priority)

- **PTY flake, unattributed** (descriptor-table gate): the first `gate.sh full` run on a
  frozen tree ended `GATE RESULT: FAIL` and an identical re-run was `21/21 OK`. The
  flaking suite was not identifiable because the first run's output was tail-truncated.
  The store-concerns gate was asked to re-run on failure and to capture the full log.
  If it recurs, identify the suite from a complete log — a flaky PTY leg is a gate
  reliability problem, not a cosmetic one.
- **Stale `store.rs` references in source comments** (outside every landed lane's
  fence): `src/app/flow_tests.rs:50` ("`store_with_index` in store.rs"),
  `src/syntax/queries.rs:8` ("in `app/store.rs`"). Cosmetic; fold into whichever
  lane next touches those files.
- **insta snapshot metadata** still records `source: src/app/store.rs` in the two
  `store/snapshots/*.snap` headers. insta ignores it (the tests pass) and rewrites
  it on regeneration — leave unless a snapshot is regenerated anyway.
- **`point_byte_offset`** remains in `store/mod.rs` (its 4 call sites are in-impl).
  Revisit when the navigation concern moves.

## Start here

`nav/index.rs:425–650` (M1+M2, one contiguous verified batch delete), then
`crates/redline-resolve/tests/golden_go.rs` ↔ `golden_python.rs` (T1).

## 01-inventory.md (full text)

# 01 — Concern inventory + module map

Phase 1 · Depends on: —

## Objective

Plan the `store.rs` split on measured facts: group the 869 methods into
coherent concerns with line ranges, name the target modules, list the
mechanical constraints, and publish it as `docs/architecture.md` so every
later stage (and every reviewer) works from the same map.

## Steps

1. Inventory `src/app/store.rs`: the struct's fields (state) by concern;
   every `fn` with its line and a one-phrase tag; the test module's
   sections. Use grep/read (no AST tooling needed) and produce a table.
2. Propose the module layout (`src/app/store/mod.rs` + submodules) with,
   per module: the methods it takes, the fields it needs (and therefore
   which fields must become `pub(crate)`), its line count estimate.
3. Note the mechanical constraints: multiple `impl AppStore` blocks are
   legal in the same crate; field visibility is the only real friction;
   `#[path]`-style module wiring used by `flow_tests.rs` today; the
   store's `#[cfg(test)]` module move.
4. Write `docs/architecture.md`: the crate/module map (all of `src/` +
   `crates/`), the store split plan, the constraints, and a one-line
   "how to find things" guide.
5. Do NOT move code in this issue. It is analysis + docs only.

## Verification

- The inventory's per-module method counts sum to the real method count
  (869) within a stated tolerance for free functions/constants.
- A reviewer spot-checks 10 tagged methods against the file.
- No source file changes; `cargo test --workspace` unaffected (still run it).

## Files

`docs/architecture.md` (new). Read-only everywhere else.

## 02-language-descriptor-design.md (full text)

# 02 — Per-language descriptor table: full design (raw lane report)

Provenance: round-2 structural scan (local model, read-verified), $PWD at HEAD
`8159c96`→`46dfe05` era. Copied verbatim from the lane's output so the
19-row inventory and the 7-step migration order survive outside the session dir.
See `00-worklist.md` §Round-2 structural deltas for the summary and staging.

---

# Code Context — round 2: structural verification + split/descriptor designs

Read-verified by full or targeted reads (no grep-only claims below). All line numbers are against the current tree at `/home/gary/dev/red`. `.agents/` excluded; never read.

## Files Retrieved
1. `src/syntax/queries.rs` (1–410, 412–511, 949–1008; full map via item grep) — per-language query consts + extraction engine + 690-line tests.
2. `src/syntax/node.rs` (54–356, 355–474, 545–679; full item map) — the three parallel dispatch tables + two extra `LanguageId` matches + 1,270-line tests.
3. `src/syntax/registry.rs` (full, 502) — `LanguageId`, `name()`, `ALL`, `ext_map`, `GrammarRegistry::build`, `highlight_query_for`.
4. `src/syntax/highlight.rs` (240–350 + item map) — `supports_reuse`, `reuse_language`, `ReuseEngine`, `build_reuse_engines`.
5. `src/syntax/tokens.rs` (30–70 + item map) — `token_class_query_for`: a 7th per-language match site.
6. `src/syntax/cache.rs` (item map) — clean, small.
7. `src/nav/index.rs` (1–560 + full item map) — `SymbolIndex`/build/refresh/`IndexProgress`/`IndexBus` in one file.
8. `src/ui/root.rs` (item map) — input translation + cursor math + `Root` + widgets + 789-line tests.
9. `src/app/command.rs` (100–170, 785–810; register-count) — `seed()` verified.
10. `src/git/repo.rs` (full item map) — `GitRepo` impl + free hunk math + 805-line tests.
11. `src/app/store.rs` (22,992 lines! full item map + method inventory of the impl) — the file round 1 never flagged.
12. `src/app/watcher.rs` (1–215), `src/app/events.rs` (item map), `src/app/keymap.rs` (1–40 + item map) — clean.
13. `src/model/{sections,buffer,files,project,text_width,mod}.rs`, `src/ui/views/{mod,buffer}.rs`, `src/search/{rg,occur,references,mod}.rs`, `src/syntax/mod.rs` — item maps + targeted reads.
14. `crates/redline-resolve/src/{lib.rs (1–260), cargo.rs (item map), providers/mod.rs (full), Cargo.toml (full)}` — resolver layer.

## Claim verification (round 1)

**Claim 1 — queries.rs mixes ~245 lines of query consts with the engine: CONFIRMED.**
Brace-verified layout (1,697 lines):
- 16–169: shared types (`SymbolKind` 31, `Symbol` 66, `StructField`/`ImplKind`/`LocalBinding`/`ImplMethod`/`RustTables` 88–153, `ImplAcc` 160).
- 170–410: per-language consts (241 lines): `RUST_QUERY` 173, `RUST_TABLES_QUERY` 195, `TYPESCRIPT_QUERY` 226, `JAVASCRIPT_QUERY` 238, `PYTHON_QUERY` 248, `GO_QUERY` 253, `C_QUERY` 261, `CPP_QUERY` 267, `TOML_QUERY` 273, `JSON_QUERY` 280, `YAML_QUERY` 284, `BASH_QUERY` 288, `MARKDOWN_QUERY` 292, `JAVA_QUERY` 305, `C_SHARP_QUERY` 321, `CLOJURE_QUERY` 348, `C_SHARP_HIGHLIGHTS` 361 (vendored `include_str!`), `CLOJURE_HIGHLIGHTS` 379, `RUBY_QUERY` 383, `SCHEME_QUERY` 407.
- 420–465: `flat_define_kind` (Scheme/Clojure gating) + `query_for`.
- 468–495: `language_for` (grammar per language — a grammar-pin duplicate, see Deliverable B).
- 501–659: `ThreadLocal` + `extract_all` (engine core).
- 660–817: `extract_symbols` 665 + `extract_rust_tables` 680–817.
- 818–949: lexical seams `rust_self_type_at` 828, `rust_binding_type_at` 881.
- 950–1007: `kind_of`. 1008–1697: tests (690 lines, 41%).

**Claim 2 — node.rs three parallel dispatch tables: CONFIRMED, counts slightly off.**
- `is_path_segment` 201–356 (one 156-line `match lang`, 11 arms + default).
- `is_*_identifier_kind`: 355–581 = **16 fns** (15 per-language predicates + dispatcher `is_identifier_kind` 555–573), not 17.
- `*_scope_path`: 576–981 = **14 fns** (13 per-language walkers + dispatcher `scope_path_for` 623–642), not 16. Scheme/Clojure have no scope walkers (flat grammars — consistent, but it's another silent per-language default).
- Two EXTRA `LanguageId` matches round 1 missed: `parse_source` 96–103 (a 18-language "parseable" whitelist — a 4th identity list duplicating `LanguageId::ALL` and `ext_map`) and `in_identifier_position` 175–190 (Ruby `call`-shape + JSON pair/key gates).
- Tests 982–2251 = 1,270 lines = **56% of the file**.

**Claim 3 — nav/index.rs conflates index + engine + bus: CONFIRMED.**
`Location` 40, `SymbolIndex` 50–425 (fields + 20 methods incl. the tricky same-check-before-removal logic), `TraitImpl`/`TraitImplLocation` 82/91, `IndexProgress` 426–497 (+ `PROGRESS_STEP` 437), `extract_file`/`build_index`/`refresh_in_place`/`enclosing_symbol` 498–597, `IndexEvent`/`IndexBus` 598–648, tests 649–1075 (427 lines). Many items carry `#[allow(dead_code)] // used by the background index thread (future wiring)` — i.e. parts are currently test-only; verify liveness before splitting.

**Claim 4 — ui/root.rs grab-bag: CONFIRMED.**
`code_to_app_code` 36–94 (iocraft `KeyCode` → app `KeyCode` translation — round 1 called it `to_app_key`; that name does not exist), `cursor_cell` 95–196, `click_pane` 197–203, `Snapshot` 204–300 (hand-built read model of the store), `Root` 301–813 (513 lines), `StaticRenderWidth`/`render_at_width` 814–841 (public static-render seam consumed by app tests), `Minibuffer` 847, `StatusLine` 894, tests 964–1752 (789 lines = 45%).

**Claim 5 — command.rs `seed()` 694 lines / 108 commands: CONFIRMED (697 lines).**
`seed()` 109–805; exactly **108** `reg.register(Command::new(…))` calls (grep-counted). Commands already carry a category string ("navigation"/"files"/"buffers"/"region"/…) — the split axis exists in the data.

**Claim 6 — git/repo.rs mixes queries + mutations + pure hunk math: CONFIRMED.**
`GitRepo` struct 16, single `impl` 23–664 (641 lines, 26 methods: status/diff queries `branch` 35/`status` 76/`diff` 191; index mutations `stage_file` 211/`unstage_*`/`stage_hunk` 252/`discard_*` 390–541/`apply_hunk_to_index` 611; workdir helpers 542–583). Free fns 665–908: pure hunk math `revert_hunk_in_content` 676, `reverse_apply_hunk_in_content` 721, `split_lines_inclusive` 806, `ensure_utf8` 825, plus index-entry plumbing `copy_index_entry` 833/`find_path_in_tree` 852/`default_entry` 885. Tests 909–1713 (805 lines = 47%, 21 tests incl. a real-repo snapshot at 1684).

**Claim 7 — layering audit clean: CONFIRMED, with one nuance.**
- `app/` production code: zero `crate::ui` imports. The only app→ui references are in `src/app/flow_tests.rs` (lines 70, 3295: `crate::ui::root::render_at_width`), and that file is test-only — hung off `store.rs` at line 22,990 with `#[cfg(test)] #[path = "flow_tests.rs"] mod flow_tests;`.
- `syntax/`: zero imports of `crate::{app,nav,git,ui}` (module doc states the layering rule; verified in use blocks).
- `nav/`: zero code-level app/ui imports (one doc-comment link to `AppStore::apply_index_event` at index.rs:617 — not a dependency).
- `crates/redline-resolve`: dependencies are `serde`, `serde_json`, `anyhow` only (Cargo.toml read in full) — genuinely app-free.
- Nuance: `ui::root::render_at_width` is a `pub fn` that exists solely for the app-side test twins (flow_tests + views tests) — a test seam exported across the layer boundary. Acceptable, but worth naming.

## New structural findings (files round 1 never opened)

**F-1 (structural, biggest miss): `src/app/store.rs` is 22,992 lines — more than 40× the next largest file and bigger than the other six claimed monoliths combined.**
Layout (item-map verified): state types 56–1,700 (~40 structs: buses `ResolveBus`/`CrateIndexBus`, `ViewId` 176–504 (a 328-line enum+impl!), picker/tree/notes/search/log/commit state, `AppStore` struct 1,429–1,700); **`impl AppStore` 1,701–11,588 = 9,887 lines, 253 `pub fn`s**; free helpers 11,588–11,804 (`window_slice` 11,703, `keep_cursor_visible` 11,715, `prefill_commit_message` 11,627, `extract_commit_message` 11,648, `point_byte_offset` 11,798, …); `#[cfg(test)] mod tests` 11,805–22,985 = **11,181 lines of tests**; `flow_tests` 3,624 lines attached. Method clusters readable from the inventory: buffers/point/kill 1,264–5,587, view/scroll/mouse 2,057–5,496, notes/annotations 2,661–3,245, picker 4,030–4,546, tree 4,596–4,736, isearch 5,672–5,840, git log/blame/commit 5,900+, index/xref 8,561–9,820, re-walk 3,715–4,029 (314 lines in one fn).
- **Location**: `src/app/store.rs`.
- **Current**: god-object; a single 9,887-line `impl` + 1,645 lines of state types + 11.2k lines of tests.
- **Issue**: no mechanical split possible of the `impl` without traits/restructuring; but 1.5–3k lines ARE extractable (see Deliverable A §7).
- **Suggested**: extract (a) notes doc parse/serialize (`parse_notes` 895 / `parse_notes_section` 930 / `parse_record_block` 958 / `serialize_notes` 1,032 / `NotesDoc` 871 → `app/notes_doc.rs`, already pure and store-free), (b) free helper fns 11,588–11,804 → `app/store/helpers.rs`, (c) test module split into `#[cfg(test)] #[path]` files by cluster (they reference private fields — must stay in-crate, `#[path]` mods work).
- **Risk**: high for anything touching the impl itself; medium for (a)/(b); low for (c). Tag: **structural**.

**F-2 (structural): `src/app/flow_tests.rs` is 3,624 lines of PTY-twin tests** in one flat module; each test rebuilds a store + renders at 80 cols via `ui::root::render_at_width`. Split candidates by sweep-flow group exist (doc says naming mirrors `tools/sweep_flows.py` flows). Tag: **tests**.

**F-3 (mechanical): `src/ui/views/` is a dead-weight directory** — `mod.rs` is 2 lines, the only child is `buffer.rs` (102 lines, one `#[component]` + embedded test). Either inline into `ui/` or it signals an unfinished extraction. Note its test module (lines 15–59) sits *above* the component it tests.

**F-4 (structural): resolver providers are the 2nd–4th largest files in the repo.** `js_provider.rs` 1,642, `go_provider.rs` 1,601, `python_provider.rs` 1,033; each owns one `ToolingProvider` impl + its own package/item-locating scan (cf. cargo.rs `locate_in_pkg` 368 / `locate_item` 398 / `line_defines_item` 442). `lib.rs` 717 is clean (core API 1–331, tests 332+); `providers/mod.rs` is a 7-line re-export. No cross-crate layering issue. Cross-provider duplication of locate/line-scan logic unverified beyond naming; flag for a follow-up read. Tag: **structural**.

**F-5 (clean):** `model/sections.rs` (761: prod 1–442 / tests 443+), `model/buffer.rs` (596), `model/project.rs` (364; four small concerns — `Project`/`Registry`/`Recents`/`ProjectStore` — fine at this size), `model/files.rs`, `model/text_width.rs`, `search/rg.rs` (1,039: pipeline 1–601, tests 602+; cohesive, though `run()` 181–368 is a 187-line fn), `search/occur.rs`, `search/references.rs`, `syntax/cache.rs` (407: prod 1–241), `app/watcher.rs` (656: prod 1–213), `app/events.rs` (201), `app/keymap.rs` (628: prod 1–447). No splits justified.

## Deliverable A — module-split plan, verified

### A1. `src/syntax/queries.rs` (1,697) → 3 modules
| Concern | Exact range | Dest | Takes | Friction |
|---|---|---|---|---|
| Per-language definition-query consts | 170–410 (241 lines, 18 consts + 2 vendored `include_str!` consts) | `queries/definitions.rs` | the 18 `*_QUERY` consts verbatim | zero — plain `&str` consts; but `C_SHARP_HIGHLIGHTS`/`CLOJURE_HIGHLIGHTS` (361/379) are `pub` and referenced from `registry.rs` (build + `highlight_query_for`) — keep them `pub` or migrate both ref sites to the Descriptor table (Deliverable B kills this) |
| Rust-tables extraction | 155–166 (`ImplAcc`), 195–225 (`RUST_TABLES_QUERY`), 660–817 (`extract_symbols`, `extract_rust_tables`), 818–949 (`rust_self_type_at`, `rust_binding_type_at`) | `queries/rust_tables.rs` | all 010-01/010-03/010-04 Rust-only machinery | private fns — move is mechanical; `extract_all` keeps a call into it; `RustTables`/`ImplKind` types stay in the parent (consumed by `nav/index.rs`) |
| Engine (keep in `queries.rs`) | 16–153 (types), 420–465 (`flat_define_kind` + `query_for`), 468–495 (`language_for`), 501–659 (ThreadLocal + `extract_all`), 950–1007 (`kind_of`) | — | — | `query_for`/`language_for` get replaced by table lookups in Deliverable B step 3 |
Tests 1008–1697 (690 lines) split by subject and follow the fns; they use `use super::*`, so `queries.rs` must re-export (`pub(crate) use`) what the test blocks still need.
**Risk: LOW** — pure moves, no signature changes. Tag: mechanical.

### A2. `src/syntax/node.rs` (2,251) → 3 submodules (internal `pub(crate)`)
| Concern | Exact range | Dest |
|---|---|---|
| `is_path_segment` | 201–356 (156 lines, one `match`) | `node/paths.rs` |
| `is_*_identifier_kind` | 355–581 (16 fns; becomes a kind-list lookup after Deliverable B step 5 — then only `in_identifier_position` 175–190 remains here) | `node/identifier_kinds.rs` (temporary home until B-5) |
| scope walkers | 576–981 (14 fns: 13 walkers + `scope_path_for` dispatcher) | `node/scopes.rs` |
| Keep in `node.rs`: `NodeInfo` 22, `node_at` 54, `scope_path_at` 77, `parse_source` 95, `innermost_at` 118, `nearest_identifier` 151, `in_identifier_position` 175 | | |
Friction: all three tables' fns are `pub(crate)`-private; `nearest_identifier` calls all three → `super::paths::…` etc. Tests (982–2251, 56%) call private fns directly and use `use super::*` — moving a table means its dedicated tests move too and the parent test block loses `super::` visibility to the moved fns unless the submodule is `pub(crate)` and re-imported.
**Risk: LOW–MEDIUM** (test shuffling is the only non-trivial part). Tag: mechanical.

### A3. `src/nav/index.rs` (1,075) → 4 modules under `nav/index/`
`index.rs` (mod root, re-exports) | `symbol_index.rs`: 40–91 + 97–425 (`Location`, `SymbolIndex`, `TraitImpl`, `TraitImplLocation`) | `progress.rs`: 426–497 (`IndexProgress`, `PROGRESS_STEP`) | `builder.rs`: 498–597 (`extract_file`, `build_index`, `refresh_in_place`, `enclosing_symbol`) | `bus.rs`: 598–648 (`IndexEvent`, `IndexBus`).
Friction: all items already `pub`; parent module must `pub use` each (external names: `nav::index::SymbolIndex`, `Symbol`, `IndexBus`… — check `nav/mod.rs` re-exports). The `#[allow(dead_code)] // future wiring` on `extract_file`/`build_index`/`refresh_in_place`/`IndexProgress` means builder.rs may be tests-only today — verify call sites (`app/store.rs` index path) before declaring it production.
**Risk: LOW**. Tag: mechanical.

### A4. `src/ui/root.rs` (1,752) → 5 modules
| Concern | Range | Dest |
|---|---|---|
| iocraft→app key translation | 36–94 | `ui/input.rs` |
| cursor/click geometry | 95–203 (`cursor_cell`, `click_pane`) | `ui/geometry.rs` |
| `Snapshot` read model | 204–300 | `ui/snapshot.rs` |
| `Root` component | 301–813 | stays |
| `StaticRenderWidth`/`render_at_width` (the cross-layer test seam) | 814–841 | `ui/render.rs` |
| `Minibuffer`/`StatusLine` widgets | 842–963 | `ui/widgets.rs` |
Friction: `Snapshot` fields are private and read inside `Root` → needs `pub(crate)` or `pub(super)`; widgets use `theme::*` and iocraft `element!` — moving is fine but each module re-imports. `code_to_app_code` maps onto `app::keymap::KeyCode` — keep the conversion UI-side (layering rule: ui may touch app).
**Risk: MEDIUM** (iocraft `#[component]` re-derivation + Snapshot visibility). Tag: structural.

### A5. `src/app/command.rs` `seed()` (109–805, 108 commands) → per-category seed fns
The data already carries category strings. Split into `fn seed_navigation(&mut Self)`, `seed_files`, `seed_buffers`, `seed_region`, `seed_git`, `seed_search`, … (one per existing category; verify the exact set of category strings across the 108 registrations when implementing — "navigation"/"files"/"buffers"/"region" confirmed by read). `seed()` becomes a one-line call list. A declarative (name, desc, category, `fn(&mut AppStore, &str)`) table is possible but the closures capture different `store` methods with different signatures — per-category fns is the mechanical 90%; the table is the optional extra.
**Risk: LOW**. Tag: mechanical.

### A6. `src/git/repo.rs` (1,713) → extract free fns; impl stays
- `git/hunks.rs`: 676–824 (`revert_hunk_in_content`, `reverse_apply_hunk_in_content`, `split_lines_inclusive`, `ensure_utf8`) — pure byte math, zero `GitRepo` state; the most testable extraction in the file (its 805-line tests are exactly these behaviors).
- `git/index_ops.rs`: 833–908 (`copy_index_entry`, `find_path_in_tree`, `default_entry`) + 825 if desired.
- `impl GitRepo` (23–664) stays whole: Rust does not allow splitting one `impl` across files without traits; a `GitStatus`/`GitStage`/`GitDiscard` trait split is a design change, not a move.
**Risk: LOW** for the extractions (private fns, called by name). Tag: mechanical.

### A7. `src/app/store.rs` (22,992) — see F-1. Only three safe moves: notes-doc extraction (~300 lines incl. types), free-helper extraction (~220 lines), test-module split via `#[path]`. The 9,887-line `impl` is **not** mechanically splittable; treat as a separate structural project (candidate axes: pull `ViewId`'s 328-line enum+impl out first, then view-stack/scroll/picker/notes/search/git-view subsystems — each needs its own state struct out of `AppStore`, i.e. a refactor, not a move).
**Risk: HIGH** (anything beyond the three safe moves). Tag: structural.

## Deliverable B — the per-language descriptor table

### B-0: the sync sites are SIX, but they are really EIGHT matches
Round 1's "six places" undercounts. Verified sites:

| # | Site | File:lines | What it says per language |
|---|---|---|---|
| S1 | identity: `name()` + `ALL` | registry.rs 36–59, 61–84 | 19 names; 18-element `ALL` (all but `Plain`) |
| S2 | extensions: `ext_map()` | registry.rs 96–152 | ext → `LanguageId` (incl. quirks: `h`→C, `mdx`→Markdown, `ini`/`conf`→Toml, `jsx`/`mjs`/`cjs`→JavaScript) |
| S3 | grammar + highlight/injections/locals in `GrammarRegistry::build` | registry.rs 254–365 | 18 arms; each picks `Language::from(…)`, highlights const, injections const, locals const (JS/TS/TSX/Ruby carry `LOCALS_QUERY`; Rust/JS/Markdown carry injections; C#/Clojure use vendored consts) |
| S4 | `highlight_query_for` | registry.rs 436–458 | **duplicates S3's highlights selection arm-for-arm** (incl. both vendored-const arms) |
| S5 | definition query: `query_for` + grammar: `language_for` | queries.rs 444–465, 469–495 | 18 arms each; `query_for`: TS **and** TSX → `TYPESCRIPT_QUERY`; `language_for`: TS → `LANGUAGE_TYPESCRIPT`, TSX → `LANGUAGE_TSX` (different grammars!) |
| S6 | reuse: `supports_reuse` + `reuse_language` | highlight.rs 250–264, 274–298 | `supports_reuse`: exactly 10 (Rust, Python, Go, C, Cpp, Toml, Json, Yaml, Bash, Markdown); `reuse_language`: same 10, **duplicating the grammar pin a third time**; the other 9 fall in one exhaustive arm with a "new variant must land here" comment |
| S7 | node predicates: `parse_source` whitelist (96–103), `in_identifier_position` (175–190), `is_path_segment` (201–356), `is_identifier_kind` + 15 predicates (355–581), scope walkers (576–981) | node.rs | see claim 2 |
| S8 | token-class query: `token_class_query_for` | tokens.rs 39–53 | TS/TSX and Cpp get dedicated comment/string queries; Markdown → `None` (documented exception); everything else → `highlight_query_for` |

**Inconsistencies / hazards found between the sites:**
1. **Grammar pinned in three places** (S3, S5, S6) — any grammar re-pin (the "grammar-bumps suite" workflow these comments describe) touches three match arms; a miss silently falls back to plain text (the ABI-guard test `all_grammars_set_language_succeeds`, registry.rs test at ~line 470s, only checks S5's pin).
2. **Highlights pinned in two places** (S3, S4) — including the two vendored `include_str!` arms appearing twice.
3. **TS/TSX aliasing is implicit**: shared definition query + shared node predicates, but two different grammars and two different `build` arms. Nothing records *why* TSX borrows TS's query; only the per-arm comments hint at it.
4. **`supports_reuse` is a non-exhaustive `matches!`** — a new language silently defaults to "no reuse" (the safe slow path), while `reuse_language`'s exhaustive arm forces a compile error. The two can drift (e.g. a language added to `reuse_language` but forgotten in `supports_reuse`, or vice versa — the warm-up test `reuse_engines_are_warm_after_cache_construction` catches it only per-asserted-language).
5. **`parse_source`'s 18-language whitelist (S7) duplicates `LanguageId::ALL` (S1)** — a fourth identity list; adding a language without touching it makes M-. dead for that language with zero compile error.
6. **Rust is special in four different ways** (tables query, self-type seam, bindings seam, `RustTables` in the index) — none of which is expressible in a bool; it's a capability enum, not a flag column.

### B-1: proposed struct and location
New file `src/syntax/language.rs` (module of `syntax`, sibling of `registry.rs`; `registry.rs` keeps only `LanguageId` + extension resolution, `queries.rs` keeps only the extraction engine):

```rust
/// One row per language — the single source of truth for everything
/// that is DATA about a language (not algorithm).
pub struct LanguageSpec {
    pub id: LanguageId,
    pub name: &'static str,               // S1
    pub extensions: &'static [&'static str], // S2
    /// None for Plain. Replaces S3/S5/S6 grammar arms. `fn` so no
    /// `const`-fn dependence on `Language::from`; called through
    /// `OnceLock`-cached `Language` handles where needed.
    pub grammar: Option<fn() -> Language>,
    /// Highlight query (S3/S4). May point at vendored include_str consts.
    pub highlight_query: Option<&'static str>,
    pub injections_query: &'static str,   // "" when none
    pub locals_query: &'static str,       // "" when none
    /// S8: None ⇒ derive from highlight_query; Some(…) ⇒ dedicated
    /// query; a per-spec `token_class_inactive` bool covers Markdown's
    /// documented exception (or: `token_class: TokenClass` enum with
    /// FromHighlight | Dedicated(&'static str) | Inactive).
    pub token_class: TokenClass,
    /// S5: None for Plain. TS and TSX both point at the shared TS
    /// definition query — the aliasing becomes one visible row-pair.
    pub definition_query: Option<&'static str>,
    /// Rust-only tables query (010-01/010-03); None elsewhere.
    pub rust_tables_query: Option<&'static str>,
    /// Scheme/Clojure: extraction gates on head-symbol text
    /// (`flat_define_kind`); None elsewhere.
    pub flat_define: Option<FlatDefineGate>,
    /// S6: true for the 10 reuse languages; false for JS/TS/TSX (locals
    /// tracking), Java/CSharp/Ruby/Scheme/Clojure (Highlighter path),
    /// Plain (no grammar). Derivable from `locals_query` + injections
    /// callback, but stated directly with a comment — it is policy, not data.
    pub supports_reuse: bool,
    /// S7 (mechanical part only): identifier-ish node kinds, e.g.
    /// Rust: ["identifier","field_identifier","type_identifier",
    /// "scoped_identifier","scoped_type_identifier","primitive_type"].
    /// Empty = none (Markdown/Plain; today: default arm `_ => false`).
    pub identifier_kinds: &'static [&'static str],
}
```

Plus:

```rust
pub static LANGUAGES: [LanguageSpec; 19] = [ …all 19 rows… ];
pub fn spec(lang: LanguageId) -> &'static LanguageSpec;      // index match, exhaustively
pub const fn parseable(lang: LanguageId) -> bool;            // replaces S7 whitelist: grammar.is_some()
```

Location rationale: it sits next to `LanguageId` (registry.rs) but owns no `HighlightConfiguration`, so `registry.rs::build()` becomes a loop over `LANGUAGES` calling `build_config(spec.grammar(), spec.name, spec.highlight_query, …)` — S3 and S4 collapse to one field each.

### B-2: the 19-row inventory (what each site currently says)
| Lang | S1 name | S2 exts | S3/S4 grammar + queries | S5 def query | S6 reuse | S7 kinds / path / scope | S8 token |
|---|---|---|---|---|---|---|---|
| Rust | rust | rs, rsi | `tree_sitter_rust::LANGUAGE`, HIGHLIGHTS + INJECTIONS, no locals | RUST_QUERY; + RUST_TABLES_QUERY | yes | kinds: identifier, field_identifier, type_identifier, scoped_identifier, scoped_type_identifier, primitive_type; path: `::` scoped pairs; scope: mod/trait/fn/impl | from highlight |
| TypeScript | typescript | ts | `LANGUAGE_TYPESCRIPT`, HIGHLIGHTS, LOCALS | TYPESCRIPT_QUERY (shared w/ TSX) | no (locals) | kinds: identifier, property_identifier, type_identifier, member_expression, nested_type_identifier; path: member/nested; scope: js_ts | **dedicated** (comment, string_fragment) |
| Tsx | tsx | tsx | `LANGUAGE_TSX`, HIGHLIGHTS, LOCALS | TYPESCRIPT_QUERY (alias) | no (locals) | same as TS (shared arms) | **dedicated** (same string as TS) |
| JavaScript | javascript | js, jsx, mjs, cjs | `tree_sitter_javascript::LANGUAGE`, HIGHLIGHT + INJECTIONS + LOCALS | JAVASCRIPT_QUERY | no (locals) | kinds: identifier, property_identifier, member_expression; path/scope shared | from highlight |
| Python | python | py, pyi | HIGHLIGHTS, no inj/locals | PYTHON_QUERY | yes | kinds: identifier, attribute; path: attribute; scope: fn/class | from highlight |
| Go | go | go | HIGHLIGHTS | GO_QUERY | yes | kinds: identifier, field_identifier, type_identifier, selector_expression, qualified_type; path: selector/qualified; scope: go | from highlight |
| C | c | c, **h** | `HIGHLIGHT_QUERY` | C_QUERY | yes | kinds incl. field_expression; path: field_expression (shared w/ Cpp); scope: c | from highlight |
| Cpp | cpp | cpp, cc, cxx, hpp, hh, hxx | HIGHLIGHT_QUERY | CPP_QUERY | yes | C kinds + namespace_identifier, qualified_identifier; path: + qualified_identifier; scope: cpp | **dedicated** (comment, string_literal, char_literal) |
| Toml | toml | toml, **ini, conf** | toml-ng HIGHLIGHTS | TOML_QUERY | yes | kinds: bare_key, quoted_key, dotted_key; path: dotted_key; scope: toml | from highlight |
| Json | json | json, jsonc | HIGHLIGHTS | JSON_QUERY | yes | kind: `string` **gated by pair/key position** (S7 `in_identifier_position` arm); no path; scope: json | from highlight |
| Yaml | yaml | yaml, yml | HIGHLIGHTS | YAML_QUERY | yes | no kinds (default false); no path; scope: none (not in walker list!) | from highlight |
| Bash | bash | sh, bash, zsh | HIGHLIGHT_QUERY | BASH_QUERY | yes | kinds: command_name, variable_name; path: command_name child; scope: bash | from highlight |
| Markdown | markdown | md, markdown, **mdx** | `HIGHLIGHT_QUERY_BLOCK` + `INJECTION_QUERY_BLOCK` | MARKDOWN_QUERY | yes | **no kinds, no path** (documented: prose is not identifier-ish); scope: markdown headings only | **inactive** (documented exception) |
| Java | java | java | HIGHLIGHTS | JAVA_QUERY | no (Highlighter path) | kinds: identifier, type_identifier, scoped_identifier(s), field_access; path: scoped/field_access; scope: java | from highlight |
| CSharp | csharp | cs | **vendored** C_SHARP_HIGHLIGHTS (include_str) | C_SHARP_QUERY | no | kinds incl. member_access/qualified; path: member_access_expression, qualified_name; scope: csharp | from highlight |
| Ruby | ruby | rb | HIGHLIGHTS + LOCALS | RUBY_QUERY | no (locals) | kinds incl. constant; **`call` gated by receiver-no-arguments shape** (two S7 arms); path: call/scope_resolution; scope: ruby | from highlight |
| Scheme | scheme | scm, ss, sls, sld | HIGHLIGHTS | SCHEME_QUERY (flat; `flat_define_kind` gates) | no | kinds: symbol-family (is_scheme_identifier_kind); no path; **no scope** (flat grammar) | from highlight |
| Clojure | clojure | clj, cljs, cljc | **vendored** CLOJURE_HIGHLIGHTS (include_str) | CLOJURE_QUERY (flat) | no | kinds: sym_lit-family; no path; **no scope** | from highlight |
| Plain | plain | — (fallback) | none | none | no | none | none |

**Cross-site inconsistencies to record in the design (and pin with a test):**
- Yaml has a scope-path *absence* but is listed in every other table — fine today, but "no scope walker" is only visible in the `_ => Vec::new()` default; the spec should carry `scope: ScopeBehavior` (None/…) so absence is explicit per row.
- `h` → C (not Cpp) and `mdx` → Markdown, `ini`/`conf` → Toml are policy quirks; they must move verbatim into `extensions`.
- Rust's `rust_tables_query` is the only per-language *second* definition query — it must be its own column, not `definition_query`.
- TS/TSX: one definition query, two grammars, two highlight rows — the spec makes the alias visible instead of hiding it in a match arm.

### B-3: dispatch changes (mechanical vs. real-thought)
Mechanically collapsible (pure data → row): S1 `name`/`ALL` (become the array), S2 `ext_map` (build the HashMap from `extensions`), S3+S4 (one `build_config` loop; `highlight_query_for` deleted), S5 `query_for`/`language_for` (field reads; `language_for` deleted in favor of `spec.grammar`), S6 `supports_reuse` (bool read; `reuse_language` deleted — the third grammar pin goes away), S8 `token_class_query_for` (field read), S7-`parse_source` whitelist (`parseable()`), S7-identifier kinds (kind-list membership — build a per-language lookup once, or a `&'static` slice scan; these lists are ≤6 entries).
Needs real thought (NOT data): S7 `in_identifier_position` (Ruby/JSON position gates), S7 `is_path_segment` (11 arms of structural predicates), S7 scope walkers (13 algorithms), `flat_define_kind` (head-text gating, keep as function behind an enum), and the reuse *policy* (the 10/8 split rationale lives in comments; a new language must consciously choose). Those stay in `node.rs` (Deliverable A2) and `highlight.rs`; the table only removes the *declarative* duplication.

### B-4: migration order (each step compiles + full test suite)
1. Add `language.rs` with `LanguageSpec` + the 19 rows + `spec()`; add a sync test: every non-Plain row has name + ≥1 extension + grammar + definition_query; ext round-trips (`resolve_language` on each ext yields the owning row); `supports_reuse == (locals_query empty && not in {Java,CSharp,Ruby,Scheme,Clojure})` as a cross-check on the stated policy.
2. `registry.rs`: `ext_map`/`ext_map_static`/`name()`/`ALL` read the table; `build()` loops rows; `highlight_query_for` deleted, callers (highlight.rs build path, tokens.rs) use `spec().highlight_query`.
3. `queries.rs`: `query_for` → table; `language_for` → `spec().grammar` (callers: store.rs ~10 sites, tokens.rs, registry ABI test — update or re-export); move consts per A1.
4. `highlight.rs`: `supports_reuse` → bool read; delete `reuse_language` (third grammar pin removed).
5. `node.rs`: `parse_source` whitelist → `parseable()`; `is_identifier_kind` dispatch → kind-list lookup (delete the 15 predicates; keep `in_identifier_position` + `is_path_segment` + scope walkers).
6. `tokens.rs`: `token_class_query_for` → `spec().token_class`.
7. Delete the now-empty matches; run the ABI guard + `reusable_pipeline_is_byte_identical_to_highlighter` (highlight.rs test, 758+) as the behavioral backstop — it already covers all 10 reuse languages.

Friction notes: `C_SHARP_HIGHLIGHTS`/`CLOJURE_HIGHLIGHTS` are `pub` in queries.rs and referenced twice in registry.rs — move the `include_str!`s into `language.rs` and update those 2 refs in step 2/3 (the only cross-module pub-const coupling). `Language::from` is not relied on being `const`, hence `fn() -> Language`. The `thread_local!` caches in queries.rs/tokens.rs key on `LanguageId` — untouched.

## Start Here
Open `src/syntax/registry.rs` (502 lines, read above in full) — `LanguageId`, `name()`, `ALL`, `ext_map`, and `build()` are the hub of Deliverable B; every other sync site is a duplicate of one of these four. Then `src/syntax/queries.rs` 170–495 (the consts + `query_for`/`language_for`) and `src/syntax/highlight.rs` 240–300 (the reuse flags).

## Additional areas (2–3)
1. **`app/store.rs` state bloat beyond the impl**: `ViewId` alone is a 328-line enum + impl (176–504) with view-specific display logic, and ~40 state structs (56–1,700) live in the same file as the store they model. Extracting the state types into `app/store/state/` (or per-subsystem files) is the prerequisite for any future store decomposition and for making the 11.2k-line test module navigable. *structural*.
2. **The vendored highlight queries have no checksum test**: `C_SHARP_HIGHLIGHTS`/`CLOJURE_HIGHLIGHTS` are pinned "sha256 at copy time" only in comments (queries.rs 352–371); nothing verifies `third_party/tree-sitter-c-sharp-0.23.5/highlights.scm` still matches the recorded hash or that the vendored file was not edited. A small `#[test]` hashing both files (hashes committed in the test) would close the gap — and it should move with the consts into `language.rs`. *tests*.
3. **Resolver provider triad** (`js_provider.rs` 1,642 / `go_provider.rs` 1,601 / `python_provider.rs` 1,033 + `cargo.rs` 705): each provider re-implements package-locate/line-scan logic (cf. `locate_in_pkg`/`locate_item`/`line_defines_item` in cargo.rs) with its own golden test (~400 lines each). Not verified line-by-line for duplication, but if the locate/scan idioms are near-identical, a shared `src/locate.rs` in the crate is the natural next extraction. *structural, needs a follow-up read of the three providers' locate sections*.
