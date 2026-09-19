//! Python golden-corpus suite for the PYTHON provider (plan 011, issue 08).
//!
//! The corpus at `tests/corpus/python/` is a small, genuine package-shaped
//! project (`project/`: a `gears` package with `__init__.py`, sibling
//! modules, `main.py` carrying the real use sites — plain import, wildcard,
//! dotted from-entry, aliased import). Each `*.golden` file is BOTH the
//! probe spec (dotted or bare symbol, from_file, scope hint, probe root)
//! AND the expected outcome, pinned byte-for-byte: a `ResolvedSource`
//! (file / source root — corpus-root-relative, or stdlib-root-relative when
//! `path_base=stdlib` — line, external flag) or the exact bail message
//! (the honest degradations are golden too: bare-without-hint, plain-import
//! never-hints-bare, wildcard, dotted-from-entry, item-not-found,
//! offline-refusal).
//!
//! Run discipline: the provider shells to the REAL `python3` (live
//! `find_spec` + `is_stdlib` subprocess legs, one per probe) with the
//! probe root as the working directory — the workspace package resolves
//! because `-c` puts the CWD on `sys.path`. The provider is built
//! `.offline()`: the network `pip install` leg is never touched in the
//! gate, so the missing-module degradation is pinned as an exact
//! offline-refusal bail instead of a network-dependent message. When the
//! `python3` binary is absent the whole suite skips LOUDLY. No PTY is used
//! anywhere.
//!
//! Stdlib honesty: the live stdlib legs (probes 008/009) pin line numbers
//! of the REAL interpreter's stdlib — verified live on python 3.14.4
//! (Linux: `json.dumps` → `json/__init__.py:185`, `os.path.join` →
//! frozen-origin fallback → `posixpath.py:72`). stdlib content drifts
//! across python versions and `posixpath` is the POSIX spelling
//! (`ntpath` on Windows) — see the corpus README.
//!
//! A deliberate provider/seam behavior change is a deliberate golden diff:
//! these goldens are HAND-AUTHORED — each file is BOTH the probe spec and
//! the expected outcome, pinned byte-for-byte with hand-written header
//! prose — so a re-bless does NOT re-render them (unlike the re-derivable
//! js/go goldens). A bless run writes EVERY golden back UNCHANGED (the
//! exact bytes a green run compares against), eprintlns a no-change line
//! per file, then FAILS the run (011-08 follow-up P2-1: an accidental
//! `GOLDEN_BLESS` can never end green; a deliberate hand-edit round-trips
//! byte-identically):
//! `GOLDEN_BLESS=1 cargo test -p redline-resolve --test golden_python`.
//!
//! Deterministic probe walk: `*.golden` files under the corpus root in
//! sorted (byte) order, one provider resolve per probe.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use redline_resolve::providers::python_provider::PythonProvider;
use redline_resolve::{ResolvedSource, SymbolContext, ToolingProvider};

/// The checked-in corpus, relative to the crate manifest dir.
fn corpus_src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/python")
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
    /// The app's scope hint (import declaration path, item included;
    /// empty = no hint — the byte-for-byte degradation contract).
    scope: Vec<String>,
    /// Probe workspace root, corpus-root-relative.
    root: String,
    resolved: Option<ExpectedResolved>,
    /// The exact bail message with `{root}` / `{corpus}` placeholders
    /// standing in for the dynamic absolute paths.
    bail: Option<String>,
}

struct ExpectedResolved {
    external: bool,
    /// `true` when `file` / `source_root` are relative to the live stdlib
    /// root (the interpreter's own tree) instead of the corpus copy root.
    stdlib: bool,
    /// File relative to the corpus root (or the stdlib root).
    file: String,
    /// Source root relative to the corpus root (or the stdlib root).
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
                stdlib: map.get("path_base").map(|s| s.as_str()) == Some("stdlib"),
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
/// `{root}`, the corpus copy root becomes `{corpus}`. Everything else must
/// match byte-for-byte.
fn normalize(msg: &str, corpus: &Path, probe_root: &Path) -> String {
    let s = sub_path(msg.to_string(), probe_root, "{root}");
    sub_path(s, corpus, "{corpus}")
}

/// `p` relative to `base` (raw or canonicalized base prefix).
fn rel_to(base: &Path, p: &Path) -> Option<String> {
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

/// The live stdlib root of the ambient `python3`
/// (`sysconfig.get_path('stdlib')` — e.g. `/usr/lib/python3.14`).
fn stdlib_root() -> PathBuf {
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg("import sysconfig; print(sysconfig.get_path('stdlib'))")
        .output()
        .and_then(|o| {
            if o.status.success() {
                Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "python3 sysconfig query failed",
                ))
            }
        })
        .expect("python3 stdlib query failed");
    PathBuf::from(out)
}

// ── golden bless bookkeeping (011-08 follow-up P2-1, python lane) ──────────
//
// The python goldens are hand-authored and authoritative: the bytes a green
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

/// Bless-mode write (no-ops unless `GOLDEN_BLESS` is set): a python golden is
/// hand-authored, so bless writes the CHECKED-IN golden back with the exact
/// bytes a green run compares against (byte-preserving — writes to the
/// checked-in corpus, not the tempdir copy, so `git diff` reflects it) and
/// eprintlns a no-change line. `changed` is the js/go-compatible bookkeeping
/// (always false here: there is no independent live-derived rendering that
/// could differ from the checked-in file).
fn bless_python_golden(corpus_src: &Path, tmp_golden: &Path) {
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
fn check_resolved(
    src: &ResolvedSource,
    exp: &ExpectedResolved,
    corpus: &Path,
    stdlib: &Path,
) -> Vec<String> {
    let mut bad = Vec::new();
    if src.external != exp.external {
        bad.push(format!("external: expected {}, got {}", exp.external, src.external));
    }
    let base = if exp.stdlib { stdlib } else { corpus };
    let file_rel = rel_to(base, &src.file)
        .unwrap_or_else(|| format!("<outside base: {}>", src.file.display()));
    if file_rel != exp.file {
        bad.push(format!("file: expected {}, got {file_rel}", exp.file));
    }
    let root_rel = rel_to(base, &src.source_root)
        .unwrap_or_else(|| format!("<outside base: {}>", src.source_root.display()));
    let expected_root = if exp.source_root == "." {
        String::new()
    } else {
        exp.source_root.clone()
    };
    if root_rel != expected_root {
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
fn python_golden_corpus() {
    // Loud skip when the real python3 toolchain is absent.
    let python_ok = std::process::Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !python_ok {
        eprintln!(
            "SKIP (loud): `python3` binary not available; the python golden corpus \
             (live `find_spec` / `is_stdlib` legs) was NOT run"
        );
        return;
    }

    // Copy the corpus to a tempdir so the live interpreter's scratch
    // (any __pycache__ from the find_spec/import fallbacks) never touches
    // the checked-in files, and so the golden path placeholders have a
    // stable shape to normalize against.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    let corpus_src = corpus_src_dir();
    copy_tree(&corpus_src, &corpus).expect("copy corpus");

    // OFFLINE provider: every LIVE leg in the suite is the python3
    // interpreter itself (find_spec, is_stdlib, frozen __file__ fallback,
    // sysconfig stdlib root). The network `pip install` leg is deliberately
    // refused, so the missing-module degradation is a deterministic,
    // byte-for-byte offline bail — the gate never touches the network.
    let provider = PythonProvider::new().offline();

    let stdlib = stdlib_root();

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
        bless_python_golden(&corpus_src, path);
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
            language: Some("python".to_string()),
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
                let bad = check_resolved(&src, exp, &corpus, &stdlib);
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
                let got = normalize(&e.to_string(), &corpus, &probe_root);
                if got != *exp {
                    failures.push(format!(
                        "{} ({}): bail mismatch\n  expected: {exp}\n  got:      {got}",
                        probe.name, probe.symbol
                    ));
                }
            }
        }
    }

    assert_bless_stopped("python_golden_corpus");
    assert!(
        failures.is_empty(),
        "{} golden mismatch(es) out of {} probes:\n\n{}",
        failures.len(),
        goldens.len(),
        failures.join("\n\n")
    );
}
