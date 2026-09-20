# Task: 0.25-generation grammar crate bumps (the queued per-grammar drift decision)

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `docs/tree-sitter-runtime-matrix.md` (the recorded follow-up list and
the dev-dependency/ABI-window model), `.agents/skills/tree-sitter/SKILL.md`
(the current 0.25.10 stack + pin table), and the corpus READMEs.

## Origin (queued follow-up, user: continue the queue)

The runtime bump to 0.25.10 was deliberately behavior-neutral: NO grammar
crate was bumped. Newer grammar releases in the 0.25 generation are
available (js 0.25.0, c-sharp 0.23.5, rust 0.24.1/0.24.2, python/go
0.25.0, c 0.24.0–0.24.2, bash 0.25.0/0.25.1, yaml 0.7.1/0.7.2, md 0.5.1,
…). Each is a per-grammar DRIFT decision: a newer grammar can change
parses → highlighting, extraction, node predicates, corpus goldens.

## What to do (per grammar, staged commits)

1. **Enumerate the candidates** from the registry index cache: for every
   pinned grammar, the newest release that works with the ABI window
   (13..=15) and compiles against our runtime. Note each candidate's
   changes if determinable (CHANGELOG/commit log in the crate source).
   NOTE: `tree-sitter-md 0.5.1` was observed as a known-bad straggler
   (the 011-resolver-parity cache note) — verify whether it works now
   (ABI window + query compat) and report honestly either way.
2. **Bump ONE grammar at a time** (exact pin, do-not-bump comment
   preserved), then run the FULL behavioral evidence for that grammar:
   - the extraction tests + highlight byte-identity (`highlight.rs`'s
     reusable-vs-highlighter tests)
   - the node predicates/scope tests
   - the corpus goldens for that language (if any) — a golden flip is
     DRIFT and must be judged: is the new parse CORRECT (accept + note)
     or a regression (revert the bump)?
   - the ABI guard (`all_grammars_set_language_succeeds`)
3. **Keep-and-report or revert-and-report per grammar.** The deliverable
   is a per-grammar verdict table: bumped (no drift) / bumped (drift,
   accepted with evidence) / NOT bumped (drift, rejected — with the
   specific diff) / NOT bumped (candidate incompatible/unavailable).
   Never absorb drift silently; never bump wholesale.
4. **Update** the runtime matrix's follow-up section with the verdict
   table + the new pins if any grammar moved.

## Constraints

- Gate: `cargo test --workspace` + clippy (PIPESTATUS exit) +
  `tools/gate.sh full` (flock, cargo build first). Budget ~55 tool
  calls; staged per grammar; honest-stop at half (report the table
  partial + what remains).
- Scope fence: `Cargo.toml`/`Cargo.lock` (the grammar pins only),
  `docs/tree-sitter-runtime-matrix.md`, `docs/language-coverage.md`
  (only if a grammar's capability row changes), the syntax files ONLY
  where a grammar's changed node kinds force a predicate/query fix
  (report each), `third_party/` if a vendored highlights file must be
  re-copied (the C#/Clojure pattern: verbatim + sha256). NO store.rs /
  main.rs / provider changes. Parallel lanes own store.rs (jumpback)
  and main.rs (input-lat) — do not touch either.
