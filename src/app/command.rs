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
            "Open the scratch view (demo: view push)",
            "view",
            |store, _arg| {
                store.push_view(ViewId::Scratch);
                store.minibuffer_message("Opened *scratch*");
            },
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
            "Insert demo text into the scratch view",
            "scratch",
            |store, _arg| {
                if store.insert_text("demo text\n") {
                    store.minibuffer_message("inserted demo text");
                } else {
                    store.minibuffer_message("insert-demo-text: no scratch view on top");
                }
            },
        ));
        reg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_store() -> AppStore {
        AppStore::new()
    }

    #[test]
    fn registry_has_the_placeholder_commands() {
        let reg = CommandRegistry::seed();
        let names: Vec<_> = reg.list().map(|c| c.name).collect();
        assert_eq!(names.len(), 10, "expected 10 seed commands: {names:?}");
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
        let mut store = new_store();
        reg.dispatch_by_name(&mut store, "open-scratch", None).unwrap();
        assert_eq!(store.view_stack.len(), 2);
        assert_eq!(store.top_view(), ViewId::Scratch);

        reg.dispatch_by_name(&mut store, "insert-demo-text", None).unwrap();
        assert!(store.current_text().contains("demo text"));
    }
}
