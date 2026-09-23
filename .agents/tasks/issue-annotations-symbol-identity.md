# issue-annotations-symbol-identity — annotations must be tied to the SYMBOL, in every language

**User's directive (2026-09-23):** *"I think annotations are still tied to a line and not the symbol
at point? I want them tied to a symbol."* The user is right: below is exactly how it is tied today.

## What is true today (measured from the code)

The notes record carries `path`, `line`, `col`, `anchor` (the **line's text**), `syntax`
(`Option<SyntaxAnchor{kind, name}>`), `orphaned`. Re-anchoring runs syntax-first, and its comment
claims *"the note follows the symbol, not a coincidental text match"* (survives a 100-line
insertion). But the symbol half only fires when **all** of these hold:

1. **The buffer is Rust.** `build_syntax_index_for_key`: `if lang != LanguageId::Rust { return
   None; }` — so for the **other 18 languages** every record rides the text rules alone.
2. **The point was on an identifier-ish node at creation.** `capture_syntax_anchor` returns `None`
   for EOL, whitespace, and any non-identifier node (`is_syntax_anchor_kind`, a **Rust-only** closed
   list mirrored from `is_rust_identifier_kind`).
3. **`(kind, name)` is UNIQUE in the whole file.** Zero or multiple matches fall through to the
   text rules. A function's name occurs at its definition *and* at every call site, so the symbol
   tie silently degrades to text exactly where it would matter most.

Everything else is the text rules: the stored line still holds the anchor text, else a ±25-line
content search (`ANNOTATION_REANCHOR_WINDOW`), else **flag `orphaned` and never guess** — that last
rule is good and must survive.

## Requirement

An annotation attaches to **the symbol at point, in every supported language**, and follows that
symbol when code moves — not to a line number and not to a coincidental text match. Where no symbol
can be identified, or the identity is genuinely ambiguous, it degrades to today's text rules and
**flags orphaned rather than guessing**.

## Stage 1 — generalise the existing mechanism (all languages)

- Remove the Rust gate; build the index for every language that has a grammar.
- Replace the Rust-only kind list with a **per-language set of identifier-ish kinds**. The grammars
  differ, so this is a table, and it must be *derived from the pinned grammars*, not guessed —
  `node-types.json` in each grammar crate is the authority (the language-highlight lane used exactly
  this method). Where a language genuinely has no identifier kind, saying so is the honest answer.
- **No notes-format change is needed for stage 1**: `syntax_kind`/`syntax_name` already round-trip
  and are already optional/additive.

Stage 1 gives symbol ties wherever `(kind, name)` is unique — i.e. mostly definitions.

## Stage 2 — a scope-aware identity (so usages and repeated names work)

The uniqueness rule is what makes stage 1 weak. Key the anchor so that an occurrence is
identifiable even when its name repeats:

- include the **enclosing definition's** `(kind, name)` — the scope — and the occurrence's
  **ordinal within that scope**;
- so annotating `foo` inside `fn bar` resolves to `bar` + `foo` + ordinal, which is stable when
  unrelated code is inserted above, and does not collide with `foo` in `baz` or with `bar`'s own
  definition.
- Store the extra fields as **new additive keys** (the format already degrades gracefully: absent
  keys → `None` → legacy behaviour), and keep the round-trip test.

If stage 2 proves too large for one lane, land stage 1, say so, and file stage 2 — but **do not
fake it** by loosening the uniqueness rule (that reintroduces guessing).

## Acceptance

- **Per-language matrix**: an annotation on a symbol in at least **Python, TypeScript, Go, C/C++ and
  Rust** follows the symbol across a **100-line insertion above it** (the exact case the Rust-only
  code comments claim to survive). One test per language, driven through the real re-anchor path.
- **A usage site of a repeated name** follows its own occurrence (stage 2) — the case today's
  uniqueness rule breaks. Pin it with a fixture where the name appears at least twice.
- **Ambiguity still degrades honestly**: when the identity cannot be resolved, the record rides the
  text rules and sets `orphaned`; it never migrates to a wrong symbol. Pin that too — a wrong tie is
  worse than no tie.
- **A point that is not on a symbol** (EOL, whitespace, a comment) yields no syntax anchor, and the
  record behaves exactly as today.
- **Legacy records are untouched**: a record with no syntax keys re-anchors by text exactly as
  before, and the notes file round-trips (absent keys → `None`).
- No new panic path: a re-anchor must never index out of bounds (016-01's gate found a reachable
  `Char range out of bounds`, and this code re-anchors while buffers change).
- `cargo test --workspace`, clippy `--workspace --all-targets -- -D warnings`, `tools/gate.sh full`.

## Fence

`src/app/store/notes.rs` (capture + the re-anchor index + the rules), `src/app/store/notes_doc.rs`
(the additive keys and the round-trip), `src/syntax/` for any per-language kind table, tests and
drives. Disclose anything else with before/after.

## Report

Say which languages got a real kind set and which did not (and why). For the matrix, give the
literal per-language evidence that the annotation followed the symbol. If stage 2 is not landed,
say so plainly rather than describing stage 1 as if it solved the repeated-name case.
