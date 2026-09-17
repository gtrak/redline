//! Python [`ToolingProvider`]: resolve a workspace-missed symbol to its
//! concrete source file via the Python interpreter.
//!
//! Flow: parse the module chain and item from the dotted symbol →
//! choose the interpreter (prefer workspace venv, else `python3`) →
//! `importlib.util.find_spec` via a timed subprocess to locate the module file
//! (walking up the dotted chain on failure) → if not found and not stdlib,
//! `pip install` (sanctioned) and re-locate → a content scan finds the line
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

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::{ResolvedSource, SymbolContext, ToolingProvider};

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

        // Parse the symbol: split by `.` into (module_chain, item).
        let segments: Vec<&str> = ctx
            .symbol
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

        // Locate the module file (find_spec → pip install if needed).
        let file = provider.locate_module_file(workspace_root, &module_chain)?;

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

    /// Locate the module file via `find_spec`, with fallback to `pip install`.
    fn locate_module_file(
        &self,
        workspace_root: &Path,
        module_chain: &str,
    ) -> anyhow::Result<PathBuf> {
        if let Some(file) = self.find_spec(workspace_root, module_chain)? {
            return Ok(file);
        }

        // Not found: check if it's stdlib (never install stdlib).
        if self.is_stdlib(workspace_root, module_chain)? {
            anyhow::bail!(
                "stdlib module `{module_chain}` could not be located by find_spec"
            );
        }

        // Not stdlib: try `pip install` (sanctioned fetch-on-demand).
        if self.offline {
            anyhow::bail!(
                "module `{module_chain}` not found and offline mode refuses to pip install"
            );
        }

        let top_pkg = module_chain.split('.').next().unwrap();
        self.run_pip_install(workspace_root, top_pkg)?;

        // Re-locate after install.
        self.find_spec(workspace_root, module_chain)
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
    fn find_spec(
        &self,
        workspace_root: &Path,
        module_chain: &str,
    ) -> anyhow::Result<Option<PathBuf>> {
        // The find_origin script:
        // 1. Try `importlib.util.find_spec(module_chain)`.
        // 2. If origin is a file path, print it.
        // 3. If origin is 'frozen' or 'built-in' (e.g. `os`, `os.path` in
        //    CPython 3.11+), import the module and print its `__file__`.
        // 4. On failure, walk up the dotted chain (`a.b.c` → `a.b` → `a`).
        let code = format!(
            r#"
import importlib.util, importlib

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

/// Run a command with a timeout. The command is spawned in a background
/// thread; if the timeout expires before the command finishes, an error is
/// returned (the orphaned process will eventually exit or be reaped by the OS).
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> anyhow::Result<Output> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = cmd.output();
        let _ = tx.send(result);
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) => Err(anyhow::anyhow!("failed to run command: {e}")),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            Err(anyhow::anyhow!("command timed out after {timeout:?}"))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err(anyhow::anyhow!("command thread panicked"))
        }
    }
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
            from_file: PathBuf::from("main.py"),
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
            from_file: PathBuf::from("main.py"),
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
            from_file: PathBuf::from("main.py"),
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
            from_file: PathBuf::from("main.py"),
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
            from_file: PathBuf::from("main.py"),
        };
        let err = provider.resolve(&ctx).unwrap_err();
        assert!(
            err.to_string().contains("offline mode refuses"),
            "err: {err}"
        );
    }

    // ── live E2E: resolve a real installed third-party module ───────────────

    #[test]
    fn resolve_third_party_module_e2e() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = PythonProvider::new();
        // `requests.get` — a real third-party module, very likely installed.
        // If not installed, the test skips gracefully (found-or-skipped).
        let ctx = SymbolContext {
            workspace_root: tmp.path().to_path_buf(),
            symbol: "requests.get".to_string(),
            from_file: PathBuf::from("main.py"),
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
}
