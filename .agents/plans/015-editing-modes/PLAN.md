# Plan 015 — two editing modes: annotation (coarse, default) and light cursor-accurate editing

**Status:** design approved by the user; issue 01 (the annotations picker) is
LANDED; issue 02 (honest `current_point_byte` + the per-buffer mode) is the
open keystone and blocks 03 and 04.

## 1. Why

Redline has one editing model today, and it is neither of the two the user wants. The
code calls it "bounded editing", and it is genuinely coarse:

| Operation | Current behaviour |
|---|---|
| self-insert (`notes_insert_char` → `insert_text`) | appends at the **end of the buffer** (`rope.len_chars()`) |
| backspace (`notes_backspace`) | deletes the **last character of the buffer**, wherever the cursor is |
| region | **whole lines** (`current_point_byte()` returns the point's **line start**) |
| `C-y` yank | inserts at the **line start** — neither the append model nor emacs's insert-at-point |

The user wants two modes: **annotation** (coarse; the default for every buffer) and
**light cursor-accurate editing** (opt-in per buffer). Annotations themselves are
created through the `A` prompt (a minified input per file+line, prefilled when one
exists) — so the notes buffer is a *view* of records, not the primary editing surface.

## 2. The decisions (resolved)

1. **Default = annotation mode for every buffer**; edit mode is an **opt-in per-buffer
   toggle**. Binding: **`C-x C-q`** — already bound (`toggle-read-only`), and it is
   exactly emacs's per-buffer mode toggle. No new binding.
2. **Marks stay fine in both modes.** Annotation is about a *specific region*, so an
   exact mark is meaningful even in annotation mode.
3. **Coarse navigation is block motion on `M-n` / `M-p`.** NOT `M-{`/`M-}` (user
   preference). This matches the parity reference exactly — magit binds `n`/`p` to
   `magit-section-forward`/`-backward` and **`M-n`/`M-p` to the *sibling* (coarser)
   variants** (`lisp/magit-section.el:472-475`) — and both keys are free in redline.
4. **Annotations get a temporary picker list**, navigable within the list (like
   find-file) — *not* `M-n`/`M-p` next/prev over annotations.
5. **Yank:** append at the end in annotation mode; insert at point in edit mode.
6. **The keystone: `current_point_byte()` must become honest** (return the true point,
   not the line start). It is the single accessor that *lies*, and that lie is what
   makes the region line-granular, makes `C-y` land at the line start (wrong under
   *both* modes), and keeps `C-x C-q`/`C-x C-x` column work invisible. With it honest,
   each mode's coarseness lives where it belongs — in the operations — and the
   accurate mode mostly falls out. **This is not optional**; every item below depends
   on it.

## 3. The cutline — **SETTLED by the user (2026-09-21)**

**P0 — prerequisite:** honest `current_point_byte()`.

**P0 — mode plumbing:** default annotation; per-buffer opt-in via `C-x C-q`.

**P1 — accurate mode, in priority order:**

| # | Command | Binding | Today |
|---|---|---|---|
| 1 | insert at point | self-insert | appends at end |
| 2 | delete before point | `BACKSPACE` / `C-h` | only in the notes-edit modal |
| 3 | **newline** | `RET` | **unhandled** — cannot split a line |
| 4 | **kill line** | `C-k` | missing |
| 5 | delete char forward | `C-d` | **collides** with `scroll-half-page-down` |
| 6 | kill word / backward | `M-d` / `M-DEL` | missing |
| 7 | **undo** | `C-/` | **absent entirely** — a subsystem, not a command |
| 8 | open line / transpose | `C-o` / `C-t` | missing |
| 9 | universal argument | `C-u` | **collides** with `scroll-half-page-up` |

**P1 — annotation mode:** block motion (`M-n`/`M-p`) + the annotations picker.

**P2:** yank semantics; `C-o`/`C-t` polish.

**SETTLED SCOPE (user, 2026-09-21):** accurate mode takes **①–⑥ and ⑧** — insert at
point · backspace-before-point · `RET` newline · `C-k` kill-line · `C-d` delete-char ·
`M-d`/`M-DEL` kill-word · **`C-o` open-line / `C-t` transpose** (the user: *"I use
transpose"*). Yank semantics (⑦'s neighbours, P2) and `C-o`/`C-t` *polish* stay P2; undo
is plan 016 with **both `C-x u` and `C-/`**.

**The two key collisions are resolved by freeing `C-d`/`C-u`.** They currently scroll a
half page, which is *not* emacs — emacs uses `C-d` = delete-char and `C-u` =
universal-argument. Redline already binds the real emacs page-scroll keys (`C-v`
scroll-page-down, `M-v` scroll-page-up, plus PGDN/PGUP), so the only loss is half-page
scrolling. Recommendation accepted unless the user objects; if they do, move half-page
scroll to another key rather than re-colliding.

**Original recommendation, kept for the record:** the annotations picker first
(self-contained, reuses the picker machine, no dependency on the point model), then P0,
then accurate mode ①–④.

## 4. Success criteria

- Every stage is behaviour-preserving except the behaviour it deliberately changes;
  the workspace suite + PTY battery stay green.
- Annotation mode stays *coarse*: no accidental char-accurate editing creeps in.
- Edit mode is emacs-recognisable for the commands in the chosen cutline.
- The notes buffer can still be parsed by the notes document (no format change).
- `tools/fn_survey.py` shows no production function over ~150 lines introduced.

## 5. Task order

| Issue | Depends on | Notes |
|---|---|---|
| `01-annotations-picker.md` | — | temporary picker over all annotations; reuses `PickerKind`/`run_selected`; **independent of the point model**. Task spec: `.agents/tasks/issue-015-01-annotations-picker.md` |
| `02-honest-point-and-modes.md` | — | honest `current_point_byte()` + the per-buffer mode + `C-x C-q` opt-in. Task spec: `.agents/tasks/issue-015-02-point-and-modes.md` |
| `03-accurate-editing.md` | 02 | the cutline's commands: insert at point · backspace-before-point · `RET` · `C-k`. Task spec: `.agents/tasks/issue-015-03-accurate-editing.md` |
| `04-yank-semantics.md` | 02 | append in annotation / insert at point in accurate. Task spec: `.agents/tasks/issue-015-04-yank-semantics.md` |
| — | — | **undo: its own plan — `.agents/plans/016-undo/PLAN.md`** (an undo stack over rope edits; deliberately not a command) |

**A design resolution found while speccing 02**, which simplifies the plan: `editable`
*already is* the per-buffer opt-in (file buffers open `false`; the notes buffer opens
`true`; `C-x C-q` already toggles it). So 02 adds **no second editability flag** — it adds
a mode that selects *how* modification behaves, with the invariant `Accurate ⟹ editable`.
And because the user ruled that a fine mark is meaningful in annotation mode, the region
becomes **exact in both modes** (the honest point does it); coarse now applies only to
editing shape and to motion.

## 6. Risks

- **The mode flag must not become a second source of truth for editability.**
  `buf.editable` already exists; the mode should be expressed *through* it (or beside
  it with a stated relationship), not as an independent flag that can disagree.
- **Coarse-mode regressions are easy to miss**: the current coarse behaviour is what
  the existing tests pin, so the tests must be *deliberately* re-pinned per mode
  (test authority: an implementation-level pin yields; a requirement-level pin — "the
  region is exact in edit mode" — is kept).
- **`current_point_byte()`'s honesty is a blast-radius change**: it feeds the region,
  kill/copy, yank and `C-x C-x`. Land it as its own commit with the existing
  region/kill/yank tests as the regression net.
