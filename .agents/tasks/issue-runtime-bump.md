# Task: tree-sitter runtime bump 0.24.7 → 0.25.x (unblocks Clojure + modern grammars)

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `.agents/skills/tree-sitter/SKILL.md` (ABI-pinning rules) and
`.agents/plans/archive/011-resolver-parity.md` Gaps item 8 (the recorded
Clojure evidence: tree-sitter-clojure 0.1.0 requires ^0.25.6 and the
`links = "tree-sitter"` mechanism makes 0.24.7 unresolvable alongside it).

## Origin (user directive)

"use new treesitter" — bump the runtime to 0.25.x so the modern grammar
generation is usable (tree-sitter-clojure 0.1.0, and any other grammar
that needs 0.25).

## What to build

1. **The compatibility matrix FIRST** (decision-free evidence): for every
   currently pinned grammar crate (12 + the 4 new: java, c-sharp, ruby,
   scheme), read its tree-sitter runtime requirement from the registry
   index cache. Classify: already-compatible with 0.25.x / needs a crate
   version bump (is there a newer release compatible with 0.25? — note
   the exact versions) / CANNOT follow (no release compatible — the
   language would be lost; report honestly).
2. **The bump**: `tree-sitter = "=0.25.x"` (pick the newest 0.25.x that
   satisfies the most grammars); bump each grammar crate to its
   0.25-compatible version (pin exact, the do-not-bump discipline);
   `all_grammars_set_language_succeeds` is the guard — it MUST pass over
   ALL 18 LanguageIds (the 001/007 crown jewel; one runtime in the lock).
3. **Language losses are BLOCKING-report, not silent**: if a grammar
   cannot follow the bump (e.g. c-sharp 0.23.x requires <0.25 and no
   0.25-compatible version exists), report it as a lost language with
   the evidence — the user decides whether a language is worth blocking
   the bump.
4. **Behavioral pins must survive**: the highlight byte-identity,
   extraction tests, node predicates, the corpus goldens — the bump is
   supposed to be behavior-neutral at our query level (grammar versions
   may bump too, which CAN change parses — run the full workspace +
   battery; any golden/parse drift is reported per grammar, not
   silently absorbed).
5. **Then Clojure**: with 0.25.x in the lock, `cargo add tree-sitter-
   clojure` (0.1.0) should resolve; land Clojure per the new-languages
   pattern (registry + outline query + predicates + walk set + coverage
   row + ABI guard passes over ALL now 19 languages) — if clojure 0.1.0
   still fails (e.g. needs a newer 0.25 minor), report the exact bound.

## Constraints

- Gate: `cargo test --workspace` + clippy (PIPESTATUS exit) +
  `tools/gate.sh full` (flock, cargo build first). Budget ~55 tool
  calls; staged commits (compatibility matrix → the bump → per-grammar
  fixes → clojure); honest-stop at half.
- Scope fence: `Cargo.toml`/`Cargo.lock` (the runtime + grammar bumps),
  the syntax files the compat fixes require (registry.rs, queries.rs,
  node.rs — ONLY where a grammar version change forces a fix), the
  vendored C# highlights (if the crate bump moves it — keep the
  verbatim-checksum pattern), coverage/matrix rows (the Clojure gap
  cell + the runtime note). NO provider changes; no golden absorption.
- Parallel lanes: none — the tree is yours (the watchlist batch was
  NOT dispatched).
