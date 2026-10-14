# Task spec — issue-019-02: control socket + client dispatch

## Do this first (mandatory — do NOT explore)

The 019-01 worker twice burned its whole budget reading. Follow the same
discipline that worked for 01a/01b: read ONLY the files below, then WRITE
CODE immediately, compile, fix only the errors the compiler names.

1. `src/main.rs` — the WHOLE file (you edit `main` and the headless path).
2. `src/session.rs` — the `Session` API you will call server-side
   (`Session::{at, store, do_command, key, keys, text, snapshot, quote,
   buffer_text, notes, quit_requested}`; `Snapshot` is `Serialize`).
3. `src/app/events.rs` — the `ChangeBus` `watch`-channel pattern (COPY this
   shape for the repaint bus; it is the established idiom).
4. `src/ui/root/hooks.rs` lines 130–180 (`drain_project_changes`) — COPY
   this `use_future` drain shape for the repaint drain.
5. `src/ui/root/mod.rs` lines 55–95 (where the drains are installed, in
   order; you add one more).
6. `.agents/skills/tokio/SKILL.md` and `.agents/skills/support-crates/SKILL.md`
   — the tokio features available. **`net` is NOT currently enabled** in
   `Cargo.toml` (`features = ["macros","rt-multi-thread","sync","time","fs"]`).
   You MUST add `"net"` and `"io-util"` to the tokio features. This is the
   one allowed dependency-manifest change (still no new crates).

## Part A — a repaint bus on the store

Add to `src/app/events.rs` (mirror `ChangeBus`):

```rust
/// A monotonically-counting repaint signal: the control socket bumps it
/// after every mutating request so the render loop repaints. `watch` so
/// many subscribers are fine and the latest value wins.
#[derive(Clone)]
pub struct RepaintBus { tx: watch::Sender<u64> }
impl RepaintBus {
    pub fn new() -> Self { let (tx, _rx) = watch::channel(0u64); Self { tx } }
    pub fn subscribe(&self) -> watch::Receiver<u64> { self.tx.subscribe() }
    pub fn bump(&self) { let n = *self.tx.borrow() + 1; let _ = self.tx.send(n); }
}
impl Default for RepaintBus { fn default() -> Self { Self::new() } }
```

Add a `pub repaint_bus: RepaintBus` field to `AppStore` (init it in both
`new()` and `at()`), and a `pub fn repaint_bus(&self) -> &RepaintBus`
accessor next to `watch_bus` (in `src/app/store/index_wiring.rs`).

## Part B — the Root drain

In `src/ui/root/hooks.rs`, add (mirror `drain_project_changes`):

```rust
/// The `use_future` drain of the control-socket repaint bus. The socket
/// task bumps it after every mutation; bumping the revision tick makes
/// iocraft re-render from the already-mutated store.
pub(super) fn drain_repaint(hooks: &mut Hooks, store: Arc<Mutex<AppStore>>, mut tick: State<u64>) {
    let mut rx = store.lock().unwrap().repaint_bus().subscribe();
    hooks.use_future(async move {
        while rx.changed().await.is_ok() {
            // Coalesce a burst into one repaint.
            while rx.has_changed().unwrap_or(false) {
                let _ = rx.borrow_and_update();
            }
            tick.set(tick.get() + 1);
        }
    });
}
```

Install it in `Root` (add one line in the unconditional hook block, at the
END of the existing drain list so hook order stays stable).

## Part C — the protocol + server

Create `src/control.rs` with `mod` registered in `main.rs` (`mod control;`).

```rust
#[derive(Deserialize, Serialize, Debug)]
pub struct Request { pub id: u64, pub cmd: String, #[serde(default)] pub args: serde_json::Value }

#[derive(Serialize, Deserialize, Debug)]
pub struct Response { pub id: u64, pub ok: bool, #[serde(skip_serializing_if="Option::is_none")] pub result: Option<serde_json::Value>, #[serde(skip_serializing_if="Option::is_none")] pub error: Option<WireError> }

#[derive(Serialize, Deserialize, Debug)]
pub struct WireError { pub code: String, pub message: String }
```

- **Discovery** (`pub fn socket_path(explicit: Option<&Path>) -> PathBuf`):
  explicit arg, else `$REDLINE_SOCKET`, else
  `$XDG_RUNTIME_DIR/redline-<uid>.sock`, else `/tmp/redline-<uid>.sock`.
  Get the uid from `std::os::unix::fs::MetadataExt` on `/proc/self` OR
  simply `unsafe { libc::getuid() }` is NOT available (no libc dep) — use
  `std::env::var("UID")` with a fallback to the process id. Document the
  choice. Keep it deterministic.
- **`pub async fn serve(sock: PathBuf, store: Arc<Mutex<AppStore>>) -> anyhow::Result<()>`**:
  remove a stale socket file if the path exists and connect fails; then
  `UnixListener::bind`. Spawn (or accept-loop inline) per connection:
  read newline-delimited requests with `BufReader::new(stream).lines()`,
  parse each as `Request`, apply via `apply_request`, write one JSON line +
  `\n`, flush. **No store lock across an await**: `apply_request` locks,
  mutates, drops the guard, returns the `Response`; then the caller writes.
- **`pub fn apply_request(store: &Arc<Mutex<AppStore>>, root: &Path, width: u16, req: &Request, session: &mut Session) -> Response`**
  — hmm: the socket owns a `Session` (built once at bind, rooted at the TUI
  project) so `state`/`quote`/`buffer`/`notes` reuse 019-01. But the TUI's
  live store is `Arc<Mutex<AppStore>>` inside `main`, NOT the Session's.
  **Design decision (already made): the socket's `Session` wraps the SAME
  `Arc<Mutex<AppStore>>` the TUI uses.** So add:

```rust
impl Session {
    /// Wrap an existing store handle (the TUI's live store) — used by the
    /// control socket so a remote command mutates the session the human
    /// is watching. The `root`/`width` describe the live session.
    pub fn from_store(store: Arc<Mutex<AppStore>>, root: PathBuf, width: u16, height: u16) -> Self;
}
```

  Use `Session::from_store` for the socket; the headless CLI keeps
  `Session::at`.

- **Command mapping** in `apply_request`:
  - `ping` → `{}`.
  - `state` → `serde_json::to_value(session.snapshot())`.
  - `quote` → `{"text": session.quote()}`.
  - `buffer` (args `{start?, end?}`) → `{"text": session.buffer_text(range)}`.
  - `notes` → `{"text": session.notes(plain)}` (args `{plain?:bool}`).
  - `do` (`{name, arg?}`) → `{}` or error `unknown-command`.
  - `key` (`{token}`), `keys` (`{seq}`), `text` (`{text}`) → `{}` or error.
  - `quit` → set `store.quit = true`? NO — the TUI quits via its own path.
    For the socket, `quit` should set `store.quit = true` so `Root`'s
    `if snap.quit { system.exit() }` ends the loop. Use
    `session.do_command("quit", None)` (the registry `quit` → `begin_quit`,
    which prompts on modified buffers). Return `{}`.
  - unknown cmd → error `{code:"unknown-command"}`.
  - **After EVERY request, `store.lock().unwrap().repaint_bus().bump()`**
    (before or after building the response — bump once).

- **Error mapping**: `SessionError::UnknownCommand` → `unknown-command`;
  `BadKey` → `bad-key`; malformed JSON line → `{id:0, ok:false,
  error:{code:"bad-request"}}` (keep serving).

## Part D — wire `main`

- TUI path: before `app.fullscreen()...await`, build the socket path from
  `cli.socket`; `tokio::spawn(control::serve(sock, Arc::clone(&store_handle)))`
  (or spawn after store construction, before the loop). Print the bound path
  to **stderr**: `redline: control socket at <path>`. Log it with tracing.
  If binding fails, warn and continue (the TUI must still run).
- Headless/client path: if `cli.socket` OR `REDLINE_SOCKET` is set, connect
  to it and route the subcommand over the wire instead of building a local
  Session. Implement a blocking client `control::client::send(sock, req)
  -> anyhow::Result<Response>` using `std::os::unix::net::UnixStream` (std,
  no tokio needed client-side). Map the response to stdout exactly as the
  headless path does (state text/json, quote, buffer, notes), and errors to
  stderr + exit 1. `--attached` makes a missing socket a hard error;
  otherwise fall back to headless **only when `--socket` was not given**
  (a given `--socket` that is missing is always an error).

  Keep this SIMPLE: a `fn try_attached(cli) -> Option<anyhow::Result<()>>`
  that returns `None` when no socket was requested.

## Part E — tests

1. `control::tests::apply_request_round_trips` — build a `Session::at` temp
   project, call `apply_request` for `ping`, `state`, `open`-via-keys,
   `key`, `text`, assert `ok` and the payload shape.
2. `socket_path_prefers_explicit_then_env_then_default` — with a temp
   `REDLINE_SOCKET`, assert precedence. Serialize env mutation with
   `crate::ENV_LOCK` if mutating `std::env`.
3. `control::tests::serve_round_trip` — a `#[tokio::test]` that binds a
   temp socket path, spawns `serve`, connects with the std client, sends
   `ping` + `do open-palette`, asserts `ok`, then drops.
4. A PTY drive `tools/drive_control.py` (COPY the shape of an existing
   `tools/drive_*.py` — read `tools/drive_tree.py` for the harness): launch
   `target/debug/redline --socket <tmpsock>` in a sized PTY, parse the
   `control socket at` line from stderr, then run a SECOND process
   `target/debug/redline --socket <tmpsock> -c 'do find-file' -c 'text Cargo.toml' -c 'key RET' quote`
   and assert the frame text contains `Cargo.toml`; then `--socket ... quit`
   and assert the PTY process exits. Register it in BOTH `tools/gate.sh`
   (SHARED_SUITES) and `tools/pool.py` (BATTERY).

## Verification

```
cargo build --workspace > /tmp/b2.log 2>&1; echo BUILD_EXIT=$?
cargo clippy --workspace --all-targets -- -D warnings > /tmp/c2.log 2>&1; echo CLIPPY_EXIT=$?
cargo test --bin redline > /tmp/t2.log 2>&1; echo TEST_EXIT=$?
python3 tools/drive_control.py > /tmp/d2.log 2>&1; echo DRIVE_EXIT=$?; tail -20 /tmp/d2.log
```
Then read the logs. Commit when green.

## Cargo.toml change (allowed)
Add `"net"` and `"io-util"` to the tokio feature list. No other dep changes.

## Report format
Files changed; the socket/session wiring decision; gate outputs with exit
codes + counts; the drive result; test names + what each would do on absent
code; deviations; anything not done.
