//! Emacs-style keymap engine: a `Key` (modifiers + code), a `KeySeq`
//! (a `Vec<Key>`), a trie `KeyMap` with prefix-sequence lookup, and a
//! parser for emacs notation strings (`"C-x C-f"`, `"M-x"`, `"RET"`, …)
//! which is required because config overrides are TOML strings.
//!
//! The ui layer converts iocraft/crossterm events to `Key` (the app
//! layer has zero iocraft dependencies).

use std::collections::HashMap;
use std::fmt;

/// Key code. Character keys carry the character itself; modifiers live
/// in `Key.modifiers` (Shift on chars only affects the case of the char,
/// so it is not stored for them).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    Tab,
    BackTab,
    Escape,
    /// Named by `"SPC"` in emacs notation; also produced by Shift+Space.
    #[default]
    Space,
}

impl fmt::Display for KeyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            KeyCode::Char(c) => return write!(f, "{}", c),
            KeyCode::Enter => "RET",
            KeyCode::Backspace => "DEL",
            KeyCode::Delete => "DF",
            KeyCode::Home => "HOME",
            KeyCode::End => "END",
            KeyCode::PageUp => "PGUP",
            KeyCode::PageDown => "PGDN",
            KeyCode::Up => "UP",
            KeyCode::Down => "DOWN",
            KeyCode::Left => "LEFT",
            KeyCode::Right => "RIGHT",
            KeyCode::Tab => "TAB",
            KeyCode::BackTab => "ISO-Backtab",
            KeyCode::Escape => "ESC",
            KeyCode::Space => "SPC",
        };
        f.write_str(name)
    }
}

/// One keypress: a code plus modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Key {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Key {
    pub fn new(code: KeyCode) -> Self {
        Self {
            code,
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    pub fn char(c: char) -> Self {
        Self::new(KeyCode::Char(c))
    }

    pub fn enter() -> Self {
        Self::new(KeyCode::Enter)
    }

    /// Construct a Tab key (used by tests; the built-in keymap uses the
    /// string parser, which produces the same `Key`).
    #[allow(dead_code)]
    pub fn tab() -> Self {
        Self::new(KeyCode::Tab)
    }

    pub fn up() -> Self {
        Self::new(KeyCode::Up)
    }

    pub fn down() -> Self {
        Self::new(KeyCode::Down)
    }

    pub fn space() -> Self {
        Self::new(KeyCode::Space)
    }

    pub fn ctrl_char(c: char) -> Self {
        Self {
            code: KeyCode::Char(c),
            ctrl: true,
            ..Key::new(KeyCode::default())
        }
    }

    /// Construct an Alt+char key (used by tests; the built-in keymap uses
    /// the string parser, which produces the same `Key`).
    #[allow(dead_code)]
    pub fn alt_char(c: char) -> Self {
        Self {
            code: KeyCode::Char(c),
            alt: true,
            ..Key::new(KeyCode::default())
        }
    }

    /// True if this key produces a printable character the picker can
    /// append to its query: an unmodified ASCII 0x20..=0x7E char, or Space.
    pub fn printable(self) -> bool {
        match self.code {
            KeyCode::Char(c) => !self.ctrl && !self.alt && (0x20u32..=0x7Eu32).contains(&(c as u32)),
            KeyCode::Space => !self.ctrl && !self.alt,
            _ => false,
        }
    }

    /// The character this key contributes to text input; `None` for
    /// non-printable keys.
    pub fn char_value(self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if self.printable() => Some(c),
            KeyCode::Space if self.printable() => Some(' '),
            _ => None,
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("C-")?;
        }
        if self.alt {
            f.write_str("M-")?;
        }
        if self.shift && !matches!(self.code, KeyCode::Char(_)) {
            f.write_str("S-")?;
        }
        write!(f, "{}", self.code)
    }
}

/// A key sequence: the keys pressed so far (prefix state) or a full
/// binding.
pub type KeySeq = Vec<Key>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseKeyError {
    /// Token is a bare modifier with no key (e.g. `"C-"` or `""`).
    MissingKey(String),
    /// Token is not a recognized key name (e.g. `"FOO"`).
    UnknownKey(String),
}

impl fmt::Display for ParseKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseKeyError::MissingKey(t) => write!(f, "modifier token `{}` is missing a key", t),
            ParseKeyError::UnknownKey(t) => write!(f, "unknown key token `{}`", t),
        }
    }
}

impl std::error::Error for ParseKeyError {}

/// Parse one token of emacs notation: an optional run of modifiers
/// (`C-`, `M-`, `S-`, repeatable in any order) followed by a key name.
///
/// Key names: a single character, or `RET`, `SPC`, `TAB`, `ESC`, `UP`,
/// `DOWN`, `LEFT`, `RIGHT`, `DEL` / `Backspace`, `DF` / `Delete`, `HOME`,
/// `END`, `PGUP`, `PGDN`, or `^`-notation (`^X` == C-x, `^@`, `^?`).
pub fn parse_key(token: &str) -> Result<Key, ParseKeyError> {
    let mut rest = token;
    let mut key = Key::default();
    loop {
        if let Some(r) = rest.strip_prefix("C-") {
            key.ctrl = true;
            rest = r;
            continue;
        }
        if let Some(r) = rest.strip_prefix("M-") {
            key.alt = true;
            rest = r;
            continue;
        }
        if let Some(r) = rest.strip_prefix("S-") {
            key.shift = true;
            rest = r;
            continue;
        }
        break;
    }
    parse_key_code(rest, &key, token)
}

fn parse_key_code(name: &str, key: &Key, token: &str) -> Result<Key, ParseKeyError> {
    let key = *key;
    let code = match name {
        "RET" => KeyCode::Enter,
        "SPC" => KeyCode::Space,
        "TAB" => KeyCode::Tab,
        "ESC" | "ESCAPE" => KeyCode::Escape,
        "UP" => KeyCode::Up,
        "DOWN" => KeyCode::Down,
        "LEFT" => KeyCode::Left,
        "RIGHT" => KeyCode::Right,
        "DEL" | "Backspace" => KeyCode::Backspace,
        "DF" | "Delete" => KeyCode::Delete,
        "HOME" => KeyCode::Home,
        "END" => KeyCode::End,
        "PGUP" => KeyCode::PageUp,
        "PGDN" => KeyCode::PageDown,
        _ => {
            // `^` notation (emacs): "^X" == C-x, "^@" == NUL, "^?" == DEL.
            if name.starts_with('^') && name.len() == 2 {
                let c = name.as_bytes()[1] as char;
                let code = match c {
                    'A'..='Z' => c.to_ascii_lowercase(),
                    'a'..='z' => c,
                    '@' => '\0',
                    '?' => 0x7f as char,
                    _ => return Err(ParseKeyError::UnknownKey(token.to_string())),
                };
                return Ok(Key {
                    code: KeyCode::Char(code),
                    ctrl: true,
                    alt: key.alt,
                    shift: key.shift,
                });
            }
            // A bare key name: a single graphic character.
            if name.len() == 1
                && let Some(c) = name.chars().next()
                && c.is_ascii_graphic()
            {
                // Fall through to the ctrl post-processing below (it
                // case-folds C-X to C-x).
                KeyCode::Char(c)
            } else {
                return Err(ParseKeyError::UnknownKey(token.to_string()));
            }
        }
    };
    // "C-x" and "C-X" are both control-x (letters are case-folded);
    // C-SPC is NUL.
    let code = if key.ctrl {
        match code {
            KeyCode::Char(c) if c.is_ascii_alphabetic() => {
                KeyCode::Char(c.to_ascii_lowercase())
            }
            KeyCode::Space => KeyCode::Char(' '),
            other => other,
        }
    } else {
        code
    };
    Ok(Key {
        code,
        ctrl: key.ctrl,
        alt: key.alt,
        shift: key.shift,
    })
}

/// Parse a full sequence string like `"C-x C-f"` or `"M-x"`.
pub fn parse_sequence(s: &str) -> Result<KeySeq, ParseKeyError> {
    let tokens: Vec<&str> = s.split_whitespace().collect();
    if tokens.is_empty() {
        return Err(ParseKeyError::MissingKey(s.to_string()));
    }
    tokens.iter().map(|t| parse_key(t)).collect()
}

/// Lookup outcome for a key sequence in a `KeyMap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup<'a> {
    /// The sequence is a strict prefix of some binding: keep pending.
    Pending,
    /// The sequence binds to a command.
    Command(&'a str),
}

#[derive(Clone, Debug, Default)]
struct Node {
    next: HashMap<Key, Node>,
    command: Option<String>,
}

impl Node {
    fn lookup(&self, keys: &[Key]) -> Option<Lookup<'_>> {
        let mut cur = self;
        for k in keys {
            cur = cur.next.get(k)?;
        }
        match &cur.command {
            Some(cmd) => Some(Lookup::Command(cmd)),
            None => Some(Lookup::Pending),
        }
    }

    /// Recursively collect every leaf (sequence, command) binding under
    /// this node into `out`.
    fn collect_command_pairs(&self, prefix: &mut KeySeq, out: &mut Vec<(KeySeq, String)>) {
        for (k, child) in &self.next {
            prefix.push(*k);
            if let Some(cmd) = &child.command {
                out.push((prefix.clone(), cmd.clone()));
            }
            child.collect_command_pairs(prefix, out);
            prefix.pop();
        }
    }
}

/// Human-readable form of a sequence (`C-x C-f`), used in error messages.
fn keys_display(keys: &[Key]) -> String {
    keys.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(" ")
}

/// A trie of key sequences → command name. Prefix nodes (sequences that
/// are a strict prefix of a longer binding) are recorded as pending.
#[derive(Default, Clone, Debug)]
pub struct KeyMap {
    root: Node,
}

impl KeyMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind `command` to `keys`. Fails if a binding already exists for a
    /// strict prefix of `keys` (the new binding would be unreachable),
    /// if `keys` is a strict prefix of an existing binding (the old
    /// binding would be unreachable), or if the same sequence is already
    /// bound to another command.
    pub fn bind(&mut self, keys: &[Key], command: &str) -> Result<(), String> {
        let last = keys
            .last()
            .ok_or_else(|| "empty key sequence".to_string())?;
        let seq = keys_display(keys);
        let mut cur = &mut self.root;
        for k in &keys[..keys.len() - 1] {
            let node = cur.next.entry(*k).or_default();
            if let Some(shadowed) = node.command.as_deref() {
                return Err(format!(
                    "cannot bind `{seq}` → `{command}`: sequence prefix already binds `{shadowed}`"
                ));
            }
            cur = node;
        }
        let node = cur.next.entry(*last).or_default();
        if let Some(old) = &node.command {
            if old != command {
                return Err(format!(
                    "`{seq}` → `{command}`: already bound to `{old}`"
                ));
            }
        } else if !node.next.is_empty() {
            // A binding here would shadow every longer sequence under
            // this prefix (e.g. a new `C-x` while `C-x C-c` is bound).
            return Err(format!(
                "cannot bind `{seq}` → `{command}`: existing key sequences extend beyond this prefix"
            ));
        } else {
            node.command = Some(command.to_string());
        }
        Ok(())
    }

    /// `None` if no binding starts with `keys` (dead end); `Some(Pending)`
    /// if `keys` is a strict prefix of some binding; `Some(Command)` on a
    /// full match.
    pub fn lookup(&self, keys: &[Key]) -> Option<Lookup<'_>> {
        self.root.lookup(keys)
    }

    /// True when `keys` walks to a trie node that has children (i.e. the
    /// sequence is a strict prefix of some longer binding).
    pub fn is_prefix(&self, keys: &[Key]) -> bool {
        let mut cur = Some(&self.root);
        for k in keys {
            cur = cur.and_then(|node| node.next.get(k));
            if cur.is_none() {
                return false;
            }
        }
        cur.map(|node| !node.next.is_empty()).unwrap_or(false)
    }

    /// Every (sequence, command) leaf binding in this map, sorted by the
    /// sequence's display string then the command name (deterministic; the
    /// trie's `HashMap` order is not). Used to derive the transient menu
    /// (issue 002) from the keymap rather than a hand-written table.
    pub fn command_pairs(&self) -> Vec<(KeySeq, String)> {
        let mut out: Vec<(KeySeq, String)> = Vec::new();
        let mut prefix: KeySeq = Vec::new();
        self.root.collect_command_pairs(&mut prefix, &mut out);
        out.sort_by(|a, b| {
            let da: String = a.0.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(" ");
            let db: String = b.0.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(" ");
            da.cmp(&db).then_with(|| a.1.cmp(&b.1))
        });
        out
    }
}

/// Load a `&[(&str, &str)]` table of (emacs-notation sequence, command)
/// pairs into a `KeyMap`. Panics on a parse or bind error, naming the
/// offending sequence and command (fail-loud at construction, preserving
/// the old per-call `.unwrap()` behaviour).
pub fn load_bindings(table: &[(&str, &str)]) -> KeyMap {
    let mut km = KeyMap::new();
    for (seq_str, command) in table {
        let seq = parse_sequence(seq_str)
            .unwrap_or_else(|e| panic!("keymap: invalid sequence `{seq_str}`: {e}"));
        km.bind(&seq, command)
            .unwrap_or_else(|e| panic!("keymap: {e}"));
    }
    km
}

/// Global + per-view keymaps. The view's map is tried first; on a dead
/// end the global map is tried (per-view bindings beat global on
/// conflict, global still fills the gaps).
#[derive(Default, Clone, Debug)]
pub struct KeymapEngine {
    pub global: KeyMap,
    pub view: KeyMap,
}

impl KeymapEngine {
    pub fn new(global: KeyMap, view: KeyMap) -> Self {
        Self { global, view }
    }

    /// Resolve `keys` against the view map, falling back to global on a
    /// dead end. Returns `None` if the sequence is dead in both maps.
    pub fn resolve(&self, keys: &[Key]) -> Option<Lookup<'_>> {
        match self.view.lookup(keys) {
            Some(l) => Some(l),
            None => self.global.lookup(keys),
        }
    }

    /// True when `keys` walks to a prefix node in either map (view first).
    pub fn prefix_exists(&self, keys: &[Key]) -> bool {
        self.view.is_prefix(keys) || self.global.is_prefix(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(s: &str) -> KeySeq {
        parse_sequence(s).unwrap()
    }

    #[test]
    fn parse_simple_keys() {
        assert_eq!(parse_sequence("q").unwrap(), vec![Key::char('q')]);
        assert_eq!(parse_sequence("RET").unwrap(), vec![Key::enter()]);
        assert_eq!(parse_sequence("SPC").unwrap(), vec![Key::space()]);
        assert_eq!(parse_sequence("TAB").unwrap(), vec![Key::new(KeyCode::Tab)]);
        assert_eq!(parse_sequence("ESC").unwrap(), vec![Key::new(KeyCode::Escape)]);
        assert_eq!(parse_sequence("UP").unwrap(), vec![Key::up()]);
        assert_eq!(parse_sequence("DOWN").unwrap(), vec![Key::down()]);
    }

    #[test]
    fn parse_c_spc_matches_terminal_representation() {
        // C-SPC in emacs notation must produce the same key that the terminal
        // delivers: Char(' ') + ctrl (NUL decoded by crossterm/iocraft).
        let k = parse_sequence("C-SPC").unwrap()[0];
        assert_eq!(k, Key::ctrl_char(' '));
        assert_eq!(k.code, KeyCode::Char(' '));
        assert!(k.ctrl);
    }

    #[test]
    fn parse_caret_notation() {
        let k = parse_sequence("^X").unwrap()[0];
        assert_eq!(k, Key::ctrl_char('x'));
        assert_eq!(parse_sequence("^@").unwrap()[0].code, KeyCode::Char('\0'));
        assert_eq!(parse_sequence("^?").unwrap()[0].code, KeyCode::Char(0x7f as char));
    }

    #[test]
    fn parse_modifiers() {
        let k = parse_sequence("C-x").unwrap()[0];
        assert_eq!(k, Key::ctrl_char('x'));
        assert_eq!(k.to_string(), "C-x");

        let k = parse_sequence("M-x").unwrap()[0];
        assert_eq!(k, Key::alt_char('x'));
        assert_eq!(k.to_string(), "M-x");

        assert_eq!(parse_sequence("C-g").unwrap(), vec![Key::ctrl_char('g')]);
        assert_eq!(
            parse_sequence("C-x C-f").unwrap(),
            vec![Key::ctrl_char('x'), Key::ctrl_char('f')]
        );
        assert_eq!(
            parse_sequence("C-c p f").unwrap(),
            vec![Key::ctrl_char('c'), Key::char('p'), Key::char('f')]
        );
    }

    #[test]
    fn parse_control_uppercase_is_same_as_lowercase() {
        assert_eq!(parse_sequence("C-X").unwrap()[0], Key::ctrl_char('x'));
        let k = parse_sequence("C-M-x").unwrap()[0];
        assert!(k.ctrl && k.alt);
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_sequence("").is_err());
        assert!(parse_sequence("   ").is_err());
        assert!(parse_sequence("FOO").is_err());
        assert!(parse_sequence("C-").is_err());
        assert!(parse_sequence("M-").is_err());
    }

    #[test]
    fn lookup_full_match() {
        let mut km = KeyMap::new();
        km.bind(&seq("q"), "quit").unwrap();
        assert_eq!(km.lookup(&seq("q")), Some(Lookup::Command("quit")));
    }

    #[test]
    fn lookup_prefix_is_pending() {
        let mut km = KeyMap::new();
        km.bind(&seq("C-x C-f"), "open-file").unwrap();
        assert_eq!(km.lookup(&seq("C-x")), Some(Lookup::Pending));
        assert_eq!(
            km.lookup(&seq("C-x C-f")),
            Some(Lookup::Command("open-file"))
        );
    }

    #[test]
    fn lookup_dead_end_is_none() {
        let mut km = KeyMap::new();
        km.bind(&seq("C-x C-f"), "open-file").unwrap();
        assert_eq!(km.lookup(&seq("C-x C-q")), None);
        assert_eq!(km.lookup(&seq("C-z")), None);
    }

    #[test]
    fn bind_rejects_shading_a_prefix_binding() {
        let mut km = KeyMap::new();
        km.bind(&seq("C-x"), "open-palette").unwrap();
        let err = km.bind(&seq("C-x C-c"), "quit").unwrap_err();
        assert!(err.contains("open-palette"), "{err}");
    }

    #[test]
    fn bind_same_command_twice_is_ok() {
        let mut km = KeyMap::new();
        km.bind(&seq("M-x"), "open-palette").unwrap();
        km.bind(&seq("M-x"), "open-palette").unwrap();
    }

    #[test]
    fn engine_view_beats_global() {
        let mut g = KeyMap::new();
        g.bind(&seq("q"), "global-command").unwrap();
        g.bind(&seq("C-x C-c"), "quit").unwrap();
        let mut v = KeyMap::new();
        v.bind(&seq("q"), "view-command").unwrap();
        let engine = KeymapEngine::new(g, v);
        assert_eq!(
            engine.resolve(&seq("q")),
            Some(Lookup::Command("view-command"))
        );
        // global fills the gaps
        assert_eq!(
            engine.resolve(&seq("C-x C-c")),
            Some(Lookup::Command("quit"))
        );
    }

    #[test]
    fn engine_prefix_pending_stays_in_view_map() {
        // View has "C-x o"; global has "C-x C-c". Pressing C-x must go
        // pending (in the view map, where C-x o lives), not fall through.
        let mut g = KeyMap::new();
        g.bind(&seq("C-x C-c"), "quit").unwrap();
        let mut v = KeyMap::new();
        v.bind(&seq("C-x o"), "open-scratch").unwrap();
        let engine = KeymapEngine::new(g, v);
        assert_eq!(engine.resolve(&seq("C-x")), Some(Lookup::Pending));
        assert_eq!(
            engine.resolve(&seq("C-x o")),
            Some(Lookup::Command("open-scratch"))
        );
        assert_eq!(
            engine.resolve(&seq("C-x C-c")),
            Some(Lookup::Command("quit"))
        );
    }

    #[test]
    fn command_pairs_enumerates_all_leaves_deterministically() {
        let mut km = KeyMap::new();
        km.bind(&seq("q"), "quit").unwrap();
        km.bind(&seq("C-x C-f"), "find-file").unwrap();
        km.bind(&seq("C-x o"), "open-scratch").unwrap();
        km.bind(&seq("C-c p f"), "project-find").unwrap();
        let pairs = km.command_pairs();
        // Every leaf binding is present exactly once (as a set of names).
        let names: std::collections::HashSet<&str> =
            pairs.iter().map(|(_, c)| c.as_str()).collect();
        assert_eq!(
            names,
            ["quit", "find-file", "open-scratch", "project-find"]
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
            "all leaf commands must be enumerated"
        );
        assert_eq!(pairs.len(), 4, "no duplicates: {pairs:?}");
        // Deterministic: two enumerations agree.
        assert_eq!(km.command_pairs(), pairs);
        // The right command sits under the right sequence.
        let by_seq = |s: &str| pairs.iter().find(|(seq, _)| seq == &parse_sequence(s).unwrap());
        assert_eq!(by_seq("q").unwrap().1, "quit");
        assert_eq!(by_seq("C-x o").unwrap().1, "open-scratch");
        assert_eq!(by_seq("C-c p f").unwrap().1, "project-find");
    }

    /// Equivalence test: the declarative binding tables produce the same
    /// keymap as the old imperative `bind()` calls. For every table entry,
    /// `km.lookup(parse_sequence(seq))` resolves to the named command, and
    /// the total binding count is the A2 pre-change count (142) plus the
    /// bindings added since: 143 (the `C-c n a` annotations picker), then
    /// 144 (jump-ambiguity's `M-END` → point-buffer-end, the binding that
    /// frees `M->` for the force-definition-list hotkey — it lands in the
    /// BUFFER view next to `M-.`, not in the global table, so the global
    /// count stays 23 and the per-view total moves 120 → 121) — restate
    /// the history, never renumber it, or a future lane can "fix" drift by
    /// bumping this number again.
    #[test]
    fn load_bindings_equivalence() {
        use crate::app::store::{
            BLAME_BINDINGS, BUFFER_BINDINGS, BUFFER_LIST_BINDINGS, COMMIT_DIFF_BINDINGS,
            COMMIT_EDITOR_BINDINGS, GLOBAL_BINDINGS, HOME_BINDINGS, LOG_BINDINGS,
            MAGIT_STATUS_BINDINGS, SEARCH_BINDINGS,
        };

        // Global map: 23 bindings, every entry resolves correctly.
        let global = load_bindings(GLOBAL_BINDINGS);
        assert_eq!(global.command_pairs().len(), 23, "global binding count");
        for (seq_str, cmd) in GLOBAL_BINDINGS {
            let seq = parse_sequence(seq_str).unwrap();
            assert_eq!(
                global.lookup(&seq),
                Some(Lookup::Command(cmd)),
                "global: `{seq_str}` should resolve to `{cmd}`"
            );
        }

        // Per-view maps: 121 total bindings across 9 views (120 plus the
        // jump-ambiguity `M-END` buffer-view binding).
        let views: &[(&str, &[(&str, &str)])] = &[
            ("Buffer", BUFFER_BINDINGS),
            ("BufferList", BUFFER_LIST_BINDINGS),
            ("MagitStatus", MAGIT_STATUS_BINDINGS),
            ("Log", LOG_BINDINGS),
            ("Blame", BLAME_BINDINGS),
            ("CommitDiff", COMMIT_DIFF_BINDINGS),
            ("CommitEditor", COMMIT_EDITOR_BINDINGS),
            ("Home", HOME_BINDINGS),
            ("Search", SEARCH_BINDINGS),
        ];
        let total_per_view: usize = views.iter().map(|(_, t)| t.len()).sum();
        assert_eq!(total_per_view, 121, "total per-view bindings must be 121");
        for (name, table) in views {
            let km = load_bindings(table);
            assert_eq!(km.command_pairs().len(), table.len(), "{name} binding count");
            for (seq_str, cmd) in *table {
                let seq = parse_sequence(seq_str).unwrap();
                assert_eq!(
                    km.lookup(&seq),
                    Some(Lookup::Command(cmd)),
                    "{name}: `{seq_str}` should resolve to `{cmd}`"
                );
            }
        }

        // Representative spot checks.
        assert_eq!(
            global.lookup(&parse_sequence("C-x C-c").unwrap()),
            Some(Lookup::Command("quit")),
            "C-x C-c → quit"
        );
        let buffer_km = load_bindings(BUFFER_BINDINGS);
        assert_eq!(
            buffer_km.lookup(&parse_sequence("q").unwrap()),
            Some(Lookup::Command("close-view")),
            "q in Buffer → close-view"
        );
    }
}
