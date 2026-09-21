# Task: A2 — the keymap as data, not 142 imperative `bind()` calls

The built-in keymap is configuration written as code:

| Where | Size | Binds |
|---|---|---|
| `AppStore::at` (`src/app/store/mod.rs:1557`) | **200 lines** | 22 (the global map) |
| `ViewId::keymap` (`src/app/store/mod.rs:237`) | **284 lines** | 120 (per-view maps) |

That is **142 statements** like `global.bind(&[Key::ctrl_char('g')], "cancel").unwrap();`
— 79 distinct commands — **inside constructors**. Adding one binding means editing a
giant function, and the "why" of each binding lives in interleaved comments.

## The design is already decided by the codebase

**A string→`Key` parser already exists, is tested, and is already used in
production**: `crate::app::keymap::parse_sequence("C-x C-f") -> Result<KeySeq, ParseKeyError>`
(`src/app/keymap.rs:279`; `parse_key` at `:185`; tests at `:457-514` covering `C-SPC`,
caret notation, modifiers, and rejection of garbage). It is used by
`src/app/config.rs:128` — `Config::validate_bindings` iterates
`Config.key_bindings: BTreeMap<String, String>` (command → **emacs-notation sequence
string**) and parses each.

So the *user-config* path already speaks `"C-x C-f"`, while the built-in keymap is
imperative `Key::ctrl_char('x')` calls. **Unify them: the built-in keymap becomes a
table of `(sequence, command)` string pairs**, loaded through the same parser. This
matches how the docs, the flow tests (`flow_tests.rs:26` uses `parse_key`), and the
PTY drives already express keys.

## What to build

Replace the two imperative bodies with declarative tables, e.g.:

```rust
/// (emacs-notation key sequence, command name)
const GLOBAL_BINDINGS: &[(&str, &str)] = &[
    ("C-g", "cancel"),
    ("M-x", "open-palette"),
    ("C-x C-c", "quit"),
    // …the comment that explains why this binding exists stays with it
];

fn view_bindings(view: ViewId) -> &'static [(&str, &str)] { match view { … } }
```

plus a loader that parses and binds each entry, reporting a conflict with **both the
sequence and the command** in the error (better than today's bare `.unwrap()`).

## Key decisions

- **Preserve the conflict detection.** Today `bind` returns `Result<(), String>` and
  every call `.unwrap()`s, so a duplicate — or a binding that is a strict prefix of a
  longer one — **panics loudly at construction**. That is a feature (it caught the
  `M-s o` / `M-s` prefix conflict documented in the current comments). Keep a
  fail-loud path, and make the message name the key and the command.
- **Bind ORDER may be semantic — verify it.** The current comment says *"the engine
  forbids a command on a strict prefix of a longer binding"*, which implies `bind`
  validates against what is already bound. Determine whether the resulting map is
  order-independent; if it is not, preserve the original order exactly (the table's
  row order then *is* the load order) and say so.
- **Every comment must survive.** ~40 of them carry decisions (why cycling moved to
  `M-x`, why `C-x` stays a prefix, why bare `q` closes rather than quits, issue
  references). A conversion that drops them is a **P1**. Report the comment-line
  count before/after.
- **Do not change any binding.** This is a representation change, not a keymap
  redesign. Same sequences, same commands, same per-view maps, same global map.
- **Do not touch `config.rs`'s behaviour** — but do note in the report whether the
  two paths can now share one loader (that would be a follow-up, not this task).
- If `Key`'s constructors would need to become `const fn` for a `const` table, that
  is **not** the intended route — use the string parser instead.

## Files

| File | Change |
|---|---|
| `src/app/store/mod.rs` | `at` loses the 22-bind block; `keymap` becomes table-driven (or the tables move next to `keymap`) |
| `src/app/keymap.rs` | only if a shared loader belongs there (it likely does) — do not change `parse_sequence`'s behaviour |

Fence: `src/app/store/mod.rs`, `src/app/keymap.rs`. **Do not touch
`src/app/store/keys.rs`** (that is B1, a separate concern).

## Verification

- **Equivalence is the whole test.** Add a test that proves the new keymap is
  identical to the old: for every table entry, `km.lookup(parse_sequence(seq))`
  resolves to the named command; and assert the **total binding count** matches the
  pre-change count (142 across the global + per-view maps, or the count you measure).
  A spot-check of representative resolutions (`C-x C-c` → `quit`; `q` in `Buffer` →
  `close-view`) is worth having too. If `KeyMap` has no way to enumerate bindings,
  `lookup` per table entry is sufficient — say what you did.
- `cargo build`; `cargo test --workspace` (reconcile `test result:` lines against the
  baseline: redline 854/0/2, resolver 123/0/4, integration 7,2,3,1,1, doctests 0);
  `cargo clippy --workspace --all-targets` (read `${PIPESTATUS[0]}`);
  `timeout 900 tools/gate.sh full` — **the keymap is what every PTY drive exercises**,
  so the battery is the real check. If swap is exhausted, run
  `cargo test --workspace` + both windowing drives and report the battery DEFERRED.
- Report: the table's row count per view, the conflict-message change, comment
  accounting, order-independence finding, and the equivalence test's assertions.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.
