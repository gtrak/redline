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
| cleanup tail: inline `ui/views/` (F-5) · dedup the 10 `git_cli` + fixture family in `store/tests` · optional `Snapshot::from_store` (removes 67 `pub(super)`) | hygiene | **in flight** (`cleanup-tail`) |
| iocraft post-canvas hook (the cursor race's only real fix) | decision + upstream PR / vendored patch | **open — user decision** |
| clearing swap (`swapoff -a && swapon -a`) to stop load-induced flakes | ops | **open — user decision** |
| plan 012 archival | mechanical | ready on request |
| `Snapshot::from_store` if the tail lane skips it | design (small) | optional |
| **A7 — split `store/navigation.rs`** (2,394 lines, ONE impl, ~70 methods) | structural (pure move) | **NEW — the plan's own criterion is unmet, see below** |
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
