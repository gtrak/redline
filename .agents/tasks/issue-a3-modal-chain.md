# Task: A3 — `key_event` is a 345-line modal chain; name its stages

`src/app/store/keys.rs:16` — `pub fn key_event(&mut self, key: Key)` is **345 lines**
of a 561-line file (61% of it). It is **not** a dispatch match: it is a hand-rolled
**modal chain** — 16 sequential guards where priority *is* statement order, ending in
`self.dispatch_key(key)`.

## The measured chain (re-derive before relying on it)

| # | Line | Guard | Body |
|---|---|---|---|
| 1 | 17 | `if self.quit` | return |
| 2 | 23 | `if self.quit_prompt_active()` | → `quit_prompt_key` (**named**) |
| 3 | 30 | `if self.menu_open()` | → `menu_key_event` (**named**) |
| 4 | 36 | `if self.discard_armed()` | → `discard_key_event` (**named**) |
| 5 | 44 | `if self.toggle_ro_active()` | inline |
| 6 | 48 | `if self.picker.is_some()` | **inline ~48 lines** |
| 7 | 96 | `if self.top_view() == ViewId::…` | **inline ~45 lines** |
| 8 | 141 | `if self.branch_create.is_some()` | **inline ~24 lines** |
| 9 | 165 | `if self.isearch.active` | **inline ~32 lines** |
| 10 | 197 | `if self.goto_line_active` | **inline ~25 lines** |
| 11 | 222 | `if self.note_prompt_active` | **inline ~23 lines** |
| 12 | 245 | `if self.search_prompt.is_some()` | **inline ~31 lines** |
| 13 | 276 | `if self.top_view() == ViewId::…` | **inline ~47 lines** |
| 14 | 323 | `if self.tree_visible()` | **inline ~24 lines** |
| 15 | 347 | `if key == Key::…` | inline |
| 16 | 356 | `if key == Key::…` | inline |

So five modals already route to a named method, and **eleven bodies are inline** —
that is ~300 of the 345 lines. The *chain* is fine; the *bodies* are what make the
function unreadable.

## The deliverable (step 1 — mandatory)

**Extract each inline body into a named method, and leave the chain explicit.**
Each extracted method takes the key and returns whether it **consumed** it (or simply
returns `()` if the guard guarantees consumption), e.g.:

```rust
if self.picker.is_some() { self.picker_key_event(key); return; }
```

After this, `key_event` is ~35 lines that read as a **priority list of named modals**,
which is exactly what it is. The bodies become individually readable and individually
testable, and adding a modal means adding one clearly-named line in one place.

Preserve the *swallowing* semantics exactly: some modals consume every key (the quit
prompt explicitly ignores unbound keys rather than echoing), others fall through.
Read each body's comments — they state which.

## Step 2 (OPTIONAL — and argue for it or against it, do not do it by reflex)

Converting the chain into an ordered handler list (`&[fn(&mut AppStore, Key) -> bool]`)
would make the priority *data*. **Be skeptical:** an explicit `if` chain in priority
order **is already a readable priority list**, so a table here may add indirection
without adding clarity — the opposite of the A2 keymap case, where the imperative form
was ~480 lines of `bind()` data with no structure at all. If you do step 2, justify it
in the report; if you don't, say why. **Do not let step 2 jeopardise step 1.**

## Key decisions

- **Priority must not change.** The guard sequence and its order are behaviour. Report
  the guard list before and after, position by position.
- **This is behaviour-sensitive**: key routing is the core UX. Every modal has PTY
  coverage, and `flow_tests.rs` drives long key sequences — the battery is the real
  check.
- **Pure extraction**: no condition inverted, no body rewritten, no capture changed.
- **Comments survive.** These bodies carry the "why" of each modal (e.g. "anything else
  is a no-op, no 'unbound key' echo mid-prompt"). Report comment accounting.
- **Guards 15–16** (`if key == Key::…`) are plain key-equality checks, not state
  modals. Judge whether they belong in the chain, in `dispatch_key`, or as a small
  named helper — and say what you chose.
- **Do not touch `dispatch_key`/`dispatch` semantics**, and do not touch the
  `quit_prompt_*` methods (they are already extracted and reviewed).

## Files

`src/app/store/keys.rs` (the fence). If the file grows past comfortable reading after
extraction, a `keys/` split is *allowed* but not required — the user's criterion is
logic organization, not file size, so do not split for size alone.

## Verification

- `cargo build`; `cargo test --workspace` — reconcile against the **current** baseline
  (**855** passed / 0 failed / 2 ignored redline — A2 bumped this from 854; do not
  copy an older number), resolver 123/0/4, integration 7,2,3,1,1, doctests 0;
  `cargo clippy --workspace --all-targets` (read `${PIPESTATUS[0]}`);
  **`timeout 900 tools/gate.sh full`** — mandatory here, because key routing is what
  every PTY drive exercises. If swap blocks it, run `cargo test --workspace` + both
  winding drives + `check_cursor_stream.py` and report the battery DEFERRED.
- Report: `key_event`'s before/after line count, the extracted method list with sizes,
  the guard-order comparison, comment accounting, and whether you did step 2.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.
