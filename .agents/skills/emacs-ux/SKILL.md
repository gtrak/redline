---
name: emacs-ux
description: >-
  Authoritative reference for core GNU Emacs interaction UX — minibuffer and
  echo-area prompts, keymaps and prefix keys, C-g cancel semantics, universal
  argument, M-x, motion and paging keys, incremental search (isearch), buffers
  and buffer list, mode line, xref navigation (M-./M-, and the *xref* buffer),
  imenu, and occur — sourced from the GNU Emacs 31.1 manual with verified
  keys and explicit NOT VERIFIED markers. Use when implementing redline issues
  01 (app skeleton/command system), 02 (file picker), 03 (syntax & file view),
  05 (symbols & jump), 06 (search & references) so the TUI behaves like Emacs
  without relying on model training data.
---

# emacs-ux

In-repo reference for building Emacs-flavored interaction in redline. Every
behavior below was read from the GNU Emacs manual (version 31.1.50) during a
single research session; node URLs are listed in Sources. Anything not read
from a fetched page is marked **NOT VERIFIED** — do not implement from memory.

# Manual version

GNU Emacs Manual, "updated for Emacs version 31.1.50" (stated on the manual
index page). All keys/behaviors below are as documented in that edition.

# Input model

## Keys, commands, keymaps (node: Commands.html)

- Emacs does not assign meanings to keys directly. Keys are *bound* to named
  *commands* (dashed English names like `next-line`, `forward-char`); the
  bindings live in tables called *keymaps*. "C-n moves down one line" is
  shorthand for "C-n is bound to `next-line`".
- Consequence for redline: the command registry + keymap engine (issue 01)
  mirrors this exactly: named commands, key sequences → command bindings,
  rebindable.
- Key input (node: User-Input.html): characters, control characters (RET,
  TAB, DEL, ESC, function keys, arrows), and modifier combinations. `C-x` =
  Control+x, `M-x` = Meta (Alt)+x. Meta can also be typed as the two-key
  sequence `ESC x` (ESC is pressed and released, not held). This two-key form
  matters for terminals where Alt is unreliable.

## Prefix keys

- A prefix key (e.g. `C-x`, `C-c`, `M-g`) has no meaning alone; it waits for
  a continuation. `C-x C-f`, `C-c p f`-style sequences, and `M-g g` /
  `M-g c` (two distinct commands under the `M-g` prefix) are all documented
  in the fetched nodes.
- Pending-prefix feedback: the fetched nodes do NOT document Emacs's own
  echo-area display of a pending prefix (that is in the Elisp manual's
  command-echo node — **NOT VERIFIED** here). The Emacs manual does document
  the third-party `which-key` minor mode (node: Key-Help.html): "displays
  keybindings following your currently entered incomplete command (prefix),
  in a popup." Redline's pending-key indicator in the status line (issue 01)
  is the analog; exact Emacs echo format is NOT VERIFIED.

## C-g — quit (node: Quitting.html)

`C-g` (`keyboard-quit`) cancels "a running or partially typed command."
Context-specific semantics, all verified:

| Context | C-g does |
|---|---|
| Partially typed command / pending prefix | Cancels it; back to command level |
| Command still running | Stops it "in a relatively safe way" (e.g. kill: text either all still in buffer or all in kill ring) |
| Region active | Deactivates the mark (unless Transient Mark mode off) |
| Minibuffer active | "Quits the command that opened that minibuffer, closing it" |
| Incremental search | Special — see isearch section; "it may take two successive C-g characters to get out of a search" |
| Numeric argument being typed | Cancels the argument |

Implementation detail noted by the manual: C-g works by setting `quit-flag`
to `t` the instant it is typed; running code checks it frequently. C-g is only
executed as a command when Emacs is waiting for input. On a text terminal, a
second C-g typed before the first is recognized triggers emergency escape
back to the shell (not relevant to redline, but explains why double-C-g
behavior must be deliberate, not accidental).

`ESC ESC ESC` (`keyboard-escape-quit`) "can either quit or abort": cancels a
prefix argument, clears a region, exits query replace (like C-g), exits a
minibuffer or recursive edit (like C-] ). It cannot stop a running command.

`C-]` (`abort-recursive-edit`) exits one recursive editing level and cancels
the invoking command — C-g deliberately does NOT do this.

## Universal argument — C-u (node: Arguments.html)

- `C-u` alone = argument 4 ("multiplies the argument for the next command by
  four"); `C-u C-u` = 16 (`C-u C-u C-f` moves forward 16 chars).
- `C-u` followed by digits = that number; `C-u -` (minus without digits) = −1;
  negative arguments make motion commands act in the opposite direction.
- `M-<digit>` (`digit-argument`) also sets a prefix; subsequent digits need
  not hold Meta (`M-5 0 C-n` moves down 50 lines).
- "No argument is equivalent to an argument of one" for repeat-count commands;
  some commands treat a plain `C-u` as a boolean flag (e.g. `M-q` justifies;
  `C-u C-x C-b` lists only file-visiting buffers).
- Prefix arguments are typed BEFORE the command (vs. minibuffer arguments,
  entered after invoking the command).
- Redline mapping: issue 01 keymap engine should support a pending numeric
  argument state; `C-u C-x C-c`-style quit paths and `C-u M-.` (prompt for
  identifier) depend on it.

## M-x — run command by name (node: M_002dx.html)

- `M-x` (`execute-extended-command`): minibuffer prompt is the literal string
  `M-x`. Type command name, RET runs it.
- Completion is available on command names (`M-x forw TAB c RET` example).
  M-x completion ignores commands obsolete in a previous major version; can
  exclude mode-irrelevant commands (`read-extended-command-predicate`);
  `M-S-x` restricts the list to commands belonging to the current major/minor
  modes.
- C-g at the prompt: "To cancel the M-x and not run a command, type C-g
  instead of entering the command name. This takes you back to command level."
- Prefix argument before M-x: `C-u 42 M-x forward-char RET` — "the argument
  value appears in the prompt while the command name is being read".
- After running a command that HAS a key binding, Emacs echoes a hint in the
  echo area (e.g. "You can run this command by typing M-f"), shown ~2 seconds
  (`suggest-key-bindings`). Completion list also shows equivalent key
  bindings.
- Redline: `M-x` over the command registry is the palette (issue 01). Adopt:
  prompt = `M-x`, TAB/completion, RET run, C-g cancel.

## C-h k / C-h f — self-documentation (node: Key-Help.html)

- `C-h c <key>` (`describe-key-briefly`): echo-area one-liner with the
  command name bound to the key (e.g. `C-h c C-f` → `forward-char`).
- `C-h k <key>` (`describe-key`): opens a help buffer containing the command's
  *documentation string*.
- `C-h K <key>`: shows the manual section describing the command.
- `C-h w <command> RET` (`where-is`): lists keys bound to a command, in the
  echo area; "not on any key" ⇒ must use M-x.
- All of these work for any key sequence including prefixes.
- (Redline maps this to a describe-command Picker; the manual's model is
  doc-string lookup keyed by command, with key-sequence lookup as the entry
  point.)

# Minibuffer & echo area

## Prompt format (node: Basic-Minibuffer.html)

- The echo area is the bottom line of the frame; the mode line sits right
  above it (node: Mode-Line.html).
- When active, the minibuffer appears IN the echo area, with a cursor.
- "The minibuffer starts with a *prompt*, usually ending with a colon. The
  prompt states what kind of input is expected." Prompt uses the
  `minibuffer-prompt` face (highlighted).
- RET submits and exits; C-g cancels the requesting command and exits.
- *Default argument*: "Sometimes, the prompt shows a *default argument*,
  inside parentheses before the colon. This default will be used as the
  argument if you just type RET." (e.g. buffer-name prompts show the current
  buffer name as default.) Customizable display via
  `minibuffer-default-prompt-format`.
- While the minibuffer is active: (a) "Emacs does not echo keystrokes";
  (b) any error/informational message is shown **in brackets after the
  minibuffer text** for a few seconds or until you type something. This is
  the echo-area-message vs active-prompt coexistence rule — redline's
  minibuffer + message area must reproduce it.
- The specific prompt string for `C-x C-f` (e.g. "Find file: ") is **NOT
  VERIFIED** — the Basic-Files node documents `C-x C-f test.emacs RET`
  using the minibuffer but does not print the prompt string.

## Minibuffer history (node: Minibuffer-History.html)

- Everything typed is saved in a per-kind *history list* (separate lists for
  file names, buffer names, command names, etc.).
- `M-p` (`previous-history-element`): replaces minibuffer contents with the
  earlier history item. `M-n` (`next-history-element`): moves forward.
- `M-n` when there are no later entries fetches from "future history" —
  values you're likely to enter (e.g. file name/URL at point for file
  prompts, via `file-name-at-point-functions`/ffap).
- `M-r <regexp> RET` / `M-s <regexp> RET`: jump to earlier/later history item
  matching a regexp (reads the regexp recursively in the minibuffer).
- `UP`/`DOWN` like M-p/M-n but walk multi-line items line by line first.
- Editing a fetched history item does not change the stored entry; the
  submitted (edited) text is appended to the list.
- `history-length` caps list length; `history-delete-duplicates` (default
  nil).
- `goto-line` has its own history list (node: Moving-Point.html).
- Redline: history per prompt-kind with M-p/M-n is worth adopting verbatim.

## Completion model (node: Completion.html) — ancestor of helm, for contrast

- "When completion is available, certain keys (usually TAB, RET, and SPC) are
  rebound in the minibuffer to special completion commands." They complete
  the text based on *completion alternatives* supplied by the requesting
  command.
- Typing `?` pops up a buffer named `*Completions*` displaying the matching
  alternatives; you navigate and choose from it. (Sub-nodes Completion-Commands
  / Completion-Exit / Completion-Styles / Completion-Options were NOT fetched
  — details like exact TAB behavior, partial-select, and style selection are
  **NOT VERIFIED**.)
- Verified special case (node: Select-Buffer.html): `C-x b` uses *permissive
  completion with confirmation* — RET on a name that doesn't exist prints
  `[Confirm]` and a second RET submits it.
- Contrast: redline's Picker (nucleo fuzzy, always-visible candidate list +
  preview) is a *descendant* of this model, not a reproduction of it. Emacs
  completion is prefix-based and lazy (candidates appear on demand via ?);
  redline's Picker is fuzzy and eager. Document this as a deliberate
  deviation in help.

# Motion & scrolling

All from node: Moving-Point.html unless noted.

| Key | Command | Verified behavior |
|---|---|---|
| `C-f` | `forward-char` | Move right one character |
| `C-b` | `backward-char` | Move left one character |
| `C-n` / DOWN | `next-line` | Down one **screen** line, preserving horizontal position |
| `C-p` / UP | `previous-line` | Up one screen line, preserving position |
| `C-a` / Home | `move-beginning-of-line` | Beginning of the **logical** line |
| `C-e` / End | `move-end-of-line` | End of the logical line |
| `M-f` | `forward-word` | Forward one word |
| `M-b` | `backward-word` | Backward one word |
| `M-<` | `beginning-of-buffer` | Top of buffer; with numeric arg n → n/10 of the way from top |
| `M->` | `end-of-buffer` | End of buffer |
| `M-r` | `move-to-window-line-top-bottom` | Cycles point to top/center/bottom screen line |
| `C-x C-n` | `set-goal-column` | Semipermanent goal column for C-n/C-p |

- Screen-line vs logical-line distinction: C-n/C-p move by *screen* (visual)
  lines by default (`line-move-visual`); C-a/C-e operate on *logical* lines.
- `C-v` (`scroll-up-command`, also PageDown/`next`): "Scroll the display one
  screen forward" — "take the two lines at the bottom of the window and put
  them at the top, followed by lines that were not previously visible." If
  point was in the text that scrolled off the top, it ends up on the window's
  new topmost line.
- `M-v` (`scroll-down-command`, also PageUp/`prior`): scrolls backward the
  same way.
- Overlap: controlled by `next-screen-context-lines`, **default 2**.
- **Terminology trap** (node: Scrolling.html): "scrolling up or down refers
  to the direction that the text moves in the window, not the direction the
  window moves relative to the text. Hence, the strange result that PageDown
  scrolls up in the Emacs sense." C-v = forward = toward end of buffer.
- Numeric prefix arg n to C-v/M-v scrolls by n lines; "C-v with a negative
  argument is like M-v and vice versa".
- Default at buffer boundaries: "these commands signal an error (by beeping
  or flashing the screen) if no more scrolling is possible"
  (`scroll-error-top-bottom` default nil).
- `M-g g` (`goto-line`): "Read a number n and move point to the beginning of
  line number n. Line 1 is the beginning of the buffer. If point is on or
  just after a number in the buffer, that is the default for n. Just type RET
  in the minibuffer to use it." Numeric prefix arg moves directly without
  prompting. Has its own history list.
- `M-g c`: move to buffer *position* n (not line). `M-g TAB`: column n.
- `C-u M-g M-g` (`goto-line` with plain prefix): "selects the most recently
  selected buffer other than the current buffer in another window" and goes
  to line n there — cross-buffer line jump (node: Select-Buffer.html).
- `C-l` recenter: the manual has a "Recentering" node (16.2) but it was NOT
  fetched — C-l behavior is **NOT VERIFIED** in this session.

# Incremental search

Nodes: Basic-Isearch, Repeat-Isearch, Error-in-Isearch, Lax-Search.

## Start / extend

- `C-s` (`isearch-forward`): starts forward incremental search, reads
  characters from the keyboard, "moves point just past the end of the next
  occurrence of those characters in the buffer."
- `C-r` (`isearch-backward`): reverse; finds matches that *end* before the
  starting point.
- Each typed character extends the search string and re-searches: typing
  F→O→O moves past F, then past the first FO, then the first FOO ("the 'F' in
  that 'FO' might not be the first 'F' previously found").
- Current match highlighted with the `isearch` face; the search string is
  displayed in the echo area.
- `DEL` (`isearch-delete-char`): "cancels the last input item" (not just one
  character — an item can be a yanked word or a whole repeat).
- Note: plain `n`/`N` have NO special meaning inside isearch — they are
  inserted into the search string. Next/previous match is C-s/C-r (see below).
  (Redline issue 03's "n/N navigate" is a deviation from Emacs — see Mapping.)

## Exit

- `RET` (`isearch-exit`): "stops searching, leaving the cursor where the
  search brought it."
- Any command not specially meaningful in search exits the search and then
  executes (e.g. C-a exits and moves to line start; arrow keys exit and move).
- Special exception: RET with an EMPTY search string launches nonincremental
  search.
- On exit, "it adds the original value of point to the mark ring" —
  C-x C-x / C-u C-SPC returns to the pre-search position (if the mark wasn't
  already active).

## Cancel / C-g — verified two-step semantics (node: Error-in-Isearch)

- "If the search has found what you specified and is waiting for input, C-g
  cancels the entire search, moving the cursor back to where you started the
  search."
- "If C-g is typed when there are characters in the search string that have
  not been found … the search string characters which have not been found are
  discarded from the search string." Then the search is successful again, so
  "a second C-g will cancel the entire search."
- So: C-g does NOT unconditionally jump to the start point. It either
  (a) discards the unmatched tail, or (b) aborts back to the start point.
  `ESC ESC ESC` (`isearch-cancel`) and `C-g C-g` (`isearch-abort`) abandon
  and return to the start point (node: Basic-Isearch).

## Repeat / wrap

- Another `C-s` (`isearch-repeat-forward`): move to the next occurrence;
  `C-r` (`isearch-repeat-backward`): previous. Repeatable; numeric prefix n
  = nth occurrence.
- After exiting, `C-s C-s` re-searches the last search string (first C-s
  invokes isearch, second repeats); `C-r C-r` backward. Direction used to
  find the last string doesn't matter.
- `C-r` during a forward search switches direction, string unchanged (and
  vice versa). By default the first command after a direction change "remains
  on the same match" and moves the cursor to the other end of it
  (`isearch-repeat-on-direction-change` default nil).
- Failing + repeat: "it starts again from the beginning of the buffer"
  (backward: from the end). "This is called *wrapping around*, and 'Wrapped'
  appears in the search prompt once this has happened. If you keep on going
  past the original starting point … it changes to 'Overwrapped'."
- `isearch-wrap-pause` default `t`: signal an error (ding) when no more
  matches; repeating wraps. Other values wrap immediately or never.

## Failing search echo

- "If your string is not found at all, the echo area says 'Failing
  I-Search', and the cursor moves past the place where Emacs found as much of
  your string as it could" (FOOT→cursor after the FOO in FOOL). The
  unmatched part of the string is highlighted with the `isearch-fail` face.
  From there: DEL to fix, RET to stay, C-g to drop the unmatched chars.

## Lazy highlight

- "If you pause for a little while during incremental search, Emacs
  highlights all the other possible matches for the search string that are
  present on the screen," using the `lazy-highlight` face (distinct from the
  current match). Disable: `isearch-lazy-highlight` nil.

## Search ring (history inside isearch)

- `M-p` (`isearch-ring-retreat`) / `M-n` (`isearch-ring-advance`): move
  through the search ring (most recent 16 by default, `search-ring-max`).
  The chosen string is placed in the minibuffer where you can edit it;
  C-s/C-r or RET accepts and searches for it.
- `M-e` (`isearch-edit-string`): edit the current search string in the
  minibuffer without touching the ring.
- `C-w` (yank word into search string): the yank facility is verified to
  exist and, by default (`search-upper-case` = `not-yanks`), yanked text is
  down-cased so such searches are case-insensitive (node: Lax-Search) — but
  the Isearch-Yank node itself was NOT fetched, so the exact C-w binding and
  variants are **NOT VERIFIED**.

## Case folding (node: Lax-Search)

- "Searches in Emacs by default ignore the case of the text … if you specify
  the search string in lower case. Thus, if you specify searching for 'foo',
  then 'Foo' and 'fOO' also match."
- "An upper-case letter anywhere in the search string makes the search
  case-sensitive. Thus, searching for 'Foo' does not find 'foo' or 'FOO'."
  Controlled by `search-upper-case` (default `not-yanks`).
- `M-c` / `M-s c` (`isearch-toggle-case-fold`): toggles case sensitivity of
  the current search only, overriding the upper-case-letter effect.
- `case-fold-search` (per-buffer): nil ⇒ all letters must match exactly.
- Lax whitespace (default on): each space (or run of spaces) in the search
  string matches any run of whitespace (spaces/tabs). Toggle: `M-s SPC`
  (`isearch-toggle-lax-whitespace`).

# Buffers

## Current buffer / switching (node: Select-Buffer)

- "Current buffer" = the buffer receiving keyboard input / that commands
  operate on; `C-x b` makes the named buffer current and displays it.
- `C-x b` (`switch-to-buffer`): minibuffer prompt for a buffer name (default
  shown in parens per the minibuffer rules). Empty input: "specifies the
  buffer that was current most recently among those not now displayed in any
  window." Nonexistent name: creates a new empty buffer (Fundamental mode)
  and selects it.
- Completion here is permissive-with-confirmation: RET on a nonexistent
  buffer prints `[Confirm]`; second RET submits (see Minibuffer section).
- `C-x LEFT` / `C-x RIGHT` (`previous-buffer` / `next-buffer`): select the
  previous/next buffer "following the order of most recent selection";
  numeric arg = repeat count.
- Window/frame variants (`C-x 4 b`, `C-x 5 b`) exist in Emacs; redline has a
  single view, so only the core prompt+switch semantics apply.

## Buffer list (node: List-Buffers)

- `C-x C-b` (`list-buffers`): pops up a buffer named `*Buffer List*`; each
  line shows "one buffer's name, size, major mode and visited file".
- Order: "the buffers that were current most recently come first."
- Indicators in the first field: `.` = buffer is current; `%` = read-only;
  `*` = modified.
- `C-u C-x C-b`: list only buffers visiting files.
- Example layout from the manual (columns: flags, name, size, mode, file):

  ```
  CRM Buffer                Size  Mode              File
  . * .emacs                3294  Elisp/l           ~/.emacs
   %  *Help*                  101  Help
      search.c             86055  C                 ~/cvs/emacs/src/search.c
  ```

- The in-list keys (k to kill, RET to switch, etc.) live in the
  "Operating on Several Buffers" node, which was NOT fetched — buffer-list
  keys are **NOT VERIFIED** in this session.

## Kill (node: Kill-Buffer)

- `C-x k <buffer> RET` (`kill-buffer`): kills one buffer; default (bare RET)
  is the current buffer.
- Killing the current buffer: "another buffer becomes current: one that was
  current in the recent past but is not displayed in any window now."
- Killing a *modified* file-visiting buffer requires a `yes` confirmation.
- Killing releases the buffer's memory (the "close" of other editors).
- Bulk: `kill-some-buffers` (ask one by one), `kill-matching-buffers`
  (regexp, confirm each), `kill-matching-buffers-no-ask`.
- Redline mapping: "kill buffer" = close/discard a file view (and drop its
  index entries); the modified-confirmation rule doesn't apply (read-only),
  but the current-buffer-fallback rule does.

# Mode line

Node: Mode-Line.html. "At the bottom of each window is a mode line, which
describes what is going on in the current buffer."

Format (verified): `cs:ch-dfr  buf      pos line   (major minor)`

- `ch`: `--` unmodified; `**` modified; `%*` read-only modified; `%%`
  read-only.
- `buf`: buffer name (usually the file name).
- `pos`: `All` if whole buffer visible; else `Top`, `Bot`, or `nn%` —
  percentage of the buffer above the top of the window.
- `line`: `L` followed by the line number at point.
- `major`: major mode name (may append extra info, e.g. compilation status);
  `minor`: list of enabled minor modes; `Narrow`, `Def` also shown.
- On text terminals the text is "followed by a series of dashes extending to
  the right edge of the window."
- `cs:` prefix describes coding system / end-of-line convention (irrelevant
  to redline).
- Redline status line analog (issue 01): buffer/file name, modified-or-not
  indicator (read-only in v1), major-mode analog (language), position
  indicator (All/Top/Bot/nn%), line number, plus redline's own additions
  (project, pending prefix keys, async-activity indicators). The which-
  function name in the status line (issue 05) corresponds to Emacs's
  Which Function mode (node exists: Which-Function.html, NOT fetched —
  **NOT VERIFIED**).

# Navigation (xref)

Nodes: Xref, Looking-Up-Identifiers, Xref-Commands.

## Model

- "Emacs provides a unified interface to these capabilities, called 'xref'."
  Mode-specific work (what files to search, how to find references, how to
  complete on identifiers) is delegated to a *backend* — exactly redline's
  `Xref` trait (tree-sitter now, LSP later).

## M-. — find definition (node: Looking-Up-Identifiers)

- `M-.` (`xref-find-definitions`): "shows the definition of the identifier at
  point. With a prefix argument, or if there's no identifier at point, it
  prompts for the identifier" — i.e. `C-u M-.` always prompts; bare M-.
  prompts only when point isn't on an identifier.
  (`xref-prompt-for-identifier` = t forces always-prompt.)
- Prompt completion candidates = known identifier names.
- Unique definition: jump there (buffer switch to the file containing it).
- Multiple candidates: "by default pops up the *xref* buffer showing the
  matching candidates and selects that buffer's window. Each candidate is
  normally shown … as the name of a file and the matching identifier(s) in
  that file." Default `xref-auto-jump-to-first-definition` = nil: nothing is
  preselected or shown until you pick a candidate.
- Related: `C-M-.` (`xref-find-apropos`) — regexp over identifier names,
  always pops *xref*. `C-x 4 .` / `C-x 5 .` — other-window/frame variants
  (no analog in redline's single view).

## M-, / jump stack

- `M-,` (`xref-go-back`): "Go back to where you previously invoked M-. and
  friends." "It jumps back to the point of the last invocation of M-." The
  stack therefore records the **origin positions from which lookups were
  invoked**, not the landing positions.
- "M-, allows you to retrace the steps you made forward in the history of
  places, all the way to the first place in history … or to any
  place in-between."
- `C-M-,` (`xref-go-forward`): go forward again — "retrace all the steps you
  made back … all the way to the last place in history."
  (Redline issue 05 plans `C-i` for forward — deviation, see Mapping.)
- The manual does not spell out the internal push mechanism (which exact
  positions are pushed on each jump — e.g. whether M-. from an *xref*
  candidate also pushes) beyond the above; internal details
  (`xref-push-marker-stack`) are **NOT VERIFIED** in this session.

## *xref* buffer keys (node: Xref-Commands)

| Key | Command | Behavior |
|---|---|---|
| RET / mouse-1 | `xref-goto-xref` | Display the reference on the current line; with prefix arg, also bury the *xref* buffer |
| n / . | `xref-next-line` | Next reference, displayed in the other window |
| N | `xref-next-group` | First reference of next group |
| p / , | `xref-prev-line` | Previous reference |
| P | `xref-prev-group` | First reference of previous group |
| C-o | `xref-show-location-at-point` | Show in other window (non-selecting) |
| g | `revert-buffer` | Refresh |
| M-, | `xref-quit-and-pop-marker-stack` | Quit *xref* window AND jump to previous stack location |
| q | `xref-quit` | Quit *xref* window only |

- "In addition, the usual navigation commands, such as the arrow keys, C-n,
  and C-p are available for moving around the buffer without displaying the
  references."
- After leaving the *xref* window, `M-g M-n` / `M-g M-p` (`next-error` /
  `previous-error`) move between candidates.
- Redline: the *xref* buffer becomes a Picker overlay; adopt RET = jump,
  n/p = navigate, q = dismiss, and the M-, = dismiss-and-pop combo.

# Imenu

Node: Imenu.html.

- Invocation: `M-g i` (`imenu`) — "reads the name of a definition using the
  minibuffer, then moves point to that definition. You can use completion to
  specify the name; the command displays the list of matching valid names in
  the completions buffer." (Redline's plan uses `M-i` — deviation.)
- Presentation: completion-driven (minibuffer + *Completions*), NOT a
  dedicated menu buffer by default. "If the index is hierarchical, then by
  default the completion candidates are also shown hierarchically, as a
  nested list: first you need to choose a section, then a subsection, etc.,
  and finally the name of the definition."
- `imenu-flatten` (non-nil) selects a flat list instead, with one of three
  styles: `prefix` (section names prefixed, levels joined by
  `imenu-level-separator`, default `:`), `annotation` (section names after
  the definition name), `group` (candidates grouped by section).
- Mouse: if `imenu` is bound to a mouse click it shows nested mouse menus;
  `imenu-add-menubar-index` adds the index to the menu bar.
- `*Rescan*` item rebuilds the index after edits; automatic with
  `imenu-auto-rescan` (disabled above `imenu-auto-rescan-maxout` bytes;
  aborted after `imenu-max-index-time` seconds).
- Default order: "names are ordered as they occur in the buffer"
  (`imenu-sort-function`).
- Redline: `M-i` → nested outline Picker over the tree-sitter symbol index;
  `imenu-flatten`-style flattened list with `:`-joined section prefixes is
  the natural Picker representation.

# Occur

Node: Other-Repeating-Search.html.

- `M-s o` (`occur`): "Prompt for a regexp, and display a list showing each
  line in the buffer that contains a match for it." Matched text highlighted
  with the `match` face.
- At the prompt, `M-n` reuses search strings from previous incremental
  searches.
- Numeric argument n: display n lines of context before/after each match
  (default from `list-matching-lines-default-context-lines`).
- Can be run while an incremental search is active — uses the current search
  string.
- Scope: "the text from point to the end of the buffer, or on the region if
  it is active" (whole-buffer for multi-occur variants).
- Case: "They all ignore case in matching, if the pattern contains no
  upper-case letters and case-fold-search is non-nil" (same folding rule as
  isearch).
- The result buffer is `*Occur*` (major mode: Occur mode). Format: matching
  lines listed (the manual does not document per-line counts in the single-
  buffer case; `M-x how-many` prints a match count instead).

## *Occur* buffer keys (all verified)

| Key | Behavior |
|---|---|
| n | Next match (prefix arg = that many) |
| p | Previous match |
| digit keys | Bound to `digit-argument`, so `5 n` moves 5 matches (no C-u needed) |
| SPC / DEL | Scroll the *Occur* buffer down / up |
| RET (or click a match) | "visits the corresponding position in the original buffer that was searched" |
| o / C-o | Display the match in another window; C-o does not select that window |
| M-g M-n (`next-error`) | Visit occurrences one by one |
| q | "quits the window showing the *Occur* buffer and buries the buffer" |
| e | Occur Edit mode (edit matches in place; C-c C-c to leave) — N/A for read-only redline |

- `multi-occur` / `multi-occur-in-matching-buffers`: same idea over several
  buffers (specified by name, or by regexp over visited file names / buffer
  names). Redline's cross-file grep results (issue 06) map to this shape:
  grouped per-file, n/p across matches, RET opens the file at the match.

# Key quick-reference

Consolidated table: key → command → verified behavior → redline issue.

| Key | Command | Verified behavior | Redline issue |
|---|---|---|---|
| C-g | keyboard-quit / isearch-abort | Context cancel: prefix, running cmd, minibuffer, 2-step isearch | 01 |
| C-u … | universal-argument | 4 / 16 / digit / -1 prefix args | 01 |
| M-x | execute-extended-command | Prompt "M-x", completion, C-g cancel, RET run | 01 |
| C-h k | describe-key | Help buffer with doc string | 01 (describe-command Picker) |
| C-h c | describe-key-briefly | Echo-area one-liner | 01 |
| C-h w | where-is | List bindings of a command | 01 |
| M-p / M-n | previous/next-history-element | Minibuffer history | 01 |
| C-x C-f | find-file | Minibuffer file prompt → open buffer | 02 |
| C-x b | switch-to-buffer | Prompt w/ default; empty = most-recently-current; creates new buffer | 02 |
| C-x C-b | list-buffers | *Buffer List*: name/size/mode/file, recency order, . % * flags | 02 |
| C-x k | kill-buffer | Kill buffer; default = current; confirm if modified | 02 (close view) |
| C-f / C-b | forward/backward-char | Char motion | 03 |
| C-n / C-p | next/previous-line | Screen-line motion, keep column | 03 |
| C-a / C-e | move-beginning/end-of-line | Logical line | 03 |
| M-f / M-b | forward/backward-word | Word motion | 03 |
| M-< / M-> | beginning/end-of-buffer | Top (arg n: n/10) / end | 03 |
| C-v / M-v | scroll-up/down-command | Page forward/backward, 2-line overlap, error at bounds | 03 |
| M-g g | goto-line | Prompt line number (default: number at point); own history | 03 |
| C-s / C-r | isearch-forward/backward | Incremental search start | 03 |
| C-s (again) | isearch-repeat-forward | Next match; C-r previous; wrap → "Wrapped"/"Overwrapped" | 03 |
| DEL (in isearch) | isearch-delete-char | Cancel last input item | 03 |
| C-g / C-g C-g (in isearch) | isearch-abort | Discard unmatched tail, then abort to start point | 03 |
| ESC ESC ESC (in isearch) | isearch-cancel | Abandon, return to start point | 03 |
| M-p / M-n (in isearch) | isearch-ring-retreat/advance | Search ring, 16 entries | 03 |
| M-c (in isearch) | isearch-toggle-case-fold | Toggle case sensitivity | 03 |
| M-g i | imenu | Minibuffer completion over (hierarchical) definitions | 05 (as M-i) |
| M-. | xref-find-definitions | Def at point; C-u M-. prompts; multi → *xref* buffer | 05 |
| M-, | xref-go-back | Pop jump stack (origin positions) | 05 |
| C-M-, | xref-go-forward | Forward in jump stack | 05 (as C-i) |
| RET / n / p / q (in *xref*) | xref-goto-xref / next / prev / quit | Jump, navigate, dismiss | 05 |
| M-s o | occur | Regexp prompt → *Occur* buffer of matching lines | 06 |
| n / p / RET / q (in *Occur*) | — | Next/prev match, visit original position, quit+bury | 06 |

# Mapping to redline

## Adopt verbatim

- **C-g model**: one cancel key with context-dependent meaning — command
  level (discard pending prefix/sequence), minibuffer (cancel requesting
  command), isearch (two-step: discard unmatched tail, then abort to start
  point), picker (dismiss, no action). Plus ESC ESC ESC as an alternate
  "get out" (issue 01).
- **Isearch semantics**: C-s/C-r start/extend, point just past match end,
  C-s/C-r repeat, DEL cancels last input item, RET exits at match,
  Failing-I-Search echo with isearch-fail highlight, wrap with Wrapped/
  Overwrapped prompt states, lazy-highlight of on-screen matches after a
  pause, search ring on M-p/M-n (16 entries), case folding (lowercase query
  → case-insensitive; any uppercase → case-sensitive; M-c toggle), lax
  whitespace matching by default (M-s SPC toggle).
- **M-./M-, jump stack**: origin positions pushed; M-, pops, forward exists
  (C-M-, in Emacs). Every navigation records an entry (issue 05).
- **Echo/minibuffer split**: passive messages vs active prompt with
  colon-terminated prompt and parenthesized default that RET accepts;
  messages during an active prompt render in brackets after the prompt text;
  no keystroke echoing while the minibuffer is active.
- **Motion keys**: C-f/C-b/C-n/C-p/C-a/C-e/M-f/M-b/M-</M-> with the
  screen-line-vs-logical-line distinction; C-v/M-v paging with 2-line overlap
  and error-at-buffer-boundary (or equivalent feedback).
- **M-x**: prompt `M-x`, completion over the registry, C-g cancel, RET run,
  prefix arg before M-x shown in the prompt.
- **Minibuffer history**: per-kind lists, M-p/M-n.
- **Mode line content**: name, state indicator, mode, position (All/Top/Bot/
  nn%), line number.
- **Occur/xref result navigation**: n/p, RET to jump, q to dismiss; digit
  keys as implicit prefix arguments in result buffers.

## Simplify (document as deliberate)

- **Single view, no windows/frames**: drop `C-x 4 b`, `C-x 5 .`, "other
  window" display variants, `o`/`C-o` in *Occur*. The `*xref*`, `*Occur*`,
  and `*Buffer List*` buffers become Picker overlays in the same view.
  "Bury/quit window" (q) becomes "dismiss overlay, keep buffer state".
- **Read-only**: no kill ring, no undo (C-/ C-x u out of scope), no buffer
  modification, no Occur Edit mode, no save. `C-x k` becomes "close/discard
  view"; the modified-confirmation rule is dropped; the
  current-buffer-fallback rule (kill current → most-recently-current other
  buffer) is kept.
- **Completion → Picker**: Emacs's lazy prefix completion (TAB extends, ?
  opens *Completions*) is replaced by the eager fuzzy Picker (nucleo) with
  always-visible candidates and preview. Keep the *names* of the interactions
  (TAB/RET/C-g) but the candidate model differs. Permissive-completion
  `[Confirm]` second-RET has no analog (no buffer creation in v1).
- **Buffers**: "buffer" ≈ open file view + special views. Buffer-list
  recency ordering and the empty-input = most-recently-current rule carry
  over to the file/buffer picker.

## Deviations to document in redline help

- `C-i` forward-jump (issue 05) vs Emacs `C-M-,` (`xref-go-forward`).
- `M-i` imenu (issue 05) vs Emacs `M-g i`.
- Issue 03's "n/N navigate" in isearch: in Emacs, n/N are ordinary search
  characters; next/previous match is C-s/C-r. If redline binds n/N in
  isearch, that is a redline extension (harmless only because Emacs users
  rarely type n/N mid-search, but it changes what "type n" does).
- `C-x C-c` as redline's quit key: the Exiting node was NOT fetched; Emacs's
  `C-x C-c` behavior is **NOT VERIFIED** in this session.
- Match count in the isearch prompt (issue 03): the manual only documents
  the search string in the echo area; a match count display is a redline
  addition (**NOT VERIFIED** as Emacs behavior).
- `C-l` recenter: **NOT VERIFIED** (Recentering node not fetched).

# Sources

Fetched 2026-09-16 from the GNU Emacs Manual (Emacs 31.1.50):

- https://www.gnu.org/software/emacs/manual/html_node/emacs/ (index / TOC, manual version)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/User-Input.html (2 Kinds of User Input)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Commands.html (5 Keys and Commands)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Quitting.html (52 Quitting and Aborting)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Arguments.html (9.10 Numeric Arguments)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/M_002dx.html (11 Running Commands by Name)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Key-Help.html (12.2 Documentation for a Key)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Basic-Minibuffer.html (10.1 Using the Minibuffer)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Minibuffer-History.html (10.5 Minibuffer History)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Completion.html (10.4 Completion)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Moving-Point.html (9.2 Changing the Location of Point)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Scrolling.html (16.1 Scrolling)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Basic-Isearch.html (17.1.1 Basics of Incremental Search)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Repeat-Isearch.html (17.1.2 Repeating Incremental Search)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Error-in-Isearch.html (17.1.4 Errors in Incremental Search)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Lax-Search.html (17.9 Lax Matching During Searching)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Select-Buffer.html (21.1 Creating and Selecting Buffers)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/List-Buffers.html (21.2 Listing Existing Buffers)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Kill-Buffer.html (21.4 Killing Buffers)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Mode-Line.html (1.3 The Mode Line)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Imenu.html (28.2.4 Imenu)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Other-Repeating-Search.html (17.11 Other Search-and-Loop Commands)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Xref.html (30.4 Find Identifier References)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Looking-Up-Identifiers.html (30.4.1.1 Looking Up Identifiers)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Xref-Commands.html (30.4.1.2 Commands Available in the *xref* Buffer)
- https://www.gnu.org/software/emacs/manual/html_node/emacs/Basic-Files.html (9.5 Files)

Not fetched (behaviors relying on them are marked NOT VERIFIED in the body):
Recentering, Isearch-Yank, Isearch-Minibuffer, Not-Exiting-Isearch,
Completion-Commands/Exit/Styles/Options, Minibuffer-File, Minibuffer-Edit,
Exiting, Several-Buffers (buffer-list keys), Which-Function,
Nonincremental-Search, Echo-Area. (Attempted
elisp/Cross-References.html → 404; xref semantics were taken from the Emacs
manual 30.4.x nodes instead.)
