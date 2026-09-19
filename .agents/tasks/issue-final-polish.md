# Task: final polish — the queued review P2s across landed lanes

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `.agents/skills/*.md` as ground truth.

## Items (all small, from landed review rounds; each cites its lane)

1. **python src/ hardening P2-1** (py-roots review): canonicalize
   `dir/src` before returning it from `src_layout_root` and require
   `starts_with(canonicalize(workspace_root))` — a symlinked src →
   outside currently satisfies the marker (bounded: outside landings
   flag external=true; no leak, but harden anyway).
2. **python src/ hardening P2-2** (py-roots review): a control-char path
   component makes the re-probe subprocess hard-error (Rust `{:?}`
   emits `\u{…}`, invalid in a Python string literal) — degrade to a
   graceful miss (detect and skip the discovered path, or pre-validate
   the component) instead of hard-erroring.
3. **hint-rel P2-1**: one probe line in
   `resolver_scope_js_relative_import_carries_sibling_path`
   (store.rs ~20485) pinning the `{ default as D }` → `["./m"]`
   entry-only emission (the one shape that deliberately drops the
   member).
4. **js-polish P2-1**: malformed-package.json bail half — unit test
   (malformed intermediate + valid above → walk BAILS, not climbs past)
   for `find_local_path_dep`.
5. **Matrix**: the python section gains the src/-layout row (py-roots
   landed — the js-polish lane correctly left it); verify no other stale
   cell (go count, alias notes verified 011-08).
6. **Bless-flow tiny**: field-level `#[expect(dead_code)]` on
   `golden_go.rs`'s `Probe.bail` (currently struct-level — the reviewer
   judged field-level more precise; keep the reason string).

## Constraints

- Gate: `cargo test --workspace` + clippy + `tools/gate.sh full` (flock
  discipline, `cargo build` first). Budget ~30 tool calls; honest-stop.
- Scope fence: `crates/redline-resolve/src/providers/python_provider.rs`
  (+ tests), `crates/redline-resolve/tests/golden_go.rs`, `src/app/store.rs`
  (ONE test-line addition in the relative pin only),
  `docs/provider-matrix.md` (python row). NOTHING else. No golden flips
  expected anywhere (all six items are hardening/pins/docs); if any
  golden flips, STOP and report.
