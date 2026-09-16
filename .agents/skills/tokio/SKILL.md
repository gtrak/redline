---
name: tokio
description: >-
  Tokio 1.53.1 async runtime reference for redline: #[tokio::main], task
  spawning, the mpsc/watch/oneshot channels and when each fits,
  tokio::select!, time utilities, tokio::fs for config/cache IO, and where
  CancellationToken actually lives (tokio-util, not tokio).
---

# tokio

## Version

`tokio = { version = "1.53", features = ["macros", "rt-multi-thread", "sync", "time", "fs"] }`
(pinned to 1.53.1 in Cargo.toml). Verified against https://docs.rs/tokio/1.53.1/tokio/ .

- `macros` — `#[tokio::main]` / `#[tokio::test]` (also needs `rt`)
- `rt-multi-thread` — multi-thread scheduler; `spawn`, `spawn_blocking`, `block_in_place`
- `sync` — channels (mpsc/watch/oneshot/broadcast), Mutex, RwLock, Notify, Semaphore
- `time` — sleep / interval / timeout
- `fs` — async file utilities (spawn_blocking-based)

## Core API

### Entry point and tasks

```rust
#[tokio::main]
async fn main() { /* runs on the multi-thread runtime */ }
```

- `tokio::spawn(future) -> JoinHandle` (re-exported at crate root; `task::spawn` is the
  same). Awaiting the handle returns the value or a `JoinError`.
- `tokio::task::spawn_blocking(|| ...) -> JoinHandle` — runs blocking code on a
  dedicated blocking thread pool. Use for sync git2 / rayon fan-in calls.
- `JoinHandle::abort()` cancels a task at its next `.await`. `spawn_blocking`
  tasks cannot be aborted.

### Channels (tokio::sync)

| Channel | Shape | Use |
|---|---|---|
| `mpsc::channel(cap)` | many producers → 1 consumer, backpressure | worker → UI event stream |
| `mpsc::unbounded_channel()` | many producers → 1 consumer, no backpressure; `send` is sync | sync code → async |
| `watch::channel(initial)` | many producers → many consumers, last value only | state broadcast (config, project-change) |
| `oneshot::channel()` | 1 → 1, single value | request/response, waiting on a background result |

- mpsc: `tx.send(v).await?`; `rx.recv().await -> Option<T>` (`None` when all
  senders drop). Bounded `send` awaits when full (backpressure).
- watch: `tx.send(v)?` is sync; `rx.changed().await?` then `*rx.borrow_and_update()`;
  `rx.has_changed()` is a sync check; `tx.subscribe()` for late receivers.
- oneshot: `tx.send(v)` is sync (OK from any thread, including rayon);
  `rx.await -> Result<T, RecvError>`. Inside `select!` use `&mut rx`.

### select!

```rust
tokio::select! {
    ev = rx.recv() => { /* first branch to finish */ }
    _ = tokio::time::sleep(d) => { /* timeout */ }
}
```

- Resolves on the first branch to complete, cancelling the rest — branches must be
  cancellation-safe (`mpsc::Receiver::recv` and `watch::Receiver::changed` are).
- `biased;` forces top-to-bottom polling order (default is random).
- `branch, if cond =>` disables a branch when `cond` is false.
- Panics if all branches are disabled and there is no `else =>` arm.

### time

- `tokio::time::sleep(Duration)` / `sleep_until(Instant)`
- `tokio::time::interval(period)` — the first `tick()` completes immediately
- `tokio::time::timeout(d, fut)` — `Err(Elapsed)` on timeout
- `tokio::time::Duration` re-exports `std::time::Duration`

### fs (config and cache IO)

```rust
let cfg = tokio::fs::read_to_string(config_path).await?;
tokio::fs::create_dir_all(cache_dir).await?;
tokio::fs::write(path, bytes).await?;
tokio::fs::try_exists(path).await? // Ok(true) if the path exists
```

Every op runs on the `spawn_blocking` pool: batch into as few calls as possible
(build a `String`, one `write`), and `flush()` a `File` after writes.

### CancellationToken — NOT in tokio

tokio 1.53.1 has no `CancellationToken` (verified: `tokio::sync` contains only
broadcast, mpsc, oneshot, watch + Mutex/RwLock/Notify/Semaphore/Barrier/OnceCell).
It lives in `tokio_util::sync::CancellationToken` (verified on docs.rs/tokio-util):
`new()`, `cancel()`, `cancelled()`, `child_token()`, `drop_guard()`,
`run_until_cancelled(fut)`. redline does not depend on tokio-util; add it if
cancellation tokens are needed, or model shutdown with a watch channel / mpsc close.

## Usage in redline

- App chassis (issue 01): `#[tokio::main]` awaits iocraft's `fullscreen()` future.
- grep/index/git workers: `tokio::spawn` (or `spawn_blocking` for sync git2 / rayon
  fan-in); each worker posts results through a cloned `mpsc::Sender`; the UI loop
  consumes the `Receiver` (e.g. inside a `use_future` poll loop) and updates state.
- Project-change events (issue 04, plan decision 7): `watch` channel — the
  debounced watcher sends the latest event; git status and the symbol index
  subscribe via `tx.subscribe()` and only ever need the newest value.
- One-shot waits (index ready, picker preview loads): `oneshot`.
- Debounce / refresh timers: `interval` (watcher ~1s) and `sleep`.
- Config (`~/.config/redline/config.toml`) and cache (`~/.cache/redline/`):
  `tokio::fs` from async tasks; never call `std::fs` in async code.

## Gotchas

1. `select!` cancels unfinished branches each iteration — in `loop { select! }`
   only use cancellation-safe futures (mpsc `recv` is; `write_all`/`read_to_string`
   are not).
2. `oneshot::Receiver` needs `&mut` inside `select!`: `msg = &mut rx => ...`.
3. `interval`'s first `tick()` fires immediately — drop it or use `interval_at` if
   you want the first tick after one period.
4. `tokio::fs` = `spawn_blocking` under the hood: many small reads/writes each pay
   a thread-hop; batch them.
5. `CancellationToken` is `tokio_util::sync`, not `tokio::sync`.
6. `spawn_blocking` tasks can't be aborted; keep blocking work short and
   cancellation-aware (check a watch flag).
