# Issue 019-02 — Control socket + client dispatch

**Objective:** let a running redline be controlled from outside. The TUI
binds a Unix socket and serves NDJSON requests against its live `AppStore`;
the CLI connects to it and routes the 019-01 verbs over the wire.

**Why now:** 019-01 made the verbs exist headlessly. This issue makes them
reach the human's live screen.

## Key decisions

- **Transport: Unix stream socket, NDJSON, one request/response per line.**
  Local, same-uid, filesystem-permission auth only. No TCP, no HTTP, no auth
  tokens (v1).
- **Discovery:** `$REDLINE_SOCKET` if set, else
  `$XDG_RUNTIME_DIR/redline-<uid>.sock`, else `/tmp/redline-<uid>.sock`.
  `--socket PATH` overrides on both sides. The app prints the bound path to
  stderr at startup so a human/agent can find it.
- **Serving on the existing runtime.** Bind with `tokio::net::UnixListener`
  before/at `main`'s runtime setup; a spawned task accepts connections and
  serves each. **No store lock across an await**: lock, apply the request,
  drop the guard, then write the response.
- **Repaint on mutation.** After every mutating request, bump the render
  tick exactly as the key handler does. The Root component needs a way to be
  poked from the socket task — reuse the existing revision-tick mechanism
  (the store is behind `Arc<Mutex<…>>`; the tick is iocraft `State`). The
  clean seam: a `watch`/`Notify` the Root subscribes to, OR a store-owned
  "revision" counter the Root already reads. Choose the smallest change that
  makes a socket-driven mutation repaint; **do not** fork the render model.
- **Client/commander mode** (from 019-01): verbs route over the socket when
  reachable; else headless unless `--attached` was given. The 019-01 flags
  (`--project`, `--json`, `-c`) already exist.

## Files

| File | Change |
|------|--------|
| `src/control/mod.rs` (or `src/control.rs`) | **new** — protocol types (`Request`, `Response`), the tokio serve task, socket discovery. |
| `src/control/client.rs` | **new** — blocking client: connect, send, read one response line. |
| `src/session.rs` | add `Session::apply(Request) -> Response` (the server-side dispatch); reuse 019-01. |
| `src/main.rs` | TUI path binds + serves; client path connects; `--socket`/`--attached` wired; print bound path. |
| `src/ui/root/mod.rs` (or `hooks.rs`) | a repaint seam for socket-driven mutations (smallest viable). |

## Steps

1. Protocol: `Request { id, cmd, args }`, `Response { id, ok, result?, error? }`;
   serde to/from NDJSON. Errors carry a `code` (`unknown-command`, `bad-key`,
   `no-session`, `bad-request`, …).
2. `Session::apply` — map each cmd (`ping`, `state`, `quote`, `buffer`,
   `notes`, `do`, `key`, `keys`, `text`, `quit`) to the 019-01 method,
   overriding the snapshot shape only where the request asks (`state`/`quote`
   return their payloads in `result`).
3. Socket discovery + a `bind` that removes a stale socket file. Print path.
4. Serve task: accept loop; per connection, read lines, apply, write lines.
5. Repaint seam: mutation → tick. Verify a socket `open` repaints the TUI.
6. Client: blocking connect with a short timeout; a missing socket is a
   clean error (`no-session`) unless a headless fallback applies.
7. Wire `main`: TUI binds+serves when `--socket`/default path is free; the
   commander route in front of the headless route.

## Verification

- `cargo build`, `clippy -D warnings`, `cargo test --bin redline` green.
- A **PTY drive** (`tools/drive_control.py`, registered in `tools/gate.sh`
  SHARED_SUITES **and** `tools/pool.py` BATTERY) that: launches redline in a
  sized PTY with `--socket <tmp>`, waits for the path on stderr, then from a
  **second process** runs `redline --socket <tmp> open <file>`, `... quote`,
  `... state --json`, asserts the frame shows the file (the human's screen
  moved), then `... quit` and asserts exit 0. This is the assertion that the
  whole feature exists.
- A unit/integration test for `Session::apply` round-tripping each cmd
  without a socket.
- A negative test: connecting to a nonexistent socket yields `no-session`
  and a nonzero exit; `--attached` makes it a hard error.
