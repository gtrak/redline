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

/// Handler for a command. `arg` carries an optional argument (e.g. a
/// project-relative path for a file command). `Send + Sync` so the store
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

    /// The command set shipped with issue 01 (and every issue since).
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
        // ── plan 004 issue 05b: file-view point (line, col) + emacs motion ─
        // These move the read-focused file-view's point; the window follows.
        // They supersede the window-scroll bindings on C-n/C-p/arrows and add
        // C-f/C-b/C-a/C-e and M-</M-> point motion (the plan-001 item-4
        // arrows-scroll stopgap is retired by explicit user directive).
        reg.register(Command::new(
            "point-down",
            "Point down one line, preserving the goal column (C-n / Down)",
            "motion",
            |store, _arg| store.point_down(),
        ));
        reg.register(Command::new(
            "point-up",
            "Point up one line, preserving the goal column (C-p / Up)",
            "motion",
            |store, _arg| store.point_up(),
        ));
        reg.register(Command::new(
            "point-forward",
            "Point forward one character, wrapping to the next line at EOL (C-f / Right)",
            "motion",
            |store, _arg| store.point_forward(),
        ));
        reg.register(Command::new(
            "point-backward",
            "Point backward one character, wrapping to the previous line end at BOL (C-b / Left)",
            "motion",
            |store, _arg| store.point_backward(),
        ));
        reg.register(Command::new(
            "point-line-start",
            "Point to the beginning of the line (C-a)",
            "motion",
            |store, _arg| store.point_line_start(),
        ));
        reg.register(Command::new(
            "point-line-end",
            "Point to the end of the line (C-e)",
            "motion",
            |store, _arg| store.point_line_end(),
        ));
        reg.register(Command::new(
            "point-buffer-start",
            "Point to the start of the buffer; the window follows (M-<)",
            "motion",
            |store, _arg| store.point_buffer_start(),
        ));
        reg.register(Command::new(
            "point-buffer-end",
            "Point to the end of the buffer; the window follows (M-> / G)",
            "motion",
            |store, _arg| store.point_buffer_end(),
        ));
        reg.register(Command::new(
            "word-forward",
            "Point forward one word, wrapping across lines (M-f; emacs forward-word)",
            "motion",
            |store, _arg| store.point_word_forward(),
        ));
        reg.register(Command::new(
            "word-backward",
            "Point backward one word, wrapping across lines (M-b; emacs backward-word)",
            "motion",
            |store, _arg| store.point_word_backward(),
        ));
        reg.register(Command::new(
            "recenter",
            "Recenter (C-l; emacs recenter-top-bottom): the point stays put and its screen row cycles middle → top → bottom (a fresh C-l goes to middle; the cycle resets on any other command)",
            "motion",
            |store, _arg| store.recenter(),
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
            "open-transient-menu",
            "Open this view's command menu (?; h in magit views)",
            "navigation",
            |store, _arg| store.open_menu(),
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
            "magit-discard",
            "Discard the file or hunk change at point; confirmation-gated (k)",
            "git",
            |store, _arg| store.magit_discard(),
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
        // ── issue 08: log / blame / commit / branches / stash ────
        reg.register(Command::new(
            "magit-log",
            "Open the git log for the current branch (l in the status buffer)",
            "git",
            |store, _arg| store.open_log(),
        ));
        reg.register(Command::new(
            "magit-blame",
            "Blame the current file (b in the status buffer)",
            "git",
            |store, _arg| store.open_blame(),
        ));
        reg.register(Command::new(
            "magit-commit",
            "Open the inline commit editor for staged changes (c in the status buffer)",
            "git",
            |store, _arg| store.open_commit_editor(),
        ));
        reg.register(Command::new(
            "branch-picker",
            "Check out a local branch (y in the status buffer)",
            "git",
            |store, _arg| store.open_branch_picker(),
        ));
        reg.register(Command::new(
            "stash-list",
            "List stashes; RET pops, x drops (z in the status buffer)",
            "git",
            |store, _arg| store.open_stash_picker(),
        ));
        reg.register(Command::new(
            "branch-create",
            "Create a new local branch at HEAD (name prompt)",
            "git",
            |store, _arg| store.branch_create_start(),
        ));
        reg.register(Command::new(
            "log-next-page",
            "Next page of the log (n in the log view)",
            "git",
            |store, _arg| store.log_next_page(),
        ));
        reg.register(Command::new(
            "log-prev-page",
            "Previous page of the log (p in the log view)",
            "git",
            |store, _arg| store.log_prev_page(),
        ));
        reg.register(Command::new(
            "log-move-down",
            "Move the log selection down (j / C-n in the log view)",
            "git",
            |store, _arg| store.log_move_down(),
        ));
        reg.register(Command::new(
            "log-move-up",
            "Move the log selection up (k / C-p in the log view)",
            "git",
            |store, _arg| store.log_move_up(),
        ));
        reg.register(Command::new(
            "log-open-commit",
            "Open the selected commit's diff (RET in the log view)",
            "git",
            |store, _arg| store.log_open_commit(),
        ));
        reg.register(Command::new(
            "commit-editor-commit",
            "Commit the staged changes with the editor's message (C-c C-c)",
            "git",
            |store, _arg| store.commit_editor_commit(),
        ));
        reg.register(Command::new(
            "commit-editor-abort",
            "Discard the commit editor without touching the repo (C-c C-k)",
            "git",
            |store, _arg| store.commit_editor_abort(),
        ));
        // ── issue 003-02: shared windowing (commit-diff + blame motion) ────
        // The commit-diff pane has no cursor: these move the window itself via
        // the emacs-motion vocabulary (the FileView keys, no new bindings).
        reg.register(Command::new(
            "commit-diff-scroll-down",
            "Scroll the commit-diff window down one row (C-n)",
            "motion",
            |store, _arg| store.commit_diff_scroll_down(),
        ));
        reg.register(Command::new(
            "commit-diff-scroll-up",
            "Scroll the commit-diff window up one row (C-p)",
            "motion",
            |store, _arg| store.commit_diff_scroll_up(),
        ));
        reg.register(Command::new(
            "commit-diff-page-down",
            "Scroll the commit-diff window down one page (C-v)",
            "motion",
            |store, _arg| store.commit_diff_page_down(),
        ));
        reg.register(Command::new(
            "commit-diff-page-up",
            "Scroll the commit-diff window up one page (M-v)",
            "motion",
            |store, _arg| store.commit_diff_page_up(),
        ));
        reg.register(Command::new(
            "commit-diff-scroll-top",
            "Scroll the commit-diff window to the top (M-<)",
            "motion",
            |store, _arg| store.commit_diff_scroll_top(),
        ));
        reg.register(Command::new(
            "commit-diff-scroll-bottom",
            "Scroll the commit-diff window to the bottom (M->)",
            "motion",
            |store, _arg| store.commit_diff_scroll_bottom(),
        ));
        // The blame pane has a cursor; these move it and the window follows
        // (keeps the cursor row in view).
        reg.register(Command::new(
            "blame-next",
            "Move the blame cursor down one row (C-n)",
            "git",
            |store, _arg| store.blame_cursor_down(),
        ));
        reg.register(Command::new(
            "blame-prev",
            "Move the blame cursor up one row (C-p)",
            "git",
            |store, _arg| store.blame_cursor_up(),
        ));
        reg.register(Command::new(
            "blame-page-down",
            "Move the blame cursor down one page (C-v)",
            "git",
            |store, _arg| store.blame_page_down(),
        ));
        reg.register(Command::new(
            "blame-page-up",
            "Move the blame cursor up one page (M-v)",
            "git",
            |store, _arg| store.blame_page_up(),
        ));
        reg.register(Command::new(
            "blame-top",
            "Move the blame cursor to the first row (M-<)",
            "git",
            |store, _arg| store.blame_cursor_top(),
        ));
        reg.register(Command::new(
            "blame-bottom",
            "Move the blame cursor to the last row (M->)",
            "git",
            |store, _arg| store.blame_cursor_bottom(),
        ));
        // ── issue 04: file watching ─────────────────────────────────
        reg.register(Command::new(
            "reload-buffer",
            "Force-reload the current file buffer from disk (g); supersedes a \"changed on disk\" conflict",
            "watching",
            |store, _arg| store.reload_current_buffer(),
        ));
        reg.register(Command::new(
            "toggle-watcher",
            "Suspend/resume live file watching",
            "watching",
            |store, _arg| store.toggle_watcher(),
        ));
        // ── issue 05: symbol navigation ─────────────────────────────
        reg.register(Command::new(
            "xref-find-definitions",
            "Jump to the definition of the symbol under point (M-.)",
            "navigation",
            |store, _arg| store.xref_find_definitions(),
        ));
        reg.register(Command::new(
            "jump-back",
            "Pop back to the prior position in the jump stack (M-,)",
            "navigation",
            |store, _arg| store.jump_back(),
        ));
        reg.register(Command::new(
            "jump-forward",
            "Walk forward in the jump stack (C-i)",
            "navigation",
            |store, _arg| store.jump_forward(),
        ));
        reg.register(Command::new(
            "imenu",
            "Open the imenu outline of the current file (M-i)",
            "navigation",
            |store, _arg| store.open_imenu(),
        ));
        reg.register(Command::new(
            "open-symbol-picker",
            "Open the project-wide symbol picker (M-x)",
            "navigation",
            |store, _arg| store.open_symbol_picker(),
        ));
        // ── issue 06: search & references ───────────────────────────
        reg.register(Command::new(
            "project-search",
            "Project-wide literal search with a prompt (C-c p s s)",
            "search",
            |store, _arg| {
                store.search_prompt_start(crate::app::store::SearchPromptKind::Project);
            },
        ));
        reg.register(Command::new(
            "references-at-point",
            "References to the symbol under point (M-?)",
            "search",
            |store, _arg| store.references_at_point(),
        ));
        reg.register(Command::new(
            "occur",
            "Occurrences of a regex in the current buffer (M-s o)",
            "search",
            |store, _arg| {
                store.search_prompt_start(crate::app::store::SearchPromptKind::Occur);
            },
        ));
        reg.register(Command::new(
            "search-next",
            "Move to the next match in the search results (n)",
            "search",
            |store, _arg| store.search_next(),
        ));
        reg.register(Command::new(
            "search-prev",
            "Move to the previous match in the search results (p)",
            "search",
            |store, _arg| store.search_prev(),
        ));
        reg.register(Command::new(
            "search-jump",
            "Jump to the match under the cursor (RET in the results view); M-, returns",
            "search",
            |store, _arg| store.search_jump(),
        ));
        reg.register(Command::new(
            "search-rerun",
            "Re-run the current search (g in the results view)",
            "search",
            |store, _arg| store.search_rerun(),
        ));
        reg.register(Command::new(
            "search-cancel",
            "Cancel the in-flight search (C-g in the results view)",
            "search",
            |store, _arg| store.search_cancel(),
        ));
        reg.register(Command::new(
            "close-search-view",
            "Cancel and close the search results view (q / ESC)",
            "search",
            |store, _arg| store.search_close(),
        ));
        // ── issue 09: tree sidebar ────────────────────────────────────────
        reg.register(Command::new(
            "toggle-tree",
            "Toggle the project file-tree sidebar (C-c p t)",
            "files",
            |store, _arg| store.toggle_tree(),
        ));
        reg.register(Command::new(
            "toggle-tree-follow",
            "Toggle tree buffer-follow: opening a file moves the tree cursor (off by default)",
            "files",
            |store, _arg| store.toggle_tree_follow(),
        ));
        reg.register(Command::new(
            "open-notes",
            "Open the per-project notes buffer (C-x n)",
            "buffers",
            |store, _arg| store.open_notes(),
        ));
        reg.register(Command::new(
            "save-buffer",
            "Save the current buffer to its file (C-x C-s)",
            "buffers",
            |store, _arg| store.save_buffer(),
        ));
        // ── plan 004 issue 03: mark/region + kill ring ─────────────────
        reg.register(Command::new(
            "set-mark",
            "Set the mark at the current position (C-SPC)",
            "region",
            |store, _arg| store.set_mark(),
        ));
        reg.register(Command::new(
            "kill-region",
            "Kill the marked region (C-w); in read-only views copies to kill ring",
            "region",
            |store, _arg| store.kill_region(),
        ));
        reg.register(Command::new(
            "copy-region",
            "Copy the marked region to the kill ring (M-w)",
            "region",
            |store, _arg| store.copy_region(),
        ));
        reg.register(Command::new(
            "yank",
            "Yank the kill ring's top entry at point (C-y, editable buffers only)",
            "region",
            |store, _arg| store.yank(),
        ));
        reg.register(Command::new(
            "yank-pop",
            "Replace the last yank with the previous kill ring entry (M-y)",
            "region",
            |store, _arg| store.yank_pop(),
        ));
        reg.register(Command::new(
            "exchange-point-and-mark",
            "Exchange point and mark (C-x C-x)",
            "region",
            |store, _arg| store.exchange_point_and_mark(),
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
        assert_eq!(names.len(), 100, "expected 100 seed commands: {names:?}");
        for expected in [
            "quit",
            "cancel",
            "open-palette",
            "cycle-view-next",
            "cycle-view-prev",
            "open-scratch",
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
            "point-down",
            "point-up",
            "point-forward",
            "point-backward",
            "point-line-start",
            "point-line-end",
            "point-buffer-start",
            "point-buffer-end",
            "word-forward",
            "word-backward",
            "recenter",
            "goto-line",
            "isearch-forward",
            "isearch-backward",
            "close-view",
            "open-transient-menu",
            "open-buffer-list-selected",
            "buffer-list-next",
            "buffer-list-prev",
            "magit-status",
            "magit-stage",
            "magit-unstage",
            "magit-discard",
            "magit-fold",
            "magit-visit-file",
            "magit-refresh",
            "magit-next",
            "magit-prev",
            "magit-log",
            "magit-blame",
            "magit-commit",
            "branch-picker",
            "stash-list",
            "branch-create",
            "log-next-page",
            "log-prev-page",
            "log-move-down",
            "log-move-up",
            "log-open-commit",
            "commit-editor-commit",
            "commit-editor-abort",
            "commit-diff-scroll-down",
            "commit-diff-scroll-up",
            "commit-diff-page-down",
            "commit-diff-page-up",
            "commit-diff-scroll-top",
            "commit-diff-scroll-bottom",
            "blame-next",
            "blame-prev",
            "blame-page-down",
            "blame-page-up",
            "blame-top",
            "blame-bottom",
            "reload-buffer",
            "toggle-watcher",
            "xref-find-definitions",
            "jump-back",
            "jump-forward",
            "imenu",
            "open-symbol-picker",
            "project-search",
            "references-at-point",
            "occur",
            "search-next",
            "search-prev",
            "search-jump",
            "search-rerun",
            "search-cancel",
            "close-search-view",
            "save-buffer",
            "toggle-tree-follow",
            "set-mark",
            "kill-region",
            "copy-region",
            "yank",
            "yank-pop",
            "exchange-point-and-mark",
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
        reg.dispatch_by_name(&mut store, "open-scratch", None).unwrap();
        assert_eq!(store.view_name_display(), "*scratch*");

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

        reg.dispatch_by_name(&mut store, "list-buffers", None).unwrap();
        assert_eq!(store.top_view(), ViewId::BufferList);
    }

    /// Issue 09 step 7: the README keymap table must only document commands
    /// that exist in the registry. Parses the `| key | command |` rows from
    /// README.md and asserts each command name is registered.
    #[test]
    fn readme_keymap_table_matches_registry() {
        let readme = std::fs::read_to_string("README.md")
            .expect("README.md must exist at the repo root");
        let reg = CommandRegistry::seed();
        let registry_names: std::collections::HashSet<&str> =
            reg.list().map(|c| c.name).collect();
        // Parse the keymap table: lines starting with `|` that have a
        // second cell (the command name) in backticks.
        let mut documented: Vec<&str> = Vec::new();
        for line in readme.lines() {
            let line = line.trim();
            if !line.starts_with('|') {
                continue;
            }
            let cells: Vec<&str> = line.split('|').collect();
            // Only process 3-column tables (Key | Command | Description):
            // splitting by '|' gives 5 parts. Skip 2-column tables (4 parts)
            // like the magit-status keymap where the second cell is a
            // description, not a command name.
            if cells.len() < 5 {
                continue;
            }
            // Only process rows where the key cell contains backticks
            // (all keymap rows use backticks for key sequences; this
            // skips the performance table and other non-keymap tables).
            let key_cell = cells[1].trim();
            let cmd_cell = cells[2].trim();
            if !key_cell.contains('`') {
                continue;
            }
            // Extract the command name: from backticks if present, otherwise
            // the trimmed cell (the README uses plain text for command names).
            let cmd = if let Some(start) = cmd_cell.find('`')
                && let Some(end) = cmd_cell[start + 1..].find('`')
            {
                &cmd_cell[start + 1..start + 1 + end]
            } else {
                cmd_cell
            };
            if !cmd.is_empty() && !cmd.starts_with('-') {
                documented.push(cmd);
            }
        }
        assert!(!documented.is_empty(), "keymap table must have entries");
        for cmd in &documented {
            assert!(
                registry_names.contains(*cmd),
                "README documents `{cmd}` but it is not in the command registry"
            );
        }
    }
}
