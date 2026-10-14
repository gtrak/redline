# Driving redline from outside — the control API

Redline exposes a small, stable control surface so an agent or shell script
can steer a **live** review session (moving the human's screen) or run an
**ephemeral headless** session with no terminal at all.

The whole surface is one object — a `Session` over the existing `AppStore` —
and two transports to it:

- **Attached** — a running TUI binds a Unix control socket; a client connects
  and mutates the very session the human is watching.
- **Headless** — no TUI; the CLI builds a throwaway session, runs the verbs,
  and prints the resulting state.

This document is the frozen contract (schema + verbs). The design plan is
`.agents/plans/019-external-control/PLAN.md`.

## 1. Discovery and the socket

One session = one socket path. Precedence (deterministic):

1. `--socket PATH`
2. `$REDLINE_SOCKET`
3. `$XDG_RUNTIME_DIR/redline-<uid>.sock`
4. `/tmp/redline-<uid>.sock` (the system temp dir)

`<uid>` is `$UID` when set, else the process id. The TUI prints the path it
bound to stderr (`redline: control socket at <path>`) at startup. A stale
socket file (a crashed predecessor) is removed before binding unless a
listener is still answering on it; the file is unlinked again on clean exit.

**Mode selection**

- With no socket requested (no `--socket`, no `$REDLINE_SOCKET`, no
  `--attached`), verbs run **headless**.
- With `--socket` given explicitly and unreachable ⇒ hard error (exit 1).
- With only `$REDLINE_SOCKET` set and unreachable ⇒ **headless fallback**.
- With `--attached` ⇒ **any** unreachable socket is a hard error (exit 1):
  the flag promises a live session, so a silent fallback would lie.

The wire is **NDJSON** over the Unix socket: one JSON `Request` per line, one
JSON `Response` per line.

```
→ {"id":7,"cmd":"open","args":{"arg":"src/main.rs:40"}}
← {"id":7,"ok":true,"result":{}}
← {"id":7,"ok":false,"error":{"code":"unknown-command","message":"..."}}
```

`error.code` is one of `unknown-command`, `bad-key`, `bad-request`, or
`internal`. After every request the server bumps the store's repaint bus, so
the live UI repaints from the already-mutated store.

## 2. The verbs

Every verb is one socket round trip (attached) and one `perform` call
(headless). Primitives are the original 019-01/02 set; sugar verbs (019-03)
are thin compositions of **real store operations**, never a registry
argument replay.

### Primitives

| verb | args | result |
|------|------|--------|
| `ping` | — | `{}` (liveness) |
| `state` | — | the full `Snapshot` (below) |
| `quote` | — | `{"text": <frame>}`, the static render at the session width — the **human's real terminal width** when attached (see §4) |
| `buffer` | `start`,`end` (1-based inclusive; both or neither) | `{"text": <buffer text>}` |
| `notes` | `plain` (bool) | `{"text": <annotation dump>}` |
| `do` | `name`, optional `arg` | `{}` — dispatch a registry command |
| `key` | `token` (e.g. `C-x`, `RET`, `TAB`, `a`) | `{}` |
| `keys` | `seq` (e.g. `"C-x C-f"`) | `{}` |
| `text` | `text` (literal; `\n` = Enter) | `{}` — routed through the key path, so read-only buffers still refuse |
| `quit` | — | `{}` — requests exit; **prints nothing** and is terminal |

### Sugar verbs

| verb | arg | semantics |
|------|-----|-----------|
| `open` | `PATH[:LINE[:COL]]` | `AppStore::open_path_at`; `PATH` project-relative; `LINE`/`COL` 1-based |
| `goto` | `LINE` | land the point on `LINE` (1-based) via the real goto-line prompt |
| `jump` | `SYMBOL` | open the project symbol picker filtered to `SYMBOL` (index-backed; a miss is a no-op, not an error) |
| `search` | `QUERY` | start a project-wide literal search (`start_project_search`) |
| `isearch` | `QUERY` | start an in-buffer incremental search and type `QUERY` |
| `select` | `N` \| `next` \| `prev` | picker: move the selection to 1-based row `N` (clamped), or step |
| `confirm` | — | picker: accept the selected candidate (`run_selected`, RET-equivalent) |
| `cancel` | — | the global cancel (`C-g` equivalent) |
| `annotate` | `TEXT` | annotate at point with `TEXT` (commits the note) |
| `magit` | — | open magit status |

**Grammar rules (frozen):**

- `PATH` in `open` consumes a **trailing** run of one or two `:N` numeric
  segments as line/column. A non-numeric trailing segment belongs to the path
  (`weird:name.rs` is a path).
- A verb that **takes no argument** (`state`, `quote`, `notes`, `cancel`,
  `confirm`, `magit`, `quit`) **rejects** stray trailing tokens with a usage
  error (exit 2 for the CLI; `bad-request` over the wire). A silently ignored
  flag is never accepted.
- A verb that **requires an argument** (`open`, `goto`, `jump`, `search`,
  `isearch`, `annotate`) errors when it is missing.
- `select` is optional (defaults to no movement).

## 3. The JSON `Snapshot`

`state` returns one `Snapshot` object. The top-level key set is **frozen** —
an accidental rename reddens `snapshot_top_level_keys_are_frozen` in
`src/session.rs`:

| key | type | meaning |
|-----|------|---------|
| `project` | string | project display name |
| `root` | string | project root path |
| `view` | string | active view label (`Home`/`Buffer`/`BufferList`/`MagitStatus`/`Search`/`Log`/`Blame`/`CommitDiff`/`CommitEditor`) |
| `view_name` | string | the store's view name |
| `buffer` | string | current buffer display name |
| `mode` | string | buffer mode display |
| `message` | string | minibuffer message |
| `pending` | string | pending key prefix |
| `activity` | string | activity indicator |
| `point` | object | see below |
| `window` | object | see below |
| `position` | string | scroll position display (`Top`/`Bot`/percent) |
| `which_function` | string | enclosing symbol at point |
| `annotations` | string | annotation count display |
| `dirty` | object \| null | `{staged,unstaged,untracked}` git counts |
| `picker` | object \| null | see below |
| `search` | object \| null | see below |
| `magit` | object \| null | see below |
| `tree` | object \| null | see below |
| `isearch` | object \| null | see below |
| `buffer_list` | object \| null | see below |
| `indexing` | string | symbol-index progress |
| `resolving` | string | definition-resolver progress |
| `crate_indexing` | string | external-crate index progress |

Nested objects (key sets also frozen):

```
point         { line, col }                       # both 0-based
window        { top_line, total_lines, viewport }
dirty         { staged, unstaged, untracked }
picker        { prompt, query, selected, total, rows: [PickerRow] }
PickerRow     { name, label, detail }
search        { title, running, error, narrow_query, total, selected, rows: [SearchRow] }
SearchRow     { kind, file, line, col, text, count }   # line/col/count nullable
magit         { rows_total, narrow_query }
tree          { visible, query, rows_total, selected }
isearch       { active, selected, total }
buffer_list   { query, total, selected }
```

## 4. `quote` and geometry

`quote` renders the frame from the store via the same static render the tests
use, at the **session width**:

- A **headless** session's width comes from `--width` (default 80).
- The **attached** socket's `Session` reads the terminal size the live TUI
  last observed (recorded at startup and on every resize), so an attached
  `quote` renders at the **human's real width** — a 120-column terminal
  yields a 120-column frame. (Before W19 this was hardcoded to 80 and the
  attached `quote` was an 80-column approximation; it is now faithful.)

The height is not part of the frame text, so only the width matters here.
For the strongest proof of what the human sees, read the live terminal
screen directly (the PTY screen), which is what `tools/drive_control.py`
does for the repaint assertion (W14).

## 5. Worked examples

Attached (a TUI is running with the default socket):

```sh
redline open src/main.rs:40           # move the human's cursor
redline goto 10
redline jump parse_args               # symbol picker, filtered
redline search 'fn main'              # project search
redline select 2 && redline confirm   # drive a picker
redline annotate 'look here'          # note at point
redline state --json | jq .point
redline quit
```

Headless (no TUI, one ephemeral session):

```sh
redline --project /repo open Cargo.toml:5 state
redline --project /repo -c 'open src/main.rs' -c 'goto 10' state --json
redline -c 'search needle' -c 'select 1' -c 'confirm' quote
```

Attached, requiring a live session:

```sh
redline --attached open src/main.rs:1 state   # exit 1 if no socket
```
