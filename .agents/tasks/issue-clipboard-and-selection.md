# issue-clipboard-and-selection: M-w reaches only the internal kill ring, and the captured mouse blocks terminal selection

**User report (verbatim):** *"I also need to be able to copy/paste from the terminal text, and I can't
highlight with the mouse, and M-w doesn't send to clipboard."*

Three separate defects, one theme: **text cannot leave redline.**

## 1. `M-w` copies to the internal kill ring and never to the system clipboard

`M-w` is bound and implemented (`src/app/store/mod.rs:350` → the `copy-region` command;
`src/app/command.rs:935`: *"Copy the marked region to the kill ring (M-w)"*). It is correct
emacs behaviour — `kill-ring-save` — but it is **not what the user asked for**: the text is only
reachable by yanking inside redline.

**There is no clipboard integration anywhere.** Exhaustive search for `clipboard`, `Clipboard`,
`osc52`, `OSC52`, `\x1b]52`, `]52;` across `src/` and `Cargo.toml` → **zero hits**.

**Fix: emit OSC 52 (`ESC ] 52 ; c ; <base64 of UTF-8> BEL`) when the region is copied to the kill
ring.** OSC 52 is the terminal-native clipboard escape: no dependency, works over SSH (where a
clipboard crate cannot), and needs no new crate. Caveat to measure, not assume: some terminals
(iTerm2 by default, some multiplexer configs) ignore or restrict OSC 52 — record what this terminal
does rather than asserting it works. A clipboard crate is an acceptable *additional* sink but must not
be the only one.

Requirements:
- `M-w` continues to populate the kill ring (emacs parity — `C-y` must still yank it).
- The same text is also sent to the system clipboard.
- Base64 must be correct for multi-byte UTF-8 (the annotations and the source are full of it), and
  the payload must be size-guarded (a large region should not emit a multi-megabyte escape; state
  the cap and the behaviour past it).
- The copy path must not corrupt the terminal state when the payload contains control bytes — the
  base64 encoding is what makes that safe; do not "simplify" it away.

## 2. The mouse cannot highlight text, because the app captures the mouse

The app handles mouse clicks (that is how `mouse_click_position` and the tree clicks work), so
iocraft enables mouse reporting. **A terminal only offers its own drag-to-select when the application
is *not* reporting mouse events**, which is why highlighting is impossible today.

iocraft *does* surface the events needed to fix this in-app — `use_terminal_events` delivers
`FullscreenMouseEvent { kind, row, column, modifiers }` with
`MouseEventKind::Down/Up/Drag/Moved` and `MouseButton::{Left,Right,Middle}`
(`.agents/skills/iocraft/SKILL.md:391-393`) — so a drag can drive a redline-owned selection.

**The region is already rendered**, so this needs no new rendering: `FileView` paints the region
background for every row whose buffer line falls in the region
(`src/ui/file_view.rs:96-101`, from `props.region_lines`).

**Fix: a left-button drag sets the mark at the press and the point at the drag position**, so the
existing `region_lines` highlight appears and `M-w`/`C-w` act on exactly what is highlighted. State
the behaviour on: release (does the selection persist for `M-w`, as emacs's mark does, or clear?),
a plain click (must remain a point-set — the existing click paths and their tests are a contract),
and drags outside the file view (the tree, the picker) which must not create a region.

Known limit to disclose rather than hide: `region_lines` is **line-granular**, so a drag highlights
whole lines. Character-precise selection is a refinement, not part of this issue — but the column is
available at the press/drag, so do not design something that would prevent it.

## 3. Pasting — establish what actually happens before changing it

The user asks for paste too. Determine, by measurement, whether iocraft surfaces bracketed paste at
all (the skill documents `TerminalEvent::Key`/`Resize`/`FullscreenMouseEvent` and no `Paste`
variant), and whether a terminal paste currently arrives as ordinary key events that self-insert
correctly in an edit buffer. If pasting already works, say so with evidence; if it does not, scope it
explicitly rather than half-implementing it.

## Acceptance

- Copy the region with `M-w` and paste it into **another program** — the text arrives.
- `M-w` still populates the kill ring (`C-y` yanks it) — emacs parity is not traded away.
- Drag with the left button over the file view → the selection is visibly highlighted → `M-w` copies
  exactly that text to another program.
- A plain click still sets the point (existing tests unchanged); a drag in the tree/picker does not
  create a region.
- Multi-byte text (annotations, non-ASCII source) survives the round trip byte-for-byte.
- The paste question is answered with evidence, whichever way it goes.

## Verification notes for the implementer

- **A test never vetoes a requirement**; gates must execute. A unit test that asserts "OSC 52 was
  written" is a claim about a string — the *requirement* is that another program receives the text.
  Where a live check is impossible, say so explicitly and pin the byte-exact escape sequence instead
  (base64 of a known multi-byte string) so the encoding cannot silently rot.
- The known battery failure is `check_cursor_stream.py`'s `menu@80` (filed, pre-existing) — do not
  chase it; confirm it is the only one.
- Fence: the copy/kill-ring path, a new clipboard module if you add one, the mouse-event seam, and
  tests. Disclose every other file with before/after.
