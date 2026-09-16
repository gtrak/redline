//! Root component: renders the top-of-stack view, the picker overlay
//! (when open), the minibuffer line, and the status line. Converts
//! iocraft terminal key events into app `Key` presses fed to the
//! keymap engine against the store (which lives in the element
//! context, set up by `main`).

use std::sync::{Arc, Mutex};

use iocraft::prelude::*;

use crate::app::keymap::{Key as AppKey, KeyCode as AppKeyCode};
use crate::app::store::{AppStore, PickerCandidate};
use crate::theme;
use crate::ui::picker::Picker;
use crate::ui::views::scratch::ScratchView;
use crate::ui::{face_bg, face_color, face_weight};

/// iocraft key codes to app key codes (the ui layer owns this
/// conversion; `app/` has no iocraft dependency).
impl From<iocraft::KeyCode> for AppKeyCode {
    fn from(k: iocraft::KeyCode) -> Self {
        use iocraft::KeyCode as K;
        match k {
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
            _ => AppKeyCode::Space,
        }
    }
}

/// Convert an iocraft key event (crossterm-shaped) to an app key.
pub(crate) fn to_app_key(key: &KeyEvent) -> Option<AppKey> {
    let code = AppKeyCode::from(key.code);
    let mut app_key = match code {
        AppKeyCode::Enter => AppKey::enter(),
        AppKeyCode::Up => AppKey::up(),
        AppKeyCode::Down => AppKey::down(),
        AppKeyCode::Space => AppKey::space(),
        other => AppKey::new(other),
    };
    app_key.ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    app_key.alt = key.modifiers.contains(KeyModifiers::ALT);
    app_key.shift = key.modifiers.contains(KeyModifiers::SHIFT);
    Some(app_key)
}

/// One render's worth of store state, extracted as owned values so the
/// `Mutex` guard can be dropped before the element tree is built.
struct Snapshot {
    quit: bool,
    project: String,
    view: String,
    pending: String,
    activity: String,
    message: String,
    text: String,
    picker: bool,
    prompt: String,
    query: String,
    selected: usize,
    candidates: Vec<PickerCandidate>,
    total: usize,
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
        if let TerminalEvent::Key(key) = &event
            && key.kind != KeyEventKind::Release
            && let Some(app_key) = to_app_key(key)
        {
            event_store.lock().unwrap().key_event(app_key);
        }
    });

    let snap = {
        let s = store.lock().unwrap();
        let picker = s.picker_open();
        Snapshot {
            quit: s.quit,
            project: s.project_display().to_string(),
            view: s.view_name_display(),
            pending: s.pending_display(),
            activity: s.activity_display(),
            message: s.message.clone(),
            text: s.current_text(),
            picker,
            prompt: s.picker_prompt().to_string(),
            query: s.picker_query().to_string(),
            selected: s.picker_selected(),
            candidates: s
                .picker_filtered()
                .iter()
                .map(|(c, _)| c.clone())
                .collect(),
            total: s.picker_count().1,
        }
    };

    if snap.quit {
        system.exit();
    }

    element! {
        View(flex_direction: FlexDirection::Column) {
            ScratchView(text: snap.text)
            #(if snap.picker {
                Some(element! {
                    Picker(
                        prompt: snap.prompt,
                        query: snap.query,
                        selected: snap.selected,
                        candidates: snap.candidates,
                        total: snap.total,
                    )
                })
            } else {
                None
            })
            Minibuffer(message: snap.message)
            StatusLine(
                project: snap.project,
                view: snap.view,
                pending: snap.pending,
                activity: snap.activity,
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

    /// The initial frame shows the scratch view, the status line (project
    /// placeholder + view name), and the ready-state minibuffer.
    #[test]
    fn root_initial_frame_renders_scratch() {
        let store = AppStore::new();
        let s = render_frame(store);
        assert!(s.contains("*scratch*"), "{s:?}");
        assert!(s.contains("no project"), "{s:?}");
        assert!(s.contains("ready"), "{s:?}");
    }

    /// M-x palette: the prompt + typed query, the surviving nucleo
    /// candidate, and the picker's count line are all in the frame.
    #[test]
    fn root_palette_renders_prompt_query_and_count() {
        let mut store = AppStore::new();
        store.key_event(crate::app::keymap::Key::alt_char('x'));
        store.key_event(crate::app::keymap::Key::char('q'));
        store.key_event(crate::app::keymap::Key::char('u'));
        let s = render_frame(store);
        assert!(s.contains("M-x qu"), "palette prompt+query missing:\n{s}");
        assert!(s.contains("quit"), "filtered candidate missing:\n{s}");
        assert!(s.contains("1 of 10"), "picker count line missing:\n{s}");
        // "qu" filters out the other seed commands.
        assert!(!s.contains("insert-demo-text"), "{s}");
    }

    /// Unknown keys echo in the minibuffer.
    #[test]
    fn root_unknown_key_echoes_in_minibuffer() {
        let mut store = AppStore::new();
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
}
