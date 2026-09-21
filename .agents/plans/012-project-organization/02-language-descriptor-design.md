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
