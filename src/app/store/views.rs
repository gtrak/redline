use super::*;

impl AppStore {
    pub fn push_view(&mut self, view: ViewId) {
        let view_km = view.keymap();
        self.engine = KeymapEngine::new(self.engine.global.clone(), view_km);
        self.view_stack.push(view);
    }

    /// Pop the top view (if a non-root view is on top) and restore the
    /// new top view's keymap (re-normalizing home ⇄ buffer, 06a).
    pub fn close_view(&mut self) {
        if self.view_stack.len() > 1 {
            self.view_stack.pop();
            self.buffer_list_selected = 0;
            self.normalize_top_view();
        }
    }

    /// C-x 1 (emacs's `only-this-window`) on the view-stack model: close
    /// every view except the current buffer view — the stack collapses to
    /// exactly `[Buffer]`. Bound only in the buffer view's keymap (and that
    /// view renders only WITH a current buffer, 06a), so the buffer view
    /// always exists; when it is already the only view the command is a
    /// no-op, like emacs's C-x 1 on a single window.
    pub fn close_other_views(&mut self) {
        if self.top_view() != ViewId::Buffer || self.view_stack.len() <= 1 {
            return;
        }
        self.view_stack = vec![ViewId::Buffer];
        self.buffer_list_selected = 0;
        self.normalize_top_view();
    }

    /// C-x 2 (emacs's `split-window-vertically`) on the single-pane
    /// view-stack model: there is no split layout yet (the scoped
    /// follow-up — a real split needs per-pane buffer / point / scroll /
    /// edit-mode state, a second render pane, and window-focus cycling),
    /// so the key is bound and reports the model instead of dead-ending:
    /// no state change (buffer, point, and view stack all untouched), the
    /// note lands in the minibuffer.
    pub fn split_window_vertical(&mut self) {
        self.minibuffer_message(
            "single pane by design: redline runs as a herdr popup, one pane",
        );
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
        // Re-assert the top view's keymap after rotating (and re-normalize
        // home ⇄ buffer, 06a).
        self.normalize_top_view();
    }

    /// Whether the transient menu overlay is open.
    pub fn menu_open(&self) -> bool {
        self.menu.open
    }

    /// The current submenu path (empty at the top level; test-only accessor,
    /// no production caller).
    #[allow(dead_code)] // test-only accessor
    pub fn menu_path(&self) -> &KeySeq {
        &self.menu.path
    }

    /// `?` (any view) / `h` (magit views): open the transient menu at the top
    /// level.
    pub fn open_menu(&mut self) {
        self.menu.open = true;
        self.menu.path.clear();
        self.pending.clear();
    }

    fn close_menu(&mut self) {
        self.menu.open = false;
        self.menu.path.clear();
    }

    /// The active view's effective bindings: the view map plus the global
    /// map, with the view map winning on a shared sequence (mirroring
    /// `KeymapEngine::resolve`). Source of truth for the menu — no
    /// hand-written table, so the menu cannot drift from the keymap.
    pub(super) fn menu_bindings(&self) -> Vec<(KeySeq, String)> {
        let mut map: std::collections::HashMap<Vec<Key>, String> = self
            .engine
            .global
            .command_pairs()
            .into_iter()
            .collect();
        for (s, c) in self.engine.view.command_pairs() {
            map.insert(s, c);
        }
        map.into_iter().collect()
    }

    /// The menu entries at `path`: each is the next key and either a leaf
    /// (a full binding) or a prefix (a strict prefix of a longer binding).
    pub fn menu_entries_for_path(&self, path: &KeySeq) -> Vec<TransientMenuEntry> {
        let bindings = self.menu_bindings();
        let mut entries: Vec<(Key, Option<String>)> = Vec::new();
        for (seq, cmd) in &bindings {
            if seq.len() > path.len() && seq[..path.len()] == *path {
                let next = seq[path.len()];
                // A node is either a command or a prefix, never both: a
                // one-key extension is a leaf, a longer one a prefix.
                let is_leaf = seq.len() == path.len() + 1;
                entries.push((next, if is_leaf { Some(cmd.clone()) } else { None }));
            }
        }
        // Deterministic order + leaf-wins on a key that is both a leaf and a
        // prefix (the engine resolves the view's leaf first).
        entries.sort_by(|a, b| {
            a.0.to_string().cmp(&b.0.to_string()).then(a.1.is_none().cmp(&b.1.is_none()))
        });
        entries.dedup_by(|a, b| a.0 == b.0);
        entries
            .into_iter()
            .map(|(key, cmd)| {
                let mut key_display = path
                    .iter()
                    .map(|k| k.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                if !key_display.is_empty() {
                    key_display.push(' ');
                }
                key_display.push_str(&key.to_string());
                match cmd {
                    Some(name) => {
                        let meta = self.registry.get(&name);
                        TransientMenuEntry {
                            key,
                            key_display,
                            command: Some(name.clone()),
                            is_prefix: false,
                            docs: meta.map(|m| m.docs.to_string()).unwrap_or_default(),
                            category: meta.map(|m| m.category.to_string()).unwrap_or_default(),
                        }
                    }
                    None => TransientMenuEntry {
                        key,
                        key_display,
                        command: None,
                        is_prefix: true,
                        docs: String::new(),
                        category: String::new(),
                    },
                }
            })
            .collect()
    }

    /// The menu entries at the current submenu path (test-only accessor,
    /// no production caller).
    #[allow(dead_code)] // test-only accessor
    pub fn menu_entries(&self) -> Vec<TransientMenuEntry> {
        self.menu_entries_for_path(&self.menu.path)
    }

    /// The menu's display rows for the current path: prefix (submenu) entries
    /// first, then leaf commands grouped by registry category and sorted
    /// within each group. A header row marks each group.
    pub fn menu_rows(&self) -> Vec<TransientMenuRow> {
        let entries = self.menu_entries_for_path(&self.menu.path);
        let mut rows = Vec::new();

        // Submenus (prefix keys) come first, as `KEY …` rows.
        let prefixes: Vec<&TransientMenuEntry> =
            entries.iter().filter(|e| e.is_prefix).collect();
        if !prefixes.is_empty() {
            rows.push(TransientMenuRow {
                is_header: true,
                key_display: String::new(),
                label: "submenus".into(),
                is_prefix: false,
            });
            for e in &prefixes {
                rows.push(TransientMenuRow {
                    is_header: false,
                    key_display: e.key_display.clone(),
                    label: String::new(),
                    is_prefix: true,
                });
            }
        }

        // Leaf commands grouped by registry category (sorted), then sorted by
        // key within each group.
        let mut by_cat: std::collections::BTreeMap<String, Vec<&TransientMenuEntry>> =
            std::collections::BTreeMap::new();
        for e in &entries {
            if !e.is_prefix {
                by_cat.entry(e.category.clone()).or_default().push(e);
            }
        }
        for (cat, mut es) in by_cat {
            es.sort_by(|a, b| a.key_display.cmp(&b.key_display));
            rows.push(TransientMenuRow {
                is_header: true,
                key_display: String::new(),
                label: cat,
                is_prefix: false,
            });
            for e in es {
                rows.push(TransientMenuRow {
                    is_header: false,
                    key_display: e.key_display.clone(),
                    label: e.docs.clone(),
                    is_prefix: false,
                });
            }
        }
        rows
    }

    /// The overlay's row budget: the menu's row count (title + rows), capped
    /// at the viewport height so the menu never exceeds the frame.
    pub fn menu_height(&self) -> u32 {
        (self.menu_rows().len() + 1).min(self.viewport_lines) as u32
    }

    /// The home view's derived body rows (plan 004 issue 06a): every
    /// EFFECTIVE binding (the live keymap × global map, via `menu_bindings` —
    /// on home the view map is empty, so these are the commands that work
    /// from home), grouped by registry category and sorted by key within
    /// each group — the same grouping `menu_rows` uses, queried at the
    /// TOP level of the whole keymap (the "all top-level groups" query;
    /// no new formatter, no hand-maintained list).
    pub fn home_rows(&self) -> Vec<TransientMenuRow> {
        let bindings = self.menu_bindings();
        let mut by_cat: std::collections::BTreeMap<String, Vec<(String, String)>> =
            std::collections::BTreeMap::new();
        for (seq, cmd) in bindings {
            let key_display = seq
                .iter()
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            let (docs, category) = match self.registry.get(&cmd) {
                Some(m) => (m.docs.to_string(), m.category.to_string()),
                None => (String::new(), String::new()),
            };
            by_cat.entry(category).or_default().push((key_display, docs));
        }
        let mut rows = Vec::new();
        for (cat, mut entries) in by_cat {
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            rows.push(TransientMenuRow {
                is_header: true,
                key_display: String::new(),
                label: cat,
                is_prefix: false,
            });
            for (key_display, docs) in entries {
                rows.push(TransientMenuRow {
                    is_header: false,
                    key_display,
                    label: docs,
                    is_prefix: false,
                });
            }
        }
        rows
    }

    /// The home view's header: `redline · {project}` + the dirty counts
    /// (`+` staged, `~` unstaged, `?` untracked — the status-line
    /// convention) when git reports any.
    pub fn home_title(&self) -> String {
        let mut title = format!("redline · {}", self.project_display());
        if let Some(d) = self.dirty_counts()
            && d.staged + d.unstaged + d.untracked > 0
        {
            title.push_str(&format!("  +{} ~{} ?{}", d.staged, d.unstaged, d.untracked));
        }
        title
    }

    /// The home body rows bounded to the viewport (06a): the terminal is
    /// `viewport_lines + 3` rows (the resize handler's contract: viewport =
    /// height - 3), the main view is `viewport_lines + 1` of those
    /// (minibuffer + status aside); home keeps its title + help chrome
    /// (2 rows), and an open overlay (the 12-row picker canvas, the menu)
    /// takes its fixed height — the body yields it, the way the file
    /// view's canvas yields the same space.
    pub fn home_body_rows(&self) -> Vec<TransientMenuRow> {
        let overlay = if self.picker_open() {
            12 // the picker's fixed canvas height (src/ui/picker.rs)
        } else if self.menu_open() {
            self.menu_height() as usize
        } else {
            0
        };
        let budget = self
            .viewport_lines
            .saturating_sub(1)
            .saturating_sub(overlay);
        self.home_rows().into_iter().take(budget).collect()
    }

    /// Handle a key while the menu is open: C-g closes; a listed leaf closes
    /// the menu and runs its command; a listed prefix descends; anything else
    /// is swallowed (the menu ignores non-listed keys).
    pub(super) fn menu_key_event(&mut self, key: Key) {
        if key == Key::ctrl_char('g') {
            self.close_menu();
            self.minibuffer_message("cancel");
            return;
        }
        let entries = self.menu_entries_for_path(&self.menu.path);
        if let Some(entry) = entries.iter().find(|e| e.key == key) {
            match &entry.command {
                Some(cmd) => {
                    let cmd = cmd.clone();
                    self.close_menu();
                    let _ = self.dispatch(&cmd, None);
                }
                None => {
                    self.menu.path.push(key);
                }
            }
        }
        // else: swallowed (ignored).
    }
}
