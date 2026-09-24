# issue-annotation-per-symbol-creation — `A` edits the line's first annotation instead of the one at point

**The user's report:** *"we only apparently allow a single annotation on a line? So when I create a new
one, I am just editing the old one."* Correct, and the mechanism is narrower than it looks.

## What happens

`src/app/store/notes.rs::record_index_for_line`:

```rust
self.notes_doc.entries.iter()
    .position(|e| matches!(e, NotesEntry::Record(a) if a.path == rel && a.line == line))
```

The `A` path asks for a record **by path + line**, and `.position()` returns the **first** match. So on a
line that already carries an annotation, `A` **edits** it — even when the cursor is on a *different
symbol* on that line, and even when several records exist (it always edits the first). The user hit
this immediately after the marker-cell work: annotate the inner `String` of
`    map: HashMap<String, u32>,`, then annotate the field name — and the second `A` edits the first
annotation.

`delete_annotation_at(line)` has the same line-keyed shape.

## Why it is now the wrong key

Everything else is already per-symbol:

- `Annotation` carries `col` and `syntax: Option<SyntaxAnchor>` (kind + name + scope);
- the re-anchor pass is per record, by symbol;
- the renderer supports **several markers and note rows on one line** — the marker-cell issue pinned
  two markers with two inserted cells and a per-annotation fold arrow;
- the notes file stores one record per annotation, and the picker lists them all.

Only the **creation and deletion** paths still key on the line, which is a leftover from before
annotations were symbol-tied. A line is no longer the unit of annotation; a symbol is.

## Requirement

**`A` acts on the annotation at the POINT, and creates a new one otherwise.** Concretely:

1. Look up the record at `(path, line, col)` — where `col` is the point's column **after the
   symbol-start snap** the marker-cell work introduced, so pointing anywhere inside a symbol addresses
   that symbol's annotation.
2. Exactly one match → **edit** it (today's behaviour, but for the right record).
3. No match → **create** a new record, even if the line already has annotations.
4. Two records that would collide on `(path, line, col)` are still deduped — never two annotations for
   the same cell.
5. `d` / the menu's delete acts on the record **at point** for the same reason, and its label
   ("Delete the annotation on the line") should say so.
6. Empty input on the prompt keeps its meaning — it deletes the record being edited (the one at point)
   — and cancels when there was none.

**Line-tied records** (no symbol at the point: EOL, whitespace, a comment) keep the line as their
effective key, so annotating elsewhere on such a line still edits it. Say explicitly what you chose,
because that is the one case where "at point" cannot be exact.

## Acceptance

- **The user's exact case**: annotate the inner `String` in `    map: HashMap<String, u32>,`, then press
  `A` on `map` — a **second** annotation is created and the first is untouched. Then `A` on `String`
  again edits the first, not the second.
- Two annotations on one line render as two markers and two note rows (already pinned) and both
  survive a save/load round-trip through `.redline-notes.md`.
- `d` at point deletes only that one.
- Empty input at point deletes only that one; on a fresh point it cancels and creates nothing.
- A line-tied point behaves as the chosen rule says, pinned.
- Re-anchor, orphan handling and the picker keep working with several records on one line.
- `cargo test --workspace`, clippy `--workspace --all-targets -- -D warnings`, `tools/gate.sh full`.

## Fence

`src/app/store/notes.rs` (the lookup + `annotate` + the delete paths), the menu label if it changes,
tests and drives. Disclose anything else with before/after.

## Note

The marker-cell issue's own tests built multi-annotation lines **by hand** and recorded, in a comment,
that "the `A` key path dedupes per line" — the tests documented the limitation instead of questioning
it, which is why it survived a symbol-precision rewrite of the anchoring around it.
