//! The background index job's progress plumbing: the shared per-file
//! counter, the coarse progress events, and the result bus the app
//! subscribes to.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::watch;

use super::symbol_index::SymbolIndex;

/// A cheap shared progress counter for a background index job: the number of
/// files parsed so far (`done`) of `total`. The app reads it to render a
/// status-line progress indicator while a job is in flight.
///
/// When constructed with a publisher (bus + generation), each file parse also
/// publishes a coarse progress event to the bus every `PROGRESS_STEP` files
/// (PART A fix, item 2: an honest `indexing N/M` that advances, instead of a
/// frozen `0/N` for the whole build). The FINAL event is still published by
/// the caller after the build completes (unchanged generation contract).
#[derive(Clone, Default, Debug)]
pub struct IndexProgress {
    done: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    bus: Option<IndexBus>,
    generation: usize,
}

/// Publish a progress event every this many parsed files (PART A fix, item 2).
pub const PROGRESS_STEP: usize = 25;

impl IndexProgress {
    pub fn new(total: usize) -> Self {
        Self {
            done: Arc::new(AtomicUsize::new(0)),
            total: Arc::new(AtomicUsize::new(total)),
            bus: None,
            generation: 0,
        }
    }

    /// Attach a result bus + generation so progress is published (every
    /// `PROGRESS_STEP` files) as the build runs. The caller still publishes
    /// the final event itself.
    pub fn with_publisher(mut self, bus: IndexBus, generation: usize) -> Self {
        self.bus = Some(bus);
        self.generation = generation;
        self
    }

    /// Mark one file parsed AND, when a publisher is attached and this file
    /// completes a `PROGRESS_STEP` window, publish a coarse progress event.
    /// `done % PROGRESS_STEP == 0` is hit by exactly one worker (the one that
    /// made `done` reach that value), so no extra synchronization is needed.
    pub fn note_file_done(&self) {
        let done = self.done.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(bus) = &self.bus
            && done.is_multiple_of(PROGRESS_STEP)
        {
            bus.send(IndexEvent {
                index: SymbolIndex::new(), // partial; the app keeps its index on progress
                indexing: true,
                done,
                total: self.total(),
                generation: self.generation,
            });
        }
    }

    pub fn done(&self) -> usize {
        self.done.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> usize {
        self.total.load(Ordering::Relaxed)
    }

    #[allow(dead_code)] // test-only seam (no production caller)
    pub fn finished(&self) -> bool {
        self.done.load(Ordering::Relaxed) >= self.total.load(Ordering::Relaxed)
    }
}

/// A result the background indexer publishes to the app via the [`IndexBus`].
/// `index` is the (possibly incremental) snapshot to install; `indexing`
/// drives the status-line indicator; `done`/`total` are the progress;
/// `generation` is the generation counter at the time the job was started,
/// used to discard events from stale jobs (previous project).
#[derive(Clone, Debug, Default)]
pub struct IndexEvent {
    pub index: SymbolIndex,
    pub indexing: bool,
    pub done: usize,
    pub total: usize,
    pub generation: usize,
}

/// The index-result bus: a `watch` channel over [`IndexEvent`]. The background
/// indexer publishes; the app subscribes (a `use_future`) and installs the
/// newest result into the store.
///
/// **Concurrency contract:** the `watch` channel retains the latest
/// *published* value, not the newest *index*: a job that cloned an older
/// base can publish last and erase another job's file updates (lost update).
/// The store wiring therefore runs **at most one index job per generation**
/// at a time. Changed paths arriving while a job is in flight are accumulated
/// in a pending set and coalesced into one incremental job when the flight
/// clears. Events carrying a stale `generation` (from a previous project's
/// job) are discarded by [`AppStore::apply_index_event`](crate::app::store::AppStore::apply_index_event).
#[derive(Clone, Default)]
pub struct IndexBus {
    tx: watch::Sender<IndexEvent>,
}

impl std::fmt::Debug for IndexBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexBus").finish()
    }
}

impl IndexBus {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(IndexEvent::default());
        Self { tx }
    }

    /// Subscribe: a receiver at the current value. `changed()` fires on the
    /// next publish (latest value wins).
    pub fn subscribe(&self) -> watch::Receiver<IndexEvent> {
        self.tx.subscribe()
    }

    /// Publish an index result (sync; safe from a background thread).
    pub fn send(&self, event: IndexEvent) {
        let _ = self.tx.send(event);
    }
}
