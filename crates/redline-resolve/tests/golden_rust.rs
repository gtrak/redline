//! Rust golden-corpus suite for the CARGO provider (plan 011, issue 08).
//!
//! The corpus at `tests/corpus/rust/` is a small, genuine cargo workspace
//! (members `gearbox` + `widgets`) plus a vendored sibling crate
//! (`frobnicator/`, OUTSIDE the workspace root so its landings are
//! `external = true`) and a `not_a_crate/` directory for the
//! no-Cargo.toml bail. Each `*.golden` file is BOTH the probe spec
//! (symbol, from_file, scope hint, workspace root) AND the expected
//! outcome, pinned byte-for-byte: a `ResolvedSource` (file / source_root
//! relative to the corpus root, line, external flag) or the exact bail
//! message (the honest degradations are golden too: bare symbol without a
//! scope hint, workspace miss, item-not-found, not-a-cargo-project,
//! unparseable symbol).
//!
//! Run discipline: the provider shells to the REAL cargo (`cargo
//! metadata` on the copied workspace, isolated `CARGO_HOME`) — every
//! probe is a live subprocess leg. The corpus uses path dependencies only
//! (no registry fetch, no network). When the cargo binary is absent the
//! whole suite skips LOUDLY. No PTY is used anywhere.
//!
//! A deliberate provider/seam behavior change is a deliberate golden diff:
//! these goldens are HAND-AUTHORED `*.golden` files — each file is BOTH the
//! probe spec and the expected outcome, pinned byte-for-byte with hand-
//! written header prose — so a re-bless does NOT re-render them (unlike the
//! re-derivable js/go goldens). A bless run writes EVERY golden back
//! UNCHANGED (the exact bytes a green run compares against), eprintlns a
//! no-change line per file, then FAILS the run (011-08 follow-up P2-1: an
//! accidental `GOLDEN_BLESS` can never end green; a deliberate hand-edit
//! round-trips byte-identically):
//! `GOLDEN_BLESS=1 cargo test -p redline-resolve --test golden_rust`.
//!
//! Deterministic probe walk: `*.golden` files under the corpus root in
//! sorted (byte) order, one provider resolve per probe.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use redline_resolve::{CargoProvider, ResolvedSource, SymbolContext, ToolingProvider};

/// The checked-in corpus, relative to the crate manifest dir.
fn corpus_src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/rust")
}

/// Recursively copy `src` into `dst` (plain fs; the corpus is small).
fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
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

/// One parsed probe: the spec AND the expected outcome.
struct Probe {
    name: String,
    symbol: String,
    from_file: String,
    /// The app's scope hint (use-declaration path; empty = no hint).
    scope: Vec<String>,
    /// Probe workspace root, corpus-root-relative. Defaults to
    /// `project` (the workspace); `not_a_crate` is the no-Cargo.toml dir.
    root: String,
    resolved: Option<ExpectedResolved>,
    /// The exact bail message with `{root}` / `{corpus}` / `{home}`
    /// placeholders standing in for the dynamic absolute paths.
    bail: Option<String>,
}

struct ExpectedResolved {
    external: bool,
    /// File relative to the corpus root.
    file: String,
    /// Source root relative to the corpus root.
    source_root: String,
    line: u32,
}

fn parse_golden(path: &Path) -> Probe {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let mut map: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line.split_once('=').unwrap_or_else(|| {
            panic!("bad golden line in {}: {line:?}", path.display())
        });
        map.insert(k.trim().to_string(), v.trim().to_string());
    }
    let need = |key: &str| -> String {
        map.get(key)
            .cloned()
            .unwrap_or_else(|| panic!("probe {} is missing `{key}`", path.display()))
    };
    let symbol = need("symbol");
    let from_file = need("from_file");
    let scope_raw = need("scope");
    let inner = scope_raw.trim_start_matches('[').trim_end_matches(']');
    let scope = if inner.trim().is_empty() {
        Vec::new()
    } else {
        inner
            .split(',')
            .map(|p| p.trim().trim_matches('"').to_string())
            .collect()
    };
    let root = map.get("root").cloned().unwrap_or_else(|| "project".to_string());
    let kind = need("kind");
    let (resolved, bail) = match kind.as_str() {
        "resolved" => (
            Some(ExpectedResolved {
                external: need("external") == "true",
                file: need("file"),
                source_root: need("source_root"),
                line: need("line").parse().unwrap_or_else(|e| {
                    panic!("probe {}: bad line value: {e}", path.display())
                }),
            }),
            None,
        ),
        "bail" => (None, Some(need("bail"))),
        other => panic!("probe {}: unknown kind `{other}`", path.display()),
    };
    Probe {
        name: path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        symbol,
        from_file,
        scope,
        root,
        resolved,
        bail,
    }
}

/// Replace a path (raw and canonicalized form) with its placeholder.
fn sub_path(mut out: String, p: &Path, placeholder: &str) -> String {
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

/// Normalize an actual outcome string: the probe's workspace root becomes
/// `{root}`, the corpus copy root becomes `{corpus}`, the isolated
/// `CARGO_HOME` becomes `{home}`. Everything else must match byte-for-byte.
fn normalize(msg: &str, corpus: &Path, probe_root: &Path, home: &Path) -> String {
    let s = sub_path(msg.to_string(), probe_root, "{root}");
    let s = sub_path(s, corpus, "{corpus}");
    sub_path(s, home, "{home}")
}

/// `p` relative to `corpus` (raw or canonicalized corpus prefix).
fn rel_to_corpus(corpus: &Path, p: &Path) -> Option<String> {
    if let Ok(r) = p.strip_prefix(corpus) {
        return Some(r.to_string_lossy().into_owned());
    }
    if let Ok(c) = std::fs::canonicalize(corpus)
        && let Ok(r) = p.strip_prefix(&c)
    {
        return Some(r.to_string_lossy().into_owned());
    }
    None
}

// ── golden bless bookkeeping (011-08 follow-up P2-1, rust lane) ─────────────
//
// The rust goldens are hand-authored and authoritative: the bytes a green
// run compares against ARE the checked-in file's own bytes. So bless does
// not re-render; it rewrites every golden back UNCHANGED and records the
// bless so the end-of-test gate fails the run. A no-op re-bless is always a
// no-change; a deliberate hand-edit round-trips byte-identically.
static BLESSED_RUN: AtomicBool = AtomicBool::new(false);
static BLESSED_CHANGED: AtomicUsize = AtomicUsize::new(0);

/// End-of-test gate: a bless run writes EVERY golden back first, then fails
/// exactly once per test (never per file — a per-file panic would abort the
/// loop and leave later goldens unwritten). An accidental GOLDEN_BLESS can
/// therefore never end green.
fn assert_bless_stopped(test_name: &str) {
    if !BLESSED_RUN.load(Ordering::SeqCst) {
        return;
    }
    let changed = BLESSED_CHANGED.load(Ordering::SeqCst);
    panic!(
        "GOLDEN_BLESS run of {test_name} rewrote {changed} golden(s) — \
         run the suite again WITHOUT GOLDEN_BLESS to verify the new goldens pass"
    );
}

/// Bless-mode write (no-ops unless `GOLDEN_BLESS` is set): a rust golden is
/// hand-authored, so bless writes the CHECKED-IN golden back with the exact
/// bytes a green run compares against (byte-preserving — writes to the
/// checked-in corpus, not the tempdir copy, so `git diff` reflects it) and
/// eprintlns a no-change line. `changed` is the js/go-compatible bookkeeping
/// (always false here: there is no independent live-derived rendering that
/// could differ from the checked-in file).
fn bless_rust_golden(corpus_src: &Path, tmp_golden: &Path) {
    if std::env::var_os("GOLDEN_BLESS").is_none() {
        return;
    }
    let checkin = corpus_src.join(tmp_golden.file_name().unwrap());
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
    if changed {
        BLESSED_CHANGED.fetch_add(1, Ordering::SeqCst);
    }
    BLESSED_RUN.store(true, Ordering::SeqCst);
}

/// Check an actual `ResolvedSource` against a resolved golden; returns the
/// list of field mismatches (empty = exact match).
fn check_resolved(src: &ResolvedSource, exp: &ExpectedResolved, corpus: &Path) -> Vec<String> {
    let mut bad = Vec::new();
    if src.external != exp.external {
        bad.push(format!("external: expected {}, got {}", exp.external, src.external));
    }
    let file_rel = rel_to_corpus(corpus, &src.file)
        .unwrap_or_else(|| format!("<outside corpus copy: {}>", src.file.display()));
    if file_rel != exp.file {
        bad.push(format!("file: expected {}, got {file_rel}", exp.file));
    }
    let root_rel = rel_to_corpus(corpus, &src.source_root)
        .unwrap_or_else(|| format!("<outside corpus copy: {}>", src.source_root.display()));
    if root_rel != exp.source_root {
        bad.push(format!(
            "source_root: expected {}, got {root_rel}",
            exp.source_root
        ));
    }
    match src.line {
        Some(l) if l != exp.line => bad.push(format!("line: expected {}, got {l}", exp.line)),
        Some(_) => {}
        None => bad.push(format!("line: expected {}, got None", exp.line)),
    }
    bad
}

/// The whole corpus, one pass, deterministic (sorted) probe walk.
#[test]
fn rust_golden_corpus() {
    // Loud skip when the real cargo toolchain is absent.
    let cargo_ok = std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !cargo_ok {
        eprintln!(
            "SKIP (loud): `cargo` binary not available; the rust golden corpus \
             (live `cargo metadata` legs) was NOT run"
        );
        return;
    }

    // Copy the corpus to a tempdir so cargo's own scratch (Cargo.lock,
    // target/ if any) never touches the checked-in files, and so the
    // golden path placeholders have a stable shape to normalize against.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    let corpus_src = corpus_src_dir();
    copy_tree(&corpus_src, &corpus).expect("copy corpus");

    // Isolated CARGO_HOME: the live cargo legs never touch the ambient
    // registry (path dependencies only — nothing can be fetched anyway).
    let home = tempfile::tempdir().expect("tempdir");
    let provider = CargoProvider::new().with_cargo_home(home.path());

    let goldens: Vec<PathBuf> = std::fs::read_dir(&corpus)
        .expect("read corpus dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("golden"))
        .collect();
    let mut goldens = goldens;
    goldens.sort();
    assert!(
        !goldens.is_empty(),
        "no *.golden probes found under {}",
        corpus.display()
    );

    let mut failures: Vec<String> = Vec::new();
    for path in &goldens {
        let probe = parse_golden(path);
        // Bless bookkeeping: a byte-preserving re-bless writes the checked-in
        // golden back unchanged and records the bless (no-op unless
        // GOLDEN_BLESS is set).
        bless_rust_golden(&corpus_src, path);
        // An empty root (none of the goldens use one) would make
        // `Path::join("")` append a trailing slash and skew normalization.
        let probe_root = if probe.root.is_empty() {
            corpus.clone()
        } else {
            corpus.join(&probe.root)
        };
        let ctx = SymbolContext {
            workspace_root: probe_root.clone(),
            symbol: probe.symbol.clone(),
            from_file: probe.from_file.clone().into(),
            scope: probe.scope.clone(),
            language: Some("rust".to_string()),
        };
        match provider.resolve(&ctx) {
            Ok(src) => {
                let exp = match &probe.resolved {
                    Some(e) => e,
                    None => {
                        failures.push(format!(
                            "{} ({}): expected a bail but resolve succeeded: {src:?}",
                            probe.name, probe.symbol
                        ));
                        continue;
                    }
                };
                let bad = check_resolved(&src, exp, &corpus);
                if !bad.is_empty() {
                    failures.push(format!(
                        "{} ({}): resolved-mismatch\n  {}",
                        probe.name,
                        probe.symbol,
                        bad.join("\n  ")
                    ));
                }
            }
            Err(e) => {
                let exp = match &probe.bail {
                    Some(b) => b,
                    None => {
                        failures.push(format!(
                            "{} ({}): expected resolved but provider bailed: {}",
                            probe.name, probe.symbol, e
                        ));
                        continue;
                    }
                };
                let got = normalize(&e.to_string(), &corpus, &probe_root, home.path());
                if got != *exp {
                    failures.push(format!(
                        "{} ({}): bail mismatch\n  expected: {exp}\n  got:      {got}",
                        probe.name, probe.symbol
                    ));
                }
            }
        }
    }

    assert_bless_stopped("rust_golden_corpus");
    assert!(
        failures.is_empty(),
        "{} golden mismatch(es) out of {} probes:\n\n{}",
        failures.len(),
        goldens.len(),
        failures.join("\n\n")
    );
}
