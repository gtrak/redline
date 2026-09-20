# Task: post-grammar-bumps doc/comment refresh (the queued review P2s)

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `docs/tree-sitter-runtime-matrix.md` (the verdict table — the source
of truth for the new pins) and `.agents/skills/tree-sitter/SKILL.md`.

## Origin (grammar-bumps review, 2026-09-20)

Nine grammar pins moved (rust 0.24.2, js 0.25.0, python 0.25.0, go 0.25.0,
c 0.24.2, bash 0.25.1, yaml 0.7.2, c-sharp 0.23.5, md 0.5.1) on the
0.25.10 runtime. Six stale doc/comment sites were flagged (all
report-only). Fix them all:

1. **`Cargo.toml`** block header "grammar pins: UNCHANGED by the runtime
   bump … byte-for-byte the 0.24.7 ones" — now false (9 moved). Reword to
   point at the verdict table.
2. **`Cargo.toml`** the "c-sharp 0.23.5 exists — a follow-up lane, do not
   bump" sentence — contradicted by the actual `=0.23.5` pin. Update.
3. **`docs/language-coverage.md`** "Runtime note (ts-bump lane)" claims
   "all 16 pre-existing grammar pins UNCHANGED" / ABIs "13/14" — add the
   one-line addendum pattern (point at the verdict table; md 0.5.1 is ABI
   15).
4. **`docs/tree-sitter-runtime-matrix.md`** "at `7505583` (5 committed
   bumps + the yaml WIP lock state)" — off-by-one: 6 committed bumps
   preceded the checkpoint.
5. **`.agents/skills/tree-sitter/SKILL.md`** (the big one): the pin table
   (all 9 pre-bump pins), the "dev-dependency finding (why the bump forced
   zero grammar moves)" claim (now false — grammar moves DID happen), and
   the Gotchas line "0.25-gen follow-up releases deliberately not taken"
   (now false). Refresh all three against the verdict table; keep the
   skill's terse voice; verify each pin against `Cargo.toml`/`Cargo.lock`
   before writing.
6. **Probe-record comments** referencing pre-bump pins: `src/syntax/
   queries.rs` (C# "pinned tree-sitter-c-sharp 0.23.1"), `src/syntax/
   node.rs` (js sexp probe records, 4 sites) — update the referenced
   version numbers where the pin moved (comments only; add
   "(re-pinned by the grammar-bumps suite)" where useful).

## Constraints

- Comments/docs only — no behavior changes; `Cargo.toml` comments must not
  alter any pin. Gate: `cargo test --workspace` + clippy (PIPESTATUS exit)
  to prove the comment edits compile; no PTY battery needed (no runtime
  behavior touched) — say so in the report.
- Budget ~20 tool calls.
- Note: `src/syntax/*` doc-comment edits are in-fence for THIS lane (the
  grammar lane's fence did not include them).
