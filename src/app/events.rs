//! Project-change event bus (plan decision #7): a `tokio::sync::watch`
//! channel that carries the latest change summary (which project paths
//! changed, and how). Many subscribers, latest value wins — exactly the
//! semantics the `watch` channel provides, so coalesced/rapid changes
//! collapse to a single value.
//!
//! Subscribers:
//! - `FileView` auto-reload (`AppStore::apply_project_change`, this issue).
//! - git status `refresh()` (issue 07's seam, wired in this issue).
//! - symbol index (issue 05 — incremental refresh on watcher events).
//!
//! Subscription pattern (documented for the later consumers):
//! ```ignore
//! let mut rx = bus.subscribe();          // watch::Receiver<ProjectChange>
//! loop {
//!     rx.changed().await?;               // latest value wins; Err => no publisher
//!     let change = rx.borrow_and_update().clone();
//!     apply(&change);                    // reload / refresh / reindex
//! }
//! ```

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::watch;

/// What happened to a project path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Create,
    Modify,
    Remove,
    Other,
}

/// A summary of project changes since the last publish: the noise-filtered,
/// coalesced changed paths and the kind of change for each. `paths` and
/// `kinds` are parallel (same length, same order).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectChange {
    /// Monotonic sequence number, bumped by each publish, so a subscriber
    /// can tell a fresh value from a stale one. `0` is the initial
    /// (no-change) value.
    pub seq: u64,
    /// Changed (noise-filtered) paths, absolute.
    pub paths: Vec<PathBuf>,
    /// Kinds, parallel to `paths`.
    pub kinds: Vec<ChangeKind>,
}

impl ProjectChange {
    /// True when this summary carries no changes (the initial value, or a
    /// batch that was entirely noise).
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// The project-change bus: a `watch` channel over `ProjectChange`. Cheap to
/// `Clone` (wraps a `watch::Sender` + an `Arc<AtomicU64>`), so the watcher
/// task and every subscriber hold their own handle to the same bus.
#[derive(Clone)]
pub struct ChangeBus {
    tx: watch::Sender<ProjectChange>,
    seq: Arc<AtomicU64>,
}

impl std::fmt::Debug for ChangeBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangeBus").finish()
    }
}

impl ChangeBus {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(ProjectChange::default());
        Self {
            tx,
            seq: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Subscribe: a receiver initialized to the current value. Awaiting
    /// `changed()` fires on the NEXT publish (latest value wins).
    pub fn subscribe(&self) -> watch::Receiver<ProjectChange> {
        self.tx.subscribe()
    }

    /// The current value (synchronous, no update). Used by tests/diagnostics;
    /// the UI consumes the bus via a live [`subscribe`](Self::subscribe) receiver.
    #[allow(dead_code)]
    pub fn current(&self) -> ProjectChange {
        self.tx.borrow().clone()
    }

    /// Publish a change summary: bump the sequence, then send (sync). A
    /// dropped receiver (all subscribers gone) makes this a no-op.
    pub fn publish(&self, mut change: ProjectChange) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        change.seq = seq;
        let _ = self.tx.send(change);
    }
}

impl Default for ChangeBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn initial_value_is_empty_seq_zero() {
        let bus = ChangeBus::new();
        let v = bus.current();
        assert!(v.is_empty());
        assert_eq!(v.seq, 0);
    }

    #[test]
    fn publish_bumps_sequence_and_stores_value() {
        let bus = ChangeBus::new();
        // A live receiver is required for `send` to update the stored value
        // (watch drops the value when every receiver is gone).
        let _rx = bus.subscribe();
        bus.publish(ProjectChange {
            seq: 0,
            paths: vec![PathBuf::from("/p/a.rs")],
            kinds: vec![ChangeKind::Modify],
        });
        bus.publish(ProjectChange {
            seq: 0,
            paths: vec![PathBuf::from("/p/b.rs")],
            kinds: vec![ChangeKind::Create],
        });
        let v = bus.current();
        assert_eq!(v.seq, 2, "two publishes → seq 2");
        assert_eq!(v.paths, vec![PathBuf::from("/p/b.rs")], "latest value wins");
    }

    #[tokio::test]
    async fn latest_value_wins_for_slow_subscriber() {
        // A subscriber that only polls once after two publishes must see the
        // newest value, not the first (watch: last value wins).
        let bus = ChangeBus::new();
        let mut rx = bus.subscribe();
        bus.publish(ProjectChange {
            seq: 0,
            paths: vec![PathBuf::from("/p/one")],
            kinds: vec![],
        });
        bus.publish(ProjectChange {
            seq: 0,
            paths: vec![PathBuf::from("/p/two")],
            kinds: vec![],
        });
        // `changed` fires because a value newer than the initial was sent.
        rx.changed().await.unwrap();
        let v = rx.borrow_and_update().clone();
        assert_eq!(v.paths, vec![PathBuf::from("/p/two")]);
        assert_eq!(v.seq, 2);
    }

    #[tokio::test]
    async fn subscriber_wakes_on_each_publish() {
        let bus = ChangeBus::new();
        let mut rx = bus.subscribe();
        bus.publish(ProjectChange {
            seq: 0,
            paths: vec![PathBuf::from("/p/x")],
            kinds: vec![],
        });
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), rx.changed())
            .await
            .expect("publish must wake the subscriber");
        assert_eq!(rx.borrow_and_update().seq, 1);
    }

    #[tokio::test]
    async fn many_subscribers_each_get_latest() {
        // Every subscriber independently tracks the latest value.
        let bus = ChangeBus::new();
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        bus.publish(ProjectChange {
            seq: 0,
            paths: vec![PathBuf::from("/p/m")],
            kinds: vec![ChangeKind::Modify],
        });
        a.changed().await.unwrap();
        b.changed().await.unwrap();
        assert_eq!(a.borrow_and_update().paths, vec![PathBuf::from("/p/m")]);
        assert_eq!(b.borrow_and_update().paths, vec![PathBuf::from("/p/m")]);
        assert_eq!(a.borrow_and_update().seq, b.borrow_and_update().seq);
    }
}
