//! Root hook installers: the terminal-event handler, the five async
//! bus-drain futures, the jump-landing-highlight animation driver (the
//! first time-driven re-render; self-stopping), and the hardware-cursor
//! effect. Each function
//! makes exactly one `use_*` hook call, unconditionally; `Root` invokes
//! them in a fixed order so iocraft's hook-order contract holds.

use std::sync::{Arc, Mutex};

use iocraft::prelude::*;
use tokio::sync::mpsc;

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
        // clicks in the code pane are shifted by TREE_WIDTH);
        // left drag-select in the file pane (issue-clipboard-and-selection:
        // the app captures the mouse, so the terminal's own
        // drag-to-select is unavailable — the drag is driven in-app).
        // Limitations: no click in pickers/menus, no click-to-select in
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
                    // the code point. issue-clipboard-and-selection: the
                    // press ALSO arms drag-select (mouse_drag_begin) so a
                    // subsequent left DRAG can grow a region from it — a
                    // press in the tree disarms instead (a tree drag must
                    // never create a code region).
                    let row = (mouse.row as usize).saturating_sub(1);
                    let mut store = event_store.lock().unwrap();
                    let (in_tree, pane_col) =
                        click_pane(store.tree_visible(), mouse.column as usize);
                    if in_tree {
                        store.mouse_drag_end();
                        store.tree_click_row(mouse.row as usize);
                    } else {
                        store.mouse_click_position(row, pane_col);
                        store.mouse_drag_begin(row);
                    }
                    tick.set(tick.get() + 1);
                }
                // issue-clipboard-and-selection (part 2): the left drag is
                // the redline-owned selection. The terminal only offers its
                // own drag-to-select when the app is NOT capturing the
                // mouse — and this app IS capturing it (that is how the
                // clicks above work), so the drag must be driven in-app:
                // each DRAG event rebuilds the whole-line region from the
                // press line (the mark) to the drag line (the point); the
                // existing `region_lines` face paints it, and M-w / C-w act
                // on exactly those lines. A press in the tree or with the
                // picker open never armed, so a drag there is a no-op.
                //
                // Known limitation (P3-2, disclosed): a drag that STARTS in
                // the file pane and WANDERS over the tree columns still
                // grows the region — the Drag arm ignores the column (only
                // the Down arm consults `click_pane` to route tree clicks).
                // The region follows the rendered row under the pointer
                // (via `click_target_line`), so a drag into the tree's
                // columns maps to the same buffer row as the file pane at
                // that terminal row. Benign: the region is still bounded
                // by the buffer's lines, and the user can always re-drag
                // or re-click to correct it. Not clamped: the tree's
                // columns are narrow and the drag is a transient gesture.
                MouseEventKind::Drag(iocraft::MouseButton::Left) => {
                    let row = (mouse.row as usize).saturating_sub(1);
                    event_store.lock().unwrap().mouse_drag_position(row);
                    tick.set(tick.get() + 1);
                }
                // The release disarms the drag tracking; the region itself
                // (mark + point) PERSISTS (the emacs mark survives
                // mouse-up) so M-w copies exactly what the drag
                // highlighted. No tick: nothing repaints on release.
                MouseEventKind::Up(iocraft::MouseButton::Left) => {
                    event_store.lock().unwrap().mouse_drag_end();
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

/// The animation step (jump-highlight), extracted so the termination
/// rule is unit-testable WITHOUT iocraft hooks or a render loop:
/// await one wake (a landing highlight was set — the ONLY sender is
/// `record_landing_highlight`, so "no highlight -> no timer" holds at the
/// source: while idle this sits in `recv().await`, zero ticks, zero CPU),
/// then bump `tick` at most once per `JUMP_HIGHLIGHT_FRAME` until the
/// store's highlight has elapsed `JUMP_HIGHLIGHT_DURATION` — that final
/// bump paints the cleared frame — and stop (no further ticks, ever; a
/// jump costs at most `ceil(DURATION / FRAME) + 1` ticks). A highlight
/// CLEARED mid-fade stops it early (the store is re-read each frame);
/// a jump that REPLACES the highlight mid-fade just extends the window
/// (the latest `set_at` wins; the capacity-1 wake channel coalesces the
/// burst).
pub(super) async fn jump_animation_loop(
    store: Arc<Mutex<AppStore>>,
    rx: &mut mpsc::Receiver<()>,
    mut tick: impl FnMut(),
) {
    use crate::app::store::{JUMP_HIGHLIGHT_DURATION, JUMP_HIGHLIGHT_FRAME};
    use std::time::Instant;
    while rx.recv().await.is_some() {
        // Coalesce any pending wakes (rapid consecutive jumps).
        while rx.try_recv().is_ok() {}
        loop {
            let deadline = {
                let s = store.lock().unwrap();
                s.jump_highlight()
                    .map(|h| h.set_at + JUMP_HIGHLIGHT_DURATION)
            };
            let Some(deadline) = deadline else {
                break; // cleared by the next command: stop, no tick
            };
            let now = Instant::now();
            tick();
            if now >= deadline {
                // This tick paints the cleared frame; then nothing, ever.
                break;
            }
            tokio::time::sleep(JUMP_HIGHLIGHT_FRAME.min(deadline - now)).await;
        }
    }
}

/// The `use_future` driver of the jump-landing-highlight fade
/// (jump-highlight): the app's first time-driven re-render, the iocraft
/// wrapper around [`jump_animation_loop`] (the wake receiver is taken out
/// of the store exactly once — the issue-04/05 bus precedent; a second
/// install, e.g. on the static render path, gets `None` and does nothing).
pub(super) fn drive_jump_highlight(
    hooks: &mut Hooks,
    anim_store: Arc<Mutex<AppStore>>,
    mut tick: State<u64>,
) {
    hooks.use_future(async move {
        let Some(mut rx) = anim_store.lock().unwrap().take_jump_wake_rx() else {
            return; // already taken (e.g. by a test)
        };
        jump_animation_loop(anim_store, &mut rx, move || tick.set(tick.get() + 1)).await;
    });
}

/// The OSC 52 clipboard drain (issue-clipboard-and-selection part 1):
/// `copy_region` publishes the finished escape to the store's clipboard
/// bus; this task writes it to stdout. The escape is self-contained and
/// cursor-neutral (it ends in BEL and parks nothing), and each write is a
/// single locked stdout operation. It cannot split one of iocraft's own
/// frame writes (each iocraft frame is a single locked write); it may
/// land *between* iocraft's frame writes, which is harmless — the escape
/// is self-delimiting (OSC … BEL) and cursor-neutral, so a terminal that
/// honours it applies it atomically and the next frame repaints the same
/// cells. No tick on write: the copy already repainted (the message
/// line), the escape itself changes nothing on screen.
pub(super) fn drain_clipboard(hooks: &mut Hooks, store: Arc<Mutex<AppStore>>) {
    hooks.use_future(async move {
        let Some(mut rx) = store.lock().unwrap().take_clipboard_rx() else {
            return; // already taken (e.g. by a test)
        };
        while let Some(escape) = rx.recv().await {
            let _ = crossterm::execute!(std::io::stdout(), crossterm::style::Print(escape));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::store::{JUMP_HIGHLIGHT_DURATION, JUMP_HIGHLIGHT_FRAME};
    use std::time::Duration;

    /// A store with an open `src/a.rs` and a synchronous symbol index,
    /// the point parked on the `alpha` REFERENCE (line 2, col 5) — so an
    /// `M-.` keypress lands the unique definition (line 0) and the landing
    /// hook sets the highlight + wakes the driver, all through the PUBLIC
    /// API (key events + keymap, exactly as the live loop drives them).
    fn store_with_pending_jump() -> AppStore {
        use crate::app::keymap::parse_key;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/a.rs"),
            "fn alpha() {}\nfn beta() {\n    alpha();\n}\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/a.rs");
        // Build the symbol index synchronously (the store tests' shape).
        let files_list = crate::model::files::FileList::build(dir.path()).unwrap();
        s.set_index(crate::nav::index::build_index(dir.path(), &files_list.files, None));
        // Point (0,0) -> the reference at (2,5) by public motion keys.
        for _ in 0..2 {
            s.key_event(parse_key("C-n").unwrap());
        }
        for _ in 0..5 {
            s.key_event(parse_key("C-f").unwrap());
        }
        s
    }

    /// jump-highlight (spec h): the animation driver is SELF-STOPPING —
    /// (1) no highlight set, no timer: the loop idles with ZERO ticks;
    /// (2) a jump sets the highlight and wakes it: a BOUNDED burst of
    /// ticks (at most `ceil(DURATION/FRAME) + 1`), then (3) NO further
    /// ticks, ever — the fade is done and nothing keeps spinning.
    #[tokio::test]
    async fn jump_animation_idle_has_no_ticks_and_terminates_after_fade() {
        let store = Arc::new(Mutex::new(store_with_pending_jump()));
        let rx = store.lock().unwrap().take_jump_wake_rx().expect("wake receiver");
        let (tick_tx, mut tick_rx) = mpsc::unbounded_channel::<()>();
        let wake_tx = tick_tx.clone();
        let task = tokio::spawn({
            let mut rx = rx;
            let store = store.clone();
            async move { jump_animation_loop(store, &mut rx, move || { let _ = wake_tx.send(()); }).await }
        });

        // (1) No highlight set -> no timer: 150 ms of idling costs zero
        // ticks (a spinning 30 fps loop would have ticked ~4 times).
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            tick_rx.len(),
            0,
            "no highlight set: the driver must not tick while idle"
        );

        // A jump (M-.) sets the landing highlight + wakes the driver.
        store
            .lock()
            .unwrap()
            .key_event(crate::app::keymap::parse_key("M-.").unwrap());
        assert!(
            store.lock().unwrap().jump_highlight().is_some(),
            "the M-. landing set the highlight"
        );

        // (2) Collect the tick burst: it must finish (a quiet 100 ms
        // after the last tick, well inside the 600 ms cap).
        let mut ticks = 0usize;
        let mut last_tick = std::time::Instant::now();
        let cap = std::time::Instant::now() + Duration::from_millis(600);
        loop {
            match tick_rx.try_recv() {
                Ok(()) => {
                    ticks += 1;
                    last_tick = std::time::Instant::now();
                }
                Err(_) => {
                    if ticks > 0 && last_tick.elapsed() >= Duration::from_millis(100) {
                        break;
                    }
                    if std::time::Instant::now() >= cap {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
        }
        let bound =
            (JUMP_HIGHLIGHT_DURATION.as_millis() / JUMP_HIGHLIGHT_FRAME.as_millis()) as usize;
        assert!(
            ticks >= 3,
            "the fade must animate (at least one frame tick + the clear tick), got {ticks}"
        );
        assert!(
            ticks <= bound + 2,
            "the fade must be BOUNDED (at most {bound} + 2 ticks), got {ticks}"
        );

        // (3) Termination: once the fade has completed, NO further ticks —
        // a leftover 30 fps loop would keep ticking forever (the power
        // burn the spec forbids).
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            tick_rx.len(),
            0,
            "no tick bumps after the animation completed (self-stopping)"
        );

        task.abort();
    }
}
