//! iocraft-to-app key translation: the ui layer owns this conversion;
//! `app/` has no iocraft dependency.

use iocraft::prelude::*;

use crate::app::keymap::{Key as AppKey, KeyCode as AppKeyCode};

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
#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(KeyEventKind::Press, code)
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

    /// plan 016 issue 01: pin the terminal control-code fact for the `C-/`
    /// undo binding, then pin the APP BOUNDARY that follows from it.
    ///
    /// What this test does NOT do: feed 0x1F to crossterm. crossterm's unix
    /// `parse_event` reads from a file descriptor (not a byte buffer a test
    /// can hand it), so the crossterm half of the mapping is a CITED fact,
    /// not one this test asserts: crossterm 0.29.0 decodes control byte 0x1F
    /// (what byte-based terminals send for BOTH `Ctrl+/` and `Ctrl-_`) as
    /// `Char('7') + CONTROL` — cited source, crossterm 0.29.0 unix parse:
    /// `c @ b'\x1C'..=b'\x1F' => Char(c - 0x1C + b'4') + CONTROL` (i.e.
    /// 0x1C/1D/1E/1F → C-4/5/6/7).
    ///
    /// What this test DOES pin — the app boundary: given the KeyEvent
    /// crossterm produces for 0x1F (`Char('7') + CONTROL`), `to_app_key`
    /// yields the app key `C-7` (NOT `C-/`); and given a CSI-u /
    /// kitty-protocol terminal's physical `Ctrl+/` (codepoint 0x2F '/' +
    /// ctrl → `Char('/') + CONTROL`), `to_app_key` yields exactly the key the
    /// `C-/` binding stores. Consequence (documented at the binding): the
    /// `C-/` mnemonic fires on modern CSI-u terminals, but on byte-based
    /// terminals the same physical key arrives as `C-7`, so `C-x u` is the
    /// universal fallback.
    #[test]
    fn crossterm_0x1f_decodes_to_c_7() {
        // The iocraft/crossterm event for raw control byte 0x1F (Ctrl+/ and
        // Ctrl-_ on byte-based terminals) is Char('7') + CONTROL → C-7.
        let mut ev = KeyEvent::new(KeyEventKind::Press, KeyCode::Char('7'));
        ev.modifiers = KeyModifiers::CONTROL;
        let app_key = to_app_key(&ev).unwrap();
        assert_eq!(
            app_key,
            crate::app::keymap::Key::ctrl_char('7'),
            "crossterm 0x1F (Ctrl+/ / Ctrl-_) must arrive as C-7, not C-/"
        );
        assert_eq!(app_key.code, crate::app::keymap::KeyCode::Char('7'));
        assert!(app_key.ctrl);

        // A CSI-u / modern-terminal Ctrl+/ (codepoint '/' + ctrl) is the app
        // key the `C-/` binding stores.
        let mut ev_slash = KeyEvent::new(KeyEventKind::Press, KeyCode::Char('/'));
        ev_slash.modifiers = KeyModifiers::CONTROL;
        let app_slash = to_app_key(&ev_slash).unwrap();
        assert_eq!(
            app_slash,
            crate::app::keymap::parse_sequence("C-/").unwrap()[0],
            "a CSI-u Ctrl+/ (Char('/') + ctrl) matches the C-/ binding"
        );
    }
}
