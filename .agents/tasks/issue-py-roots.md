# Task: python workspace-root honesty — src/ layouts (011-08 python finding 2)

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `.agents/skills/*.md` as ground truth.

## Origin

The python golden-corpus README finding 2 (011-08, pinned as an honest
observation): `python_provider.rs` resolves workspace imports via
`find_spec` with the CWD on sys.path — a `src/`-layout package (sources
under `src/pkg/`) fails find_spec in the offline-bail shape because the
package isn't importable from the buffer's directory. The finding is
documented, NOT probed. This issue decides and implements.

## What to build

1. **The decision** (state it in the report with rationale): when the
   provider's CWD-based find_spec misses, can the provider find a
   src/-layout package honestly? Options: (a) walk upward from
   `from_file`'s dir for a `pyproject.toml` (PEP 621) and add its
   `src/` (or the pyproject dir) to sys.path for the find_spec probe —
   the provider already shells to python3 with a controlled environment;
   adding a sys.path entry derived from a FOUND pyproject.toml is
   honest root discovery, not a guess. (b) bail dedicated. Decide by
   reading `resolve_python`'s flow (module_chain → find_spec probe) and
   the existing stdlib-root discovery pattern (sysconfig) for the
   shape of honest discovery. Prefer (a) if the machinery fits; the
   corpus README's finding note must flip to FIXED with the hash.
2. **Pin it**: add a corpus probe (the reviewer suggested `root=
   project_src` — same corpus under src/ → deterministic outcome) OR a
   unit test with a src/-layout fixture. The 011-08 discipline applies:
   the probe must discriminate the fix (today it bails; after the fix it
   resolves).
3. **Re-bless any flipped golden** with the suite's (retrofitted) bless
   flow; keep every other golden byte-identical.
4. **Matrix**: the python workspace-resolution honesty note (the README
   finding 2 text) updates to the new truth.

## Constraints

- Gate: `cargo test --workspace` (live python3 legs) + `tools/gate.sh
  full`. Budget ~35 tool calls; honest-stop provision.
- Scope fence: `crates/redline-resolve/src/providers/python_provider.rs`,
  `crates/redline-resolve/tests/corpus/python/` (probe + golden + README
  notes — a flip here is the ACCEPTANCE), provider unit tests. NO
  js_provider/go_provider/cargo/store.rs changes. A parallel lane owns
  `docs/provider-matrix.md` — do not edit it.
