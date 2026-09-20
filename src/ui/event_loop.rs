//! Event-loop input drain-and-coalescing (input-latency issue: queued,
//! repeating cursor motion over a streaming connection).
//!
//! The live loop shape (iocraft `terminal_render_loop`): every terminal
//! event wakes the loop, and the `use_terminal_events` hook drains ALL
//! queued events in one poll pass, invoking the app's callback per
//! event; the loop then renders once per pass. There is no coalescing
//! anywhere, so each queued repeat of a held motion key was APPLIED to
//! the store individually. Over a lossy stream the keyboard repeat rate
//! outruns the drain: the queue keeps replaying cursor motion after the
//! user has released the key.
//!
//! The fix is a drain-and-coalesce policy at the app's event boundary:
//!
//! * Motion keys (pure cursor movement, see [`is_motion_key`]) coalesce
//!   last-wins into one pending slot and are NOT applied on arrival.
//! * State-changing keys (alpha, C-x, M-x, ...) apply individually on
//!   arrival, flushing the pending motion FIRST so event ordering is
//!   preserved.
//! * The component flushes the pending motion on the render tick —
//!   before taking the store snapshot — so a queued burst applies at
//!   most once per drain pass and the last motion key wins.
//!
//! Byte-for-byte rule: a single (non-queued) keypress behaves exactly as
//! before — the coalescer applies it exactly once (on the next frame,
//! which iocraft renders for every event anyway). Only the queued path
//! changes: N queued motion keys apply once per drain pass, and after a
//! "release" at most the one pending motion flushes (the release-drain
//! bound); nothing further replays.
//!
//! Motion membership is judged by key IDENTITY, not by the bound command
//! (bindings differ per view, and char keys such as `j`/`k`/`n` double
//! as text input in editable views and as picker-query input, so char
//! keys are never coalesced). The set: arrows and PageUp/PageDown
//! (unmodified), C-n, C-p, C-v, M-v.

use crate::app::keymap::{KeyCode, Key};

/// A key whose rapid repeated arrival can be safely coalesced: a pure
/// cursor motion that is not also text input and is never a sequence
/// prefix (all of these are single-key bindings, so coalescing one of
/// them can never eat the first key of a later prefix sequence).
pub fn is_motion_key(key: Key) -> bool {
    if key.shift {
        return false; // conservative: never coalesce a shifted combo
    }
    match key.code {
        KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right
        | KeyCode::PageUp
        | KeyCode::PageDown => !key.ctrl && !key.alt,
        KeyCode::Char('n') | KeyCode::Char('p') => key.ctrl && !key.alt,
        // C-v (scroll up) and M-v (scroll down) — but not both.
        KeyCode::Char('v') => key.ctrl ^ key.alt,
        _ => false,
    }
}

/// The drain-and-coalesce pending slot (one `use_ref` per component
/// instance; the render tick and the event callback share the handle).
#[derive(Default)]
pub struct InputCoalescer {
    pending: Option<Key>,
}

impl InputCoalescer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one drained event.
    ///
    /// Motion key: replace the pending slot (last-wins) and return
    /// `false` (nothing applied yet — the render tick flushes it).
    /// Other key: apply the pending motion first (ordering preserved),
    /// then apply the key itself via `apply`, returning `true`.
    /// `apply` is invoked at most twice per call (the flush + the key).
    pub fn drain(&mut self, key: Key, apply: &mut dyn FnMut(Key)) -> bool {
        if is_motion_key(key) {
            self.pending = Some(key);
            false
        } else {
            if let Some(pending) = self.pending.take() {
                apply(pending);
            }
            apply(key);
            true
        }
    }

    /// Flush the pending motion key on the render tick (no-op when the
    /// burst already drained through a state-changing key). `apply` is
    /// invoked at most once.
    pub fn flush(&mut self, apply: &mut dyn FnMut(Key)) {
        if let Some(pending) = self.pending.take() {
            apply(pending);
        }
    }

    /// The pending motion key, if any (test/introspection).
    pub fn pending(&self) -> Option<Key> {
        self.pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn c(c: char) -> Key {
        Key::ctrl_char(c)
    }

    fn m(c: char) -> Key {
        Key::alt_char(c)
    }

    /// A recording apply sink: `rec` holds the applied keys, `count` the
    /// total apply call count (the "expensive passes" the burst must not
    /// multiply).
    struct Recorder {
        keys: Mutex<Vec<Key>>,
        count: Mutex<usize>,
    }

    impl Recorder {
        fn new() -> Self {
            Self {
                keys: Mutex::new(Vec::new()),
                count: Mutex::new(0),
            }
        }
        fn apply(&self, k: Key) {
            self.keys.lock().unwrap().push(k);
            *self.count.lock().unwrap() += 1;
        }
    }

    fn rec_apply(r: &Arc<Recorder>) -> impl FnMut(Key) + '_ {
        move |k| r.apply(k)
    }

    /// The coalesce set is exactly: unmodified arrows + PageUp/PageDown,
    /// C-n, C-p, C-v, M-v.
    #[test]
    fn motion_key_classification() {
        let motion = [
            Key::up(),
            Key::down(),
            Key::new(KeyCode::Left),
            Key::new(KeyCode::Right),
            Key::new(KeyCode::PageUp),
            Key::new(KeyCode::PageDown),
            c('n'),
            c('p'),
            c('v'),
            m('v'),
        ];
        for k in motion {
            assert!(is_motion_key(k), "{k:?} must coalesce");
        }
        let not_motion = [
            Key::char('n'), // char keys are text input (picker query, editor)
            Key::char('j'),
            c('f'), // C-f is not in the coalesce set (word/char motion)
            c('c'), // prefix of C-c sequences — must never coalesce
            Key {
                code: KeyCode::Down,
                shift: true,
                ..Key::new(KeyCode::Down)
            },
            Key::new(KeyCode::Enter),
            Key::new(KeyCode::Escape),
            Key {
                code: KeyCode::Char('v'),
                ctrl: true,
                alt: true,
                ..Key::new(KeyCode::default())
            }, // C-M-v: both modifiers, excluded
        ];
        for k in not_motion {
            assert!(!is_motion_key(k), "{k:?} must NOT coalesce");
        }
    }

    /// REPRO (b): a burst of 100 queued C-n coalesces to ONE apply —
    /// the number of store applications is bounded (1), independent of
    /// the burst size.
    #[test]
    fn burst_motion_keys_coalesce_to_last() {
        let c_n = c('n');
        let mut coalescer = InputCoalescer::new();
        let rec = Arc::new(Recorder::new());
        for _ in 0..100 {
            coalescer.drain(c_n, &mut rec_apply(&rec));
        }
        coalescer.flush(&mut rec_apply(&rec));
        assert_eq!(*rec.keys.lock().unwrap(), vec![c_n], "100 queued C-n → exactly one apply");
        assert_eq!(*rec.count.lock().unwrap(), 1);
        assert_eq!(coalescer.pending(), None, "the slot is drained after the flush");
    }

    /// REPRO (c): release-drain bound — after the burst flushes, idle
    /// render ticks (no new events) apply NOTHING: the queued repeats do
    /// not replay after the simulated release.
    #[test]
    fn release_drain_bounded_to_one_step() {
        let c_n = c('n');
        let mut coalescer = InputCoalescer::new();
        let rec = Arc::new(Recorder::new());
        for _ in 0..64 {
            coalescer.drain(c_n, &mut rec_apply(&rec));
        }
        coalescer.flush(&mut rec_apply(&rec));
        let after_release = *rec.count.lock().unwrap();
        assert_eq!(after_release, 1, "the burst applies exactly once");
        // Idle ticks after the release: nothing queued, nothing replays.
        for _ in 0..10 {
            coalescer.flush(&mut rec_apply(&rec));
        }
        assert_eq!(*rec.count.lock().unwrap(), after_release, "no replay after release");
    }

    /// A state-changing key arriving with a pending motion applies the
    /// motion FIRST (ordering), then the state key — both exactly once.
    #[test]
    fn state_key_flushes_pending_motion_in_order() {
        let c_n = c('n');
        let c_f = c('f');
        let mut coalescer = InputCoalescer::new();
        let rec = Arc::new(Recorder::new());
        coalescer.drain(c_n, &mut rec_apply(&rec));
        coalescer.drain(c_n, &mut rec_apply(&rec)); // last-wins so far
        coalescer.drain(c_f, &mut rec_apply(&rec)); // flush + state key
        coalescer.flush(&mut rec_apply(&rec));
        assert_eq!(
            *rec.keys.lock().unwrap(),
            vec![c_n, c_f],
            "queued motion (coalesced) precedes the state key"
        );
    }

    /// Different queued motion keys: only the LAST survives (last-wins).
    #[test]
    fn mixed_motion_keys_last_wins() {
        let mut coalescer = InputCoalescer::new();
        let rec = Arc::new(Recorder::new());
        coalescer.drain(c('n'), &mut rec_apply(&rec));
        coalescer.drain(Key::down(), &mut rec_apply(&rec));
        coalescer.drain(Key::up(), &mut rec_apply(&rec));
        coalescer.flush(&mut rec_apply(&rec));
        assert_eq!(*rec.keys.lock().unwrap(), vec![Key::up()]);
    }

    /// Byte-for-byte: a single keypress through the coalescer path
    /// applies exactly once — indistinguishable from the direct path.
    #[test]
    fn single_keypress_applies_exactly_once() {
        for key in [c('n'), Key::down(), Key::char('a'), c('x')] {
            let mut coalescer = InputCoalescer::new();
            let rec = Arc::new(Recorder::new());
            coalescer.drain(key, &mut rec_apply(&rec));
            coalescer.flush(&mut rec_apply(&rec));
            assert_eq!(*rec.keys.lock().unwrap(), vec![key], "{key:?} must apply exactly once");
        }
    }
}
