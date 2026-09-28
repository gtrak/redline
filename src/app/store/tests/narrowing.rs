//! Plan 018 issue 05 — the narrowing inventory cross-check: PLAN §1's
//! inventory made un-driftable (PLAN §5.5's registry-driven /
//! cross-checked rule, the `0829ddd` precedent — the closest existing
//! form of this file is `src/gate_registry.rs`: a check that reads its
//! sources of truth straight, and whose failure names the offender).
//!
//! The inventory is DATA, not prose: the `INVENTORY` table below is one
//! row per row-list surface — the 13 `PickerKind`s, the isearch list,
//! the results view, the buffer list, and the declared-DIFFER set (tree,
//! magit status, log, blame, home, transient menu, commit diff/editor,
//! file view) — and
//! `narrowing_inventory_declares_a_verdict_for_every_row_list_surface`
//! asserts, per surface:
//!
//! 1. **a verdict exists, and the table matches the sources of truth**:
//!    every `PickerKind` variant (read from the production enumeration
//!    `picker_kinds()`, never a hand-typed list) owns a row, every
//!    `ViewId` variant (the exhaustive `view_id_row` match + the
//!    `ALL_VIEWS` array — a new variant reddens BOTH, and the match
//!    names it) owns a row, and no row names a surface that does not
//!    exist. A new surface owning row-list state with no declared
//!    verdict fails the test AND names the surface (the
//!    `type_globs_covers_all_registry_languages` failure-message rule).
//! 2. **for every narrows=true surface, its recompute path IS what the
//!    verdict claims** (the observable form: a discriminating query
//!    produces exactly the ordering the claimed mechanism produces —
//!    the 018-01 Pin C shape generalized). Picker kinds, results, and
//!    buffer list route through the shared core; isearch's FIRST
//!    dimension does NOT — see below (its second dimension DOES, since
//!    U-E13 landed).
//! 3. **every DIFFER verdict that is a *named follow-up* (not a silent
//!    omission) still carries its doc marker**: magit status → U-E10,
//!    log → U-E11, tree → U-E12, isearch's second query dimension →
//!    U-E13 — one greppable paragraph each in
//!    `docs/ux-testing-plan.md` (§ U-E · Search & references). Comment
//!    → doc, never comment → memory.
//!
//! **Why isearch's FIRST dimension is `narrows=true, mechanism=own
//! (literal search)` — NOT a shared-core path.** The plan's success
//! criterion was CORRECTED (2026-09-28, after 018-02 measurably
//! disproved the original "everything recompute through the core"
//! reading): `narrow` is a nucleo score-and-REORDER over display
//! strings, and (i) isearch rows are a literal byte-search result in
//! which SEVERAL rows can share one line's text, so scoring the line
//! text cannot recover row identity, and (ii) the match order IS the
//! search (forward/backward from the pre-search point) — a re-rank
//! would corrupt it. Isearch keeps `find_all_matches`
//! (`src/app/store/search.rs`); the list is narrowed BY the search
//! itself — every keystroke re-runs the search, and the list is its
//! result set in search order. It takes the seam's *shape*
//! (source-ordered rows + one selection index + a display projection
//! per row), not the seam's path for this dimension. Asserting a
//! shared-core path for the first dimension would be exactly the false
//! assurance this repo has been bitten by — a pin reading a copy of a
//! fact. `check_isearch_narrows_by_the_search_itself` self-verifies
//! the fixture: it asserts the shared core WOULD rank the fixture's
//! line texts differently, so the literal-order assertion
//! discriminates instead of passing silently.
//!
//! **The second dimension (U-E13, landed): a nucleo filter INSIDE the
//! list.** Once the literal search has produced its match rows, `C-o`
//! arms a second, optional query — a `NarrowSession` run FilterOnly
//! through the shared core over the rows' line-text projection (the
//! display projection 018-02 built exactly for this): the core scores
//! and filters, the surface keeps SOURCE order (match order IS the
//! search — the first dimension's pinned contract). This dimension
//! USES the seam; the first does NOT. Both are checked:
//! `check_isearch_narrows_by_the_search_itself` (dim 1, own) and
//! `check_isearch_second_dimension_is_the_shared_core` (dim 2, core).
//!
//! **Mutation evidence (this issue's verification — measured on this
//! tree, each captured red, then restored and re-verified green):**
//! - dropping the `Palette` row from `INVENTORY` → the cross-check
//!   reddens, naming `PickerKind Palette` ("owns row-list state … but
//!   declares NO verdict"); restore → green.
//! - replacing the results recompute's shared-core call
//!   (`search_narrow_recompute`) with a contains-filter scratch mutation
//!   → the cross-check reddens, naming `search results` ("the narrowed
//!   hit indices ([]) are not the shared core's survivor set") — the
//!   query `l t` is a subsequence, not a substring, of the lib.rs
//!   display, so the mutation's empty set diverges from the core's;
//!   restore → green.
//! - re-ranking the isearch rows (a `sort_by_key(line_text)` scratch
//!   mutation of `isearch_derive_rows`) → the cross-check reddens,
//!   naming `isearch` ("the rows must be in LITERAL SEARCH order");
//!   restore → green.
//!
//! The picker's routing is independently pinned by Pin C in
//! `tests::picker` and the isearch order by the `isearch_list_rows_*`
//! pins in `tests::search`; this file is the plan-level combination
//! (PLAN § "two green lanes can still break each other").

use super::*;
use crate::app::store::narrowing::narrow;
use nucleo_matcher::{Config, Matcher};
use std::collections::BTreeSet;

/// Which mechanism re-derives a narrows=true surface's rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mechanism {
    /// Re-derives its rows through the ONE shared core
    /// (`narrowing::narrow` + the surface's session/recompute): the
    /// picker's 13 kinds (Reorder), the results view and the buffer list
    /// (FilterOnly — the core ranks, the surface keeps source order).
    SharedCore,
    /// isearch's TWO-dimension state (U-E13 landed): the FIRST dimension
    /// is the measured exception (PLAN §6, CORRECTED 2026-09-28) — the
    /// list is the result set of its own literal byte search
    /// (`find_all_matches`), in search order, with no nucleo scoring; see
    /// the module doc for why a shared-core path is impossible there,
    /// not merely declined. The SECOND (optional, `C-o`-armed) filters
    /// those matches through the shared core, FilterOnly over the rows'
    /// line-text projection (source order preserved — match order IS the
    /// search).
    OwnLiteralAndSharedCoreFilter,
}

/// One row of the inventory: a surface, the plan's verdict for it, and
/// why (the honest reason from PLAN §4; the follow-up's doc marker when
/// the deferral is a named candidate plan, U-E10..U-E13 in
/// `docs/ux-testing-plan.md`).
#[derive(Debug)]
struct Row {
    id: &'static str,
    /// The plan's verdict (PLAN §4 + the corrected success criterion).
    /// For the picker kinds the id is the `PickerKind` variant's Debug
    /// name; the cross-check reads the variants from `picker_kinds()`.
    narrows: bool,
    /// narrows=true rows only: which mechanism the recompute path uses.
    mechanism: Option<Mechanism>,
    /// The stated reason for the verdict — never blank; a differ row
    /// without one is a silent omission, the failure class this plan's
    /// §4 exists to forbid.
    reason: &'static str,
    /// The named follow-up's doc marker (docs/ux-testing-plan.md) when
    /// the surface's deferral is a candidate plan rather than a closed
    /// decision.
    follow_up: Option<&'static str>,
}

/// The plan's verdicts as data. 13 picker kinds + isearch + results +
/// buffer list narrows=true; 9 declared-DIFFER surfaces (8 from PLAN §4
/// + the file view, out of scope by construction: code, not a list).
const INVENTORY: &[Row] = &[
    // — narrows=true, mechanism=shared core: the picker's 13 kinds. The
    // picker is the seam's SOURCE (PLAN §4); 018-01 migrated all 13
    // kinds to the session byte-for-byte.
    Row { id: "Palette", narrows: true, mechanism: Some(Mechanism::SharedCore), reason: "picker kind; recompute routes through the shared core (018-01, Pin C)", follow_up: None },
    Row { id: "FindFile", narrows: true, mechanism: Some(Mechanism::SharedCore), reason: "picker kind; recompute routes through the shared core (018-01, Pin C)", follow_up: None },
    Row { id: "RecentFiles", narrows: true, mechanism: Some(Mechanism::SharedCore), reason: "picker kind; recompute routes through the shared core (018-01, Pin C)", follow_up: None },
    Row { id: "Buffers", narrows: true, mechanism: Some(Mechanism::SharedCore), reason: "picker kind; recompute routes through the shared core (018-01, Pin C)", follow_up: None },
    Row { id: "KillBuffer", narrows: true, mechanism: Some(Mechanism::SharedCore), reason: "picker kind; recompute routes through the shared core (018-01, Pin C)", follow_up: None },
    Row { id: "Projects", narrows: true, mechanism: Some(Mechanism::SharedCore), reason: "picker kind; recompute routes through the shared core (018-01, Pin C)", follow_up: None },
    Row { id: "Xref", narrows: true, mechanism: Some(Mechanism::SharedCore), reason: "picker kind; recompute routes through the shared core (018-01, Pin C)", follow_up: None },
    Row { id: "Impls", narrows: true, mechanism: Some(Mechanism::SharedCore), reason: "picker kind; recompute routes through the shared core (018-01, Pin C)", follow_up: None },
    Row { id: "Imenu", narrows: true, mechanism: Some(Mechanism::SharedCore), reason: "picker kind; recompute routes through the shared core (018-01, Pin C)", follow_up: None },
    Row { id: "Symbols", narrows: true, mechanism: Some(Mechanism::SharedCore), reason: "picker kind; recompute routes through the shared core (018-01, Pin C)", follow_up: None },
    Row { id: "Branch", narrows: true, mechanism: Some(Mechanism::SharedCore), reason: "picker kind; recompute routes through the shared core (018-01, Pin C)", follow_up: None },
    Row { id: "Stash", narrows: true, mechanism: Some(Mechanism::SharedCore), reason: "picker kind; recompute routes through the shared core (018-01, Pin C)", follow_up: None },
    Row { id: "Annotations", narrows: true, mechanism: Some(Mechanism::SharedCore), reason: "picker kind; recompute routes through the shared core (018-01, Pin C)", follow_up: None },
    // — narrows=true, mechanism=OWN (first dim) + SHARED CORE (second
    // dim): isearch. Two query dimensions — see the module doc for the
    // why of each; do not "unify" the first dimension back onto the
    // shared core without re-running the mismatch that corrected the
    // plan's criterion.
    Row {
        id: "isearch",
        narrows: true,
        mechanism: Some(Mechanism::OwnLiteralAndSharedCoreFilter),
        reason: "TWO query dimensions. FIRST (mechanism=own, PLAN §6 criterion CORRECTED 2026-09-28; PLAN §3): every keystroke re-runs find_all_matches and the list is its result set in search order — scoring cannot recover row identity (several rows share one line's text) and re-ordering would corrupt the search. SECOND (U-E13, landed): C-o arms a nucleo filter INSIDE that list — a NarrowSession run FilterOnly through the shared core over the rows' line-text projection; the literal match set stays the source, the filter's set re-derives with the search, and C-g is layered (clear the filter, then cancel)",
        follow_up: None,
    },
    // — narrows=true, mechanism=shared core (FilterOnly).
    Row { id: "search results", narrows: true, mechanism: Some(Mechanism::SharedCore), reason: "018-03: FilterOnly projection at view time through the shared core over the hits' display projection; the canonical hits/rows stay untouched (streaming + generation guard)", follow_up: None },
    Row { id: "buffer list", narrows: true, mechanism: Some(Mechanism::SharedCore), reason: "018-04: FilterOnly over the ONE shared display source (the Buffers/KillBuffer picker scores the same strings, marker slot included); MRU order never re-ranked", follow_up: None },
    // — the declared-DIFFER set (PLAN §4: each carries its honest
    // reason; the four with a follow_up are named candidate plans, not
    // silent omissions — each is one paragraph in
    // docs/ux-testing-plan.md, cross-checked below).
    Row { id: "tree", narrows: false, mechanism: None, reason: "hierarchical (filter-children-keep-parents) AND the one renderer-windowed surface (PLAN §1 row 4) — re-home the windowing to the store first; its actual job (find a file fast) is already the narrowing FindFile picker", follow_up: Some("U-E12") },
    Row { id: "magit status", narrows: false, mechanism: None, reason: "diff payload: a score-reorder destroys the diff, and even filter-only must keep context lines glued to their hunk headers; the right primitive is magit's own section narrow (PLAN §2.3-2)", follow_up: Some("U-E10") },
    Row { id: "log", narrows: false, mechanism: None, reason: "server-paged (git log range fetch): a client-side filter of one page hides commits and breaks n/p paging; real log narrow is a `git log` query change (PLAN §2.3-4)", follow_up: Some("U-E11") },
    Row { id: "blame", narrows: false, mechanism: None, reason: "read-only aligned lines; no string dimension the user would type (filter by author/commit is a git query, not a string filter)", follow_up: None },
    Row { id: "home", narrows: false, mechanism: None, reason: "a help screen: no cursor, no selection (HOME_BINDINGS deliberately empty)", follow_up: None },
    Row { id: "transient menu", narrows: false, mechanism: None, reason: "prefix-key navigation IS the interaction; a query prompt would shadow the very keys it would list", follow_up: None },
    Row { id: "commit diff", narrows: false, mechanism: None, reason: "diff payload, the same obstacle as magit status", follow_up: None },
    Row { id: "commit editor", narrows: false, mechanism: None, reason: "a text buffer, not a list", follow_up: None },
    Row { id: "file view", narrows: false, mechanism: None, reason: "code, not a list — out of scope by construction (PLAN §1)", follow_up: None },
];

/// Every `ViewId`, in declaration order — the single enumeration this
/// file iterates. KEEP IT IN LOCKSTEP with the `ViewId` enum (the same
/// reviewed-lockstep contract as `picker_kinds()` in `mod.rs` — pure
/// Rust cannot enumerate a plain enum's variants at runtime): a variant
/// added to the enum but not here is a COMPILE error (the array's type
/// is `[ViewId; 9]`), and the exhaustive `view_id_row` match names it in
/// its own error.
const ALL_VIEWS: [ViewId; 9] = [
    ViewId::Home,
    ViewId::Buffer,
    ViewId::BufferList,
    ViewId::MagitStatus,
    ViewId::Search,
    ViewId::Log,
    ViewId::Blame,
    ViewId::CommitDiff,
    ViewId::CommitEditor,
];

/// The inventory id of one `ViewId` — the ONLY exhaustive match in this
/// file, so a new `ViewId` variant cannot be added without an arm (the
/// compiler names the variant), and without a verdict row in
/// `INVENTORY` (the lookup below names it).
fn view_id_row(view: ViewId) -> &'static str {
    match view {
        ViewId::Home => "home",
        // The file view: PLAN §1's "out of scope by construction" row.
        ViewId::Buffer => "file view",
        ViewId::BufferList => "buffer list",
        ViewId::MagitStatus => "magit status",
        ViewId::Search => "search results",
        ViewId::Log => "log",
        ViewId::Blame => "blame",
        ViewId::CommitDiff => "commit diff",
        ViewId::CommitEditor => "commit editor",
    }
}

fn row_by_id(id: &str) -> &'static Row {
    INVENTORY
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("no inventory row for surface `{id}` in INVENTORY (src/app/store/tests/narrowing.rs) — add its declared verdict"))
}

/// THE cross-check (plan 018 issue 05's primary deliverable): one test
/// over every row-list surface — verdict exists, table matches the
/// sources of truth, and every narrows=true surface's recompute path is
/// the mechanism its verdict claims. A new surface with no declared
/// verdict reddens this, naming the surface.
#[test]
fn narrowing_inventory_declares_a_verdict_for_every_row_list_surface() {
    // — Data integrity: the table cannot contradict itself.
    let ids: BTreeSet<&str> = INVENTORY.iter().map(|r| r.id).collect();
    assert_eq!(
        ids.len(),
        INVENTORY.len(),
        "INVENTORY carries a duplicate surface row: {ids:?}"
    );
    for row in INVENTORY {
        assert_eq!(
            row.narrows,
            row.mechanism.is_some(),
            "row `{}`: narrows and mechanism must agree — a narrows=true row without a mechanism is an undeclared surface",
            row.id
        );
        assert!(
            !row.reason.is_empty(),
            "row `{}`: a verdict without a stated reason is a silent omission (PLAN §4)",
            row.id
        );
    }

    // — The 13 `PickerKind`s, read from the PRODUCTION enumeration
    // (`picker_kinds`, kept in lockstep with the enum + `candidates_for`
    // arms), never a hand-typed list. Every variant owns a verdict row;
    // every picker row in the table names a real variant (drift in
    // either direction reddens, naming the kind).
    let kind_ids: BTreeSet<String> =
        picker_kinds().iter().map(|k| format!("{k:?}")).collect();
    assert_eq!(
        kind_ids.len(),
        13,
        "the plan's inventory is 13 picker kinds; picker_kinds() drifted to {} — re-derive the plan's count before touching this",
        kind_ids.len()
    );
    for kind in picker_kinds() {
        let id = format!("{kind:?}");
        let row = INVENTORY
            .iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!(
                "PickerKind {kind:?} owns row-list state (a picker kind narrows through the shared core — candidates_for + the 018-01 session) but declares NO verdict in the 018-05 inventory (INVENTORY in src/app/store/tests/narrowing.rs). Add its row (narrows=true, mechanism=shared core), or change the verdict in PLAN 018 §4 and say so — an undeclared surface must not exist."
            ));
        assert!(
            row.narrows && matches!(row.mechanism, Some(Mechanism::SharedCore)),
            "PickerKind {kind:?}: the plan's verdict is narrows=true via the SHARED core (PLAN §4: the picker is the seam's source); a different verdict needs a PLAN §4 change, not a silent one"
        );
    }
    let table_kind_rows: BTreeSet<String> = INVENTORY
        .iter()
        .filter(|r| kind_ids.contains(r.id))
        .map(|r| r.id.to_string())
        .collect();
    assert_eq!(
        table_kind_rows,
        kind_ids,
        "the inventory's picker rows drifted from picker_kinds(): only-in-table {:?}, missing-from-table {:?} — name the kind and declare its verdict",
        table_kind_rows.difference(&kind_ids).collect::<Vec<_>>(),
        kind_ids.difference(&table_kind_rows).collect::<Vec<_>>()
    );

    // — The 9 `ViewId`s (via the reviewed lockstep array + the
    // exhaustive match above; a new variant fails to compile, naming
    // itself). Every view owns a verdict row; the narrows=true views
    // must be exactly the plan's two (results, buffer list).
    for view in ALL_VIEWS {
        let id = view_id_row(view);
        let row = row_by_id(id);
        match view {
            ViewId::Search | ViewId::BufferList => assert!(
                row.narrows && matches!(row.mechanism, Some(Mechanism::SharedCore)),
                "ViewId {view:?} (`{id}`): the plan's verdict is narrows=true via the shared core (018-03 / 018-04)"
            ),
            _ => assert!(
                !row.narrows && row.mechanism.is_none(),
                "ViewId {view:?} (`{id}`): the plan's verdict is DIFFER (PLAN §4) — narrows in INVENTORY means the plan table was edited without a re-derivation"
            ),
        }
    }

    // — The declared count, stated: 13 kinds + isearch + results +
    // buffer list + 9 declared-DIFFER surfaces = 25 rows.
    assert_eq!(
        INVENTORY.len(),
        25,
        "the inventory is 26 surfaces (13 picker kinds, isearch, search results, buffer list, 9 declared-DIFFER); it reads {} — a row was added or dropped without a verdict re-derivation",
        INVENTORY.len()
    );

    // — Per surface: the narrows=true recompute path IS the mechanism
    // the verdict claims (the observable form — a discriminating query
    // produces exactly that mechanism's ordering). One rich store
    // serves all 13 picker kinds (Pin C's fixture, shared).
    let mut empty_kinds = Vec::new();
    let mut rich = super::picker::rich_picker_store();
    for row in INVENTORY {
        if !row.narrows {
            continue;
        }
        match row.id {
            "isearch" => {
                check_isearch_narrows_by_the_search_itself();
                check_isearch_second_dimension_is_the_shared_core();
            }
            "search results" => check_results_recompute_is_the_shared_core(),
            "buffer list" => check_buffer_list_recompute_is_the_shared_core(),
            id if kind_ids.contains(id) => {
                let kind = *picker_kinds()
                    .iter()
                    .find(|k| format!("{k:?}") == *id)
                    .expect("the loop above proved this id is a real kind");
                check_picker_kind_routes_through_the_shared_core(&mut rich, kind, &mut empty_kinds);
            }
            other => panic!("narrows=true row `{other}` has no recompute-path check in this test — a verdict that is not checked is prose"),
        }
    }
    assert!(
        empty_kinds.is_empty(),
        "rich_picker_store gave no candidates for {empty_kinds:?} — the shared-core check is vacuous for those kinds; populate them"
    );
}

/// A picker kind's recompute path IS the shared core: the store's own
/// query path (`open_picker` + `picker_query_char` — `candidates_for`
/// re-derives, the session recomputes) must produce exactly
/// `narrowing::narrow`'s ordering over the same `candidates_for(kind)`
/// displays and the same matcher instance (`picker_kind_uses_file_matcher`
/// — the shared predicate, so both sides read the same source). The
/// 018-01 Pin C shape, generalized to the inventory's per-surface assert.
fn check_picker_kind_routes_through_the_shared_core(
    s: &mut AppStore,
    kind: PickerKind,
    empty_kinds: &mut Vec<PickerKind>,
) {
    let candidates = s.candidates_for(kind);
    if candidates.is_empty() {
        empty_kinds.push(kind);
    }
    s.open_picker(kind, "018-05 inventory: ", candidates.clone());
    // 'a' is not a kind's verb char (Stash `x`, Annotations `d`).
    s.picker_query_char('a');
    let query = s.picker_query().to_string();
    let uses_file = picker_kind_uses_file_matcher(kind);
    let displays: Vec<&str> = candidates.iter().map(|c| c.display.as_str()).collect();
    let expected = {
        let m = if uses_file { &mut s.file_matcher } else { &mut s.matcher };
        narrow(&query, &displays, m)
    };
    let expected_names: Vec<&str> =
        expected.iter().map(|(i, _)| candidates[*i].name.as_str()).collect();
    let actual_names: Vec<&str> =
        s.picker_filtered().iter().map(|(c, _)| c.name.as_str()).collect();
    let prefix = 6.min(actual_names.len()).min(expected_names.len());
    assert_eq!(
        actual_names, expected_names,
        "surface `picker kind {kind:?}`: the recompute path diverges from the shared core — its filtered rows ({:?}…) are not narrowing::narrow's ordering over the same candidates_for({kind:?}) + matcher ({:?}…) — a divergence means this kind bypassed the seam",
        &actual_names[..prefix.max(1).min(actual_names.len())],
        &expected_names[..prefix.max(1).min(expected_names.len())]
    );
}

/// The results view's recompute path IS the shared core: typing a
/// discriminating query on the view's own prompt (the guard path —
/// `key_event`, not a field poke) must leave exactly the core's
/// survivor set, in source order (the surface's FilterOnly policy —
/// the hits' `(path, line, col)` order). The query "l t" is a
/// SUBSEQUENCE of exactly one display — `src/lib.rs:1 pub fn target()
/// {}` (`l` … space … `t`) — and a contiguous substring of NONE:
/// a hand-rolled contains-filter recompute keeps zero hits, the core
/// keeps hit 0. (It must avoid n/p/g/q — the view's own decision keys,
/// which the prompt guard falls through to the keymap.)
fn check_results_recompute_is_the_shared_core() {
    let (_dir, mut store) = search_project();
    let mut rx = store.search_rx().unwrap();
    store.start_project_search("target".into());
    drain_search_finished(&mut store, &mut rx);
    assert_eq!(
        store.search.hits.len(),
        4,
        "the fixture must hold 4 hits (1 lib.rs + 3 main.rs) — a different count invalidates the discriminating query"
    );
    for c in "l t".chars() {
        store.key_event(Key::char(c));
    }
    assert_eq!(
        store.search_narrow_query(),
        "l t",
        "the typed chars must reach the results narrow query (the prompt guard)"
    );
    // The display projection is the production one (`hit_display`,
    // pub(crate) for this read): never a copied projection string.
    let displays: Vec<String> =
        store.search.hits.iter().map(AppStore::hit_display).collect();
    let refs: Vec<&str> = displays.iter().map(|d| d.as_str()).collect();
    let mut expected = narrow("l t", &refs, &mut store.matcher);
    expected.sort_by_key(|&(i, _)| i); // FilterOnly: source order (018-03)
    assert!(!expected.is_empty(), "non-vacuous: `l t` must keep >=1 hit through the core");
    let actual: Vec<(usize, u32)> = store.search.narrow.filtered.clone();
    assert_eq!(
        actual,
        expected,
        "surface `search results`: the narrowed hit indices ({actual:?}) are not the shared core's survivor set ({expected:?}, source order) over the hits' own display projection — a divergence means the recompute bypassed narrowing::narrow"
    );
}

/// The buffer list's recompute path IS the shared core: typing a
/// discriminating query on the list's own prompt must leave exactly the
/// core's survivors over the list's displays — the SAME `buffer_row_display`
/// strings the Buffers/KillBuffer picker scores (018-04's one shared
/// display source, marker slot included), in MRU source order
/// (FilterOnly). "om rs" is a SUBSEQUENCE of exactly one display —
/// `* src/omega.rs` (`o m` … space … `r s`) — and a contiguous
/// substring of none: a contains-filter keeps zero, the core keeps
/// omega (source index 0).
fn check_buffer_list_recompute_is_the_shared_core() {
    let dir = tempfile::tempdir().unwrap();
    project_with_files(dir.path());
    let mut store = store(dir.path());
    store.open_path("src/main.rs");
    store.open_path("src/lib.rs");
    std::fs::write(dir.path().join("src/alpha.rs"), "// a\n").unwrap();
    store.open_path("src/alpha.rs");
    std::fs::write(dir.path().join("src/omega.rs"), "// w\n").unwrap();
    store.open_path("src/omega.rs");
    // MRU (newest first): omega (current), alpha, main, lib.
    store.dispatch("list-buffers", None).unwrap();
    assert_eq!(store.top_view(), ViewId::BufferList);
    for c in "om rs".chars() {
        store.key_event(Key::char(c));
    }
    assert_eq!(store.buffer_list_query(), "om rs");
    let keys: Vec<String> = store
        .buffers
        .list()
        .into_iter()
        .map(|(k, _)| k.to_string())
        .collect();
    assert_eq!(keys.len(), 4, "the fixture must hold 4 buffers");
    let displays: Vec<String> = keys
        .iter()
        .map(|key| store.buffer_row_display(key))
        .collect();
    let refs: Vec<&str> = displays.iter().map(|d| d.as_str()).collect();
    let mut expected = narrow("om rs", &refs, &mut store.matcher);
    expected.sort_by_key(|&(i, _)| i); // FilterOnly: MRU source order (018-04)
    assert!(!expected.is_empty(), "non-vacuous: `om rs` must keep >=1 buffer through the core");
    assert!(
        expected.len() < keys.len(),
        "discriminating: the query must NARROW (fewer survivors than the full list)"
    );
    let (rows, _scroll, total, _sel_row) = store.buffer_list_view_info();
    assert_eq!(
        total,
        expected.len(),
        "surface `buffer list`: the narrowed set size ({total}) is not the shared core's survivor count ({:?}) — a divergence means the recompute bypassed narrowing::narrow",
        expected.iter().map(|&(i, _)| i).collect::<Vec<_>>()
    );
    for (i, &(src_idx, _)) in expected.iter().enumerate() {
        let key = &keys[src_idx];
        assert_eq!(
            rows[i].name,
            store.buffer_row_display(key),
            "surface `buffer list`: narrowed row {i} must be the core's source row {src_idx} (MRU order, the shared display source)"
        );
    }
}

/// Isearch: narrows=true, mechanism=OWN (literal search) — see the
/// module doc for the why (the plan's corrected success criterion). The
/// list is the search's result set: every keystroke re-runs
/// `find_all_matches` and the rows are its matches, in SEARCH order,
/// with no nucleo scoring. The fixture is self-verifying: the shared
/// core WOULD rank its line texts differently (it scores the word-start
/// `ab x` at 62 above the mid-word `xab xab` at 36 — measured, and the
/// non-vacuous assert below re-measures it on every run), so a recompute
/// that routed through the core reddens the literal-order assert below —
/// and if the core ever stops doing that (e.g. a tie, where the stable
/// sort keeps source order and masquerades as the literal order), the
/// non-vacuous assert says the fixture stopped discriminating, instead of
/// passing silently.
fn check_isearch_narrows_by_the_search_itself() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
    // "cabc" (one literal "ab" match, MID-word) over "ab x" (one match,
    // word-start): 2 literal matches. The scores differ by measure
    // (62 vs 36 — the non-vacuous assert below re-measures them on
    // every run), so the core's order (line 1 first) is the REVERSE of
    // the literal search order (line 0 first). A tie would not
    // discriminate (the stable sort keeps source order and masquerades
    // as the literal order) — that is exactly why the fixture is
    // self-verified instead of asserted once in a comment.
    std::fs::write(dir.path().join("src/t.rs"), "cabc\nab x\n").unwrap();
    let base = tempfile::tempdir().unwrap();
    let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
    s.open_path("src/t.rs");

    // Non-vacuous, self-verified: on the rows' own line texts the shared
    // core ranks the "ab x" row (index 2) FIRST (62 > 36). If it ever
    // did not — a tie, where the stable sort would keep source order and
    // pass the literal-order assert by accident — this fixture cannot
    // tell own-mechanism from a shared-core path, and the test must say
    // so, not pass.
    let line_texts: Vec<&str> = vec!["cabc", "ab x"];
    let mut m = Matcher::new(Config::DEFAULT);
    let core_order = narrow("ab", &line_texts, &mut m);
    assert_eq!(
        core_order.first().map(|(i, _)| *i),
        Some(1),
        "the fixture must discriminate: the shared core must rank the `ab x` row (word-start, 62) above the `cabc` row (mid-word, 36) — otherwise a stable sort keeps source order and this check cannot tell isearch's literal-search order from a shared-core re-rank (re-pick the fixture; ties do not discriminate)"
    );

    s.isearch_start(IsearchDirection::Forward);
    s.isearch_query_char('a');
    s.isearch_query_char('b');
    // narrows=true: the list is the search's result set — 2 literal
    // matches, SEARCH order: line 0's first (the search runs forward
    // from the pre-search point at (0,0)), then line 1's — the REVERSE
    // of the core's score order (line 1 first).
    let (rows, _current, active) = s.isearch_list();
    assert!(active, "isearch must be active with a non-empty result set");
    assert_eq!(
        rows.len(),
        2,
        "one row per literal match — a re-rank, a dedup, or a missing row reddens this"
    );
    let order: Vec<(usize, usize)> =
        rows.iter().map(|r| (r.line_no, r.match_col)).collect();
    assert_eq!(
        order,
        vec![(0, 1), (1, 0)],
        "surface `isearch`: the rows must be in LITERAL SEARCH order (match order IS the search; the shared core would put the line-1 row FIRST here) — {order:?}"
    );
    // And it narrows as the query grows: "ab x" keeps exactly 1 row
    // (line 1 — the only line holding that literal; line 0's "cabc"
    // holds "ab" but not "ab x"), search order.
    s.isearch_query_char(' ');
    s.isearch_query_char('x');
    let (rows2, _, _) = s.isearch_list();
    let order2: Vec<(usize, usize)> =
        rows2.iter().map(|r| (r.line_no, r.match_col)).collect();
    assert_eq!(
        order2,
        vec![(1, 0)],
        "extending the query must shrink the list to the surviving literal matches, search order — got {order2:?}"
    );
}

/// Isearch's second dimension (U-E13, landed): a C-o-armed filter
/// routes through the ONE shared core. Arming the filter on the guard's
/// own path (`key_event`, not a field poke) and typing a discriminating
/// query must leave exactly the core's survivor set over the match
/// rows' line-text projection — the production projection
/// (`IsearchState.rows`, 1:1 with `matches`), never a copied display
/// string. The query "g m" is a SUBSEQUENCE of exactly one row —
/// `foo gamma` (g@4 … m@6) — and a contiguous substring of none:
/// a hand-rolled contains-filter recompute keeps zero rows, the core
/// keeps row 1.
fn check_isearch_second_dimension_is_the_shared_core() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
    // Three literal "foo" matches (lines 0, 1, 3); the row on line 2 is
    // a NON-match ("bar delta") and must never enter the projection.
    std::fs::write(
        dir.path().join("src/t.rs"),
        "foo alpha\nfoo gamma\nbar delta\nfoo epsilon\n",
    )
    .unwrap();
    let base = tempfile::tempdir().unwrap();
    let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
    s.open_path("src/t.rs");

    s.isearch_start(IsearchDirection::Forward);
    s.isearch_query_char('f');
    s.isearch_query_char('o');
    s.isearch_query_char('o');
    let displays: Vec<String> = s
        .isearch
        .rows
        .iter()
        .map(|r| r.line_text.clone())
        .collect();
    assert_eq!(
        displays.iter().map(String::as_str).collect::<Vec<_>>(),
        vec!["foo alpha", "foo gamma", "foo epsilon"],
        "the fixture must hold 3 match rows (one per literal 'foo') — the bar line is not a match"
    );

    // Arm the second dimension on the guard's own path and type the
    // discriminating query (the pre-U-E13 tree swallowed C-o and
    // extended the literal query — the asserts below redden).
    s.key_event(crate::app::keymap::Key::ctrl_char('o'));
    for c in "g m".chars() {
        s.key_event(crate::app::keymap::Key::char(c));
    }
    let refs: Vec<&str> = displays.iter().map(|d| d.as_str()).collect();
    let mut expected = narrow("g m", &refs, &mut s.matcher);
    expected.sort_by_key(|&(i, _)| i); // FilterOnly: source order (U-E13)
    assert!(!expected.is_empty(), "non-vacuous: `g m` must keep >=1 row through the core");
    assert!(
        expected.len() < displays.len(),
        "discriminating: the query must NARROW (fewer survivors than the full set)"
    );
    let (rows, _selected, active) = s.isearch_list();
    assert!(active, "isearch stays active with the filter armed");
    assert_eq!(
        rows.len(),
        expected.len(),
        "surface `isearch` (second dimension): the filtered row count ({}) is not the shared core's survivor count ({:?}) — a divergence means the filter bypassed narrowing::narrow",
        rows.len(),
        expected.iter().map(|&(i, _)| i).collect::<Vec<_>>()
    );
    for (i, &(src_idx, _)) in expected.iter().enumerate() {
        assert_eq!(
            rows[i].line_no,
            s.isearch.rows[src_idx].line_no,
            "surface `isearch` (second dimension): filtered row {i} must be the core's source row {src_idx} (search order kept)"
        );
    }
}

/// The DIFFER verdicts are NAMED FOLLOW-UPS, not silent omissions
/// (PLAN §4): every row that declares a follow-up marker must still
/// carry it in `docs/ux-testing-plan.md` — the decision is greppable
/// where the plan says it lives (comment → doc, not comment → memory).
/// Reading the doc straight (not a copy of its contents) is the
/// `0829ddd` / `src/gate_registry.rs` rule.
#[test]
fn declared_differ_surfaces_carry_named_follow_ups_in_the_doc() {
    let doc = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("docs")
            .join("ux-testing-plan.md"),
    )
    .expect("docs/ux-testing-plan.md must exist — it carries the 018 named follow-ups");
    let mut markers = Vec::new();
    for row in INVENTORY.iter().filter(|r| r.follow_up.is_some()) {
        let marker = row.follow_up.expect("filtered on Some");
        markers.push(marker);
        assert!(
            doc.contains(marker),
            "surface `{}` declares follow-up `{marker}` (one named paragraph in docs/ux-testing-plan.md, PLAN §4 / 018-05) but the doc no longer carries that marker — the deferral is about to become a silent omission; restore the paragraph or re-declare the row",
            row.id
        );
    }
    assert_eq!(
        markers.len(),
        3,
        "the plan names exactly three OPEN follow-ups (magit status U-E10, log U-E11, tree U-E12; U-E13 — isearch's second dimension — LANDED, its doc checkbox flipped); the table declares {markers:?}"
    );
}
