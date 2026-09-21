# Task: an annotations picker — a temporary list of every annotation

The user wants annotations navigable through a **temporary picker list, like find-file**
— filter, move within the list, RET to jump — rather than `M-n`/`M-p` next/prev over
annotations. This is plan 015 issue 01, and it is **independent of the editing-mode
work**: it needs no change to the point model.

## Why this is small: the picker machine already exists

`PickerKind` (`src/app/store/mod.rs:557`) is a 12-variant enum
(`Palette, FindFile, RecentFiles, Buffers, KillBuffer, Projects, Xref, Impls, Imenu,
Symbols, Branch, Stash`) opened uniformly:

```rust
self.open_picker(PickerKind::X, "prompt ", candidates)   // candidates: Vec<PickerCandidate>
```

with a per-kind candidate builder (`fn xref_candidates(&self)`, `fn imenu_candidates`,
…), a per-kind total (`src/app/store/picker.rs:490-503`), and a **per-kind accept
dispatcher: `run_selected()`** (reached from `picker_key_event`'s Enter arm in
`src/app/store/keys.rs`).

So this lane adds one variant, one candidate builder, one accept arm, one total arm —
and inherits fuzzy filtering (nucleo), name-first rows, `j`/`k`/`C-n`/`C-p` navigation,
the prompt line, and the picker's own keys.

## What to build

1. **`PickerKind::Annotations`** — add to the enum with a doc comment in the file's
   established style.
2. **`annotations_candidates(&self) -> Vec<PickerCandidate>`** over
   `self.notes_doc.entries`:
   - **Include ONLY `NotesEntry::Record(ann)` entries.** `NotesEntry` is an enum of
     `Record(Annotation)` and `Raw(String)`; `Raw` is preserved-verbatim non-annotation
     content (the doc comment at `notes_doc.rs:19` says it is "kept verbatim … never
     [interpreted]"), so it must NOT appear as a candidate. **Filtering this correctly
     is the one substantive correctness requirement of the lane.**
   - **Row shape:** `name` = the annotation **text**, `detail` = `path:line`. (The
     name-first convention — the name owns the space and the detail truncates
     tail-keeping.) If a single-line annotation text is very long, the existing
     cell-aware truncation handles it; do not add new truncation logic.
   - **Sort: document order** (as stored: file, then line) — predictable, and it is
     what the notes document already holds. Do not sort by recency.
   - Ensure `notes_doc` is loaded (`ensure_notes_doc()`) before building candidates, or
     state why not.
3. **Accept (`run_selected`): jump to the annotation's `(path, line)`.** Reuse the
   existing jump/landing path that other location pickers use (see how
   `PickerKind::Xref` / `Imenu` / `Impls` land) — do **not** hand-roll a new
   file-open + `set_point` sequence. Note this lands through the column-aware landing
   path that landed recently, so the cursor lands on the annotated line.
4. **The per-kind total arm** (`picker.rs:490-503`) so the count row is right.
5. **Command + binding.** Register the command in the registry
   (`src/app/command.rs`) and bind it. Proposal: **`C-c n a`** (notes → annotations);
   `C-c n` is free, and note that `C-x n` is *already a complete binding*
   (`open-notes`) — the engine forbids a command on a strict prefix of a longer
   binding, so do **not** build a `C-x n …` sequence. Confirm the choice in your
   report; the exact binding is a detail, the command is the deliverable.

## Optional, only if budget allows

- **`d` in the picker deletes the selected annotation** (the notes view already has
  `annotate-delete`). Do this only if the picker's key handling has a clean place for
  it; otherwise leave it and say so.
- Editing an annotation needs **no new command**: RET to jump, then `A` (which already
  prefills from the existing annotation on that line).

## Key decisions

- **Do not change the notes document format** or the annotation records.
- **Do not touch the editing-mode/point model** — that is issue 02.
- **Preserve the picker's existing behaviour** for every other kind; the new arm must
  be additive.
- **Empty state:** with no annotations, the picker should open with an empty list (the
  picker already handles that) rather than erroring. If it is better to message
  "no annotations" and not open, argue it — but do not silently open a broken list.

## Files

`src/app/store/mod.rs` (the `PickerKind` variant), `src/app/store/picker.rs` (the
candidate builder + the total arm), `src/app/store/keys.rs` or wherever `run_selected`
lives (the accept arm — **check where it is**; it may be in `picker.rs`), and
`src/app/command.rs` (registration). Tests: `src/app/store/tests/picker.rs` (or
`tests/notes.rs` for annotation-record fixtures).

## Verification

- `cargo build`; `cargo test --workspace` — reconcile against the **current** baseline
  (**713** passed / 0 failed / 2 ignored redline + **154** `redline-syntax` = 867;
  resolver 123/0/4; integration 7,2,3,1,1; doctests 0 — measure it yourself), and
  account for the tests you add.
- `cargo clippy --workspace --all-targets -- -D warnings` (read `${PIPESTATUS[0]}`).
- **Tests to add**: (a) the candidate builder includes `Record` entries and **excludes
  `Raw`** (the discriminating one — a `Raw` entry in the notes doc must not appear);
  (b) the row shape (name = text, detail = `path:line`); (c) accept lands on the
  annotation's file+line; (d) the count row. Prefer a fixture with at least one `Raw`
  entry so (a) genuinely discriminates.
- **`timeout 900 tools/gate.sh full`** — the picker is PTY-visible, so the battery
  matters. Recent gates have all run it successfully with swap full.
- Report: the variant/candidate/accept/total diffs, the binding you chose, the
  `Raw`-exclusion test's assertions, and the gate output.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.
