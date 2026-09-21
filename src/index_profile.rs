//! Headless fine-grained index profiler (`--index-profile[=PATH]`).
//!
//! Drives the EXACT production indexing path and reports where the time
//! goes, so a user can point it at a large project and hand back the
//! numbers:
//! - walk: `model::files::FileList::build` (the gitignore-filtered walk);
//! - parse: the same rayon `par_iter` fan-out as
//!   `nav::index::builder::extract_files`, per file via
//!   `std::fs::read_to_string` + `redline_syntax::queries::extract_all_timed`
//!   (the production extractor, with its optional timings out-param);
//! - assembly: `nav::index::builder::assemble_index` (the production
//!   serial loop, verbatim).
//!
//! Nothing here changes indexing behaviour (no caps, no skipping, no
//! tuning), and when the flag is absent this module is never called (one
//! flag check in `main`).
//!
//! Privacy (default): no real path ever appears in the report, the
//! `--profile-out` CSV, or an error message. Every file is printed as a
//! pseudonym `t<top>/d<depth>/f<id>.<ext>` — the top-level component
//! (id-assigned), the depth (component count), the extension, and a
//! sequential id assigned ONCE per file, in walk-sorted order, before
//! any printing (see the pre-assignment in `run`). Because the assignment
//! order is the deterministic walk order, the ids are stable across
//! runs on the same tree (verified: `--profile-repeat=2` yields zero
//! differing ids across the two CSVs). The ids are intentionally OPAQUE:
//! not a hash (a hash is dictionary-reversible against a known candidate
//! set) and not derivable from the path. The root prints as `<root>`.
//! `--profile-real-paths` lifts this (local use only).
//!
//! Measurement caveats (also printed in the report): the per-file
//! `Instant` pairs are negligible; repeat runs share the process, so run
//! 2+ is warmer than a fresh-process warm start (page cache AND the
//! in-process per-thread query/parser caches are already hot); peak RSS
//! includes the profiler's own per-file rows; the ms figures are
//! load-sensitive (file/symbol counts are not).

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rayon::prelude::*;

use redline_syntax::queries::{extract_all_timed, ExtractTimings, RustTables, Symbol};
use redline_syntax::registry::{resolve_language, file_extension, LanguageId};

use crate::model::files::FileList;
use crate::nav::index::assemble_index;

/// Profiler options (parsed in `main`).
pub struct Options {
    pub root: PathBuf,
    pub top: usize,
    pub out: Option<PathBuf>,
    pub repeat: usize,
    pub real_paths: bool,
}

/// One per-file record (read + extraction + the parse-vs-query split).
struct Raw {
    rel: String,
    lang: LanguageId,
    bytes: u64,
    read: Duration,
    extract: Duration,
    stages: ExtractTimings,
    unreadable: bool,
    /// Symbol count, captured before `assemble` moves the vec out (the
    /// report and the CSV print per-file counts after assembly).
    symbols: u64,
    /// The pseudonym (or real path), assigned ONCE per run in walk order
    /// before any printing — so every consumer (top-N lists, CSV) shows
    /// the same id for the same file.
    label: String,
    syms: Vec<Symbol>,
    tables: RustTables,
}

/// Timed phases of one run.
struct Run {
    walk: Duration,
    parse_wall: Duration,
    assembly: Duration,
    cpu_sum: Duration,
    total_bytes: u64,
    total_symbols: u64,
    parse_sum: Duration,
    queries_sum: Duration,
    compile_sum: Duration,
    plain: u64,
    unreadable: u64,
    zero_symbol: u64,
    raws: Vec<Raw>,
}

/// Stable, structure-preserving path pseudonyms (see the module docs).
/// Both id caches make `label` idempotent: the same path always gets the
/// same `t<top>` and `f<id>` within a run, no matter how many times or in
/// what order it is asked for.
struct Anon {
    tops: HashMap<String, u32>,
    files: HashMap<String, u32>,
    next_top: u32,
    next_file: u32,
}

impl Anon {
    /// Pseudonym for a project-relative path: `t<top>/d<depth>/f<id>.<ext>`
    /// (no extension when the file has none). The file id is assigned on
    /// first ask and cached, so repeated calls return the same label;
    /// `run` pre-assigns every row in walk-sorted order before printing,
    /// which is what makes the ids (including the top-level ids, which are
    /// allocated in that same deterministic order) stable across runs on
    /// the same tree.
    fn label(&mut self, rel: &str) -> String {
        let comps: Vec<&str> = rel.split('/').filter(|c| !c.is_empty()).collect();
        let top = comps[0].to_string();
        let top_id = match self.tops.get(&top) {
            Some(id) => *id,
            None => {
                self.next_top += 1;
                self.tops.insert(top, self.next_top);
                self.next_top
            }
        };
        let file_id = match self.files.get(rel) {
            Some(id) => *id,
            None => {
                self.next_file += 1;
                self.files.insert(rel.to_string(), self.next_file);
                self.next_file
            }
        };
        let ext = file_extension(rel)
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        format!(
            "t{top_id:02}/d{}/f{id:04}{ext}",
            comps.len(),
            id = file_id
        )
    }
}

/// The production walk (`FileList::build`), timed.
///
/// Walk count vs `git ls-files`: the walk additionally drops (a) tracked
/// files under hidden directories (e.g. `.agents/`) — `hidden(true)`, and
/// (b) tracked dotfiles/dotdirs at any depth, plus (c) tracked-but-ignored
/// files (e.g. under a gitignored `graft/`). So for this repo at HEAD:
/// 490 tracked − 172 `.agents/` − 5 tracked dotfiles − 1 tracked-but-
/// ignored = 312 walked. (The ms figures are load-sensitive; this count
/// arithmetic is not.)
fn walk(root: &Path) -> anyhow::Result<(Vec<String>, Duration)> {
    let t = Instant::now();
    let list = FileList::build(root)
        .map_err(|_| anyhow::anyhow!("cannot walk <root>: it is not a directory"))?;
    Ok((list.files, t.elapsed()))
}

/// The production parse phase, timed, with per-file detail: the same
/// `par_iter` fan-out as `builder::extract_files`, each file read via
/// `std::fs::read_to_string` and extracted via the production
/// `extract_all_timed` (the `extract_file` body, with timings).
fn parse_files(root: &Path, files: &[String]) -> (Duration, Vec<Raw>) {
    let t = Instant::now();
    let raws: Vec<Raw> = files
        .par_iter()
        .map(|rel| {
            let lang = resolve_language(rel);
            let abs = root.join(rel);
            let rt = Instant::now();
            let Ok(text) = std::fs::read_to_string(&abs) else {
                return Raw {
                    rel: rel.clone(),
                    lang,
                    bytes: 0,
                    read: rt.elapsed(),
                    extract: Duration::ZERO,
                    stages: ExtractTimings::default(),
                    unreadable: true,
                    symbols: 0,
                    label: String::new(),
                    syms: Vec::new(),
                    tables: RustTables::default(),
                };
            };
            let read = rt.elapsed();
            let et = Instant::now();
            let mut stages = ExtractTimings::default();
            let (syms, tables) = extract_all_timed(lang, &text, Some(&mut stages));
            let extract = et.elapsed();
            Raw {
                rel: rel.clone(),
                lang,
                bytes: text.len() as u64,
                read,
                extract,
                stages,
                unreadable: false,
                symbols: syms.len() as u64,
                label: String::new(),
                syms,
                tables,
            }
        })
        .collect();
    (t.elapsed(), raws)
}

/// The production serial assembly loop, timed (verbatim `assemble_index`).
fn assemble(raws: &mut [Raw]) -> Duration {
    let entries: Vec<(String, Vec<Symbol>, RustTables)> = raws
        .iter_mut()
        .map(|r| {
            (
                r.rel.clone(),
                std::mem::take(&mut r.syms),
                std::mem::take(&mut r.tables),
            )
        })
        .collect();
    let t = Instant::now();
    let _index = assemble_index(entries);
    t.elapsed()
}

/// Drive one full production indexing pass with per-phase + per-file
/// timing.
fn run_once(root: &Path) -> anyhow::Result<Run> {
    let (files, walk) = walk(root)?;
    let (parse_wall, mut raws) = parse_files(root, &files);

    // Per-file aggregates FIRST (assemble moves the per-file symbols out
    // of `raws`).
    let mut cpu_sum = Duration::ZERO;
    let mut total_bytes = 0u64;
    let mut total_symbols = 0u64;
    let mut parse_sum = Duration::ZERO;
    let mut queries_sum = Duration::ZERO;
    let mut compile_sum = Duration::ZERO;
    let mut plain = 0u64;
    let mut unreadable = 0u64;
    let mut zero_symbol = 0u64;
    for r in &raws {
        cpu_sum += r.read + r.extract;
        total_bytes += r.bytes;
        total_symbols += r.symbols;
        parse_sum += r.stages.parse;
        queries_sum += r.stages.queries;
        compile_sum += r.stages.query_compile;
        if r.lang == LanguageId::Plain {
            plain += 1;
        }
        if r.unreadable {
            unreadable += 1;
        }
        if r.symbols == 0 {
            zero_symbol += 1;
        }
    }

    let assembly = assemble(&mut raws);

    Ok(Run {
        walk,
        parse_wall,
        assembly,
        cpu_sum,
        total_bytes,
        total_symbols,
        parse_sum,
        queries_sum,
        compile_sum,
        plain,
        unreadable,
        zero_symbol,
        raws,
    })
}

/// Entry point (called from `main` when `--index-profile` is present,
/// before any terminal/TUI setup; works with no tty).
pub fn run(opts: &Options) -> anyhow::Result<()> {
    let root = opts
        .root
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("cannot resolve <root>: it does not exist"))?;
    if !root.is_dir() {
        anyhow::bail!("<root> is not a directory");
    }
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);
    let repeat = opts.repeat.max(1);
    let mut out = String::new();
    out.push_str("redline index profile\n");
    if opts.real_paths {
        out.push_str("  paths:   REAL (--profile-real-paths: local use only, do not paste this report)\n");
    } else {
        out.push_str("  paths:   ANONYMIZED — file ids are opaque (t<top>/d<depth>/f<id>); stable per tree, not derivable from the path, not hashed\n");
    }
    out.push_str(&format!("  root:    <root> ({repeat} run(s))\n"));
    out.push_str(&format!("  build:   {}\n", build_profile()));
    #[cfg(debug_assertions)]
    {
        out.push_str("  ! WARNING: DEBUG BUILD — parse/extract times are MANY TIMES slower than a release build; do not compare these numbers to release.\n");
    }

    let mut csv_error: Option<std::io::Error> = None;
    for n in 1..=repeat {
        // One Anon per run. The file ids are assigned ONCE per file, in
        // walk-sorted order (the deterministic order), before ANY
        // printing or CSV writing — the top-N lists print in timing rank
        // order and the CSV in walk order, but every consumer reads the
        // pre-assigned `Raw::label`. Because the assignment order is the
        // walk order (not the timing order), repeat runs on the same tree
        // assign identical ids: the same file gets the SAME id in every
        // run, so repeat-run CSVs align row-for-row.
        let mut anon = Anon {
            tops: HashMap::new(),
            files: HashMap::new(),
            next_top: 0,
            next_file: 0,
        };
        let mut run = run_once(&root)?;
        for r in &mut run.raws {
            r.label = if opts.real_paths {
                r.rel.clone()
            } else {
                anon.label(&r.rel)
            };
        }
        if n == 1 {
            // The rayon pool is initialized by run 1's parse phase;
            // report its size for the parallelism comparison.
            let pool = rayon::current_num_threads();
            out.push_str(&format!("  cores:   {cores} | rayon pool: {pool} workers\n"));
        }
        let warm = if n == 1 {
            "cold (first pass in this process: page cache, per-thread query compile, and pool startup included)"
        } else {
            "warm (repeat: page cache warm, in-process query/parser caches hot, pool already up)"
        };
        out.push_str(&format!("\nrun {n}/{repeat} — {warm}\n\n"));

        print_phases(&mut out, &run, cores);
        print_per_file(&mut out, &run, opts);
        print_breakdowns(&mut out, &run);

        if let Some(csv) = csv_path(&opts.out, n, repeat) {
            // The REPORT is printed below even when this fails: the CSV
            // error is remembered, not propagated mid-way through the
            // loop.
            if let Err(kind) = write_csv(&csv, &run) {
                csv_error = Some(kind);
            } else {
                let label = if opts.real_paths { "REAL PATHS" } else { "anonymized" };
                out.push_str(&format!(
                    "\nper-file CSV: written to the --profile-out file ({label} paths)\n"
                ));
            }
        }
    }

    out.push_str(&format!(
        "\npeak RSS: {} (whole process, VmHWM — includes the index, the profiler's per-file rows, and this report)\n",
        peak_rss_line()
    ));
    out.push_str(
        "\ncaveats: per-file timing overhead is a pair of Instant::now() calls (negligible); \
repeat runs share the process, so run 2+ is warmer than a fresh-process warm start; \
the profiler's par_iter mirrors the production fan-out but allocates a per-file row (the production path allocates none when this flag is absent); \
the ms figures are LOAD-SENSITIVE — treat them as machine-load-dependent, not reproducible constants (file/symbol counts are stable); \
a --profile-out file INSIDE the profiled root becomes part of the tree (a warm repeat run walks it — the file is created during run 1).\n",
    );
    print!("{out}");
    if let Some(err) = csv_error {
        anyhow::bail!(
            "the report above is complete, but a --profile-out CSV could not be written ({}; the path itself is not echoed)",
            err.kind()
        );
    }
    Ok(())
}

fn build_profile() -> &'static str {
    #[cfg(debug_assertions)]
    {
        "debug (!)"
    }
    #[cfg(not(debug_assertions))]
    {
        "release"
    }
}

fn print_phases(out: &mut String, run: &Run, cores: usize) {
    let n = run.raws.len() as u64;
    let total = run.walk + run.parse_wall + run.assembly;
    let warm = run.parse_wall + run.assembly;
    out.push_str("phases (wall clock):\n");
    out.push_str(&format!(
        "  walk      FileList::build (gitignore-filtered)     {:>10.3} ms  {n} files\n",
        ms(run.walk)
    ));
    out.push_str(&format!(
        "  parse     rayon par_iter: read + tree-sitter       {:>10.3} ms wall\n",
        ms(run.parse_wall)
    ));
    out.push_str(&format!(
        "            per-file CPU (sum of read+extract)       {:>10.3} ms\n",
        ms(run.cpu_sum)
    ));
    out.push_str(&format!(
        "  assembly  serial set_file/set_file_tables loop     {:>10.3} ms\n",
        ms(run.assembly)
    ));
    out.push_str(&format!("  total                                           {:>10.3} ms\n", ms(total)));
    out.push_str(&format!(
        "  total - walk  (parse + assembly)                {:>10.3} ms   <- what a WARM PERSISTED INDEX would cost to start (the number that decides whether persistence is worth building)\n",
        ms(warm)
    ));
    if run.parse_wall > Duration::ZERO {
        let ratio = run.cpu_sum.as_secs_f64() / run.parse_wall.as_secs_f64();
        let pct = if cores > 0 {
            ratio / cores as f64 * 100.0
        } else {
            0.0
        };
        let verdict = if cores > 0
            && ratio < cores as f64 * 0.75
            && run.cpu_sum > Duration::from_millis(200)
        {
            " — LOW: the fan-out is not filling the pool (I/O contention, a few dominant files, or thread-count ceiling)"
        } else {
            ""
        };
        out.push_str(&format!(
            "parallelism:  CPU/wall = {ratio:.1}x (of {cores} cores, {pct:.0}% of the pool){verdict}\n"
        ));
    }
    if total > Duration::ZERO && n > 0 {
        let secs = total.as_secs_f64();
        let files_s = n as f64 / secs;
        let mb_s = (run.total_bytes as f64) / 1_000_000.0 / secs;
        let per_core = if cores > 0 {
            mb_s / cores as f64
        } else {
            mb_s
        };
        out.push_str(&format!(
            "throughput:   {files_s:.0} files/s wall | {mb_s:.1} MB/s overall | {per_core:.1} MB/s per core (bytes/wall/cores)\n"
        ));
    }
}

fn print_per_file(out: &mut String, run: &Run, opts: &Options) {
    let n = run.raws.len();
    // Distribution helpers work on the per-run value vectors.
    let extracts: Vec<f64> = run.raws.iter().map(|r| ms(r.extract)).collect();
    let bytes: Vec<f64> = run.raws.iter().map(|r| r.bytes as f64).collect();
    out.push_str(&format!(
        "extract_ms:    p50={:.3} p90={:.3} p99={:.3} max={:.3}  (n={n})\n",
        pctile(&extracts, 50.0),
        pctile(&extracts, 90.0),
        pctile(&extracts, 99.0),
        extracts
            .iter()
            .copied()
            .fold(0.0_f64, f64::max)
    ));
    out.push_str(&format!(
        "bytes:         p50={} p90={} p99={} max={}  (n={n})\n",
        human_bytes(pctile(&bytes, 50.0)),
        human_bytes(pctile(&bytes, 90.0)),
        human_bytes(pctile(&bytes, 99.0)),
        human_bytes(bytes.iter().copied().fold(0.0_f64, f64::max))
    ));

    // Top-N (labels were pre-assigned per run in walk order, so the
    // timing-rank order here cannot renumber any file).
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| {
        run.raws[*b]
            .extract
            .cmp(&run.raws[*a].extract)
            .then_with(|| a.cmp(b))
    });
    let top = opts.top.min(n);
    out.push_str(&format!("\ntop {top} slowest by extract_ms (a few dominant files = a size cap would help):\n"));
    for (i, &idx) in order.iter().take(top).enumerate() {
        let r = &run.raws[idx];
        out.push_str(&format!(
            "  {:>3}. {}  extract {:>9.3} ms  read {:>7.3} ms  {:>8}  {:>8} syms  {}\n",
            i + 1,
            r.label,
            ms(r.extract),
            ms(r.read),
            human_bytes(r.bytes as f64),
            r.symbols,
            r.lang.name()
        ));
    }

    order.sort_by(|a, b| {
        run.raws[*b]
            .bytes
            .cmp(&run.raws[*a].bytes)
            .then_with(|| a.cmp(b))
    });
    out.push_str(&format!("\ntop {top} largest by bytes:\n"));
    for (i, &idx) in order.iter().take(top).enumerate() {
        let r = &run.raws[idx];
        out.push_str(&format!(
            "  {:>3}. {}  {:>8}  extract {:>9.3} ms  {:>8} syms  {}\n",
            i + 1,
            r.label,
            human_bytes(r.bytes as f64),
            ms(r.extract),
            r.symbols,
            r.lang.name()
        ));
    }
}

fn print_breakdowns(out: &mut String, run: &Run) {
    // Per-language: files, bytes, ms (sum of read+extract), symbols.
    #[derive(Default)]
    struct LangStat {
        files: u64,
        bytes: u64,
        ms: f64,
        symbols: u64,
    }
    let mut by_lang: HashMap<String, LangStat> = HashMap::new();
    for r in &run.raws {
        let e = by_lang.entry(r.lang.name().to_string()).or_default();
        e.files += 1;
        e.bytes += r.bytes;
        e.ms += ms(r.read + r.extract);
        e.symbols += r.symbols;
    }
    let mut langs: Vec<(&String, &LangStat)> = by_lang.iter().collect();
    langs.sort_by(|a, b| b.1.ms.partial_cmp(&a.1.ms).unwrap_or(std::cmp::Ordering::Equal));
    out.push_str("\nper language (by ms):\n");
    out.push_str("  language   files    bytes        ms    symbols\n");
    for (name, stat) in langs {
        out.push_str(&format!(
            "  {language:<10} {files:>7} {bytes:>10} {m:>11.3} ms  {syms:>9}\n",
            language = name.as_str(),
            files = stat.files,
            bytes = human_bytes(stat.bytes as f64),
            m = stat.ms,
            syms = stat.symbols
        ));
    }
    let n = run.raws.len();
    out.push_str(&format!(
        "\ncounts: total symbols {} | zero-symbol files {z}/{n} | unresolved language (plain, skipped) {p}/{n} | unreadable (read failed) {u}/{n}\n",
        run.total_symbols,
        z = run.zero_symbol,
        p = run.plain,
        u = run.unreadable
    ));
    out.push_str(&format!(
        "extract_ms split (per-file sums): tree-sitter parse {:.3} ms | query execution {:.3} ms | one-time query compile {:.3} ms | (the remainder of extract_ms is symbol construction + sorting)\n",
        ms(run.parse_sum),
        ms(run.queries_sum),
        ms(run.compile_sum)
    ));
}

/// `--profile-out` path for run `n` (1-based): unchanged for a single
/// run, `<base>-run<n><ext>` when repeating.
fn csv_path(out: &Option<PathBuf>, n: usize, repeat: usize) -> Option<PathBuf> {
    let out = out.as_ref()?;
    if repeat <= 1 {
        return Some(out.clone());
    }
    let base = out
        .file_name()
        .map(|b| b.to_string_lossy().into_owned())
        .unwrap_or_else(|| "index.csv".to_string());
    let (stem, ext) = match base.rfind('.') {
        Some(i) if i > 0 => (base[..i].to_string(), base[i..].to_string()),
        _ => (base, String::new()),
    };
    Some(out.with_file_name(format!("{stem}-run{n}{ext}")))
}

/// The per-file CSV: every row, `path,lang,bytes,read_ms,extract_ms,symbols`
/// (paths anonymized unless `--profile-real-paths`; the row's pre-assigned
/// `label` is used, so the CSV and the report agree row-for-row).
fn write_csv(out: &Path, run: &Run) -> std::io::Result<()> {
    let file = std::fs::File::create(out)?;
    let mut w = std::io::BufWriter::new(file);
    w.write_all(b"path,lang,bytes,read_ms,extract_ms,symbols\n")?;
    for r in &run.raws {
        let path = csv_field(&r.label);
        let read = format!("{:.3}", ms(r.read));
        let extract = format!("{:.3}", ms(r.extract));
        w.write_all(
            format!(
                "{path},{},{},{},{},{}\n",
                r.lang.name(),
                r.bytes,
                read,
                extract,
                r.symbols
            )
            .as_bytes(),
        )?;
    }
    w.flush()?;
    Ok(())
}

/// CSV-quote a field only when needed (paths with commas/quotes).
fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Nearest-rank percentile of an unsorted slice (0 when empty).
fn pctile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = ((p / 100.0) * v.len() as f64).ceil().clamp(1.0, v.len() as f64) as usize - 1;
    v[rank]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn human_bytes(b: f64) -> String {
    if b < 1024.0 {
        format!("{b:.0}B")
    } else if b < 1024.0 * 1024.0 {
        format!("{:.1}K", b / 1024.0)
    } else {
        format!("{:.1}M", b / (1024.0 * 1024.0))
    }
}

/// Peak RSS from `/proc/self/status` (Linux); "n/a" elsewhere.
fn peak_rss_line() -> String {
    #[cfg(target_os = "linux")]
    {
        let status = match std::fs::read_to_string("/proc/self/status") {
            Ok(s) => s,
            Err(_) => return "n/a (could not read /proc/self/status)".into(),
        };
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmHWM:")
                && let Ok(kb) = rest.trim().trim_end_matches("kB").trim().parse::<u64>()
            {
                return format!("{:.1} MB", kb as f64 / 1024.0);
            }
        }
        "n/a (no VmHWM line)".into()
    }
    #[cfg(not(target_os = "linux"))]
    {
        "n/a (peak RSS is read from /proc/self/status on Linux)".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anon_preserves_structure_and_is_stable() {
        let mut a = Anon {
            tops: HashMap::new(),
            files: HashMap::new(),
            next_top: 0,
            next_file: 0,
        };
        let l1 = a.label("src/main.rs");
        let l2 = a.label("src/nav/index/builder.rs");
        // Same path asked again (the top-N lists and the CSV each ask for
        // the same file): the id must NOT advance.
        let l3 = a.label("src/main.rs");
        assert_eq!(l1, "t01/d2/f0001.rs");
        assert_eq!(l2, "t01/d4/f0002.rs");
        assert_eq!(l3, l1, "re-asking must return the cached id");
        // A different top-level component gets its own top id.
        assert_eq!(a.label("crates/redline-syntax/src/queries.rs"), "t02/d4/f0003.rs");
        // No extension -> no dot in the label.
        assert_eq!(a.label("Makefile"), "t03/d1/f0004");
    }

    #[test]
    fn anon_never_echoes_the_input() {
        let mut a = Anon {
            tops: HashMap::new(),
            files: HashMap::new(),
            next_top: 0,
            next_file: 0,
        };
        let label = a.label("secret-project/src/top-secret.rs");
        assert!(!label.contains("secret"), "leak: {label}");
    }

    #[test]
    fn pctile_nearest_rank() {
        assert_eq!(pctile(&[], 50.0), 0.0);
        assert_eq!(pctile(&[4.0], 99.0), 4.0);
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(pctile(&v, 50.0), 50.0);
        assert_eq!(pctile(&v, 90.0), 90.0);
        assert_eq!(pctile(&v, 99.0), 99.0);
        assert_eq!(pctile(&v, 100.0), 100.0);
    }

    #[test]
    fn csv_field_quoting() {
        assert_eq!(csv_field("a/b.rs"), "a/b.rs");
        assert_eq!(csv_field("a,b.rs"), "\"a,b.rs\"");
        assert_eq!(csv_field("a\"b.rs"), "\"a\"\"b.rs\"");
    }

    #[test]
    fn csv_path_repeat_suffix() {
        assert_eq!(
            csv_path(&Some(PathBuf::from("/tmp/i.csv")), 1, 1),
            Some(PathBuf::from("/tmp/i.csv"))
        );
        assert_eq!(
            csv_path(&Some(PathBuf::from("/tmp/i.csv")), 2, 3),
            Some(PathBuf::from("/tmp/i-run2.csv"))
        );
        assert_eq!(
            csv_path(&Some(PathBuf::from("/tmp/out")), 2, 3),
            Some(PathBuf::from("/tmp/out-run2"))
        );
        assert_eq!(csv_path(&None, 1, 1), None);
    }

    #[test]
    fn human_bytes_units() {
        assert_eq!(human_bytes(512.0), "512B");
        assert_eq!(human_bytes(2048.0), "2.0K");
        assert_eq!(human_bytes(3.5 * 1024.0 * 1024.0), "3.5M");
    }

    /// End-to-end on a tempdir fixture: the production walk + parse +
    /// assembly must run headless and produce sane per-file numbers.
    #[test]
    fn fixture_run_produces_sane_numbers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn main() {}\npub fn other() {}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("notes.txt"), "just text\n").unwrap();
        let opts = Options {
            root: dir.path().to_path_buf(),
            top: 5,
            out: None,
            repeat: 1,
            real_paths: false,
        };
        // run() prints to stdout; capture that it succeeds and the walk
        // sees all three files.
        run(&opts).unwrap();
        let (files, _) = walk(dir.path()).unwrap();
        assert_eq!(files.len(), 3);
        let (_, raws) = parse_files(dir.path(), &files);
        assert_eq!(raws.len(), 3);
        let rust = raws.iter().find(|r| r.rel == "src/main.rs").unwrap();
        assert_eq!(rust.syms.len(), 2, "main + other");
        assert!(rust.extract > Duration::ZERO);
        let plain = raws.iter().find(|r| r.rel == "notes.txt").unwrap();
        assert_eq!(plain.lang, LanguageId::Plain);
        assert!(plain.syms.is_empty());
    }
}
