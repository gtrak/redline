# Plan 019 — external control: drive a live redline from a CLI

**Status:** new. Issues 01–03 open, in dependency order.

**Origin:** user directive (2026-10-02): *"Right now this is a one-shot CLI
review tool, I want to make it controllable from the outside with a CLI so
the agent can drive it in interactive review and walk me through different
things."*

## 1. Why

Redline today is a self-contained TUI: a human types keys, redline reacts.
The agent-coding loop the tool exists to serve has no way in. The agent can
*read* redline only by capturing stdout at quit (the `--notes` dump), and
cannot *steer* it at all. What the user wants is the inverse: the agent
holds the wheel during an interactive review, moving the human's screen
(file, point, search, magit) and reading back what is on it, so it can walk
the human through a change step by step.

That is a **control channel**: the running redline accepts commands from a
local client, and a CLI exposes those commands so an agent (or a shell
script) can drive a session the human is watching.

Two modes fall out of one core, and the core is the same either way:

- **Attached** — redline is running in a pane; the agent's `redline <verb>`
  call connects to its control socket, mutates the live session, and the
  human's screen moves.
- **Headless** — no TUI; `redline --headless -c open -c goto:40 state`
  builds an ephemeral session, runs the commands, and prints the resulting
  state as text/JSON. Pure scripting, no terminal needed.

## 2. What (the design)

### 2.1 The core: a `Session` over `AppStore`

Everything the external CLI can do is expressed against one object:

```
Session {
    store: AppStore,          // the existing headless data model
    project_root: PathBuf,
    base: PathBuf,            // persistence dir (recents/registry/notes)
}
```

with three operations:

- **`do(name, arg)`** — dispatch a named registry command (the M-x set).
- **`feed(keys)` / `text(s)`** — feed one key token, a key sequence, or
  literal text through `key_event` (the *same* routing the terminal uses).
- **`snapshot()`** — read the full observable state: view, project, point,
  window, message, pending/activity, buffer, picker/search/magit/tree
  listings, dirty counts, indexing/search indicators, annotation count.

`AppStore` already exposes every one of these as a `pub fn`; the Session is
a thin, serializable façade, not new state.

### 2.2 The wire: NDJSON over a Unix socket

One request/response per line:

```
→ {"id":7,"cmd":"do","args":{"name":"find-file","arg":"src/main.rs"}}
← {"id":7,"ok":true,"result":{"view":"Buffer","point":{"line":0,"col":0}}}
← {"id":7,"ok":false,"error":{"code":"unknown-command","message":"..."}}
```

Commands (v1): `ping`, `state`, `quote`, `buffer`, `notes`, `do`, `key`,
`keys`, `text`, `quit`. `quote` returns the current frame **as plain text**
(the same static render the tests use), so an agent can see what the human
sees without a terminal.

- **Discovery:** `$XDG_RUNTIME_DIR/redline-<uid>.sock`, else
  `/tmp/redline-<uid>.sock`; overridable with `REDLINE_SOCKET` or
  `--socket`. The app prints the path it bound at startup (stderr).
- **Concurrency:** the socket task serializes on the same
  `Arc<Mutex<AppStore>>` the render loop uses and bumps the revision tick
  after every mutation, so the live UI repaints. The handler runs on the
  tokio runtime; no store lock is held across an await.
- One session per socket path; a stale socket file is detected and replaced.

### 2.3 The client CLI

`redline` with no subcommand stays the TUI (unchanged). With a subcommand
or `-c`, it becomes a **client/commander**:

```
redline state [--json]            # read observable state
redline quote                     # the current frame as text
redline buffer [--range L..M]     # current buffer text
redline notes [--plain]           # annotation dump (existing shape)
redline do <command> [arg]        # dispatch a registry command
redline key <token>               # one key (C-x, RET, M-x, a, TAB, ...)
redline keys "<sequence>"         # a key sequence string ("C-x C-f")
redline text "<literal>"          # type literal text into the insert context
redline quit                      # ask the session to exit
```

Sugar verbs (issue 03) are thin compositions of the above:
`open <path>[:line[:col]]`, `goto <line>`, `jump <symbol>`,
`search <query>`, `isearch <query>`, `select <n>`, `confirm`, `cancel`,
`annotate <text>`, `magit`.

**Mode selection:** if a session socket is reachable, commands go over it
(attached). Otherwise, unless `--attached` is given, they run in an
ephemeral headless session (`--project DIR`, repeatable `-c`), and the final
`state`/`quote` prints. `--attached` makes a missing socket an error.

## 3. Success criteria

1. With redline running, `redline open src/main.rs:40` moves the human's
   cursor, and `redline quote` returns a frame containing that line.
2. `redline --headless --project <repo> -c 'open src/main.rs' -c 'goto 10'
   state --json` prints parseable state and exits 0 with no terminal.
3. `redline do find-file src/main.rs`, `redline key RET`, `redline keys
   "C-x C-b"`, `redline text "hello"` are all accepted and observable in a
   following `state`.
4. A key/command that opens a picker is visible in `state` (picker rows +
   selection) and can be driven (`select`, `confirm`).
5. `state --json` is stable, documented, and round-trips.
6. No regression: the TUI path is byte-for-byte unchanged when no subcommand
   is used; existing tests + the PTY battery remain green.

## 4. Task order

| # | Issue | Depends on | Chunk |
|---|-------|-----------|-------|
| 019-01 | Session core + headless CLI (`do`/`key`/`keys`/`text`/`state`/`quote`/`buffer`/`notes`, `-c`, `--project`, `--json`) | — | 1 |
| 019-02 | Control socket (NDJSON, discovery, serve) + client dispatch in the CLI | 019-01 | 2 |
| 019-03 | Sugar verbs, docs (README + `redline.1`), PTY drive, JSON schema freeze | 019-02 | 3 |

## 5. Explicit non-goals (v1)

- No network transport (Unix socket only; local, same-uid).
- No multi-session registry / named sessions (one socket path = one session).
- No auth beyond filesystem permissions on the socket.
- No new editing surface: `text` routes through the existing key path, so
  read-only buffers still refuse.
- No Windows transport (Unix socket; documented).
