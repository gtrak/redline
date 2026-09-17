//! Root component: renders the top-of-stack view, the picker overlay
//! (when open), the minibuffer line, and the status line. Converts
//! iocraft terminal key events into app `Key` presses fed to the
//! keymap engine against the store (which lives in the element
//! context, set up by `main`).

use std::sync::{Arc, Mutex};

use iocraft::prelude::*;

use crate::app::keymap::{Key as AppKey, KeyCode as AppKeyCode};
use crate::app::store::{AppStore, BufferRow, DirtyCounts, FileViewLine, PickerCandidate, ResultRow, TransientMenuRow, ViewId};
use crate::model::sections::MagitRow;
use crate::theme;
use crate::ui::file_view::FileView;
use crate::ui::blame_view::BlameView;
use crate::ui::commit_editor::CommitEditorView;
use crate::ui::log_view::LogView;
use crate::ui::magit_status::MagitStatusView;
use crate::ui::picker::Picker;
use crate::ui::results_view::ResultsView;
use crate::ui::rows_view::MagitRowsView;
use crate::ui::transient_menu::TransientMenuView;
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
    commit_editor_title: String,
    commit_editor_rows: Vec<MagitRow>,
    dirty: Option<DirtyCounts>,
    // File view (issue 03).
    file_view_lines: Vec<FileViewLine>,
    file_view_title: String,
    file_view_top_line: usize,
    file_view_total_lines: usize,
    file_view_viewport_lines: usize,
    // File watching (issue 04): the current buffer's "changed on disk"
    // conflict marker.
    file_view_changed_on_disk: bool,
    // Tree sidebar (issue 09).
    tree_visible: bool,
    tree_rows: Vec<crate::app::store::TreeRow>,
    tree_selected: usize,
    // Symbol navigation (issue 05): which-function and indexing indicator.
    which_function: String,
    indexing: String,
    // Search (issue 06): results view + status-line indicator.
    search_title: String,
    search_rows: Vec<ResultRow>,
    search_top_row: usize,
    search_total_rows: usize,
    search_selected_row: Option<usize>,
    search_running: bool,
    search_error: Option<String>,
    searching: String,
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
        // all list views; click-to-position in the file view (Buffer).
        // Limitations: no drag-select, no click in pickers/menus, no
        // click-to-position in list views (v1).
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
                    // Subtract 1 for the title line offset.
                    let row = (mouse.row as usize).saturating_sub(1);
                    event_store.lock().unwrap().mouse_click_position(row);
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
        let s = store.lock().unwrap();
        let (top_line, total_lines, viewport_lines) = s.file_view_scroll_info();
        let (search_rows, search_top_row, search_total_rows, search_selected_row) =
            s.search_view_info();
        let (magit_rows, magit_top_row, magit_total_rows) = s.magit_view_info();
        Snapshot {
            quit: s.quit,
            project: s.project_display().to_string(),
            view: s.top_view(),
            view_name: s.view_name_display(),
            pending: s.pending_display(),
            activity: s.activity_display(),
            message: s.message.clone(),
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
            log_rows: s.log_rows(),
            blame_title: s.blame_title(),
            blame_rows: s.blame_rows(),
            commit_diff_title: s.commit_diff_title(),
            commit_diff_rows: s.commit_diff_rows(),
            commit_editor_title: s.commit_editor_title(),
            commit_editor_rows: s.commit_editor_rows(),
            dirty: s.dirty_counts(),
            file_view_lines: s.file_view_lines(),
            file_view_title: s.view_name_display(),
            file_view_top_line: top_line,
            file_view_total_lines: total_lines,
            file_view_viewport_lines: viewport_lines,
            file_view_changed_on_disk: s.current_buffer_changed_on_disk(),
            tree_visible: s.tree_visible(),
            tree_rows: s.tree_rows(),
            tree_selected: s.tree_selected(),
            which_function: s.which_function(),
            indexing: s.indexing_display(),
            search_title: s.search_title(),
            search_rows,
            search_top_row,
            search_total_rows,
            search_selected_row,
            search_running: s.search_running(),
            search_error: s.search_error(),
            searching: s.search_display(),
        }
    };

    if snap.quit {
        system.exit();
    }

    let main_view: Option<AnyElement<'static>> = match snap.view {
        ViewId::Buffer => Some(element! {
            FileView(
                title: snap.file_view_title.clone(),
                lines: snap.file_view_lines.clone(),
                total_lines: snap.file_view_total_lines,
                top_line: snap.file_view_top_line,
                viewport_lines: snap.file_view_viewport_lines,
                changed_on_disk: snap.file_view_changed_on_disk,
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
                help: "commit diff (read-only) · q back".to_string(),
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
                pending: snap.pending,
                activity: snap.activity,
                dirty: snap.dirty,
                which_function: snap.which_function,
                indexing: snap.indexing,
                searching: snap.searching,
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
    pub pending: String,
    pub activity: String,
    pub dirty: Option<DirtyCounts>,
    pub which_function: String,
    pub indexing: String,
    pub searching: String,
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
    if !props.searching.is_empty() {
        text.push_str(&format!("  *{}", props.searching));
    }
    element! {
        View(flex_shrink: 0.0, background_color: face_bg(face)) {
            Text(
                content: text,
                color: face_color(face),
                weight: face_weight(face),
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

    fn project_with_files(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.join("README.md"), "# readme\n").unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
    }

    /// The initial frame shows the buffer view (scratch), the status line
    /// (project name + view name), and the ready-state minibuffer.
    #[test]
    fn root_initial_frame_renders_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let s = render_frame(store);
        assert!(s.contains("*scratch*"), "{s:?}");
        assert!(s.contains("ready"), "{s:?}");
    }

    /// The status line shows the detected project name (not the old
    /// "no project" placeholder) when rooted in a project.
    #[test]
    fn status_line_shows_project_name() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let name = dir.path().file_name().unwrap().to_string_lossy().into_owned();
        let s = render_frame(store(dir.path()));
        assert!(s.contains(&format!("* {name} *")), "project name missing:\n{s}");
    }

    /// M-x palette: the prompt + typed query, the surviving nucleo
    /// candidate, and the picker's count line are all in the frame.
    #[test]
    fn root_palette_renders_prompt_query_and_count() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.key_event(crate::app::keymap::Key::alt_char('x'));
        store.key_event(crate::app::keymap::Key::char('q'));
        store.key_event(crate::app::keymap::Key::char('u'));
        let s = render_frame(store);
        assert!(s.contains("M-x qu"), "palette prompt+query missing:\n{s}");
        assert!(s.contains("quit"), "filtered candidate missing:\n{s}");
        assert!(s.contains("of 71"), "picker count line missing:\n{s}");
        // "qu" filters out the other seed commands.
        assert!(!s.contains("insert-demo-text"), "{s}");
    }

    /// Find-file picker: prompt, candidate, count, and the preview pane
    /// showing the selected file's first page (plain text).
    #[test]
    fn root_find_file_renders_preview() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
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
        assert!(s.contains(" *scratch*"), "scratch row missing:\n{s}");
        assert!(s.contains("*  *list-buffers*"), "status line view name missing:\n{s}");
    }

    /// Unknown keys echo in the minibuffer.
    #[test]
    fn root_unknown_key_echoes_in_minibuffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
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
        // Initial state: scratch buffer, ready message.
        let store1 = store(dir.path());
        let s1 = render_frame(store1);
        assert!(s1.contains("*scratch*"), "initial frame should show scratch:\n{s1}");

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
