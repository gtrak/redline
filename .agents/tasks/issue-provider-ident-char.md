# Task: finish the word-character rule in the resolver providers

## Why (recorded because nothing tracked it)
The C15 lane unified the word/identifier predicate on the Unicode rule inside `redline`
(`model/buffer.rs::is_word_char`) and converted `crates/redline-resolve/src/cargo.rs`.
But the resolver is **still internally mixed**:

| file | predicate | rule |
|---|---|---|
| `crates/redline-resolve/src/lib.rs` | `is_ident_char` | Unicode |
| `crates/redline-resolve/src/providers/python_provider.rs` | `is_ident_char` | Unicode |
| `crates/redline-resolve/src/cargo.rs` | `is_ident_char` | Unicode (converted by C15) |
| `crates/redline-resolve/src/providers/js_provider.rs:793` | `is_ident_char` | **ASCII** |
| `crates/redline-resolve/src/providers/go_provider.rs:619` | `is_ident_char` | **ASCII** |

This is a **pre-existing** disagreement (C15 did not introduce it), but JS and Go both
allow Unicode identifiers, so the ASCII check is semantically wrong in those two
providers: a non-ASCII identifier's line-scan can mis-detect a definition boundary, and
the resolver then disagrees with itself depending on which provider handled the file.

## Required outcome
Apply the same Unicode rule in `js_provider.rs` and `go_provider.rs`, each with a
cross-referencing comment to `redline::model::buffer::is_word_char` and to the sibling
`cargo.rs` copy (mirroring the comment style C15 introduced). If a provider has a real
reason to stay ASCII, **stop and report** it rather than diverging silently.

## Pins
Add a discriminating test per converted provider — each must **fail under the ASCII rule**:
- `js_provider`: a line-scan/`line_defines_*` case with a non-ASCII identifier, asserting
  the definition boundary is found correctly (e.g. the `greeté`-vs-`greet` whole-word
  distinction C15 used in `cargo.rs`).
- `go_provider`: the same shape for its `line_defines_*`/locate path.
State for each test that it fails under ASCII.

## Fence
`crates/redline-resolve/src/providers/{js_provider,go_provider}.rs` (+ their test modules).
Nothing else.

## Gate
`cargo build`, `cargo test --workspace` (state the resolver suites' counts),
`cargo clippy --workspace --all-targets` (read `${PIPESTATUS[0]}`), and
`timeout 900 tools/gate.sh full` **if swap has headroom** — the box has been
swap-exhausted with unattributed SIGTERM kills of rustc/test harnesses under that
pressure, so if it is exhausted run the resolver suites and report the battery deferred.
Budget ~25 tool calls. Report: both conversions, the cross-ref comments, the two
discriminating tests with their ASCII-rule failure evidence, gate counts.
