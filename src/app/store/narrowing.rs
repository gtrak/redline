//! The shared list-narrowing seam (plan 018, issue 01).
//!
//! There is exactly ONE narrowing engine in the app: a nucleo score-and-filter
//! over a list of **display strings**. It used to live privately inside the
//! picker overlay (`Picker::recompute`). It now lives here, store-side, as a
//! pure core (`narrow`) plus a per-surface session (`NarrowSession`), so the
//! picker's 13 kinds and the later narrowable surfaces (isearch, results,
//! buffer list — issues 02–04) implement against a proven core instead of
//! copying it.
//!
//! The seam deals in **display strings and source-row indices only**. No
//! byte/char/column conversion enters it (plan 018 §5.8): a surface supplies
//! a display projection (`row -> &str`) and its own `Matcher` instance, and
//! gets back `(index, score)` rankings. It never copies row data and never
//! touches a landing path's units.
//!
//! The picker keeps its own row type (`PickerCandidate`) and verb machinery;
//! it is the seam's first consumer, migrated byte-for-byte (the picker's whole
//! existing test suite is issue 01's acceptance). It is NOT "make every list
//! emit `PickerCandidate`s" (plan 018 §2.2a names the cost both ways).

use nucleo_matcher::{
    Matcher,
    pattern::{CaseMatching, Normalization, Pattern},
    Utf32Str,
};
use std::cmp::Reverse;

/// Score `displays` against `query` and return the survivors as
/// `(source-row index, score)` pairs, best-first (highest score first).
///
/// Semantically identical to the picker's former `recompute`:
/// - an empty `query` returns **every** row, in source order, at `u32::MAX`
///   (no scoring involved — the full list in its own order);
/// - otherwise one `Pattern::parse(query, CaseMatching::Ignore,
///   Normalization::Smart)` per call and ONE `Utf32Str` scratch buffer reused
///   across rows;
/// - `sort_by_key(Reverse(score))` — a **stable** sort, so rows with equal
///   scores keep their source order. That stability is an observable
///   behaviour (the empty-query and equal-score cases read deterministically),
///   not an implementation detail; do not swap it for an unstable sort.
///
/// The result is a list of *indices* into `displays` (not the strings): the
/// seam ranks rows, it does not copy them.
pub fn narrow(query: &str, displays: &[&str], matcher: &mut Matcher) -> Vec<(usize, u32)> {
    if query.is_empty() {
        return (0..displays.len()).map(|i| (i, u32::MAX)).collect();
    }
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut buf = Vec::new();
    let mut scored: Vec<(usize, u32)> = displays
        .iter()
        .enumerate()
        .filter_map(|(i, d)| {
            let haystack = Utf32Str::new(d, &mut buf);
            pattern.score(haystack, matcher).map(|score| (i, score))
        })
        .collect();
    scored.sort_by_key(|item| Reverse(item.1));
    scored
}

/// Per-surface narrowing state: the query, the clamped selection, and the
/// `(source-row index, score)` ranking the core produced for the current
/// query.
///
/// The selection rule is the picker's one rule — after every recompute the
/// selection is clamped into the (possibly shrunken) filtered set:
/// `selected = selected.min(filtered.len().saturating_sub(1))`. Clamping a
/// zero selection is a no-op (empty set → `saturating_sub(1)` yields 0), so a
/// fresh session's `selected == 0` survives both the full and the empty list.
///
/// Per-kind verb keys (Stash `x`, Annotations `d`, …) do NOT live here — they
/// stay in the surface's key handler (plan 018 §2.2: verb keys are
/// surface-specific).
#[derive(Debug, Default)]
pub struct NarrowSession {
    /// The current narrowing query.
    pub query: String,
    /// The selected row of the (filtered) list, clamped after each recompute.
    pub selected: usize,
    /// `(source-row index, score)`, best-first, for the current query.
    pub filtered: Vec<(usize, u32)>,
}

impl NarrowSession {
    /// Re-score `displays` against the session's current `query`, replace the
    /// ranking, and clamp the selection into the new set.
    pub fn recompute(&mut self, displays: &[&str], matcher: &mut Matcher) {
        self.filtered = narrow(&self.query, displays, matcher);
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
    }
}
