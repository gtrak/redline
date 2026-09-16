---
name: tokio
description: >-
  Tokio 1.53.1 async backbone for redline: the #[tokio::main] runtime that hosts
  iocraft's render loop, background grep/index/git workers posting events to the
  UI via mpsc/watch/oneshot channels, timers (sleep/interval), select! for the
  UI loop, and tokio::fs for config/cache IO. Documents only the parts redline
  uses (no net, no process, no io-stdin). Verified against the vendored tokio
  1.53.1 source in the cargo registry.
---

# tokio

## Version & features

`tokio = { version = "1.53", features = ["macros", "rt-multi-thread", "sync", "time", "fs"] }`
(resolves to 1.53.1; all facts below verified against the vendored source at
`~/.cargo/registry/src/.../tokio-1.53.1/`).

What each enabled feature unlocks (per tokio's own `Cargo.toml` features section
and crate-root docs):

- `macros` — enables the `#[tokio::main]` and `#[tokio::test]` attributes
  (depends on `tokio-macros`).
- `rt-multi-thread` — the multi-threaded, work-stealing scheduler. Feature
  definition is `rt-multi-thread = ["rt"]`, so `rt` comes along: `tokio::spawn`,
  the current-thread scheduler, and non-scheduler utilities.
- `sync` — all `tokio::sync` types (mpsc, watch, oneshot, broadcast, Mutex, …).
- `time` — `tokio::time` types and lets the schedulers enable the built-in timer.
- `fs` — `tokio::fs` types.

Not enabled (and therefore NOT available): `net`, `process`, `signal`,
`io-std`, `io-util`, `parking_lot`, `test-util`.

**CancellationToken is NOT in tokio.** Verified by searching the entire
tokio-1.53.1 source tree: zero occurrences of `CancellationToken`. It lives in
`tokio_util::sync::CancellationToken` (verified: `tokio-util-0.7.19` in the
registry, `src/sync/cancellation_token.rs`). redline's Cargo.toml does NOT
depend on tokio-util; if a cancellation token is needed, either add tokio-util
or model shutdown with an mpsc `close()` / a `watch` channel.

## Runtime

### Entry point

`#[tokio::main]` (macro, verified in tokio-macros 2.7.2 `entry.rs`):

- Default flavor: **`multi_thread`** for `main`; **`current_thread`** for
  `#[tokio::test]`. Override with `flavor = "current_thread"` /
  `flavor = "multi_thread"`. (Old names `single_thread` / `basic_scheduler` /
  `threaded_scheduler` are rejected with an error.)
- Other supported attributes: `worker_threads`, `start_paused` (test clock),
  `name`, `unhandled_panic` (`"ignore"` / `"shutdown_runtime"`), `crate_name`.
- It expands to a **synchronous** function that creates a runtime and
  `block_on`s the body. The marked async fn is NOT a spawned worker — the
  macro docs say awaiting other futures from it "will not perform as fast as
  those spawned as workers". For redline this is exactly right: `main` awaits
  iocraft's `fullscreen()` future, and everything else runs as spawned tasks.

### Spawning tasks

```rust
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where F: Future + Send + 'static, F::Output: Send + 'static
```

- Re-exported at crate root as `tokio::spawn`. The task starts running in the
  background immediately, even if the `JoinHandle` is never awaited.
- `tokio::spawn` must be called from inside a runtime context (or via
  `Runtime::enter` / `Handle::enter`).
- "There is no guarantee that a spawned task will execute to completion" — on
  runtime shutdown, outstanding tasks are dropped.
- `JoinHandle::abort(&self)` cancels an async task at its next `.await`;
  awaiting it then fails with a cancelled `JoinError`. **On a
  `spawn_blocking` handle, `abort` "will not have any effect"** (verified doc
  comment) — the blocking task keeps running.

### Blocking work

```rust
pub fn spawn_blocking<F, R>(f: F) -> JoinHandle<R>
where F: FnOnce() -> R + Send + 'static, R: Send + 'static
```

Runs the closure on a dedicated blocking thread pool (threads spawned on
demand, kept alive per `thread_keep_alive`, upper limit configurable on
`Builder`). This is where redline's sync work belongs:

- **git2 calls** (libgit2 is fully synchronous) — never call `git2` from an
  async task on a worker thread.
- **rayon fan-in for the tree-sitter symbol index** — the plan's tokio docs
  hint is literal: "If using rayon, you can use a `oneshot` channel to send
  the result back to Tokio when the rayon task finishes."
- `tokio::fs` uses this pool internally (see fs section).

`task::block_in_place(f)` runs blocking code on the *current* worker thread,
but **panics if called from a `current_thread` runtime** — not needed for
redline; prefer `spawn_blocking`.

### Shutdown (TUI-relevant)

Verified from `Runtime` struct docs:

- Dropping the `Runtime` (or `shutdown_background` / `shutdown_timeout`)
  shuts it down. Spawning is no longer possible; outstanding tasks are
  dropped, `spawn_blocking` work keeps running until it returns.
- `shutdown_timeout(duration)` blocks at most `duration`, then leaks the
  still-running work; `shutdown_background` = `shutdown_timeout(0)`.
- For a TUI, the usual shape is: the UI loop exits (user quits), `main`
  returns, the runtime drops, and spawned workers are dropped with it. If
  workers hold resources needing explicit cleanup (closing channels so
  consumers unblock), do it cooperatively — e.g. drop all `mpsc::Sender`
  clones so the UI's `recv()` yields `None` — rather than relying on runtime
  shutdown to stop them.

## Channels

All verified from `src/sync/{mpsc,watch,oneshot}`.

### mpsc (multi-producer, single-consumer)

Constructors:

- `mpsc::channel<T>(buffer: usize) -> (Sender<T>, Receiver<T>)` — bounded.
- `mpsc::unbounded_channel<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>)`.

Semantics:

- **Bounded `Sender::send` is `async`** (`pub async fn send(&self, value: T)
  -> Result<(), SendError<T>>`); at capacity it awaits until capacity is
  available — the module docs call this **backpressure** explicitly.
- **Unbounded `Sender::send` is sync** (`pub fn send(&self, message: T) ->
  Result<(), SendError<T>>`) and "will always complete immediately" — usable
  from sync code (e.g. a rayon thread).
- The receive method is **`recv`**, not `receive`:
  `Receiver::recv(&mut self) -> Option<T>` (async). Returns `None` when all
  senders are dropped and the buffer is drained (the "termination event").
  Also available: `try_recv()` (sync), `blocking_recv()`, `close()` (stop
  accepting sends, then drain — "clean shutdown"), `is_closed()`, `len()`,
  `capacity()`.
- Bounded `Sender` also has `try_send`, `send_timeout`, `blocking_send`,
  `closed()` (async wait for receiver drop), and permit APIs
  (`reserve`/`try_reserve`).
- Dropping the `Receiver` makes all further sends fail and unread messages are
  dropped.

### watch (single-broadcast, last value wins)

- `watch::channel<T>(init: T) -> (Sender<T>, Receiver<T>)` — one sender, many
  receivers via `Sender::subscribe(&self) -> Receiver<T>`.
- **`Sender::send(&self, value: T) -> Result<(), SendError<T>>` is sync**
  (also `send_modify`, `send_replace`).
- **The value is retained only while at least one receiver is alive**: with
  no live receiver, `send` returns `SendError` and the value is *not* stored
  (so `Sender::borrow()` stays at the initial value). If you read the latest
  via `borrow()`/`current()` while no consumer is subscribed, keep a keeper
  receiver alive (verified: a watcher bus published with zero subscribers
  left `borrow()` at the initial value).
- `Receiver::changed(&mut self) -> Result<(), RecvError>` (async) — awaits
  until the value changes, then `borrow_and_update(&mut self) -> Ref<'_, T>`
  or `borrow(&self) -> Ref<'_, T>` (sync, no update). `has_changed()` is a
  sync check. `RecvError` means the sender was dropped.
- Receivers only ever need the newest value — intermediate values are
  coalesced.

### oneshot (one value, one receiver)

- `oneshot::channel<T>() -> (Sender<T>, Receiver<T>)`.
- `Sender::send(self, t: T) -> Result<(), T>` — **sync**, takes ownership,
  OK to call from any thread (the mpsc module docs explicitly say sending on
  a oneshot "from outside the runtime is perfectly fine").
- **`Receiver` has no inherent async `recv` method.** It implements
  `Future<Output = Result<T, RecvError>>`, so you await it as `rx.await` (or
  `&mut rx` inside `select!`). Sync variants: `try_recv()` and
  `blocking_recv()`.

### Which fits what (redline)

| Need | Channel |
|---|---|
| Worker → UI events (grep hits, index progress, git output) | `mpsc` (bounded, for backpressure; unbounded if workers must never block) |
| State broadcast (project-change events from the watcher, "index is ready", config) | `watch` — consumers only care about the newest value |
| Request/response (await the result of one index build, one git command) | `oneshot` |

## Time

Verified from `src/time/`:

- `tokio::time::sleep(duration: Duration) -> Sleep` / `sleep_until(deadline:
  Instant)`. Panics if there is no current timer (i.e. created outside a
  runtime context, or on a runtime without the time driver).
- `tokio::time::interval(period: Duration) -> Interval`:
  - **The first `tick()` completes immediately** (documented example:
    `tick()` → immediate, then every `period`).
  - `tick(&mut self) -> Instant` (async).
  - Panics if `period` is zero.
  - **Missed-tick overlap behavior**: default `MissedTickBehavior` is
    **`Burst`** — if the UI was busy and a tick was missed, the interval
    "fires ticks as fast as possible until it is caught up in time".
    `Interval::set_missed_tick_behavior(MissedTickBehavior::Delay)` waits a
    full period after a missed tick; `Skip` jumps to the next scheduled tick.
    (These strategies only apply when the delay is greater than 5 ms.)
  - For a UI heartbeat you usually want `Skip` (or `Delay`) — `Burst` would
    fire a burst of catch-up ticks after a heavy frame.
- `interval_at(start: Instant, period: Duration)` — first tick completes at
  `start` instead of immediately.
- `timeout(duration, future)` / `timeout_at(deadline, future)` — for
  bounding e.g. a git2 call.
- `tokio::time::Duration` is a re-export of `std::time::Duration`;
  `tokio::time::Instant` is tokio's own type (not `std::time::Instant`).

## select!

Verified from the `select!` macro docs (`src/macros/select.rs`):

- Shape: `<pattern> = <async expression> (, if <precondition>)? => <handler>`
  branches, plus an optional `else => <expr>`. Waits on all branches
  concurrently and returns when the **first** branch's value matches its
  pattern; the remaining branches are **cancelled**.
- Panics if all branches are disabled (precondition false or pattern mismatch)
  and there is no `else` branch.
- Default branch order is random; `biased;` forces top-to-bottom polling order
  (and the docs warn: if one branch is constantly ready, place the other
  branch earlier).
- **Cancellation safety** (documented list): `mpsc::Receiver::recv`,
  `UnboundedReceiver::recv`, `watch::Receiver::changed` ARE cancellation
  safe; `read_exact`/`read_to_end`/`read_to_string`/`write_all` and
  `Mutex::lock`/`RwLock::*`/`Semaphore::acquire`/`Notify::notified` are NOT.
  In a `loop { select! { … } }`, only cancellation-safe receives belong in
  branches, or you can lose messages.

Realistic redline snippet — UI loop ticking + draining the worker event
channel:

```rust
let mut worker_rx = worker_tx.subscribe(); // mpsc::Receiver<WorkerEvent>
let mut heartbeat = tokio::time::interval(Duration::from_millis(250));
heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);

loop {
    tokio::select! {
        maybe_event = worker_rx.recv() => {
            match maybe_event {
                Some(event) => apply_event(event),   // grep hit, index done, git output
                None => break,                       // all workers dropped → quit
            }
        }
        _ = heartbeat.tick() => {
            // UI heartbeat: re-render, check for stale views
        }
    }
}
```

(`recv` and `tick` are both cancellation-safe, so the loop never loses
events.)

## fs

Verified from `src/fs/`:

- `tokio::fs::read_to_string(path) -> io::Result<String>` (async)
- `tokio::fs::write(path, contents: impl AsRef<[u8]>) -> io::Result<()>`
  (async; **overwrites** the existing file, if any)
- Also: `read` (→ `Vec<u8>`), `open`/`File::create`, `create_dir_all`,
  `metadata`, `symlink_metadata`, `read_dir`, `rename`, `remove_file`,
  `remove_dir_all`, `try_exists`, `canonicalize`.

**Important documented behavior**: "most operating systems do not provide
asynchronous file system APIs. Because of that, Tokio will use ordinary
blocking file operations behind the scenes. This is done using the
`spawn_blocking` threadpool." So every `tokio::fs` call pays a thread-hop —
batch config/cache work into as few calls as possible (build the whole
`String`, one `write`).

**When `std::fs` off-thread is equally fine**: anything already running on a
blocking thread (a `spawn_blocking` closure, a rayon task) may just use
`std::fs` directly — no need for `tokio::fs` there, since `tokio::fs` would
only re-hop onto the same blocking pool. `tokio::fs` earns its keep when
you're in an async context on a worker thread and want to avoid blocking it.
(For redline: config load at startup and cache writes from workers can be
plain `std::fs` inside `spawn_blocking`; `tokio::fs` is the tool for the
rare async-context read.)

## Usage in redline

Mapping to `docs/plans/001-redline-code-browser/PLAN.md`:

- **App chassis (issue 01)**: `#[tokio::main]` (default `multi_thread`)
  awaits iocraft's `fullscreen()` render-loop future. The render loop is the
  long-running "UI task"; everything else posts to it.
- **grep/index/git workers**: `tokio::spawn` an async worker task per
  operation; sync work (git2, rayon tree-sitter index fan-in) goes through
  `spawn_blocking` (or rayon + `oneshot` back to the async side, per tokio's
  own hint). Each worker holds a cloned `mpsc::Sender<WorkerEvent>`; the UI
  loop drains the single `Receiver` (e.g. via `select!` in a `use_future`
  poll loop). Bounded channel gives backpressure so a slow UI can't be
  flooded by a huge grep.
- **Watcher bus (issue 04, plan decision 7)**: the debounced `notify`
  watcher publishes project-change events; git status and the symbol index
  "subscribe" via `watch::Sender::subscribe()` — each consumer only needs
  the newest event, which is exactly `watch` semantics.
- **One-shot waits**: "index ready", "git status finished", picker preview
  loads → `oneshot` (sync `send` from the worker thread, `rx.await` in the
  UI task).
- **Timers**: `interval` (with `Skip`/`Delay` missed-tick behavior) for
  heartbeat/debounce; `sleep` for one-shot delays (e.g. ~1s live-reload).
- **Config/cache IO**: `~/.config/redline/config.toml` and
  `~/.cache/redline/` via `tokio::fs` in async contexts or `std::fs` inside
  `spawn_blocking`.

## Gotchas

All items below are verified in the 1.53.1 source/doc comments:

1. **Method name is `recv`, not `receive`** — `mpsc::Receiver::recv()`,
   `watch::Receiver::changed()`, `oneshot::Receiver::try_recv()/blocking_recv()`.
   And oneshot's `Receiver` has **no async `recv()` at all** — it is a
   `Future` itself (`rx.await` / `&mut rx` in `select!`).
2. **Sync vs async send varies by channel**: bounded `mpsc` `send` is async
   (awaits at capacity = backpressure); unbounded `mpsc` `send`, `watch`
   `send`, and `oneshot` `send` are all sync.
3. **`interval`'s first `tick()` fires immediately**, and the default
   missed-tick behavior is **`Burst`** (catch-up burst of ticks), not
   skip. A UI heartbeat left on `Burst` fires a burst of extra ticks after
   any heavy frame — set `Skip` or `Delay`.
4. **`CancellationToken` is `tokio_util::sync`, not `tokio`** — and redline
   does not depend on tokio-util. Model cancellation with mpsc `close()` /
   sender drop (`recv()` → `None`) or a `watch` flag.
5. **`JoinHandle::abort()` does nothing on `spawn_blocking` tasks** — the
   blocking closure keeps running to completion. Keep blocking work short
   and/or cancellation-aware (poll a `watch` flag / `AtomicBool`).
6. **`select!` cancels the other branches every iteration** — in a
   `loop { select! }`, only cancellation-safe receives (mpsc `recv`, watch
   `changed`) belong in branches, or messages are silently lost.
7. **Runtime context is required**: `tokio::spawn` and timer creation
   (`sleep`/`interval` construction) panic outside a runtime context. If a
   helper thread needs to run async work, enter the context
   (`Runtime::enter` / `Handle::enter` / `Handle::block_on`).
8. **`tokio::fs` = `spawn_blocking` under the hood** — each call is a
   thread-hop; batch config/cache IO into as few calls as possible.
9. **`#[tokio::main]` defaults differ**: `multi_thread` for `main`,
   `current_thread` for `#[tokio::test]`. Tests that need multi-thread
   behavior must set `flavor` explicitly.
10. **Runtime shutdown drops async tasks** — "no guarantee that a spawned
    task will execute to completion". Clean worker shutdown is cooperative:
    drop the senders so the UI's `recv()` returns `None` instead of relying
    on the runtime to stop workers.
