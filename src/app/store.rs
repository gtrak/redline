//! Central app store: the single source of truth the ui layer renders
//! and updates. Holds the view stack, keymap state, pending key
//! sequence, minibuffer message, status line state, quit flag, and the
//! picker overlay state.

use nucleo_matcher::{
    Matcher, pattern::{CaseMatching, Normalization, Pattern},
};
use ropey::Rope;

use crate::app::command::{CommandRegistry, RegistryError};
use crate::app::config::Config;
use crate::app::keymap::{Key, KeyCode, KeyMap, KeySeq, KeymapEngine, Lookup, parse_sequence};
use crate::theme::Theme;

/// A view on the stack. Only `Scratch` exists for issue 01; later
/// issues add `Buffer`, `GitStatus`, …
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewId {
    Scratch,
}

impl ViewId {
    pub fn name(self) -> &'static str {
        match self {
            ViewId::Scratch => "scratch",
        }
    }

    fn keymap(self) -> KeyMap {
        match self {
            ViewId::Scratch => {
                let mut km = KeyMap::new();
                km.bind(&[Key::char('q')], "quit").unwrap();
                km
            }
        }
    }
}

/// One candidate in the picker (palette entries are commands). Nucleo
/// matches against the command name (emacs M-x semantics: typing filters
/// by command name, docs are display-only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerCandidate {
    pub name: String,
    pub docs: String,
    pub category: String,
}

/// Picker overlay state. The filtered list is recomputed in the store
/// (not the ui) so navigation keys can move through it headlessly.
#[derive(Default, Debug)]
struct Picker {
    prompt: String,
    query: String,
    selected: usize,
    /// (candidate, nucleo score) for the current query, best-first.
    filtered: Vec<(PickerCandidate, u32)>,
}

pub struct AppStore {
    pub theme: Theme,
    pub registry: CommandRegistry,
    pub engine: KeymapEngine,
    /// View stack; the top of the stack is what the main view renders.
    pub view_stack: Vec<ViewId>,
    /// Buffer contents keyed by view (scratch text for issue 01).
    scratch: Rope,
    /// Strict prefix of a key sequence pressed so far (shown in the
    /// status line), e.g. `[C-x]`.
    pub pending: KeySeq,
    pub quit: bool,
    /// Project placeholder until issue 02.
    pub project: String,
    /// Async-activity indicator slots (status line, e.g. `*indexing`).
    pub activity: Vec<String>,
    /// Minibuffer message (the echo area).
    pub message: String,
    picker: Option<Picker>,
    /// Long-lived nucleo matcher: ~135KB of scratch, built once, never
    /// per keystroke (nucleo skill gotcha #1).
    matcher: Matcher,
}

impl AppStore {
    /// A store with default config: default theme, seed registry, and
    /// the default global + scratch keymaps.
    pub fn new() -> Self {
        let registry = CommandRegistry::seed();

        let mut global = KeyMap::new();
        global.bind(&[Key::ctrl_char('g')], "cancel").unwrap();
        global.bind(&[Key::alt_char('x')], "open-palette").unwrap();
        // C-x C-c (quit): bare C-x stays a prefix (pending), so both the
        // C-x C-c global binding and the view-map C-x o / C-x C-i
        // bindings remain reachable.
        global
            .bind(&[Key::ctrl_char('x'), Key::ctrl_char('c')], "quit")
            .unwrap();
        global.bind(&[Key::alt_char('s')], "cycle-view-next").unwrap();
        global.bind(&[Key::alt_char('p')], "cycle-view-prev").unwrap();

        let mut scratch = KeyMap::new();
        scratch.bind(&[Key::char('q')], "quit").unwrap();
        scratch.bind(&[Key::alt_char('o')], "open-scratch").unwrap();
        scratch
            .bind(&[Key::ctrl_char('x'), Key::char('o')], "open-scratch")
            .unwrap();
        scratch
            .bind(&[Key::ctrl_char('x'), Key::ctrl_char('i')], "insert-demo-text")
            .unwrap();

        let engine = KeymapEngine::new(global, ViewId::Scratch.keymap());

        Self {
            theme: Theme::default(),
            registry,
            engine,
            view_stack: vec![ViewId::Scratch],
            scratch: Rope::new(),
            pending: Vec::new(),
            quit: false,
            project: String::from("no project"),
            activity: Vec::new(),
            message: String::new(),
            picker: None,
            matcher: Matcher::new(nucleo_matcher::Config::DEFAULT),
        }
    }

    /// Apply a config: select the theme and (re)bind key overrides.
    /// Returns an error describing the first unbindable override.
    pub fn apply_config(&mut self, config: &Config) -> Result<(), String> {
        self.theme = Theme::from(config.theme);
        for (command, sequence) in &config.key_bindings {
            let seq = parse_sequence(sequence)
                .map_err(|e| format!("`{command}`: invalid key sequence `{sequence}`: {e}"))?;
            self.engine.global.bind(&seq, command).map_err(|e| {
                format!("`{command}`: cannot bind `{sequence}`: {e}")
            })?;
        }
        Ok(())
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn top_view(&self) -> ViewId {
        *self.view_stack.last().expect("view stack is never empty")
    }

    pub fn view_name(&self) -> &'static str {
        self.top_view().name()
    }

    pub fn view_name_display(&self) -> String {
        match self.top_view() {
            ViewId::Scratch => "*scratch*".to_string(),
        }
    }

    pub fn project_display(&self) -> &str {
        &self.project
    }

    pub fn pending_display(&self) -> String {
        self.pending
            .iter()
            .map(|k| k.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn activity_display(&self) -> String {
        self.activity.join(" ")
    }

    pub fn current_text(&self) -> String {
        self.scratch.to_string()
    }

    pub fn push_view(&mut self, view: ViewId) {
        let view_km = view.keymap();
        self.engine = KeymapEngine::new(self.engine.global.clone(), view_km);
        self.view_stack.push(view);
    }

    /// Rotate the view stack by `delta` positions (positive: the top
    /// view moves toward the back; negative: the back moves to the top).
    pub fn cycle_view(&mut self, delta: i64) {
        let n = self.view_stack.len() as i64;
        if n < 2 {
            return;
        }
        let steps = (delta.rem_euclid(n)) as usize;
        if steps > 0 {
            let start = (n - steps as i64) as usize;
            let moved: Vec<ViewId> = self.view_stack.drain(start..).collect();
            self.view_stack.extend(moved);
        }
        // Re-assert the top view's keymap after rotating.
        let view_km = self.top_view().keymap();
        self.engine = KeymapEngine::new(self.engine.global.clone(), view_km);
    }

    /// Insert text into the scratch buffer; false if the top view is not
    /// the scratch view.
    pub fn insert_text(&mut self, text: &str) -> bool {
        if self.top_view() != ViewId::Scratch {
            return false;
        }
        let pos = self.scratch.chars().count();
        self.scratch.insert(pos, text);
        true
    }

    /// Feed one keypress from the terminal. While the picker is open,
    /// printable characters extend the query and a small set of keys
    /// drives the picker (RET runs, C-g cancels, arrows / C-n / C-p
    /// move); everything else goes to the keymap engine.
    pub fn key_event(&mut self, key: Key) {
        if self.quit {
            return;
        }
        if self.picker.is_some() {
            if let Some(c) = key.char_value() {
                self.picker_query_char(c);
                return;
            }
            if key.code == KeyCode::Enter {
                self.run_selected();
                return;
            }
            if key.code == KeyCode::Down || key == Key::ctrl_char('n') {
                self.picker_select_next();
                return;
            }
            if key.code == KeyCode::Up || key == Key::ctrl_char('p') {
                self.picker_select_prev();
                return;
            }
            // fall through to the keymap engine for other keys
        }
        // C-g aborts from any state: it clears the pending sequence and
        // closes the picker before the keymap engine can start one.
        if key == Key::ctrl_char('g') {
            self.cancel();
            return;
        }
        self.dispatch_key(key);
    }

    fn dispatch_key(&mut self, key: Key) {
        let mut seq = self.pending.clone();
        seq.push(key);
        match self.engine.resolve(&seq) {
            Some(Lookup::Command(cmd)) => {
                let cmd = cmd.to_string();
                self.pending.clear();
                let _ = self.dispatch(&cmd, None);
            }
            Some(Lookup::Pending) => {
                self.pending = seq;
            }
            None => {
                self.pending.clear();
                if self.picker.is_none() {
                    self.minibuffer_message(&format!("unbound key: {key}"));
                }
            }
        }
    }

    /// Dispatch-by-name through the store's own registry: clone the
    /// command out (the registry is a field of this store, so the
    /// handler may not borrow it while running), then run it here.
    pub fn dispatch(&mut self, name: &str, arg: Option<String>) -> Result<(), RegistryError> {
        let command = self
            .registry
            .get(name)
            .cloned()
            .ok_or_else(|| RegistryError::UnknownCommand(name.to_string()))?;
        command.run(self, arg);
        Ok(())
    }

    /// C-g semantics: clear a pending sequence, close the picker, and
    /// echo a cancel notice in the minibuffer.
    pub fn cancel(&mut self) {
        let mut did = false;
        if !self.pending.is_empty() {
            self.pending.clear();
            did = true;
        }
        if self.picker.take().is_some() {
            did = true;
        }
        if did {
            self.minibuffer_message("cancel");
        }
    }

    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }

    pub fn minibuffer_message(&mut self, msg: &str) {
        self.message = msg.to_string();
    }

    /// Open the `M-x` palette over the command registry.
    pub fn open_palette(&mut self) {
        let candidates: Vec<PickerCandidate> = self
            .registry
            .list()
            .map(|c| PickerCandidate {
                name: c.name.to_string(),
                docs: c.docs.to_string(),
                category: c.category.to_string(),
            })
            .collect();
        let mut picker = Picker {
            prompt: "M-x ".to_string(),
            query: String::new(),
            selected: 0,
            filtered: Vec::new(),
        };
        picker.recompute(&candidates, &mut self.matcher);
        self.picker = Some(picker);
    }

    pub fn picker_open(&self) -> bool {
        self.picker.is_some()
    }

    pub fn picker_prompt(&self) -> &str {
        self.picker.as_ref().map(|p| p.prompt.as_str()).unwrap_or("")
    }

    pub fn picker_query(&self) -> &str {
        self.picker.as_ref().map(|p| p.query.as_str()).unwrap_or("")
    }

    /// The filtered candidates for the current query, best-first.
    pub fn picker_filtered(&self) -> &[(PickerCandidate, u32)] {
        self.picker
            .as_ref()
            .map(|p| p.filtered.as_slice())
            .unwrap_or(&[])
    }

    pub fn picker_count(&self) -> (usize, usize) {
        let total = self.registry.list().count();
        match &self.picker {
            Some(p) => (p.filtered.len(), total),
            None => (0, total),
        }
    }

    pub fn picker_selected(&self) -> usize {
        self.picker.as_ref().map(|p| p.selected).unwrap_or(0)
    }

    /// Run the candidate the picker has selected (RET in the palette).
    pub fn run_selected(&mut self) {
        let name = self
            .picker
            .as_ref()
            .and_then(|p| p.filtered.get(p.selected))
            .map(|(c, _)| c.name.clone());
        self.picker = None;
        match name {
            Some(name) => {
                let _ = self.dispatch(&name, None);
            }
            None => self.minibuffer_message("no command selected"),
        }
    }

    pub fn picker_select_next(&mut self) {
        if let Some(p) = self.picker.as_mut() && !p.filtered.is_empty() {
            p.selected = (p.selected + 1) % p.filtered.len();
        }
    }

    pub fn picker_select_prev(&mut self) {
        if let Some(p) = self.picker.as_mut() && !p.filtered.is_empty() {
            // Wrap-decrement: prev at index 0 lands on the last candidate.
            p.selected = (p.selected + p.filtered.len() - 1) % p.filtered.len();
        }
    }

    fn picker_query_char(&mut self, c: char) {
        if let Some(p) = self.picker.as_mut() {
            p.query.push(c);
            let candidates: Vec<PickerCandidate> = self
                .registry
                .list()
                .map(|c| PickerCandidate {
                    name: c.name.to_string(),
                    docs: c.docs.to_string(),
                    category: c.category.to_string(),
                })
                .collect();
            p.recompute(&candidates, &mut self.matcher);
            p.selected = p.selected.min(p.filtered.len().saturating_sub(1));
        }
    }
}

impl Default for AppStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Picker {
    fn recompute(&mut self, candidates: &[PickerCandidate], matcher: &mut Matcher) {
        let query = self.query.as_str();
        if query.is_empty() {
            self.filtered = candidates
                .iter()
                .map(|c| (c.clone(), u32::MAX))
                .collect();
            return;
        }
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let mut buf = Vec::new();
        let mut scored: Vec<(&PickerCandidate, u32)> = candidates
            .iter()
            .filter_map(|c| {
                let haystack = nucleo_matcher::Utf32Str::new(&c.name, &mut buf);
                pattern
                    .score(haystack, matcher)
                    .map(|score| (c, score))
            })
            .collect();
        scored.sort_by_key(|item| std::cmp::Reverse(item.1));
        self.filtered = scored.into_iter().map(|(c, s)| (c.clone(), s)).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::keymap::parse_key;

    fn key(s: &str) -> Key {
        parse_key(s).unwrap()
    }

    #[test]
    fn unknown_keys_echo_in_minibuffer() {
        let mut store = AppStore::new();
        store.key_event(key("Backspace"));
        assert_eq!(store.message, "unbound key: DEL");
        assert!(store.pending.is_empty());
    }

    #[test]
    fn pending_prefix_shows_and_cancels() {
        let mut store = AppStore::new();
        store.key_event(key("C-x"));
        assert_eq!(store.pending_display(), "C-x");
        assert!(store.message.is_empty());

        store.key_event(key("C-g"));
        assert!(store.pending.is_empty());
        assert_eq!(store.message, "cancel");
    }

    #[test]
    fn pending_then_full_match_dispatches() {
        let mut store = AppStore::new();
        store.key_event(key("C-x"));
        assert_eq!(store.pending_display(), "C-x");
        store.key_event(key("C-c"));
        assert!(store.quit);
        assert!(store.pending.is_empty());
    }

    #[test]
    fn unknown_key_after_prefix_cancels_pending() {
        let mut store = AppStore::new();
        store.key_event(key("C-x"));
        assert_eq!(store.pending_display(), "C-x");
        store.key_event(key("C-z"));
        assert!(store.pending.is_empty());
        assert_eq!(store.message, "unbound key: C-z");
    }

    #[test]
    fn per_view_q_beats_global_but_global_still_reaches() {
        let mut store = AppStore::new();
        store.key_event(key("q"));
        assert!(store.quit, "view-local `q` must bind quit");

        let mut store = AppStore::new();
        store.key_event(key("M-x"));
        assert!(store.picker_open());
    }

    #[test]
    fn picker_query_filters_candidates() {
        let mut store = AppStore::new();
        store.open_palette();
        assert_eq!(store.picker_count().0, 10);

        store.key_event(key("q"));
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["quit"], "{names:?}");

        // Backspace (not printable-appended) goes to the keymap: here
        // it is unbound and the picker stays open with its query intact.
        store.key_event(key("Backspace"));
        assert!(store.picker_open());
        assert_eq!(store.picker_query(), "q");
    }

    #[test]
    fn palette_navigation_and_run() {
        let mut store = AppStore::new();
        store.open_palette();
        assert_eq!(store.picker_count().0, 10);

        // Shipped UI path (M-x, Down, Up): Up must wrap-decrement, not
        // reflect — prev(1) is 0, not 8.
        store.key_event(key("DOWN"));
        assert_eq!(store.picker_selected(), 1);
        store.key_event(key("UP"));
        assert_eq!(store.picker_selected(), 0);

        // Mid-list: Up from an interior index decrements by exactly one.
        store.picker_select_next();
        store.picker_select_next(); // 0 -> 2
        store.picker_select_prev();
        assert_eq!(store.picker_selected(), 1);

        // Wrap at top: Up at index 0 lands on the last candidate.
        store.picker_select_prev(); // 1 -> 0
        store.picker_select_prev();
        assert_eq!(store.picker_selected(), 9);

        // C-p goes through the same wrap-decrement path as Up.
        store.key_event(key("C-p"));
        assert_eq!(store.picker_selected(), 8);

        // RET runs the candidate at the selected index (8: demo-message-2).
        store.key_event(key("RET"));
        assert!(!store.picker_open());
        assert_eq!(store.message, "hello from the command registry (2)");
    }

    #[test]
    fn palette_ret_runs_selected_command() {
        let mut store = AppStore::new();
        store.open_palette();
        // Seed order: quit is first.
        store.key_event(key("RET"));
        assert!(!store.picker_open());
        assert!(store.quit);
    }

    #[test]
    fn palette_c_g_closes() {
        let mut store = AppStore::new();
        store.open_palette();
        store.key_event(key("C-g"));
        assert!(!store.picker_open());
        assert_eq!(store.message, "cancel");
    }
}
