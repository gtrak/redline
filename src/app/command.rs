//! Command registry: every interactive action is a named, documented
//! command with a category and a handler. Keymaps bind key sequences to
//! command names; the `M-x` palette is a picker over this registry.
//!
//! Handlers run on the `AppStore` that owns the registry. Because of that
//! ownership, dispatch extracts a (cheap, `Arc`-backed) `Command` from the
//! registry and runs it on the store — see `AppStore::dispatch`.

use std::sync::Arc;

use std::collections::HashMap;

use crate::app::store::{AppStore, ViewId};

/// Handler for a command. `arg` carries an optional argument (e.g. the
/// text of a `message-echo <text>` demo). `Send + Sync` so the store
/// (which owns the registry) can cross threads into the render loop.
pub type CommandHandler = Arc<dyn Fn(&mut AppStore, Option<String>) + Send + Sync>;

#[derive(Clone)]
pub struct Command {
    pub name: &'static str,
    pub docs: &'static str,
    pub category: &'static str,
    handler: CommandHandler,
}

impl Command {
    pub fn new(
        name: &'static str,
        docs: &'static str,
        category: &'static str,
        handler: impl Fn(&mut AppStore, Option<String>) + Send + Sync + 'static,
    ) -> Self {
        Self {
            name,
            docs,
            category,
            handler: Arc::new(handler),
        }
    }

    /// Run this command against a store. The store may own the registry
    /// the command came from: the command was extracted (cloned) first.
    pub fn run(&self, store: &mut AppStore, arg: Option<String>) {
        (self.handler)(store, arg);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    UnknownCommand(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::UnknownCommand(name) => write!(f, "unknown command `{name}`"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// Registry of all commands, kept in insertion order for the palette.
#[derive(Default)]
pub struct CommandRegistry {
    commands: HashMap<&'static str, Command>,
    order: Vec<&'static str>,
}

impl CommandRegistry {
    pub fn register(&mut self, command: Command) {
        let name = command.name;
        self.commands.entry(name).or_insert_with(|| {
            self.order.push(name);
            command
        });
    }

    /// All commands in registry order (palette candidates).
    pub fn list(&self) -> impl Iterator<Item = &Command> {
        self.order.iter().filter_map(|name| self.commands.get(name))
    }

    pub fn get(&self, name: &str) -> Option<&Command> {
        self.commands.get(name)
    }

    /// Dispatch-by-name against a standalone registry (the store does not
    /// need to be the one that owns this registry). In-app dispatch is
    /// `AppStore::dispatch`, which clones the `Command` out of the store's
    /// own registry first.
    #[allow(dead_code)] // unit-tested here; in-app dispatch is AppStore::dispatch
    pub fn dispatch_by_name(
        &self,
        store: &mut AppStore,
        name: &str,
        arg: Option<String>,
    ) -> Result<(), RegistryError> {
        let command = self
            .get(name)
            .ok_or_else(|| RegistryError::UnknownCommand(name.to_string()))?;
        command.run(store, arg);
        Ok(())
    }

    /// The ~10 placeholder commands shipped with issue 01.
    pub fn seed() -> Self {
        let mut reg = Self::default();
        reg.register(Command::new(
            "quit",
            "Quit redline",
            "navigation",
            |store, _arg| {
                store.clear_pending();
                store.quit = true;
            },
        ));
        reg.register(Command::new(
            "cancel",
            "Cancel a pending key sequence or close the picker (C-g)",
            "navigation",
            |store, _arg| store.cancel(),
        ));
        reg.register(Command::new(
            "open-palette",
            "Run a command: list all commands with M-x",
            "navigation",
            |store, _arg| store.open_palette(),
        ));
        reg.register(Command::new(
            "cycle-view-next",
            "Rotate the view stack forward",
            "navigation",
            |store, _arg| {
                store.cycle_view(1);
                store.minibuffer_message(&format!("View: {}", store.view_name()));
            },
        ));
        reg.register(Command::new(
            "cycle-view-prev",
            "Rotate the view stack backwards",
            "navigation",
            |store, _arg| {
                store.cycle_view(-1);
                store.minibuffer_message(&format!("View: {}", store.view_name()));
            },
        ));
        reg.register(Command::new(
            "open-scratch",
            "Switch to the *scratch* buffer",
            "buffers",
            |store, _arg| store.open_scratch(),
        ));
        reg.register(Command::new(
            "message-echo",
            "Echo its argument in the minibuffer (demo)",
            "demo",
            |store, arg| store.minibuffer_message(&arg.unwrap_or_default()),
        ));
        reg.register(Command::new(
            "demo-message-1",
            "Show demo message one (placeholder)",
            "demo",
            |store, _arg| store.minibuffer_message("hello from the command registry (1)"),
        ));
        reg.register(Command::new(
            "demo-message-2",
            "Show demo message two (placeholder)",
            "demo",
            |store, _arg| store.minibuffer_message("hello from the command registry (2)"),
        ));
        reg.register(Command::new(
            "insert-demo-text",
            "Insert demo text into the current buffer",
            "demo",
            |store, _arg| {
                if store.insert_text("demo text\n") {
                    store.minibuffer_message("inserted demo text");
                } else {
                    store.minibuffer_message("insert-demo-text: no buffer is current");
                }
            },
        ));
        // ── issue 02: browse layer ────────────────────────────────────────
        reg.register(Command::new(
            "find-file",
            "Find a file in the project (C-x C-f)",
            "files",
            |store, _arg| store.open_find_file(),
        ));
        reg.register(Command::new(
            "switch-buffer",
            "Switch to an open buffer (C-x b)",
            "buffers",
            |store, _arg| store.open_switch_buffer(),
        ));
        reg.register(Command::new(
            "list-buffers",
            "List open buffers (C-x C-b)",
            "buffers",
            |store, _arg| {
                store.push_view(ViewId::BufferList);
                store.minibuffer_message("buffer list");
            },
        ));
        reg.register(Command::new(
            "kill-buffer",
            "Kill a buffer (C-x k)",
            "buffers",
            |store, _arg| store.open_kill_buffer(),
        ));
        reg.register(Command::new(
            "switch-project",
            "Switch to another known project (C-c p p)",
            "project",
            |store, _arg| store.open_switch_project(),
        ));
        reg.register(Command::new(
            "recent-files",
            "Open a recently visited file (C-c p e)",
            "files",
            |store, _arg| store.open_recent_files(),
        ));
        reg.register(Command::new(
            "re-walk",
            "Re-walk the project's file list (C-c p i)",
            "files",
            |store, _arg| store.re_walk(),
        ));
        // ── issue 03: motion + isearch + goto-line ──────────────────────────
        reg.register(Command::new(
            "scroll-line-down",
            "Scroll down one line (C-n / j)",
            "motion",
            |store, _arg| store.scroll_line_down(),
        ));
        reg.register(Command::new(
            "scroll-line-up",
            "Scroll up one line (C-p / k)",
            "motion",
            |store, _arg| store.scroll_line_up(),
        ));
        reg.register(Command::new(
            "scroll-page-down",
            "Scroll down one page (C-v)",
            "motion",
            |store, _arg| store.scroll_page_down(),
        ));
        reg.register(Command::new(
            "scroll-page-up",
            "Scroll up one page (M-v)",
            "motion",
            |store, _arg| store.scroll_page_up(),
        ));
        reg.register(Command::new(
            "scroll-half-page-down",
            "Scroll down half a page (C-d)",
            "motion",
            |store, _arg| store.scroll_half_page_down(),
        ));
        reg.register(Command::new(
            "scroll-half-page-up",
            "Scroll up half a page (C-u)",
            "motion",
            |store, _arg| store.scroll_half_page_up(),
        ));
        reg.register(Command::new(
            "scroll-top",
            "Scroll to the top of the buffer (g / M-<)",
            "motion",
            |store, _arg| store.scroll_to_top(),
        ));
        reg.register(Command::new(
            "scroll-bottom",
            "Scroll to the bottom of the buffer (G / M->)",
            "motion",
            |store, _arg| store.scroll_to_bottom(),
        ));
        reg.register(Command::new(
            "goto-line",
            "Jump to a line number (M-g g)",
            "motion",
            |store, _arg| store.goto_line_start(),
        ));
        reg.register(Command::new(
            "isearch-forward",
            "Incremental search forward (C-s)",
            "search",
            |store, _arg| {
                store.isearch_start(crate::app::store::IsearchDirection::Forward);
            },
        ));
        reg.register(Command::new(
            "isearch-backward",
            "Incremental search backward (C-r)",
            "search",
            |store, _arg| {
                store.isearch_start(crate::app::store::IsearchDirection::Backward);
            },
        ));
        reg.register(Command::new(
            "close-view",
            "Close the top view (q in list views)",
            "navigation",
            |store, _arg| store.close_view(),
        ));
        reg.register(Command::new(
            "open-buffer-list-selected",
            "Open the buffer-list selection and close the list (RET)",
            "buffers",
            |store, _arg| store.open_buffer_list_selected(),
        ));
        reg.register(Command::new(
            "buffer-list-next",
            "Move the buffer-list selection down",
            "buffers",
            |store, _arg| store.buffer_list_next(),
        ));
        reg.register(Command::new(
            "buffer-list-prev",
            "Move the buffer-list selection up",
            "buffers",
            |store, _arg| store.buffer_list_prev(),
        ));
        // ── issue 07: magit status & staging ───────────────────────────
        reg.register(Command::new(
            "magit-status",
            "Show/refresh the git status buffer (C-x g)",
            "git",
            |store, _arg| store.open_magit_status(),
        ));
        reg.register(Command::new(
            "magit-stage",
            "Stage the file or hunk at point (s)",
            "git",
            |store, _arg| store.magit_stage(),
        ));
        reg.register(Command::new(
            "magit-unstage",
            "Unstage the file or hunk at point (u)",
            "git",
            |store, _arg| store.magit_unstage(),
        ));
        reg.register(Command::new(
            "magit-fold",
            "Fold/unfold the section at point (TAB)",
            "git",
            |store, _arg| store.magit_toggle_fold(),
        ));
        reg.register(Command::new(
            "magit-visit-file",
            "Visit the file at point in the buffer view (RET)",
            "git",
            |store, _arg| store.magit_visit_file(),
        ));
        reg.register(Command::new(
            "magit-refresh",
            "Manually refresh the git status buffer (g)",
            "git",
            |store, _arg| store.magit_refresh(),
        ));
        reg.register(Command::new(
            "magit-next",
            "Move to the next section in the status buffer (n / C-n)",
            "git",
            |store, _arg| store.magit_cursor_down(),
        ));
        reg.register(Command::new(
            "magit-prev",
            "Move to the previous section in the status buffer (p / C-p)",
            "git",
            |store, _arg| store.magit_cursor_up(),
        ));
        reg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_store() -> AppStore {
        // Everything the store needs is loaded/copied during `at`, so
        // the temp base may drop with this function (later persistence
        // writes in these tests are best-effort no-ops on a gone dir).
        let dir = tempfile::tempdir().unwrap();
        AppStore::at(dir.path(), dir.path().to_path_buf())
    }

    #[test]
    fn registry_has_the_seed_commands() {
        let reg = CommandRegistry::seed();
        let names: Vec<_> = reg.list().map(|c| c.name).collect();
        assert_eq!(names.len(), 40, "expected 40 seed commands: {names:?}");
        for expected in [
            "quit",
            "cancel",
            "open-palette",
            "cycle-view-next",
            "cycle-view-prev",
            "open-scratch",
            "message-echo",
            "demo-message-1",
            "demo-message-2",
            "insert-demo-text",
            "find-file",
            "switch-buffer",
            "list-buffers",
            "kill-buffer",
            "switch-project",
            "recent-files",
            "re-walk",
            "scroll-line-down",
            "scroll-line-up",
            "scroll-page-down",
            "scroll-page-up",
            "scroll-half-page-down",
            "scroll-half-page-up",
            "scroll-top",
            "scroll-bottom",
            "goto-line",
            "isearch-forward",
            "isearch-backward",
            "close-view",
            "open-buffer-list-selected",
            "buffer-list-next",
            "buffer-list-prev",
            "magit-status",
            "magit-stage",
            "magit-unstage",
            "magit-fold",
            "magit-visit-file",
            "magit-refresh",
            "magit-next",
            "magit-prev",
        ] {
            assert!(names.contains(&expected), "missing `{expected}`");
        }
    }

    #[test]
    fn dispatch_by_name_runs_the_command() {
        let reg = CommandRegistry::seed();
        let mut store = new_store();
        reg.dispatch_by_name(&mut store, "quit", None).unwrap();
        assert!(store.quit);

        let mut store = new_store();
        reg.dispatch_by_name(&mut store, "demo-message-1", None).unwrap();
        assert!(store.message.contains("hello"));

        let mut store = new_store();
        reg.dispatch_by_name(&mut store, "message-echo", Some("hi there".into()))
            .unwrap();
        assert_eq!(store.message, "hi there");

        // In-app path: dispatch through the store's own registry.
        let mut store = new_store();
        store.dispatch("quit", None).unwrap();
        assert!(store.quit);
    }

    #[test]
    fn dispatch_unknown_command_errors() {
        let reg = CommandRegistry::seed();
        let mut store = new_store();
        let err = reg.dispatch_by_name(&mut store, "does-not-exist", None).unwrap_err();
        assert!(matches!(err, RegistryError::UnknownCommand(_)));
        assert!(err.to_string().contains("does-not-exist"));

        let mut store = new_store();
        assert!(store.dispatch("nope", None).is_err());
    }

    #[test]
    fn view_commands_work() {
        let reg = CommandRegistry::seed();
        let dir = tempfile::tempdir().unwrap();
        let mut store = AppStore::at(dir.path(), dir.path().to_path_buf());
        reg.dispatch_by_name(&mut store, "open-scratch", None).unwrap();
        assert_eq!(store.view_name_display(), "*scratch*");

        reg.dispatch_by_name(&mut store, "insert-demo-text", None).unwrap();
        assert!(store.buffer_text().contains("demo text"));
    }
}
