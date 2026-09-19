# Python golden corpus (plan 011, issue 08)

A small, genuine package-shaped project exercising the PYTHON provider
(`crates/redline-resolve/src/providers/python_provider.rs`) end to end,
with every probe's expected outcome checked in as a `*.golden` file.
`tests/golden_python.rs` walks the goldens in sorted order; a behavior
change in the provider or the resolution seam shows up as a deliberate
golden diff — never a silent regression.

## Corpus layout

```
project/                # probe root (`root=project` in every golden)
  pyproject.toml        # project marker (not read by the provider)
  main.py               # the real USE SITES every probe references:
                        #   import gears.engine        (plain import)
                        #   from gears import *        (wildcard)
                        #   from gears import engine   (dotted from-entry)
                        #   from gears.engine import spin as rotate  (alias)
  gears/
    __init__.py         # hello() :9, Gearbox :14 (re-exports + own items)
    engine.py           # MAX_TORQUE :3, spin :6, torque :11, Motor :18
    render.py           # stdlib use sites: json.dumps, os.path.join
```

## Golden format

Key/value lines (`#` = comment), one file per probe:

| key | meaning |
|---|---|
| `symbol` | the dotted or bare symbol as the app passes it |
| `from_file` | workspace-relative file the use site lives in (context; the provider does not read it) |
| `scope` | the app's tree-sitter hint (`["gears","engine","spin"]`), `[]` = no hint |
| `root` | probe workspace root, corpus-root-relative (default `project`) |
| `kind` | `resolved` or `bail` |
| `external` / `file` / `source_root` / `line` | the expected `ResolvedSource` (resolved only) |
| `path_base` | `stdlib` → `file`/`source_root` are relative to the live interpreter's stdlib root (`sysconfig.get_path('stdlib')`), resolved live at test time; absent → relative to the corpus copy root. `source_root=.` means the base itself. |
| `bail` | the exact expected bail message, byte-for-byte (bail only); `{root}`/`{corpus}` placeholders for the dynamic absolute paths |

## Live vs. unit verification

- **Live (every resolved probe + probes 013/014's find_spec leg):** the
  provider shells to the REAL `python3` (`find_spec`, `is_stdlib`, the
  frozen-`os.path` `__file__` fallback) with the probe root as CWD —
  `python3 -c` puts the CWD on `sys.path`, which is how the workspace
  package resolves. When `python3` is absent the suite skips LOUDLY.
- **Offline by construction:** the provider is built `.offline()`, so the
  network `pip install` leg is never touched in the gate; the
  missing-module degradation (probe 013) is pinned as the deterministic
  offline-refusal bail. No PTY anywhere.
- **Unit-only:** the def-shape scanner (`def`/`async def`/`class`/
  `ITEM = …`) is pinned by the provider's own `#[cfg(test)]` suite; this
  corpus pins it through real files (probes 001–005).

## Stdlib honesty (read before editing probes 008/009)

Probes 008/009 pin LINE NUMBERS inside the ambient interpreter's own
stdlib. Verified live on **python 3.14.4, Linux** in the gate sandbox:

- `json.dumps` → `json/__init__.py:185` (re-verified 011-08; first pinned
  live by 011-02)
- `os.path.join` → `find_spec` origin `frozen` (CPython 3.11+) → fallback
  `__file__` → `posixpath.py:72`

These drift with the python version (and `posixpath` is the POSIX
spelling — `ntpath` on Windows), so a stdlib-upgrade on the gate host is a
**deliberate, reviewed** golden edit, exactly like any other behavior
change. The test discovers the stdlib root live; only the paths-within
and line numbers are pinned.

## Findings (observed — NOT fixed in this issue)

1. **The python provider never applies the 011-02 alias rewrite**
   (`scope_qualified_alias`, used by the JS provider): a path-shaped
   symbol like `engine.torque` (use site of `from gears import engine`)
   ignores the app's scope hint and looks for a top-level module
   `engine` → offline bail (probe 013) / a `pip install engine` attempt
   online. The corpus pins current behavior; the rewrite gap is a
   candidate follow-up.
2. **Workspace resolution is CWD-based**: the package must be importable
   as a top-level name from the workspace root (flat layout). A `src/`
   layout (`src/gears/`) would fail `find_spec` in the same offline-bail
   shape — not probed here (would need the same follow-up decision),
   noted for honesty.

## Determinism notes (011-08 review P2-2)

- Probe 013's bail assumes the gate host has NO top-level `engine`
  module installed in its python3; a host-installed `engine` would
  change the outcome LOUDLY (golden flip), never silently.
- Probe 009's `posixpath.py` landing goes through the frozen-stdib
  `__file__` fallback (CPython 3.11+ freezes `os.path`; `find_spec`
  origin is `frozen`, the provider imports and reads `__file__`). On
  a pre-3.11 host the direct origin yields the same file, so the
  branch itself is carried by the host version + the unit pin
  `resolve_dotted_chain_fallback` (python_provider.rs).
