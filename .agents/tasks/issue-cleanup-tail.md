# Task: the cleanup tail (F-5 · test-helper dedup · optional Snapshot constructor)

Three small, independent items. Land each as its own commit; honest-stop at half budget
(each is a valid landing on its own). This is the last of plan 012's cleanup list.

## Item 1 — inline the `ui/views/` directory (F-5)
`src/ui/views/` is dead weight: `mod.rs` is 2 lines and its only child is `buffer.rs`
(102 lines, one `#[component]` plus a test module that sits *above* the component it
tests). Either inline the component into `src/ui/` (e.g. `src/ui/buffer_view.rs`) and
delete the directory, or — if reading suggests the directory was the start of a real
grouping — say so and leave it with a comment explaining the intent. Update the `mod`
declaration in `src/ui/mod.rs` and every import path; the component must still be
reachable from `Root`.
Also: move the test module **below** the component it tests (convention).

## Item 2 — deduplicate the store test helpers (T4/T5)
The store test module was deliberately split as a PURE MOVE, so its duplicated helpers
are now visible and located rather than interleaved:
- **10 `fn git_cli` variants**: `tests/magit.rs` ×6, `tests/mod.rs` ×2, `tests/commit.rs` ×1,
  `tests/index_wiring.rs` ×1. Some are nested *inside* test bodies, some are module-level.
- The fixture family: `notes_store`, `notes_store_with_lines`, `store_with_*`,
  `store_with_index`, `install_index`, `git_store`, `tall_commit_repo`, …
Target: **one** shared `git_cli` in `tests/mod.rs` plus one index-install helper and a
shared project-scaffold helper, imported by the concern files.
**Rules**: this is a dedup, not a move, so the bar is higher — **every assertion must
still assert the same thing** (diff the test bodies to prove it; only helper bodies may
change). **Do not collapse real differences**: the variants differ in env values
(`"T"/"t@e.com"` vs `"Test"/"test@example.com"`) and one may omit the hermetic env vars
entirely — parameterize honestly or keep a variant with a comment saying why. If a variant
is non-hermetic (missing `GIT_CONFIG_GLOBAL=/dev/null` etc.), that is a latent host-identity
flake: align it and **say so** (it is a behaviour change to a test, allowed with the
reason stated).

## Item 3 (optional, only if budget allows) — `Snapshot::from_store`
`src/ui/root/snapshot.rs` needed **67 `pub(super)`** (the struct + all 66 fields) because
`Root` builds the `Snapshot { … }` literal in a sibling module. The gate judged that
visibility-neutral (still invisible outside `ui::root`) but noted a constructor would be
materially better design. Move the extraction into `Snapshot::from_store(&AppStore)` (or
an equivalent) so the fields can go back to private, and report the new `pub(super)` count.
This touches the render path, so it needs the PTY battery (or the windowing drives if swap
is exhausted). If it looks risky, skip it and say so — it is genuinely optional.

## Fence
`src/ui/views/*` + `src/ui/mod.rs` (Item 1), `src/app/store/tests/*` (Item 2),
`src/ui/root/*` (Item 3). Nothing else.

## Gate
`cargo build`; `cargo test --workspace`; `cargo clippy --workspace --all-targets` (read
`${PIPESTATUS[0]}`); `timeout 900 tools/gate.sh full`.
**Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` **and swap**. NOTE:
`check_cursor_stream.py` is a known loud failure **under concurrent load** only (it is
80/80 on an idle box, verified by the final battery) — if it is the only failing suite,
say so and do not treat it as your regression.
Budget ~45 tool calls. Report per item: what changed, the assertion-preservation evidence
(Item 2), the new `pub(super)` count (Item 3), gate counts, anything skipped and why.
