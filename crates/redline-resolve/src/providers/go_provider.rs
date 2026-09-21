//! Go/go-modules [`ToolingProvider`]: resolve a workspace-missed Go symbol to
//! its concrete source file via the Go module cache.
//!
//! Flow: parse the package name from the dot-qualified symbol
//! (`fmt.Println` → package `fmt`, item `Println`) → find the matching module
//! in the workspace's `go.mod` require block → get the version (go.mod,
//! falling back to go.sum) → compute the module cache dir (`go env
//! GOMODCACHE` or `$HOME/go/pkg/mod` fallback) → locate the module dir
//! (case-encoded) → find the item definition in the module's `.go` files.
//!
//! **Limitations**:
//! - Package-name → module-path mapping uses the last path segment of the
//!   module. The app's tree-sitter import context (which maps package names
//!   to full module paths from the file's import block) is not yet
//!   integrated.
//! - A bare symbol (no dot) cannot be resolved, mirroring the cargo
//!   provider's bare-symbol rule.
//! - Non-local `replace` directives (module → module) fall through to the
//!   original module path; only local-path replaces are fully honored.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::{
    run_with_timeout, scope_qualified, scope_qualified_alias, ResolvedSource, SymbolContext,
    ToolingProvider,
};

/// Timeout for `go env` (fast local call).
const GO_ENV_TIMEOUT: Duration = Duration::from_secs(30);
/// Timeout for `go mod download` (network fetch).
const MOD_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);

/// Resolve Go symbols to real source via the Go module cache.
pub struct GoProvider {
    /// Explicit module cache directory. `None` = use `go env GOMODCACHE`
    /// (live) or fall back to `$HOME/go/pkg/mod`.
    mod_cache: Option<PathBuf>,
    /// Go binary to invoke (overridable for testing).
    go_bin: String,
    /// When `true`, never fetch; a missing module cache dir is a clean error.
    offline: bool,
}

impl Default for GoProvider {
    fn default() -> Self {
        Self {
            mod_cache: None,
            go_bin: "go".to_string(),
            offline: false,
        }
    }
}

impl GoProvider {
    /// Online provider using the ambient environment (the default).
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the module cache directory (for testing or custom GOMODCACHE).
    pub fn with_mod_cache(mut self, cache: impl Into<PathBuf>) -> Self {
        self.mod_cache = Some(cache.into());
        self
    }

    /// Override the go binary (mainly for tests).
    pub fn with_go_bin(mut self, bin: impl Into<String>) -> Self {
        self.go_bin = bin.into();
        self
    }

    /// Never fetch: refuse on a missing module cache dir (clean error).
    pub fn offline(mut self) -> Self {
        self.offline = true;
        self
    }

    /// Resolve the module cache directory: explicit override → `go env
    /// GOMODCACHE` → `$HOME/go/pkg/mod`.
    fn resolve_mod_cache(&self) -> anyhow::Result<PathBuf> {
        if let Some(cache) = &self.mod_cache {
            return Ok(cache.clone());
        }
        let mut go_cmd = Command::new(&self.go_bin);
        go_cmd.arg("env").arg("GOMODCACHE");
        let out =
            run_with_timeout(go_cmd, GO_ENV_TIMEOUT)
                .map_err(|e| anyhow::anyhow!("failed to run `go env GOMODCACHE`: {e}"))?;
        if out.status.success() {
            let cache = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !cache.is_empty() {
                return Ok(PathBuf::from(cache));
            }
        }
        let home =
            std::env::var("HOME").map_err(|_| anyhow::anyhow!("neither GOMODCACHE nor HOME set"))?;
        Ok(PathBuf::from(home).join("go/pkg/mod"))
    }

    /// Run `go mod download <module>` in the workspace (fetch-on-demand,
    /// sanctioned by the operator directive).
    fn run_mod_download(&self, workspace_root: &Path, module: &str) -> anyhow::Result<()> {
        let mut cmd = Command::new(&self.go_bin);
        cmd.current_dir(workspace_root)
            .arg("mod")
            .arg("download")
            .arg(module);
        let out =
            run_with_timeout(cmd, MOD_DOWNLOAD_TIMEOUT)
                .map_err(|e| anyhow::anyhow!("failed to run `go mod download {module}`: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(anyhow::anyhow!(
                "`go mod download {module}` failed in {}: {stderr}",
                workspace_root.display()
            ));
        }
        Ok(())
    }

    fn resolve_go(&self, ctx: &SymbolContext) -> anyhow::Result<ResolvedSource> {
        let workspace_root = &ctx.workspace_root;
        let go_mod_path = workspace_root.join("go.mod");
        if !go_mod_path.exists() {
            anyhow::bail!(
                "no go.mod under workspace root {}; not a Go module project",
                workspace_root.display()
            );
        }

        let go_mod = parse_go_mod(&go_mod_path)?;

        // A BARE symbol with a scope hint (007-03: the app's import
        // context) normalizes to `package.Item` up front and flows
        // through the SAME go.mod / module-cache machinery as a
        // dot-qualified symbol. No hint keeps the bail, byte-for-byte.
        // 011-08: a PATH-SHAPED symbol whose first segment is a local
        // import alias (`pe.Wrap` from `import pe "github.com/pkg/errors"`,
        // hinted as `["errors", "Wrap"]` — the real package path, item
        // included) is rewritten to the real package path and flows
        // through the SAME go.mod / module-cache machinery; identity
        // hints and non-rewrites leave the symbol's own path untouched
        // (mirrors js_provider's composition).
        let qualified = scope_qualified(".", &ctx.symbol, &ctx.scope)
            .or_else(|| scope_qualified_alias(".", &ctx.symbol, &ctx.scope));
        let symbol = qualified.as_deref().unwrap_or(&ctx.symbol);

        let (package, item) = go_package_from_symbol(symbol).ok_or_else(|| {
            anyhow::anyhow!(
                "cannot parse a package name from symbol `{}`; \
                 Go uses dot qualification (e.g. `fmt.Println` → package `fmt`) \
                 and a bare symbol needs import context (tree-sitter) not yet provided",
                ctx.symbol
            )
        })?;

        // Workspace-local package: the package name matches the last segment
        // of the workspace's own module path.
        let ws_last = go_mod
            .module_path
            .rsplit('/')
            .next()
            .unwrap_or(&go_mod.module_path);
        if package == ws_last {
            let (file, line) = locate_go_item(workspace_root, item).ok_or_else(|| {
                anyhow::anyhow!(
                    "workspace-local package `{package}` has no definition of `{item}` \
                     in workspace {}",
                    workspace_root.display()
                )
            })?;
            return Ok(ResolvedSource {
                file,
                source_root: workspace_root.to_path_buf(),
                external: false,
                line: Some(line),
            });
        }

        // External package: find the matching module in go.mod's require block.
        let module_path = go_mod
            .requires
            .iter()
            .find(|r| r.module.rsplit('/').next() == Some(package))
            .map(|r| r.module.clone())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "package `{package}` is not a required module in go.mod (workspace {})",
                    workspace_root.display()
                )
            })?;

        // Local-path replace directive: external=false, source_root = the
        // replaced path (how workspaces use local forks).
        if let Some(replacement) = go_mod
            .replaces
            .iter()
            .find(|r| r.original == module_path)
            .map(|r| r.replacement.as_str())
            .filter(|r| is_local_path(r))
        {
            let replace_path = resolve_local_path(workspace_root, replacement);
            let (file, line) = locate_go_item(&replace_path, item).ok_or_else(|| {
                anyhow::anyhow!(
                    "package `{package}` (replaced to {}) has no definition of `{item}`",
                    replace_path.display()
                )
            })?;
            return Ok(ResolvedSource {
                file,
                source_root: replace_path,
                external: false,
                line: Some(line),
            });
        }

        // Normal external module: get version from go.mod require block,
        // falling back to go.sum if the require entry has no version.
        let version = go_mod
            .requires
            .iter()
            .find(|r| r.module == module_path)
            .map(|r| r.version.clone())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                parse_go_sum(&workspace_root.join("go.sum"))
                    .ok()
                    .and_then(|sum| sum.get(&module_path).cloned())
            })
            .ok_or_else(|| {
                anyhow::anyhow!("module `{module_path}` version not found in go.mod or go.sum")
            })?;

        let mod_cache = self.resolve_mod_cache()?;
        let encoded = case_encode(&module_path);
        let module_dir = mod_cache.join(format!("{encoded}@{version}"));

        if !module_dir.exists() {
            if self.offline {
                anyhow::bail!(
                    "module `{module_path}` cache dir {} is missing and offline \
                     mode refuses to fetch",
                    module_dir.display()
                );
            }
            self.run_mod_download(workspace_root, &module_path)?;
            if !module_dir.exists() {
                anyhow::bail!(
                    "module `{module_path}` cache dir {} still missing after \
                     `go mod download`",
                    module_dir.display()
                );
            }
        }

        let (file, line) = locate_go_item(&module_dir, item).ok_or_else(|| {
            anyhow::anyhow!(
                "module `{module_path}` at {} has no definition of `{item}`",
                module_dir.display()
            )
        })?;

        Ok(ResolvedSource {
            file,
            source_root: module_dir,
            external: true,
            line: Some(line),
        })
    }
}

impl ToolingProvider for GoProvider {
    fn name(&self) -> &'static str {
        "go"
    }

    fn languages(&self) -> &'static [&'static str] {
        &["go"]
    }

    fn resolve(&self, ctx: &SymbolContext) -> anyhow::Result<ResolvedSource> {
        self.resolve_go(ctx)
    }
}

// ── go.mod parsing ───────────────────────────────────────────────────────────

/// Parsed `go.mod` contents.
struct GoMod {
    /// The workspace's own module path (from the `module` directive).
    module_path: String,
    /// Required modules (module path + version).
    requires: Vec<GoModRequire>,
    /// Replace directives (original module → replacement string).
    replaces: Vec<GoModReplace>,
}

struct GoModRequire {
    module: String,
    version: String,
}

struct GoModReplace {
    original: String,
    replacement: String,
}

/// Hand-parse a `go.mod` file. Tolerates `exclude` blocks, `go`/`toolchain`
/// version lines, and comments.
fn parse_go_mod(path: &Path) -> anyhow::Result<GoMod> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;

    let mut module_path = String::new();
    let mut requires: Vec<GoModRequire> = Vec::new();
    let mut replaces: Vec<GoModReplace> = Vec::new();

    #[derive(PartialEq)]
    enum Block {
        Require,
        Replace,
        Exclude,
    }
    let mut current_block: Option<Block> = None;

    for raw_line in content.lines() {
        let line = raw_line.trim();

        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        // Block end.
        if line == ")" {
            current_block = None;
            continue;
        }

        // Block start (may also contain inline content: `require (mod ver)`).
        if let Some(rest) = line.strip_prefix("require (") {
            current_block = Some(Block::Require);
            let inner = rest.trim_end_matches(')').trim();
            if !inner.is_empty() && let Some(req) = parse_require_line(inner) {
                requires.push(req);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("replace (") {
            current_block = Some(Block::Replace);
            let inner = rest.trim_end_matches(')').trim();
            if !inner.is_empty() && let Some(rep) = parse_replace_line(inner) {
                replaces.push(rep);
            }
            continue;
        }
        if line.starts_with("exclude (") {
            current_block = Some(Block::Exclude);
            continue;
        }

        // Block content.
        if let Some(block) = &current_block {
            match block {
                Block::Require => {
                    if let Some(req) = parse_require_line(line) {
                        requires.push(req);
                    }
                }
                Block::Replace => {
                    if let Some(rep) = parse_replace_line(line) {
                        replaces.push(rep);
                    }
                }
                Block::Exclude => {
                    // Tolerate: don't parse exclude entries.
                }
            }
            continue;
        }

        // Single-line directives.
        if let Some(rest) = line.strip_prefix("module ") {
            module_path = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("require ")
            && let Some(req) = parse_require_line(rest.trim())
        {
            requires.push(req);
        } else if let Some(rest) = line.strip_prefix("replace ")
            && let Some(rep) = parse_replace_line(rest.trim())
        {
            replaces.push(rep);
        }
        // `go <version>`, `toolchain <version>`, etc. → ignored.
    }

    Ok(GoMod {
        module_path,
        requires,
        replaces,
    })
}

/// Parse a single require entry: `<module> <version>`.
fn parse_require_line(line: &str) -> Option<GoModRequire> {
    let mut parts = line.splitn(2, char::is_whitespace);
    let module = parts.next()?.trim().to_string();
    let version = parts.next()?.trim().to_string();
    if module.is_empty() || version.is_empty() {
        return None;
    }
    Some(GoModRequire { module, version })
}

/// Parse a single replace entry: `<original> => <replacement>`.
fn parse_replace_line(line: &str) -> Option<GoModReplace> {
    let arrow = line.find("=>")?;
    let original = line[..arrow].trim().to_string();
    let replacement = line[arrow + 2..].trim().to_string();
    if original.is_empty() || replacement.is_empty() {
        return None;
    }
    Some(GoModReplace {
        original,
        replacement,
    })
}

// ── go.sum parsing ───────────────────────────────────────────────────────────

/// Parse a `go.sum` file into a map of module path → version.
///
/// Each line is `<module> <version> h1:<hash>=` or
/// `<module> <version>/go.mod h1:<hash>=`. The version is extracted from the
/// second field (stripping any `/go.mod` suffix). If a module appears on
/// multiple lines, the last occurrence wins (all lines for the same module
/// carry the same version in practice).
fn parse_go_sum(path: &Path) -> anyhow::Result<HashMap<String, String>> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;

    let mut versions: HashMap<String, String> = HashMap::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, char::is_whitespace);
        let module = parts.next().unwrap_or("");
        let version_token = parts.next().unwrap_or("");
        if module.is_empty() || version_token.is_empty() {
            continue;
        }
        // Strip `/go.mod` suffix if present.
        let version = version_token.split('/').next().unwrap_or(version_token);
        if !version.is_empty() {
            versions.insert(module.to_string(), version.to_string());
        }
    }

    Ok(versions)
}

// ── case encoding ────────────────────────────────────────────────────────────

/// Encode a module path for the Go module cache directory.
///
/// Each uppercase ASCII letter is replaced with `!` + the lowercase
/// equivalent. Lowercase letters and all other characters are unchanged.
/// For example, `github.com/USER/Repo` → `github.com/!u!s!e!r/!repo`.
///
/// Source: Go module reference (go.dev/ref/mod#canonical-import-paths) and
/// the Go source `cmd/go/internal/modfetch/fetch.go` — "uppercase letters
/// are replaced with `!` followed by the lowercase letter".
fn case_encode(module_path: &str) -> String {
    let mut out = String::with_capacity(module_path.len());
    for c in module_path.chars() {
        if c.is_ascii_uppercase() {
            out.push('!');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

// ── symbol parsing ───────────────────────────────────────────────────────────

/// Extract the package name and item from a dot-qualified Go symbol.
///
/// Go uses dot qualification: `fmt.Println` → package `fmt`, item `Println`.
/// A bare symbol (no dot) returns `None` — it needs import context
/// (tree-sitter) not yet provided by the app, mirroring the cargo provider's
/// bare-symbol rule.
fn go_package_from_symbol(symbol: &str) -> Option<(&str, &str)> {
    let (pkg, item) = symbol.split_once('.')?;
    if pkg.is_empty() || item.is_empty() {
        return None;
    }
    Some((pkg, item))
}

// ── item locate (plain fs walk + content scan) ──────────────────────────────

/// Walk `root` for `.go` files, skipping dot-dirs, in a deterministic
/// (sorted) order.
fn walk_go_files(root: &Path) -> Vec<PathBuf> {
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
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("go") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Best-effort file+line locate for a definition of `item` under `root`.
/// Returns `(file, 1-based line)` of the first definition-shaped match found
/// in sorted file order.
fn locate_go_item(root: &Path, item: &str) -> Option<(PathBuf, u32)> {
    let files = walk_go_files(root);
    for file in &files {
        let contents = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for (i, line) in contents.lines().enumerate() {
            if go_line_defines_item(line, item).is_some() {
                return Some((file.clone(), (i + 1) as u32));
            }
        }
    }
    None
}

/// Does `line` define an item named `item` in Go source?
///
/// Matches: `func X`, `func (r R) X`, `type X struct/interface`,
/// `const X`, `var X`. Rejects: comments, assignments (`x := X`),
/// and receiver-type references (`func (r X) Method` where `X` is the
/// receiver type, not the defined item).
fn go_line_defines_item(line: &str, item: &str) -> Option<&'static str> {
    let trimmed = line.trim_start();

    // Reject line comments and block-comment starts.
    if trimmed.starts_with("//") || trimmed.starts_with("/*") {
        return None;
    }

    // func X(...) — regular function.
    if let Some(rest) = trimmed.strip_prefix("func") {
        let rest = rest.trim_start();
        if rest.is_empty() {
            return None;
        }
        if rest.starts_with('(') {
            // Method: func (r R) X(...)
            // The item name follows the closing paren of the receiver.
            if let Some(close) = find_closing_paren(rest) {
                let after = rest[close + 1..].trim_start();
                return item_at_start(after, item).then_some("func");
            }
            return None;
        }
        // Regular function: func X(...)
        return item_at_start(rest, item).then_some("func");
    }

    // type X struct { ... } / type X interface { ... } / type X = ...
    if let Some(rest) = trimmed.strip_prefix("type") {
        let rest = rest.trim_start();
        return item_at_start(rest, item).then_some("type");
    }

    // const X = ... / const X Type = ...
    if let Some(rest) = trimmed.strip_prefix("const") {
        let rest = rest.trim_start();
        return item_at_start(rest, item).then_some("const");
    }

    // var X = ... / var X Type = ...
    if let Some(rest) = trimmed.strip_prefix("var") {
        let rest = rest.trim_start();
        return item_at_start(rest, item).then_some("var");
    }

    None
}

/// Check if `item` appears at the start of `s` with a word boundary
/// (the next character is not an identifier character).
fn item_at_start(s: &str, item: &str) -> bool {
    if !s.starts_with(item) {
        return false;
    }
    // The boundary is the first char AFTER `item`, read whole so a multi-byte
    // char is not truncated to a single byte (the old `get(len..len+1)` slice
    // bailed on non-ASCII and treated it as a boundary).
    match s.get(item.len()..).and_then(|r| r.chars().next()) {
        None => true, // item extends to end of string
        Some(c) => !is_ident_char(c),
    }
}

fn is_ident_char(c: char) -> bool {
    // C15: mirrors the redline crate's single word-char rule —
    // `redline::model::buffer::is_word_char` (Unicode alphanumeric or `_`).
    // redline-resolve has no dependency on redline, so the rule is
    // duplicated here rather than imported; keep the two in sync (the
    // sibling `crate::cargo::is_ident_char` does the same).
    c.is_alphanumeric() || c == '_'
}

/// Find the index of the matching closing `)` for the opening `(` at
/// position 0 of `s`. Handles nested parens.
fn find_closing_paren(s: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

// ── local-path helpers ───────────────────────────────────────────────────────

/// Check if a replace directive's replacement is a local filesystem path
/// (as opposed to another module path).
fn is_local_path(replacement: &str) -> bool {
    replacement.starts_with("./")
        || replacement.starts_with("../")
        || replacement.starts_with('/')
}

/// Resolve a local-path replacement relative to the workspace root.
fn resolve_local_path(workspace_root: &Path, path: &str) -> PathBuf {
    if path.starts_with('/') {
        PathBuf::from(path)
    } else {
        workspace_root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── case encoding ────────────────────────────────────────────────────────

    #[test]
    fn case_encode_uppercase_escaped() {
        // Source: go.dev/ref/mod#canonical-import-paths — each uppercase
        // ASCII letter is replaced with `!` + lowercase; lowercase unchanged.
        // USER → !u!s!e!r (all uppercase)
        // Repo → !repo (only R is uppercase)
        assert_eq!(
            case_encode("github.com/USER/Repo"),
            "github.com/!u!s!e!r/!repo"
        );
    }

    #[test]
    fn case_encode_lowercase_unchanged() {
        assert_eq!(case_encode("github.com/x/y"), "github.com/x/y");
    }

    #[test]
    fn case_encode_mixed_case() {
        // MyOrg: M→!m, y→y, O→!o, r→r, g→g  =>  !my!org
        // MyRepo: M→!m, y→y, R→!r, e→e, p→p, o→o  =>  !my!repo
        assert_eq!(
            case_encode("example.com/MyOrg/MyRepo"),
            "example.com/!my!org/!my!repo"
        );
    }

    #[test]
    fn case_encode_all_uppercase() {
        assert_eq!(case_encode("ABC"), "!a!b!c");
    }

    // ── symbol parsing ───────────────────────────────────────────────────────

    #[test]
    fn go_package_from_symbol_dot_qualified() {
        assert_eq!(
            go_package_from_symbol("fmt.Println"),
            Some(("fmt", "Println"))
        );
        assert_eq!(
            go_package_from_symbol("gin.Router"),
            Some(("gin", "Router"))
        );
        assert_eq!(
            go_package_from_symbol("errors.New"),
            Some(("errors", "New"))
        );
    }

    #[test]
    fn go_package_from_symbol_bare_returns_none() {
        // Bare symbol: no dot → cannot determine package without import context.
        assert_eq!(go_package_from_symbol("Println"), None);
        assert_eq!(go_package_from_symbol(""), None);
        assert_eq!(go_package_from_symbol("."), None);
        assert_eq!(go_package_from_symbol("fmt."), None);
        assert_eq!(go_package_from_symbol(".Println"), None);
    }

    // ── go.mod parser ────────────────────────────────────────────────────────

    #[test]
    fn parse_go_mod_full() {
        let tmp = tempfile::tempdir().unwrap();
        let go_mod_path = tmp.path().join("go.mod");
        std::fs::write(
            &go_mod_path,
            r#"module github.com/myorg/myrepo

go 1.21

require (
    github.com/gin-gonic/gin v1.9.1
    github.com/stretchr/testify v1.8.4
)

require github.com/pkg/errors v0.9.1

replace (
    github.com/old/lib => ./local/lib
    github.com/another/lib => github.com/new/lib v2.0.0
)

replace github.com/third/lib => ../local/third

exclude (
    github.com/excluded/lib v1.0.0
)
// a comment line
"#,
        )
        .unwrap();

        let go_mod = parse_go_mod(&go_mod_path).unwrap();

        assert_eq!(go_mod.module_path, "github.com/myorg/myrepo");

        // Requires: 2 from block + 1 single-line = 3 total.
        assert_eq!(go_mod.requires.len(), 3);
        assert_eq!(go_mod.requires[0].module, "github.com/gin-gonic/gin");
        assert_eq!(go_mod.requires[0].version, "v1.9.1");
        assert_eq!(go_mod.requires[1].module, "github.com/stretchr/testify");
        assert_eq!(go_mod.requires[1].version, "v1.8.4");
        assert_eq!(go_mod.requires[2].module, "github.com/pkg/errors");
        assert_eq!(go_mod.requires[2].version, "v0.9.1");

        // Replaces: 2 from block + 1 single-line = 3 total.
        assert_eq!(go_mod.replaces.len(), 3);
        assert_eq!(go_mod.replaces[0].original, "github.com/old/lib");
        assert_eq!(go_mod.replaces[0].replacement, "./local/lib");
        assert_eq!(go_mod.replaces[1].original, "github.com/another/lib");
        assert_eq!(
            go_mod.replaces[1].replacement,
            "github.com/new/lib v2.0.0"
        );
        assert_eq!(go_mod.replaces[2].original, "github.com/third/lib");
        assert_eq!(go_mod.replaces[2].replacement, "../local/third");
    }

    #[test]
    fn parse_go_mod_minimal() {
        let tmp = tempfile::tempdir().unwrap();
        let go_mod_path = tmp.path().join("go.mod");
        std::fs::write(&go_mod_path, "module example.com/mine\ngo 1.22\n").unwrap();

        let go_mod = parse_go_mod(&go_mod_path).unwrap();
        assert_eq!(go_mod.module_path, "example.com/mine");
        assert!(go_mod.requires.is_empty());
        assert!(go_mod.replaces.is_empty());
    }

    #[test]
    fn parse_go_mod_inline_require() {
        let tmp = tempfile::tempdir().unwrap();
        let go_mod_path = tmp.path().join("go.mod");
        std::fs::write(
            &go_mod_path,
            "module example.com/mine\nrequire github.com/x/y v1.0.0\n",
        )
        .unwrap();

        let go_mod = parse_go_mod(&go_mod_path).unwrap();
        assert_eq!(go_mod.requires.len(), 1);
        assert_eq!(go_mod.requires[0].module, "github.com/x/y");
        assert_eq!(go_mod.requires[0].version, "v1.0.0");
    }

    // ── go.sum parser ────────────────────────────────────────────────────────

    #[test]
    fn parse_go_sum_multiple_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let go_sum_path = tmp.path().join("go.sum");
        std::fs::write(
            &go_sum_path,
            "github.com/gin-gonic/gin v1.9.1 h1:abc123=\n\
             github.com/gin-gonic/gin v1.9.1/go.mod h1:def456=\n\
             github.com/pkg/errors v0.9.1 h1:ghi789=\n\
             github.com/pkg/errors v0.9.1/go.mod h1:jkl012=\n",
        )
        .unwrap();

        let sum = parse_go_sum(&go_sum_path).unwrap();
        assert_eq!(
            sum.get("github.com/gin-gonic/gin").unwrap(),
            "v1.9.1"
        );
        assert_eq!(sum.get("github.com/pkg/errors").unwrap(), "v0.9.1");
    }

    #[test]
    fn parse_go_sum_strips_go_mod_suffix() {
        // A module appearing only on a /go.mod line should still yield its version.
        let tmp = tempfile::tempdir().unwrap();
        let go_sum_path = tmp.path().join("go.sum");
        std::fs::write(
            &go_sum_path,
            "github.com/x/y v2.0.0/go.mod h1:abc=\n",
        )
        .unwrap();

        let sum = parse_go_sum(&go_sum_path).unwrap();
        assert_eq!(sum.get("github.com/x/y").unwrap(), "v2.0.0");
    }

    #[test]
    fn version_selection_picks_module_version() {
        // Multiple go.sum lines for the same module (module hash + /go.mod hash)
        // all carry the same version; the parser should yield that version.
        let tmp = tempfile::tempdir().unwrap();
        let go_sum_path = tmp.path().join("go.sum");
        std::fs::write(
            &go_sum_path,
            "github.com/x/y v1.2.3 h1:hash1=\n\
             github.com/x/y v1.2.3/go.mod h1:hash2=\n",
        )
        .unwrap();

        let sum = parse_go_sum(&go_sum_path).unwrap();
        assert_eq!(sum.get("github.com/x/y").unwrap(), "v1.2.3");
    }

    // ── item regexes ─────────────────────────────────────────────────────────

    #[test]
    fn go_line_defines_func() {
        assert_eq!(
            go_line_defines_item("func Println(a ...interface{}) (n int, err error) {", "Println"),
            Some("func")
        );
        assert_eq!(
            go_line_defines_item("func New() *Error {", "New"),
            Some("func")
        );
        // Indented function.
        assert_eq!(
            go_line_defines_item("    func helper() {", "helper"),
            Some("func")
        );
    }

    #[test]
    fn go_line_defines_method_with_receiver() {
        assert_eq!(
            go_line_defines_item("func (w *Writer) Write(p []byte) (int, error) {", "Write"),
            Some("func")
        );
        assert_eq!(
            go_line_defines_item("func (s *Server) Handle(r *http.Request) {", "Handle"),
            Some("func")
        );
    }

    #[test]
    fn go_line_defines_type() {
        assert_eq!(
            go_line_defines_item("type Router struct {", "Router"),
            Some("type")
        );
        assert_eq!(
            go_line_defines_item("type Handler interface {", "Handler"),
            Some("type")
        );
        assert_eq!(
            go_line_defines_item("type Context = context.Context", "Context"),
            Some("type")
        );
    }

    #[test]
    fn go_line_defines_const_and_var() {
        assert_eq!(
            go_line_defines_item("const MaxRetries = 5", "MaxRetries"),
            Some("const")
        );
        assert_eq!(
            go_line_defines_item("const Version string = \"1.0\"", "Version"),
            Some("const")
        );
        assert_eq!(
            go_line_defines_item("var ErrNotFound = errors.New(\"not found\")", "ErrNotFound"),
            Some("var")
        );
        assert_eq!(
            go_line_defines_item("var mu sync.Mutex", "mu"),
            Some("var")
        );
    }

    #[test]
    fn go_line_defines_rejects_assignment() {
        // `x := X` is an assignment, not a definition of X.
        assert_eq!(go_line_defines_item("x := 42", "42"), None);
        assert_eq!(
            go_line_defines_item("x := fmt.Sprintf(\"%d\", 1)", "Sprintf"),
            None
        );
    }

    #[test]
    fn go_line_defines_rejects_comments() {
        assert_eq!(go_line_defines_item("// func X() {", "X"), None);
        assert_eq!(go_line_defines_item("/* func X() { */", "X"), None);
        // Indented comment.
        assert_eq!(go_line_defines_item("    // func X() {", "X"), None);
    }

    #[test]
    fn go_line_defines_rejects_receiver_type_reference() {
        // Looking for item "R" in `func (r R) Method` — R is the receiver
        // type (a reference), not a definition.
        assert_eq!(
            go_line_defines_item("func (r *R) Method() {", "R"),
            None
        );
        assert_eq!(
            go_line_defines_item("func (r R) Method() {", "R"),
            None
        );
        // But looking for "Method" in the same line does match.
        assert_eq!(
            go_line_defines_item("func (r R) Method() {", "Method"),
            Some("func")
        );
    }

    #[test]
    fn go_line_defines_rejects_substring() {
        // "Println" should not match "PrintlnExtra".
        assert_eq!(
            go_line_defines_item("func PrintlnExtra() {", "Println"),
            None
        );
        // "X" should not match "XY".
        assert_eq!(go_line_defines_item("func XY() {", "X"), None);
    }

    #[test]
    fn go_line_defines_rejects_non_definition_contexts() {
        // Return type reference.
        assert_eq!(go_line_defines_item("func f() *Error {", "Error"), None);
        // Type assertion.
        assert_eq!(
            go_line_defines_item("if e, ok := x.(Error); ok {", "Error"),
            None
        );
        // Function call.
        assert_eq!(
            go_line_defines_item("fmt.Println(\"hello\")", "Println"),
            None
        );
        // Struct field.
        assert_eq!(
            go_line_defines_item("type Config struct { Timeout int }", "Timeout"),
            None
        );
    }

    /// C15: identifier-constituency mirrors the redline crate's Unicode
    /// word-char rule (`redline::model::buffer::is_word_char`) — a non-ASCII
    /// letter is an identifier character, so it blocks a whole-word boundary
    /// just like an ASCII one (the sibling `crate::cargo` copy is pinned the
    /// same way). Under the old ASCII rule the `é` in `greeté` was not an
    /// identifier char, so `greet` was mis-detected as a definition.
    #[test]
    fn go_line_defines_ident_char_is_unicode_aware() {
        assert!(is_ident_char('é'), "accented letter");
        assert!(is_ident_char('漢'), "CJK letter");
        assert!(is_ident_char('_'));
        assert!(!is_ident_char('-'));
        assert!(!is_ident_char(' '));
        // Whole-word pin: `greet` inside `greeté` is NOT a definition of
        // `greet` (the `é` is an identifier char, not a boundary).
        assert_eq!(go_line_defines_item("func greeté() {", "greet"), None);
        // And the full Unicode identifier IS a definition of itself.
        assert_eq!(go_line_defines_item("func greeté() {", "greeté"), Some("func"));
    }

    // ── locate_go_item ───────────────────────────────────────────────────────

    #[test]
    fn locate_go_item_finds_definition_in_tempdir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("main.go"),
            "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("util.go"),
            "package main\n\n// Helper is a utility function.\nfunc Helper() int {\n\treturn 42\n}\n",
        )
        .unwrap();

        let (file, line) = locate_go_item(tmp.path(), "Helper").unwrap();
        assert_eq!(file.file_name().unwrap(), "util.go");
        // Line 4 (1-based): "func Helper() int {"
        assert_eq!(line, 4);
    }

    #[test]
    fn locate_go_item_finds_method() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("server.go"),
            "package server\n\ntype Server struct{}\n\nfunc (s *Server) Start() error {\n\treturn nil\n}\n",
        )
        .unwrap();

        let (file, line) = locate_go_item(tmp.path(), "Start").unwrap();
        assert_eq!(file.file_name().unwrap(), "server.go");
        // Line 5 (1-based): "func (s *Server) Start() error {"
        assert_eq!(line, 5);
    }

    #[test]
    fn locate_go_item_missing_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("main.go"),
            "package main\nfunc main() {}\n",
        )
        .unwrap();
        assert!(locate_go_item(tmp.path(), "NonExistent").is_none());
    }

    #[test]
    fn locate_go_item_skips_dotdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let hidden = tmp.path().join(".hidden");
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::write(hidden.join("secret.go"), "func Secret() {}\n").unwrap();
        std::fs::write(tmp.path().join("main.go"), "func main() {}\n").unwrap();

        assert!(locate_go_item(tmp.path(), "Secret").is_none());
    }

    // ── local-path helpers ───────────────────────────────────────────────────

    #[test]
    fn is_local_path_detects_relative_and_absolute() {
        assert!(is_local_path("./local/lib"));
        assert!(is_local_path("../sibling"));
        assert!(is_local_path("/abs/path"));
        // Module paths are not local.
        assert!(!is_local_path("github.com/new/lib v2.0.0"));
        assert!(!is_local_path("example.com/other"));
    }

    #[test]
    fn resolve_local_path_relative() {
        let ws = Path::new("/home/user/project");
        assert_eq!(
            resolve_local_path(ws, "./forks/gin"),
            PathBuf::from("/home/user/project/forks/gin")
        );
        assert_eq!(
            resolve_local_path(ws, "../sibling"),
            PathBuf::from("/home/user/project/../sibling")
        );
    }

    #[test]
    fn resolve_local_path_absolute() {
        let ws = Path::new("/home/user/project");
        assert_eq!(
            resolve_local_path(ws, "/opt/forks/gin"),
            PathBuf::from("/opt/forks/gin")
        );
    }

    // ── integration: resolve_go with fixture workspace ───────────────────────

    /// Build a fixture workspace with a go.mod, go.sum, and an empty module
    /// cache directory. Returns the tempdir and the module cache path.
    fn fixture_workspace() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().to_path_buf();

        // go.mod: workspace module + one required external module.
        std::fs::write(
            ws.join("go.mod"),
            "module github.com/myorg/app\n\ngo 1.21\n\n\
             require github.com/gin-gonic/gin v1.9.1\n",
        )
        .unwrap();

        // go.sum.
        std::fs::write(
            ws.join("go.sum"),
            "github.com/gin-gonic/gin v1.9.1 h1:abc=\n\
             github.com/gin-gonic/gin v1.9.1/go.mod h1:def=\n",
        )
        .unwrap();

        // A local Go file in the workspace (for workspace-local resolution).
        std::fs::write(
            ws.join("main.go"),
            "package app\n\n// App is the main application type.\ntype App struct{}\n\nfunc (a *App) Run() error {\n\treturn nil\n}\n",
        )
        .unwrap();

        // Empty module cache dir (populated by tests as needed).
        let mod_cache = ws.join("gomodcache");
        std::fs::create_dir_all(&mod_cache).unwrap();

        (tmp, mod_cache)
    }

    #[test]
    fn resolve_workspace_local_package() {
        let (tmp, _mod_cache) = fixture_workspace();
        let ws = tmp.path().to_path_buf();

        let provider = GoProvider::new().with_mod_cache(ws.join("gomodcache"));
        let ctx = SymbolContext {
            workspace_root: ws.clone(),
            symbol: "app.Run".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("main.go"),
            language: None,
        };

        let result = provider.resolve(&ctx).unwrap();
        assert!(!result.external);
        assert_eq!(result.source_root, ws);
        assert_eq!(result.file.file_name().unwrap(), "main.go");
        // Line 6 (1-based): "func (a *App) Run() error {"
        assert_eq!(result.line, Some(6));
    }

    #[test]
    fn resolve_external_package() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().to_path_buf();

        std::fs::write(
            ws.join("go.mod"),
            "module github.com/myorg/app\n\ngo 1.21\n\n\
             require github.com/pkg/errors v0.9.1\n",
        )
        .unwrap();
        std::fs::write(
            ws.join("go.sum"),
            "github.com/pkg/errors v0.9.1 h1:abc=\n",
        )
        .unwrap();
        std::fs::write(ws.join("main.go"), "package main\nfunc main() {}\n").unwrap();

        // Module cache. All-lowercase module path → no case-encoding needed.
        // case_encode("github.com/pkg/errors") = "github.com/pkg/errors"
        let mod_cache = ws.join("gomodcache");
        let errors_dir = mod_cache.join("github.com/pkg/errors@v0.9.1");
        std::fs::create_dir_all(&errors_dir).unwrap();
        std::fs::write(
            errors_dir.join("errors.go"),
            "package errors\n\n// New is a simple error.\nfunc New(text string) error {\n\treturn nil\n}\n",
        )
        .unwrap();

        let provider = GoProvider::new().with_mod_cache(mod_cache.clone());
        let ctx = SymbolContext {
            workspace_root: ws,
            symbol: "errors.New".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("main.go"),
            language: None,
        };

        let result = provider.resolve(&ctx).unwrap();
        assert!(result.external);
        assert_eq!(result.source_root, errors_dir);
        assert_eq!(result.file.file_name().unwrap(), "errors.go");
        // Line 4 (1-based): "func New(text string) error {"
        assert_eq!(result.line, Some(4));
    }

    #[test]
    fn resolve_replaces_local_path() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().to_path_buf();

        std::fs::write(
            ws.join("go.mod"),
            "module github.com/myorg/app\n\ngo 1.21\n\n\
             require github.com/old/lib v1.0.0\n\n\
             replace github.com/old/lib => ./forks/lib\n",
        )
        .unwrap();
        std::fs::write(ws.join("main.go"), "package main\nfunc main() {}\n").unwrap();

        // Local fork at ./forks/lib.
        let fork_dir = ws.join("forks/lib");
        std::fs::create_dir_all(&fork_dir).unwrap();
        std::fs::write(
            fork_dir.join("lib.go"),
            "package lib\n\n// Helper is in the local fork.\nfunc Helper() int {\n\treturn 1\n}\n",
        )
        .unwrap();

        let provider = GoProvider::new();
        let ctx = SymbolContext {
            workspace_root: ws,
            symbol: "lib.Helper".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("main.go"),
            language: None,
        };

        let result = provider.resolve(&ctx).unwrap();
        assert!(!result.external);
        assert_eq!(result.source_root, fork_dir);
        assert_eq!(result.file.file_name().unwrap(), "lib.go");
        // Line 4 (1-based): "func Helper() int {"
        assert_eq!(result.line, Some(4));
    }

    #[test]
    fn resolve_no_go_mod_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().to_path_buf();
        // No go.mod.

        let provider = GoProvider::new();
        let ctx = SymbolContext {
            workspace_root: ws,
            symbol: "fmt.Println".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("main.go"),
            language: None,
        };

        let err = provider.resolve(&ctx).unwrap_err();
        assert!(
            err.to_string().contains("no go.mod"),
            "err: {err}"
        );
    }

    #[test]
    fn resolve_bare_symbol_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().to_path_buf();
        std::fs::write(ws.join("go.mod"), "module github.com/myorg/app\n").unwrap();

        let provider = GoProvider::new();
        let ctx = SymbolContext {
            workspace_root: ws,
            symbol: "Println".to_string(),
            from_file: PathBuf::from("main.go"),
            scope: Vec::new(),
            language: None,
        };

        let err = provider.resolve(&ctx).unwrap_err();
        assert!(
            err.to_string().contains("cannot parse a package name"),
            "err: {err}"
        );
    }

    // ── 007-03: bare symbol + scope hint (import context) ────────────────

    /// Discriminating: a BARE symbol + the import-context scope resolves
    /// through the SAME go.mod / module-cache machinery as the dot-
    /// qualified twin (`resolve_external_package` pins that twin's
    /// landing).
    #[test]
    fn bare_symbol_with_scope_resolves_external_package() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().to_path_buf();

        std::fs::write(
            ws.join("go.mod"),
            "module github.com/myorg/app\n\ngo 1.21\n\n\
             require github.com/pkg/errors v0.9.1\n",
        )
        .unwrap();
        std::fs::write(ws.join("go.sum"), "github.com/pkg/errors v0.9.1 h1:abc=\n").unwrap();
        std::fs::write(ws.join("main.go"), "package main\nfunc main() {}\n").unwrap();

        let mod_cache = ws.join("gomodcache");
        let errors_dir = mod_cache.join("github.com/pkg/errors@v0.9.1");
        std::fs::create_dir_all(&errors_dir).unwrap();
        std::fs::write(
            errors_dir.join("errors.go"),
            "package errors\n\n// New is a simple error.\nfunc New(text string) error {\n\treturn nil\n}\n",
        )
        .unwrap();

        let provider = GoProvider::new().with_mod_cache(mod_cache);
        let ctx = SymbolContext {
            workspace_root: ws,
            symbol: "New".to_string(),
            from_file: PathBuf::from("main.go"),
            scope: vec!["errors".to_string(), "New".to_string()],
            language: None,
        };

        let result = provider.resolve(&ctx).unwrap();
        assert!(result.external);
        assert_eq!(result.source_root, errors_dir);
        assert_eq!(result.file.file_name().unwrap(), "errors.go");
        assert_eq!(result.line, Some(4));
    }

    #[test]
    fn resolve_unrequired_module_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().to_path_buf();
        std::fs::write(ws.join("go.mod"), "module github.com/myorg/app\n").unwrap();

        let provider = GoProvider::new();
        let ctx = SymbolContext {
            workspace_root: ws,
            symbol: "gin.Router".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("main.go"),
            language: None,
        };

        let err = provider.resolve(&ctx).unwrap_err();
        assert!(
            err.to_string().contains("not a required module"),
            "err: {err}"
        );
    }

    #[test]
    fn resolve_item_not_found_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().to_path_buf();
        std::fs::write(
            ws.join("go.mod"),
            "module github.com/myorg/app\n\ngo 1.21\n\n\
             require github.com/pkg/errors v0.9.1\n",
        )
        .unwrap();

        let mod_cache = ws.join("gomodcache");
        let errors_dir = mod_cache.join("github.com/pkg/errors@v0.9.1");
        std::fs::create_dir_all(&errors_dir).unwrap();
        std::fs::write(
            errors_dir.join("errors.go"),
            "package errors\nfunc New(text string) error { return nil }\n",
        )
        .unwrap();

        let provider = GoProvider::new().with_mod_cache(mod_cache);
        let ctx = SymbolContext {
            workspace_root: ws,
            symbol: "errors.NotFound".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("main.go"),
            language: None,
        };

        let err = provider.resolve(&ctx).unwrap_err();
        assert!(
            err.to_string().contains("no definition of"),
            "err: {err}"
        );
    }

    // ── 011-08: path-shaped alias rewrite (`scope_qualified_alias`) ─────

    /// Build an `errors`-module fixture: a go.mod requiring
    /// `github.com/pkg/errors v0.9.1` plus a module-cache layout whose
    /// `wrap.go` defines `Wrap` on line 5 (mirrors corpus probe 08).
    /// Returns the workspace, the cache dir, and the module dir.
    fn alias_rewrite_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().to_path_buf();
        std::fs::write(
            ws.join("go.mod"),
            "module github.com/myorg/app\n\ngo 1.21\n\n\
             require github.com/pkg/errors v0.9.1\n",
        )
        .unwrap();
        std::fs::write(
            ws.join("go.sum"),
            "github.com/pkg/errors v0.9.1 h1:abc=\n",
        )
        .unwrap();
        std::fs::write(ws.join("main.go"), "package main\nfunc main() {}\n").unwrap();

        let mod_cache = ws.join("gomodcache");
        let errors_dir = mod_cache.join("github.com/pkg/errors@v0.9.1");
        std::fs::create_dir_all(&errors_dir).unwrap();
        std::fs::write(
            errors_dir.join("wrap.go"),
            "package errors\n\n// Wrap annotates the given err with a message.\n\
             // Wrapf is Wrap formatted.\n\
             func Wrap(err error, message string) error {\n\treturn nil\n}\n",
        )
        .unwrap();

        (tmp, mod_cache, errors_dir)
    }

    /// Discriminating (corpus probe 08 flip): `pe.Wrap` — the PATH-SHAPED
    /// use site of the aliased import `import pe
    /// "github.com/pkg/errors"` — with the hint naming the real package
    /// path rewrites the alias (`pe` → `errors`) and lands on the `Wrap`
    /// definition in the module cache, through the SAME go.mod /
    /// module-cache machinery.
    #[test]
    fn aliased_dot_qualified_use_rewrites_to_real_package() {
        let (tmp, mod_cache, errors_dir) = alias_rewrite_fixture();
        let ws = tmp.path().to_path_buf();

        let provider = GoProvider::new().offline().with_mod_cache(mod_cache);
        let ctx = SymbolContext {
            workspace_root: ws,
            symbol: "pe.Wrap".to_string(),
            from_file: PathBuf::from("main.go"),
            scope: vec!["errors".to_string(), "Wrap".to_string()],
            language: None,
        };

        let result = provider.resolve(&ctx).unwrap();
        assert!(result.external);
        assert_eq!(result.source_root, errors_dir);
        assert_eq!(result.file.file_name().unwrap(), "wrap.go");
        // Line 5 (1-based): "func Wrap(err error, message string) error {"
        assert_eq!(result.line, Some(5));
    }

    /// No-op pins for the rewrite: an EMPTY scope (byte-for-byte
    /// degradation), an identity hint (`errors.Wrap` +
    /// `["errors", "Wrap"]` — the hint IS the symbol's own path), and a
    /// mismatched-item hint (`pe.Wrap` + `["errors", "Other"]`) must all
    /// leave the symbol's own path untouched: the identity case resolves
    /// through the symbol's OWN package path, and the alias cases keep
    /// `pe` → the byte-for-byte "not a required module" bail.
    #[test]
    fn aliased_dot_qualified_identity_mismatched_and_unhinted_are_no_ops() {
        let (tmp, mod_cache, errors_dir) = alias_rewrite_fixture();
        let ws = tmp.path().to_path_buf();
        let provider = GoProvider::new().offline().with_mod_cache(mod_cache);

        // Identity: the symbol's own path wins — same landing as the
        // plain `errors.Wrap` resolve.
        let ctx = SymbolContext {
            workspace_root: ws.clone(),
            symbol: "errors.Wrap".to_string(),
            from_file: PathBuf::from("main.go"),
            scope: vec!["errors".to_string(), "Wrap".to_string()],
            language: None,
        };
        let result = provider.resolve(&ctx).unwrap();
        assert!(result.external);
        assert_eq!(result.source_root, errors_dir);
        assert_eq!(result.file.file_name().unwrap(), "wrap.go");
        assert_eq!(result.line, Some(5));

        // Unhinted and mismatched-item: the alias `pe` is still taken as
        // the package name → the byte-for-byte "not a required module".
        for scope in [
            Vec::new(),
            vec!["errors".to_string(), "Other".to_string()],
        ] {
            let ctx = SymbolContext {
                workspace_root: ws.clone(),
                symbol: "pe.Wrap".to_string(),
                from_file: PathBuf::from("main.go"),
                scope,
                language: None,
            };
            let err = provider.resolve(&ctx).unwrap_err();
            assert!(
                err.to_string().contains("not a required module"),
                "no-op case bailed with: {err}"
            );
        }
    }

    // ── live tests (require go toolchain) ────────────────────────────────────

    /// Live: requires the `go` binary on PATH. Verifies that `go env
    /// GOMODCACHE` returns a valid absolute path and that `resolve_mod_cache`
    /// uses it when no explicit override is set.
    #[test]
    #[ignore] // requires go toolchain
    fn live_go_env_gomodcache_returns_path() {
        let provider = GoProvider::new();
        let cache = provider.resolve_mod_cache().unwrap();
        assert!(
            cache.is_absolute(),
            "GOMODCACHE should be an absolute path: {cache:?}"
        );
    }

    /// Live: requires the `go` binary and network access. Verifies that
    /// `go mod download` fetches a module into the module cache and that the
    /// module dir is created with the correct case-encoded path.
    #[test]
    #[ignore] // requires go toolchain + network
    fn live_go_mod_download_populates_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        std::fs::write(
            ws.join("go.mod"),
            "module test.local\ngo 1.21\nrequire github.com/pkg/errors v0.9.1\n",
        )
        .unwrap();
        std::fs::write(
            ws.join("go.sum"),
            "github.com/pkg/errors v0.9.1 h1:abc=\n",
        )
        .unwrap();
        std::fs::write(
            ws.join("main.go"),
            "package main\nfunc main() {}\n",
        )
        .unwrap();

        let provider = GoProvider::new();
        let cache = provider.resolve_mod_cache().unwrap();
        let encoded = case_encode("github.com/pkg/errors");
        let module_dir = cache.join(format!("{encoded}@v0.9.1"));

        assert!(
            !module_dir.exists(),
            "module dir should not exist before download"
        );

        provider.run_mod_download(ws, "github.com/pkg/errors").unwrap();

        assert!(
            module_dir.exists(),
            "module dir should exist after `go mod download`"
        );
    }

    /// Live: requires the `go` binary and network access. Full E2E: resolve
    /// `errors.New` from `github.com/pkg/errors` through the module cache.
    #[test]
    #[ignore] // requires go toolchain + network
    fn live_resolve_external_e2e() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        std::fs::write(
            ws.join("go.mod"),
            "module test.local\ngo 1.21\nrequire github.com/pkg/errors v0.9.1\n",
        )
        .unwrap();
        std::fs::write(
            ws.join("go.sum"),
            "github.com/pkg/errors v0.9.1 h1:abc=\n",
        )
        .unwrap();
        std::fs::write(
            ws.join("main.go"),
            "package main\nfunc main() {}\n",
        )
        .unwrap();

        let provider = GoProvider::new();
        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "errors.New".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("main.go"),
            language: None,
        };

        let result = provider.resolve(&ctx).unwrap();
        assert!(result.external);
        assert_eq!(result.file.file_name().unwrap(), "errors.go");
        assert!(result.line.is_some());
    }
}
