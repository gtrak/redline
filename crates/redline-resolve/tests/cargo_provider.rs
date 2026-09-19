//! End-to-end tests for [`redline_resolve::CargoProvider`]: tempdir
//! workspaces, a real crates.io fetch, and the offline-refusal path.
//!
//! These run real cargo subprocesses. The "fetched crate" tests use a fresh
//! `CARGO_HOME` so the registry crate must be downloaded (fetch exercised).
//! The offline test uses an empty home + `--offline` so no network is needed.

use std::io::Write;
use std::path::PathBuf;

use redline_resolve::{CargoProvider, ResolvedSource, SymbolContext, ToolingProvider};

/// Write a file under `root`, creating parent dirs.
fn write_file(root: &std::path::Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(contents.as_bytes()).unwrap();
}

fn ctx(root: &std::path::Path, symbol: &str) -> SymbolContext {
    SymbolContext {
        workspace_root: root.to_path_buf(),
        symbol: symbol.to_string(),
        from_file: PathBuf::from("src/lib.rs"),
        scope: Vec::new(),
    }
}

/// As [`ctx`], but with a scope hint (007-03: the app's use-declaration
/// path for a bare symbol).
fn ctx_with_scope(root: &std::path::Path, symbol: &str, scope: &[&str]) -> SymbolContext {
    SymbolContext {
        workspace_root: root.to_path_buf(),
        symbol: symbol.to_string(),
        from_file: PathBuf::from("src/lib.rs"),
        scope: scope.iter().map(|s| s.to_string()).collect(),
    }
}

/// A single-crate cargo workspace whose root package is itself a member, with
/// no external dependencies (so no network is required).
fn make_local_ws(tmp: &tempfile::TempDir) {
    let root = tmp.path();
    write_file(
        root,
        "member/Cargo.toml",
        "[package]\nname = \"member\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write_file(
        root,
        "member/src/lib.rs",
        "//! member crate\npub fn member_fn() -> u32 {\n    42\n}\n",
    );
    write_file(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"member\"]\n",
    );
}

#[test]
fn workspace_member_resolves_internal() {
    let tmp = tempfile::tempdir().unwrap();
    make_local_ws(&tmp);
    // Fresh (empty) CARGO_HOME: no deps means no network needed.
    let home = tempfile::tempdir().unwrap();

    let provider = CargoProvider::new().with_cargo_home(home.path());
    let res = provider
        .resolve(&ctx(tmp.path(), "member::member_fn"))
        .expect("workspace member should resolve");

    assert!(!res.external, "a workspace member must be internal");
    assert_eq!(res.file.file_name().unwrap(), "lib.rs");
    assert!(
        res.file.starts_with(tmp.path().join("member")),
        "file {:?} not under the member crate",
        res.file
    );
    // The source root is the member crate root (parent of its Cargo.toml).
    assert_eq!(res.source_root, tmp.path().join("member"));
    // `pub fn member_fn` is on line 2 (line 1 is the innerdoc comment).
    assert_eq!(res.line, Some(2));
}

/// A workspace that depends on `anyhow` from crates.io.
fn make_dep_ws(tmp: &tempfile::TempDir) {
    let root = tmp.path();
    write_file(
        root,
        "Cargo.toml",
        "[package]\nname = \"ws_dep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\nanyhow = \"1.0\"\n\n[workspace]\n",
    );
    write_file(root, "src/lib.rs", "pub fn f() {}\n");
}

#[test]
fn fetched_dep_resolves_to_registry_external() {
    let tmp = tempfile::tempdir().unwrap();
    make_dep_ws(&tmp);
    // Fresh CARGO_HOME → anyhow must be fetched from crates.io.
    let home = tempfile::tempdir().unwrap();

    let provider = CargoProvider::new().with_cargo_home(home.path());
    let res = provider
        .resolve(&ctx(tmp.path(), "anyhow::Error"))
        .expect("fetched dep should resolve");

    assert!(res.external, "a registry dep must be external");
    assert_eq!(res.file.file_name().unwrap(), "lib.rs");
    // Crate root lives under the CARGO_HOME registry, not the workspace.
    assert!(
        res.source_root.starts_with(home.path()),
        "source_root {:?} not under CARGO_HOME {:?}",
        res.source_root,
        home.path()
    );
    assert!(!res.source_root.starts_with(tmp.path()));
    // The crate root dir is named `anyhow-<version>`.
    let root_name = res
        .source_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap();
    assert!(
        root_name.starts_with("anyhow-"),
        "unexpected crate root name: {root_name}"
    );
}

#[test]
fn fetched_dep_locates_error_definition() {
    let tmp = tempfile::tempdir().unwrap();
    make_dep_ws(&tmp);
    let home = tempfile::tempdir().unwrap();

    let provider = CargoProvider::new().with_cargo_home(home.path());
    let res: ResolvedSource = provider
        .resolve(&ctx(tmp.path(), "anyhow::Error"))
        .expect("fetched dep should resolve");

    assert!(res.external);
    let line = res.line.expect("a definition line should be located");
    // The located line is `anyhow`'s `pub struct Error {` definition.
    let contents = std::fs::read_to_string(&res.file).unwrap();
    let line_text = contents
        .lines()
        .nth(line as usize - 1)
        .expect("line within file range");
    assert!(
        line_text.contains("struct Error"),
        "located line {line} does not define Error: {line_text:?}"
    );
}

#[test]
fn offline_refusal_is_clean_error() {
    let tmp = tempfile::tempdir().unwrap();
    make_dep_ws(&tmp);
    // Empty CARGO_HOME + offline: `anyhow` cannot be resolved without a
    // network fetch, so the provider must fail with a clean offline error
    // (and never attempt a download).
    let home = tempfile::tempdir().unwrap();

    let provider = CargoProvider::new().with_cargo_home(home.path()).offline();
    let err = provider
        .resolve(&ctx(tmp.path(), "anyhow::Error"))
        .expect_err("offline + uncached dep must fail");
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("offline"),
        "error should mention offline refusal: {msg}"
    );
}

#[test]
fn unknown_crate_is_clean_error() {
    let tmp = tempfile::tempdir().unwrap();
    make_dep_ws(&tmp);
    let home = tempfile::tempdir().unwrap();

    let provider = CargoProvider::new().with_cargo_home(home.path());
    let err = provider
        .resolve(&ctx(tmp.path(), "does_not_exist::Thing"))
        .expect_err("unknown crate must fail cleanly");
    let msg = err.to_string();
    assert!(
        msg.contains("does_not_exist"),
        "error should name the crate: {msg}"
    );
}

#[test]
fn bare_symbol_needs_scope_info() {
    let tmp = tempfile::tempdir().unwrap();
    make_local_ws(&tmp);
    let home = tempfile::tempdir().unwrap();

    let provider = CargoProvider::new().with_cargo_home(home.path());
    let err = provider
        .resolve(&ctx(tmp.path(), "member_fn"))
        .expect_err("a bare symbol has no crate path");
    assert!(
        err.to_string().to_lowercase().contains("scope info"),
        "error should say scope info is needed: {}",
        err
    );
}

/// 007-03: a BARE symbol + the scope hint (the `use` path) resolves a
/// workspace member through the same machinery as `member::member_fn`
/// (the path-shaped twin above pins that landing).
#[test]
fn bare_symbol_with_scope_resolves_workspace_member() {
    let tmp = tempfile::tempdir().unwrap();
    make_local_ws(&tmp);
    let home = tempfile::tempdir().unwrap();

    let provider = CargoProvider::new().with_cargo_home(home.path());
    let res = provider
        .resolve(&ctx_with_scope(tmp.path(), "member_fn", &["member", "member_fn"]))
        .expect("bare + scope should resolve");
    assert!(!res.external, "a workspace member must be internal");
    assert_eq!(res.file.file_name().unwrap(), "lib.rs");
    assert_eq!(res.line, Some(2));
}
