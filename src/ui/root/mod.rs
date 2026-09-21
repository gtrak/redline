//! Root component: renders the top-of-stack view, the picker overlay
//! (when open), the minibuffer line, and the status line. Converts
//! iocraft terminal key events into app `Key` presses fed to the
//! keymap engine against the store (which lives in the element
//! context, set up by `main`).

mod geometry;
mod input;
mod render;
mod snapshot;
mod widgets;

#[cfg(test)]
pub use render::render_at_width;

use std::sync::{Arc, Mutex};

use iocraft::prelude::*;

use crate::app::store::{AppStore, ViewId};
use crate::ui::blame_view::BlameView;
use crate::ui::commit_editor::CommitEditorView;
use crate::ui::file_view::FileView;
use crate::ui::home_view::HomeView;
use crate::ui::log_view::LogView;
use crate::ui::magit_status::MagitStatusView;
use crate::ui::picker::Picker;
use crate::ui::results_view::ResultsView;
use crate::ui::rows_view::MagitRowsView;
use crate::ui::transient_menu::TransientMenuView;
use crate::ui::tree::TreeSidebar;
use crate::ui::views::buffer::BufferListView;

use geometry::click_pane;
use geometry::cursor_cell;
use input::to_app_key;
use render::StaticRenderWidth;
use snapshot::Snapshot;
use widgets::{Minibuffer, StatusLine};

#[component]
pub fn Root(mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    // The store lives in the element context as `Arc<Mutex<AppStore>>`:
    // the event closure must be Send+Sync, and a plain context RefMut
    // handle is not.
    let store_handle = hooks.use_context::<Arc<Mutex<AppStore>>>();
    let store: Arc<Mutex<AppStore>> = (*store_handle).clone();
    let mut system = hooks.use_context_mut::<SystemContext>();
    // Terminal dimensions (drives the root View's width + height so the pane
    // fills the screen; also re-renders on resize). In the static render path
    // (tests) the size is 0; fall back to 80x24. Both are set explicitly
    // (PART A fix: the root View previously set only `height`, so the pane
    // was content-sized horizontally and did not fill the terminal).
    //
    // The width is only set when the runtime reports a real terminal size
    // (`tw_raw > 0`). In the static `.to_string()` render path the terminal
    // size is 0 and iocraft renders at its own (narrower) default width, so
    // forcing an 80-wide root there would clip/garble the layout — leave the
    // width unset there (content-sized) so the static tests keep working.
    let (tw_raw, term_h_raw) = hooks.use_terminal_size();
    // loop-03: the static render helper may pin the root to a terminal width
    // (the live contract) instead of content-sized — see `render_at_width`.
    let static_width = hooks.try_use_context::<StaticRenderWidth>().map(|w| w.0);
    use iocraft::Size;
    let term_w: Size = if tw_raw > 0 {
        Size::Length(tw_raw as u32)
    } else if let Some(w) = static_width {
        Size::Length(w as u32)
    } else {
        Size::Auto // plain static render path: content-sized (no terminal width)
    };
    let term_h: u32 = (term_h_raw as u32).max(24);

    // Revision tick: bumping this State after every store mutation (event
    // handler, resize, or async bus drain) forces iocraft to re-render and
    // re-read the store snapshot. Without it, store mutations are invisible
    // to the render loop (iocraft only re-renders when tracked State changes
    // or a hook future wakes AND a State is set in that future).
    let mut tick = hooks.use_state(|| 0u64);

    // Clone for the event closure (it must be Send); keep `store` for the
    // render snapshot below.
    let event_store = store.clone();
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

    // Live file watching (issue 04): subscribe to the project-change bus and
    // apply each change to the store (auto-reload non-edited buffers keeping
    // the scroll anchor, set the conflict marker on locally-edited ones,
    // refresh git status). The watch channel is latest-value-wins; `changed`
    // is cancellation-safe, so no change is lost on re-poll.
    let bus_store = store.clone();
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

    // Symbol index drain (issue 05): subscribe to the IndexBus and install
    // each result into the store. The concurrency contract runs at most one
    // in-flight index job per generation; changed paths arriving during a
    // flight are accumulated in a pending set and coalesced into one job
    // when the flight clears. Events from a stale generation (previous
    // project) are discarded by `apply_index_event`.
    let idx_store = store.clone();
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

    // Tooling-resolve drain (plan 006 issue 02): the M-. workspace-miss
    // fall-through publishes its result here (a `watch` channel, latest-
    // value-wins, like the index bus). Applying the event lands the jump or
    // reports the miss on the input path — the (slow) provider chain itself
    // already ran off it via `spawn_blocking`.
    let resolve_store = store.clone();
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

    // Crate-index drain (plan 006 issue 03): the background crate-index
    // builds (registry sources, off the input path) publish their finished
    // indexes here — same `watch` latest-value-wins pattern as the resolve
    // bus above. A crate-index build can only start AFTER a tooling
    // landing, so the drain's subscription (Root start-up) is always live
    // before the first publish (no zero-receiver race).
    let crate_store = store.clone();
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

    // Search drain (issue 06): take the store's SearchBus receiver out of
    // the store (exactly once; `UnboundedReceiver` is not cloneable, so no
    // subscription is needed) and apply each event to the store. Because
    // this runs as a hook task, each `recv().await` registers the
    // component's waker — every streamed event (first hit, per-file count,
    // the `searching…` → finished transition) wakes the render loop and
    // repaints immediately, without a keypress.
    let search_store = store.clone();
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

    let snap = {
        // Establish the render's dependency on the revision tick: iocraft
        // re-renders when a State read during the previous render changes.
        // Without this read, tick bumps from the event handler / bus drains
        // are invisible (written but never observed) and the UI stays on its
        // first frame.
        let _revision = tick.get();
        let mut s = store.lock().unwrap();
        let (top_line, total_lines, viewport_lines) = s.file_view_scroll_info();
        let (search_rows, search_top_row, search_total_rows, search_selected_row) =
            s.search_view_info();
        let (magit_rows, magit_top_row, magit_total_rows) = s.magit_view_info();
        // Issue 003-02 shared windowing: the log / blame / commit-diff panes
        // render their pre-computed visible window (the store keeps the
        // cursor row in view; paging resets the log window).
        let (log_rows, _log_top, _log_total) = s.log_view_info();
        let (blame_rows, _blame_top, _blame_total) = s.blame_view_info();
        let (commit_diff_rows, commit_diff_top_row, commit_diff_total_rows) =
            s.commit_diff_view_info();
        Snapshot {
            quit: s.quit,
            project: s.project_display().to_string(),
            view: s.render_view(),
            view_name: s.view_name_display(),
            pending: s.pending_display(),
            activity: s.activity_display(),
            message: s.message.clone(),
            home_title: s.home_title(),
            home_rows: s.home_body_rows(),
            buffer_rows: s.buffer_rows(),
            buffer_list_selected: s.buffer_list_selected(),
            picker: s.picker_open(),
            prompt: s.picker_prompt().to_string(),
            query: s.picker_query().to_string(),
            selected: s.picker_selected(),
            candidates: s
                .picker_filtered()
                .iter()
                .map(|(c, _)| c.clone())
                .collect(),
            total: s.picker_count().1,
            preview: s.picker_preview().to_string(),
            magit_rows,
            magit_top_row,
            magit_total_rows,
            menu_open: s.menu_open(),
            menu_rows: s.menu_rows(),
            menu_height: s.menu_height(),
            log_title: s.log_title(),
            log_rows,
            blame_title: s.blame_title(),
            blame_rows,
            commit_diff_title: s.commit_diff_title(),
            commit_diff_rows,
            commit_diff_top_row,
            commit_diff_total_rows,
            commit_editor_title: s.commit_editor_title(),
            commit_editor_rows: s.commit_editor_rows(),
            dirty: s.dirty_counts(),
            file_view_rows: s.file_view_rows(),
            file_view_total_rows: s.file_view_total_rows(),
            file_view_title: s.view_name_display(),
            file_view_top_line: top_line,
            file_view_total_lines: total_lines,
            file_view_viewport_lines: viewport_lines,
            file_view_point_line: s.file_view_point().0,
            file_view_point_col: s.file_view_point().1,
            file_view_changed_on_disk: s.current_buffer_changed_on_disk(),
            file_view_current_buffer_editable: s.current_buffer_editable(),
            buffer_mode: s.buffer_mode_display(),
            tree_visible: s.tree_visible(),
            tree_rows: s.tree_rows(),
            tree_selected: s.tree_selected(),
            terminal_width: tw_raw,
            which_function: s.which_function(),
            indexing: s.indexing_display(),
            resolving: s.resolving_display(),
            crate_indexing: s.crate_indexing_display(),
            search_title: s.search_title(),
            search_rows,
            search_top_row,
            search_total_rows,
            search_selected_row,
            search_running: s.search_running(),
            search_error: s.search_error(),
            searching: s.search_display(),
            position: s.file_view_position_display(),
            annotations: s.annotation_count_display(),
            region_lines: s.region_line_range(),
            region_size: s.region_size_bytes(),
        }
    };

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
    let revision = tick.get();
    let cursor_cell_opt = cursor_cell(&snap);
    let cursor_live = tw_raw > 0 && !snap.quit;
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

    if snap.quit {
        system.exit();
    }

    let main_view: Option<AnyElement<'static>> = match snap.view {
        ViewId::Home => Some(element! {
            HomeView(
                title: snap.home_title.clone(),
                rows: snap.home_rows.clone(),
                help: "C-x C-c quit · ? menu".to_string(),
            )
        }
        .into()),
        ViewId::Buffer => Some(element! {
            FileView(
                title: snap.file_view_title.clone(),
                rows: snap.file_view_rows.clone(),
                total_rows: snap.file_view_total_rows,
                top_line: snap.file_view_top_line,
                viewport_lines: snap.file_view_viewport_lines,
                changed_on_disk: snap.file_view_changed_on_disk,
                buffer_editable: snap.file_view_current_buffer_editable,
                region_lines: snap.region_lines,
            )
        }
        .into()),
        ViewId::BufferList => Some(element! {
            BufferListView(
                rows: snap.buffer_rows.clone(),
                selected: snap.buffer_list_selected,
            )
        }
        .into()),
        ViewId::MagitStatus => Some(element! {
            MagitStatusView(
                rows: snap.magit_rows.clone(),
                top_row: snap.magit_top_row,
                total_rows: snap.magit_total_rows,
            )
        }
        .into()),
        ViewId::Log => Some(element! {
            LogView(title: snap.log_title.clone(), rows: snap.log_rows.clone())
        }
        .into()),
        ViewId::Blame => Some(element! {
            BlameView(title: snap.blame_title.clone(), rows: snap.blame_rows.clone())
        }
        .into()),
        ViewId::CommitDiff => Some(element! {
            MagitRowsView(
                title: snap.commit_diff_title.clone(),
                rows: snap.commit_diff_rows.clone(),
                top_row: snap.commit_diff_top_row,
                total_rows: snap.commit_diff_total_rows,
                help: "commit diff (read-only) · C-n/C-p · C-v/M-v · M->/M-< · q back".to_string(),
            )
        }
        .into()),
        ViewId::CommitEditor => Some(element! {
            CommitEditorView(
                title: snap.commit_editor_title.clone(),
                rows: snap.commit_editor_rows.clone(),
            )
        }
        .into()),
        ViewId::Search => Some(element! {
            ResultsView(
                title: snap.search_title.clone(),
                rows: snap.search_rows.clone(),
                top_row: snap.search_top_row,
                total_rows: snap.search_total_rows,
                selected_row: snap.search_selected_row,
                running: snap.search_running,
                error: snap.search_error.clone(),
            )
        }
        .into()),
    };

    element! {
        View(flex_direction: FlexDirection::Column, width: term_w, height: term_h) {
            // The row and the inner column pin their width to the root's
            // resolved width (100% each). Without this, a content-wide child
            // (the home view's NoWrap command rows, which can run well past
            // 80 cols) pins the column's width to its own content width and
            // full-width children (the picker's right-aligned count line)
            // render off-screen. In the static render path (root width Auto)
            // 100% resolves to the same content width as before — no change.
            View(flex_direction: FlexDirection::Row, flex_grow: 1.0f32, width: iocraft::Size::Percent(100.0)) {
                #(if snap.tree_visible {
                    Some(element! {
                        TreeSidebar(
                            rows: snap.tree_rows.clone(),
                            selected: snap.tree_selected,
                        )
                    })
                } else {
                    None
                })
                View(flex_direction: FlexDirection::Column, flex_grow: 1.0f32, width: iocraft::Size::Percent(100.0)) {
                    #(main_view)
                    #(if snap.picker {
                        Some(element! {
                            Picker(
                                prompt: snap.prompt,
                                query: snap.query,
                                selected: snap.selected,
                                candidates: snap.candidates.clone(),
                                total: snap.total,
                                preview: snap.preview,
                                viewport: snap.file_view_viewport_lines as u32,
                            )
                        })
                    } else {
                        None
                    })
                    #(if snap.menu_open && !snap.picker {
                        Some(element! {
                            TransientMenuView(
                                rows: snap.menu_rows.clone(),
                                height: snap.menu_height,
                            )
                        })
                    } else {
                        None
                    })
                }
            }
            Minibuffer(message: snap.message)
            StatusLine(
                project: snap.project,
                view: snap.view_name,
                mode: snap.buffer_mode,
                pending: snap.pending,
                activity: snap.activity,
                dirty: snap.dirty,
                which_function: snap.which_function,
                indexing: snap.indexing,
                resolving: snap.resolving,
                crate_indexing: snap.crate_indexing,
                searching: snap.searching,
                position: snap.position,
                annotations: snap.annotations,
                region_size: snap.region_size,
            )
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// A store rooted in a temp project (never touches the real cache
    /// dir; the persistence base is a throwaway sibling so the walk
    /// never sees the persistence files).
    fn store(dir: &std::path::Path) -> AppStore {
        let base = tempfile::tempdir().unwrap();
        AppStore::at(dir, base.path().to_path_buf())
    }

    /// Render one frame from a store (static render, no event loop).
    ///
    /// The issue-01 constraint forbids adding dependencies, and
    /// `mock_terminal_render_loop` needs a `futures_core::stream::Stream`
    /// of events plus `StreamExt` on the frames — neither is nameable from
    /// this crate (iocraft does not re-export them). So keypresses are fed
    /// to the store directly (the exact `Key` the component's event path
    /// would produce, verified by `key_event_conversion` in `input.rs`) and the
    /// resulting frame is rendered statically.
    fn render_frame(store: AppStore) -> String {
        let mut app = element! {
            ContextProvider(value: Context::owned(Arc::new(Mutex::new(store)))) {
                Root
            }
        };
        app.to_string()
    }

    /// Static-render mirror of the PTY matrix terminal (80x24): the resize
    /// handler sets `viewport_lines = height - 3 = 21`, so the static
    /// fixture stores match it (the home body is bounded to this viewport,
    /// exactly as live 24-row terminals are).
    fn pty_store(dir: &std::path::Path) -> AppStore {
        let mut s = store(dir);
        s.set_viewport_lines(21);
        s
    }

    fn project_with_files(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.join("README.md"), "# readme\n").unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
    }

    /// The initial frame shows the HOME view (plan 004 issue 06a: the
    /// buffer table starts empty — no auto-created scratch), the status
    /// line (project name + view name), and the ready-state minibuffer.
    #[test]
    fn root_initial_frame_renders_home() {
        let dir = tempfile::tempdir().unwrap();
        let store = pty_store(dir.path());
        let s = render_frame(store);
        assert!(s.contains("redline"), "home header missing: {s:?}");
        assert!(s.contains("C-x C-c quit"), "home help line missing: {s:?}");
        assert!(s.contains("ready"), "{s:?}");
        assert!(!s.contains("*scratch*"), "no auto-created scratch at boot: {s:?}");
    }

    /// The status line shows the detected project name (not the old
    /// "no project" placeholder) when rooted in a project.
    #[test]
    fn status_line_shows_project_name() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let name = dir.path().file_name().unwrap().to_string_lossy().into_owned();
        let s = render_frame(pty_store(dir.path()));
        assert!(s.contains(&format!("* {name} *")), "project name missing:\n{s}");
    }

    /// plan 005 issue 01: the status line shows the current buffer's mode
    /// word — `Read-only` for a file buffer, `Edit` after `toggle-read-only`
    /// (and `Edit` on scratch, which is always editable).
    #[test]
    fn status_line_shows_buffer_mode() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let s1 = store(dir.path());
        {
            let mut s = s1;
            s.open_path("src/main.rs");
            let rendered = render_frame(s);
            assert!(
                rendered.contains("Read-only"),
                "file buffer must show Read-only:\n{rendered}"
            );
        }

        // C-x C-q flips the file buffer into edit mode.
        let mut store2 = store(dir.path());
        store2.open_path("src/main.rs");
        store2.key_event(crate::app::keymap::Key::ctrl_char('x'));
        store2.key_event(crate::app::keymap::Key::ctrl_char('q'));
        let s2 = render_frame(store2);
        assert!(
            s2.contains("Edit") && !s2.contains("Read-only"),
            "edit mode must show Edit, not Read-only:\n{s2}"
        );
    }

    /// loop-03 regression pin: the pre-df95113 off-screen count-line shape.
    /// A NoWrap home body row wider than the 80-col terminal (the 118-col
    /// content that drove the original failure) with the find-file picker
    /// open. Pre-df95113 the overlay column resolved to the home content
    /// width, so the picker's right-aligned 'N of M' count line landed past
    /// col 80 and a width-80 render clipped it away entirely — this test
    /// FAILS on that shape. (The width pin is in root/mod.rs/home_view.rs;
    /// the fixture only widens the content, it does not re-introduce the
    /// bug.)
    #[test]
    fn render_at_width_catches_offscreen_picker_count_line() {
        use crate::app::command::Command;
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = pty_store(dir.path());
        // A home body row ~118 cols wide: `C-c p z` + 2 + 110-char docs.
        // Category `0wide` sorts first so the row survives the viewport
        // budget with the picker open (8 body rows: 21 - 1 - 12).
        let wide_docs = "x".repeat(110);
        let docs: &'static str = Box::leak(wide_docs.into_boxed_str());
        store
            .registry
            .register(Command::new("wide-doc-cmd", docs, "0wide", |_s, _a| {}));
        match store
            .engine
            .global
            .bind(
                &[
                    crate::app::keymap::parse_key("C-c").unwrap(),
                    crate::app::keymap::parse_key("p").unwrap(),
                    crate::app::keymap::parse_key("z").unwrap(),
                ],
                "wide-doc-cmd",
            ) {
            Ok(()) => {}
            Err(e) => panic!("bind the wide-doc command: {e}"),
        }
        store.open_find_file();

        let frame = render_at_width(store, 80);
        // The picker's count line ('N of M', right-aligned) must be ON the
        // 80-col screen: a row matching the PTY count-line shape.
        let count_line = frame
            .lines()
            .find(|l| {
                let t = l.trim();
                t.split_once(" of ")
                    .map(|(n, m)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())
                        && !m.is_empty() && m.chars().all(|c| c.is_ascii_digit()))
                    .unwrap_or(false)
            })
            .unwrap_or_else(|| {
                panic!("picker count line 'N of M' must be on-screen at width 80:\n{frame}")
            });
        // It sits right-aligned inside the window (the pre-fix shape had it
        // at col 109+, i.e. clipped from the 80-col canvas entirely).
        let last = count_line.trim_end().chars().count();
        assert!(
            last <= 80,
            "count line extends past the 80-col window: col {last}: {count_line:?}"
        );
    }

    /// M-x palette: the prompt + typed query, the surviving nucleo
    /// candidate, and the picker's count line are all in the frame.
    #[test]
    fn root_palette_renders_prompt_query_and_count() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = pty_store(dir.path());
        store.key_event(crate::app::keymap::Key::alt_char('x'));
        store.key_event(crate::app::keymap::Key::char('q'));
        store.key_event(crate::app::keymap::Key::char('u'));
        let s = render_frame(store);
        assert!(s.contains("M-x qu"), "palette prompt+query missing:\n{s}");
        assert!(s.contains("quit"), "filtered candidate missing:\n{s}");
        assert!(s.contains("of 108"), "picker count line missing:\n{s}");
        // "qu" filters out the other seed commands.
        assert!(!s.contains("insert-demo-text"), "{s}");
    }

    /// Find-file picker: prompt, candidate, count, and the preview pane
    /// showing the selected file's first page (plain text).
    #[test]
    fn root_find_file_renders_preview() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = pty_store(dir.path());
        store.open_find_file();
        // Select src/main.rs so its contents preview.
        store.key_event(crate::app::keymap::Key::char('m'));
        let s = render_frame(store);
        assert!(s.contains("Find file: m"), "prompt+query missing:\n{s}");
        assert!(s.contains("src/main.rs"), "candidate missing:\n{s}");
        assert!(s.contains("fn main"), "preview missing:\n{s}");
    }

    // ── issue picker-density: render80 twins for the new layout ─────────

    /// B: the picker canvas is sized to content + viewport, not a fixed
    /// 12 rows. 3 candidates => prompt + 3 rows + count = a 5-row box.
    #[test]
    fn picker_canvas_sized_to_content() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path()); // Cargo.toml, README.md, src/main.rs => 3 files
        let mut store = pty_store(dir.path());
        store.open_find_file();
        assert_eq!(store.picker_count().0, 3, "3 file candidates");
        let frame = render_at_width(store, 80);
        let lines: Vec<&str> = frame.lines().collect();
        let prompt_row = lines
            .iter()
            .position(|l| l.contains("Find file:"))
            .expect("picker prompt row");
        let count_row = lines
            .iter()
            .position(|l| l.trim() == "3 of 3")
            .unwrap_or_else(|| panic!("count line '3 of 3' missing:\n{frame}"));
        assert_eq!(
            count_row - prompt_row,
            4,
            "5-row box (prompt + 3 + count) for 3 candidates:\n{frame}"
        );
    }

    /// A: name-first rows — the symbol NAME sits left and OWNS the space; the
    /// `[kind] path` detail is right-aligned and is what truncates. Asserts
    /// the store's structured fields AND that the 80-col frame renders a name
    /// LONGER than the old 32-cell label budget in full.
    #[test]
    fn symbol_picker_name_first_not_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::create_dir_all(p.join("src/very/deep/nested/directory/path")).unwrap();
        std::fs::write(p.join("Cargo.toml"), "[package]\n").unwrap();
        let long = "src/very/deep/nested/directory/path/mod.rs";
        // A name LONGER than the old 32-cell label budget (62% of the 53-cell
        // candidate column at 80 cols with a preview): it must render in full.
        let name = "alpha_symbol_name_that_is_really_quite_long";
        std::fs::write(p.join(long), format!("fn {name}() {{}}\n")).unwrap();
        let mut store = pty_store(p);
        let files_list = crate::model::files::FileList::build(p).unwrap();
        store.set_index(crate::nav::index::build_index(p, &files_list.files, None));
        store.open_symbol_picker();
        assert!(store.picker_open(), "symbol picker open");
        let cand = store
            .picker_filtered()
            .iter()
            .find(|(c, _)| c.label == name)
            .expect("candidate with the symbol name as its label");
        // Name-first: label = the name, detail = `[fn] <long path>`.
        assert_eq!(cand.0.label, name);
        assert_eq!(cand.0.detail, format!("[fn] {long}"), "right-aligned detail");
        assert!(
            crate::model::text_width::display_width(name) > 32,
            "name must exceed the old 32-cell label budget: len {}",
            crate::model::text_width::display_width(name)
        );
        // The 80-col frame renders the FULL name at the left (the old code
        // left-truncated it at 32 cells) and keeps the detail's tail (the file
        // name) — the repetitive path prefix is what the detail drops.
        let frame = render_at_width(store, 80);
        assert!(frame.contains(name), "name not truncated (exceeds old budget):\n{frame}");
        assert!(frame.contains("mod.rs"), "detail tail (file name) survives:\n{frame}");
    }

    /// D: no dead preview space — when the selected candidate has an empty
    /// preview the candidate rows take the FULL width. A branch longer than
    /// the 2/3 (col 53) split must not be clipped when no preview is shown.
    #[test]
    fn picker_empty_preview_uses_full_width() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(p.join("README.md"), "# readme\n").unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(p)
                .args(args)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .status()
                .unwrap()
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["add", "-A"]);
        run(&["commit", "-q", "-m", "init"]);
        // A branch long enough to exceed the 2/3 (col 53) split point.
        let branch = format!("feature/{:0>50}", "");
        run(&["checkout", "-q", "-b", branch.as_str()]);

        let mut store = pty_store(p);
        store.key_event(crate::app::keymap::Key::ctrl_char('x'));
        store.key_event(crate::app::keymap::Key::char('g'));
        store.key_event(crate::app::keymap::Key::char('y'));
        assert!(store.picker_open(), "branch picker open");
        // No preview pane for the branch picker (empty preview).
        assert!(store.picker_preview().is_empty(), "branch preview is empty");
        let frame = render_at_width(store, 80);
        // The full branch name (past col 53) must survive — a dead 2/3
        // preview split would clip it.
        assert!(
            frame.contains(&branch),
            "full-width row (no dead preview) must not clip the long branch:\n{frame}"
        );
    }

    /// The buffer-list view renders open buffers with the current one
    /// marked, and the status line names the view.
    #[test]
    fn root_buffer_list_renders() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_path("src/main.rs");
        store.push_view(ViewId::BufferList);
        let s = render_frame(store);
        assert!(s.contains("*list-buffers*"), "view title missing:\n{s}");
        assert!(s.contains("*src/main.rs"), "current buffer row missing:\n{s}");
        // 06a: no scratch row exists (the table never auto-created one).
        assert!(!s.contains("*scratch*"), "no scratch row: \n{s}");
        assert!(s.contains("*  *list-buffers*"), "status line view name missing:\n{s}");
    }

    /// Unknown keys echo in the minibuffer.
    #[test]
    fn root_unknown_key_echoes_in_minibuffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = pty_store(dir.path());
        store.key_event(crate::app::keymap::Key::char('z'));
        let s = render_frame(store);
        assert!(s.contains("unbound key: z"), "{s}");
    }

    /// Regression: the tick State in Root forces a re-render after every
    /// store mutation (key event, resize, or async bus drain). This test
    /// verifies the render reads fresh store state: mutate the store
    /// (simulating what a key_event would do), render, and assert the new
    /// content is visible. In the live render loop, the tick bump is what
    /// causes iocraft to re-render and re-read the store snapshot.
    #[test]
    fn tick_bump_causes_render_to_read_fresh_state() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        // Initial state: home view (06a: no auto-created scratch), ready message.
        let store1 = pty_store(dir.path());
        let s1 = render_frame(store1);
        assert!(s1.contains("redline"), "initial frame should show home:\n{s1}");

        // Mutate: open a file (simulates what C-x C-f + RET would do).
        let mut store2 = store(dir.path());
        store2.open_path("src/main.rs");
        let s2 = render_frame(store2);
        assert!(s2.contains("src/main.rs"), "file title missing after open:\n{s2}");
        assert!(s2.contains("fn main"), "file content missing after open:\n{s2}");
    }

    /// Manual pty verification (verified-once, not automated):
    ///
    /// The five live checks that exercise the tick mechanism end-to-end:
    /// (a) C-x C-f opens the file picker (frame shows prompt + candidates)
    /// (b) typing filters and RET opens a real file (frame shows content)
    /// (c) C-x C-c quits before timeout with exit 0 (NOT exit 124); bare `q`
    ///     no longer quits (issue 05, finding 5 — it is now a no-op
    ///     close-view on the root buffer view)
    /// (d) touching a viewed file on disk repaints the frame (watcher path)
    /// (e) M-x opens the palette (frame shows prompt + commands)
    ///
    /// All five pass as of this commit. The pty harness:
    /// ```python
    /// import pty, os, time, fcntl, termios, struct, select
    /// pid, fd = pty.fork()
    /// if pid == 0:
    ///     fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
    ///     os.environ['TERM'] = 'xterm-256color'
    ///     os.execve('./target/debug/redline', ['redline'], os.environ)
    /// else:
    ///     fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
    ///     time.sleep(2)
    ///     os.write(fd, b'\x18\x06')  # C-x C-f
    ///     time.sleep(1)
    ///     os.write(fd, b'm')        # filter
    ///     time.sleep(0.5)
    ///     os.write(fd, b'\r')       # RET opens file
    ///     time.sleep(1)
    ///     os.write(fd, b'\x18\x03')  # C-x C-c quit (`q` no longer quits)
    /// ```
    #[ignore]
    #[test]
    fn pty_live_verification_documented() {
        // This test is a documentation placeholder. The actual pty checks
        // require a running terminal and cannot be automated in the unit
        // test suite (iocraft's mock_terminal_render_loop needs `futures`
        // which is not a direct dependency).
    }
}
