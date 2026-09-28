//! The bin's test-only helpers that cannot live in a shared util crate.
//!
//! The hermetic git CLI harness (`git_cli` / `git_repo_init`) moved to
//! `crates/redline-testutil` (plan 014 stage 2) when `src/git` became
//! the `redline-git` crate. What is left here is app-typed and stays in
//! the bin on purpose: `paste_keys` returns a
//! `crate::app::keymap::Key`, so it cannot live in a git-only
//! test-util crate.

/// **Bin-only — do NOT promote to `redline-testutil` (plan 014).** Unlike
/// `redline_testutil::git_cli`/`git_repo_init` (deliberately app-type-free so
/// the `redline-git` extraction lifted them as a move), this helper returns a
/// `crate::app::keymap::Key` — an app type — so it cannot live in a
/// git-only test-util crate. plan 014's promotion must leave it here.
///
/// issue-commit-editor-pasted-newline / issue-paste-newline-dropped: a
/// terminal paste arrives as ordinary key bytes (the app requests no
/// bracketed paste). The MEASURED decode (tools/probe_keydump, raw mode) of
/// a pasted string is: printable chars -> `Char(c)`, and the LF byte (0x0A)
/// -> `Char('j')`+CONTROL (C-j) — NOT an `Enter` event, NOT a literal
/// `Char('\\n')`. This single source of truth mirrors that decode so the
/// tests feed the modal/buffer exactly what a real paste would deliver.
pub(crate) fn paste_keys(paste: &str) -> Vec<crate::app::keymap::Key> {
    paste
        .chars()
        .map(|c| {
            if c == '\n' {
                crate::app::keymap::Key::ctrl_char('j')
            } else {
                crate::app::keymap::Key::new(crate::app::keymap::KeyCode::Char(c))
            }
        })
        .collect()
}
