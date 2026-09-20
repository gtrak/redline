# Task: whole-path M-. for Java/C#/Ruby (dotted_path_container arms) + doc sweep

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `.agents/tasks/issue-rung4-and-paths.md` (the landed pattern — C/Cpp/
Toml arms) and the 011-06 guards.

## Origin

The new-languages lane landed Java/C#/Ruby/Scheme, but the app-side
whole-path upgrade (`dotted_path_container`, store.rs:10025) does not
enumerate their containers — M-. on `o.method()` / `o.field` in Java/C#/
Ruby stays bare (the coverage rows say so explicitly; the queued
follow-up). This issue closes it, plus the stale-doc sweep.

## What to build

1. **Container arms**: Java `field_access` + `scoped_identifier`/
   `scoped_type_identifier`; C# `member_access_expression` +
   `qualified_name`; Ruby `call` with receiver + no arguments (the
   node.rs position gate + container-validity rule apply — a Ruby call
   WITH arguments must never match; the grammar analysis is in the
   landed node tests). Verify the node kinds against node.rs's landed
   probes (they're pinned there — do NOT re-probe from memory).
2. **Rust self.*/Scheme untouched**; the 011-06 all-identifier guard
   still gates everything (a Ruby `a.b(1).c` → the whole argumentless
   outer call; an argument-carrying segment → degrades — verify the
   interaction per shape and pin it).
3. **e2e pins per language**: M-. on a Java/C#/Ruby member access lands
   via the index fall-through with the whole path (the index stores
   class/method names — judge which segments match the outline symbols
   and pin the honest landing; degradation byte-for-byte where nothing
   matches).
4. **Doc sweep**: the coverage rows (Java/C#/Ruby/Scheme) + the matrix's
   new-languages section update to the new truth (the "does not
   enumerate" notes → the landed arms, cited); also the C/Cpp/Toml rows'
   stale "extraction stays bare" text (superseded by the rung4
   enumeration — re-verify each cell against code before rewriting).
5. **Clojure**: nothing (no registry variant).

## Constraints

- Gate: `cargo test --workspace` + clippy (PIPESTATUS exit) +
  `tools/gate.sh full` (flock, cargo build first). Budget ~35 tool
  calls; honest-stop at half.
- Scope fence: `src/app/store.rs` (dotted_path_container + tests),
  `docs/language-coverage.md`, `docs/provider-matrix.md`. NOTHING else.
