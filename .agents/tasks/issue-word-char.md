# Task: unify the word-character rule (C15)

## Why
Five sites decide "is this character part of a word/identifier" and they **disagree**:
- `src/app/store/mod.rs` — `is_word_char(c) = c.is_alphanumeric() || c == '_'` — **Unicode-aware**,
  and the documented rule (`docs/emacs-parity-log.md` records it as the fixed rule).
- `src/search/rg.rs` — its word-boundary sink uses `is_ascii_alphanumeric`.
- `src/search/references.rs` — `symbol_under_point` uses Unicode `is_alphanumeric`.
- `crates/redline-resolve/src/cargo.rs` — `is_ident_char` is ASCII.
- plus two inline closures in `src/app/store/mod.rs` (`|c| c.is_alphanumeric() || c == '_'`).

Consequence: for a non-ASCII identifier like `café`, word motion (store) and M-? symbol
extraction (references) treat `é` as part of the word, while the rg word-boundary sink and
the resolver's locator do not — so the same symbol splits differently between the
candidate-generation and in-line column-check paths. `rg.rs` even carries a branch that
exists *only* because of this class of mismatch.

## Required outcome
**One rule, applied everywhere**: the Unicode rule
(`c.is_alphanumeric() || c == '_'`), which is the documented one.
- Inside the `redline` crate: define it **once** (a small `pub(crate)` helper — put it
  where a text/word predicate belongs, e.g. `src/model/`; `model/text_width.rs` is about
  cell widths, so a sibling module or `model/buffer.rs` is likely a better home — your
  call, say which) and use it at **every** redline site, including the inline closures.
- **Cross-crate note**: `crates/redline-resolve` is a separate crate with no dependency on
  `redline`, so it cannot import the helper. Apply the same rule there and leave a comment
  cross-referencing the redline definition (do not add a shared crate for one predicate).
  If you conclude the resolver *must* stay ASCII for a real reason, **stop and report** that
  reason rather than silently diverging.

## Behaviour change (be explicit)
This is a **behaviour change**, not a pure move: non-ASCII identifiers now split
consistently. That is the point, but it must be pinned:
- Add a multibyte test that demonstrates the consistency — e.g. a symbol like `café` (or a
  CJK identifier) exercised through the **M-? / references** path AND the **word-motion**
  path, asserting they agree on the symbol's extent.
- Add/extend a test for the **rg word-boundary sink** showing the same rule applies there
  (a search for a non-ASCII symbol must not be truncated by an ASCII-only boundary).
- If any existing test asserts the old ASCII-only behaviour, it is an
  **implementation-level pin** — update it and say what the new pin protects (per this
  repo's test-authority policy). Do not weaken anything.

## Fence
`src/app/store/mod.rs`, `src/search/{rg,references}.rs`, `src/model/*` (the new helper),
`crates/redline-resolve/src/cargo.rs`, and tests in those modules. **Nothing else** — in
particular do NOT touch `src/ui/root.rs` or `tools/check_cursor_stream.py` (another lane
owns them).
Honest-stop at half budget: unifying the redline-side sites (and pinning them) is a valid
landing even if the resolver side is left with a documented follow-up — say which.

## Gate
`cargo build`; `cargo test --workspace`; `cargo clippy --workspace --all-targets` (read
`${PIPESTATUS[0]}`); `timeout 900 tools/gate.sh full`.
**Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` **and swap** — the box is
swap-exhausted and has produced unattributed SIGTERM kills of rustc/test harnesses under
that pressure. If swap is exhausted, run `cargo test --workspace` + targeted suites and
report the battery as deferred (do not sleep-wait).
Budget ~40 tool calls. Report: the chosen helper location + rule, every site converted,
the multibyte/word-boundary tests and what they catch, any pin you had to update and why,
gate counts.
