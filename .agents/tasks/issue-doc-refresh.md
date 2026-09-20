# Task: post-ts-bump doc refresh + queued doc P2s

You are the implementation worker. Repo root is your cwd. Self-contained.

## Origin

The ts-bump lane landed tree-sitter 0.25.10 + Clojure (19 languages).
Its review queued 3 doc P2s + one skill-doc staleness (skills are truth —
out of date docs mislead workers).

## Items

1. **`.agents/skills/tree-sitter/SKILL.md`**: refresh to the 0.25.10
   stack (the runtime version, the ABI window 13..=15, the 19-language
   grammar list with exact pins, the dev-dependency finding — grammar
   crates' tree-sitter reqs are DEV-deps imposing no resolution
   constraint, only the ABI window matters; the C#/Clojure vendored-
   highlights pattern with the sha256/verbatim rule).
2. **`docs/language-coverage.md` gap item 8**: add the one-line addendum
   ("landed by the ts-bump lane — see the runtime note above; the probe
   evidence below is historical") — the review P2-1.
3. **`docs/provider-matrix.md:380-389`**: the "Clojure deliberately not
   landed … links conflict with 0.24.7" claim is now wrong (pin is
   0.25.10; clojure resolves) — fix (review P2-2).
4. **`src/syntax/highlight.rs:287-292`** catch-arm comment: one-word
   update to include Clojure (review P2-3, cosmetic).
5. **`docs/tree-sitter-runtime-matrix.md`**: note the 0.25-gen grammar
   releases (js 0.25.0, c-sharp 0.23.5, etc.) as the recorded optional
   follow-up (deliberately not taken — per-grammar drift decision).

## Constraints

- Mostly docs; item 4 is a one-token comment. Budget ~15 tool calls.
- Commit to main. Report per-item.
