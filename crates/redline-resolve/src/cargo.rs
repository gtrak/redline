//! Rust/cargo [`ToolingProvider`]: resolve a workspace-missed symbol to its
//! concrete source file via cargo.
//!
//! Flow: parse the crate name from the use-path → `cargo metadata` on the
//! workspace root → pick the package by name → its `manifest_path` gives the
//! source dir (a workspace member lives inside the workspace; a registry dep
//! lives under `$CARGO_HOME/registry/src/<hash>/<name>-<version>/`). If the
//! registry source is absent we `cargo fetch` (sanctioned) and re-locate. Then
//! a plain fs walk + content scan finds the file and a best-effort line for
//! the item's definition.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use crate::{crate_from_symbol, run_with_timeout, scope_qualified, ResolvedSource, SymbolContext, ToolingProvider};

/// Timeout for `cargo metadata` (local, but can be slow on large workspaces).
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);
/// Timeout for `cargo fetch` (network download).
const FETCH_TIMEOUT: Duration = Duration::from_secs(120);

/// Resolve Rust symbols to real source via cargo metadata / fetch.
pub struct CargoProvider {
    /// Explicit `CARGO_HOME` for the cargo subprocesses. `None` = inherit the
    /// ambient environment (i.e. the user's default `~/.cargo`). Tests use a
    /// fresh tempdir here to force a real fetch in isolation.
    cargo_home: Option<PathBuf>,
    /// When `true`, run cargo with `--offline` and never fetch. A registry
    /// crate whose source is not already cached then fails with a clean
    /// offline-refusal error.
    offline: bool,
    /// Cargo binary to invoke (overridable for testing).
    cargo_bin: String,
}

impl Default for CargoProvider {
    fn default() -> Self {
        Self {
            cargo_home: None,
            offline: false,
            cargo_bin: "cargo".to_string(),
        }
    }
}

impl CargoProvider {
    /// Online provider using the ambient `CARGO_HOME` (the default).
    pub fn new() -> Self {
        Self::default()
    }

    /// Point the cargo subprocesses at a specific `CARGO_HOME` (e.g. a fresh
    /// tempdir so a registry crate must be fetched).
    pub fn with_cargo_home(mut self, home: impl Into<PathBuf>) -> Self {
        self.cargo_home = Some(home.into());
        self
    }

    /// Never fetch: run cargo `--offline` and refuse on an unfetched registry
    /// crate (clean error).
    pub fn offline(mut self) -> Self {
        self.offline = true;
        self
    }

    /// Invoke a specific cargo binary (mainly for tests).
    pub fn with_cargo_bin(mut self, bin: impl Into<String>) -> Self {
        self.cargo_bin = bin.into();
        self
    }

    fn cargo_home(&self) -> Option<&Path> {
        self.cargo_home.as_deref()
    }

    fn with_home(&self, cmd: &mut Command) {
        if let Some(home) = self.cargo_home() {
            cmd.env("CARGO_HOME", home);
        }
    }

    fn run_metadata(&self, workspace_root: &Path) -> anyhow::Result<CargoMetadata> {
        let mut cmd = Command::new(&self.cargo_bin);
        cmd.current_dir(workspace_root)
            .arg("metadata")
            .arg("--format-version")
            .arg("1");
        if self.offline {
            cmd.arg("--offline");
        }
        self.with_home(&mut cmd);
        // `cargo metadata` prints progress to stderr and JSON to stdout.
        let out =
            run_with_timeout(cmd, METADATA_TIMEOUT).map_err(|e| {
                anyhow::anyhow!("failed to run `cargo metadata` in {}: {e}", workspace_root.display())
            })?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let msg = if self.offline {
                format!(
                    "cargo metadata failed offline for workspace {} (registry crate not cached; \
                     run `cargo fetch` or retry online to populate it): {stderr}",
                    workspace_root.display()
                )
            } else {
                format!(
                    "cargo metadata failed for workspace {}: {stderr}",
                    workspace_root.display()
                )
            };
            return Err(anyhow::anyhow!("{msg}"));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        serde_json::from_str(&stdout)
            .map_err(|e| anyhow::anyhow!("could not parse cargo metadata JSON: {e}"))
    }

    /// Fetch missing registry sources (sanctioned operator directive for this
    /// issue). Runs in the workspace so the whole graph is fetched.
    fn run_fetch(&self, workspace_root: &Path) -> anyhow::Result<()> {
        let mut cmd = Command::new(&self.cargo_bin);
        cmd.current_dir(workspace_root).arg("fetch");
        self.with_home(&mut cmd);
        let out =
            run_with_timeout(cmd, FETCH_TIMEOUT).map_err(|e| anyhow::anyhow!("failed to run `cargo fetch`: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(anyhow::anyhow!(
                "`cargo fetch` failed for workspace {}: {stderr}",
                workspace_root.display()
            ));
        }
        Ok(())
    }

    /// Locate the concrete source for a symbol, end to end.
    fn resolve_cargo(&self, ctx: &SymbolContext) -> anyhow::Result<ResolvedSource> {
        let workspace_root = &ctx.workspace_root;
        if !workspace_root.join("Cargo.toml").exists() {
            anyhow::bail!(
                "no Cargo.toml under workspace root {}; not a cargo project",
                workspace_root.display()
            );
        }

        // Split the use-path into (crate, item). A BARE symbol with a
        // scope hint (007-03: the app's use-declaration path) normalizes
        // to `crate::item` UP FRONT so the rest of the flow is identical
        // to a path-shaped symbol — the SAME locate_source_dir /
        // locate_in_pkg machinery, no parallel path. No hint (empty
        // scope) keeps today's bail, byte-for-byte.
        let qualified = scope_qualified("::", &ctx.symbol, &ctx.scope);
        let symbol = qualified.as_deref().unwrap_or(&ctx.symbol);
        let crate_name = crate_from_symbol(symbol).ok_or_else(|| {
            anyhow::anyhow!("cannot parse a crate name from symbol `{}`", ctx.symbol)
        })?;
        let segments: Vec<&str> = symbol.split("::").filter(|s| !s.is_empty()).collect();
        if segments.len() < 2 {
            anyhow::bail!(
                "bare symbol `{}` has no crate path; resolving it to a crate \
                 needs scope info (tree-sitter) not yet provided by the app",
                ctx.symbol
            );
        }
        // The hint's item is its LAST segment (`use serde::de::Deserialize`
        // → `Deserialize`); a path-shaped symbol keeps the historical
        // second-segment rule.
        let item = match &qualified {
            Some(_) => *segments.last().unwrap(),
            None => segments[1],
        };

        // metadata → package → crate root; fetch + re-locate if a registry
        // source is missing.
        let (pkg, external) = self.locate_source_dir(workspace_root, crate_name)?;
        let crate_root = source_dir_for(&pkg);

        // Find the file (and best-effort line) where `item` is defined, scanning
        // the crate's source root(s) (lib/bin target dir, then src/, then root).
        let (file, line) = locate_in_pkg(&pkg, item).ok_or_else(|| {
            anyhow::anyhow!(
                "crate `{crate_name}` located at {} but no definition of `{item}` \
                 was found in its source",
                crate_root.display()
            )
        })?;

        Ok(ResolvedSource {
            file,
            source_root: crate_root,
            external,
            line: Some(line),
        })
    }

    /// Given a crate name, find its package in the workspace's cargo graph and
    /// report whether it lives outside the workspace, fetching the registry
    /// source if missing. Returns the chosen (owned) package.
    fn locate_source_dir(
        &self,
        workspace_root: &Path,
        crate_name: &str,
    ) -> anyhow::Result<(Pkg, bool)> {
        let canonical_root = std::fs::canonicalize(workspace_root)
            .map_err(|e| anyhow::anyhow!("cannot canonicalize {}: {e}", workspace_root.display()))?;
        let meta = self.run_metadata(workspace_root)?;
        let pkg = pick_package(&meta.packages, crate_name, &canonical_root).ok_or_else(|| {
            anyhow::anyhow!(
                "crate `{crate_name}` is not in the workspace's cargo graph (workspace {})",
                workspace_root.display()
            )
        })?;
        let pkg = pkg.clone();
        let source_dir = source_dir_for(&pkg);
        let external = !source_dir.starts_with(&canonical_root);

        // A registry source is normally extracted by `cargo metadata`; if it is
        // nonetheless absent, fetch and re-locate (code reality wins).
        if !source_dir.exists() {
            if self.offline {
                anyhow::bail!(
                    "crate `{crate_name}` source dir {} is missing and offline mode \
                     refuses to fetch",
                    source_dir.display()
                );
            }
            self.run_fetch(workspace_root)?;
            let meta2 = self.run_metadata(workspace_root)?;
            let pkg2 = pick_package(&meta2.packages, crate_name, &canonical_root)
                .ok_or_else(|| anyhow::anyhow!("crate `{crate_name}` still not found after `cargo fetch`"))?
                .clone();
            let source_dir = source_dir_for(&pkg2);
            if !source_dir.exists() {
                anyhow::bail!(
                    "crate `{crate_name}` source dir {} still missing after `cargo fetch`",
                    source_dir.display()
                );
            }
            return Ok((pkg2, external));
        }
        Ok((pkg, external))
    }
}

fn source_dir_for(pkg: &Pkg) -> PathBuf {
    Path::new(&pkg.manifest_path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(&pkg.manifest_path))
}

impl ToolingProvider for CargoProvider {
    fn name(&self) -> &'static str {
        "rust"
    }

    fn languages(&self) -> &'static [&'static str] {
        &["rust"]
    }

    fn resolve(&self, ctx: &SymbolContext) -> anyhow::Result<ResolvedSource> {
        self.resolve_cargo(ctx)
    }
}

// ── package selection ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CargoMetadata {
    packages: Vec<Pkg>,
    #[serde(default)]
    #[allow(dead_code)]
    workspace_root: PathBuf,
}

#[derive(Deserialize, Clone)]
struct Pkg {
    name: String,
    version: String,
    /// `null` for path/workspace packages; `registry+…` for crates.io deps.
    #[serde(default)]
    source: Option<String>,
    manifest_path: String,
    #[serde(default)]
    targets: Vec<Target>,
}

#[derive(Deserialize, Clone)]
struct Target {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    kind: Vec<String>,
    #[serde(default)]
    src_path: Option<String>,
}

/// Pick the package named `crate_name`. Prefer a workspace member (source
/// `None`); else the highest version among registry/path packages.
fn pick_package<'a>(
    packages: &'a [Pkg],
    crate_name: &str,
    workspace_root: &Path,
) -> Option<&'a Pkg> {
    let matches: Vec<&'a Pkg> = packages.iter().filter(|p| p.name == crate_name).collect();
    if matches.is_empty() {
        return None;
    }
    let members: Vec<&'a Pkg> = matches.iter().copied().filter(|p| p.source.is_none()).collect();
    if !members.is_empty() {
        // Prefer a workspace member; if several share the name, prefer the one
        // under the workspace root, else the first.
        return members
            .iter()
            .copied()
            .find(|p| Path::new(&p.manifest_path).starts_with(workspace_root))
            .or_else(|| members.first().copied());
    }
    // No workspace member: choose the highest version among registry/external.
    matches
        .iter()
        .copied()
        .max_by(|a, b| version_key(&a.version).cmp(&version_key(&b.version)))
}

fn version_key(v: &str) -> Vec<u64> {
    v.split('.')
        .map(|s| s.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
        .map(|s| s.parse::<u64>().unwrap_or(0))
        .collect()
}

// ── symbol-in-file locate (plain fs walk + content scan) ─────────────────────

/// Walk `root` for `.rs` files, skipping `target/` and dot-dirs, in a
/// deterministic (sorted) order.
fn walk_rs_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "target" || name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Best-effort file+line locate for a definition of `item`, scanning the
/// crate's likely source roots in order: the lib target's source dir, the bin
/// target's source dir, `src/`, then the crate root.
fn locate_in_pkg(pkg: &Pkg, item: &str) -> Option<(PathBuf, u32)> {
    let crate_root = source_dir_for(pkg);
    let mut roots: Vec<PathBuf> = Vec::new();
    for kind in ["lib", "bin"] {
        if let Some(src_root) = pkg
            .targets
            .iter()
            .find(|t| t.kind.iter().any(|k| k == kind))
            .and_then(|t| t.src_path.as_ref())
            .and_then(|sp| Path::new(sp).parent())
        {
            roots.push(src_root.to_path_buf());
        }
    }
    roots.push(crate_root.join("src"));
    roots.push(crate_root);
    let mut seen = std::collections::HashSet::new();
    for root in roots {
        if !seen.insert(root.clone()) {
            continue;
        }
        if let Some(r) = locate_item(&root, item) {
            return Some(r);
        }
    }
    None
}

/// Best-effort line locate for a definition of `item` under `source_dir`.
/// Returns `(file, 1-based line)` of the strongest definition-shaped match.
fn locate_item(source_dir: &Path, item: &str) -> Option<(PathBuf, u32)> {
    let files = walk_rs_files(source_dir);
    let mut best: Option<(PathBuf, u32, u8)> = None; // (file, line, priority)
    for file in &files {
        let contents = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for (i, line) in contents.lines().enumerate() {
            if let Some(kind) = line_defines_item(line, item) {
                let prio = kind_priority(kind);
                let cand = (file.clone(), (i + 1) as u32, prio);
                match &best {
                    None => best = Some(cand),
                    Some((_, _, bp)) => {
                        if prio > *bp {
                            best = Some(cand);
                        }
                    }
                }
            }
        }
    }
    best.map(|(f, l, _)| (f, l))
}

/// Priority of a definition kind: a real item definition beats an `impl`
/// block (which only references the type).
fn kind_priority(kind: &str) -> u8 {
    match kind {
        "struct" | "enum" | "fn" | "trait" | "type" | "const" | "mod" => 3,
        "impl" => 1,
        _ => 0,
    }
}

/// Does `line` define an item named `item` (a definition-shaped match)?
/// Returns the item keyword when it does.
///
/// Heuristic: strip generic `<…>` lists, find `item` as a whole word, and
/// require the immediately-preceding token to be an item keyword
/// (`struct/enum/fn/trait/type/const/mod/impl`). This matches
/// `pub struct Error`, `fn greet`, `impl<T> Foo` while rejecting
/// `pub use crate::Error;`, `fn f() -> Error`, and `impl Display for Error`.
fn line_defines_item(line: &str, item: &str) -> Option<&'static str> {
    // Comment lines are never definitions (covers `//`, `///`, `//!`).
    if line.trim_start().starts_with("//") {
        return None;
    }
    let stripped = strip_generics(line);
    let mut search_from = 0usize;
    loop {
        let rest = &stripped[search_from..];
        let rel = rest.find(item)?;
        let idx = search_from + rel;
        let end = idx + item.len();
        let before_ok = char_at(&stripped, idx.checked_sub(1)?)
            .is_none_or(|c| !is_ident_char(c));
        let after_ok = char_at(&stripped, end).is_none_or(|c| !is_ident_char(c));
        if before_ok && after_ok {
            let prefix = &stripped[..idx];
            let last = prefix
                .rsplit(char::is_whitespace)
                .find(|s| !s.is_empty())
                .unwrap_or("");
            return match last {
                "struct" => Some("struct"),
                "enum" => Some("enum"),
                "fn" => Some("fn"),
                "trait" => Some("trait"),
                "type" => Some("type"),
                "const" => Some("const"),
                "mod" => Some("mod"),
                "impl" => Some("impl"),
                _ => None,
            };
        }
        search_from = idx + 1;
    }
}

/// Remove balanced `<…>` generic parameter lists (char-based; `->` and lone
/// `>` are left intact because there is no matching `<`).
fn strip_generics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth: i32 = 0;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

fn char_at(s: &str, i: usize) -> Option<char> {
    s.char_indices().find(|(idx, _)| *idx == i).map(|(_, c)| c)
}

fn is_ident_char(c: char) -> bool {
    // C15: mirrors the redline crate's single word-char rule —
    // `redline::model::buffer::is_word_char` (Unicode alphanumeric or `_`).
    // redline-resolve has no dependency on redline, so the rule is
    // duplicated here rather than imported; keep the two in sync.
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_defines_struct_and_fn() {
        assert_eq!(line_defines_item("pub struct Error {", "Error"), Some("struct"));
        assert_eq!(
            line_defines_item("pub fn greet(name: &str) -> String {", "greet"),
            Some("fn")
        );
        assert_eq!(line_defines_item("pub enum Kind {", "Kind"), Some("enum"));
        assert_eq!(line_defines_item("trait Foo {", "Foo"), Some("trait"));
        assert_eq!(line_defines_item("pub type Result<T> = T;", "Result"), Some("type"));
        assert_eq!(line_defines_item("pub const LIMIT: u32 = 8;", "LIMIT"), Some("const"));
    }

    #[test]
    fn line_defines_impl_with_generics() {
        assert_eq!(line_defines_item("impl<T> Foo {", "Foo"), Some("impl"));
        assert_eq!(line_defines_item("impl Foo {", "Foo"), Some("impl"));
    }

    #[test]
    fn line_defines_rejects_non_definitions() {
        // re-export / usage / type-as-generic-arg are not definitions
        assert_eq!(line_defines_item("pub use crate::Error;", "Error"), None);
        assert_eq!(line_defines_item("pub fn f() -> Error {", "Error"), None);
        assert_eq!(line_defines_item("impl Display for Error {", "Error"), None);
        assert_eq!(
            line_defines_item("pub type Result<T, E = Error> = T;", "Error"),
            None
        );
        // substring of a longer identifier is not a whole word
        assert_eq!(line_defines_item("pub struct ErrorKind {", "Error"), None);
        assert_eq!(line_defines_item("pub struct Error {", "ErrorKind"), None);
        // comment lines are never definitions
        assert_eq!(line_defines_item("// fn helper() {", "helper"), None);
        assert_eq!(line_defines_item("/// fn main() {", "main"), None);
        assert_eq!(line_defines_item("//! fn main() {", "main"), None);
    }

    #[test]
    fn strip_generics_keeps_arrow() {
        assert_eq!(strip_generics("fn f<T>() -> Vec<u32> {"), "fn f() -> Vec {");
        assert_eq!(strip_generics("impl<T> Foo {"), "impl Foo {");
    }

    /// C15: identifier-constituency mirrors the redline crate's Unicode
    /// word-char rule (`redline::model::buffer::is_word_char`) — non-ASCII
    /// letters are identifier characters, so they block a whole-word
    /// boundary just like ASCII letters.
    #[test]
    fn ident_char_is_unicode_aware() {
        assert!(is_ident_char('é'), "accented letter");
        assert!(is_ident_char('漢'), "CJK letter");
        assert!(is_ident_char('_'));
        assert!(!is_ident_char('-'));
        assert!(!is_ident_char(' '));
        // Whole-word pin: `greet` inside `greeté` is NOT a definition of
        // `greet` (the `é` is an identifier char, not a boundary).
        assert_eq!(line_defines_item("fn greeté() {", "greet"), None);
    }

    #[test]
    fn version_key_orders() {
        assert!(version_key("1.0.104") > version_key("1.0.2"));
        assert!(version_key("2.0") > version_key("1.9"));
    }

    #[test]
    fn locate_finds_definition_in_tempdir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("mod_a.rs"), "pub fn in_mod() {}\n").unwrap();
        std::fs::write(tmp.path().join("lib.rs"), "pub struct Target {\n    x: u32,\n}\n").unwrap();
        let (file, line) = locate_item(tmp.path(), "Target").unwrap();
        assert_eq!(file.file_name().unwrap(), "lib.rs");
        assert_eq!(line, 1);
        let (file2, _) = locate_item(tmp.path(), "in_mod").unwrap();
        assert_eq!(file2.file_name().unwrap(), "mod_a.rs");
    }

    #[test]
    fn locate_missing_item_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("lib.rs"), "pub struct Foo {}\n").unwrap();
        assert!(locate_item(tmp.path(), "NotThere").is_none());
    }

    // ── 007-03: bare symbol + scope hint (use-declaration path) ───────────

    /// A cargo workspace (tempdir) with a synthetic path-dependency crate
    /// `fakeserde` whose lib defines a `Deserialize` struct — a known, no-
    /// network stand-in for a registry crate.
    fn ws_with_fakeserde(tmp: &Path) -> PathBuf {
        std::fs::create_dir_all(tmp.join("src")).unwrap();
        std::fs::write(
            tmp.join("Cargo.toml"),
            r#"[package]
name = "wsapp"
version = "0.1.0"
edition = "2021"

[dependencies]
fakeserde = { path = "fakeserde" }
"#,
        )
        .unwrap();
        std::fs::write(tmp.join("src/main.rs"), "fn main() { fakeserde::Deserialize; }\n").unwrap();
        let dep = tmp.join("fakeserde");
        std::fs::create_dir_all(dep.join("src")).unwrap();
        std::fs::write(
            dep.join("Cargo.toml"),
            r#"[package]
name = "fakeserde"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        std::fs::write(dep.join("src/lib.rs"), "pub struct Deserialize { x: u32 }\n").unwrap();
        tmp.to_path_buf()
    }

    /// Discriminating: a BARE symbol + the use-path scope resolves through
    /// the SAME locate_in_pkg machinery as a path-shaped symbol (the
    /// path-shaped twin asserts the identical landing).
    #[test]
    fn bare_symbol_with_scope_resolves_through_locate_in_pkg() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = ws_with_fakeserde(tmp.path());
        let lib = tmp.path().join("fakeserde/src/lib.rs");

        // The bare symbol + scope hint (use fakeserde::Deserialize;).
        let ctx = SymbolContext {
            workspace_root: ws.clone(),
            symbol: "Deserialize".to_string(),
            from_file: PathBuf::from("src/main.rs"),
            scope: vec!["fakeserde".to_string(), "Deserialize".to_string()],
            language: None,
            confirm_fetch: None,
        };
        let src = CargoProvider::new().resolve(&ctx).unwrap();
        assert_eq!(src.file, lib);
        assert_eq!(src.line, Some(1));
        // A path dependency lives inside the workspace root → internal.
        assert!(!src.external);

        // Same-machinery proof: the path-shaped symbol lands identically.
        let ctx2 = SymbolContext {
            workspace_root: ws.clone(),
            symbol: "fakeserde::Deserialize".to_string(),
            from_file: PathBuf::from("src/main.rs"),
            scope: Vec::new(),
            language: None,
            confirm_fetch: None,
        };
        assert_eq!(CargoProvider::new().resolve(&ctx2).unwrap(), src);
    }

    /// An aliased import (`use fakeserde::Deserialize as D;`): the hint is
    /// the ORIGINAL path — the item comes from the hint's last segment,
    /// not the alias.
    #[test]
    fn bare_symbol_with_scope_alias_uses_original_item() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = ws_with_fakeserde(tmp.path());
        let ctx = SymbolContext {
            workspace_root: ws,
            symbol: "D".to_string(),
            from_file: PathBuf::from("src/main.rs"),
            scope: vec!["fakeserde".to_string(), "Deserialize".to_string()],
            language: None,
            confirm_fetch: None,
        };
        let src = CargoProvider::new().resolve(&ctx).unwrap();
        assert_eq!(
            src.file,
            tmp.path().join("fakeserde/src/lib.rs")
        );
        assert_eq!(src.line, Some(1));
    }

    /// Regression pin: an EMPTY scope still bails with the existing
    /// message, byte-for-byte (no hint → today's behavior).
    #[test]
    fn bare_symbol_empty_scope_still_bails() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = ws_with_fakeserde(tmp.path());
        let ctx = SymbolContext {
            workspace_root: ws,
            symbol: "Deserialize".to_string(),
            from_file: PathBuf::from("src/main.rs"),
            scope: Vec::new(),
            language: None,
            confirm_fetch: None,
        };
        let err = CargoProvider::new().resolve(&ctx).unwrap_err();
        assert!(
            err.to_string()
                .contains("bare symbol `Deserialize` has no crate path; resolving it to a crate \
                           needs scope info (tree-sitter) not yet provided by the app"),
            "err: {err}"
        );
    }

    /// Std/prelude names are NOT guessed: a bare prelude symbol with no
    /// hint must still bail, never resolve.
    #[test]
    fn prelude_name_without_hint_is_not_guessed() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = ws_with_fakeserde(tmp.path());
        let ctx = SymbolContext {
            workspace_root: ws,
            symbol: "String".to_string(),
            from_file: PathBuf::from("src/main.rs"),
            scope: Vec::new(),
            language: None,
            confirm_fetch: None,
        };
        let err = CargoProvider::new().resolve(&ctx).unwrap_err();
        assert!(err.to_string().contains("needs scope info"), "err: {err}");
    }
}
