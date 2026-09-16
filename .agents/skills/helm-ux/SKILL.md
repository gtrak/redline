---
name: helm-ux
description: Authoritative reference for how helm (the Emacs fuzzy-completion framework, wiki docs for helm >= 3.6.2) actually behaves — the completion-window session model, multi-pattern narrowing, marking, actions, persistent action (C-j/C-z), follow-mode live preview, and the verified behavior of helm-M-x / helm-find-files / helm-mini / helm-buffers-list / helm-grep — so a Rust implementer can faithfully build redline's helm-style Picker (plan decision #3, issues 01/02/05/06) without relying on training-data guesses. Every key and behavior is cited to a fetched wiki page; anything not present in the fetched pages is explicitly marked NOT VERIFIED.
---

# helm-ux

In-repo reference for redline's Picker. Sourced from the official helm GitHub
wiki (see Sources). Rules: keys and behaviors below are as documented on the
fetched pages. Items the fetched pages do not document are marked
**NOT VERIFIED** — do not "fill them in" from memory when implementing.

## Manual version

- Source: the helm project wiki at `https://github.com/emacs-helm/helm/wiki`
  (Home + 11 targeted pages, fetched 2026-09). The official manual at
  `emacs-helm.github.io/helm` was **not** fetched (single giant page; the wiki
  pages carry the needed detail).
- Version: the wiki Home page states "Helm require an emacs version >= 25.1
  starting from version 3.6.2" — the docs therefore describe helm ≥ 3.6.2. No
  exact release number is stated on the fetched pages.
- The wiki is a collection of short pages (26 total), not one manual; some
  pages are stale (several last edited 2016–2018), so treat specific
  keybindings on old pages with the page's edit date in mind.

## Session model

- A helm session = a **completion window** (a buffer) that displays candidate
  lists, plus the minibuffer for typing. "Emacs completion is based on the
  minibuffer. Helm completion is based on the completion window." Typing in
  the minibuffer filters candidates in the completion window; `RET` selects
  the currently **highlighted** item in the completion window (not text
  completion of the minibuffer).
- Multiple lists can be shown in one session: "Helm is able to complete
  multiple lists dispatched in different sources against a pattern." Sources
  are stacked in the one buffer (helm-mini stacks buffers + recently visited
  files — see Core sources).
- Layout: "Helm can display its completion buffer in different window
  layouts and in separate frame." Verified knobs:
  - `helm-full-frame` (local variable set via `helm-set-local-variable` in the
    wiki's `hcd` script) — display in a full frame.
  - `helm-display-function 'helm-display-buffer-in-own-frame` — own frame.
  - `helm-display-buffer-width` / `helm-display-buffer-height` — frame size.
  - `helm-autoresize-mode` — resize the completion window based on the number
    of candidates; `helm-autoresize-max-height` defaults to 40 (percent of
    frame height), `helm-autoresize-min-height` is the floor; set both equal
    for a fixed size.
- Exit vs resume: each session creates a buffer that **stays around** "for
  further use when you want to resume previous session"; do not kill those
  buffers if you want to resume. `helm-resume` (default `<helm-prefix> b`)
  resumes the previous session; with a universal argument (`C-u`) you can
  **choose between different Helm sessions** — verified: yes, you can return
  to a previous session. `C-x C-b` "switches back to the resumed Helm
  sources" (wiki Home, file-workflow section).
- What the last filter does on resume: **NOT VERIFIED** on the fetched pages.
- The mode line acts as a persistent help line: the general bindings "are
  also documented in the mode line", and "a tip to hit `C-h m` within a Helm
  session can be found on the mode line".
- In-session help: `C-h m` "displays the embeded help in an org buffer
  without quitting helm session"; `helm-documentation` generates the full
  grouped help. `C-h c` customizes variables specific to the current session.

## Narrowing

- **Multiple patterns, space-separated, ANDed**: "Patterns can be combined.
  For example, for buffers in `emacs-lisp-mode`, match 'helm', and end in
  'foo', the pattern is: `*lisp helm foo`" (Buffers page). Grep likewise
  supports "multi matches (i.e add a space between each pattern)" (Grep
  page). Verified.
- **Comma = OR within one pattern**: `*lisp,sh` matches `emacs-lisp-mode`
  **or** `sh-mode` (Buffers page). Verified.
- **Negation with `!`**: `*!lisp,!info` = "Name excludes 'lisp', 'info'"
  (Buffers page). Verified.
- **Anchors**: `^helm` = "Name starts with 'helm'" (Buffers page). Verified.
- **Fuzzy matching is DISABLED by default** (Fuzzy-matching page) — contrary
  to the common assumption that helm is fuzzy everywhere. Per-command
  switches: `helm-M-x-fuzzy-match`, `helm-buffers-fuzzy-matching`,
  `helm-recentf-fuzzy-match`, `helm-locate-fuzzy-match`,
  `helm-semantic-fuzzy-match`, `helm-imenu-fuzzy-match`,
  `helm-apropos-fuzzy-match`, `helm-etags-select` via
  `helm-etags-fuzzy-match`, `helm-session-fuzzy-match`. Global for helm-mode:
  `helm-mode-fuzzy-match`, `helm-completion-in-region-fuzzy-match`.
  Exception: **`helm-find-files` has fuzzy matching enabled by default.**
- **Candidate cap**: `helm-candidate-number-limit`, default 100
  ("For faster fuzzy matching, set … to 100 or less. Default is 100").
- **Matching order / scoring**: the scoring algorithm is **NOT VERIFIED** on
  the fetched pages. `helm-M-x` has a listed feature "Smart sorting" (Commands
  page) but the ranking rule itself is not documented there.
- **Wildcards in file finding**: `*.el` selects `.el` files in the current
  directory; `**.el` selects recursively — "activated by default with the
  option `helm-file-globstar`" (Find-Files page).
- **Async / delayed sources**: `helm-grep` is "Incremental" and "Faster than
  Emacs grep"; results update while you type. `C-!` **suspends/restarts Helm
  updates** while you write a regexp (documented for TRAMP use; general
  mechanism) (Grep page). Source classes include `helm-source-async` (FAQ
  page lists `helm-source-sync`, `helm-source-async`, `helm-source-in-buffer`,
  `helm-source-dummy`). This is the model redline's rg-stream/file-walk
  sources must mirror: candidates arrive in the background and the list
  refreshes as they stream in.
- **Minibuffer editing**: `C-k` = `helm-delete-minibuffer-contents` (clears
  all pattern text; with `helm-delete-minibuffer-contents-from-point` it
  deletes from point to end) (Minibuffer page).
- **Yanking into the pattern**: `M-n` copies the symbol at point into the
  minibuffer; `C-w` appends the word next to point and advances to the next
  word; `C-_` undoes the last insertion (Home page).
- `helm-ff-auto-update-initial-value` appears in the wiki's `hcd` script (Home
  page); only the variable name is verified, its exact behavior is not
  described on the fetched pages.

## Navigation & selection

- `C-n` move down, `C-p` move up (Find-Files "Navigation" table). Verified.
- `C-l` = "Up directory" in `helm-find-files` (Find-Files page). Verified.
- `M->` / `M-<` (first/last candidate): **NOT VERIFIED** on the fetched pages.
- Wrap-around at the list ends: **NOT VERIFIED** on the fetched pages.
- **Marking**: `M-SPC` or `C-SPC` or `C-@` "marks a candidate" (Home page).
  Verified. `M-a` "marks all files in a directory" (Find-Files page) —
  verified for find-files; a general "select all candidates" meaning for
  `M-a` is **NOT VERIFIED** on the fetched pages.
- **Actions on marked candidates**: "Helm allows marking candidates to
  execute chosen action against this set of candidates" (Home). Concrete
  verified examples: grep on marked files ("To grep marked files, just mark
  some files with `C-<space>` and launch grep. Marked files can be from
  different directories." — Grep page); Copy/Rename/Symlink/Hardlink actions
  take marked files as their set (Find-Files page).
- Jumping N lines: with `helm-linum-relative-mode`, `C-x <n>` jumps n lines
  before and `C-c <n>` jumps n lines after the current candidate (Home page).

## Actions

- `RET` = "runs the first action of action list" (Home page). Verified.
- `TAB` (or `C-i`) = "lists available actions" (Home page). "Helm's
  interactivity makes the `<tab>` key redundant for completion … tab
  completion is not supported. In Helm, `<tab>` is used to view available
  actions to be taken on a candidate" (Home page).
- **Persistent action**: `C-j` **or** `C-z` "invokes the persistent action"
  (Home page). Note: the persistent action is context-dependent. Verified
  per-context values:
  - `helm-find-files`: File → "Shows file name in the Helm buffer" (i.e. show
    the file's contents inside the helm window — the live preview);
    Directory → "Steps into the directory"; Symlink → "Expand to symlink's
    true name" (Find-Files page).
  - `helm-M-x`: "Show documentation with persistent action (`C-z`)"
    (Commands page).
  - `helm-grep`: "will bring up the buffer corresponding to the file being
    grepped"; `C-u C-z` "will record the location in the mark ring" (Grep
    page).
  - `C-z` is therefore **not** a "brief/summary display" key — it *is* the
    persistent action, same as `C-j`. (Verified: both keys documented as
    invoking the persistent action.)
- `C-e`: **NOT VERIFIED** on the fetched pages.
- `C-RET` / `M-RET`: exit a `completing-read` with an empty string (FAQ page;
  "This can be seen in the mode line").
- `C-c C-y`: "To put the command in the minibuffer, hit `C-c C-y` on the
  highlighted command" in `helm-M-x` (Commands page).
- `C-x C-s` in a grep session: save grep results in a `helm-grep-mode` buffer
  (Grep page).
- Prefix arguments in `helm-M-x`: pass them **after** invoking `helm-M-x` —
  type `C-u` or `M-9` on the highlighted command; "you will see a prefix arg
  counter appearing in mode-line". Prefix args typed *before* `helm-M-x` are
  shown in the prompt and the first `C-u` inside cancels them (Commands page).
- `helm-find-files` extras (Home page, file workflow): `C-x C-d`
  (`helm-browse-project`) shows buffers + files in the project; `C-c C-d`
  with prefix shows files recursively in this directory; `M-p` = history of
  `helm-find-files`; `C-c h` = full file-name history.
- Window control inside a session: `C-t` split windows vertically, `C-}` /
  `C-{` shrink/enlarge the Helm window (Find-Files image-browsing section).

## Follow & preview

- **`C-c C-f` toggles `helm-follow-mode`** — verified in two places:
  "Turn on `follow-mode` with `C-c C-f`" (Find-Files page) and FAQ: "when you
  hit `C-c C-f` in any source helm-follow-mode will be turned on now and for
  next emacs sessions until you hit again `C-c C-f`" (with
  `helm-follow-mode-persistent t`).
- Effect: with follow enabled, moving the selection live-displays the
  candidate. Verified concrete example (Find-Files, image browsing): "Turn on
  `follow-mode` with `C-c C-f`. Now, you can navigate the image directory
  with the `<up>` and `<down>` arrow keys, or `C-n` and `C-p`" — i.e. each
  selection move renders the selected item. The wiki does not use the exact
  phrase "auto-execute persistent action on each selection move" on the
  fetched pages; that wording is an **inference** from the verified behavior
  above plus the source `:follow` slot mentioned in the FAQ
  (`(setf (slot-value source 'follow) 1)`).
- A source can be declared follow-oriented at construction
  (`helm-make-source … :follow 1`, FAQ page) — the analog of redline marking
  a source "previewable".
- Redline's preview pane is exactly this: selection move ⇒ re-render preview
  of the selected candidate (file first page, symbol definition context, etc).

## Core sources & commands

Only as documented on the fetched pages.

- **`helm-M-x`** (Commands page): "is used to launch commands in Helm. For
  convenience, you should bind it to `M-x`." Default binding
  `<helm-prefix> M-x`. Verified features: **Smart sorting**; **shows key
  bindings next to command names**; **shows documentation with persistent
  action (`C-z`)**; **shows prefix arguments in the mode line**. Fuzzy
  matching off by default (`helm-M-x-fuzzy-match`). `C-c C-y` copies the
  command name into the minibuffer.
- **`helm-find-files`** (Find-Files page + Home): "Helm's version of
  `find-files`; it allows easy navigation of file hierarchies." Default
  `<helm-prefix> C-x C-f`; recommended global `C-x C-f`.
  - Starts at `default-directory` or `thing-at-point` (via `ffap`).
  - `C-u` at start: "displays a history of previously visited directories".
  - Fuzzy matching on by default (Fuzzy-matching page).
  - **Create**: "Navigate to the directory where you want to create the new
    file, then type the file's name and hit `RET`. If the name ends in a
    slash (`/`), Helm prompts you to create a directory (possibly with parent
    directories)."
  - **RET on an existing file vs directory**: **NOT VERIFIED** verbatim on
    the fetched pages (the persistent-action table covers `C-z`/`C-j` only:
    file → show in helm buffer, directory → step in). Inferred, not documented:
    RET opens the file / enters the directory.
  - **Type-ahead traversal** (type a path fragment and the list follows you
    down the tree): **NOT VERIFIED** on the fetched pages. What is verified:
    `C-j`/`C-z` on a directory steps into it, `C-l` goes up, and typing a
    new name + `RET` creates.
  - Wildcards `*.el` / `**.el` select file sets for actions (see Narrowing).
- **`helm-buffers-list`** (Buffers page): "Helm's version of
  `switch-to-buffer` or `list-buffers`. By default, it is bound to
  `<helm-prefix> C-x C-b`." Buffer faces: modified = orange, externally
  modified = red; `helm-boring-buffer-regexp-list` hides boring buffers.
  Pattern language on buffer names/major modes: `*lisp` (major mode match),
  `^helm` (name prefix), `*!lisp,!info` (negation), comma = OR, spaces = AND.
- **`helm-mini`** (Buffers page): "displays buffers and recently visited
  files. It may be more useful than `helm-buffers-list`. Tip: Use `helm-mini`
  instead of `helm-buffers-list`." — i.e. the canonical **stacked-sources**
  session: buffers section + recents section, one pattern over both. Fuzzy
  for it = `helm-buffers-fuzzy-matching` + `helm-recentf-fuzzy-match`.
- **`helm-occur`**: **NOT VERIFIED** on the fetched pages. The wiki has no
  fetched page describing it; the FAQ references the `helm-source-multi-occur`
  source class (examples named "Occur" / "Moccur" built with
  `helm-make-source … :follow 1`). Live-grep-of-current-buffer behavior and
  jump semantics are not documented on the fetched pages — do not assume.
- **`helm-grep`** (Grep page): features: **Incremental** (results update while
  typing), **Recursive**, **Supports wildcards**, **Allow multi matching**
  (space-separated patterns), **Respects `grep-find-ignored-files` /
  `grep-find-ignored-directories`**, **Faster than Emacs `grep`**.
  Launch paths: from `helm-find-files` via `TAB` → `grep` (prefix arg =
  recursive), `(C-u) M-g s` directly, or "start `helm-find-files`, choose
  your files and hit `C-s`" (FAQ). Persistent action `C-z` opens the file
  being grepped; `C-u C-z` pushes the location onto the mark ring; `C-x C-s`
  saves results into a `helm-grep-mode` buffer; `C-!` suspends/resumes
  updates. Backends: grep, ack-grep, ag, pt, **rg** (`helm-grep-ag-command`
  with a documented ripgrep invocation), git-grep.
- **`helm-regexp`** (Regexp page): Helm's `re-builder` — "highlights matches
  in the buffer and provides instant feedback as you write a regexp"; bound
  to `<helm-prefix> r`; matches listed and navigable with standard Helm
  commands; numbered capturing groups shown.

## Quitting

- `C-g` / `ESC ESC` abort semantics: **NOT VERIFIED** on the fetched pages
  (the wiki never documents them in the pages fetched). Redline's `C-g`
  cancel (plan issue 01) is therefore a redline convention, not a verified
  helm behavior.
- Documented "get out" paths instead: execute an action (`RET` / persistent
  action) which leaves the session; `helm-resume` / `C-x C-b` to get back to
  the still-alive session buffer; `C-u C-z` (grep) to stash a location in the
  mark ring before leaving; `C-x C-s` (grep) to save results to a named
  buffer. Session buffers persist after exit for resuming (FAQ), so
  "last filter on resume" is implied by buffer persistence but **NOT
  VERIFIED** verbatim.

## Key quick-reference

All rows verified from the fetched pages unless noted.

| Key | Behavior | Verified source | redline Picker element |
|---|---|---|---|
| type in minibuffer | filter candidates (space = extra ANDed pattern) | Home, Buffers, Grep | prompt + nucleo multi-pattern filter |
| `RET` | run first action of action list | Home | `Picker::execute_default` |
| `TAB` / `C-i` | list available actions (no tab-completion) | Home | `Picker::show_actions` (or simplified) |
| `C-j` / `C-z` | persistent action (context-dependent) | Home, Find-Files, Commands, Grep | `Picker::persistent_action` (preview/descend/doc) |
| `M-SPC` / `C-SPC` / `C-@` | mark a candidate | Home | `Picker::toggle_mark` |
| `M-a` | mark all files in a directory (ff only verified) | Find-Files | `Picker::mark_all` (document deviation) |
| `C-n` / `C-p` | move down / up | Find-Files | `Picker::move` |
| `C-l` | up directory (helm-find-files) | Find-Files | file source: `parent_dir` action |
| `C-k` | delete minibuffer contents | Minibuffer | `Picker::clear_prompt` |
| `M-n` / `C-w` / `C-_` | yank symbol / word into pattern, undo | Home | `Picker::yank_symbol` |
| `C-h m` | in-session help (org buffer, does not quit) | Home, FAQ | `Picker::show_help` |
| `C-c C-y` | put command name in minibuffer (helm-M-x) | Commands | copy-command-name action |
| `C-c C-f` | toggle `helm-follow-mode` (live preview on selection move) | Find-Files, FAQ | `Picker::toggle_preview_follow` |
| `C-!` | suspend / resume candidate updates | Grep | `Picker::toggle_updates` (async sources) |
| `C-RET` / `M-RET` | exit completing-read with empty string | FAQ | accept-empty action |
| `C-x C-b` | resume last Helm session (helm sources) | Home | `Picker::resume` (per-source history) |
| `helm-resume` (`<prefix> b`, `C-u` = choose) | resume previous / pick a session | Resume | same |
| `C-u` (at ff start) | history of previously visited directories | Find-Files | file source recents |
| `M-p` | history of helm-find-files | Home | file source recents |
| `C-x C-d` | `helm-browse-project` (buffers + files in project) | Home | project picker source |
| `C-c C-d` (prefix) | recursive files in this directory | Home | recursive file source |
| `C-s` (in ff) | launch grep on selected files | FAQ | `Picker::grep` action (issue 06) |
| `C-x C-s` | save grep results to `helm-grep-mode` buffer | Grep | `Picker::save_results` |
| `C-t` / `C-}` / `C-{` | vertical split / shrink / enlarge helm window | Find-Files | N/A (no splits in redline v1) |
| `C-g` | quit/cancel | **NOT VERIFIED** | redline convention (issue 01) |
| `M->` / `M-<` | first/last candidate | **NOT VERIFIED** | optional `Picker::jump_end` |

## Mapping to redline

The Picker contract (plan decision #3) implements, from this reference:

- **Prompt + multi-pattern fuzzy narrowing**: helm's pattern language is
  space-separated ANDed patterns with `!` negation and `,` OR. Redline uses
  nucleo-matcher over the whole prompt; to stay faithful, redline should
  split the prompt on spaces into multiple ANDed sub-patterns (verified helm
  semantics) and document that `,`-OR and `!`-negation are not supported
  (or implement `!` as a filter, cheap with nucleo). Note helm's default is
  *non*-fuzzy matching per source with fuzzy opt-in — redline deviates by
  being fuzzy-by-default (like `helm-find-files`, the only verified
  fuzzy-default source). Document in help.
- **Single source vs stacked sources**: `helm-mini` proves the stacked
  model (buffers section + recents section, one shared pattern). Redline's
  file picker should stack **recents + project files** with section headers,
  mirroring `helm-mini`. `helm-browse-project` (`C-x C-d`) is the precedent
  for a buffers+files project source.
- **Selection + preview pane (follow)**: helm's follow-mode
  (`C-c C-f`) makes each selection move render the selected candidate;
  `helm-find-files`'s persistent action on a file *is* "show the file in the
  helm buffer". Redline's persistent preview pane is this behavior, but
  **always on** (no toggle needed in v1; keep `C-c C-f` as a toggle for
  parity — see key conflict below).
- **RET = default action**: verified "runs the first action of action list".
  Per source: file → open; symbol → jump; command → execute; buffer → switch.
- **TAB = action menu**: verified. Redline v1 may simplify (plan issue 01's
  palette needs only RET + C-g) but the Picker component should carry the
  action-menu concept so issue 02's file actions (grep via `C-s`, browse
  project) have a home.
- **C-g quits**: helm's `C-g` is **NOT VERIFIED** in the fetched wiki pages;
  redline's `C-g` cancel (issue 01 verification step) is a redline
  convention. Document it as a deviation in the in-app help.
- **Async sources**: helm's model (helm-grep "Incremental"; `C-!` to suspend
  updates; `helm-source-async` class) is exactly what redline needs for the
  rg stream, file walk, and symbol index build: candidates stream in and the
  list re-sorts without blocking the UI.
- **Resume**: session buffers persist and `helm-resume`/`C-x C-b` return to
  them; per-source recents (`M-p`, `C-u` at ff start, `C-c h`) give the
  "recents survive" behavior issue 02 requires. Redline: keep last prompt +
  last source per picker type in `~/.cache/redline/`.
- **Simplifications (no frame splitting)**: redline is a TUI overlay — no
  own-frame, no `C-t`/`C-}`/`C-{` window control, no autoresize window; the
  "completion window" becomes the overlay's candidate list region and the
  "persistent action / follow" pane becomes the preview pane. `C-RET`
  empty-string exit and mark-ring mechanics have no TUI analog in v1.
- **Key conflict flagged by the magit skill**: magit's commit and log
  buffers bind **`C-c C-f` to move forward in revision history** (magit
  skill, lines 253/343), which collides with helm's `C-c C-f` follow-mode
  toggle. Because redline keymaps are per-view (plan decision #2), resolve by
  scope: inside the **Picker overlay** `C-c C-f` = follow/preview toggle
  (helm), inside **magit-style views** `C-c C-f` = forward history (magit).
  Document both in the in-app help. Related: redline's `M-x` palette is the
  direct analog of `helm-M-x` (fuzzy command picker over the registry with
  key bindings shown next to names and docstring on hover/preview — both
  verified helm-M-x features), and magit's `h` dispatch is documented in the
  magit skill as the "M-x-style Picker equivalent" — all three should share
  the one Picker component.

## Sources

Fetched 2026-09 (12 pages, from `https://github.com/emacs-helm/helm/wiki`):

- https://github.com/emacs-helm/helm/wiki (Home — interaction model, general
  keys, file workflow, prefix key, `helm-full-frame`, version note)
- https://github.com/emacs-helm/helm/wiki/Fuzzy-matching (fuzzy defaults,
  per-command switches, `helm-candidate-number-limit`)
- https://github.com/emacs-helm/helm/wiki/Find-Files (navigation table,
  persistent actions per file type, create, wildcards, `M-a`, follow-mode,
  window keys)
- https://github.com/emacs-helm/helm/wiki/Resume (`helm-resume`, `C-u` pick)
- https://github.com/emacs-helm/helm/wiki/Commands (`helm-M-x` features,
  prefix args, `C-c C-y`)
- https://github.com/emacs-helm/helm/wiki/Grep (incremental/recursive/multi
  match, `C-!`, persistent action, save session, rg backend)
- https://github.com/emacs-helm/helm/wiki/Buffers (`helm-buffers-list`,
  `helm-mini`, pattern language incl. `!` negation and `,` OR, faces)
- https://github.com/emacs-helm/helm/wiki/Minibuffer (`C-k` clear)
- https://github.com/emacs-helm/helm/wiki/Regexp (`helm-regexp`)
- https://github.com/emacs-helm/helm/wiki/FAQ (session buffers persist,
  `C-h m`/`helm-documentation`, `C-RET`/`M-RET` empty exit, `C-c C-f`
  follow persistence, source classes, `C-s` grep entry)
- https://github.com/emacs-helm/helm/wiki/frame (own-frame display, size vars)
- https://github.com/emacs-helm/helm/wiki/helm-autoresize (autoresize mode,
  min/max height)

Not fetched (budget): the `emacs-helm.github.io/helm` manual, and remaining
wiki pages (Developing, Popwin, Projects, Locate, Shell, Eshell, Etags,
Findutils, Info-Files, org-mode, Migemo, Bugs, Firefox Bookmarks, helm-mode,
Shell + 11 hidden pages). Behaviors sourced only from those pages are marked
NOT VERIFIED above.
