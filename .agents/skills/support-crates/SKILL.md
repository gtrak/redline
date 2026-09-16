---
name: support-crates
description: >-
  Small utility crates pinned in redline's Cargo.toml: serde/serde_json/
  toml (config + cache), anyhow/thiserror (errors), dirs (standard paths),
  rayon (background symbol indexer), and dev deps tempfile (git test repos)
  + insta (magit section-tree snapshots). Verified on docs.rs.
---

# support-crates

Utility crates around the UI/async core. Each section is short on purpose;
API facts verified against docs.rs for the pinned versions.

## Version

- serde 1.0.229 (feature `derive`) · serde_json 1.0.151 · toml 1.1.6
- anyhow 1.0.104 · thiserror 2.0.20 (v2 line) · dirs 7.0.0 · rayon 1.12.0
- dev: tempfile 3.27.0 · insta 1.48.0

## Core API

### serde + serde_json (1.0.229 / 1.0.151)

- `#[derive(Serialize, Deserialize)]` — gated on serde's `derive` feature
  (enabled in Cargo.toml).
- `serde_json::from_str::<T>(s)`, `from_slice`, `from_reader`;
  `to_string(&t)`, `to_string_pretty`, `to_vec`, `to_writer`;
  `serde_json::Result<T>` for typed errors.
- `serde_json::Value` (Null/Bool/Number/String/Array/Object) + `json!`
  macro; indexing `v["key"]` returns `&Value` (Null on miss).

### toml 1.1.6 (the 1.x line — not 0.8)

- `toml::from_str::<T>(s)`, `from_slice(bytes)` (new in 1.x),
  `to_string(&t)`, `to_string_pretty`; `toml::toml!` macro.
- `toml::Table` is a *type alias*: `pub type Table = Map<String, Value>`
  (feature `serde`); lexicographic key order, `preserve_order` feature for
  source order. `Table::try_from(t)` / `table.try_into::<T>()`.
- `toml::Value` enum: String, Integer(i64), Float(f64), Boolean, Datetime,
  Array, Table; `value.as_str()` etc.
- vs 0.8 (checked against 0.8.23 docs): 0.8's `Table` lacked `Serialize`
  and `Deserialize` impls (1.x has both); 0.8 has no `from_slice`; 1.x
  makes serde an optional (default-on) feature and `Datetime` is toml's
  own `toml::value::Datetime` (0.8 re-exported the `toml_datetime` type).

### anyhow 1.0.104

- `anyhow::Error`, `anyhow::Result<T>`; `?` propagates any
  `std::error::Error`.
- `Context` trait: `result.context("msg")` /
  `result.with_context(|| format!(...))`.
- `bail!("...")` (early `Err` return), `ensure!(cond, "...")`,
  `anyhow!`/`format_err!` for ad-hoc errors.

### thiserror 2.0.20

- `#[derive(thiserror::Error, Debug)]` on enums/structs;
  `#[error("...")]` per variant, `{0}`/`{field}` interpolation.
- `#[from]` generates `From` + `source`; `#[error(transparent)]` for
  pass-through variants.

### dirs 7.0.0

- Free functions returning `Option<PathBuf>`: `config_dir()`,
  `cache_dir()`, `home_dir()`, `data_dir()`, ... (18 total).
- redline: config = `config_dir()/redline/config.toml`,
  cache = `cache_dir()/redline/`.

### rayon 1.12

- `use rayon::prelude::*` → `slice.par_iter()`, `vec.into_par_iter()`,
  `par_iter_mut()`; `map`/`filter`/`for_each`/`collect`/`try_fold`.
- `par_bridge` is a *trait method* in 1.12, not a free function:
  `use rayon::iter::ParallelBridge; iter.par_bridge()` (blanket impl for
  `T: Iterator<Item: Send> + Send`). The old `rayon::iter::par_bridge(iter)`
  free function is gone.
- `rayon::scope(|s| s.spawn(|| ...))` for tasks borrowing the stack;
  `rayon::spawn` for `'static` tasks; `ThreadPoolBuilder` to cap threads.

### dev: tempfile 3.27

- `tempfile::tempdir() -> Result<TempDir>`; `dir.path()`, `dir.close()`;
  `Builder` for custom options (prefix, ...). Deleted on drop.
- Early-drop pitfall: pass `&dir` to `AsRef<Path>` params, never move the
  `TempDir` in.

### dev: insta 1.48

- `insta::assert_snapshot!(value)` (Display/String) and
  `assert_debug_snapshot!(value)`; inline form: `assert_snapshot!(v, @"...")`.
- `assert_json_snapshot!` needs the `json` feature; redactions need the
  `redactions` feature (`dynamic_redaction`, `sorted_redaction`,
  `rounded_redaction`).
- Failing runs write `.snap.new` (default `INSTA_UPDATE=auto`); review
  with `cargo insta review`.

## Usage in redline

- config (issues 01/02): `toml::from_str` into a `#[derive(Deserialize)]`
  struct from `~/.config/redline/config.toml`; defaults on missing file.
- cache: symbol index + search caches in `~/.cache/redline/` (dirs);
  `serde_json` for cache payloads.
- errors: `anyhow` + `Context` at app level (main, config load, watcher
  setup); `thiserror` for library-style error enums (e.g. indexer) that
  get wrapped into `anyhow`.
- indexer (issue 05): `ignore::Walk` → `par_bridge()` → parallel
  tree-sitter symbol extraction; `scope` if tasks borrow the walk.
- tests: `tempfile::tempdir()` for git2 test repos; `insta`
  `assert_snapshot!` for magit section-tree data models (issues 07/08).

## Gotchas

- toml 1.x: `from_str`/`to_string` are feature-gated (`parse`+`serde`,
  `display`+`serde` — both default-on); 0.8-era code that passed `Table`
  to serializers will now work (Table serializes in 1.x).
- rayon `par_bridge` output is unordered — sort before snapshotting.
- insta: enable the features you use (`json`, `redactions`) in
  `[dev-dependencies]`; defaults still pull serde for the toml/yaml macros.
- `TempDir` cleanup relies on the destructor — keep it alive for the whole
  test; a panic before drop leaks the directory.
