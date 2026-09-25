//! 011-08 (js lane) — golden-corpus suite for the JS/TS provider.
//!
//! The corpus at `tests/corpus/js/` is a small, genuine npm app: mixed
//! CommonJS / TypeScript / ESM sources (`src/`), two local `file:`
//! packages (`vendor/jslocal`, `vendor/@redline/fixture`), and real
//! registry dependencies (`left-pad`, `dot-prop`, `pascal-case`). The
//! probe spec (`probes.toml`) lists probe points — a corpus file + line,
//! the symbol, and the import-hint (`scope`) the app's tree-sitter
//! hand-off would supply — and each probe's outcome is checked in as
//! `golden/<id>.golden`:
//!
//! - a resolution: `external`, `source_root` (workspace-relative), `file`
//!   (source-root-relative), `line` (1-based; `null` for entry landings);
//! - a bail: the EXACT provider error message, byte-for-byte, with the
//!   per-run workspace root normalized to `<ws>` (everything else verbatim).
//!
//! Run discipline:
//! - `mode = "deterministic"` — offline provider: NO shell-outs at all,
//!   byte-stable on any machine;
//! - `mode = "online-deterministic"` — online provider, but the probe
//!   shape is guaranteed to bail before any `npm` spawn (documented per
//!   probe); no network needed;
//! - `mode = "live"` — a real `npm install` in a per-run tempdir copy.
//!   `#[ignore]`d with its accepting hook REMOVED (P1-2, gate): a run of
//!   the live leg refuses every install and can never end green by
//!   accident — re-verifying these goldens is a deliberate, reviewed act
//!   (re-add a hook + run `-- --ignored golden_live_probes`). When
//!   `npm` or `node` is absent from PATH an explicit run of the leg
//!   SKIPs LOUDLY (printed to stderr); the deterministic probes always
//!   run, and the live goldens are simply not verified that run.
//!
//! A deliberate provider/seam behavior change is a deliberate golden
//! diff: re-bless with
//! `GOLDEN_BLESS=1 cargo test -p redline-resolve --test golden_js`
//! and review the diff. The corpus OBSERVES the provider — it never
//! changes it (bugs it exposes are findings, not fixes, in this issue).
//! No PTY anywhere.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use redline_resolve::providers::js_provider::JsProvider;
use redline_resolve::{SymbolContext, ToolingProvider};

const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/js");
const GOLDEN_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/js/golden");

// NOTE: the js suite deliberately does NOT share from `tests/common/mod.rs`
// (unlike go/python/rust). Its helpers are genuinely different, and folding
// them into the shared module would collapse real differences:
//   - probes come from a `probes.toml` manifest, not `*.golden` files;
//   - bail messages normalize the workspace to `<ws>` via a raw
//     `.replace(ws_canon, …)` — not `common::sub_path`'s raw+canonicalized
//     substitution (which would also substitute the non-canonical root);
//   - `assert_bless_stopped` here is a whitespace variant (the original
//     `run          the suite` spacing) and uses fully-qualified
//     `std::sync::atomic::Ordering`; `check_golden` renders + creates the
//     golden dir itself.
// Each of these is preserved byte-for-byte below.

// ── probe spec (checked-in manifest: a strict, known subset of TOML) ─────────

#[derive(Debug, Clone)]
struct Probe {
    id: String,
    file: String,
    line: u32,
    symbol: String,
    scope: Vec<String>,
    language: Option<String>,
    ws: String,
    mode: String,
    note: String,
}

fn read_probes() -> Vec<Probe> {
    let path = Path::new(CORPUS).join("probes.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read probe spec {}: {e}", path.display()));
    let mut probes: Vec<Probe> = Vec::new();
    for raw in text.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed == "[[probe]]" {
            probes.push(Probe {
                id: String::new(),
                file: String::new(),
                line: 0,
                symbol: String::new(),
                scope: Vec::new(),
                language: None,
                ws: "corpus".to_string(),
                mode: "deterministic".to_string(),
                note: String::new(),
            });
            continue;
        }
        let probe = probes
            .last_mut()
            .unwrap_or_else(|| panic!("probes.toml: `{trimmed}` outside any [[probe]] block"));
        let (key, value) = trimmed
            .split_once('=')
            .unwrap_or_else(|| panic!("probes.toml: malformed line (no `=`): `{trimmed}`"));
        let key = key.trim();
        let value = value.trim();
        match key {
            "id" | "file" | "symbol" | "language" | "ws" | "mode" | "note" => {
                let s = parse_string(value, trimmed);
                match key {
                    "id" => probe.id = s,
                    "file" => probe.file = s,
                    "symbol" => probe.symbol = s,
                    "language" => probe.language = Some(s),
                    "ws" => probe.ws = s,
                    "mode" => probe.mode = s,
                    "note" => probe.note = s,
                    _ => unreachable!(),
                }
            }
            "scope" => probe.scope = parse_string_array(value, trimmed),
            "line" => probe.line = value.parse().unwrap_or_else(|_| {
                panic!("probes.toml: `line` is not an integer: `{trimmed}`")
            }),
            other => panic!("probes.toml: unknown key `{other}`: `{trimmed}`"),
        }
    }
    validate_probes(&probes);
    probes
}

fn parse_string(value: &str, line: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .map(|s| s.to_string())
        .unwrap_or_else(|| panic!("probes.toml: expected a \"quoted\" string: `{line}`"))
}

fn parse_string_array(value: &str, line: &str) -> Vec<String> {
    let inner = value
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .unwrap_or_else(|| panic!("probes.toml: expected a [\"string\", …] array: `{line}`"));
    inner
        .split('"')
        // Keep only the quoted contents; the unquoted separators
        // (whitespace + commas between elements) carry no string.
        .filter(|s| s.chars().any(|c| !c.is_whitespace() && c != ','))
        .map(str::to_string)
        .collect()
}

fn validate_probes(probes: &[Probe]) {
    assert!(!probes.is_empty(), "probe spec is empty");
    let mut seen = std::collections::BTreeSet::new();
    for p in probes {
        assert!(!p.id.is_empty(), "a probe without an id");
        assert!(seen.insert(&p.id), "duplicate probe id `{}`", p.id);
        assert!(
            !p.file.is_empty() && p.line > 0,
            "probe `{}`: missing corpus anchor (file/line)",
            p.id
        );
        assert!(!p.symbol.is_empty(), "probe `{}`: missing symbol", p.id);
        assert!(
            matches!(p.ws.as_str(), "corpus" | "empty"),
            "probe `{}`: ws must be `corpus` or `empty` (got `{}`)",
            p.id,
            p.ws
        );
        assert!(
            matches!(
                p.mode.as_str(),
                "deterministic" | "online-deterministic" | "live"
            ),
            "probe `{}`: unknown mode `{}`",
            p.id,
            p.mode
        );
    }
}

// ── probe execution ──────────────────────────────────────────────────────────

/// The captured outcome of one probe, in golden form: resolution fields
/// workspace/source-root-relative, bail messages with the ws root
/// normalized to `<ws>`.
#[derive(Debug)]
enum Outcome {
    Resolved {
        external: bool,
        source_root_rel: String,
        file_rel: String,
        line: Option<u32>,
    },
    Bail(String),
}

/// Build the probe's workspace: a per-run tempdir. `corpus` = a copy of
/// the checked-in corpus (node_modules, package-lock.json, the golden dir
/// and this manifest are NOT copied — the copy is a pristine npm project);
/// `empty` = a bare tempdir (no package.json down from it).
fn build_ws(kind: &str) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    if kind == "corpus" {
        copy_corpus(tmp.path());
    }
    tmp
}

fn copy_corpus(dst: &Path) {
    fn walk(src: &Path, dst: &Path) {
        for entry in std::fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if path.is_dir() {
                if name == "node_modules" || name == "golden" {
                    continue;
                }
                std::fs::create_dir_all(dst.join(&name)).unwrap();
                walk(&path, &dst.join(&name));
            } else {
                if name == "package-lock.json" || name == "probes.toml" {
                    continue;
                }
                std::fs::copy(&path, dst.join(&name)).unwrap();
            }
        }
    }
    walk(Path::new(CORPUS), dst);
}

/// Run one probe against its workspace and capture the outcome.
fn capture(
    probe: &Probe,
    ws_root: &Path,
    provider: &JsProvider,
    confirm: Option<std::sync::Arc<redline_resolve::FetchConfirmFn>>,
) -> Outcome {
    let ws_canon = ws_root.canonicalize().unwrap();
    let ws_canon_str = ws_canon.to_string_lossy().into_owned();
    let ctx = SymbolContext {
        workspace_root: ws_root.to_path_buf(),
        symbol: probe.symbol.clone(),
        from_file: PathBuf::from(&probe.file),
        scope: probe.scope.clone(),
        language: probe.language.clone(),
        confirm_fetch: confirm,
    };
    match provider.resolve(&ctx) {
        Ok(src) => {
            let source_root_rel = src
                .source_root
                .strip_prefix(&ws_canon)
                .unwrap_or_else(|_| {
                    panic!(
                        "probe `{}`: source_root {:?} is not under the workspace {:?}",
                        probe.id, src.source_root, ws_canon
                    )
                })
                .to_string_lossy()
                .into_owned();
            let file_rel = src
                .file
                .strip_prefix(&src.source_root)
                .unwrap_or_else(|_| {
                    panic!(
                        "probe `{}`: file {:?} is not under source_root {:?}",
                        probe.id, src.file, src.source_root
                    )
                })
                .to_string_lossy()
                .into_owned();
            Outcome::Resolved {
                external: src.external,
                source_root_rel,
                file_rel,
                line: src.line,
            }
        }
        Err(e) => Outcome::Bail(e.to_string().replace(&ws_canon_str, "<ws>")),
    }
}

/// The provider for a probe's mode: deterministic = offline (never
/// shells out); the online modes use the ambient `npm` on PATH.
fn provider_for(mode: &str) -> JsProvider {
    if mode == "deterministic" {
        JsProvider::new().offline()
    } else {
        JsProvider::new()
    }
}

/// Toolchain present on PATH? (established skip-loudly precondition: the
/// live legs shell out to the REAL npm/node.)
fn toolchain_present() -> (bool, bool) {
    fn present(bin: &str) -> bool {
        Command::new(bin)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    (present("npm"), present("node"))
}

// ── golden rendering / comparison ────────────────────────────────────────────

fn render_golden(probe: &Probe, outcome: &Outcome) -> String {
    let mut s = format!("# golden for probe `{}`  (011-08 js lane)\n", probe.id);
    s.push_str(&format!(
        "# anchor: {}:{}   symbol: `{}`   scope: {:?}   mode: {}\n",
        probe.file, probe.line, probe.symbol, probe.scope, probe.mode
    ));
    if !probe.note.is_empty() {
        s.push_str(&format!("# note: {}\n", probe.note));
    }
    match outcome {
        Outcome::Resolved {
            external,
            source_root_rel,
            file_rel,
            line,
        } => {
            s.push_str("kind: resolved\n");
            s.push_str(&format!("external: {external}\n"));
            s.push_str(&format!("source_root: {source_root_rel}\n"));
            s.push_str(&format!("file: {file_rel}\n"));
            s.push_str(&format!(
                "line: {}\n",
                line.map(|l| l.to_string())
                    .unwrap_or_else(|| "null".to_string())
            ));
        }
        Outcome::Bail(message) => {
            s.push_str("kind: bail\n");
            s.push_str("message: |\n");
            for l in message.lines() {
                s.push_str(&format!("  {l}\n"));
            }
        }
    }
    s
}

fn golden_path(probe: &Probe) -> PathBuf {
    Path::new(GOLDEN_DIR).join(format!("{}.golden", probe.id))
}

/// Compare (or, under `GOLDEN_BLESS=1`, write) one probe's golden.
/// 011-08 js review P2-1: bless-mode bookkeeping — every test asserts at
/// its end that a bless run STOPS (writes all goldens, then fails once),
/// so an accidental GOLDEN_BLESS can never end green.
static BLESSED_RUN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static BLESSED_CHANGED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// End-of-test gate: a bless run writes EVERY golden first, then fails
// exactly once per test (never per file — a per-file panic would abort
// the loop and leave later goldens unwritten).
fn assert_bless_stopped(test_name: &str) {
    if !BLESSED_RUN.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let changed = BLESSED_CHANGED.load(std::sync::atomic::Ordering::SeqCst);
    panic!(
        "GOLDEN_BLESS run of {test_name} rewrote {changed} golden(s) — run          the suite again WITHOUT GOLDEN_BLESS to verify the new goldens pass"
    );
}

fn check_golden(probe: &Probe, outcome: &Outcome, failures: &mut Vec<String>) {
    let rendered = render_golden(probe, outcome);
    let path = golden_path(probe);
    if std::env::var_os("GOLDEN_BLESS").is_some() {
        // 011-08 js review P2-1: an accidentally-set GOLDEN_BLESS would
        // otherwise make the whole suite green while REWRITING the
        // goldens (a regression silently re-blessed). Make it impossible
        // to miss: write to stderr, and fail when the new content
        // actually DIFFERS from the checked-in golden (a no-op re-bless
        // is the only silent case — re-review `git diff` before commit).
        let changed = std::fs::read(&path)
            .map(|old| old != rendered.as_bytes())
            .unwrap_or(true);
        std::fs::create_dir_all(GOLDEN_DIR)
            .unwrap_or_else(|e| panic!("cannot create golden dir: {e}"));
        std::fs::write(&path, &rendered)
            .unwrap_or_else(|e| panic!("cannot bless {}: {e}", path.display()));
        eprintln!(
            "GOLDEN_BLESS: {} {} — review `git diff` before committing; \
             this run FAILS so the bless cannot slip through green",
            if changed { "REWROTE" } else { "no change to" },
            path.display()
        );
        if changed {
            BLESSED_CHANGED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        BLESSED_RUN.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing golden {} ({e}) — run with GOLDEN_BLESS=1 to capture it",
            path.display()
        )
    });
    if expected != rendered {
        failures.push(format!(
            "probe `{}`: golden mismatch\n\
             --- expected (checked in) ---\n{expected}\
             --- actual (this run) ---\n{rendered}\
             --- (if the behavior change is deliberate, re-bless with GOLDEN_BLESS=1 and review the diff)",
            probe.id
        ));
    }
}

fn run_probes(probes: &[&Probe], failures: &mut Vec<String>) {
    for probe in probes {
        // A fresh workspace per probe: the deterministic modes never
        // install anything, so the copies stay pristine. The fetch
        // gate (issue-non-rust-receiver-resolution) gets NO hook here:
        // a deterministic mode that reached an install would refuse —
        // the goldens pin exactly that refusal.
        let ws_root = build_ws(&probe.ws);
        let provider = provider_for(&probe.mode);
        let outcome = capture(probe, ws_root.path(), &provider, None);
        check_golden(probe, &outcome, failures);
        println!("probe `{}`: {}", probe.id, summarize(&outcome));
    }
}

fn summarize(o: &Outcome) -> String {
    match o {
        Outcome::Resolved {
            external,
            source_root_rel,
            file_rel,
            line,
        } => format!(
            "resolved {source_root_rel}/{file_rel}:{line} (external={external})",
            line = line.map(|l| l.to_string()).unwrap_or_else(|| "null".into())
        ),
        Outcome::Bail(m) => {
            format!("bail: {}", m.lines().next().unwrap_or_default())
        }
    }
}

// ── the suite ────────────────────────────────────────────────────────────────

/// All non-live probes: deterministic (offline, no shell-outs) and
/// online-deterministic (bails before any npm spawn).
#[test]
fn golden_deterministic_probes() {
    let probes = read_probes();
    let det: Vec<&Probe> = probes
        .iter()
        .filter(|p| p.mode != "live")
        .collect();
    assert!(!det.is_empty(), "no deterministic probes");
    let mut failures = Vec::new();
    run_probes(&det, &mut failures);
    assert_bless_stopped("golden_deterministic_probes");
    assert!(
        failures.is_empty(),
        "{} deterministic probe(s) off their goldens:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// The live legs: real `npm install` of the registry dependencies in one
/// shared tempdir copy of the corpus (the last probe relies on that
/// populated node_modules).
///
/// `#[ignore]`d (P1-2, gate): a real registry install — it runs ONLY on
/// explicit opt-in (`cargo test -- --ignored golden_live_probes`), never
/// in the default suite. The accepting `confirm_fetch` hook the lane
/// supplied was REMOVED with it: with no hook an accidental un-ignored
/// run REFUSES (the provider's gate bails with the refusal; the live
/// goldens — which pin the accepted-install outcome — fail red, and no
/// `npm install` is ever spawned). Verifying a live golden again
/// deliberately requires re-adding a hook (an explicit, reviewed change).
#[test]
#[ignore] // real `npm install` from the registry (network) — explicit opt-in only
fn golden_live_probes() {
    let probes = read_probes();
    let live: Vec<&Probe> = probes.iter().filter(|p| p.mode == "live").collect();
    assert!(!live.is_empty(), "no live probes");
    let (npm, node) = toolchain_present();
    if !npm || !node {
        eprintln!(
            "SKIP (loud, 011-08 js lane): npm={npm}, node={node} — npm/node not on PATH, \
             the {} live JS probe(s) are SKIPPED (goldens not verified this run); \
             deterministic probes still ran.",
            live.len()
        );
        return;
    }
    // One shared corpus workspace: the first install populates
    // node_modules; the later probes reuse it.
    let shared = build_ws("corpus");
    // NO accepting hook (P1-2): the fetch gate refuses every install
    // here — a run of this leg installs nothing and fails against the
    // accepted-install goldens (the refusal is the pinned safe default;
    // re-adding a hook is the deliberate act that verifies the goldens).
    let mut failures = Vec::new();
    for probe in &live {
        let provider = provider_for(&probe.mode);
        let outcome = capture(probe, shared.path(), &provider, None);
        check_golden(probe, &outcome, &mut failures);
        println!("probe `{}`: {}", probe.id, summarize(&outcome));
    }
    assert_bless_stopped("golden_live_probes");
    assert!(
        failures.is_empty(),
        "{} live probe(s) off their goldens:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// Manifest/golden hygiene: every probe has exactly one golden (and no
/// orphan goldens), every anchor line exists in its corpus file, and the
/// corpus shape is what the doc comment promises.
#[test]
fn manifest_and_goldens_consistent() {
    let probes = read_probes();
    let n_det = probes.iter().filter(|p| p.mode == "deterministic").count();
    let n_online_det = probes
        .iter()
        .filter(|p| p.mode == "online-deterministic")
        .count();
    let n_live = probes.iter().filter(|p| p.mode == "live").count();
    eprintln!(
        "js corpus: {} probes total (deterministic {n_det}, online-deterministic {n_online_det}, live {n_live})",
        probes.len()
    );

    // Every probe ↔ one golden, both directions.
    let mut goldens = std::fs::read_dir(GOLDEN_DIR)
        .map_or_else(
            |_| std::collections::BTreeSet::new(),
            |dir| {
                dir.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            },
        );
    for p in &probes {
        let name = format!("{}.golden", p.id);
        assert!(goldens.remove(&name), "missing golden for probe `{}`", p.id);
    }
    assert!(
        goldens.is_empty(),
        "orphan golden(s) without a probe: {goldens:?}"
    );

    // Every anchor line exists in its corpus file.
    for p in &probes {
        let text = std::fs::read_to_string(Path::new(CORPUS).join(&p.file))
            .unwrap_or_else(|e| panic!("corpus file {} unreadable: {e}", p.file));
        let line = text
            .lines()
            .nth(p.line as usize - 1)
            .unwrap_or_else(|| panic!("probe `{}`: line {} beyond end of {}", p.id, p.line, p.file));
        assert!(!line.trim().is_empty(), "probe `{}`: anchor line is blank", p.id);
    }

    // The corpus is the npm project the probes assume.
    let pj = std::fs::read_to_string(Path::new(CORPUS).join("package.json")).unwrap();
    for dep in [
        "left-pad",
        "dot-prop",
        "pascal-case",
        "jslocal",
        "@redline/fixture",
    ] {
        assert!(pj.contains(dep), "corpus package.json lost dependency {dep}");
    }
}
