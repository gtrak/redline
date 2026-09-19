# Task: plan 007 issue 04 — incremental parse reuse (perf)

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/tree-sitter/SKILL.md` is authoritative ground truth for the
API (no registry reads, no docs.rs, no fetch).

## Origin

Plan 007, issue 04. Every edit currently re-parses from scratch: the highlight
pipeline returns `HighlightResult` and **discards the `Tree`**
(`src/syntax/highlight.rs`), so `invalidate_highlight_for_key` clears the whole
cache and `ensure_highlight` re-parses the file. `Tree::edit(InputEdit)` +
re-parse would reuse the unchanged prefix/suffix, which matters on large files
in edit mode (notes, edit-mode buffers, the commit editor).

## What to build

1. **Retain the `Tree` per buffer** and apply incremental edits:
   - wherever a rope edit happens (the edit-mode/notes/commit-editor insert &
     delete paths), compute the `InputEdit` (byte + point ranges, old/new
     end positions) and call `tree.edit(&edit)` BEFORE re-parsing with the
     old tree as the parse hint (`parser.parse(source, Some(&old_tree))`).
   - `InputEdit` fields are byte offsets and `Point{row,column}` — derive them
     from the rope operation, and be careful with byte-vs-char (ropey gives
     byte offsets via `char_to_byte`; the skill documents `Point`).
2. **Keep the highlight output identical.** The tree is an internal cache: the
   rendered spans must be byte-identical to today's full re-parse. That is the
   correctness bar — prove it with a test that renders the same content both
   ways and compares.
3. **Measure.** Add a bench-ish measurement (a unit test with a large
   synthetic file, timing full vs incremental, or a `--bench`-free timing
   assertion with a generous bound) and report real numbers. If incremental
   parsing is NOT measurably faster for our edit sizes, SAY SO and recommend
   not keeping the complexity — an honest negative result is a valid outcome.
4. **Failure modes**: a stale tree (buffer reloaded from disk, not edited)
   must not poison the cache — fall back to a full parse. An edit whose
   `InputEdit` is wrong produces WRONG highlighting, which is worse than slow;
   so validate (e.g. after incremental parse, if the tree has an error where
   the old one didn't, fall back).

## Explicit non-goals

- No changes to 007-01's `node_at` API or `src/syntax/node.rs` semantics
  (it may reuse the retained tree if that's a clean win, but its public
  behavior must not change).
- No LSP, no new dependencies.
- Do not change `src/app/store.rs` beyond the edit-path wiring needed to
  supply `InputEdit`s.

## Constraints

- Skills are truth. Write-first; compile early. Record any skill correction.
- Gate: `tools/gate.sh fast` inner loop, `tools/gate.sh full` final (the gate
  is `--workspace`; do not regress the resolver crate's coverage). Progress
  streams to stderr; do NOT pipe stdout through `tail`.
- PTY flock: "shared PTY fixture is busy" + exit 3 ⇒ wait and retry; NEVER two
  suites concurrently; wrap EVERY python PTY invocation in `timeout`.
- BUDGET: land within ~60 tool calls; no new investigations after it compiles.
  This is a PERF issue — if the win is not real, report that early rather than
  forcing the change.
- Scope fence: `src/syntax/highlight.rs`, `src/syntax/cache.rs`,
  `src/app/store.rs` (edit wiring + tests). Nothing else.
- Plain `git commit`; do not `git add -A` other sessions' files.

## Verification (iterate until ALL pass)

- `tools/gate.sh full` green with HONEST counts; all existing suites unchanged
  (if a PTY flow's timing assumption changes, call it out).
- **Byte-identical highlight test**: same content, incremental vs full parse,
  identical `HighlightResult`.
- **Correctness under stress**: a sequence of many random-ish edits (insert,
  delete, multi-line, at start/middle/end, multibyte) followed by an
  incremental parse must equal a from-scratch parse of the final content.
  This is the test that catches a wrong `InputEdit`.
- Stale-tree fallback test (reload path does a full parse).
- Measured numbers on a large file (state the file size and both times).
  If not faster, say so plainly with the numbers.

## Report format

Where the tree is retained; how `InputEdit`s are derived (and the
byte/char/Point handling); the fallback rules; the byte-identical proof; the
stress test; MEASURED numbers (full vs incremental, file size); whether the
win is real and worth keeping; gate counts; skill corrections; deviations.
