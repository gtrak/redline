# Task spec — hygiene batch: W2 + W5

Two small, independent, pre-existing cleanups. No behavior change to the app
(strictly: W2 removes dead code; W5 centralizes an assignment). Both are
verified by the existing suite + a green gate.

## W2 — delete the dead `CommandRegistry::dispatch_by_name`

`src/app/command.rs:95` `pub fn dispatch_by_name(&self, store, name, arg)`
is `#[allow(dead_code)]` and **nothing in production calls it**. It is also
SUBTLELY WRONG to keep: it runs only the handler, while the real
`AppStore::dispatch` (`src/app/store/keys.rs:984`) additionally clears
`self_insert_run`, `jump_highlight`, and `recenter_cycle` — so a caller who
reached for `dispatch_by_name` would silently skip those invariants.

Do:
1. Delete `dispatch_by_name` from `src/app/command.rs`.
2. Migrate its tests (in `src/app/command.rs` `mod tests`):
   `dispatch_by_name_runs_the_command`, `dispatch_unknown_command_errors`,
   `view_commands_work` use it — re-point them at `store.dispatch(...)`
   (`AppStore::dispatch`), preserving every assertion (the `Err(
   UnknownCommand)` check, the name-in-message check, and the view checks).
   The `dispatch_by_name_runs_the_command` test's "in-app path" comment can go
   (there is now one path); keep the assertions.
3. `grep -rn dispatch_by_name src/` must return zero hits afterward.

## W5 — centralize `quit` mutation via `request_quit()`

`AppStore.quit` (`src/app/store/mod.rs:1925`) is set `= true` in THREE places
in `src/app/store/keys.rs` (`:1026` in `begin_quit`, `:1148` and `:1169` in
the quit-prompt flow). Add:

```rust
/// Request that the render loop exit. The single mutation point for the
/// `quit` flag (the flag stays `pub` for the read side — `snapshot.rs`,
/// `Session`, the Root loop read it directly). Setting it is idempotent.
pub fn request_quit(&mut self) {
    self.quit = true;
}
```

and route all three `self.quit = true;` sites through `self.request_quit();`.
**Do NOT privatize the field** — the read side is spread across the UI
(Snapshot, Root, Session) and the field's `pub` read access is the existing
convention. This wart was only about the MUTATION sites.

Add a tiny test if cheap: `begin_quit` with no modified buffers calls
`request_quit` (i.e. `store.quit` becomes true) — but only if a test doesn't
already cover that (several flow_tests do: `flow_tests.rs:1504`
`assert!(s.quit, "unmodified buffers must quit immediately")`). If coverage
already exists, say so and add none.

## Do NOT do (recorded dispositions, deliberately out of scope)

- **W1** (`command.rs` header/categories): a FALSE ALARM — 120 commands use 11
  distinct categories (git 28, motion 26, …); the header is accurate. Leave it.
- **W4** (`state` observable gaps): no current consumer; leave.
- **W6** (duplicated store construction): the TUI startup path legitimately
  differs from headless; leave.

## Verification

```
cargo build --workspace > /tmp/bh.log 2>&1; echo BUILD=$?
cargo clippy --workspace --all-targets -- -D warnings > /tmp/ch.log 2>&1; echo CLIPPY=$?
cargo test --bin redline > /tmp/th.log 2>&1; echo TEST=$?
grep -rn dispatch_by_name src/ || echo "W2: zero hits (good)"
```
Report the test counts (expect 1119, or one fewer if a test that only proved
the dead fn was removed — state which). Commit when green.

## Report
Files changed; the `dispatch_by_name` test migration (what each assertion
became); the 3 `request_quit` sites; gate outputs; any test count change and
why.
