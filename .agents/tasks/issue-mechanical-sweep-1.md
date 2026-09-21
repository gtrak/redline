# Task: mechanical sweep, part 1 (M6 · M4 · R2 · M7-partial)

Pure behavior-preserving cleanups. Each item is independent; land them as separate
commits if that is cleaner, and **honest-stop at half budget** (a subset is a valid
landing). Do not weaken or delete any assertion; every existing test must pass
unmodified.

## M6 — split `seed()` (`src/app/command.rs`)
`pub fn seed()` is ~694 lines registering **108** commands as inline closures. Split it
by the **category strings the registrations already carry** ("navigation", "files",
"buffers", "region", "git", "search", …) into private `fn register_<category>(&mut self)`
methods called from a thin `seed()`. Read the file to get the real category set — do not
guess it. The existing test asserting **108 commands and their names**
(`registry_has_the_seed_commands`) must pass **unmodified**: the names and their order
of registration must not change. Closures move verbatim.

## M4 — one `text_style` (`src/ui/`)
There are three copies of the text-style helper:
- `src/ui/picker.rs:140` and `src/ui/transient_menu.rs:144` — **byte-identical** 3-arg
  `text_style(foreground, invert, bold)`;
- `src/ui/file_view.rs:256` — a 2-arg variant (no `invert`) plus a `text_style_italic`
  at :266.
Promote **one** `pub(crate) fn text_style(fg, invert, bold)` into `src/ui/mod.rs`
beside the existing `color()` helper, and point all call sites at it (the file_view
variant becomes `text_style(fg, false, bold)`; keep an italic wrapper only if it is
still used). Behavior must be identical — the produced `CanvasTextStyle` for every
existing call site must be unchanged (verify by reading each call site's arguments).

## R2 — `src/git/repo.rs` twins
1. `head_blob_ends_with_newline` and `index_blob_ends_with_newline` are identical
   ~10-line bodies differing only in `DiffSide::Staged` vs `Unstaged`. Collapse to one
   `fn blob_side_ends_with_newline(&self, side: DiffSide, path: &str) -> …`.
2. The "find the hunk whose `new_start` matches, else `HunkNotFound`" sequence appears
   **4×** (`stage_hunk`, `unstage_hunk`, and both arms of `discard_hunk`). Extract
   `fn find_hunk_in<'a>(&self, raw: &'a git2::Diff, side, path, target) -> Result<&'a DiffHunk, GitError>`.
   `stage_hunk` has a dry-run `check` variant that uses the non-`cloned` form — cover it.
Confirm the exact current line numbers by reading; the numbers here are from an audit,
not a fresh measurement.

## M7 (partial) — free fns that should be methods
- `src/model/sections.rs`: `render(s: &Section, …)`, `collect_visible_ids(s: &Section, …)`,
  `set_fold(s: &mut Section, …)` → `impl Section` methods (keep genuinely pure byte
  helpers free).
- `src/app/keymap.rs`: `collect_command_pairs(node: &Node, …)` → `impl Node`.
Only if budget allows; report as not-done otherwise.

## Fence (strict)
`src/app/command.rs`, `src/ui/{mod,picker,transient_menu,file_view}.rs`,
`src/git/repo.rs`, `src/model/sections.rs`, `src/app/keymap.rs`.
**Nothing else** — other lanes own `src/app/store*`, `src/syntax/*`, `src/nav/*`,
`src/model/files.rs`, `src/search/*`, `crates/redline-resolve/*`. If an item needs a
file outside the fence, stop and report instead of widening it.

## Gate
`cargo build` first, then `cargo test --workspace`, `cargo clippy --workspace --all-targets`
(read `${PIPESTATUS[0]}`), and `timeout 900 tools/gate.sh full`.
**Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` before the battery and if
"available" < 8 GB run only `cargo test -p redline --lib` and report the full gate as
deferred (do not sleep-wait).
Budget ~45 tool calls. Report per item: what moved, the 108-command test result, gate counts,
anything you declined to do and why.
