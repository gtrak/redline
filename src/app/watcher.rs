//! One debounced file watcher per open project (`notify-debouncer-full`).
//!
//! `start_watch` builds a `Debouncer` rooted at the project, pumps its
//! (blocking) std-mpsc output onto a tokio mpsc, and spawns a consumer task
//! that filters noise, coalesces the batch, and publishes to the
//! project-change bus (`crate::app::events::ChangeBus`). Exactly one watcher
//! is active at a time (enforced by the store's single
//! `Option<ActiveWatcher>` slot); `ActiveWatcher` carries the stop handle
//! plus the consumer task handle.
//!
//! The watcher lives in the plain-Rust `app/` layer (tokio + notify + std;
//! no iocraft) and posts into the store only through the bus.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify_debouncer_full::{
    new_debouncer, notify::RecursiveMode, DebounceEventResult, DebouncedEvent,
};
use tokio::task::JoinHandle;

use crate::app::events::{ChangeBus, ChangeKind, ProjectChange};

/// Default debounce window for production: coalesces agent churn into one
/// bus publish per quiet period.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(400);

/// One active project watcher: the stop handle (a `watch` flag the consumer
/// polls) plus the consumer task handle.
///
/// Dropping the `ActiveWatcher` does NOT cancel the task (dropping a
/// `JoinHandle` never cancels); `Drop` signals stop best-effort so the
/// notify thread + pump thread wind down while the runtime is still alive.
pub struct ActiveWatcher {
    pub root: PathBuf,
    stop_tx: tokio::sync::watch::Sender<bool>,
    handle: Option<JoinHandle<()>>,
}

impl ActiveWatcher {
    /// Signal the consumer to stop (non-blocking). The consumer then drops
    /// the debouncer, which tears down the notify thread + the pump thread.
    pub fn stop(&mut self) {
        let _ = self.stop_tx.send(true);
    }

    /// Signal stop and await the consumer task's completion (bounded, so a
    /// wedged task can't hang shutdown).
    pub async fn stop_and_wait(&mut self) {
        self.stop();
        if let Some(handle) = self.handle.take() {
            let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
        }
    }
}

impl Drop for ActiveWatcher {
    fn drop(&mut self) {
        // Best-effort stop: the runtime is still alive when the store drops
        // on quit, so the consumer gets a moment to tear the watcher down.
        self.stop();
    }
}

/// Start a debounced watcher for `root`, publishing noise-filtered changes
/// to `bus` after `debounce` of quiescence. Returns `None` when the platform
/// watcher cannot be created (e.g. inotify limit) — the project still works,
/// just without live reload.
pub fn start_watch(root: &Path, bus: &ChangeBus, debounce: Duration) -> Option<ActiveWatcher> {
    // std-mpsc: the debouncer's internal thread sends debounced batches here.
    let (std_tx, std_rx) = mpsc::channel::<DebounceEventResult>();
    let mut debouncer = new_debouncer(debounce, None, std_tx).ok()?;
    debouncer.watch(root, RecursiveMode::Recursive).ok()?;

    // tokio-mpsc: the pump thread forwards std-mpsc events here; the consumer
    // task reads it (async, cancellation-safe `recv`).
    let (tokio_tx, mut tokio_rx) = tokio::sync::mpsc::channel::<DebounceEventResult>(64);
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let mut stop_rx = stop_rx;

    // Pump thread: blocking std-mpsc recv → tokio-mpsc. Exits when the
    // debouncer's sender is dropped (stop) or the tokio receiver is gone.
    std::thread::Builder::new()
        .name("redline-watch-pump".into())
        .spawn(move || {
            while let Ok(result) = std_rx.recv() {
                if tokio_tx.blocking_send(result).is_err() {
                    break;
                }
            }
        })
        .ok()?;

    let bus = bus.clone();
    let root = root.to_path_buf();
    let active_root = root.clone();
    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                result = tokio_rx.recv() => {
                    match result {
                        Some(Ok(events)) => {
                            let change = summarize(&root, events);
                            if !change.is_empty() {
                                bus.publish(change);
                            }
                        }
                        Some(Err(errors)) => {
                            tracing::warn!(?errors, "watcher: debouncer reported errors");
                        }
                        // tokio rx dropped: we own it, so this only happens if
                        // the channel is shut down externally — stop.
                        None => break,
                    }
                }
                _ = stop_rx.changed() => break,
            }
        }
        // Drop the debouncer: stops the notify thread + drops std_tx, so the
        // pump thread's std_rx.recv() disconnects and it exits.
        drop(debouncer);
    });

    Some(ActiveWatcher {
        root: active_root,
        stop_tx,
        handle: Some(handle),
    })
}

/// Map a debounced batch onto a `ProjectChange`, dropping noise (`.git`
/// internal churn, the redline log file) and coalescing repeated paths into
/// one entry each. Returns an empty change when the batch was all noise.
///
/// This is a pure function (no notify / no runtime) so it is unit-testable
/// without a real watcher.
pub fn summarize(root: &Path, events: Vec<DebouncedEvent>) -> ProjectChange {
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut kinds: Vec<ChangeKind> = Vec::new();
    for ev in events {
        // A whole-tree rescan: treat as a broad change of the root (rare).
        if ev.event.need_rescan() {
            push_change(&mut paths, &mut kinds, root.to_path_buf(), ChangeKind::Other);
            continue;
        }
        let kind = if ev.event.kind.is_create() {
            ChangeKind::Create
        } else if ev.event.kind.is_modify() {
            ChangeKind::Modify
        } else if ev.event.kind.is_remove() {
            ChangeKind::Remove
        } else {
            ChangeKind::Other
        };
        for path in ev.event.paths {
            if is_noise(&path, root) {
                continue;
            }
            push_change(&mut paths, &mut kinds, path, kind);
        }
    }
    ProjectChange {
        seq: 0, // the bus stamps the real seq on publish
        paths,
        kinds,
    }
}

/// Coalesce: one entry per path (the latest kind for that path wins).
fn push_change(
    paths: &mut Vec<PathBuf>,
    kinds: &mut Vec<ChangeKind>,
    path: PathBuf,
    kind: ChangeKind,
) {
    if let Some(slot) = paths.iter().position(|p| p == &path) {
        kinds[slot] = kind;
    } else {
        paths.push(path);
        kinds.push(kind);
    }
}

/// Noise filter: `.git` internal churn and the redline log file never
/// produce project-change events (they don't change working-tree content the
/// reader cares about). The watcher only reports paths under `root`, so the
/// cache dir / persistence files (outside `root`) are never seen.
fn is_noise(path: &Path, root: &Path) -> bool {
    // `.git/` internal churn (staging, reflogs, lock files, …).
    if path
        .strip_prefix(root)
        .is_ok_and(|rel| {
            matches!(
                rel.components().next(),
                Some(std::path::Component::Normal(name)) if name == ".git"
            )
        })
    {
        return true;
    }
    // The redline log file, wherever it lands.
    if path.file_name().is_some_and(|name| name == "redline.log") {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::{Event, EventKind};
    use std::time::Instant;

    /// Build a synthetic debounced batch from `(path, kind)` pairs, all
    /// under `root`, without touching the filesystem.
    fn batch(root: &str, entries: &[(&str, EventKind)]) -> Vec<DebouncedEvent> {
        entries
            .iter()
            .map(|(rel, kind)| {
                let path = Path::new(root).join(rel);
                DebouncedEvent {
                    event: Event {
                        kind: *kind,
                        paths: vec![path],
                        attrs: Default::default(),
                    },
                    time: Instant::now(),
                }
            })
            .collect()
    }

    #[test]
    fn summarize_coalesces_repeated_paths_to_one() {
        let root = "/p";
        // 10 modify events for the same path (the coalescing scenario).
        let events = (0..10)
            .map(|_| EventKind::Modify(notify::event::ModifyKind::Any))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|k| {
                DebouncedEvent {
                    event: Event {
                        kind: k,
                        paths: vec![Path::new(root).join("src/a.rs")],
                        attrs: Default::default(),
                    },
                    time: Instant::now(),
                }
            })
            .collect();
        let change = summarize(Path::new(root), events);
        assert_eq!(change.paths.len(), 1, "10 writes → 1 coalesced path");
        assert_eq!(change.paths[0], Path::new(root).join("src/a.rs"));
        assert_eq!(change.kinds, vec![ChangeKind::Modify]);
    }

    #[test]
    fn summarize_maps_kinds() {
        let root = "/p";
        let events = batch(
            root,
            &[
                ("a.rs", EventKind::Create(notify::event::CreateKind::File)),
                ("b.rs", EventKind::Modify(notify::event::ModifyKind::Any)),
                ("c.rs", EventKind::Remove(notify::event::RemoveKind::Any)),
            ],
        );
        let change = summarize(Path::new(root), events);
        assert_eq!(
            change.paths,
            vec![
                Path::new(root).join("a.rs"),
                Path::new(root).join("b.rs"),
                Path::new(root).join("c.rs")
            ]
        );
        assert_eq!(
            change.kinds,
            vec![
                ChangeKind::Create,
                ChangeKind::Modify,
                ChangeKind::Remove
            ]
        );
    }

    #[test]
    fn summarize_drops_git_and_log_noise() {
        let root = "/p";
        let events = batch(
            root,
            &[
                // `.git` internal churn — must be dropped.
                (
                    ".git/index",
                    EventKind::Modify(notify::event::ModifyKind::Any),
                ),
                (
                    ".git/refs/heads/main",
                    EventKind::Modify(notify::event::ModifyKind::Any),
                ),
                // The redline log file — must be dropped.
                ("redline.log", EventKind::Modify(notify::event::ModifyKind::Any)),
                // A real working-tree file — kept.
                ("src/main.rs", EventKind::Modify(notify::event::ModifyKind::Any)),
            ],
        );
        let change = summarize(Path::new(root), events);
        assert_eq!(
            change.paths,
            vec![Path::new(root).join("src/main.rs")],
            "noise must not appear: {change:?}"
        );
    }

    #[test]
    fn summarize_all_noise_is_empty() {
        let root = "/p";
        let events = batch(
            root,
            &[(".git/index", EventKind::Modify(notify::event::ModifyKind::Any))],
        );
        let change = summarize(Path::new(root), events);
        assert!(change.is_empty());
    }

    #[test]
    fn is_noise_classifies() {
        let root = Path::new("/p");
        assert!(is_noise(root.join(".git/HEAD").as_path(), root));
        assert!(is_noise(root.join("redline.log").as_path(), root));
        assert!(!is_noise(root.join("src/a.rs").as_path(), root));
        // A path not under the root is not treated as `.git` noise.
        assert!(!is_noise(Path::new("/elsewhere/.git/index"), root));
    }

    // ── integration tests (tempfile + real notify events) ───────────────
    //
    // These exercise the real debouncer with a short (nonzero) window and
    // assert *eventual* delivery, not instant delivery. The watcher's initial
    // rescan is settled and baselined (via the bus sequence number) before
    // each probe, so only post-baseline writes are asserted on.

    const TEST_DEBOUNCE: Duration = Duration::from_millis(50);
    const WAIT: Duration = Duration::from_secs(4);
    const SETTLE: Duration = Duration::from_millis(500);

    /// Build a real project rooted at `root` (marker file so `detect_root`
    /// finds it) with persistence under a throwaway `base`.
    fn project_store(root: &Path, base: &Path) -> crate::app::store::AppStore {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        crate::app::store::AppStore::at(root, base.to_path_buf())
    }

    /// The detected (canonical) project root, for watching so event paths
    /// match the buffer paths (built from the canonical root).
    fn proj_root(s: &crate::app::store::AppStore) -> PathBuf {
        s.project.as_ref().unwrap().root.clone()
    }

    /// Poll the bus until its sequence is past `baseline` (a fresh publish),
    /// returning the change. `None` on the deadline.
    async fn next_change(
        bus: &crate::app::events::ChangeBus,
        baseline: u64,
        deadline: Duration,
    ) -> Option<crate::app::events::ProjectChange> {
        let start = std::time::Instant::now();
        loop {
            let cur = bus.current();
            if cur.seq > baseline {
                return Some(cur);
            }
            if start.elapsed() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Start the watcher (canonical root) and settle its initial rescan,
    /// returning the baseline sequence (post-settle).
    async fn settle_baseline(s: &mut crate::app::store::AppStore, debounce: Duration) -> u64 {
        let root = proj_root(s);
        s.start_watcher_at(&root, debounce);
        tokio::time::sleep(SETTLE).await;
        s.watch_bus().current().seq
    }

    #[tokio::test]
    async fn write_to_viewed_file_produces_change_event_and_reloads() {
        let root = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = project_store(root.path(), base.path());
        let _rx = s.watch_bus().subscribe(); // keeper: `watch` retains values only while a receiver is alive
        std::fs::write(root.path().join("src/a.rs"), "v1\n").unwrap();
        s.open_path("src/a.rs");
        assert!(s.buffer_text().contains("v1"));

        let baseline = settle_baseline(&mut s, TEST_DEBOUNCE).await;
        assert_eq!(s.watcher_count(), 1);

        // A real write to the viewed file.
        std::fs::write(root.path().join("src/a.rs"), "v2\n").unwrap();
        let change = next_change(s.watch_bus(), baseline, WAIT)
            .await
            .expect("no change event observed within timeout");
        assert!(
            change.paths.iter().any(|p| p.ends_with("src/a.rs")),
            "event must mention the changed file: {change:?}"
        );
        // The store's reload path (the UI subscriber's action) re-reads it.
        s.apply_project_change(&change);
        assert!(
            s.buffer_text().contains("v2"),
            "auto-reload must re-read content"
        );
        s.stop_watcher();
    }

    #[tokio::test]
    async fn coalesces_rapid_writes_to_one_event() {
        let root = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = project_store(root.path(), base.path());
        let _rx = s.watch_bus().subscribe(); // keeper
        std::fs::write(root.path().join("src/a.rs"), "init\n").unwrap();
        let baseline = settle_baseline(&mut s, TEST_DEBOUNCE).await;

        // 10 rapid writes within one debounce window.
        for i in 0..10 {
            std::fs::write(root.path().join("src/a.rs"), format!("w{}\n", i)).unwrap();
        }
        let change = next_change(s.watch_bus(), baseline, WAIT)
            .await
            .expect("no coalesced event within timeout");
        let count = change.paths.iter().filter(|p| p.ends_with("src/a.rs")).count();
        assert_eq!(count, 1, "one coalesced entry for the path: {change:?}");
        s.stop_watcher();
    }

    #[tokio::test]
    async fn git_and_log_noise_produce_no_event() {
        let root = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".git")).unwrap();
        std::fs::write(root.path().join("redline.log"), "log\n").unwrap();
        let mut s = project_store(root.path(), base.path());
        let _rx = s.watch_bus().subscribe(); // keeper
        let baseline = settle_baseline(&mut s, TEST_DEBOUNCE).await;

        // Noise writes only: `.git` internals + the log file.
        std::fs::write(root.path().join(".git/index"), "idx\n").unwrap();
        std::fs::write(root.path().join("redline.log"), "log2\n").unwrap();
        tokio::time::sleep(SETTLE).await;
        assert_eq!(
            s.watch_bus().current().seq,
            baseline,
            "noise must not publish: {:?}",
            s.watch_bus().current()
        );

        // Control: a real working-tree write DOES publish (watcher is alive).
        std::fs::write(root.path().join("real.rs"), "x\n").unwrap();
        let change = next_change(s.watch_bus(), baseline, WAIT)
            .await
            .expect("control: a real write must publish");
        assert!(change.paths.iter().any(|p| p.ends_with("real.rs")));
        s.stop_watcher();
    }

    #[tokio::test]
    async fn suspend_drops_events_resume_watches_again() {
        let root = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = project_store(root.path(), base.path());
        std::fs::write(root.path().join("src/a.rs"), "v1\n").unwrap();
        s.open_path("src/a.rs");
        let _rx = s.watch_bus().subscribe(); // keeper
        let baseline = settle_baseline(&mut s, TEST_DEBOUNCE).await;
        assert_eq!(s.watcher_count(), 1);

        // Suspend: the watcher is stopped (events dropped, not buffered).
        s.toggle_watcher();
        assert!(s.watcher_suspended());
        assert_eq!(s.watcher_count(), 0, "suspended → watcher stopped");

        // A write while suspended must NOT publish or reload (watcher down).
        std::fs::write(root.path().join("src/a.rs"), "v2-suspended\n").unwrap();
        tokio::time::sleep(SETTLE).await;
        assert_eq!(
            s.watch_bus().current().seq,
            baseline,
            "no event while suspended: {:?}",
            s.watch_bus().current()
        );
        assert!(s.buffer_text().contains("v1"), "no reload while suspended");
        assert!(!s.current_buffer_changed_on_disk());

        // Resume: a fresh watcher starts; settle its (default-debounce) rescan.
        s.toggle_watcher();
        assert!(!s.watcher_suspended());
        assert_eq!(s.watcher_count(), 1, "resumed → watcher restarted");
        tokio::time::sleep(SETTLE * 2).await;
        let resume_baseline = s.watch_bus().current().seq;

        // A write while resumed DOES publish (and reloads via the subscriber).
        std::fs::write(root.path().join("src/a.rs"), "v3-resumed\n").unwrap();
        let change = next_change(s.watch_bus(), resume_baseline, WAIT)
            .await
            .expect("resume must watch again");
        s.apply_project_change(&change);
        assert!(
            s.buffer_text().contains("v3-resumed"),
            "reload must happen after resume"
        );
        s.stop_watcher();
    }

    #[tokio::test]
    async fn project_switch_stops_old_starts_new_exactly_one() {
        let base = tempfile::tempdir().unwrap();
        let p1 = base.path().join("alpha");
        let p2 = base.path().join("beta");
        std::fs::create_dir_all(p1.join("src")).unwrap();
        std::fs::create_dir_all(p2.join("src")).unwrap();
        std::fs::write(p1.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(p1.join("src/a.rs"), "a1\n").unwrap();
        std::fs::write(p2.join("pyproject.toml"), "[project]\n").unwrap();
        std::fs::write(p2.join("src/b.py"), "b1\n").unwrap();

        let mut s = crate::app::store::AppStore::at(&p1, base.path().to_path_buf());
        let _rx = s.watch_bus().subscribe(); // keeper
        let p1_root = proj_root(&s);
        s.start_watcher_at(&p1_root, TEST_DEBOUNCE);
        tokio::time::sleep(SETTLE).await;
        assert_eq!(s.watcher_count(), 1);
        assert_eq!(s.watcher_active_root(), Some(&p1_root));

        // Switch to p2: the old watcher stops, a new one starts, and the
        // store still tracks exactly one.
        s.switch_project_root(p2.to_str().unwrap());
        assert_eq!(s.watcher_count(), 1, "exactly one watcher after switch");
        let p2_root = proj_root(&s);
        assert_eq!(s.watcher_active_root(), Some(&p2_root), "now watching p2");

        // Let p1's consumer wind down and p2's rescan settle, then baseline.
        tokio::time::sleep(SETTLE * 2).await;
        let baseline = s.watch_bus().current().seq;

        // No cross-project events: a write to p1 must NOT publish (the p1
        // watcher was stopped by the switch) → the sequence is unchanged.
        std::fs::write(p1.join("src/a.rs"), "a2\n").unwrap();
        tokio::time::sleep(SETTLE).await;
        assert_eq!(
            s.watch_bus().current().seq,
            baseline,
            "no event from the old project: {:?}",
            s.watch_bus().current()
        );

        // A write to p2 DOES reach the bus (the new watcher is active).
        std::fs::write(p2.join("src/b.py"), "b2\n").unwrap();
        let change = next_change(s.watch_bus(), baseline, WAIT)
            .await
            .expect("the new project's watcher must be active");
        assert!(
            !change.is_empty(),
            "p2 write must publish: {change:?}"
        );
        s.stop_watcher();
    }
}
