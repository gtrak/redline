//! Python [`ToolingProvider`]: resolve a workspace-missed symbol to its
//! concrete source file via the Python interpreter.
//!
//! Flow: parse the module chain and item from the dotted symbol →
//! choose the interpreter (prefer workspace venv, else `python3`) →
//! `importlib.util.find_spec` via a timed subprocess to locate the module file
//! (walking up the dotted chain on failure; a src/-layout re-probe when the
//! CWD probe misses) → if not found and not stdlib,
//! `pip install` (behind the fetch-confirmation gate — the operator's
//! y/n; refused or unconfirmed = a clean refusal error, never a silent
//! install) and re-locate → a content scan finds the line
//! for the item's definition.
//!
//! Interpreter preference: `.venv/bin/python` → `venv/bin/python` → `python3`
//! on PATH. The matching `pip` is used for fetch-on-demand
//! (`.venv/bin/pip` → `venv/bin/pip` → `pip3`, honoring an ambient
//! `VIRTUAL_ENV`).
//!
//! Subprocess contracts:
//! - `find_spec`: `python3 -c "<find_origin script>"`, timeout 10 s.
//!   The script uses `importlib.util.find_spec` to print the module's
//!   `origin` (file path). For frozen/built-in modules it falls back to
//!   importing the module and reading its `__file__`. On failure it walks
//!   up the dotted chain (`a.b.c` → `a.b` → `a`).
//! - `is_stdlib`: `python3 -c "import sys; print('true' if X in
//!   sys.stdlib_module_names else 'false')"`, timeout 10 s.
//! - `pip install`: `pip3 install <pkg>` (or the venv's `pip`), timeout 120 s.
//!   P3-7, gate (doc): the CONFIRMED command is the canonical display form
//!   `pip install <pkg>` (what the banner shows) — the argv actually run
//!   may use the venv's `pip` or the ambient `pip3`; the package + flags
//!   are the invariant part.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::{
    confirm_or_refuse, run_with_timeout, scope_qualified, scope_qualified_alias, ResolvedSource,
    SymbolContext, ToolingProvider,
};

/// Timeout for the `find_spec` subprocess (should be fast).
const FIND_SPEC_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for `pip install` (network download).
const PIP_INSTALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Resolve Python symbols to real source via the interpreter's importlib.
pub struct PythonProvider {
    /// Default Python interpreter (used when no workspace venv is found).
    python_bin: String,
    /// When true, never run `pip install` (refuse on-demand fetch).
    offline: bool,
}

impl Default for PythonProvider {
    fn default() -> Self {
        Self {
            python_bin: "python3".to_string(),
            offline: false,
        }
    }
}

impl PythonProvider {
    /// Online provider using `python3` on PATH (or a workspace venv if present).
    pub fn new() -> Self {
        Self::default()
    }

    /// Use a specific Python interpreter (overridable for testing).
    pub fn with_python_bin(mut self, bin: impl Into<String>) -> Self {
        self.python_bin = bin.into();
        self
    }

    /// Never fetch: refuse `pip install` on a missing module (clean error).
    pub fn offline(mut self) -> Self {
        self.offline = true;
        self
    }

    /// Choose the interpreter: prefer a workspace venv
    /// (`.venv/bin/python` or `venv/bin/python`), else `default_bin` on PATH.
    ///
    /// This is a pure path check (no subprocess), so it can be tested without
    /// spawning a real interpreter.
    pub(crate) fn choose_interpreter(workspace_root: &Path, default_bin: &str) -> String {
        for venv_name in [".venv", "venv"] {
            let venv_python = workspace_root.join(venv_name).join("bin").join("python");
            if venv_python.exists() {
                return venv_python.to_string_lossy().to_string();
            }
        }
        default_bin.to_string()
    }

    /// The `pip` binary matching the chosen interpreter.
    ///
    /// Uses the venv's `pip` if a venv is present and `pip` exists in it,
    /// else `pip3` (which honors an ambient `VIRTUAL_ENV`).
    fn pip_bin(&self, workspace_root: &Path) -> String {
        for venv_name in [".venv", "venv"] {
            let venv_pip = workspace_root
                .join(venv_name)
                .join("bin")
                .join("pip");
            if venv_pip.exists() {
                return venv_pip.to_string_lossy().to_string();
            }
        }
        "pip3".to_string()
    }

    /// Run a Python `-c` command with a timeout. Returns trimmed stdout.
    fn run_python(
        &self,
        workspace_root: &Path,
        code: &str,
        timeout: Duration,
    ) -> anyhow::Result<String> {
        let mut cmd = Command::new(&self.python_bin);
        cmd.current_dir(workspace_root).arg("-c").arg(code);
        let out = run_with_timeout(cmd, timeout)?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(anyhow::anyhow!(
                "python command failed (exit {}): {stderr}",
                out.status.code().unwrap_or(-1)
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn resolve_python(&self, ctx: &SymbolContext) -> anyhow::Result<ResolvedSource> {
        let workspace_root = &ctx.workspace_root;

        // Choose interpreter (prefer workspace venv).
        let python_bin = Self::choose_interpreter(workspace_root, &self.python_bin);
        let provider = Self {
            python_bin,
            offline: self.offline,
        };

        // Parse the symbol: split by `.` into (module_chain, item). A BARE
        // symbol with a scope hint (007-03: the app's import path)
        // normalizes to `module.item` up front and flows through the SAME
        // find_spec machinery as a dotted symbol. No hint keeps the bail,
        // byte-for-byte. 011-08: a PATH-SHAPED symbol whose first segment
        // is a local alias (`engine.torque` from `from gears import
        // engine`, hinted as `["gears", "engine", "torque"]` — the
        // original import path, item included) is rewritten to the
        // import's real path and flows through the SAME find_spec
        // machinery; identity hints and non-rewrites leave the symbol's
        // own path untouched (mirrors js_provider's composition).
        let qualified = scope_qualified(".", &ctx.symbol, &ctx.scope)
            .or_else(|| scope_qualified_alias(".", &ctx.symbol, &ctx.scope));
        let symbol = qualified.as_deref().unwrap_or(&ctx.symbol);
        let segments: Vec<&str> = symbol
            .split('.')
            .filter(|s| !s.is_empty())
            .collect();
        if segments.len() < 2 {
            anyhow::bail!(
                "bare symbol `{}` has no module path; resolving it to a module \
                 needs scope info (tree-sitter) not yet provided by the app",
                ctx.symbol
            );
        }
        let item = segments.last().unwrap();
        let module_chain = segments[..segments.len() - 1].join(".");

        // Locate the module file (find_spec → pip install if needed,
        // behind the confirmation gate).
        let file =
            provider.locate_module_file(ctx, workspace_root, &ctx.from_file, &module_chain)?;

        // Find the item's definition line in the resolved file.
        let line = find_item_line(&file, item).ok_or_else(|| {
            anyhow::anyhow!(
                "item `{item}` not found in module `{module_chain}` (file: {})",
                file.display()
            )
        })?;

        // Determine source_root and external.
        let canonical_root = std::fs::canonicalize(workspace_root).map_err(|e| {
            anyhow::anyhow!("cannot canonicalize {}: {e}", workspace_root.display())
        })?;
        let canonical_file = std::fs::canonicalize(&file).unwrap_or(file.clone());
        let external = !canonical_file.starts_with(&canonical_root);
        let source_root = if external {
            let top_pkg = segments.first().unwrap();
            source_root_for_external(&canonical_file, top_pkg)
        } else {
            canonical_root
        };

        Ok(ResolvedSource {
            file,
            source_root,
            external,
            line: Some(line),
        })
    }

    /// Locate the module file via `find_spec`, with fallback to `pip install`
    /// (gated on the operator's confirmation — issue-non-rust-receiver).
    fn locate_module_file(
        &self,
        ctx: &SymbolContext,
        workspace_root: &Path,
        from_file: &Path,
        module_chain: &str,
    ) -> anyhow::Result<PathBuf> {
        if let Some(file) = self.find_spec(workspace_root, None, module_chain)? {
            return Ok(file);
        }

        // CWD miss: honest src/-layout discovery (011-08 finding 2). A
        // src/-layout package (PEP 621, sources under `<root>/src/`) is not
        // importable from the CWD. If a `pyproject.toml` with a `src/` dir is
        // found by walking up from `from_file`'s directory, re-probe
        // find_spec with that dir on sys.path — the sys.path entry is derived
        // from a FOUND file on disk, not a guess; no pyproject (or no `src/`
        // dir) leaves the miss untouched (byte-for-byte degradation).
        if let Some(src_dir) = src_layout_root(workspace_root, from_file)
            && let Some(file) = self.find_spec(workspace_root, Some(&src_dir), module_chain)?
        {
            return Ok(file);
        }

        // Not found: check if it's stdlib (never install stdlib).
        if self.is_stdlib(workspace_root, module_chain)? {
            anyhow::bail!(
                "stdlib module `{module_chain}` could not be located by find_spec"
            );
        }

        // Not stdlib: try `pip install` — behind the fetch-confirmation
        // gate (issue-non-rust-receiver-resolution): the hook (the app's
        // visible y/n prompt) decides; no hook or a decline is a refusal
        // (the provider never installs silently). `offline` stays the
        // stronger, permanent refusal (never asks, never installs).
        if self.offline {
            anyhow::bail!(
                "module `{module_chain}` not found and offline mode refuses to pip install"
            );
        }

        let top_pkg = module_chain.split('.').next().unwrap();
        // P3-7, gate (doc): canonical display form for the banner — the
        // argv `run_pip_install` actually runs may be the venv's `pip` or
        // the ambient `pip3`.
        let command = format!("pip install {top_pkg}");
        confirm_or_refuse(ctx, &command, from_file)?;
        self.run_pip_install(workspace_root, top_pkg)?;

        // Re-locate after install.
        self.find_spec(workspace_root, None, module_chain)
            .ok()
            .flatten()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "module `{module_chain}` still not found after `pip install {top_pkg}`"
                )
            })
    }

    /// Run `find_spec` via a Python subprocess. Returns the origin file path
    /// or `None` if the module cannot be located.
    ///
    /// `extra_sys_path` (the discovered `src/` dir of a src/-layout project)
    /// is inserted at the front of `sys.path` before probing; `None` keeps
    /// the CWD-based probe untouched.
    fn find_spec(
        &self,
        workspace_root: &Path,
        extra_sys_path: Option<&Path>,
        module_chain: &str,
    ) -> anyhow::Result<Option<PathBuf>> {
        // The find_origin script:
        // 0. Optionally insert the discovered src/-layout root at the front
        //    of sys.path (011-08 finding 2).
        // 1. Try `importlib.util.find_spec(module_chain)`.
        // 2. If origin is a file path, print it.
        // 3. If origin is 'frozen' or 'built-in' (e.g. `os`, `os.path` in
        //    CPython 3.11+), import the module and print its `__file__`.
        // 4. On failure, walk up the dotted chain (`a.b.c` → `a.b` → `a`).
        // `{p:?}` is a Rust-quoted string literal, which is also a valid
        // Python string literal for the (path) values involved.
        let path_prefix = match extra_sys_path {
            // A control-char path component would make Rust's `{:?}` emit
            // `\u{…}` (invalid in a Python string literal) and hard-error
            // the subprocess — pre-validate and degrade to a miss (the
            // CWD-miss flow continues untouched).
            Some(p) if python_literal_path(p) => {
                format!("import sys\nsys.path.insert(0, {p:?})\n")
            }
            Some(_) => return Ok(None),
            None => String::new(),
        };
        let code = format!(
            r#"
{path_prefix}import importlib.util, importlib

def find_origin(module_chain):
    try:
        spec = importlib.util.find_spec(module_chain)
    except (ImportError, AttributeError, ValueError):
        spec = None
    if spec is not None:
        origin = spec.origin
        if origin and origin not in ('frozen', 'built-in', 'namespace'):
            print(origin)
            return
        if origin in ('frozen', 'built-in'):
            try:
                mod = importlib.import_module(module_chain)
                f = getattr(mod, '__file__', None)
                if f:
                    print(f)
                    return
            except ImportError:
                pass
    if '.' in module_chain:
        find_origin(module_chain.rsplit('.', 1)[0])

find_origin({module_chain:?})
"#
        );
        let output = self.run_python(workspace_root, &code, FIND_SPEC_TIMEOUT)?;
        if output.is_empty() {
            Ok(None)
        } else {
            Ok(Some(PathBuf::from(output)))
        }
    }

    /// Check if the top-level module of `module_chain` is in the stdlib.
    fn is_stdlib(&self, workspace_root: &Path, module_chain: &str) -> anyhow::Result<bool> {
        let top = module_chain.split('.').next().unwrap();
        let code = format!(
            r#"import sys; print('true' if {top:?} in sys.stdlib_module_names else 'false')"#
        );
        let output = self.run_python(workspace_root, &code, FIND_SPEC_TIMEOUT)?;
        Ok(output == "true")
    }

    /// Run `pip install <pkg>` in the chosen environment.
    fn run_pip_install(&self, workspace_root: &Path, pkg: &str) -> anyhow::Result<()> {
        let pip_bin = self.pip_bin(workspace_root);
        let mut cmd = Command::new(&pip_bin);
        cmd.current_dir(workspace_root).arg("install").arg(pkg);
        let out = run_with_timeout(cmd, PIP_INSTALL_TIMEOUT)?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(anyhow::anyhow!("`pip install {pkg}` failed: {stderr}"));
        }
        Ok(())
    }
}

impl ToolingProvider for PythonProvider {
    fn name(&self) -> &'static str {
        "python"
    }

    fn languages(&self) -> &'static [&'static str] {
        &["python"]
    }

    fn resolve(&self, ctx: &SymbolContext) -> anyhow::Result<ResolvedSource> {
        self.resolve_python(ctx)
    }
}



/// Honest src/-layout root discovery: walk up from `from_file`'s
/// directory (bounded by `workspace_root`) for a `pyproject.toml` (PEP 621
/// project marker); if the project carries a `src/` dir (the src layout),
/// that dir is the sys.path entry that makes the package importable.
/// Returns `None` when no enclosing project with a `src/` layout is found —
/// the caller then leaves the CWD miss untouched (no guessing). The
/// returned `src/` dir is canonicalized and required to stay under the
/// canonical workspace root (a symlinked `src/` pointing outside never
/// satisfies the marker).
fn src_layout_root(workspace_root: &Path, from_file: &Path) -> Option<PathBuf> {
    let canonical_root = std::fs::canonicalize(workspace_root).ok()?;
    let file_path = workspace_root.join(from_file);
    // `from_file` is workspace-relative; a root-level file has no own dir,
    // so the walk starts at the workspace root itself.
    let start = file_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(workspace_root);
    let mut dir = start.to_path_buf();
    loop {
        if dir.join("pyproject.toml").is_file() && dir.join("src").is_dir() {
            // Canonicalize before returning: a symlinked `src/` resolving
            // outside the workspace is not a workspace src layout.
            let src = std::fs::canonicalize(dir.join("src")).ok()?;
            if !src.starts_with(&canonical_root) {
                return None;
            }
            return Some(src);
        }
        if dir == *workspace_root {
            return None;
        }
        dir = dir.parent()?.to_path_buf();
    }
}

/// Does `p` round-trip through Rust's `{:?}` quoting as a valid Python
/// string literal? Rust's Debug quoting renders control chars (beyond the
/// standard `\n`/`\t`/`\r`/quote/backslash escapes, which Python accepts)
/// as `\u{…}` — a SyntaxError in a Python string literal, which would
/// hard-error the re-probe subprocess. Non-UTF-8 paths cannot be checked
/// at all → treated as unsafe.
fn python_literal_path(p: &Path) -> bool {
    p.to_str()
        .map(|s| s.chars().all(|c| !c.escape_debug().to_string().contains("\\u{")))
        .unwrap_or(false)
}

/// Determine the source root for an external (non-workspace) Python file.
///
/// Walks up the file's directory chain looking for a directory named
/// `top_pkg`; the parent of that directory is the source root.
/// Falls back to the file's parent directory if no match is found
/// (e.g. a top-level module like `os.py` rather than a package dir).
fn source_root_for_external(file: &Path, top_pkg: &str) -> PathBuf {
    let mut current = file.parent();
    while let Some(dir) = current {
        if dir.file_name().and_then(|n| n.to_str()) == Some(top_pkg) {
            return dir
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| dir.to_path_buf());
        }
        current = dir.parent();
    }
    // Fallback: the parent of the file's directory.
    file.parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default()
}

/// Find the 1-based line number of the first definition-shaped match for
/// `item` in a Python file, or `None` if no match is found.
fn find_item_line(file: &Path, item: &str) -> Option<u32> {
    let contents = std::fs::read_to_string(file).ok()?;
    for (i, line) in contents.lines().enumerate() {
        if line_defines_python_item(line, item) {
            return Some((i + 1) as u32);
        }
    }
    None
}

/// Does `line` define an item named `item` (a definition-shaped match)?
///
/// Matches (with optional leading whitespace for indented definitions):
/// - `def item(...)` or `def item:` — function definition
/// - `async def item(...)` — async function definition
/// - `class item(...)` or `class item:` — class definition
/// - `item = value` — module-level constant (not `item == value`)
///
/// Rejects:
/// - `import item`, `from item import …`
/// - comments (`# def item`)
/// - strings containing `"def item"`
/// - `item == value` (comparison, not assignment)
/// - `item` as a substring of a longer identifier (`join_all` ≠ `join`)
fn line_defines_python_item(line: &str, item: &str) -> bool {
    let trimmed = line.trim_start();

    // `def item(...)` or `def item:`
    if let Some(rest) = trimmed.strip_prefix("def ")
        && let Some(after) = rest.strip_prefix(item) {
        return match after.chars().next() {
            None => true,
            Some(c) => !(c.is_alphanumeric() || c == '_'),
        };
    }

    // `async def item(...)` or `async def item:`
    if let Some(rest) = trimmed.strip_prefix("async def ")
        && let Some(after) = rest.strip_prefix(item) {
        return match after.chars().next() {
            None => true,
            Some(c) => !(c.is_alphanumeric() || c == '_'),
        };
    }

    // `class item(...)` or `class item:`
    if let Some(rest) = trimmed.strip_prefix("class ")
        && let Some(after) = rest.strip_prefix(item) {
        return match after.chars().next() {
            None => true,
            Some(c) => !(c.is_alphanumeric() || c == '_'),
        };
    }

    // `item = value` (module constant; not `item == value`)
    if let Some(rest) = trimmed.strip_prefix(item) {
        let rest = rest.trim_start();
        if let Some(after_eq) = rest.strip_prefix('=') {
            // Reject `==` (comparison).
            return !after_eq.starts_with('=');
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ── definition-shape tests ──────────────────────────────────────────────

    #[test]
    fn line_defines_function() {
        assert!(line_defines_python_item("def join(a, b):", "join"));
        assert!(line_defines_python_item("def join(a, b) -> str:", "join"));
        assert!(line_defines_python_item("    def get(self, url):", "get"));
        assert!(line_defines_python_item("async def fetch(url):", "fetch"));
    }

    #[test]
    fn line_defines_class() {
        assert!(line_defines_python_item("class Session:", "Session"));
        assert!(line_defines_python_item("class Session(object):", "Session"));
        assert!(line_defines_python_item("    class Inner:", "Inner"));
    }

    #[test]
    fn line_defines_constant() {
        assert!(line_defines_python_item("VERSION = '1.0'", "VERSION"));
        assert!(line_defines_python_item("MAX_RETRIES = 3", "MAX_RETRIES"));
        assert!(line_defines_python_item("    DEBUG = False", "DEBUG"));
    }

    #[test]
    fn line_defines_rejects_non_definitions() {
        // imports
        assert!(!line_defines_python_item("import json", "json"));
        assert!(!line_defines_python_item("from os.path import join", "join"));
        // comments
        assert!(!line_defines_python_item("# def join(a, b):", "join"));
        assert!(!line_defines_python_item("# class Session:", "Session"));
        // string literal containing "def "
        assert!(!line_defines_python_item("x = \"def join(a, b):\"", "join"));
        // comparison, not assignment
        assert!(!line_defines_python_item("if x == 5:", "x"));
        // substring of a longer identifier
        assert!(!line_defines_python_item("def join_all(a, b):", "join"));
        assert!(!line_defines_python_item("class SessionWrapper:", "Session"));
        // usage, not definition
        assert!(!line_defines_python_item("result = json.dumps(data)", "json"));
    }

    #[test]
    fn find_item_line_in_tempfile() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("mymod.py");
        std::fs::write(
            &file,
            "import os\n\n\ndef join(a, b):\n    return a + '/' + b\n\nclass Foo:\n    pass\n\nCONST = 42\n",
        )
        .unwrap();
        assert_eq!(find_item_line(&file, "join"), Some(4));
        assert_eq!(find_item_line(&file, "Foo"), Some(7));
        assert_eq!(find_item_line(&file, "CONST"), Some(10));
        assert_eq!(find_item_line(&file, "NotThere"), None);
    }

    // ── venv preference (PATH-ORDER construction, no subprocess) ───────────

    #[test]
    fn choose_interpreter_prefers_venv() {
        let tmp = tempfile::tempdir().unwrap();
        // No venv: should return the default.
        assert_eq!(
            PythonProvider::choose_interpreter(tmp.path(), "python3"),
            "python3"
        );

        // Create a fake .venv/bin/python (just a file, not a real interpreter).
        let venv_bin = tmp.path().join(".venv").join("bin");
        std::fs::create_dir_all(&venv_bin).unwrap();
        std::fs::write(venv_bin.join("python"), "#!/bin/sh\necho fake\n").unwrap();

        assert_eq!(
            PythonProvider::choose_interpreter(tmp.path(), "python3"),
            venv_bin
                .join("python")
                .to_string_lossy()
                .to_string()
        );
    }

    #[test]
    fn choose_interpreter_prefers_dotvenv_over_venv() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["venv", ".venv"] {
            let bin = tmp.path().join(name).join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(bin.join("python"), "#!/bin/sh\necho fake\n").unwrap();
        }
        // `.venv` takes priority (checked first).
        assert_eq!(
            PythonProvider::choose_interpreter(tmp.path(), "python3"),
            tmp.path()
                .join(".venv")
                .join("bin")
                .join("python")
                .to_string_lossy()
                .to_string()
        );
    }

    // ── workspace package (external=false) ──────────────────────────────────

    #[test]
    fn resolve_workspace_package() {
        let tmp = tempfile::tempdir().unwrap();
        // Create a local package: mypkg/__init__.py
        let pkg_dir = tmp.path().join("mypkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(
            pkg_dir.join("__init__.py"),
            "def hello():\n    return 'hi'\n\nVERSION = '1.0'\n",
        )
        .unwrap();

        let provider = PythonProvider::new();
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "mypkg.hello".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("main.py"),
            language: None,
            confirm_fetch: None,
        };
        let result = provider.resolve(&ctx).unwrap();
        assert_eq!(result.file, pkg_dir.join("__init__.py"));
        assert!(!result.external);
        assert_eq!(result.line, Some(1));
    }

    // ── stdlib (external=true, no network) ──────────────────────────────────

    #[test]
    fn resolve_stdlib_module() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = PythonProvider::new();
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "json.dumps".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("main.py"),
            language: None,
            confirm_fetch: None,
        };
        let result = provider.resolve(&ctx).unwrap();
        // The file should exist and be in the stdlib (external).
        assert!(result.file.exists());
        assert!(result.external);
        // source_root should be the stdlib root (parent of the `json` dir).
        assert!(result.source_root.exists());
    }

    /// `os.path` is frozen in CPython 3.11+; `find_spec` returns
    /// `origin=frozen`, so the provider falls back to importing `os.path`
    /// and reading its `__file__`, which is `posixpath.py` (Linux) or
    /// `ntpath.py` (Windows) — not `os.py`.
    #[test]
    fn resolve_dotted_chain_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = PythonProvider::new();
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "os.path.join".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("main.py"),
            language: None,
            confirm_fetch: None,
        };
        let result = provider.resolve(&ctx).unwrap();
        assert!(result.file.exists());
        let filename = result.file.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            filename == "posixpath.py" || filename == "ntpath.py",
            "unexpected file: {filename}"
        );
        assert!(result.external);
        // The item `join` should be found (def join in posixpath.py/ntpath.py).
        assert!(result.line.is_some());
    }

    // ── failure modes ───────────────────────────────────────────────────────

    #[test]
    fn bare_symbol_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = PythonProvider::new();
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "Session".to_string(),
            from_file: PathBuf::from("main.py"),
            scope: Vec::new(),
            language: None,
            confirm_fetch: None,
        };
        let err = provider.resolve(&ctx).unwrap_err();
        assert!(
            err.to_string().contains("needs scope info"),
            "err: {err}"
        );
    }

    // ── 007-03: bare symbol + scope hint (import path) ──────────────────

    /// Discriminating: a BARE symbol + the import-path scope resolves
    /// through the SAME find_spec machinery as the dotted twin
    /// (`resolve_workspace_package` pins that twin's landing).
    #[test]
    fn bare_symbol_with_scope_resolves_workspace_package() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("mypkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(
            pkg_dir.join("__init__.py"),
            "def hello():\n    return 'hi'\n\nVERSION = '1.0'\n",
        )
        .unwrap();

        let provider = PythonProvider::new();
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "hello".to_string(),
            from_file: PathBuf::from("main.py"),
            scope: vec!["mypkg".to_string(), "hello".to_string()],
            language: None,
            confirm_fetch: None,
        };
        let result = provider.resolve(&ctx).unwrap();
        assert_eq!(result.file, pkg_dir.join("__init__.py"));
        assert!(!result.external);
        assert_eq!(result.line, Some(1));
    }

    /// Stdlib names are NOT guessed: a bare stdlib function name with no
    /// hint still bails (the app never fabricates a module path for
    /// prelude/built-in names).
    #[test]
    fn stdlib_name_without_hint_is_not_guessed() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = PythonProvider::new();
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "join".to_string(),
            from_file: PathBuf::from("main.py"),
            scope: Vec::new(),
            language: None,
            confirm_fetch: None,
        };
        let err = provider.resolve(&ctx).unwrap_err();
        assert!(
            err.to_string().contains("needs scope info"),
            "err: {err}"
        );
    }

    #[test]
    fn no_interpreter_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = PythonProvider::new().with_python_bin("/nonexistent/python");
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "json.dumps".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("main.py"),
            language: None,
            confirm_fetch: None,
        };
        let err = provider.resolve(&ctx).unwrap_err();
        assert!(
            err.to_string().contains("failed to run"),
            "err: {err}"
        );
    }

    #[test]
    fn offline_mode_refuses_missing_module() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = PythonProvider::new().offline();
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "nonexistent_pkg_xyz.module_func".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("main.py"),
            language: None,
            confirm_fetch: None,
        };
        let err = provider.resolve(&ctx).unwrap_err();
        assert!(
            err.to_string().contains("offline mode refuses"),
            "err: {err}"
        );
    }

    // ── 011-08: path-shaped alias rewrite (`scope_qualified_alias`) ─────

    /// Build a `gears` workspace package: `gears/__init__.py` +
    /// `gears/engine.py` with a `torque` definition.
    fn alias_rewrite_workspace() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("gears");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("__init__.py"), "").unwrap();
        std::fs::write(
            pkg_dir.join("engine.py"),
            "MAX_TORQUE = 320.0\n\n\ndef spin(rpm):\n    return rpm > 0\n\n\ndef torque(rpm):\n    return min(MAX_TORQUE, rpm / 10.0)\n",
        )
        .unwrap();
        tmp
    }

    /// Discriminating (corpus probe 013 flip): `engine.torque` — the
    /// PATH-SHAPED use site of `from gears import engine` — with the
    /// app's hint naming the original import path rewrites the alias
    /// (`engine` → `gears.engine`) and lands on the `torque` definition
    /// in `gears/engine.py`, through the SAME find_spec machinery.
    #[test]
    fn aliased_dotted_use_rewrites_to_original_path() {
        let tmp = alias_rewrite_workspace();
        let pkg_dir = tmp.path().join("gears");

        let provider = PythonProvider::new().offline();
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "engine.torque".to_string(),
            from_file: PathBuf::from("main.py"),
            scope: vec!["gears".to_string(), "engine".to_string(), "torque".to_string()],
            language: None,
            confirm_fetch: None,
        };
        let result = provider.resolve(&ctx).unwrap();
        assert_eq!(result.file, pkg_dir.join("engine.py"));
        assert!(!result.external);
        // Line 8 (1-based): "def torque(rpm):"
        assert_eq!(result.line, Some(8));
    }

    /// No-op pins for the rewrite: an EMPTY scope (byte-for-byte
    /// degradation), an identity hint (the hint IS the symbol's own path),
    /// and a mismatched-item hint (`spin` ≠ `torque`) must all leave the
    /// symbol's own path untouched — `engine.torque` still walks its own
    /// (top-level `engine`) path and hits the offline refusal, never a
    /// silent landing on the hinted path.
    #[test]
    fn aliased_dotted_identity_mismatched_and_unhinted_are_no_ops() {
        let tmp = alias_rewrite_workspace();
        let provider = PythonProvider::new().offline();
        for scope in [
            Vec::new(),
            vec!["engine".to_string(), "torque".to_string()],
            vec!["gears".to_string(), "engine".to_string(), "spin".to_string()],
        ] {
            let ctx = SymbolContext {
                workspace_root: tmp.path().to_path_buf(),
                symbol: "engine.torque".to_string(),
                from_file: PathBuf::from("main.py"),
                scope,
                language: None,
                confirm_fetch: None,
            };
            let err = provider.resolve(&ctx).unwrap_err();
            assert!(
                err.to_string().contains("offline mode refuses"),
                "no-op case bailed with: {err}"
            );
        }
    }

    // ── 011-08 finding 2: src/-layout workspace-root discovery ──────────

    /// Discriminating (corpus probe 016): a src/-layout package (PEP 621,
    /// sources under `src/`) is not importable from the CWD; the provider
    /// walks up from `from_file`'s dir to the FOUND `pyproject.toml`, sees
    /// the sibling `src/` dir, and re-probes find_spec with that dir on
    /// sys.path — landing on the package through the SAME find_spec
    /// machinery (no guessing: no pyproject → the offline refusal below).
    #[test]
    fn resolve_src_layout_workspace_package() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("pyproject.toml"), "[project]\nname = \"demo\"\n")
            .unwrap();
        let pkg_dir = tmp.path().join("src").join("mypkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(
            pkg_dir.join("__init__.py"),
            "def hello():\n    return 'hi'\n",
        )
        .unwrap();

        let provider = PythonProvider::new().offline();
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "mypkg.hello".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("src/main.py"),
            language: None,
            confirm_fetch: None,
        };
        let result = provider.resolve(&ctx).unwrap();
        assert_eq!(result.file, pkg_dir.join("__init__.py"));
        assert!(!result.external);
        assert_eq!(result.line, Some(1));
    }

    /// No-op pin: a `src/` package WITHOUT a `pyproject.toml` on the
    /// `from_file` chain is NOT discovered (a guess) — the CWD miss flows
    /// to the offline refusal, byte-for-byte.
    #[test]
    fn src_layout_without_pyproject_is_not_guessed() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("src").join("mypkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("__init__.py"), "def hello():\n    pass\n").unwrap();

        let provider = PythonProvider::new().offline();
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "mypkg.hello".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("src/main.py"),
            language: None,
            confirm_fetch: None,
        };
        let err = provider.resolve(&ctx).unwrap_err();
        assert!(
            err.to_string().contains("offline mode refuses"),
            "err: {err}"
        );
    }

    #[test]
    fn src_layout_root_walks_up_for_pyproject() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // Flat layout: pyproject but no `src/` dir → None.
        std::fs::write(root.join("pyproject.toml"), "").unwrap();
        assert_eq!(src_layout_root(root, Path::new("main.py")), None);
        assert_eq!(src_layout_root(root, Path::new("x/main.py")), None);
        // src layout: the nearest enclosing pyproject + `src/` dir wins
        // (returned canonicalized).
        std::fs::create_dir_all(root.join("src")).unwrap();
        let src = std::fs::canonicalize(root.join("src")).unwrap();
        assert_eq!(
            src_layout_root(root, Path::new("src/main.py")),
            Some(src.clone())
        );
        // Walk-up: a nested from_file finds the root's project.
        std::fs::create_dir_all(root.join("src").join("deep")).unwrap();
        assert_eq!(src_layout_root(root, Path::new("src/deep/util.py")), Some(src));
        // No pyproject at all → None (no guessing).
        let bare = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(bare.path().join("src")).unwrap();
        assert_eq!(src_layout_root(bare.path(), Path::new("src/main.py")), None);
    }

    // ── hardening pins (review P2) ─────────────────────────────────────────

    /// A symlinked `src/` pointing OUTSIDE the workspace satisfies the
    /// naive marker (a `pyproject.toml` + an existing `src/` dir) but must
    /// not count as a workspace src layout: the canonicalized dir fails
    /// the `starts_with(canonical workspace root)` check → miss.
    #[cfg(unix)]
    #[test]
    fn src_layout_root_rejects_symlinked_outside_src() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(root.join("pyproject.toml"), "").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("src")).unwrap();
        assert_eq!(src_layout_root(root, Path::new("src/main.py")), None);
    }

    /// A control char in an ancestor component of the discovered `src/`
    /// path must not hard-error the re-probe subprocess (Rust `{:?}`
    /// emits `\u{…}`, invalid in a Python string literal) — it degrades
    /// to the plain CWD-miss flow (the offline refusal, byte-for-byte).
    #[test]
    fn control_char_in_discovered_src_path_degrades_to_miss() {
        let tmp = tempfile::tempdir().unwrap();
        // A control char in the project-dir component: the discovered
        // `src/` path carries it into the re-probe's sys.path line.
        let proj = tmp.path().join("pr\x01oj");
        std::fs::create_dir_all(proj.join("src")).unwrap();
        std::fs::write(proj.join("pyproject.toml"), "").unwrap();

        let provider = PythonProvider::new().offline();
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "gears.torque".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("pr\x01oj/src/main.py"),
            language: None,
            confirm_fetch: None,
        };
        // Graceful miss: the re-probe is skipped and the miss flows to
        // the stdlib check + offline refusal (no SyntaxError hard-error
        // from the python subprocess).
        let err = provider.resolve(&ctx).unwrap_err();
        assert!(
            err.to_string().contains("offline mode refuses"),
            "err: {err}"
        );
    }

    /// `python_literal_path` accepts paths whose Rust Debug quoting is a
    /// valid Python string literal and rejects control-char components
    /// (the `\u{…}` shapes Python's parser refuses).
    #[test]
    fn python_literal_path_rejects_unquotable_components() {
        assert!(python_literal_path(Path::new("/tmp/proj/src")));
        assert!(python_literal_path(Path::new("a\\b"))); // backslash round-trips
        assert!(!python_literal_path(Path::new("a\x01b")));
        assert!(!python_literal_path(Path::new("a\x7fb")));
    }

    // ── live E2E: resolve a real installed third-party module ───────────────

    #[test]
    fn resolve_third_party_module_e2e() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = PythonProvider::new();
        // `requests.get` — a real third-party module, very likely installed.
        // If not installed, the test skips gracefully (found-or-skipped):
        // with no confirm hook the provider REFUSES the install
        // (issue-non-rust-receiver-resolution) — it never silently
        // touches the network here.
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "requests.get".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("main.py"),
            language: None,
            confirm_fetch: None,
        };
        match provider.resolve(&ctx) {
            Ok(src) => {
                assert!(src.external);
                assert!(src.file.exists());
                eprintln!("E2E: requests.get resolved to {}", src.file.display());
            }
            Err(e) => {
                eprintln!("E2E: requests.get not resolved (skipped): {e}");
            }
        }
    }

    // ── issue-non-rust-receiver-resolution: the fetch-confirmation gate ─

    /// Build a workspace with a STUB `.venv/bin/pip` that logs its argv
    /// (one line per invocation) and exits 0 (the exit-0 choice: the
    /// provider then re-locates and reports the honest "still not
    /// found" — the log line is the proof the install step ran at all).
    /// No `.venv/bin/python` is created: the interpreter stays the real
    /// `python3` on PATH (find_spec / is_stdlib are local, no network).
    fn ws_with_stub_pip() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join(".venv").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = tmp.path().join("pip-invocations.log");
        let stub = bin.join("pip");
        std::fs::write(
            &stub,
            format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> {log:?}\nexit 0\n"),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (tmp, log)
    }

    fn pip_invocations(log: &std::path::Path) -> Vec<String> {
        std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Discriminating (the P1's safety half): a genuinely unresolvable
    /// module head does NOT install when the operator DECLINES the
    /// fetch — the refusal bails the provider and the stub pip is never
    /// invoked.
    #[test]
    fn confirm_declined_refuses_install() {
        let (tmp, log) = ws_with_stub_pip();
        let confirm: std::sync::Arc<dyn Fn(&crate::FetchRequest) -> bool + Send + Sync> =
            std::sync::Arc::new(|_req| false);
        let provider = PythonProvider::new();
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "nonexistent_pkg_xyz.module_func".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("main.py"),
            language: Some("python".to_string()),
            confirm_fetch: Some(confirm),
        };
        let err = provider.resolve(&ctx).unwrap_err();
        assert!(
            err.to_string().contains("declined at the fetch confirmation"),
            "err: {err}"
        );
        assert!(
            err.to_string().contains("pip install nonexistent_pkg_xyz"),
            "the prompt's exact command is in the refusal: {err}"
        );
        assert_eq!(pip_invocations(&log), Vec::<String>::new(), "decline: zero pip invocations");
    }

    /// The safe default: NO hook at all — the provider refuses the
    /// install rather than running it silently (a provider never
    /// installs without a confirmation; the offline flag's permanent
    /// refusal is unchanged on top).
    #[test]
    fn confirm_absent_refuses_install() {
        let (tmp, log) = ws_with_stub_pip();
        let provider = PythonProvider::new();
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "nonexistent_pkg_xyz.module_func".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("main.py"),
            language: Some("python".to_string()),
            confirm_fetch: None,
        };
        let err = provider.resolve(&ctx).unwrap_err();
        assert!(
            err.to_string().contains("requires a fetch confirmation hook"),
            "err: {err}"
        );
        assert_eq!(pip_invocations(&log), Vec::<String>::new(), "no hook: zero pip invocations");
    }

    /// The sanctioned leg: the operator ACCEPTS (the hook is the
    /// app's visible y/n prompt) — the stub pip is invoked EXACTLY ONCE
    /// with the exact argv (`install nonexistent_pkg_xyz`), and the
    /// provider still bails honestly when the re-locate misses (the
    /// stub installs nothing).
    #[test]
    fn confirm_accepted_runs_stub_pip_exactly_once() {
        let (tmp, log) = ws_with_stub_pip();
        let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_probe = seen.clone();
        let confirm: std::sync::Arc<dyn Fn(&crate::FetchRequest) -> bool + Send + Sync> =
            std::sync::Arc::new(move |req: &crate::FetchRequest| {
                seen_probe.lock().unwrap().push(req.command.clone());
                true
            });
        let provider = PythonProvider::new();
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "nonexistent_pkg_xyz.module_func".to_string(),
            scope: Vec::new(),
            from_file: PathBuf::from("main.py"),
            language: Some("python".to_string()),
            confirm_fetch: Some(confirm),
        };
        let err = provider.resolve(&ctx).unwrap_err();
        assert!(
            err.to_string().contains("still not found after `pip install nonexistent_pkg_xyz`"),
            "the install ran (then the honest re-locate miss): {err}"
        );
        // The hook saw exactly ONE request, with the exact command and
        // the file that implied it.
        assert_eq!(seen.lock().unwrap().as_slice(), &["pip install nonexistent_pkg_xyz"]);
        // The stub pip ran EXACTLY ONCE with the exact argv.
        assert_eq!(pip_invocations(&log), vec!["install nonexistent_pkg_xyz".to_string()]);
    }
}
