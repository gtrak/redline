# Task: redline-resolve — Python provider (parallel lane)

You are the implementation worker. Repo root is your cwd.

**Scope fence (absolute): you own exactly ONE file:
`crates/redline-resolve/src/providers/python_provider.rs`.** Other
providers (go_provider.rs, js_provider.rs) belong to parallel lanes —
never touch them, never touch lib.rs, Cargo.toml, src/, or tools/. Your
module is pre-declared; your `#[cfg(test)] mod tests` lives inside your
file.

Context: `crates/redline-resolve` is an app-free resolver crate. Read
`src/lib.rs` (ToolingProvider, SymbolContext, ResolvedSource,
crate_from_symbol) and `src/cargo.rs` (the CargoProvider — the shape to
mirror). Operator directive sanctions fetch-on-demand via the language's
tooling. Toolchain available: python3 3.14, pip3.

## What to build (python_provider.rs, name "python", languages ["python"])

Resolution for a SymbolContext on a Python project:
1. Module path from the symbol: `os.path.join` → module chain `os.path`,
   item `join`; `requests.Session.get` → module `requests` (+ maybe
   submodules), item best-effort. Bare `Session` → Err "needs scope info"
   (same discipline as cargo's bare-symbol rule).
2. Locate the module FILE via the interpreter itself (the language's own
   tooling — sanctioned): `python3 -c` with `importlib.util.find_spec`
   printing `origin` for the module chain; walk up the dotted chain on
   failure (`a.b.c` → `a.b` → `a`). Prefer the workspace's interpreter:
   `.venv/bin/python` / `venv/bin/python` in the workspace first, else
   `python3` on PATH. subprocess with timeout (deadlines, no hangs).
3. If find_spec fails and the module is NOT stdlib: `pip install <pkg>`
   (fetch-on-demand, sanctioned) into the SAME environment chosen above
   (`.venv/bin/pip` if present, else `pip3 --user`? NO — plain `pip3
   install` honoring an optional VIRTUAL_ENV; document the choice), then
   re-resolve. Stdlib modules (os, json, …) never install.
4. Item locate in the resolved file: definition-shaped regex
   (`^def X`, `^class X`, `^X =` for module constants), best-effort line.
5. ResolvedSource: file, source_root = the package dir (site-packages or
   the workspace package), external = site-packages/venv (true) vs
   workspace package (false).
6. Failure modes: no interpreter → Err; module unknown + install refused
   → Err; item not found → Err naming module + file.

## Tests (inside your file)

- tempdir workspace: resolve a stdlib module (`json` → its real file,
  external=true) without network.
- dotted-chain fallback: `os.path` → origin = os/pathlib? (verify at
  runtime; `os.path` is odd — it resolves to os.py; handle and document).
- definition regex: matches `def`/`class`/assignment, rejects imports,
  comments, strings containing "def ".
- venv preference: build a fake `.venv/bin/python` (a shell script that
  echoes) and assert it's chosen over PATH python? (spawning a fake python
  breaks find_spec — design the test so the preference is asserted by
  PATH-ORDER construction, not by running the fake).
- workspace package (external=false): a local `mypkg/__init__.py` found
  via sys.path insertion of the workspace? (find_spec with cwd — set the
  subprocess cwd to workspace_root; document).
- one live E2E: resolve a real installed third-party module if present
  (e.g. `pyte`) — assert found-or-skipped honestly (no network needed).

## Verification

- `cargo test -p redline-resolve` — all green (your tests + the
  pre-existing 13; you may NOT edit other files' tests).
- `cargo clippy -p redline-resolve --all-targets -- -D warnings` clean.
- Report: design, subprocess contracts (exact python commands, timeouts),
  gate counts, deviations, gaps.
