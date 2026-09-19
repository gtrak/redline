//! Root component: renders the top-of-stack view, the picker overlay
//! (when open), the minibuffer line, and the status line. Converts
//! iocraft terminal key events into app `Key` presses fed to the
//! keymap engine against the store (which lives in the element
//! context, set up by `main`).

use std::sync::{Arc, Mutex};

use iocraft::prelude::*;

use crate::app::keymap::{Key as AppKey, KeyCode as AppKeyCode};
use crate::app::store::{AppStore, BufferRow, DirtyCounts, FileViewRow, PickerCandidate, ResultRow, TransientMenuRow, ViewId};
use crate::model::sections::MagitRow;
use crate::theme;
use crate::ui::file_view::FileView;
use crate::ui::home_view::HomeView;
use crate::ui::blame_view::BlameView;
use crate::ui::commit_editor::CommitEditorView;
use crate::ui::log_view::LogView;
use crate::ui::magit_status::MagitStatusView;
use crate::ui::picker::Picker;
use crate::ui::results_view::ResultsView;
use crate::ui::rows_view::MagitRowsView;
use crate::ui::transient_menu::TransientMenuView;
use crate::model::tree_layout::TREE_WIDTH;
use crate::ui::tree::TreeSidebar;
use crate::ui::views::buffer::BufferListView;
use crate::ui::{face_bg, face_color, face_weight};

/// iocraft key codes to app key codes (the ui layer owns this
/// conversion; `app/` has no iocraft dependency). Codes with no app
/// equivalent (`F(*)`, `Insert`, `Null`, `CapsLock`, `ScrollLock`,
/// `NumLock`, `PrintScreen`, `Pause`, `Menu`, `KeypadBegin`,
/// `Media(*)`, `Modifier(*)`) map to `None` so the event is dropped
/// instead of fabricating a keypress.
fn code_to_app_code(code: iocraft::KeyCode) -> Option<AppKeyCode> {
    use iocraft::KeyCode as K;
    Some(match code {
        K::Char(c) => AppKeyCode::Char(c),
        K::Enter => AppKeyCode::Enter,
        K::Backspace => AppKeyCode::Backspace,
        K::Delete => AppKeyCode::Delete,
        K::Home => AppKeyCode::Home,
        K::End => AppKeyCode::End,
        K::PageUp => AppKeyCode::PageUp,
        K::PageDown => AppKeyCode::PageDown,
        K::Up => AppKeyCode::Up,
        K::Down => AppKeyCode::Down,
        K::Left => AppKeyCode::Left,
        K::Right => AppKeyCode::Right,
        K::Tab => AppKeyCode::Tab,
        K::BackTab => AppKeyCode::BackTab,
        K::Esc => AppKeyCode::Escape,
        _ => return None,
    })
}

/// Convert an iocraft key event (crossterm-shaped) to an app key;
/// `None` drops the event (unmapped code).
pub(crate) fn to_app_key(key: &KeyEvent) -> Option<AppKey> {
    let code = code_to_app_code(key.code)?;
    let mut app_key = match code {
        AppKeyCode::Enter => AppKey::enter(),
        AppKeyCode::Up => AppKey::up(),
        AppKeyCode::Down => AppKey::down(),
        AppKeyCode::Space => AppKey::space(),
        other => AppKey::new(other),
    };
    app_key.ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    app_key.alt = key.modifiers.contains(KeyModifiers::ALT);
    // Char codes already self-encode case; crossterm 0.29 sets SHIFT on
    // every uppercase char, so copy it only for non-Char codes.
    if !matches!(code, AppKeyCode::Char(_)) {
        app_key.shift = key.modifiers.contains(KeyModifiers::SHIFT);
    }
    Some(app_key)
}

/// The hardware-cursor position (0-based column, 0-based row — crossterm's
/// `MoveTo(col, row)` order) for the current view. The row is the view's cursor
/// row — the blue-bar (selected) row for list views, the point's screen row for
/// the read-focused buffer view (clamped to the viewport, plan 004 issue 05c)
/// — and the column is the point's DISPLAY column (char-index → display-
/// column conversion, plan 004 issue 05d) for the read-focused buffer view, 0
/// for list views).
///
/// Layout (0-based terminal rows): the main view's title is row 0 and its first
/// content row is row 1, so content row `i` (0-based within the window) is at
/// terminal row `1 + i`. With the tree sidebar visible the view's column is
/// PANE-relative while the terminal cursor is in ABSOLUTE screen coordinates
/// (the two panes are laid out by the flex row: the tree occupies terminal
/// columns `[0, TREE_WIDTH)`), so the column is offset by `TREE_WIDTH`
/// (clamped to the terminal width) and the row left unchanged (plan 004
/// issue 05g). Tree hidden: the offset is 0 (pane col 0 == absolute col 0).
fn cursor_cell(snap: &Snapshot) -> Option<(u16, u16)> {
    let cell = |i: usize| (0u16, 1 + i as u16);
    let pos = match snap.view {
        // 06a: home has no selectable row — the cursor stays where the
        // frame park put it (the status line).
        ViewId::Home => None,
        ViewId::BufferList => Some(cell(
            snap.buffer_list_selected.min(snap.buffer_rows.len().saturating_sub(1)),
        )),
        ViewId::Search => snap.search_selected_row.map(cell).or_else(|| Some(cell(0))),
        ViewId::MagitStatus => Some(cell(
            snap.magit_rows.iter().position(|r| r.selected).unwrap_or(0),
        )),
        ViewId::Log => Some(cell(snap.log_rows.iter().position(|r| r.selected).unwrap_or(0))),
        ViewId::Blame => Some(cell(snap.blame_rows.iter().position(|r| r.selected).unwrap_or(0))),
        ViewId::CommitDiff => Some(cell(
            snap.commit_diff_rows.iter().position(|r| r.selected).unwrap_or(0),
        )),
        ViewId::CommitEditor => Some(cell(0)),
        ViewId::Buffer => {
            // The "changed on disk" banner (when present) pushes the content
            // down one row; account for it so the cursor lands on the
            // intended content row, not the banner.
            let banner = u16::from(snap.file_view_changed_on_disk);
            let total = snap.file_view_total_lines;
            // The canvas gets `viewport - banner` rows; the cursor row must
            // never be placed outside the file area (plan 004 issue 05c —
            // the window can sit off the point, e.g. after a recenter clamp
            // or a resize, and C-l no longer moves the window off the point
            // in the first place).
            // The target BUFFER line: the point (read-focused) or the
            // insertion row (the last line, editable buffers — the cursor
            // stays there; plan 004 05b/05c). plan 005 issue 02: that
            // buffer line is translated to its RENDERED row through the row
            // list (note rows are extra rows; the dense `line - top`
            // offset is gone). When the target line is outside the visible
            // slice (the window can sit off the point), fall back to the
            // pre-annotation clamp.
            let target_line = if snap.file_view_current_buffer_editable {
                total.saturating_sub(1)
            } else {
                snap.file_view_point_line.min(total.saturating_sub(1))
            };
            let rows = &snap.file_view_rows;
            let content_row = match FileViewRow::row_for_line(rows, target_line) {
                Some(i) => i,
                None => target_line.saturating_sub(snap.file_view_top_line),
            }
            // plan 005 issue 02b: clamp against the RENDERED slice (which is
            // capped to <= viewport_lines by file_view_rows), not the raw
            // buffer-line viewport.
            .min(rows.len().saturating_sub(1));
            let col = if snap.file_view_current_buffer_editable {
                0
            } else {
                // The terminal cursor is positioned in CELLS, not char
                // indexes: the display column is the width of the point
                // line's prefix [0, point_col) (plan 004 issue 05d). For an
                // annotated line the code starts at cell 1 (the 1-cell
                // gutter for the \u{258e} marker, plan 005 issue 02b), so
                // add the gutter offset. Fall back to the char index when
                // the point line is outside the pre-computed visible slice.
                FileViewRow::row_for_line(rows, target_line)
                    .and_then(|i| rows.get(i))
                    .map(|r| {
                        let gutter = if r.annotated { 1 } else { 0 };
                        gutter
                            + crate::model::text_width::char_index_to_display_col(
                                &r.text,
                                snap.file_view_point_col,
                            )
                    })
                    .unwrap_or(snap.file_view_point_col)
            };
            Some((col as u16, 1 + banner + content_row as u16))
        }
    };
    // Plan 004 issue 05g: the terminal cursor is in ABSOLUTE screen
    // coordinates, but the view column computed above is PANE-relative —
    // the two panes are laid out by the flex row, and the tree sidebar (when
    // visible) occupies terminal columns `[0, TREE_WIDTH)`. Offset the column
    // by the tree width (row unchanged) so the hardware cursor does not land
    // inside the sidebar, and clamp to the terminal width so a huge column
    // cannot address outside the screen. Tree hidden: offset 0, unchanged.
    pos.map(|(col, row)| {
        let col = if snap.tree_visible {
            col.saturating_add(TREE_WIDTH).min(snap.terminal_width.saturating_sub(1))
        } else {
            col
        };
        (col, row)
    })
}

/// Plan 004 issue 05e: with the tree sidebar visible the code pane's column 0
/// is terminal column `TREE_WIDTH` (the tree occupies terminal columns
/// `[0, TREE_WIDTH)`). Splits a 0-based terminal click column into
/// `(in_tree, pane_col)`: a click inside the tree's columns must NOT move the
/// code point (the tree row under it is selected instead — the store maps the
/// terminal row); every other click passes the column shifted by the tree
/// width to the file-view click mapping. Tree hidden: the shift is 0 and no
/// click lands in the tree.
fn click_pane(tree_visible: bool, col: usize) -> (bool, usize) {
    let offset = if tree_visible { TREE_WIDTH as usize } else { 0 };
    (tree_visible && col < offset, col.saturating_sub(offset))
}

/// One render's worth of store state, extracted as owned values so the
/// `Mutex` guard can be dropped before the element tree is built.
struct Snapshot {
    quit: bool,
    project: String,
    view: ViewId,
    view_name: String,
    pending: String,
    activity: String,
    message: String,
    home_title: String,
    home_rows: Vec<TransientMenuRow>,
    buffer_rows: Vec<BufferRow>,
    buffer_list_selected: usize,
    picker: bool,
    prompt: String,
    query: String,
    selected: usize,
    candidates: Vec<PickerCandidate>,
    total: usize,
    preview: String,
    magit_rows: Vec<MagitRow>,
    magit_top_row: usize,
    magit_total_rows: usize,
    // issue 002: transient menu overlay
    menu_open: bool,
    menu_rows: Vec<TransientMenuRow>,
    menu_height: u32,
    // issue 08: log / blame / commit-diff / commit editor
    log_title: String,
    log_rows: Vec<MagitRow>,
    blame_title: String,
    blame_rows: Vec<MagitRow>,
    commit_diff_title: String,
    commit_diff_rows: Vec<MagitRow>,
    commit_diff_top_row: usize,
    commit_diff_total_rows: usize,
    commit_editor_title: String,
    commit_editor_rows: Vec<MagitRow>,
    dirty: Option<DirtyCounts>,
    // File view (issue 03).
    /// The pre-computed rendered rows: code rows + the virtual annotation
    /// note rows (plan 005 issue 02). Each row carries its buffer-line
    /// index; the cursor/click math translate through them.
    file_view_rows: Vec<FileViewRow>,
    /// The total number of rendered rows for the buffer (canvas bottom
    /// scroll indicator; plan 005 issue 02).
    file_view_total_rows: usize,
    file_view_title: String,
    file_view_top_line: usize,
    file_view_total_lines: usize,
    file_view_viewport_lines: usize,
    // plan 004 issue 05b: the file-view point (line, col) the cursor tracks.
    file_view_point_line: usize,
    file_view_point_col: usize,
    // File watching (issue 04): the current buffer's "changed on disk"
    // conflict marker.
    file_view_changed_on_disk: bool,
    // issue 03 (sweep): whether the current buffer is editable, driving the
    // per-kind "changed on disk" banner hint (M-x reload-buffer vs g).
    file_view_current_buffer_editable: bool,
    // plan 005 issue 01: the status-line buffer mode word (Edit / Read-only;
    // empty outside the buffer view).
    buffer_mode: String,
    // Tree sidebar (issue 09).
    tree_visible: bool,
    tree_rows: Vec<crate::app::store::TreeRow>,
    tree_selected: usize,
    // Terminal width (0 in the static render path; the hardware cursor is
    // live-only, so the tree offset clamps to a real width in practice).
    terminal_width: u16,
    // Symbol navigation (issue 05): which-function and indexing indicator.
    which_function: String,
    indexing: String,
    // Tooling-aware jump (plan 006 issue 02): resolving indicator.
    resolving: String,
    // External crate indexing (plan 006 issue 03): crate-index indicator.
    crate_indexing: String,
    // Search (issue 06): results view + status-line indicator.
    search_title: String,
    search_rows: Vec<ResultRow>,
    search_top_row: usize,
    search_total_rows: usize,
    search_selected_row: Option<usize>,
    search_running: bool,
    search_error: Option<String>,
    searching: String,
    // Plan 004 row 11: file-view position display (Top/Bot/L{n},{pct}%).
    position: String,
    // plan 005 issue 02: the current file's annotation count ("1 note" /
    // "3 notes"; empty when there are none).
    annotations: String,
    // Plan 004 issue 03: region line range for the file view's region face.
    region_lines: Option<(usize, usize)>,
    // Plan 004 issue 03: region size for the status line display.
    region_size: Option<usize>,
}

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
    eprintln!("DEBUG-ROOT tw_raw={} term_h_raw={}", tw_raw, term_h_raw);
    use iocraft::Size;
    let term_w: Size = if tw_raw > 0 {
        Size::Length(tw_raw as u32)
    } else {
        Size::Auto // static render path: content-sized (no terminal width)
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
            View(flex_direction: FlexDirection::Row, flex_grow: 1.0f32) {
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
                View(flex_direction: FlexDirection::Column, flex_grow: 1.0f32) {
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

#[derive(Default, Props)]
struct MinibufferProps {
    pub message: String,
}

#[component]
fn Minibuffer(props: &MinibufferProps, mut _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    let line = if props.message.is_empty() {
        "ready".to_string()
    } else {
        props.message.clone()
    };
    element! {
        View(flex_shrink: 0.0) {
            Text(
                content: format!(" {line}"),
                color: face_color(t.minibuffer),
                weight: face_weight(t.minibuffer),
            )
        }
    }
}

#[derive(Default, Props)]
struct StatusLineProps {
    pub project: String,
    pub view: String,
    /// The buffer mode word (`Edit` / `Read-only`; empty outside the buffer
    /// view, plan 005 issue 01).
    pub mode: String,
    pub pending: String,
    pub activity: String,
    pub dirty: Option<DirtyCounts>,
    pub which_function: String,
    pub indexing: String,
    /// Tooling-aware jump (plan 006 issue 02): active while an M-. miss is
    /// being resolved by the provider chain (`resolving …`).
    pub resolving: String,
    /// External crate indexing (plan 006 issue 03): active while a
    /// registry source's crate index builds in the background
    /// (`indexing crate …`).
    pub crate_indexing: String,
    pub searching: String,
    pub position: String,
    /// The current file's annotation count ("1 note" / "3 notes"; empty
    /// when there are none — plan 005 issue 02).
    pub annotations: String,
    /// Region size in bytes (plan 004 issue 03); shown when a region is active.
    pub region_size: Option<usize>,
}

#[component]
fn StatusLine(props: &StatusLineProps, mut _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    // A pending prefix puts the status line into its active face.
    let face = if props.pending.is_empty() {
        t.status_line
    } else {
        t.status_line_active
    };
    let mut text = format!("* {} *  {}", props.project, props.view);
    if !props.mode.is_empty() {
        text.push_str(&format!("  {}", props.mode));
    }
    if !props.pending.is_empty() {
        text.push_str(&format!("  [{}]", props.pending));
    }
    if let Some(d) = props.dirty
        && d.staged + d.unstaged + d.untracked > 0
    {
        // `+` staged (index vs HEAD), `~` unstaged (workdir vs index),
        // `?` untracked.
        text.push_str(&format!("  +{} ~{} ?{}", d.staged, d.unstaged, d.untracked));
    }
    if !props.which_function.is_empty() {
        text.push_str(&format!("  ({})", props.which_function));
    }
    if !props.activity.is_empty() {
        text.push_str(&format!("  *{}", props.activity));
    }
    if !props.indexing.is_empty() {
        text.push_str(&format!("  *{}", props.indexing));
    }
    if !props.resolving.is_empty() {
        text.push_str(&format!("  *{}", props.resolving));
    }
    if !props.crate_indexing.is_empty() {
        text.push_str(&format!("  *{}", props.crate_indexing));
    }
    if !props.searching.is_empty() {
        text.push_str(&format!("  *{}", props.searching));
    }
    if !props.position.is_empty() {
        text.push_str(&format!("  {}", props.position));
    }
    if !props.annotations.is_empty() {
        text.push_str(&format!("  {}", props.annotations));
    }
    if let Some(size) = props.region_size {
        text.push_str(&format!("  [{}B]", size));
    }
    element! {
        // `NoWrap` + hidden overflow: the status line must occupy EXACTLY one
        // row. A deep project path (e.g. /tmp/redline_pool/lane0/...) plus
        // mode/activity can exceed the terminal width; without this iocraft
        // wraps it onto a second row, which pushes every content row up and
        // breaks any flow that asserts the bottom row (U-J3 "status line on
        // exactly one row, no wrap artifact"; also the minibuffer-row reads).
        // Truncation is the right UX: the leftmost info (project, view) is
        // what matters most and stays visible.
        View(flex_shrink: 0.0, background_color: face_bg(face), overflow: Overflow::Hidden) {
            Text(
                content: text,
                color: face_color(face),
                weight: face_weight(face),
                wrap: TextWrap::NoWrap,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(KeyEventKind::Press, code)
    }

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
    /// would produce, verified by `key_event_conversion` below) and the
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

    /// The status line must occupy EXACTLY one row even when its text
    /// overflows the terminal width. A deep project path (the pooled
    /// lanes' `/tmp/rl/<i>/redline_*` roots) plus mode/activity can exceed
    /// 80 cols; without `NoWrap` + hidden overflow iocraft wraps it onto a
    /// second row, which pushed every content row up and broke the
    /// row-based U-J3 assertion. This is the regression pin for that layout
    /// shift: render the line at the 80-col width the PTY suites drive at
    /// and assert it stays a single row (clipped, not wrapped).
    #[test]
    fn status_line_long_text_stays_one_row() {
        // ~160 chars, well past the 80-col width below.
        let long = "r".repeat(160);
        let mut sl = element! {
            StatusLine(project: long.clone(), view: "Buffer".to_string())
        };
        let canvas = sl.render(Some(80));
        assert_eq!(
            canvas.height(),
            1,
            "status line wrapped onto a second row: {:?}",
            canvas.get_text(0, 0, 80, canvas.height())
        );
        // The text is present on that single row (truncated, not wrapped away).
        assert!(canvas.get_text(0, 0, 80, 1).contains('r'));
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
        assert!(s.contains("of 105"), "picker count line missing:\n{s}");
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

    #[test]
    fn key_event_conversion() {
        let mut key = KeyEvent::new(KeyEventKind::Press, KeyCode::Char('x'));
        key.modifiers = KeyModifiers::CONTROL;
        let app_key = to_app_key(&key).unwrap();
        assert_eq!(app_key, crate::app::keymap::Key::ctrl_char('x'));

        let app_key = to_app_key(&press(KeyCode::Enter)).unwrap();
        assert_eq!(app_key, crate::app::keymap::Key::enter());

        let app_key = to_app_key(&press(KeyCode::Up)).unwrap();
        assert_eq!(app_key, crate::app::keymap::Key::up());

        let app_key = to_app_key(&press(KeyCode::Down)).unwrap();
        assert_eq!(app_key, crate::app::keymap::Key::down());
    }

    /// Carry-over #1: unmapped iocraft key codes are dropped (None)
    /// instead of fabricating a `Space` keypress.
    #[test]
    fn unmapped_key_codes_are_dropped() {
        for code in [
            KeyCode::Insert,
            KeyCode::F(1),
            KeyCode::F(20),
            KeyCode::Null,
            KeyCode::CapsLock,
            KeyCode::ScrollLock,
            KeyCode::NumLock,
            KeyCode::PrintScreen,
            KeyCode::Pause,
            KeyCode::Menu,
            KeyCode::KeypadBegin,
            KeyCode::Media(iocraft::MediaKeyCode::Play),
            KeyCode::Modifier(iocraft::ModifierKeyCode::LeftShift),
        ] {
            assert!(to_app_key(&press(code)).is_none(), "{code:?} must drop");
        }
        // Mapped codes still convert.
        assert!(to_app_key(&press(KeyCode::Esc)).is_some());
        assert!(to_app_key(&press(KeyCode::Char('a'))).is_some());
    }

    /// Fix 3 lock-in: crossterm 0.29 sets SHIFT on every uppercase char.
    /// `to_app_key` must NOT copy that into `Key.shift` for Char codes,
    /// otherwise `G` → `Key{code: Char('G'), shift: true}` would never
    /// match a binding stored as `Key{code: Char('G'), shift: false}`.
    #[test]
    fn shift_modifier_not_set_for_char_codes() {
        let mut key = KeyEvent::new(KeyEventKind::Press, KeyCode::Char('G'));
        key.modifiers = KeyModifiers::SHIFT;
        let app_key = to_app_key(&key).unwrap();
        assert!(!app_key.shift, "shift must not be set for Char codes");
        assert_eq!(app_key, crate::app::keymap::Key::char('G'));

        // Non-Char codes still get shift.
        let mut key2 = KeyEvent::new(KeyEventKind::Press, KeyCode::PageUp);
        key2.modifiers = KeyModifiers::SHIFT;
        let app_key2 = to_app_key(&key2).unwrap();
        assert!(app_key2.shift, "shift must be set for non-Char codes");
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

    // ── plan 004 issue 05d: cursor_cell display-column conversion ───────

    /// A `Snapshot` shaped for `cursor_cell`: a read-focused Buffer view
    /// with the given lines at window top 0. All fields `cursor_cell` does
    /// not read stay at inert defaults.
    fn buffer_snapshot(lines: &[&str], point_line: usize, point_col: usize) -> Snapshot {
        let rows: Vec<FileViewRow> = lines
            .iter()
            .enumerate()
            .map(|(i, t)| FileViewRow {
                line: i,
                is_note: false,
                annotated: false,
                text: (*t).to_string(),
                spans: Vec::new(),
            })
            .collect();
        Snapshot {
            view: ViewId::Buffer,
            file_view_rows: rows.clone(),
            file_view_total_rows: lines.len(),
            file_view_top_line: 0,
            file_view_total_lines: lines.len(),
            file_view_viewport_lines: 21,
            file_view_point_line: point_line,
            file_view_point_col: point_col,
            quit: false,
            project: String::new(),
            view_name: String::new(),
            pending: String::new(),
            activity: String::new(),
            resolving: String::new(),
            crate_indexing: String::new(),
            message: String::new(),
            home_title: String::new(),
            home_rows: Vec::new(),
            buffer_rows: Vec::new(),
            buffer_list_selected: 0,
            picker: false,
            prompt: String::new(),
            query: String::new(),
            selected: 0,
            candidates: Vec::new(),
            total: 0,
            preview: String::new(),
            magit_rows: Vec::new(),
            magit_top_row: 0,
            magit_total_rows: 0,
            menu_open: false,
            menu_rows: Vec::new(),
            menu_height: 0,
            log_title: String::new(),
            log_rows: Vec::new(),
            blame_title: String::new(),
            blame_rows: Vec::new(),
            commit_diff_title: String::new(),
            commit_diff_rows: Vec::new(),
            commit_diff_top_row: 0,
            commit_diff_total_rows: 0,
            commit_editor_title: String::new(),
            commit_editor_rows: Vec::new(),
            dirty: None,
            file_view_title: String::new(),
            file_view_changed_on_disk: false,
            file_view_current_buffer_editable: false,
            buffer_mode: String::new(),
            tree_visible: false,
            tree_rows: Vec::new(),
            tree_selected: 0,
            terminal_width: 80,
            which_function: String::new(),
            indexing: String::new(),
            search_title: String::new(),
            search_rows: Vec::new(),
            search_top_row: 0,
            search_total_rows: 0,
            search_selected_row: None,
            search_running: false,
            search_error: None,
            searching: String::new(),
            position: String::new(),
            annotations: String::new(),
            region_lines: None,
            region_size: None,
        }
    }

    /// The cursor column is the point's DISPLAY column (terminal cells),
    /// not its char index (plan 004 issue 05d): char 13 of
    /// `CJK: abcd中 efgh` sits at display col 14 (中 is 2 cells).
    #[test]
    fn cursor_cell_uses_display_column_not_char_index() {
        let snap = buffer_snapshot(&["CJK: abcd中 efgh", "bb"], 0, 13);
        assert_eq!(
            cursor_cell(&snap),
            Some((14, 1)),
            "char 13 → display col 14 (0-based); content row 0 → row 1 (0-based)"
        );
        let snap = buffer_snapshot(&["CJK: abcd中 efgh", "bb"], 0, 0);
        assert_eq!(cursor_cell(&snap), Some((0, 1)));
        // ASCII lines are unchanged: display col == char index.
        let snap = buffer_snapshot(&["abcd", "bb"], 1, 1);
        assert_eq!(cursor_cell(&snap), Some((1, 2)));
        // Combining marks add no cell: char 2 of e+U+0301+x is at col 1.
        let snap = buffer_snapshot(&["e\u{301}x"], 0, 2);
        assert_eq!(cursor_cell(&snap), Some((1, 1)));
    }

    /// A point line outside the pre-computed visible slice falls back to
    /// the char index (the cursor row is clamped off the point there too).
    #[test]
    fn cursor_cell_falls_back_to_char_index_off_slice() {
        let mut snap = buffer_snapshot(&["bb", "bb"], 4, 2);
        snap.file_view_total_lines = 5;
        // plan 005 issue 02b: the clamp is against the RENDERED slice
        // (rows.len() = 2), not the buffer-line viewport. Off-slice point
        // line 4 falls back to char index 2, row clamped to last rendered
        // row (row 1) → terminal row 1 + 0 + 1 = 2.
        assert_eq!(cursor_cell(&snap), Some((2, 2)), "char index 2, clamped to rendered-slice row 1 → terminal row 2");
    }

    // ── plan 005 issue 02b: gutter + note-row overflow regression ────────

    /// A snapshot with annotated rows and interleaved note rows, shaped for
    /// the 02b regression: the point's line is at the window bottom, and a
    /// note row above it must not push the point off-canvas.
    fn annotated_snapshot(
        lines: &[(&str, bool)], // (text, annotated)
        note_lines: &[usize],   // buffer lines that have a note row below
        point_line: usize,
        point_col: usize,
        total_lines: usize,
    ) -> Snapshot {
        let rows: Vec<FileViewRow> = lines
            .iter()
            .enumerate()
            .flat_map(|(i, (text, annotated))| {
                let code_row = FileViewRow {
                    line: i,
                    is_note: false,
                    annotated: *annotated,
                    text: (*text).to_string(),
                    spans: Vec::new(),
                };
                let notes: Vec<FileViewRow> = note_lines
                    .iter()
                    .filter(|&&l| l == i)
                    .map(|&l| FileViewRow {
                        line: l,
                        is_note: true,
                        annotated: false,
                        text: format!("  \u{25b8} note {l}"),
                        spans: Vec::new(),
                    })
                    .collect();
                std::iter::once(code_row).chain(notes)
            })
            .collect();
        Snapshot {
            view: ViewId::Buffer,
            file_view_rows: rows.clone(),
            file_view_total_rows: total_lines + note_lines.len(),
            file_view_top_line: 0,
            file_view_total_lines: total_lines,
            file_view_viewport_lines: 21,
            file_view_point_line: point_line,
            file_view_point_col: point_col,
            quit: false,
            project: String::new(),
            view_name: String::new(),
            pending: String::new(),
            activity: String::new(),
            resolving: String::new(),
            crate_indexing: String::new(),
            message: String::new(),
            home_title: String::new(),
            home_rows: Vec::new(),
            buffer_rows: Vec::new(),
            buffer_list_selected: 0,
            picker: false,
            prompt: String::new(),
            query: String::new(),
            selected: 0,
            candidates: Vec::new(),
            total: 0,
            preview: String::new(),
            magit_rows: Vec::new(),
            magit_top_row: 0,
            magit_total_rows: 0,
            menu_open: false,
            menu_rows: Vec::new(),
            menu_height: 0,
            log_title: String::new(),
            log_rows: Vec::new(),
            blame_title: String::new(),
            blame_rows: Vec::new(),
            commit_diff_title: String::new(),
            commit_diff_rows: Vec::new(),
            commit_diff_top_row: 0,
            commit_diff_total_rows: 0,
            commit_editor_title: String::new(),
            commit_editor_rows: Vec::new(),
            dirty: None,
            file_view_title: String::new(),
            file_view_changed_on_disk: false,
            file_view_current_buffer_editable: false,
            buffer_mode: String::new(),
            tree_visible: false,
            tree_rows: Vec::new(),
            tree_selected: 0,
            terminal_width: 80,
            which_function: String::new(),
            indexing: String::new(),
            search_title: String::new(),
            search_rows: Vec::new(),
            search_top_row: 0,
            search_total_rows: 0,
            search_selected_row: None,
            search_running: false,
            search_error: None,
            searching: String::new(),
            position: String::new(),
            annotations: String::new(),
            region_lines: None,
            region_size: None,
        }
    }

    /// plan 005 issue 02b: the cursor column on an annotated line adds the
    /// 1-cell gutter (the \u{258e} marker at cell 0 pushes code to cell 1).
    /// Point at char 0 of an annotated line → display col 1 (gutter + 0).
    #[test]
    fn cursor_cell_annotated_line_adds_gutter_to_column() {
        // One annotated line, point at char 3 (display col 3 in the code).
        // With the gutter, the terminal cursor is at col 1 + 3 = 4.
        let snap = annotated_snapshot(
            &[("fn target_one() {}", true)],
            &[0],
            0, 3, 1,
        );
        assert_eq!(
            cursor_cell(&snap),
            Some((4, 1)),
            "annotated: gutter(1) + display_col(3) = 4; row 0 → terminal 1"
        );
        // Point at char 0 → col 1 (just the gutter).
        let snap = annotated_snapshot(&[("fn target_one() {}", true)], &[0], 0, 0, 1);
        assert_eq!(cursor_cell(&snap), Some((1, 1)), "char 0 → col 1 (gutter only)");
        // Non-annotated line: no gutter.
        let snap = annotated_snapshot(&[("plain line", false)], &[], 0, 3, 1);
        assert_eq!(cursor_cell(&snap), Some((3, 1)), "non-annotated: no gutter");
    }

    /// plan 005 issue 02b regression: a note row above the point must not
    /// push the point's rendered row past the canvas. The rendered slice is
    /// capped so the point is always drawn, and the cursor row equals the
    /// point's rendered row.
    #[test]
    fn cursor_cell_note_row_does_not_overflow_point() {
        // Simulate the capped slice: 20 code rows (lines 1-20), no notes.
        // The original slice [0, 21) had a note on line 0, pushing the total
        // to 22 > 21. The cap reduces to 20 code rows + 0 notes = 20 rows.
        // The point at line 20 is at rendered row 19 (line 1 → row 0, ...
        // line 20 → row 19). Terminal row = 1 + 0 + 19 = 20.
        let mut rows: Vec<FileViewRow> = Vec::new();
        for line in 1..=20 {
            rows.push(FileViewRow {
                line,
                is_note: false,
                annotated: false,
                text: format!("line {}", line),
                spans: Vec::new(),
            });
        }
        let snap = Snapshot {
            view: ViewId::Buffer,
            file_view_rows: rows,
            file_view_total_rows: 21,
            file_view_top_line: 1,
            file_view_total_lines: 60,
            file_view_viewport_lines: 21,
            file_view_point_line: 20,
            file_view_point_col: 0,
            quit: false,
            project: String::new(),
            view_name: String::new(),
            pending: String::new(),
            activity: String::new(),
            resolving: String::new(),
            crate_indexing: String::new(),
            message: String::new(),
            home_title: String::new(),
            home_rows: Vec::new(),
            buffer_rows: Vec::new(),
            buffer_list_selected: 0,
            picker: false,
            prompt: String::new(),
            query: String::new(),
            selected: 0,
            candidates: Vec::new(),
            total: 0,
            preview: String::new(),
            magit_rows: Vec::new(),
            magit_top_row: 0,
            magit_total_rows: 0,
            menu_open: false,
            menu_rows: Vec::new(),
            menu_height: 0,
            log_title: String::new(),
            log_rows: Vec::new(),
            blame_title: String::new(),
            blame_rows: Vec::new(),
            commit_diff_title: String::new(),
            commit_diff_rows: Vec::new(),
            commit_diff_top_row: 0,
            commit_diff_total_rows: 0,
            commit_editor_title: String::new(),
            commit_editor_rows: Vec::new(),
            dirty: None,
            file_view_title: String::new(),
            file_view_changed_on_disk: false,
            file_view_current_buffer_editable: false,
            buffer_mode: String::new(),
            tree_visible: false,
            tree_rows: Vec::new(),
            tree_selected: 0,
            terminal_width: 80,
            which_function: String::new(),
            indexing: String::new(),
            search_title: String::new(),
            search_rows: Vec::new(),
            search_top_row: 0,
            search_total_rows: 0,
            search_selected_row: None,
            search_running: false,
            search_error: None,
            searching: String::new(),
            position: String::new(),
            annotations: String::new(),
            region_lines: None,
            region_size: None,
        };
        // The point (line 20) is at rendered row 19 (0-indexed in the rows
        // slice). Terminal row = 1 (title) + 0 (banner) + 19 = 20.
        let result = cursor_cell(&snap);
        assert_eq!(result, Some((0, 20)), "point's line IS drawn; cursor on it");
    }

    // ── plan 004 issue 05g: cursor_cell tree-sidebar column offset ────

    /// plan 004 issue 05g: the terminal cursor is in ABSOLUTE screen
    /// coordinates while the view column is PANE-relative — with the tree
    /// visible the column is offset by `TREE_WIDTH` (row unchanged); tree
    /// hidden the column is unchanged (pane col 0 == absolute col 0).
    #[test]
    fn cursor_cell_tree_visible_offsets_column_by_tree_width() {
        let mut snap = buffer_snapshot(&["abcd"], 0, 3);
        snap.tree_visible = true;
        assert_eq!(
            cursor_cell(&snap),
            Some((TREE_WIDTH + 3, 1)),
            "pane col 3 → terminal col 34 + 3 (0-based)"
        );
        // Tree hidden: the column is exactly the pane column.
        let snap = buffer_snapshot(&["abcd"], 0, 3);
        assert_eq!(cursor_cell(&snap), Some((3, 1)), "tree hidden: unchanged");
    }

    /// plan 004 issue 05g: with the tree visible a huge pane column is
    /// clamped to the last terminal column so it cannot address outside
    /// the screen (80-col terminal: max 0-based col 79).
    #[test]
    fn cursor_cell_tree_visible_clamps_to_terminal_width() {
        let long = "A".repeat(200);
        let mut snap = buffer_snapshot(&[&long], 0, 100);
        snap.tree_visible = true;
        assert_eq!(
            cursor_cell(&snap),
            Some((79, 1)),
            "pane col 100 + tree 34 → clamped to terminal col 79"
        );
        // Tree hidden: no clamp (a beyond-pane column may land off-screen,
        // the pre-05g behavior).
        let snap = buffer_snapshot(&[&long], 0, 100);
        assert_eq!(cursor_cell(&snap), Some((100, 1)), "tree hidden: unchanged");
    }

    // ── plan 004 issue 05e: tree-sidebar click offset ──────────────────

    /// Tree hidden: the terminal column maps 1:1 to the code pane's column
    /// and no click lands in the tree.
    #[test]
    fn click_pane_tree_hidden_is_identity() {
        for col in [0usize, 1, 33, 34, 40, 79] {
            assert_eq!(click_pane(false, col), (false, col), "col {col}");
        }
    }

    /// Tree visible: columns `[0, TREE_WIDTH)` are the tree (no code-pane
    /// click); columns `>= TREE_WIDTH` pass through shifted by the tree
    /// width (34 → pane col 0, 40 → pane col 6).
    #[test]
    fn click_pane_tree_visible_offsets_by_tree_width() {
        for col in 0..TREE_WIDTH as usize {
            assert_eq!(click_pane(true, col), (true, 0), "col {col} is the tree");
        }
        assert_eq!(click_pane(true, 34), (false, 0), "pane col 0");
        assert_eq!(click_pane(true, 40), (false, 6), "pane col 6");
        assert_eq!(click_pane(true, 79), (false, 45), "pane col 45");
    }
}
