//! Root component: renders the top-of-stack view, the picker overlay
//! (when open), the minibuffer line, and the status line. Converts
//! iocraft terminal key events into app `Key` presses fed to the
//! keymap engine against the store (which lives in the element
//! context, set up by `main`).

use std::sync::{Arc, Mutex};

use iocraft::prelude::*;

use crate::app::keymap::{Key as AppKey, KeyCode as AppKeyCode};
use crate::app::store::{AppStore, BufferRow, DirtyCounts, FileViewLine, PickerCandidate, ViewId};
use crate::model::sections::MagitRow;
use crate::theme;
use crate::ui::file_view::FileView;
use crate::ui::magit_status::MagitStatusView;
use crate::ui::picker::Picker;
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
    dirty: Option<DirtyCounts>,
    // File view (issue 03).
    file_view_lines: Vec<FileViewLine>,
    file_view_title: String,
    file_view_top_line: usize,
    file_view_total_lines: usize,
    file_view_viewport_lines: usize,
}

#[component]
pub fn Root(mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    // The store lives in the element context as `Arc<Mutex<AppStore>>`:
    // the event closure must be Send+Sync, and a plain context RefMut
    // handle is not.
    let store_handle = hooks.use_context::<Arc<Mutex<AppStore>>>();
    let store: Arc<Mutex<AppStore>> = (*store_handle).clone();
    let mut system = hooks.use_context_mut::<SystemContext>();

    // Clone for the event closure (it must be Send); keep `store` for the
    // render snapshot below.
    let event_store = store.clone();
    hooks.use_terminal_events(move |event: TerminalEvent| {
        if let TerminalEvent::Resize(_, height) = &event {
            // Update the viewport height on resize (subtract room for
            // the title, help line, and status line).
            let viewport = (*height as usize).saturating_sub(3);
            event_store.lock().unwrap().set_viewport_lines(viewport);
        }
        if let TerminalEvent::Key(key) = &event
            && key.kind != KeyEventKind::Release
            && let Some(app_key) = to_app_key(key)
        {
            event_store.lock().unwrap().key_event(app_key);
        }
    });

    let snap = {
        let s = store.lock().unwrap();
        let (top_line, total_lines, viewport_lines) = s.file_view_scroll_info();
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
            magit_rows: s.magit_rows(),
            dirty: s.dirty_counts(),
            file_view_lines: s.file_view_lines(),
            file_view_title: s.view_name_display(),
            file_view_top_line: top_line,
            file_view_total_lines: total_lines,
            file_view_viewport_lines: viewport_lines,
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
            MagitStatusView(rows: snap.magit_rows.clone())
        }
        .into()),
    };

    element! {
        View(flex_direction: FlexDirection::Column) {
            #(main_view)
            #(if snap.picker {
                Some(element! {
                    Picker(
                        prompt: snap.prompt,
                        query: snap.query,
                        selected: snap.selected,
                        candidates: snap.candidates,
                        total: snap.total,
                        preview: snap.preview,
                    )
                })
            } else {
                None
            })
            Minibuffer(message: snap.message)
            StatusLine(
                project: snap.project,
                view: snap.view_name,
                pending: snap.pending,
                activity: snap.activity,
                dirty: snap.dirty,
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
    if !props.activity.is_empty() {
        text.push_str(&format!("  *{}", props.activity));
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
        assert!(s.contains("of 40"), "picker count line missing:\n{s}");
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
}
