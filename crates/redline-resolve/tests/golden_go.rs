//! 011-08 (go lane) — golden-corpus suite for the GO provider.
//!
//! The corpus at `tests/corpus/go/` is a small, genuine Go module
//! (`project/`: `go.mod` + root package, `cmd/`, `internal/`, `pkg/`, a
//! local fork behind a `replace` directive) plus a module-cache-shaped
//! directory layout (`modcache/`) and a no-`go.mod` probe root
//! (`not_a_module/`). Each `*.golden` file is BOTH the probe spec
//! (dot-qualified or bare symbol, from_file, scope hint, probe root)
//! AND the expected outcome, pinned byte-for-byte: a `ResolvedSource`
//! (file / source root — corpus-root-relative — line, external flag) or
//! the exact bail message (the honest degradations are golden too:
//! aliased-import-hint-ignored, stdlib, intra-module last-segment, /vN
//! suffix, module-not-in-go.mod, bare-no-hint, ambiguous dot-import,
//! cache-missing offline refusal, item-not-found, no-go-mod).
//!
//! **LAYOUT-BASED, not live-toolchain (every result in this suite):**
//! the go toolchain is absent in the gate sandbox. The provider's module
//! cache is INJECTED via `with_mod_cache(corpus/modcache)` on an
//! `.offline()` provider, so `go env GOMODCACHE` and `go mod download`
//! are never invoked — `modcache/` is a checked-in layout the provider's
//! cache pattern accepts, not the product of a live `go` run. Every
//! golden header carries a `LAYOUT-BASED` marker, asserted by
//! `go_manifest_and_goldens_consistent`. The provider is strictly
//! read-only over the corpus, so the suite runs in place (no tempdir
//! copy). No PTY anywhere.
//!
//! A deliberate provider/seam behavior change is a deliberate golden
//! diff: re-bless with `GOLDEN_BLESS=1 cargo test -p redline-resolve
//! --test golden_go`. 011-08 js-review P2-1: a bless run writes EVERY
//! golden first, eprintlns a REWROTE / no-change line per file, then
//! FAILS the run — an accidental GOLDEN_BLESS can never end green.
//!
//! Deterministic probe walk: `*.golden` files at the corpus root in
//! sorted (byte) order, one provider resolve per probe.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use redline_resolve::providers::go_provider::GoProvider;
use redline_resolve::{SymbolContext, ToolingProvider};

/// The checked-in corpus (the provider never writes to it).
const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/go");

/// One parsed probe: the spec AND the expected outcome.
struct Probe {
    name: String,
    /// The leading `#` header lines, verbatim (preserved by bless).
    header: Vec<String>,
    symbol: String,
    from_file: String,
    /// The app's scope hint (package path, item included; empty = no
    /// hint — the byte-for-byte degradation contract).
    scope: Vec<String>,
    /// Probe workspace root, corpus-root-relative.
    root: String,
    resolved: Option<ExpectedResolved>,
    /// The exact bail message with `{root}` / `{cache}` placeholders
    /// standing in for the dynamic absolute paths.
    #[expect(
        dead_code,
        reason = "`bail` is parsed for spec symmetry with `resolved`, but bail assertions flow through the golden file, so the field is never read back"
    )]
    bail: Option<String>,
}

#[expect(
    dead_code,
    reason = "the probe only reads `resolved`'s shape (Some/None counters); the field values are compared via the rendered golden"
)]
struct ExpectedResolved {
    external: bool,
    /// File relative to the corpus root.
    file: String,
    /// Source root relative to the corpus root.
    source_root: String,
    line: u32,
}

/// The captured outcome of one probe, in golden form.
enum Outcome {
    Resolved {
        external: bool,
        file_rel: String,
        source_root_rel: String,
        line: u32,
    },
    Bail(String),
}

fn parse_golden(path: &Path) -> Probe {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let lines: Vec<&str> = text.lines().collect();

    // Header = the leading blank/comment lines (bless preserves them).
    let mut header = Vec::new();
    for line in &lines {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            header.push(line.to_string());
        } else {
            break;
        }
    }
    let body: Vec<&str> = lines.iter().copied().skip(header.len()).collect();

    let mut map: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for line in body {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("bad golden line in {}: {line:?}", path.display()));
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
    let inner = scope_raw.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or_else(|| {
        panic!("probe {}: `scope` is not a [\"…\"] array: `{scope_raw}`", path.display())
    });
    let scope = if inner.trim().is_empty() {
        Vec::new()
    } else {
        inner
            .split(',')
            .map(|p| p.trim().trim_matches('"').to_string())
            .collect()
    };
    let root = map.get("root").cloned().unwrap_or_else(|| "project".to_string());    let kind = need("kind");
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
        header,
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
/// `{root}`, the injected modcache becomes `{cache}`. Everything else
/// must match byte-for-byte.
fn normalize(msg: &str, probe_root: &Path, cache: &Path) -> String {
    let s = sub_path(msg.to_string(), probe_root, "{root}");
    sub_path(s, cache, "{cache}")
}

/// `p` relative to `base` (canonicalized when both exist, so joined
/// `./` segments — as `resolve_local_path` leaves them — normalize out).
fn rel_to(base: &Path, p: &Path) -> String {
    if let Ok(c_base) = std::fs::canonicalize(base)
        && let Ok(c_p) = std::fs::canonicalize(p)
        && let Ok(r) = c_p.strip_prefix(&c_base)
    {
        return r.to_string_lossy().into_owned();
    }
    if let Ok(r) = p.strip_prefix(base) {
        return r.to_string_lossy().into_owned();
    }
    if let Ok(c_base) = std::fs::canonicalize(base)
        && let Ok(r) = p.strip_prefix(&c_base)
    {
        return r.to_string_lossy().into_owned();
    }
    format!("<outside corpus root: {}>", p.display())
}

/// Run one probe against its (checked-in, read-only) workspace and
/// capture the outcome in golden form.
fn capture(probe: &Probe, probe_root: &Path, cache: &Path, provider: &GoProvider) -> Outcome {
    let corpus = Path::new(CORPUS);
    let ctx = SymbolContext {
        workspace_root: probe_root.to_path_buf(),
        symbol: probe.symbol.clone(),
        from_file: probe_root.join(&probe.from_file),
        scope: probe.scope.clone(),
        language: Some("go".to_string()),
    };
    match provider.resolve(&ctx) {
        Ok(src) => {
            let line = src.line.unwrap_or_else(|| {
                panic!(
                    "probe `{}`: resolved source has no line ({:?})",
                    probe.name, src.file
                )
            });
            Outcome::Resolved {
                external: src.external,
                file_rel: rel_to(corpus, &src.file),
                source_root_rel: rel_to(corpus, &src.source_root),
                line,
            }
        }
        Err(e) => Outcome::Bail(normalize(&e.to_string(), probe_root, cache)),
    }
}

// ── golden rendering / comparison ────────────────────────────────────────────

fn render_scope(scope: &[String]) -> String {
    if scope.is_empty() {
        "[]".to_string()
    } else {
        format!(
            "[{}]",
            scope
                .iter()
                .map(|s| format!("\"{s}\""))
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

fn render_golden(probe: &Probe, outcome: &Outcome) -> String {
    let mut s = String::new();
    for line in &probe.header {
        s.push_str(line);
        s.push('\n');
    }
    s.push_str(&format!("symbol={}\n", probe.symbol));
    s.push_str(&format!("from_file={}\n", probe.from_file));
    s.push_str(&format!("scope={}\n", render_scope(&probe.scope)));
    s.push_str(&format!("root={}\n", probe.root));
    match outcome {
        Outcome::Resolved {
            external,
            file_rel,
            source_root_rel,
            line,
        } => {
            s.push_str("kind=resolved\n");
            s.push_str(&format!("external={external}\n"));
            s.push_str(&format!("file={file_rel}\n"));
            s.push_str(&format!("source_root={source_root_rel}\n"));
            s.push_str(&format!("line={line}\n"));
        }
        Outcome::Bail(message) => {
            s.push_str("kind=bail\n");
            s.push_str(&format!("bail={message}\n"));
        }
    }
    s
}

/// 011-08 js review P2-1: bless-mode bookkeeping — every probe test
/// asserts at its end that a bless run STOPS (writes all goldens, then
/// fails once), so an accidental GOLDEN_BLESS can never end green.
static BLESSED_RUN: AtomicBool = AtomicBool::new(false);
static BLESSED_CHANGED: AtomicUsize = AtomicUsize::new(0);

/// End-of-test gate: a bless run writes EVERY golden first, then fails
/// exactly once per test (never per file — a per-file panic would abort
/// the loop and leave later goldens unwritten).
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

fn check_golden(probe: &Probe, rendered: &str, path: &Path, failures: &mut Vec<String>) {
    if std::env::var_os("GOLDEN_BLESS").is_some() {
        // 011-08 js review P2-1: an accidentally-set GOLDEN_BLESS would
        // otherwise make the whole suite green while REWRITING the
        // goldens (a regression silently re-blessed). Write to stderr,
        // and fail (via assert_bless_stopped) when the run touched any
        // file — a no-op re-bless still FAILS the run, so even it has
        // to be noticed.
        let changed = std::fs::read(path)
            .map(|old| old != rendered.as_bytes())
            .unwrap_or(true);
        std::fs::write(path, rendered)
            .unwrap_or_else(|e| panic!("cannot bless {}: {e}", path.display()));
        eprintln!(
            "GOLDEN_BLESS: {} {} — review `git diff` before committing; \
             this run FAILS so the bless cannot slip through green",
            if changed { "REWROTE" } else { "no change to" },
            path.display()
        );
        if changed {
            BLESSED_CHANGED.fetch_add(1, Ordering::SeqCst);
        }
        BLESSED_RUN.store(true, Ordering::SeqCst);
    }
    let expected = std::fs::read_to_string(path).unwrap_or_else(|e| {
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
            probe.name
        ));
    }
}

fn summarize(o: &Outcome) -> String {
    match o {
        Outcome::Resolved {
            external,
            file_rel,
            line,
            ..
        } => format!("resolved {file_rel}:{line} (external={external})"),
        Outcome::Bail(m) => format!("bail: {}", m.lines().next().unwrap_or_default()),
    }
}

// ── the suite ────────────────────────────────────────────────────────────────

/// The whole corpus, one pass, deterministic (sorted) probe walk.
/// LAYOUT-BASED: no go toolchain is required or invoked.
#[test]
fn go_golden_corpus() {
    let corpus = Path::new(CORPUS);
    let cache = corpus.join("modcache");
    assert!(
        cache.is_dir(),
        "modcache layout missing at {} — the LAYOUT-BASED corpus is incomplete",
        cache.display()
    );

    // OFFLINE provider with the INJECTED cache layout: `go env
    // GOMODCACHE` and `go mod download` are never invoked (probe 15
    // pins the offline refusal of the fetch leg, byte-for-byte).
    let provider = GoProvider::new().offline().with_mod_cache(cache.clone());

    let mut goldens: Vec<PathBuf> = std::fs::read_dir(corpus)
        .expect("read corpus dir")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("golden"))
        .collect();
    goldens.sort();
    assert!(
        !goldens.is_empty(),
        "no *.golden probes found under {}",
        corpus.display()
    );

    let mut failures: Vec<String> = Vec::new();
    for path in &goldens {
        let probe = parse_golden(path);
        let probe_root = corpus.join(&probe.root);
        let outcome = capture(&probe, &probe_root, &cache, &provider);
        let rendered = render_golden(&probe, &outcome);
        check_golden(&probe, &rendered, path, &mut failures);
        println!("probe `{}`: {}", probe.name, summarize(&outcome));
    }

    assert_bless_stopped("go_golden_corpus");
    assert!(
        failures.is_empty(),
        "{} golden mismatch(es) out of {} probes:\n\n{}",
        failures.len(),
        goldens.len(),
        failures.join("\n\n")
    );
}

/// Manifest/golden hygiene: every golden parses and carries the
/// LAYOUT-BASED marker, every from_file exists under its probe root,
/// the modcache layout is exactly what the probes assume (and the
/// absent sessionstore dir stays absent — probe 15's contract), and the
/// corpus's go.mod still carries the require/replace shape.
#[test]
fn go_manifest_and_goldens_consistent() {
    let corpus = Path::new(CORPUS);
    let mut goldens: Vec<PathBuf> = std::fs::read_dir(corpus)
        .expect("read corpus dir")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("golden"))
        .collect();
    goldens.sort();
    assert!(!goldens.is_empty(), "no *.golden probes found");

    let mut resolved = 0usize;
    let mut bails = 0usize;
    for path in &goldens {
        let probe = parse_golden(path);
        assert!(
            probe.header.join("\n").contains("LAYOUT-BASED"),
            "probe {} lost its LAYOUT-BASED marker (go goldens are layout-based, \
             never live-toolchain — restore the marker or the result is mislabeled)",
            probe.name
        );
        assert!(
            matches!(probe.root.as_str(), "project" | "not_a_module"),
            "probe {}: unknown root `{}`",
            probe.name,
            probe.root
        );
        let use_file = corpus.join(&probe.root).join(&probe.from_file);
        assert!(
            use_file.is_file(),
            "probe {}: from_file {} does not exist",
            probe.name,
            use_file.display()
        );
        match probe.resolved {
            Some(_) => resolved += 1,
            None => bails += 1,
        }
    }
    eprintln!(
        "go corpus (LAYOUT-BASED, no go toolchain): {} goldens (resolved {}, bail {})",
        goldens.len(),
        resolved,
        bails
    );
    assert!(resolved > 0 && bails > 0, "corpus must pin both outcome shapes");

    // The modcache layout is exactly what the resolved probes assume —
    // and the sessionstore dir stays ABSENT (adding it flips probe 15 from the
    // offline refusal to a landing: a deliberate golden diff, not a
    // silent one).
    let cache = corpus.join("modcache");
    for present in [
        "github.com/pkg/errors@v0.9.1",
        "github.com/!my!org/mylib@v1.2.3",
        "github.com/redlinecorp/auxversion@v0.4.0",
    ] {
        assert!(
            cache.join(present).is_dir(),
            "modcache layout lost {present}"
        );
    }
    assert!(
        !cache.join("github.com/redlinecorp/sessionstore@v0.2.0").exists(),
        "probe 15 contracts on the sessionstore cache dir being ABSENT (offline-refusal bail)"
    );

    // The go.mod the probes resolve against keeps its shape.
    let go_mod = std::fs::read_to_string(corpus.join("project/go.mod")).expect("project/go.mod");
    for line in [
        "module github.com/redlinecorp/gearserv",
        "github.com/MyOrg/mylib v1.2.3",
        "github.com/old/lib v0.0.0",
        "github.com/pkg/errors v0.9.1",
        "github.com/redlinecorp/auxversion v0.4.0",
        "github.com/golang-jwt/jwt/v5 v5.2.1",
        "github.com/redlinecorp/sessionstore v0.2.0",
        "replace github.com/old/lib => ./forks/lib",
    ] {
        assert!(go_mod.contains(line), "go.mod lost `{line}`");
    }
}
