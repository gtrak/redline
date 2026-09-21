//! Shared helpers for the golden-corpus suites.
//!
//! The standard integration-test sharing pattern: each `golden_*.rs` file is
//! its own test binary that declares `mod common;` and calls `common::…`.
//!
//! Because each test file is a separate binary, the `GOLDEN_BLESS` stop-gate
//! statics below are per-suite even though their definitions live here — a
//! bless run of `golden_go` can never affect `golden_python`'s bookkeeping.
//!
//! What lives here is the set of helpers that is genuinely identical (or
//! identical up to the placeholder roots a suite substitutes) across the
//! go / python / rust suites:
//! - `sub_path` / `normalize_roots` (path → placeholder substitution; the
//!   suites differ only in WHICH roots they substitute);
//! - `rel_to` (a path's corpus-relative form, `Option` when it escapes);
//! - `copy_tree` (the small corpus → tempdir copy);
//! - the `GOLDEN_BLESS` stop-gate statics + `assert_bless_stopped` +
//!   `record_bless`, and `bless_handauthored_golden` (the byte-preserving
//!   re-bless for the hand-authored python/rust goldens).
//!
//! What stays per-file (`golden_*.rs`) is genuinely per-language: each
//! suite's `Probe` / `Outcome` / `ExpectedResolved` shims (the fields and
//! the `path_base=stdlib` python-only shape differ), its corpus path, its
//! golden file layout, its probe parser (`parse_golden` collects a header in
//! go and a `path_base` in python), its renderer (`render_golden`), its
//! `check_*` / `capture` logic, and its `#[test]` fns. `golden_js.rs`
//! intentionally does NOT share from here: its probe spec is a `probes.toml`
//! (not `*.golden` files), it normalizes to `<ws>` via a raw `replace` (not
//! `sub_path`'s raw+canonicalized substitution), and its
//! `assert_bless_stopped` is a deliberate whitespace variant — collapsing
//! any of those into this module would change a real difference.
//!
//! `#[allow(dead_code)]`: each binary pulls in the whole module but uses
//! only its own slice (e.g. `golden_go` never calls `copy_tree`), so the
//! per-binary unused subset is intentional, not a warning to fix.
#![allow(dead_code)]

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Replace a path (raw and canonicalized form) with its placeholder.
pub fn sub_path(mut out: String, p: &Path, placeholder: &str) -> String {
    out = out.replacen(p.to_string_lossy().as_ref(), placeholder, usize::MAX);
    if let Ok(canonical) = std::fs::canonicalize(p) {
        out = out.replacen(
            canonical.to_string_lossy().as_ref(),
            placeholder,
            usize::MAX,
        );
    }
    out
}

/// Normalize an actual outcome string by substituting each `(path,
/// placeholder)` pair, in the order given. The suites substitute different
/// roots and pass them in the order the original per-file `normalize` applied
/// them: go `(root, cache)`; python `(root, corpus)`; rust
/// `(root, corpus, home)`.
pub fn normalize_roots(msg: &str, subs: &[(&Path, &str)]) -> String {
    let mut s = msg.to_string();
    for (p, placeholder) in subs {
        s = sub_path(s, p, placeholder);
    }
    s
}

/// `p` relative to `base` (raw or canonicalized base prefix); `None` when
/// `p` escapes `base`.
pub fn rel_to(base: &Path, p: &Path) -> Option<String> {
    if let Ok(r) = p.strip_prefix(base) {
        return Some(r.to_string_lossy().into_owned());
    }
    if let Ok(c) = std::fs::canonicalize(base)
        && let Ok(r) = p.strip_prefix(&c)
    {
        return Some(r.to_string_lossy().into_owned());
    }
    None
}

/// Recursively copy `src` into `dst` (plain fs; the corpora are small).
pub fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

// ── GOLDEN_BLESS stop-gate bookkeeping (011-08 follow-up P2-1) ──────────────
//
// A bless run writes EVERY golden first, then FAILs the run once, so an
// accidental `GOLDEN_BLESS` can never end green. These statics are per
// binary (see the module doc) and are advanced by both the hand-authored
// re-bless below and go's re-rendering `check_golden`.

static BLESSED_RUN: AtomicBool = AtomicBool::new(false);
static BLESSED_CHANGED: AtomicUsize = AtomicUsize::new(0);

/// End-of-test gate: a bless run writes EVERY golden first, then fails
/// exactly once per test (never per file — a per-file panic would abort the
/// loop and leave later goldens unwritten).
pub fn assert_bless_stopped(test_name: &str) {
    if !BLESSED_RUN.load(Ordering::SeqCst) {
        return;
    }
    let changed = BLESSED_CHANGED.load(Ordering::SeqCst);
    panic!(
        "GOLDEN_BLESS run of {test_name} rewrote {changed} golden(s) — \
         run the suite again WITHOUT GOLDEN_BLESS to verify the new goldens pass"
    );
}

/// Record that a bless write happened this run (and whether any golden
/// actually changed). Replaces the two inline static updates the suites
/// used to make at the end of a bless write.
pub fn record_bless(changed: bool) {
    if changed {
        BLESSED_CHANGED.fetch_add(1, Ordering::SeqCst);
    }
    BLESSED_RUN.store(true, Ordering::SeqCst);
}

/// Byte-preserving re-bless for HAND-AUTHORED goldens (python, rust): the
/// checked-in golden IS the authoritative spec, so bless writes it back
/// unchanged (no-op unless `GOLDEN_BLESS` is set) and records the bless so
/// the end-of-test gate fails the run. `changed` is always false here —
/// there is no independent live-derived rendering that could differ from the
/// checked-in file — so this is always a no-change, and a deliberate
/// hand-edit round-trips byte-identically.
pub fn bless_handauthored_golden(corpus_src: &Path, golden: &Path) {
    if std::env::var_os("GOLDEN_BLESS").is_none() {
        return;
    }
    let checkin = corpus_src.join(golden.file_name().unwrap());
    let old = std::fs::read(&checkin)
        .unwrap_or_else(|e| panic!("cannot read golden {}: {e}", checkin.display()));
    let rendered = old.clone();
    let changed = old != rendered;
    std::fs::write(&checkin, &rendered)
        .unwrap_or_else(|e| panic!("cannot bless {}: {e}", checkin.display()));
    eprintln!(
        "GOLDEN_BLESS: {} {} — review `git diff` before committing; \
         this run FAILS so the bless cannot slip through green",
        if changed { "REWROTE" } else { "no change to" },
        checkin.display()
    );
    record_bless(changed);
}
