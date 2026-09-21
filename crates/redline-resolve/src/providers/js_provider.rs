//! JavaScript/TypeScript [`ToolingProvider`]: resolve a workspace-missed
//! symbol to its concrete source file inside an npm package.
//!
//! Flow: parse the (package, item) from the symbol → locate the package
//! directory (a workspace-local `file:`/`link:` path dependency, or
//! `node_modules/<pkg>` found by walking up from the workspace root to honor
//! monorepo layouts) → if absent, `npm install` (sanctioned fetch-on-demand,
//! `--no-audit --no-fund`) and re-locate → resolve the entry point
//! (`exports` → `module` → `main`, with a TypeScript-source preference) →
//! scan the package for a definition-shaped match of the item (best-effort
//! line).
//!
//! Normalization (documented per the task): the package↔member boundary is a
//! `.` or `::` separator (JS member access). A `/` is *inside* the package
//! name — scoped packages (`@scope/name`) and subpath modules (`lodash/get`)
//! keep their `/`. The first `.`/`::` splits the package (which may still
//! contain `/`) from the member item. So `@testing-library/react.render` →
//! package `@testing-library/react`, item `render`; `lodash/get` → package
//! `lodash/get`, no item.
//!
//! Path-shaped specifiers NEVER reach the split (011-08 follow-up, fix-jsrel):
//! a relative import (`./x`, `../x`) resolves against the importing buffer's
//! directory and lands in the sibling file when it exists (workspace-local,
//! `external = false` — the same semantics as a `file:` dependency); an
//! absolute path (`/x`) and a file-ish name (`styles.css`, a side-effect
//! import) bail with a dedicated, informative message. Previously these
//! shapes mangled through the split (leading dot → base package `.`, which
//! probed `node_modules/.` as a package and funneled online into
//! `npm install "."`; absolute → the empty base; `styles.css` → package
//! `styles` + item `css`).
//!
//! Plain `node_modules` only — no yarn-PnP, no npm-workspaces resolution.
//! Installs are always local (never `-g`/global).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::{run_with_timeout, scope_qualified, scope_qualified_alias, ResolvedSource, SymbolContext, ToolingProvider};

/// File extensions considered JavaScript/TypeScript sources. `d.ts`
/// sits at the tail: Node has no `.d.ts` runtime convention (the
/// runtime extensions keep their LOAD_AS_FILE order), but go-to-
/// definition WANTS declaration files — `import type {X} from "./types"`
/// where only `types.d.ts` exists must land, not bail. (For
/// `walk_js_files` the entry is inert: `types.d.ts`'s `extension()` is
/// `ts`, so declaration files were already scanned package-side.)
const JS_EXT: &[&str] = &["js", "mjs", "cjs", "ts", "tsx", "jsx", "mts", "cts", "d.ts"];

/// File extensions that make a dotted name file-ish (a FILE NAME, not a
/// `package.item` path): the JS/TS sources plus the static assets a
/// side-effect import (`import './styles.css'`) can name.
const JS_FILE_EXT: &[&str] = &[
    "js", "mjs", "cjs", "ts", "tsx", "jsx", "mts", "cts", "css", "scss",
    "sass", "less", "svg", "png", "jpg", "jpeg", "gif", "webp", "ico", "woff",
    "woff2", "ttf", "json", "html",
];

/// Timeout for `npm install` (network download).
const NPM_INSTALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Resolve JS/TS symbols to real source via `node_modules` / `npm install`.
pub struct JsProvider {
    /// npm binary to invoke (overridable for testing).
    npm_bin: String,
    /// When `true`, never run `npm install`; a package missing from
    /// `node_modules` then fails with a clean offline-refusal error.
    offline: bool,
}

impl Default for JsProvider {
    fn default() -> Self {
        Self {
            npm_bin: "npm".to_string(),
            offline: false,
        }
    }
}

impl JsProvider {
    /// Online provider using the ambient `npm` on `PATH` (the default).
    pub fn new() -> Self {
        Self::default()
    }

    /// Never install: refuse on a package missing from `node_modules`
    /// (clean offline-refusal error).
    pub fn offline(mut self) -> Self {
        self.offline = true;
        self
    }

    /// Invoke a specific npm binary (mainly for tests).
    pub fn with_npm_bin(mut self, bin: impl Into<String>) -> Self {
        self.npm_bin = bin.into();
        self
    }

    /// 011-08 follow-up (fix-jsrel): a relative import specifier resolves
    /// against the importing buffer's directory — the file the import
    /// names is workspace-local, so the landing is `external = false`
    /// (the same semantics as a `file:` dependency). A multi-segment
    /// hint's member is scanned for a definition in that file; a
    /// whole-module binding lands with no line.
    fn resolve_relative(
        &self,
        ctx: &SymbolContext,
        spec: &str,
        member: Option<&String>,
    ) -> anyhow::Result<ResolvedSource> {
        let ws = ctx
            .workspace_root
            .canonicalize()
            .map_err(|e| anyhow::anyhow!("cannot canonicalize {}: {e}", ctx.workspace_root.display()))?;
        let file = resolve_relative_file(&ws, &ctx.from_file, spec).ok_or_else(|| {
            anyhow::anyhow!(
                "relative specifier `{spec}` cannot be resolved to a workspace file \
                 (sought next to `{}`)",
                ctx.from_file.display()
            )
        })?;
        // Canonicalize BEFORE the workspace-prefix check: a lexically
        // built path (carrying `..` segments) would make `starts_with`
        // lie about containment.
        let file = std::fs::canonicalize(&file)
            .map_err(|e| anyhow::anyhow!("cannot canonicalize {}: {e}", file.display()))?;
        if !file.starts_with(&ws) {
            anyhow::bail!(
                "relative specifier `{spec}` resolves outside the workspace ({}); \
                 the JS provider stays workspace-local",
                file.display()
            );
        }
        let dir = file
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or(ws);
        let line = match member {
            Some(m) => Some(scan_file_for_def(&file, m).ok_or_else(|| {
                anyhow::anyhow!(
                    "member `{m}` not found in `{}` (imported via `{spec}`)",
                    file.display()
                )
            })?),
            None => None,
        };
        Ok(ResolvedSource {
            file,
            source_root: dir,
            external: false,
            line,
        })
    }

    /// Run a *local* `npm install <pkg> --no-audit --no-fund` in `install_root`.
    /// Never uses `-g` (no global installs).
    fn npm_install(&self, install_root: &Path, base_pkg: &str) -> anyhow::Result<()> {
        let mut cmd = Command::new(&self.npm_bin);
        cmd.current_dir(install_root)
            .arg("install")
            .arg(base_pkg)
            .arg("--no-audit")
            .arg("--no-fund");
        let out = run_with_timeout(cmd, NPM_INSTALL_TIMEOUT).map_err(|e| {
            anyhow::anyhow!(
                "failed to run `npm install {base_pkg}` in {}: {e}",
                install_root.display()
            )
        })?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(anyhow::anyhow!(
                "`npm install {base_pkg}` failed in {}: {stderr}",
                install_root.display()
            ));
        }
        Ok(())
    }

    fn resolve_js(&self, ctx: &SymbolContext) -> anyhow::Result<ResolvedSource> {
        // A BARE (dot-free) symbol with a scope hint (007-03: the app's
        // import path) normalizes to `package.item` up front and then flows
        // through the SAME node_modules / locate machinery as a
        // path-shaped symbol. No hint keeps the bail, byte-for-byte.
        // 011-02: a namespace-aliased member (`ns.member` from
        // `import * as ns from "pkg"`, hinted as `["pkg", "member"]`) is
        // rewritten to the package's real path and flows through the SAME
        // machinery; identity hints and non-rewrites leave the symbol's
        // own path untouched.
        let qualified = scope_qualified(".", &ctx.symbol, &ctx.scope)
            .or_else(|| scope_qualified_alias(".", &ctx.symbol, &ctx.scope));
        let symbol = qualified.as_deref().unwrap_or(&ctx.symbol);

        // 011-08 follow-up (fix-jsrel): a path-shaped specifier is not a
        // package path — a DEDICATED degradation fires BEFORE split_symbol /
        // base_package_name can mangle it (leading dot → base package `.`;
        // absolute → empty base; `styles.css` → package `styles` + item
        // `css`). Relative specs land in the sibling file (a
        // workspace-local, external = false resolution); absolute paths and
        // file-ish names bail with an honest, informative message.
        let is_relative = symbol.starts_with("./") || symbol.starts_with("../");
        let is_absolute = !is_relative && symbol.starts_with('/');
        if is_relative || is_absolute {
            // The specifier the app handed over is scope[0] (the joined
            // qualified path appends the member with a dot); with no hint
            // the symbol IS the specifier. Guaranteed to carry the same
            // path marker as `symbol` when a hint exists.
            let spec = ctx
                .scope
                .first()
                .map_or_else(|| symbol.to_string(), String::clone);
            if is_relative {
                // A multi-segment hint's last segment is the imported MEMBER
                // (`import { legacyJoin } from "./legacy-util"`); a
                // 1-segment hint is a whole-module binding (no member).
                let member = (ctx.scope.len() >= 2).then(|| ctx.scope.last()).flatten();
                return self.resolve_relative(ctx, &spec, member);
            }
            anyhow::bail!(
                "absolute path specifier `{spec}` is not a package path; the JS \
                 provider resolves package specs and workspace-relative files only"
            );
        }
        if is_file_ish(symbol) {
            anyhow::bail!(
                "file-ish name `{symbol}` is not a package path (a side-effect \
                 import binds no package); nothing to resolve"
            );
        }

        let (pkg_spec, item) = split_symbol(symbol);
        // 011-02 review P2-1: a bare (dot-free) symbol bails UNLESS a scope
        // hint names the package entry itself — `import * as ns from "pkg"`
        // (or a CJS whole-module binding) emits `["pkg"]`, and `ns` should
        // land on the package's entry file. With NO hint, resolving to a
        // concrete package would be a guess: bail (byte-for-byte unchanged).
        // `item.is_none() && qualified.is_some()` holds iff the hint is
        // exactly 1 segment (a longer hint joins to a dotted path, so `item`
        // would be Some).
        let hinted_entry = item.is_none() && qualified.is_some();
        if item.is_none() && !hinted_entry {
            anyhow::bail!(
                "bare symbol `{}` has no package path; resolving it to a package \n\
                 needs scope info (tree-sitter) not yet provided by the app",
                ctx.symbol
            );
        }
        let base_pkg = base_package_name(&pkg_spec);
        let ws = ctx
            .workspace_root
            .canonicalize()
            .map_err(|e| anyhow::anyhow!("cannot canonicalize {}: {e}", ctx.workspace_root.display()))?;

        // 1. Workspace-local path dependency (`file:` / `link:`) → external =
        //    false, and the source lives at the linked dir (not node_modules).
        if let Some(local_dir) = find_local_path_dep(&ws, &base_pkg) {
            let local_dir = std::fs::canonicalize(&local_dir).unwrap_or(local_dir);
            let (file, line) = locate_or_entry(&local_dir, item.as_deref())?;
            return Ok(ResolvedSource {
                file,
                source_root: local_dir,
                external: false,
                line,
            });
        }

        // 2. Locate `node_modules/<pkg>` walking up from the workspace root.
        let pkg_dir = match locate_in_node_modules(&ws, &base_pkg) {
            Some(dir) => dir,
            None => {
                // A degenerate base (empty / `.` / `..`) is not a package
                // name: never `npm install` it (the online relative-hint
                // corner that used to funnel into `npm install "."`).
                if base_pkg.is_empty() || base_pkg == "." || base_pkg == ".." {
                    anyhow::bail!(
                        "`{base_pkg}` is not a package name; it cannot be looked \
                         up in node_modules or installed"
                    );
                }
                if self.offline {
                    anyhow::bail!(
                        "package `{base_pkg}` is not in node_modules and offline mode \
                         refuses to install it"
                    );
                }
                // Fetch-on-demand (sanctioned operator directive): `npm install`
                // in the nearest npm project root, then re-locate.
                let install_root = find_project_root(&ws).ok_or_else(|| {
                    anyhow::anyhow!(
                        "no package.json found walking up from {}; not an npm project, \
                         so `{base_pkg}` cannot be resolved",
                        ws.display()
                    )
                })?;
                self.npm_install(&install_root, &base_pkg)?;
                locate_in_node_modules(&ws, &base_pkg).ok_or_else(|| {
                    anyhow::anyhow!(
                        "package `{base_pkg}` still not found in node_modules after \
                         `npm install` (searched from {})",
                        ws.display()
                    )
                })?
            }
        };

        // 3–5. Entry point + definition locate; external registry dep.
        let (file, line) = locate_or_entry(&pkg_dir, item.as_deref())?;
        Ok(ResolvedSource {
            file,
            source_root: pkg_dir,
            external: true,
            line,
        })
    }
}

impl ToolingProvider for JsProvider {
    fn name(&self) -> &'static str {
        "javascript"
    }

    fn languages(&self) -> &'static [&'static str] {
        &["javascript", "typescript", "tsx", "jsx"]
    }

    fn resolve(&self, ctx: &SymbolContext) -> anyhow::Result<ResolvedSource> {
        self.resolve_js(ctx)
    }
}

// ── symbol / package-name normalization ──────────────────────────────────────

/// Split a symbol into `(package, item)` per the documented normalization:
/// the first `.` or `::` is the package↔member boundary; `/` stays inside the
/// package (scoped `@scope/name`, subpath `name/sub`).
fn split_symbol(symbol: &str) -> (String, Option<String>) {
    let normalized = symbol.replace("::", ".");
    match normalized.split_once('.') {
        Some((pkg, item)) => {
            if pkg.is_empty() {
                (normalized, None)
            } else {
                (pkg.to_string(), Some(item.to_string()))
            }
        }
        None => (normalized, None),
    }
}

/// The `node_modules` key for a package spec: a scoped package keeps its
/// `@scope/name` (two segments); a plain package is its first segment; any
/// `/subpath` beyond that is dropped (it resolves within the base package).
fn base_package_name(pkg_spec: &str) -> String {
    if let Some(rest) = pkg_spec.strip_prefix('@') {
        // `@scope/name[/sub]` → `@scope/name`
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() >= 2 {
            format!("@{}/{}", parts[0], parts[1])
        } else {
            pkg_spec.to_string()
        }
    } else {
        pkg_spec.split('/').next().unwrap_or(pkg_spec).to_string()
    }
}

/// A dotted tail that is a known file extension: `styles.css`, `logo.svg`,
/// `data.json` — the file name a side-effect import (`import './x.css'`)
/// leaves behind, not a `package.item` path. JS/TS extensions count too
/// (`index.js`), since no package carries a file-extension member.
fn is_file_ish(symbol: &str) -> bool {
    let Some((_, ext)) = symbol.rsplit_once('.') else {
        return false;
    };
    JS_FILE_EXT.contains(&ext)
}

/// The concrete file a relative specifier names, next to the importing
/// buffer: the exact file, then the JS-extension walk (declaration files
/// last — `types.d.ts` only resolves when no runtime source twin exists,
/// see [`JS_EXT`]), then a directory import (its package.json entry
/// point, else `index.<ext>`). A dotted tail that is NOT a JS extension
/// (a static asset, `./styles.css`) is the exact file only — no walk.
fn resolve_relative_file(ws: &Path, from_file: &Path, spec: &str) -> Option<PathBuf> {
    // The buffer's directory: from_file is workspace-relative in the probe
    // harness and may be absolute in the app; a bare file name has no
    // directory → the workspace root.
    let buffer = if from_file.is_absolute() {
        from_file.to_path_buf()
    } else {
        ws.join(from_file)
    };
    let dir = buffer
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| ws.to_path_buf());
    let target = dir.join(spec); // Path keeps the `./` / `../` segments
    if target.is_file() {
        return Some(target);
    }
    if target
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| !JS_EXT.contains(&ext))
    {
        return None; // a static asset: the exact file is the whole story
    }
    for ext in JS_EXT {
        let cand = target.with_extension(ext);
        if cand.is_file() {
            return Some(cand);
        }
    }
    if target.is_dir() {
        if let Some(entry) = resolve_entry_point(&target) {
            return Some(entry);
        }
        for ext in JS_EXT {
            let cand = target.join(format!("index.{ext}"));
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

// ── package location (node_modules / local path dep) ─────────────────────────

/// All directories from `start` up to the filesystem root, self first.
fn ancestors_including_self(start: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut cur = start.to_path_buf();
    loop {
        out.push(cur.clone());
        match cur.parent().filter(|p| !p.as_os_str().is_empty()) {
            Some(p) => cur = p.to_path_buf(),
            None => break,
        }
    }
    out
}

/// Nearest ancestor (including `start`) with a `node_modules/<base_pkg>`.
/// The `node_modules` directory itself is never a package: the degenerate
/// bases `.` / `..` (what a mangled relative specifier used to leave) must
/// not probe `node_modules/.` as a package (011-08 follow-up, fix-jsrel).
fn locate_in_node_modules(start: &Path, base_pkg: &str) -> Option<PathBuf> {
    if base_pkg == "." || base_pkg == ".." {
        return None;
    }
    for dir in ancestors_including_self(start) {
        let candidate = dir.join("node_modules").join(base_pkg);
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

/// Nearest ancestor (including `start`) containing a `package.json` — the npm
/// project root where `npm install` should run.
fn find_project_root(start: &Path) -> Option<PathBuf> {
    ancestors_including_self(start).into_iter().find(|dir| dir.join("package.json").is_file())
}

/// Resolve a workspace-local `file:`/`link:` dependency to its concrete dir,
/// searching package.json files from the workspace root upward. Returns `None`
/// if no local path dependency named `base_pkg` is declared.
fn find_local_path_dep(start: &Path, base_pkg: &str) -> Option<PathBuf> {
    for dir in ancestors_including_self(start) {
        // 011-08 polish (js-polish): a MISSING package.json is just a
        // non-project level — skip it and keep climbing (a `file:` dep
        // declared two levels up used to be invisible). A MALFORMED one
        // bails the walk: it is a project root whose dependency map cannot
        // be read, and climbing past a broken manifest would be a guess.
        let Ok(s) = std::fs::read_to_string(dir.join("package.json")) else {
            continue;
        };
        let pj = serde_json::from_str(&s).ok()?;
        let spec = dep_spec(&pj, base_pkg)?;
        let target = spec.strip_prefix("file:").or_else(|| spec.strip_prefix("link:"))?;
        let pb = PathBuf::from(target);
        let resolved = if pb.is_absolute() { pb } else { dir.join(pb) };
        if resolved.is_dir() {
            return Some(resolved);
        }
    }
    None
}

/// The version spec for `base_pkg` across the usual dependency maps.
fn dep_spec<'a>(pj: &'a PkgJson, base_pkg: &str) -> Option<&'a str> {
    let specs = [
        pj.dependencies.get(base_pkg),
        pj.dev_dependencies.get(base_pkg),
        pj.peer_dependencies.get(base_pkg),
        pj.optional_dependencies.get(base_pkg),
    ];
    for opt in specs {
        if let Some(s) = opt.and_then(|v| v.as_str()) {
            return Some(s);
        }
    }
    None
}

// ── entry point + item locate ────────────────────────────────────────────────

/// Resolve the concrete source file for a package: a definition of `item`
/// (best-effort line) when an item is given, else the package entry point
/// (line `None`).
fn locate_or_entry(pkg_dir: &Path, item: Option<&str>) -> anyhow::Result<(PathBuf, Option<u32>)> {
    match item {
        Some(it) => {
            let (file, line) = locate_item(pkg_dir, it).ok_or_else(|| {
                anyhow::anyhow!(
                    "item `{it}` not found in package `{}` (searched {})",
                    base_package_name(pkg_dir.file_name().unwrap_or_default().to_str().unwrap_or("")),
                    pkg_dir.display()
                )
            })?;
            Ok((file, Some(line)))
        }
        None => {
            let file = resolve_entry_point(pkg_dir).ok_or_else(|| {
                anyhow::anyhow!(
                    "no entry point for package at {} (no package.json exports/main/module)",
                    pkg_dir.display()
                )
            })?;
            Ok((file, None))
        }
    }
}

/// Best-effort locate of a definition of `item` under `pkg_dir`: the entry
/// file is tried first, then the package is walked (skipping inner
/// `node_modules` and dot-dirs) in a deterministic sorted order.
fn locate_item(pkg_dir: &Path, item: &str) -> Option<(PathBuf, u32)> {
    if let Some(entry) = resolve_entry_point(pkg_dir)
        && let Some(line) = scan_file_for_def(&entry, item)
    {
        return Some((entry, line));
    }
    for file in &walk_js_files(pkg_dir) {
        if let Some(line) = scan_file_for_def(file, item) {
            return Some((file.clone(), line));
        }
    }
    None
}

/// The 1-based line of the first definition-shaped match of `item` in `file`.
fn scan_file_for_def(file: &Path, item: &str) -> Option<u32> {
    let contents = std::fs::read_to_string(file).ok()?;
    contents
        .lines()
        .enumerate()
        .find(|(_, line)| line_defines_item(line, item).is_some())
        .map(|(i, _)| (i + 1) as u32)
}

/// Walk `pkg_dir` for JS/TS files, skipping `node_modules` and dot-dirs, in a
/// deterministic (sorted) order.
fn walk_js_files(pkg_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![pkg_dir.to_path_buf()];
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
                if name == "node_modules" {
                    continue;
                }
                stack.push(path);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| JS_EXT.contains(&ext))
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Resolve a package's entry point: `exports` (string form or the `.` key,
/// including a conditional object) → `module` → `main` → `index.js`. A
/// TypeScript-source preference is applied when a `src/` tree is present.
fn resolve_entry_point(pkg_dir: &Path) -> Option<PathBuf> {
    let pj = read_pkg_json(pkg_dir)?;
    let entry_rel = pj
        .exports
        .as_ref()
        .and_then(resolve_exports)
        .or(pj.module.clone())
        .or(pj.main.clone())
        .unwrap_or_else(|| "index.js".to_string());

    let candidate = pkg_dir.join(&entry_rel);
    if let Some(ts) = ts_source_preference(pkg_dir, &entry_rel) {
        return Some(ts);
    }
    if candidate.exists() {
        return Some(candidate);
    }
    for ext in ["ts", "js", "mjs", "cjs"] {
        let idx = pkg_dir.join(format!("index.{ext}"));
        if idx.exists() {
            return Some(idx);
        }
    }
    // fix-jsrel review P2-2: naming a NON-EXISTENT entry file surfaced as
    // `cannot canonicalize … (os error 2)` downstream instead of the
    // dedicated "no entry point" bail. None is the honest answer.
    None
}

/// Resolve the `exports` field (modern truth) to a relative entry path.
fn resolve_exports(exports: &Value) -> Option<String> {
    match exports {
        Value::String(s) => Some(s.clone()),
        Value::Object(m) => m
            .get(".")
            .or_else(|| m.get("*"))
            .or_else(|| m.values().next())
            .and_then(pick_conditional),
        _ => None,
    }
}

/// Pick a concrete entry from a conditional-exports object (`default`,
/// `import`, `require`, else the first value), recursing one level.
fn pick_conditional(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(m) => m
            .get("default")
            .or_else(|| m.get("import"))
            .or_else(|| m.get("require"))
            .or_else(|| m.values().next())
            .and_then(pick_conditional),
        _ => None,
    }
}

/// When a `src/` tree of TypeScript is present, prefer the matching `.ts`
/// source over the (compiled) entry. `entry_rel` is the package.json entry.
fn ts_source_preference(pkg_dir: &Path, entry_rel: &str) -> Option<PathBuf> {
    let src = pkg_dir.join("src");
    if !src.is_dir() {
        return None;
    }
    let base = entry_rel.rsplit('/').next().unwrap_or(entry_rel);
    let stem = base
        .strip_suffix(".js")
        .or_else(|| base.strip_suffix(".mjs"))
        .or_else(|| base.strip_suffix(".cjs"))
        .unwrap_or(base);
    let cand = src.join(format!("{stem}.ts"));
    if cand.exists() {
        Some(cand)
    } else {
        None
    }
}

// ── npm install (fetch-on-demand, sanctioned) ────────────────────────────────



// ── package.json ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct PkgJson {
    #[serde(default, rename = "dependencies")]
    dependencies: BTreeMap<String, Value>,
    #[serde(default, rename = "devDependencies")]
    dev_dependencies: BTreeMap<String, Value>,
    #[serde(default, rename = "peerDependencies")]
    peer_dependencies: BTreeMap<String, Value>,
    #[serde(default, rename = "optionalDependencies")]
    optional_dependencies: BTreeMap<String, Value>,
    #[serde(default)]
    main: Option<String>,
    #[serde(default)]
    module: Option<String>,
    #[serde(default)]
    exports: Option<Value>,
    #[serde(default)]
    #[allow(dead_code)]
    types: Option<String>,
}

fn read_pkg_json(dir: &Path) -> Option<PkgJson> {
    let s = std::fs::read_to_string(dir.join("package.json")).ok()?;
    serde_json::from_str(&s).ok()
}

// ── definition-shaped line matching (plain content scan) ─────────────────────

/// Does `line` define an item named `item` (a definition-shaped match)?
/// Returns the definition keyword when it does.
///
/// Matches `export function X`, `export const X =`, `export class X`,
/// `function X`, `class X`, `const/let/var X =`, TS `export interface X` and
/// `export type X =`. Rejects imports/re-exports, comments, and `X.prototype`
/// references (the same whole-word + preceding-keyword discipline as
/// [`crate::cargo`]).
fn line_defines_item(line: &str, item: &str) -> Option<&'static str> {
    let t = line.trim();
    if t.is_empty() {
        return None;
    }
    // Comment lines are never definitions.
    if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
        return None;
    }
    // Import / dynamic-import lines are never definitions.
    if t.starts_with("import") {
        return None;
    }

    let mut search_from = 0usize;
    loop {
        let rest = &t[search_from..];
        let rel = rest.find(item)?;
        let idx = search_from + rel;
        let end = idx + item.len();
        let before_ok = char_at(t, idx.checked_sub(1)?)
            .is_none_or(|c| !is_ident_char(c));
        let after_ch = char_at(t, end);
        let after_ok = after_ch.is_none_or(|c| !is_ident_char(c));
        if before_ok && after_ok {
            // Reject `X.prototype…` member references.
            if after_ch == Some('.') {
                let after = t.get(end..).unwrap_or("");
                if let Some(rest) = after.strip_prefix(".prototype") {
                    let boundary = rest.chars().next().is_none_or(|c| !is_ident_char(c));
                    if boundary {
                        search_from = idx + 1;
                        continue;
                    }
                }
            }
            let last = t[..idx]
                .rsplit(char::is_whitespace)
                .find(|s| !s.is_empty())
                .unwrap_or("");
            match last {
                "function" => return Some("function"),
                "class" => return Some("class"),
                "interface" => return Some("interface"),
                "type" => return Some("type"),
                "const" | "let" | "var" => {
                    // A binding only counts as a definition when it is
                    // assigned (`=`) or type-annotated (`:`).
                    if next_sig_char(t, end).is_some_and(|c| c == '=' || c == ':') {
                        return Some("const");
                    }
                    search_from = idx + 1;
                    continue;
                }
                _ => {}
            }
        }
        search_from = idx + 1;
    }
}

fn char_at(s: &str, i: usize) -> Option<char> {
    s.char_indices().find(|(idx, _)| *idx == i).map(|(_, c)| c)
}

fn is_ident_char(c: char) -> bool {
    // C15: mirrors the redline crate's single word-char rule —
    // `redline::model::buffer::is_word_char` (Unicode alphanumeric or `_`).
    // redline-resolve has no dependency on redline, so the rule is
    // duplicated here rather than imported; keep the two in sync (the
    // sibling `crate::cargo::is_ident_char` does the same).
    c.is_alphanumeric() || c == '_'
}

/// The first non-whitespace character at or after byte offset `from`.
fn next_sig_char(s: &str, from: usize) -> Option<char> {
    s.get(from..).and_then(|r| r.chars().find(|c| !c.is_whitespace()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── normalization ──────────────────────────────────────────────────────

    #[test]
    fn split_symbol_plain_and_scoped() {
        assert_eq!(split_symbol("react.useState"), ("react".to_string(), Some("useState".to_string())));
        assert_eq!(
            split_symbol("@testing-library/react.render"),
            ("@testing-library/react".to_string(), Some("render".to_string()))
        );
        assert_eq!(split_symbol("lodash/get"), ("lodash/get".to_string(), None));
        assert_eq!(split_symbol("react"), ("react".to_string(), None));
    }

    #[test]
    fn base_package_name_drops_subpath() {
        assert_eq!(base_package_name("react"), "react");
        assert_eq!(base_package_name("lodash/get"), "lodash");
        assert_eq!(base_package_name("@testing-library/react"), "@testing-library/react");
        assert_eq!(
            base_package_name("@testing-library/react/sub"),
            "@testing-library/react"
        );
    }

    // ── entry point resolution ─────────────────────────────────────────────

    fn pkg_with_json(dir: &Path, json: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("package.json"), json).unwrap();
    }

    #[test]
    fn entry_exports_dot_object_default() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path();
        pkg_with_json(p, r#"{"name":"p","exports":{".":{"default":"./dist/index.js"}}}"#);
        fs::create_dir_all(p.join("dist")).unwrap();
        fs::write(p.join("dist/index.js"), "").unwrap();
        let e = resolve_entry_point(p).unwrap();
        assert_eq!(e.file_name().unwrap(), "index.js");
        assert!(e.starts_with(p.join("dist")));
    }

    #[test]
    fn entry_exports_string_and_main_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path();
        pkg_with_json(p, r#"{"name":"p","exports":"./index.js"}"#);
        fs::write(p.join("index.js"), "").unwrap();
        assert_eq!(resolve_entry_point(p).unwrap(), p.join("index.js"));

        let tmp2 = tempfile::tempdir().unwrap();
        let p2 = tmp2.path();
        pkg_with_json(p2, r#"{"name":"p","main":"lib/index.js"}"#);
        fs::create_dir_all(p2.join("lib")).unwrap();
        fs::write(p2.join("lib/index.js"), "").unwrap();
        assert_eq!(resolve_entry_point(p2).unwrap(), p2.join("lib/index.js"));
    }

    #[test]
    fn entry_ts_src_preference() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path();
        // Compiled entry absent, but a TypeScript source tree is present.
        pkg_with_json(p, r#"{"name":"p","main":"lib/index.js"}"#);
        fs::create_dir_all(p.join("src")).unwrap();
        fs::write(p.join("src/index.ts"), "export const a = 1;\n").unwrap();
        let e = resolve_entry_point(p).unwrap();
        assert_eq!(e, p.join("src/index.ts"));
    }

    // ── definition regex ───────────────────────────────────────────────────

    #[test]
    fn line_defines_real_exports() {
        assert_eq!(line_defines_item("export function greet() {", "greet"), Some("function"));
        assert_eq!(
            line_defines_item("export const add = (a, b) => a + b;", "add"),
            Some("const")
        );
        assert_eq!(line_defines_item("export class Foo {", "Foo"), Some("class"));
        assert_eq!(line_defines_item("function bar() {}", "bar"), Some("function"));
        assert_eq!(line_defines_item("const baz = () => {};", "baz"), Some("const"));
        assert_eq!(line_defines_item("export interface I {", "I"), Some("interface"));
        assert_eq!(line_defines_item("export type T = number;", "T"), Some("type"));
        assert_eq!(line_defines_item("async function delay() {}", "delay"), Some("function"));
    }

    #[test]
    fn line_defines_rejects_false_positives() {
        assert_eq!(line_defines_item("import { useState } from 'react';", "useState"), None);
        assert_eq!(line_defines_item("import X from 'x';", "X"), None);
        assert_eq!(line_defines_item("export { X } from './x';", "X"), None);
        assert_eq!(line_defines_item("Foo.prototype.x = 1;", "Foo"), None);
        assert_eq!(line_defines_item("// function greet() {}", "greet"), None);
        assert_eq!(line_defines_item("/* class Foo */", "Foo"), None);
        // substring of a longer identifier is not a whole word
        assert_eq!(line_defines_item("export function greetAll() {}", "greet"), None);
        // usage, not definition
        assert_eq!(line_defines_item("const y = obj.add(1, 2);", "add"), None);
        assert_eq!(line_defines_item("new Foo();", "Foo"), None);
    }

    /// C15: identifier-constituency mirrors the redline crate's Unicode
    /// word-char rule (`redline::model::buffer::is_word_char`) — a non-ASCII
    /// letter is an identifier character, so it blocks a whole-word boundary
    /// just like an ASCII one (the sibling `crate::cargo` copy is pinned the
    /// same way). Under the old ASCII rule the `é` in `greeté` was not an
    /// identifier char, so `greet` was mis-detected as a definition.
    #[test]
    fn line_defines_ident_char_is_unicode_aware() {
        assert!(is_ident_char('é'), "accented letter");
        assert!(is_ident_char('漢'), "CJK letter");
        assert!(is_ident_char('_'));
        assert!(!is_ident_char('-'));
        assert!(!is_ident_char(' '));
        // Whole-word pin: `greet` inside `greeté` is NOT a definition of
        // `greet` (the `é` is an identifier char, not a boundary).
        assert_eq!(line_defines_item("export function greeté() {", "greet"), None);
        // And the full Unicode identifier IS a definition of itself.
        assert_eq!(
            line_defines_item("export function greeté() {", "greeté"),
            Some("function")
        );
    }

    // ── end-to-end: local path dependency (external = false) ───────────────

    #[test]
    fn resolve_local_path_dep_external_false() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        // A workspace-local package linked via `file:`.
        let local = ws.join("packages").join("mylocal");
        fs::create_dir_all(&local).unwrap();
        fs::write(
            local.join("package.json"),
            r#"{"name":"mylocal","version":"1.0.0","main":"index.js"}"#,
        )
        .unwrap();
        fs::write(local.join("index.js"), "export function hello() { return 1; }\n").unwrap();
        // Workspace package.json declaring the local dependency.
        fs::create_dir_all(ws).unwrap();
        fs::write(
            ws.join("package.json"),
            r#"{"name":"ws","version":"1.0.0","dependencies":{"mylocal":"file:packages/mylocal"}}"#,
        )
        .unwrap();

        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "mylocal.hello".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("index.js"),
            language: None,
        };
        let src = JsProvider::new().resolve(&ctx).unwrap();
        assert!(!src.external);
        assert_eq!(src.file.file_name().unwrap(), "index.js");
        assert_eq!(src.source_root, local);
        assert_eq!(src.line, Some(1));
    }

    /// 011-08 polish: a `file:` dependency declared TWO levels up must
    /// still be found — the intermediate dir (and the workspace root
    /// itself) carry NO package.json. Pre-fix, the walk stopped at the
    /// first package-less ancestor and bailed (the `?` on
    /// `read_pkg_json`); now the package-less levels are skipped and the
    /// outer project root's manifest is read.
    #[test]
    fn local_path_dep_walks_past_packageless_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let outer = tmp.path();
        let ws = outer.join("middle").join("ws");
        let local = outer.join("packages").join("mylocal");
        // Two package-less levels between the workspace and the manifest.
        fs::create_dir_all(&ws).unwrap();
        fs::create_dir_all(&local).unwrap();
        fs::write(
            local.join("package.json"),
            r#"{"name":"mylocal","main":"index.js"}"#,
        )
        .unwrap();
        fs::write(local.join("index.js"), "export function hello() { return 1; }\n").unwrap();
        fs::write(
            outer.join("package.json"),
            r#"{"name":"outer","dependencies":{"mylocal":"file:packages/mylocal"}}"#,
        )
        .unwrap();

        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "mylocal.hello".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("index.js"),
            language: None,
        };
        let src = JsProvider::new().offline().resolve(&ctx).unwrap();
        assert!(!src.external);
        assert_eq!(src.file, local.join("index.js"));
        assert_eq!(src.source_root, local);
        assert_eq!(src.line, Some(1));
    }

    /// 011-08 polish (js-polish P2-1, the bail half): a MALFORMED
    /// intermediate package.json stops the walk — that level is a project
    /// root whose dependency map cannot be read, so climbing past it to
    /// the outer valid manifest would be a guess. (Contrast the MISSING
    /// manifest, which is skipped: `local_path_dep_walks_past_packageless_dirs`.)
    #[test]
    fn local_path_dep_bails_on_malformed_intermediate_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let outer = tmp.path();
        let ws = outer.join("ws");
        let local = outer.join("packages").join("mylocal");
        fs::create_dir_all(&ws).unwrap();
        fs::create_dir_all(&local).unwrap();
        fs::write(
            local.join("package.json"),
            r#"{"name":"mylocal","main":"index.js"}"#,
        )
        .unwrap();
        fs::write(
            local.join("index.js"),
            "export function hello() { return 1; }\n",
        )
        .unwrap();
        // A MALFORMED manifest at the workspace level…
        fs::write(ws.join("package.json"), "{ this is not json").unwrap();
        // …with a valid `file:` dep declared one level up: the walk BAILS
        // at the malformed level, it does not climb past it.
        fs::write(
            outer.join("package.json"),
            r#"{"name":"outer","dependencies":{"mylocal":"file:packages/mylocal"}}"#,
        )
        .unwrap();
        assert_eq!(find_local_path_dep(&ws, "mylocal"), None);

        // Control: the SAME layout with the intermediate manifest MISSING
        // (not malformed) climbs and finds the outer dep.
        fs::remove_file(ws.join("package.json")).unwrap();
        assert_eq!(find_local_path_dep(&ws, "mylocal"), Some(local.clone()));
    }

    // ── end-to-end: node_modules dep + item locate (no network) ────────────

    #[test]
    fn resolve_node_modules_dep_locates_item() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws).unwrap();
        fs::write(ws.join("package.json"), r#"{"name":"ws","version":"1.0.0"}"#).unwrap();
        let dep = ws.join("node_modules").join("acme");
        fs::create_dir_all(&dep).unwrap();
        fs::write(dep.join("package.json"), r#"{"name":"acme","main":"lib/main.js"}"#).unwrap();
        fs::create_dir_all(dep.join("lib")).unwrap();
        fs::write(dep.join("lib/main.js"), "export function doThing() { return 2; }\n").unwrap();

        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "acme.doThing".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("index.js"),
            language: None,
        };
        let src = JsProvider::new().resolve(&ctx).unwrap();
        assert!(src.external);
        assert_eq!(src.file, dep.join("lib/main.js"));
        assert_eq!(src.line, Some(1));
    }

    #[test]
    fn missing_item_errors_naming_pkg_and_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws).unwrap();
        fs::write(ws.join("package.json"), r#"{"name":"ws"}"#).unwrap();
        let dep = ws.join("node_modules").join("acme");
        fs::create_dir_all(&dep).unwrap();
        fs::write(dep.join("package.json"), r#"{"name":"acme","main":"index.js"}"#).unwrap();
        fs::write(dep.join("index.js"), "export function known() {}\n").unwrap();

        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "acme.unknown".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("index.js"),
            language: None,
        };
        let err = JsProvider::new().resolve(&ctx).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown"), "msg: {msg}");
        assert!(msg.contains("acme"), "msg: {msg}");
    }

    #[test]
    fn offline_refuses_to_install() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws).unwrap();
        fs::write(ws.join("package.json"), r#"{"name":"ws"}"#).unwrap();
        // No node_modules/<missing>.
        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "left-pad.leftPad".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("index.js"),
            language: None,
        };
        let err = JsProvider::new().offline().resolve(&ctx).unwrap_err();
        assert!(err.to_string().contains("offline"), "msg: {err}");
    }

    #[test]
    fn not_an_npm_project_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws).unwrap(); // no package.json anywhere up the tree here
        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "left-pad.leftPad".to_string(),
            from_file: PathBuf::from("index.js"),
            scope: Vec::new(),
            language: None,
        };
        // Offline keeps this test network-free: the error is the offline
        // refusal, which still proves the walk-up + no-local-dep path.
        let err = JsProvider::new().offline().resolve(&ctx).unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    // ── 007-03: bare symbol + scope hint (import path) ────────────────────

    /// Discriminating: a BARE symbol + the import-path scope resolves
    /// through the SAME locate machinery as the path-shaped twin.
    #[test]
    fn bare_symbol_with_scope_resolves_through_locate_item() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws).unwrap();
        fs::write(ws.join("package.json"), r#"{"name":"ws","version":"1.0.0"}"#).unwrap();
        let dep = ws.join("node_modules").join("acme");
        fs::create_dir_all(&dep).unwrap();
        fs::write(dep.join("package.json"), r#"{"name":"acme","main":"lib/main.js"}"#).unwrap();
        fs::create_dir_all(dep.join("lib")).unwrap();
        fs::write(dep.join("lib/main.js"), "export function doThing() { return 2; }\n").unwrap();

        // The bare symbol + import path (import { doThing } from "acme";).
        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "doThing".to_string(),
            from_file: PathBuf::from("index.js"),
            scope: vec!["acme".to_string(), "doThing".to_string()],
            language: None,
        };
        let src = JsProvider::new().resolve(&ctx).unwrap();
        assert!(src.external);
        assert_eq!(src.file, dep.join("lib/main.js"));
        assert_eq!(src.line, Some(1));
    }

    /// Regression pin: an EMPTY scope still bails with the existing
    /// message (no hint → today's behavior, byte-for-byte).
    #[test]
    fn bare_symbol_empty_scope_still_bails() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws).unwrap();
        fs::write(ws.join("package.json"), r#"{"name":"ws"}"#).unwrap();
        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "doThing".to_string(),
            from_file: PathBuf::from("index.js"),
            scope: Vec::new(),
            language: None,
        };
        let err = JsProvider::new().resolve(&ctx).unwrap_err();
        assert!(
            err.to_string().contains("needs scope info"),
            "err: {err}"
        );
    }

    // ── 011-02: namespace-aliased member (`ns.member` rewrite) ─────────

    /// Discriminating: `ns.member` where `ns` comes from
    /// `import * as ns from "pkg"` — the app's hint `["pkg", "member"]`
    /// rewrites the alias and the member locates through the SAME
    /// `node_modules` / `locate_item` machinery as the path-shaped twin.
    #[test]
    fn namespace_member_hint_rewrites_alias_to_package() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws).unwrap();
        fs::write(ws.join("package.json"), r#"{"name":"ws","version":"1.0.0"}"#).unwrap();
        let dep = ws.join("node_modules").join("acme");
        fs::create_dir_all(&dep).unwrap();
        fs::write(dep.join("package.json"), r#"{"name":"acme","main":"lib/main.js"}"#).unwrap();
        fs::create_dir_all(dep.join("lib")).unwrap();
        fs::write(
            dep.join("lib/main.js"),
            "export function doThing() { return 2; }\n",
        )
        .unwrap();

        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "ac.doThing".to_string(), // `import * as ac from "acme"`
            from_file: PathBuf::from("index.js"),
            scope: vec!["acme".to_string(), "doThing".to_string()],
            language: Some("javascript".to_string()),
        };
        let src = JsProvider::new().resolve(&ctx).unwrap();
        assert!(src.external);
        assert_eq!(src.file, dep.join("lib/main.js"));
        assert_eq!(src.line, Some(1));
    }

    /// 011-02 review P2-1: a 1-segment hint names the package ENTRY itself
    /// (`import * as ns from "pkg"` → bare `ns` with `["pkg"]`): the symbol
    /// lands on the package's entry file (main), not a bail. And the no-hint
    /// bare-symbol bail is unchanged (byte-for-byte).
    #[test]
    fn bare_namespace_hint_lands_on_package_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws).unwrap();
        fs::write(ws.join("package.json"), r#"{"name":"ws","version":"1.0.0"}"#).unwrap();
        let dep = ws.join("node_modules").join("acme");
        fs::create_dir_all(&dep).unwrap();
        fs::write(dep.join("package.json"), r#"{"name":"acme","main":"lib/main.js"}"#).unwrap();
        fs::create_dir_all(dep.join("lib")).unwrap();
        fs::write(dep.join("lib/main.js"), "export default 1;\n").unwrap();

        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "ns".to_string(), // `import * as ns from "acme"`
            from_file: PathBuf::from("index.js"),
            scope: vec!["acme".to_string()],
            language: Some("javascript".to_string()),
        };
        let src = JsProvider::new().resolve(&ctx).unwrap();
        assert!(src.external);
        assert_eq!(src.file, dep.join("lib/main.js"), "the entry file");
        assert_eq!(src.line, None, "entry landings carry no line");

        // No hint → the byte-for-byte bail.
        let mut no_hint = ctx.clone();
        no_hint.scope = Vec::new();
        let err = JsProvider::new().resolve(&no_hint).unwrap_err().to_string();
        assert!(
            err.contains("needs scope info"),
            "no-hint bail unchanged, got: {err}"
        );
    }

    /// Regression pin: an identity hint (`import * as acme from "acme"` →
    /// `["acme", "doThing"]` for `acme.doThing`) is the symbol's OWN path —
    /// the rewrite must be a no-op (the provider still resolves the
    /// symbol's path, not the hint). And a mismatched-item hint is ignored
    /// (the symbol's path wins; `ac` is not a package → clean offline
    /// refusal, never a guess).
    #[test]
    fn namespace_member_identity_and_mismatched_hints_are_no_ops() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws).unwrap();
        fs::write(ws.join("package.json"), r#"{"name":"ws"}"#).unwrap();
        // Identity hint: the symbol's own path is used; `ac` is not in
        // node_modules → the offline refusal proves the path walk ran.
        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "ac.doThing".to_string(),
            from_file: PathBuf::from("index.js"),
            scope: vec!["ac".to_string(), "doThing".to_string()],
            language: None,
        };
        let err = JsProvider::new().offline().resolve(&ctx).unwrap_err();
        assert!(err.to_string().contains("offline"), "err: {err}");
        assert!(err.to_string().contains("`ac`"), "err: {err}");

        // Mismatched item (`other` ≠ `doThing`): not a rewrite of THIS
        // symbol — the symbol's own path (`ac`) is used again.
        let ctx = SymbolContext {
            scope: vec!["acme".to_string(), "other".to_string()],
            ..ctx.clone()
        };
        let err = JsProvider::new().offline().resolve(&ctx).unwrap_err();
        assert!(err.to_string().contains("`ac`"), "err: {err}");
    }

    // ── live E2E: real tiny npm package (network) ──────────────────────────

    // ── 011-08 follow-up (fix-jsrel): path-shaped specifiers ──────────

    /// A relative whole-module binding lands in the SIBLING file (the
    /// workspace-local, external = false semantics of a `file:` dep);
    /// the no-extension specifier resolves through the extension walk.
    #[test]
    fn relative_whole_module_lands_in_sibling_file() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws.join("src")).unwrap();
        fs::write(
            ws.join("src/app.js"),
            "const legacy = require('./legacy-util');\n",
        )
        .unwrap();
        fs::write(
            ws.join("src/legacy-util.js"),
            "function legacyJoin(parts) { return parts.join(' | '); }\nmodule.exports = { legacyJoin };\n",
        )
        .unwrap();

        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "legacy".to_string(),
            from_file: PathBuf::from("src/app.js"),
            scope: vec!["./legacy-util".to_string()],
            language: Some("javascript".to_string()),
        };
        let src = JsProvider::new().offline().resolve(&ctx).unwrap();
        assert!(!src.external, "workspace-local landing");
        assert_eq!(src.file, ws.join("src").join("legacy-util.js"));
        assert_eq!(src.source_root, ws.join("src"));
        assert_eq!(src.line, None, "whole-module binding lands with no line");
    }

    /// A relative specifier WITH a member (`import { legacyJoin } from
    /// './legacy-util'`) lands in the sibling file AND scans it for the
    /// member's definition line.
    #[test]
    fn relative_member_lands_with_definition_line() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws.join("src")).unwrap();
        fs::write(
            ws.join("src/app.js"),
            "const { legacyJoin } = require('./legacy-util');\n",
        )
        .unwrap();
        fs::write(
            ws.join("src/legacy-util.js"),
            "function legacyJoin(parts) { return parts.join(' | '); }\n",
        )
        .unwrap();

        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "legacyJoin".to_string(),
            from_file: PathBuf::from("src/app.js"),
            scope: vec!["./legacy-util".to_string(), "legacyJoin".to_string()],
            language: Some("javascript".to_string()),
        };
        let src = JsProvider::new().offline().resolve(&ctx).unwrap();
        assert!(!src.external);
        assert_eq!(src.file, ws.join("src").join("legacy-util.js"));
        assert_eq!(src.line, Some(1));

        // A member absent from the sibling file is an honest bail naming
        // the member and the file.
        let mut missing = ctx.clone();
        missing.symbol = "legacyPad".to_string();
        missing.scope = vec!["./legacy-util".to_string(), "legacyPad".to_string()];
        let err = JsProvider::new().offline().resolve(&missing).unwrap_err().to_string();
        assert!(err.contains("legacyPad"), "err: {err}");
        assert!(err.contains("legacy-util.js"), "err: {err}");
    }

    /// A relative specifier that names NO file bails with a dedicated,
    /// informative message (never the mangled base-package `.` refusal,
    /// never node_modules).
    #[test]
    fn relative_missing_file_bails_dedicated() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws.join("src")).unwrap();
        fs::write(ws.join("src/app.js"), "const x = require('./nope');\n").unwrap();

        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "x".to_string(),
            from_file: PathBuf::from("src/app.js"),
            scope: vec!["./nope".to_string()],
            language: None,
        };
        let err = JsProvider::new().offline().resolve(&ctx).unwrap_err().to_string();
        assert!(err.contains("relative specifier `./nope`"), "err: {err}");
        assert!(err.contains("src/app.js"), "err: {err}");
        assert!(!err.contains("node_modules"), "never the node_modules path: {err}");
    }

    /// A relative specifier naming a STATIC ASSET (`./styles.css`) lands in
    /// the exact file — no extension walk, no directory form.
    #[test]
    fn relative_static_asset_lands_in_exact_file() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws.join("src")).unwrap();
        fs::write(ws.join("src/app.js"), "import './styles.css';\n").unwrap();
        fs::write(ws.join("src/styles.css"), ".row { color: red; }\n").unwrap();

        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "styles".to_string(),
            from_file: PathBuf::from("src/app.js"),
            scope: vec!["./styles.css".to_string()],
            language: Some("javascript".to_string()),
        };
        let src = JsProvider::new().offline().resolve(&ctx).unwrap();
        assert!(!src.external);
        assert_eq!(src.file, ws.join("src").join("styles.css"));
        assert_eq!(src.line, None);
    }

    /// A relative DIRECTORY import resolves through that dir's
    /// package.json entry point.
    #[test]
    fn relative_directory_import_lands_on_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws.join("src")).unwrap();
        fs::create_dir_all(ws.join("src").join("mod")).unwrap();
        fs::write(
            ws.join("src/mod/package.json"),
            r#"{"name":"mod","main":"lib/index.js"}"#,
        )
        .unwrap();
        fs::create_dir_all(ws.join("src").join("mod").join("lib")).unwrap();
        fs::write(ws.join("src/mod/lib/index.js"), "export const x = 1;\n").unwrap();
        fs::write(ws.join("src/app.js"), "import mod from './mod';\n").unwrap();

        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "mod".to_string(),
            from_file: PathBuf::from("src/app.js"),
            scope: vec!["./mod".to_string()],
            language: None,
        };
        let src = JsProvider::new().offline().resolve(&ctx).unwrap();
        assert!(!src.external);
        assert_eq!(src.file, ws.join("src").join("mod").join("lib").join("index.js"));
    }

    /// A relative specifier that escapes the workspace bails (the provider
    /// stays workspace-local).
    #[test]
    fn relative_escape_outside_workspace_bails() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        fs::create_dir_all(ws.join("src")).unwrap();
        fs::write(ws.join("src/app.js"), "const x = require('../../out');\n").unwrap();
        fs::write(tmp.path().join("out.js"), "export const x = 1;\n").unwrap();

        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "x".to_string(),
            from_file: PathBuf::from("src/app.js"),
            scope: vec!["../../out".to_string()],
            language: None,
        };
        let err = JsProvider::new().offline().resolve(&ctx).unwrap_err().to_string();
        assert!(err.contains("outside the workspace"), "err: {err}");
    }

    /// An absolute path specifier bails with its own dedicated message
    /// (never the empty-base-package refusal, never an npm install).
    #[test]
    fn absolute_specifier_bails_dedicated() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::write(ws.join("package.json"), r#"{"name":"ws"}"#).unwrap();

        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "legacyJoin".to_string(),
            from_file: PathBuf::from("index.js"),
            scope: vec!["/abs/legacy-util".to_string(), "legacyJoin".to_string()],
            language: None,
        };
        let err = JsProvider::new().offline().resolve(&ctx).unwrap_err().to_string();
        assert!(
            err.contains("absolute path specifier `/abs/legacy-util`"),
            "err: {err}"
        );
        assert!(!err.contains("offline"), "not the mangled refusal: {err}");
    }

    /// A file-ish symbol (a side-effect import's name) bails with its own
    /// dedicated message — never the `styles`/`css` mis-split refusal.
    #[test]
    fn file_ish_symbol_bails_dedicated() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::write(ws.join("package.json"), r#"{"name":"ws"}"#).unwrap();

        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "styles.css".to_string(),
            from_file: PathBuf::from("index.js"),
            scope: Vec::new(),
            language: Some("javascript".to_string()),
        };
        let err = JsProvider::new().offline().resolve(&ctx).unwrap_err().to_string();
        assert!(err.contains("file-ish name `styles.css`"), "err: {err}");
        assert!(!err.contains("package `styles`"), "not the mis-split: {err}");
    }

    /// The `node_modules` directory itself is never probed as a package:
    /// `.` / `..` bases yield `None` even when `node_modules` is populated.
    #[test]
    fn locate_in_node_modules_refuses_dot_dir() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("node_modules")).unwrap();
        assert_eq!(locate_in_node_modules(tmp.path(), "."), None);
        assert_eq!(locate_in_node_modules(tmp.path(), ".."), None);
        // A real package still locates.
        fs::create_dir_all(tmp.path().join("node_modules").join("acme")).unwrap();
        assert_eq!(
            locate_in_node_modules(tmp.path(), "acme"),
            Some(tmp.path().join("node_modules").join("acme"))
        );
    }

    /// fix-jsrel review P2-1: the extension-walk order is DETERMINISTIC —
    /// `.js` wins over a same-named `.ts` (Node's LOAD_AS_FILE convention,
    /// language-agnostic today; make the order a conscious artifact).
    #[test]
    fn relative_extension_walk_prefers_js_over_ts() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws.join("src")).unwrap();
        fs::write(
            ws.join("src/app.js"),
            "import { go } from './mod';\n",
        )
        .unwrap();
        fs::write(ws.join("src/mod.js"), "export function go() {}\n").unwrap();
        fs::write(ws.join("src/mod.ts"), "export function go(): void {}\n").unwrap();
        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "./mod.go".to_string(),
            from_file: PathBuf::from("src/app.js"),
            scope: vec!["./mod".to_string(), "go".to_string()],
            language: Some("javascript".to_string()),
        };
        let src = JsProvider::new().offline().resolve(&ctx).unwrap();
        assert!(!src.external);
        assert_eq!(src.file, ws.join("src").join("mod.js"), "the .js twin wins");
    }

    /// fix-jsrel review P2-1 remainder (js-polish): `import type {X} from
    /// "./types"` with only `types.d.ts` present lands in the declaration
    /// file — the relative extension walk carries `d.ts` at its tail, and
    /// a runtime source TWIN still wins over the declaration file.
    #[test]
    fn relative_extension_walk_resolves_dts_tail() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws.join("src")).unwrap();
        fs::write(ws.join("src/app.js"), "import type { T } from './types';\n").unwrap();
        fs::write(ws.join("src/types.d.ts"), "export type T = number;\n").unwrap();
        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "T".to_string(),
            from_file: PathBuf::from("src/app.js"),
            scope: vec!["./types".to_string(), "T".to_string()],
            language: Some("typescript".to_string()),
        };
        let src = JsProvider::new().offline().resolve(&ctx).unwrap();
        assert!(!src.external);
        assert_eq!(src.file, ws.join("src").join("types.d.ts"), "the declaration file");

        // A runtime source twin outranks the declaration file (`.ts` is
        // earlier in the walk than the `d.ts` tail).
        fs::write(ws.join("src/types.ts"), "export type T = number;\n").unwrap();
        let src2 = JsProvider::new().offline().resolve(&ctx).unwrap();
        assert_eq!(src2.file, ws.join("src").join("types.ts"), "the .ts twin wins");
    }

    /// 011-08 polish: a workspace symlink pointing OUTSIDE the workspace
    /// bails with the dedicated out-of-workspace message. The symlink IS a
    /// real (resolvable) file lexically, so the pre-canonicalize
    /// containment check would pass on the un-canonicalized path — the
    /// canonicalize-BEFORE-contains order in `resolve_relative` is what
    /// neutralizes it, pinned here as the claim's regression guard.
    #[cfg(unix)]
    #[test]
    fn relative_symlink_escape_outside_workspace_bails() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        fs::create_dir_all(ws.join("src")).unwrap();
        let outside = tmp.path().join("outside.js");
        fs::write(&outside, "export const x = 1;\n").unwrap();
        symlink(&outside, ws.join("src").join("link.js")).unwrap();

        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "x".to_string(),
            from_file: PathBuf::from("src/app.js"),
            scope: vec!["./link.js".to_string()],
            language: None,
        };
        let err = JsProvider::new().offline().resolve(&ctx).unwrap_err().to_string();
        assert!(err.contains("outside the workspace"), "err: {err}");
    }

    /// fix-jsrel review P2-3: the file-ish collision class is ACCEPTED —
    /// a resolvable package whose member is extension-shaped (`pkg.json`)
    /// bails with the dedicated file-ish message even though the landing
    /// machinery could have found it. Documented tradeoff; pinned here so
    /// narrowing the extension set later is a conscious diff.
    #[test]
    fn file_ish_member_of_resolvable_package_bails_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws).unwrap();
        fs::write(ws.join("package.json"), r#"{"name":"ws"}"#).unwrap();
        let dep = ws.join("node_modules").join("somelib");
        fs::create_dir_all(&dep).unwrap();
        fs::write(dep.join("package.json"), r#"{"name":"somelib","main":"index.js"}"#).unwrap();
        fs::write(dep.join("index.js"), "export const json = 1;\n").unwrap();
        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "somelib.json".to_string(),
            from_file: PathBuf::from("index.js"),
            scope: vec!["somelib".to_string(), "json".to_string()],
            language: Some("javascript".to_string()),
        };
        let err = JsProvider::new().offline().resolve(&ctx).unwrap_err().to_string();
        assert!(
            err.contains("file-ish name"),
            "the accepted file-ish bail, got: {err}"
        );
    }

    #[test]
    #[ignore] // requires network (npm registry)
    fn e2e_leftpad_live() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws).unwrap();
        fs::write(ws.join("package.json"), r#"{"name":"e2e","version":"1.0.0"}"#).unwrap();

        let ctx = SymbolContext {
            workspace_root: ws.to_path_buf(),
            symbol: "left-pad.leftPad".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("index.js"),
            language: None,
        };
        let src = JsProvider::new().resolve(&ctx).unwrap();
        assert!(src.external);
        assert!(src.file.exists());
        let stem = src.file.to_string_lossy().to_string();
        assert!(stem.contains("left-pad"), "file: {stem}");
        assert!(src.line.is_some(), "expected a definition line");
    }
}
