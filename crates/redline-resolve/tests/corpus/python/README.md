# Python golden corpus (plan 011, issue 08)

A small, genuine package-shaped project exercising the PYTHON provider
(`crates/redline-resolve/src/providers/python_provider.rs`) end to end,
with every probe's expected outcome checked in as a `*.golden` file.
`tests/golden_python.rs` walks the goldens in sorted order; a behavior
change in the provider or the resolution seam shows up as a deliberate
golden diff — never a silent regression.

## Re-bless discipline (011-08 follow-up P2-1)

`GOLDEN_BLESS=1 cargo test -p redline-resolve --test golden_python`
re-blesses the corpus then FAILS the run (never ends green — an accidental
bless cannot slip through); rerun without the env var to verify. The python
goldens are HAND-AUTHORED (each file is both the probe spec and the
expected outcome, pinned byte-for-byte), so a bless does NOT re-render
them (unlike the re-derivable js/go goldens): it writes every golden back
UNCHANGED — a no-op re-bless is a per-file "no change" — and a deliberate
hand-edit round-trips byte-identically. A per-file panic would abort the
loop mid-run; the end-of-test `assert_bless_stopped` makes an accidental
bless never end green. Review `git diff` before committing a re-bless.

## Corpus layout

```
project/                # probe root (`root=project` in every flat-layout golden)
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
project_src/            # probe root (`root=project_src`, probe 016): the
  pyproject.toml        #   SAME project in PEP 621 src/ layout — the package
  src/                  #   lives under src/, so the CWD-based find_spec misses
                        #   and the provider's src/-layout discovery re-probes
                        #   with src/ on sys.path
```

(The `project_src/src/` tree is a byte copy of `project/`'s `main.py` +
`gears/` — same use sites, same line numbers.)

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
  network `pip install` leg is never touched in the gate. Since the
  fix-alias flip of probe 013 (below), no python probe pins the
  missing-module offline-refusal bail — that degradation stays pinned by
  the provider's unit tests (`offline_mode_refuses_missing_module`,
  `aliased_dotted_identity_mismatched_and_unhinted_are_no_ops`). No PTY
  anywhere.
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

## Findings

1. **FIXED in `29042f3a15b2` (fix-alias, plan 011 follow-up)** — the
   python provider never applied the 011-02 alias rewrite
   (`scope_qualified_alias`, used by the JS provider): a path-shaped
   symbol like `engine.torque` (use site of `from gears import engine`)
   ignored the app's scope hint and looked for a top-level module
   `engine` → offline bail (probe 013) / a `pip install engine` attempt
   online. The fix composes `scope_qualified(...).or_else(
   scope_qualified_alias(...))` exactly like `js_provider`; probe 013
   flipped from the offline-refusal bail to the RESOLVED landing
   (`project/gears/engine.py:11`, the `torque` def), and the
   identity/mismatched/unhinted no-op cases stay pinned by the provider
   unit tests.
2. **Workspace resolution is CWD-based for the flat layout** — FIXED in
   `c0ff45c3740d` (src/-layout discovery, plan 011 follow-up): the CWD
   probe still needs the package importable as a top-level name from the
   workspace root, and a `src/`-layout package (`src/gears/`) failed
   `find_spec` in the offline-bail shape. The fix walks up from
   `from_file`'s directory (bounded by the workspace root) for a
   `pyproject.toml` (PEP 621); if the project carries a `src/` dir, the
   provider re-probes `find_spec` with that dir on `sys.path` — the
   sys.path entry is derived from a FOUND file on disk, not a guess (no
   pyproject / no `src/` dir leaves the bail byte-for-byte untouched,
   pinned by the provider unit tests). Probe 016 flipped from the
   offline-refusal bail to the RESOLVED landing
   (`project_src/src/gears/engine.py:6`).

## Determinism notes (011-08 review P2-2)

- Probe 013 now RESOLVES inside the project (fix `29042f3a15b2`), so the
  former "host-installed top-level `engine` module" hazard no longer
  applies: `gears.engine` resolves via the CWD `sys.path` entry the
  probe root provides, ahead of any site-packages `engine`.
- Probe 009's `posixpath.py` landing goes through the frozen-stdlib
  `__file__` fallback (CPython 3.11+ freezes `os.path`; `find_spec`
  origin is `frozen`, the provider imports and reads `__file__`). On
  a pre-3.11 host the direct origin yields the same file, so the
  branch itself is carried by the host version + the unit pin
  `resolve_dotted_chain_fallback` (python_provider.rs).
