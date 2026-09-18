# Task: plan 007 issue 01 — node-at-point + enclosing scope (the syntax foundation)

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth — especially
`.agents/skills/tree-sitter/SKILL.md` (the runtime API + ABI pinning rules:
verify every call against it; do NOT read docs.rs or fetch).

## Origin

Plan 007 (`syntax-aware linking`): redline already parses with tree-sitter
but keeps only a name→location table. Three capabilities are blocked on the
same missing piece — a query for **the syntax node at a byte offset** and
its **enclosing scope**:

1. M-. extracts symbols by text-splitting (006-02 added `::`-token handling,
   but it is still line text, not syntax).
2. Annotations anchor to line text, so any reformat orphans them
   (007-02 will consume this issue's node info).
3. All four resolver providers refuse bare symbols with "needs scope info
   (tree-sitter)" — they are literally waiting for `scope_path` (007-03).

This issue builds ONLY the foundation, as a plain-Rust API with no app
coupling. 007-02 and 007-03 consume it later; do not implement them here.

## What to build

1. **`node_at(lang, source, byte) -> Option<NodeInfo>`** in a new module
   `src/syntax/node.rs` (plain Rust; no iocraft/tokio — the mod.rs layering
   rule). `NodeInfo { text: String, kind: String, start_byte: usize,
   end_byte: usize, scope_path: Vec<String> }`.
   - Parse `source` with the language's grammar (reuse the registry's
     grammar accessor; do NOT build a second registry).
   - Find the smallest named node containing `byte` that is an
     identifier-ish token (Rust first: `identifier`, `field_identifier`,
     `type_identifier`, `scoped_identifier`, `scoped_type_identifier`,
     `primitive_type`; walk up to the nearest such ancestor when the offset
     sits inside a larger expression). The returned `text` is that node's
     source text (so `tokio::spawn` comes back whole as a
     `scoped_identifier`, not two fragments).
   - `scope_path` is the enclosing item chain from outermost to innermost:
     for Rust, the enclosing `mod`/`impl`/`trait`/`fn` names (e.g. for a
     call inside `impl Foo { fn bar(&self) }` → `["Foo", "bar"]`; include
     a `mod_item`'s name when present). Build it by walking ancestors and
     capturing the item's name child. Keep it simple and honest — an empty
     vec is a valid answer for top-level code.
   - Byte offsets index the RAW source string (callers pass a rope slice;
     document that `byte` is a byte offset, not a char offset).
2. **A per-language extension point**: implement Rust fully; every other
   `LanguageId` returns `None` from `node_at` (callers then degrade to
   today's behavior — that is the plan's explicit "Rust first, graceful
   degradation" decision). Put the Rust node-kind predicate behind a small
   per-language function so other languages slot in later without
   restructuring.
3. **`scope_path_at(lang, source, byte) -> Vec<String>`** (or return it via
   `node_at`; pick one surface and justify it) — 007-03 needs the scope for
   a symbol even when the node lookup is the enclosing identifier.
4. **Expose it plainly** from `src/syntax/mod.rs` (`pub mod node;`) — the
   API must be usable by `crates/redline-resolve` inputs later, so keep it
   free of app types (`crate::syntax::registry::LanguageId` is fine).

## Explicit non-goals

- Do NOT wire this into `xref_find_definitions`, imenu, or annotations
  (007-02/03 do that). This issue ships a tested, callable API.
- Do NOT change `queries.rs`'s existing highlight/symbol queries (they work;
  this is an additional facility). If you add a query string, put it beside
  the others and keep the existing ones byte-identical.
- Do NOT add dependencies. tree-sitter 0.24.7 is already pinned; use its
  `Parser`/`Node`/`TreeCursor` API per the skill.
- Do NOT retain trees or implement incremental parsing (that is 007-04).

## Constraints

- Skills are truth (`.agents/skills/tree-sitter/SKILL.md`; no registry,
  no docs.rs, no fetch). Write-first; compile early. Note: if the skill
  does not document a call you need, use the pinned API conservatively and
  record a skill correction in your report.
- PTY flock: "shared PTY fixture is busy" + exit 3 ⇒ wait and retry; NEVER
  two suites concurrently; wrap EVERY python PTY invocation in `timeout`.
  This issue probably needs no PTY run — say so if so.
- Gate runner: `tools/gate.sh fast` (build + clippy + unit tests) for the
  inner loop; `tools/gate.sh full` for the final gate (whole battery).
- Scope fence: `src/syntax/node.rs` (new), `src/syntax/mod.rs` (one `pub
  mod` line), `src/syntax/queries.rs` ONLY if you add a query string.
  Nothing in `src/app/`, `src/ui/`, `crates/`. Plain `git commit`; do not
  `git add -A` other sessions' files.
- Honest gate counts from actual output.

## Verification (iterate until ALL pass)

- `tools/gate.sh full` green (cargo test total will grow; the PTY suites
  must be UNCHANGED — this issue has no user-visible behavior).
- Unit tests in `src/syntax/node.rs` for Rust:
  - `node_at` on a `tokio::spawn(x)` call returns the whole
    `scoped_identifier` (`tokio::spawn`) — not `tokio` or `spawn`.
  - a field access `self.foo` returns `foo` as `field_identifier`.
  - a type position `Vec<String>` returns `Vec` (and the full node text
    when the offset is inside the scoped form).
  - `scope_path` inside `impl Foo { fn bar ... }` at a statement returns
    `["Foo", "bar"]`; inside a nested `mod` returns the mod name too.
  - offsets at the very start/end of the file, and an offset past EOF, do
    not panic (return `None` or the nearest node — be explicit).
  - a non-Rust language returns `None` (degradation contract).
  - a syntactically broken file (half a `fn`) still returns a node or
    `None` without panicking (tree-sitter error nodes must not crash us).
- No performance assertion needed, but note the parse cost in the report
  (one parse per call is acceptable for this issue; 007-04 addresses reuse).

## Report format

API surface + why; the Rust node-kind predicate; how `scope_path` is built
(ancestor walk); test list with the discriminating ones called out; gate
counts (honest); skill corrections; deviations; the parse-cost note.
