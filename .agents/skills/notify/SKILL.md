---
name: notify
description: >-
  notify 8.2.0 + notify-debouncer-full 0.7.0 reference for live-reload of
  agent-edited files (plan issue 04): RecommendedWatcher, Watcher trait,
  Event/EventKind shape, debouncer constructor, DebouncedEvent, threading.
  Verified against docs.rs.
---

# notify

File watching for live-reloading agent-edited files. `notify` provides the
platform watcher; `notify-debouncer-full` coalesces rapid events (agent churn).

## Version

- `notify = "8.2"` (8.2.0). `RecommendedWatcher` is a type alias for the
  platform backend (inotify on Linux; `INotifyWatcher`, `PollWatcher`,
  `NullWatcher` re-exported at the crate root).
- `notify-debouncer-full = "0.7"` (0.7.0), requires notify ^8.2; re-exports
  `notify` and `file_id`; feature pass-throughs (`macos_fsevent`,
  `crossbeam-channel`, `flume`, ...) are off by default.

## Core API

### notify 8.2.0 (verified on docs.rs)

- `notify::recommended_watcher(event_handler) -> Result<RecommendedWatcher>`
  where `event_handler: F`, `F: EventHandler`.
- `EventHandler: Send + 'static`, method
  `fn handle_event(&mut self, event: Result<Event>)`. Blanket impl for
  `FnMut(Result<Event>) + Send + 'static`; also for
  `std::sync::mpsc::Sender<Result<Event>>` (crossbeam/flume via features).
- `Watcher` trait (implemented by `RecommendedWatcher`):
  - `watch(&mut self, path: &Path, mode: RecursiveMode) -> Result<()>`
  - `unwatch(&mut self, path: &Path) -> Result<()>`
  - associated `new(event_handler, config)`, `kind() -> WatcherKind`;
    provided `paths_mut()` (batch add/remove) and `configure(config)`.
- `RecursiveMode::Recursive` / `RecursiveMode::NonRecursive`; for a file
  path the mode is ignored.
- `Event { pub kind: EventKind, pub paths: Vec<PathBuf>, pub attrs:
  EventAttributes }` — `Send + Sync`; `need_rescan()` = "assume anything changed".
- `EventKind` exact shape:
  ```rust
  pub enum EventKind {
      Any,               // unit variant — NOT a constant (notify 7+/8)
      Access(AccessKind),
      Create(CreateKind),
      Modify(ModifyKind),
      Remove(RemoveKind),
      Other,
  }
  ```
  Helpers: `is_access()`, `is_create()`, `is_modify()`, `is_remove()`, `is_other()`.

### notify-debouncer-full 0.7.0 (verified on docs.rs)

- Constructor — there is no `new_flexible` in 0.7.0:
  ```rust
  pub fn new_debouncer<F: DebounceEventHandler>(
      timeout: Duration,
      tick_rate: Option<Duration>,
      event_handler: F,
  ) -> Result<Debouncer<RecommendedWatcher, RecommendedCache>, Error>
  ```
  `timeout`: when a pending debounced event is emitted; `tick_rate`: how
  often the queue is flushed (`None` = 1/4 of timeout). `new_debouncer_opt`
  exists for custom configuration.
- `DebounceEventHandler: Send + 'static`, method
  `fn handle_event(&mut self, event: DebounceEventResult)`; blanket impl
  for `FnMut(DebounceEventResult) + Send + 'static`, and for
  `std::sync::mpsc::Sender<DebounceEventResult>`.
- `DebounceEventResult = Result<Vec<DebouncedEvent>, Vec<notify::Error>>`.
- `DebouncedEvent { pub event: Event, pub time: Instant }` — wraps the
  original event (`deb.event.paths`, `deb.event.kind`) and `Deref`s to `Event`.
- `Debouncer<T: Watcher, C: FileIdCache>` is a guard; dropping it stops
  the debouncer. Methods: `watch(impl AsRef<Path>, RecursiveMode)`,
  `unwatch(impl AsRef<Path>)`, `configure(Config)`, `stop(self)` (blocks
  up to one tick_rate), `stop_nonblocking(self)`. `watcher()`/`cache()`
  are deprecated; there is no `channel()` in 0.7 — pass an `mpsc::Sender`
  as the handler to get a receiver.

### Threading model

- The handler (closure or channel `Sender`) is moved into an internal event
  thread owned by the watcher/debouncer — both handler traits require
  `Send + 'static`. The internal thread *sends*; `rx.recv()` runs on
  whatever thread you choose (e.g., the UI loop).
- `Debouncer` is `Send + Sync`: the guard can live in a tokio task while
  the UI thread owns the `Receiver`.

## Verified snippet

Watch a project dir recursively; map changed paths to a UI refresh:

```rust
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;
use notify_debouncer_full::{new_debouncer, notify::RecursiveMode, DebounceEventResult};

let (tx, rx) = mpsc::channel::<DebounceEventResult>();
let mut debouncer = new_debouncer(
    Duration::from_secs(1), // coalesce agent churn; tick rate = 1/4 of timeout
    None,
    tx,                     // mpsc::Sender implements DebounceEventHandler
)?;
debouncer.watch(Path::new("project_dir"), RecursiveMode::Recursive)?;
// UI / event-loop thread:
while let Ok(result) = rx.recv() {
    match result {
        Ok(events) => {
            let mut changed = Vec::new();
            for ev in events {
                if ev.event.need_rescan() { continue; } // whole tree dirty
                if ev.event.kind.is_modify()
                    || ev.event.kind.is_create()
                    || ev.event.kind.is_remove()
                {
                    changed.extend(ev.event.paths);
                }
            }
            for path in &changed {
                ui_refresh(path); // publish on the project-change bus
            }
        }
        Err(errors) => { /* log watch errors */ }
    }
}
```

## Usage in redline

- `src/app/watcher.rs` — one `Debouncer` per open project:
  `new_debouncer(~1s, None, tx)` + `watch(project_root, Recursive)`.
- `src/app/events.rs` — receiver thread maps `DebouncedEvent` paths to
  project-change bus events; file views reload; git status (07) and symbol
  index (05) subscribe later.
- Config `auto_reload` toggle / suspend: drop the debouncer (or
  `stop_nonblocking()`), rebuild to resume.

## Gotchas

- `EventKind::Any` is a *variant* in notify 7+/8, not the 5.x-style constant.
- No `Debouncer::channel()` in 0.7.0 (older tutorials show it); pass an
  `mpsc::Sender` as the handler instead.
- `DebouncedEvent` fields are `event` and `time`, not `paths`/`kind`
  directly (`ev.paths` works via `Deref`).
- Some editors save as remove-then-create; the debouncer suppresses
  `Modify` after `Create` and stitches renames, but a move out of the
  watched tree can surface as `Remove`.
- Inotify limits: recursive watches count every directory against
  `fs.inotify.max_user_watches`; failures arrive on `Err(DebounceEventResult)`.
- `stop()` may block up to one tick_rate; use `stop_nonblocking()` from the UI thread.
