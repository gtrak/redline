# Task: plan 007 issue 02 — syntax-anchored annotations

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth.

## Origin

Plan 007: annotations today store `anchor: target_one();` — a line-TEXT
string. Any reformat (rustfmt), insertion, or rename above it orphans the
note; the ±25-line search (`ANNOTATION_REANCHOR_WINDOW`) is a heuristic
band-aid. A syntax anchor (the node's kind + name, e.g.
`function_item` named `target_one`) survives all three.

The user's words: the annotation system should have "automatic linking
machinery."

## Depends on

**007-01** (`src/syntax/node.rs`): `node_at(lang, source, byte) ->
Option<NodeInfo>` where `NodeInfo { text, kind, start_byte, end_byte,
scope_path }`. Read that module's actual API before starting; the baseline
is whatever 007-01 committed (it fully implements Rust, returns `None` for
other languages). If 007-01 is not yet in HEAD, STOP and report.

## What to build

1. **Extend the annotation record with an OPTIONAL syntax anchor.**
   `Annotation` (`src/app/store.rs`) gains `syntax: Option<SyntaxAnchor>`
   with `SyntaxAnchor { kind: String, name: String }`. Captured at creation
   (the `A` prompt / prefill+commit path) from 007-01's `node_at` for the
   line's first non-whitespace byte offset (or the point's byte offset —
   pick the more stable and justify it): `kind` = the node kind; `name` =
   the identifier text (the node's `text`, or the enclosing item's name if
   the node at the offset is a statement — be explicit about the rule).
   None for non-Rust / when `node_at` returns None.
2. **Serialization stays tolerant and backward-compatible.** The
   `.redline-notes.md` structured section gains keys for the syntax anchor
   (e.g. `syntax_kind:` / `syntax_name:`) — and the parser must:
   - read old records (no syntax keys) as `syntax: None` (no migration,
     byte-identical output for old records that are never touched);
   - preserve unknown keys verbatim (existing behavior — verify it still
     holds);
   - round-trip a record with a syntax anchor exactly.
   A record whose anchor keys are malformed stays a Raw block (never
   dropped), consistent with 005-02.
3. **Re-anchoring prefers the syntax anchor.** In `reanchor_for_key`
   (`src/app/store.rs`, the ±`ANNOTATION_REANCHOR_WINDOW` block):
   - If the record has a syntax anchor, first try to find a node of that
     `kind` + `name` ANYWHERE in the file (parse the buffer's rope through
     007-01). Exactly one match → re-anchor to that line (and un-orphan),
     regardless of distance — this is what makes it survive a 100-line
     insertion.
   - Then the existing line-text check (content at the recorded line still
     matches `anchor`).
   - Then the ±25-line text search.
   - Then orphan.
   Order matters: syntax first, then exact-line text, then the window.
   Zero or multiple syntax matches fall through (never guess), exactly like
   the text search's ambiguity rule.
   Note the parse cost: only parse when at least one record for this file
   HAS a syntax anchor (do not parse for the common legacy case).
4. **Preserve all existing behavior**: the key-derivation point
   `buffer_annotation_path` (008-01) is unchanged; the notes file location
   and format outside the structured section are byte-identical; the
   marker gutter, virtual note rows, count, dump (`DumpAnnotation`) and
   deletes behave the same. If you extend the dump, do it additively and
   keep the existing fields' order for the grep-shaped plain mode.

## Explicit non-goals

- Do NOT change M-. / imenu (007-03 and 006 handle resolution).
- Do NOT implement incremental parse retention (007-04).
- Do NOT add dependencies.

## Constraints

- Skills are truth (`.agents/skills/tree-sitter/SKILL.md` for the parser
  API; no registry/docs.rs/fetch). Write-first; compile early. If code
  contradicts a skill file, code wins + record a minimal skill correction.
- Gate runner: `tools/gate.sh fast` inner loop, `tools/gate.sh full` final
  (progress streams to stderr — do not pipe stdout through `tail`). PTY
  flock: "shared PTY fixture is busy" + exit 3 ⇒ wait and retry; NEVER two
  suites concurrently; wrap EVERY python PTY invocation in `timeout`.
  Honest gate counts.
- Scope fence: `src/app/store.rs` (model + re-anchor + tests), possibly a
  new PTY leg in `tools/` for the reformat-survival drive, `docs/` +
  `README.md` if the annotation UX notes change. No `src/syntax/` behavior
  changes (007-01 is frozen — call it only), no `src/ui/` unless a note-row
  render detail requires it, no resolver crate.
- Plain `git commit`; do not `git add -A` other sessions' files.

## Verification (iterate until ALL pass)

- `tools/gate.sh full` green (523+ tests; all existing suites unchanged
  EXCEPT where behavior legitimately changed — say which and why).
- Unit tests:
  - **the headline test**: annotate a Rust fn, then simulate a 100-line
    insertion above it AND a rustfmt-style reformat; on re-anchor the note
    follows the function (not orphaned). Prove it is discriminating by
    showing it would fail with only the ±25-line path (e.g. an insertion
    beyond the window).
  - legacy record (no syntax keys) still re-anchors by text and is written
    back byte-identically if never moved.
  - ambiguous syntax match → no guess → orphan.
  - malformed anchor keys → Raw block preserved.
  - non-Rust buffer → `syntax: None`, old behavior intact.
  - round-trip: record with syntax anchor → serialize → parse → equal.
- A PTY leg if the drive is practical: annotate, externally rewrite the
  file with an insertion + reformat, reload, assert the note row moved with
  the function. If not practical in PTY, say so and rely on the unit test
  plus an honest note.

## Report format

The `SyntaxAnchor` capture rule (which offset, which node, why); the
serialization keys + backward-compat proof; the re-anchor order; test list
with the discriminating headline test called out; gate counts; skill
corrections; deviations.
