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
//! Plain `node_modules` only — no yarn-PnP, no npm-workspaces resolution.
//! Installs are always local (never `-g`/global).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::{run_with_timeout, ResolvedSource, SymbolContext, ToolingProvider};

/// File extensions considered JavaScript/TypeScript sources.
const JS_EXT: &[&str] = &["js", "mjs", "cjs", "ts", "tsx", "jsx", "mts", "cts"];

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
        let (pkg_spec, item) = split_symbol(&ctx.symbol);
        // A bare (dot-free) symbol has no package path; resolving it to a
        // concrete package would require scope info (tree-sitter) not yet
        // provided by the app. Bail rather than guess an install name.
        if item.is_none() {
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
fn locate_in_node_modules(start: &Path, base_pkg: &str) -> Option<PathBuf> {
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
        let pj = read_pkg_json(&dir)?;
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
    Some(candidate)
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
    c.is_ascii_alphanumeric() || c == '_'
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
            from_file: PathBuf::from("index.js"),
        };
        let src = JsProvider::new().resolve(&ctx).unwrap();
        assert!(!src.external);
        assert_eq!(src.file.file_name().unwrap(), "index.js");
        assert_eq!(src.source_root, local);
        assert_eq!(src.line, Some(1));
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
            from_file: PathBuf::from("index.js"),
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
            from_file: PathBuf::from("index.js"),
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
            from_file: PathBuf::from("index.js"),
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
        };
        // Offline keeps this test network-free: the error is the offline
        // refusal, which still proves the walk-up + no-local-dep path.
        let err = JsProvider::new().offline().resolve(&ctx).unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    // ── live E2E: real tiny npm package (network) ──────────────────────────

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
            from_file: PathBuf::from("index.js"),
        };
        let src = JsProvider::new().resolve(&ctx).unwrap();
        assert!(src.external);
        assert!(src.file.exists());
        let stem = src.file.to_string_lossy().to_string();
        assert!(stem.contains("left-pad"), "file: {stem}");
        assert!(src.line.is_some(), "expected a definition line");
    }
}
