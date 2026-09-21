# Task: a headless fine-grained index profiler

**User request:** *"give me a way to dump fine-grained profiling info and I'll run it in that
project"* — i.e. they will point it at a **large real project** and hand back the numbers, so
we can decide what to optimize. Optimize for *their* run, not for a fixture.

## What exists (do not rebuild it, do not disturb it)

`src/perf.rs` is the **method of record** for the README's perf table: a `#[cfg(test)]`
microbench over a **synthetic 500-file Rust repo**, reporting three coarse phases (cold
start / index / search first-hit), median of 5. It is **not** what this task needs — it
cannot be pointed at an arbitrary project and it reports no per-file detail. **Leave
`src/perf.rs` untouched.**

The production path to instrument is two calls (the same ones `start_indexing`,
`src/app/store/index_wiring.rs:302`, makes):

1. `FileList::build(root)` (`src/model/files.rs`) — the gitignore-filtered walk;
2. `build_index(root, &files, progress)` (`src/nav/index/builder.rs:33`) — a rayon-parallel
   parse (`par_iter`, thread-local parser + **already-cached** per-language `Query`), then a
   **serial** assembly loop (`for (rel, syms, tables) in entries`) that is a real phase and
   must be timed separately.

Drive **these exact functions** — do not reimplement the walk or the parse, or the numbers
are fiction.

## Invocation

A **headless** mode that prints a report and exits, usable with no tty:

```
redline --index-profile[=PATH]      # PATH defaults to cwd
redline --index-profile --profile-top=25 --profile-out=/tmp/index.csv
```

- It must run **before** any terminal/TUI setup (`src/main.rs:161`'s `IsTerminal` check and
  everything after it) and must not require a tty.
- `src/main.rs:43` currently accepts only `--notes=plain` and **hard-errors on unknown
  args** with a `usage:` line — update both, and keep the hard-error behaviour for genuinely
  unknown args. There is likely an arg-parsing test; extend it.
- When the flag is absent the cost is **one flag check** — nothing else changes.
- Print the report to stdout; exit 0. Document the flag in the README (a diagnostics line
  near the perf table).

## The report (fine-grained — this is the point)

**Environment first** (numbers are unreadable without it): build profile (**warn loudly if
`debug_assertions`** — debug parse times are many times release and would mislead), rayon
pool size, available cores, the root path, and whether the run is cold or warm.

**Phase split**, each timed:
- **walk**: `FileList::build` ms + file count;
- **parse**: `build_index` ms (wall) **and** the sum of per-file times (CPU) → report
  **parallelism = CPU-sum / wall** against the core count (a low ratio means the
  parallelism is not working — that is a finding);
- **assembly**: the serial `for` loop ms;
- **total**, and **`total − walk`** labelled explicitly as **"what a warm persisted index
  would cost to start"** — this is the number that decides whether persistence is worth
  building.

**Per-file detail** (the "fine-grained" ask):
- per file: `read_ms`, `extract_ms`, bytes, symbol count, language;
- **top N slowest by `extract_ms`** (default 25) with bytes and symbols — this reveals
  whether a few pathological files dominate (minified/generated files), which is what a size
  cap would fix;
- **top N largest by bytes**;
- a **distribution** for `extract_ms` and bytes: p50 / p90 / p99 / max;
- **throughput**: bytes/s and files/s overall and per-core.

**Breakdowns**: per-language (files, bytes, ms, symbols); files with **zero** symbols;
files whose language is **unresolved** (skipped); total symbols.

**Memory**: peak RSS (`/proc/self/status` `VmHWM` on Linux) — cheap and tells us if the
index itself is the problem.

**`--profile-out=FILE`**: write **every** per-file row as CSV (`path,lang,bytes,read_ms,
extract_ms,symbols`) so they can sort and filter it themselves. This matters more than the
top-N: let them slice it.

**A repeat run**: `--profile-repeat=N` (default 1) so a warm-page-cache number can be
obtained. Label each run; do not average cold and warm together.

**Stretch (only if it stays cheap and honest)**: split `extract_ms` into tree-sitter
**parse** vs **query execution** — this needs an additive timed variant in
`crates/redline-syntax/src/queries.rs` (`extract_all` is the hot path; add a sibling that
takes an optional timings out-param, leaving the hot path unchanged). If that requires
contorting the production path, **skip it and say so** — the read/extract split plus the
top-N is already enough to decide.

## Constraints

- **Do not change indexing behaviour.** This is instrumentation: no new skipping, no caps,
  no tuning. If the profiler's own instrumentation would perturb the measurement, say how.
- Keep the collector **out of the hot path** when disabled (no per-file allocation when
  profiling is off).
- **Two other lanes are active** (one under review in slot 1, one implementing in slot 2).
  Do not touch `src/ui/`, `src/theme.rs`, or `src/app/store/` beyond reading. **You own
  `src/main.rs` and your new module** — no other lane is editing those.

## Files

New module (e.g. `src/index_profile.rs`) + `src/main.rs` (flag + usage) + `README.md` (a
diagnostics line). Optionally `crates/redline-syntax/src/queries.rs` for the stretch.

## Verification (light — this is not a rendering change)

- `cargo build`; `cargo test --workspace` — reconcile against the **current** baseline
  (measure it) and account for every change.
- `cargo clippy --workspace --all-targets -- -D warnings` (`${PIPESTATUS[0]}`).
- **Run it for real and paste the actual output**:
  (a) on a small fixture (a tempdir with a couple of files) to prove the plumbing;
  (b) **on this repo itself** (`cargo run --release -- --index-profile=.`) — it is a real
      multi-crate tree and the report must be self-consistent (files ≈ what `git ls-files`
      reports, non-zero symbols, a sane parallelism ratio);
  (c) confirm it **exits without a tty** (e.g. `| cat` / redirected to a file);
  (d) confirm `--notes=plain` still works and an unknown arg still hard-errors.
- **No PTY battery is required** (nothing renders) — but say so explicitly in your report
  rather than silently skipping, and note that the standing rule is `gate.sh full` for
  rendering changes.
- Report: the invocation, the exact report output from (b), the per-file CSV columns, what
  you measured for the parse-vs-query stretch (or why you skipped it), and anything the
  instrumentation itself could be skewing.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first (swap is
  currently exhausted — keep the battery light and use `--release` for the real runs).
