//! Root hook installers: the terminal-event handler, the five async
//! bus-drain futures, and the hardware-cursor effect. Each function
//! makes exactly one `use_*` hook call, unconditionally; `Root` invokes
//! them in a fixed order so iocraft's hook-order contract holds.

use std::sync::{Arc, Mutex};

use iocraft::prelude::*;

use crate::app::store::AppStore;

use super::geometry::click_pane;
use super::input::to_app_key;

/// The `use_terminal_events` handler: routes resize / key / mouse events
/// into the store and bumps the revision tick.
pub(super) fn install_terminal_events(
    hooks: &mut Hooks,
    event_store: Arc<Mutex<AppStore>>,
    mut tick: State<u64>,
) {
    hooks.use_terminal_events(move |event: TerminalEvent| {
        if let TerminalEvent::Resize(_, height) = &event {
            // Update the viewport height on resize (subtract room for
            // the title, help line, and status line).
            let viewport = (*height as usize).saturating_sub(3);
            event_store.lock().unwrap().set_viewport_lines(viewport);
            tick.set(tick.get() + 1);
        }
        if let TerminalEvent::Key(key) = &event
            && key.kind != KeyEventKind::Release
            && let Some(app_key) = to_app_key(key)
        {
            event_store.lock().unwrap().key_event(app_key);
            tick.set(tick.get() + 1);
        }
        // Mouse support (issue 09, step 4: best-effort). Wheel scroll in
        // all list views; click-to-position in the file view (Buffer);
        // click-to-select in the tree sidebar (plan 004 issue 05e: with the
        // tree visible, clicks in the tree's columns select a tree row and
        // clicks in the code pane are shifted by TREE_WIDTH). Limitations:
        // no drag-select, no click in pickers/menus, no click-to-select in
        // list views (v1).
        if let TerminalEvent::FullscreenMouse(mouse) = &event {
            use iocraft::MouseEventKind;
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    event_store.lock().unwrap().mouse_scroll_up();
                    tick.set(tick.get() + 1);
                }
                MouseEventKind::ScrollDown => {
                    event_store.lock().unwrap().mouse_scroll_down();
                    tick.set(tick.get() + 1);
                }
                MouseEventKind::Down(iocraft::MouseButton::Left) => {
                    // Click-to-position: the file view's content area starts
                    // at terminal row 0 (the title is part of the view's
                    // first line). The row is 0-based from the top.
                    // Subtract 1 for the title line offset. The column is a
                    // terminal (display) column, which maps 1:1 to the line's
                    // display column with the tree hidden (the file view
                    // renders from column 0 — no gutter, plan 004 issue 05c)
                    // and is shifted by the tree width when it is visible
                    // (plan 004 issue 05e): a click inside the tree's
                    // columns selects the tree row under it and never moves
                    // the code point.
                    let row = (mouse.row as usize).saturating_sub(1);
                    let mut store = event_store.lock().unwrap();
                    let (in_tree, pane_col) =
                        click_pane(store.tree_visible(), mouse.column as usize);
                    if in_tree {
                        store.tree_click_row(mouse.row as usize);
                    } else {
                        store.mouse_click_position(row, pane_col);
                    }
                    tick.set(tick.get() + 1);
                }
                _ => {}
            }
        }
    });
}

/// The `use_future` drain of the project-change bus (live file watching).
pub(super) fn drain_project_changes(
    hooks: &mut Hooks,
    bus_store: Arc<Mutex<AppStore>>,
    mut tick: State<u64>,
) {
    // Live file watching (issue 04): subscribe to the project-change bus and
    // apply each change to the store (auto-reload non-edited buffers keeping
    // the scroll anchor, set the conflict marker on locally-edited ones,
    // refresh git status). The watch channel is latest-value-wins; `changed`
    // is cancellation-safe, so no change is lost on re-poll.
    let bus_rx = bus_store.lock().unwrap().watch_bus().subscribe();
    hooks.use_future(async move {
        let mut rx = bus_rx;
        // Latest-value-wins: `changed` is cancellation-safe, so no change is
        // lost on re-poll. Loop until the publisher (the store) is dropped.
        while let Ok(()) = rx.changed().await {
            // Coalesce (PART A fix): after each wake, drain ALL
            // immediately-available changes before bumping the tick once —
            // one repaint per burst, not one per event (the churn flashing
            // fix). `borrow_and_update` consumes the latest value; if another
            // publish lands while we apply, `has_changed` catches it.
            loop {
                let change = rx.borrow_and_update().clone();
                bus_store.lock().unwrap().apply_project_change(&change);
                if !rx.has_changed().unwrap_or(false) {
                    break;
                }
            }
            tick.set(tick.get() + 1);
        }
    });
}

/// The `use_future` drain of the symbol-index bus.
pub(super) fn drain_symbol_index(
    hooks: &mut Hooks,
    idx_store: Arc<Mutex<AppStore>>,
    mut tick: State<u64>,
) {
    // Symbol index drain (issue 05): subscribe to the IndexBus and install
    // each result into the store. The concurrency contract runs at most one
    // in-flight index job per generation; changed paths arriving during a
    // flight are accumulated in a pending set and coalesced into one job
    // when the flight clears. Events from a stale generation (previous
    // project) are discarded by `apply_index_event`.
    let idx_rx = idx_store.lock().unwrap().take_index_rx();
    hooks.use_future(async move {
        let mut rx = idx_rx;
        while let Ok(()) = rx.changed().await {
            // Coalesce (PART A fix): drain all immediately-available index
            // events (progress + final) before one repaint.
            loop {
                let event = rx.borrow_and_update().clone();
                idx_store.lock().unwrap().apply_index_event(&event);
                if !rx.has_changed().unwrap_or(false) {
                    break;
                }
            }
            tick.set(tick.get() + 1);
        }
    });
}

/// The `use_future` drain of the tooling-resolve bus (M-. workspace-miss).
pub(super) fn drain_tooling_resolve(
    hooks: &mut Hooks,
    resolve_store: Arc<Mutex<AppStore>>,
    mut tick: State<u64>,
) {
    // Tooling-resolve drain (plan 006 issue 02): the M-. workspace-miss
    // fall-through publishes its result here (a `watch` channel, latest-
    // value-wins, like the index bus). Applying the event lands the jump or
    // reports the miss on the input path — the (slow) provider chain itself
    // already ran off it via `spawn_blocking`.
    let resolve_rx = resolve_store.lock().unwrap().resolve_bus.subscribe();
    hooks.use_future(async move {
        let mut rx = resolve_rx;
        while let Ok(()) = rx.changed().await {
            // Coalesce: drain all immediately-available resolve events before
            // one repaint (a superseded M-. request can burst two sends).
            loop {
                let event = rx.borrow_and_update().clone();
                resolve_store.lock().unwrap().apply_resolve_event(&event);
                if !rx.has_changed().unwrap_or(false) {
                    break;
                }
            }
            tick.set(tick.get() + 1);
        }
    });
}

/// The `use_future` drain of the crate-index bus.
pub(super) fn drain_crate_index(
    hooks: &mut Hooks,
    crate_store: Arc<Mutex<AppStore>>,
    mut tick: State<u64>,
) {
    // Crate-index drain (plan 006 issue 03): the background crate-index
    // builds (registry sources, off the input path) publish their finished
    // indexes here — same `watch` latest-value-wins pattern as the resolve
    // bus above. A crate-index build can only start AFTER a tooling
    // landing, so the drain's subscription (Root start-up) is always live
    // before the first publish (no zero-receiver race).
    let crate_rx = crate_store.lock().unwrap().crate_index_bus.subscribe();
    hooks.use_future(async move {
        let mut rx = crate_rx;
        while let Ok(()) = rx.changed().await {
            // Coalesce: drain all immediately-available crate-index events
            // before one repaint (two crates can land in quick succession).
            loop {
                let event = rx.borrow_and_update().clone();
                crate_store.lock().unwrap().apply_crate_index_event(&event);
                if !rx.has_changed().unwrap_or(false) {
                    break;
                }
            }
            tick.set(tick.get() + 1);
        }
    });
}

/// The `use_future` drain of the search bus (issue 06).
pub(super) fn drain_search(
    hooks: &mut Hooks,
    search_store: Arc<Mutex<AppStore>>,
    mut tick: State<u64>,
) {
    // Search drain (issue 06): take the store's SearchBus receiver out of
    // the store (exactly once; `UnboundedReceiver` is not cloneable, so no
    // subscription is needed) and apply each event to the store. Because
    // this runs as a hook task, each `recv().await` registers the
    // component's waker — every streamed event (first hit, per-file count,
    // the `searching…` → finished transition) wakes the render loop and
    // repaints immediately, without a keypress.
    hooks.use_future(async move {
        let Some(mut rx) = search_store.lock().unwrap().search_rx() else {
            return; // already taken (e.g. by a test)
        };
        while let Some(event) = rx.recv().await {
            search_store.lock().unwrap().apply_search_event(&event);
            // Coalesce (PART A fix): a search streams many events (first hit,
            // per-file counts, finished) in a burst — drain all queued events
            // before bumping the tick once (one repaint per burst).
            while let Ok(event) = rx.try_recv() {
                search_store.lock().unwrap().apply_search_event(&event);
            }
            tick.set(tick.get() + 1);
        }
    });
}

/// The `use_effect` that re-shows + repositions the hardware cursor after
/// every frame.
pub(super) fn install_cursor_effect(
    hooks: &mut Hooks,
    revision: u64,
    cursor_cell_opt: Option<(u16, u16)>,
    cursor_live: bool,
) {
    // issue 004-05 (hardware cursor): iocraft hides the cursor ONCE at startup
    // (?25l), never re-shows it, and re-parks it at the status line after every
    // frame's synchronized output (?2026h ... ?2026l). This effect (which fires
    // after every render) re-shows the cursor and repositions it on the current
    // view's cursor row — the blue-bar (selected) row for list views, the top
    // visible line for the buffer view (emacs -nw parity: the terminal cursor
    // sits on point).
    //
    // The write is deferred to a short-lived task: iocraft's own effect hook
    // fires mid-frame (after ?2026h, before the content draw), so a direct
    // write here would be clobbered by the frame's status-line park (24;1).
    // Deferring ~12 ms (the measured frame flush is ~5 ms) lands the ?25h + CUP
    // AFTER the frame's ?2026l, making it the last cursor position for that
    // frame. Because the effect fires on every render (key, watcher, index,
    // search, or resize), the cursor is re-asserted after every frame. Guarded
    // to the live terminal (and not on quit): the static render path reports
    // size 0 and must not emit raw cursor escapes.
    hooks.use_effect(
        move || {
            if !cursor_live {
                return;
            }
            let cell = cursor_cell_opt;
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(12)).await;
                if let Some((col, row)) = cell {
                    use crossterm::cursor::{MoveTo, Show};
                    let _ = crossterm::execute!(std::io::stdout(), Show, MoveTo(col, row));
                }
            });
        },
        (&revision,),
    );
}
