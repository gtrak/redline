# issue-language-aware-symbols: symbol constituents must be per-language (Clojure hyphens split today)

**User report (verbatim):** *"in clojure, symbols with hyphens in them are parsed as if each symbol is
split by hyphen. The rules for a symbol should be language aware. eg 'jwks/fetch-issuer-info' is a single
clojure symbol, jwks is the namespace alias at the top-level ns form, and fetch-issuer-info is the var
name."*

## The mechanism (confirmed, not inferred)

`src/model/buffer.rs:76`:

```rust
pub(crate) fn is_word_char(c: char) -> bool { c.is_alphanumeric() || c == '_' }
```

Its doc comment names every consumer: the file view's word motion, `M-?` symbol extraction
(`search::references::symbol_under_point`), the ripgrep word-boundary sink (`search::rg::is_word_boundary`)
and **identifier scanning in `app::store::navigation`**. `AppStore::symbol_at_point`
(`src/app/store/navigation/xref.rs:298-310`) uses it explicitly — `let is_ident = is_word_char;`
— so the jump's token extraction splits `fetch-issuer-info` into three tokens and sees `-` as a boundary.
`crates/redline-resolve/src/cargo.rs:499` duplicates the rule.

Clojure is a registered language (`tree-sitter-clojure = "=0.1.0"`, `LanguageId::Clojure`, with the
flat-S-expression gate for Scheme/Clojure in `crates/redline-syntax/src/language.rs:53-98`).

## Part 1 — the fix: per-language symbol constituents

1. **REPRODUCE FIRST, and determine which side splits.** Build a Clojure fixture containing
   `jwks/fetch-issuer-info` and a top-level `ns` form with a `:require … :as jwks`. Then establish **what
   tree-sitter-clojure actually produces** — dump the node kinds/sexp for that symbol. This decides the
   fix:
   - If the **grammar** yields one node covering `jwks/fetch-issuer-info`, then the grammar is right and
     redline's *word rule* is the bug: prefer the syntax node where one exists, and make the text-level
     fallback language-aware.
   - If the **grammar** splits it, say so plainly — that is a dependency-level finding and the fix has to
     account for it (join the parts, or report it upstream) rather than being papered over in redline.
2. **Make the rule per-language, not global.** `is_word_char` is used by at least four consumers; every
   one of them needs to consult the buffer's language. Do not add Clojure special-cases at each call site
   — that is how the duplication at `cargo.rs:499` happened.
3. **The rules must come from the grammar/reader, not from memory.** Establish each language's actual
   constituents and **write the table into the code or the issue**, e.g.:
   - **Clojure/Scheme**: symbols may contain letters, digits, `* + ! - _ ' ? < > =`, and `.`; `/`
     separates the namespace segment from the name; `:` introduces keywords (and `::`/`::alias/name`
     auto-resolve). **`jwks/fetch-issuer-info` is ONE symbol.**
   - **JS/TS**: `$` is a valid identifier char — check whether it is a boundary today (a global
     `is_alphanumeric` rule says **yes**, which would be the same class of bug for `$foo`).
   - **Ruby**: `?` and `!` are method-name suffixes (`empty?`, `save!`).
   - **Rust**: `_`, and a leading `'` for lifetimes.
   - **Languages where `-` must STAY a boundary**: arithmetic in Rust/Go/JS/C must not become one symbol.
     This is the point of "language aware" — a single global rule is what is being fixed.
4. **Audit table**: list every language redline highlights, its symbol-constituent rule, whether the
   current behaviour is already correct, and what changed. The user asked for the *rules* to be
   language-aware, so a mechanism plus a per-language table is the deliverable; the Clojure hyphen is the
   reported instance.
5. **No regressions**: `M-f`/`M-b` word motion, `M-?` references, the ripgrep word-boundary sink and the
   jump must be unchanged for every language whose rule does not change — and specifically, `-` must remain
   a boundary in languages where it is an operator. Pin both directions per changed language.

## Part 2 — the semantics the user described: the namespace alias

`jwks` is **not** a package or a file: it is a **namespace alias declared in the top-level `ns` form**
(`(:require [some.ns :as jwks])`), and `fetch-issuer-info` is a **var** in that namespace. So the
*correct* jump for `jwks/fetch-issuer-info` is: find the `ns` form, read the `:require … :as jwks` entry,
map that **namespace** to a **file**, and land on the var.

- **Namespace → file is a convention, not a lookup**: `foo.bar-baz` → `foo/bar_baz.clj`, i.e. dots become
  directory separators and **hyphens become underscores**. Verify this against a real Clojure project
  layout (`.clj`/`.cljc`/`.cljs`) rather than assuming it, and state what you verified.
- Handle `::alias/name` (a keyword) as well as `alias/name`, since `:` is the keyword marker in the same
  reader.
- **When the alias cannot be resolved, do NOT guess.** Flag it (the project's existing rule: when identity
  can't be resolved, mark it and never fabricate a target) — a wrong jump is worse than no jump.
- If Part 2 turns out to be too large to land well in one pass, **land Part 1 and report Part 2 as a
  scoped follow-up with a concrete plan** rather than half-implementing it. Part 1 is the reported bug and
  is the priority; Part 2 is what makes the *semantics* right.

## Acceptance

- The reported case: with the point inside `fetch-issuer-info`, redline sees **one** symbol
  (`jwks/fetch-issuer-info` — or the var with a resolvable alias, if Part 2 lands), not three.
- The per-language table exists, with the grammar/reader evidence for each entry.
- Both directions per changed language: the new constituents are one unit **and** the operators that
  should split still split.
- Word motion, `M-?` and the search word-boundary sink behave consistently with the jump for the same
  language (a symbol that jumps as one unit should move as one unit under `M-f`).
- Mutations: revert a language's rule → its test reddens; over-broaden a rule (make `-` a constituent in
  Rust) → the operator test reddens.

## Verification notes for the implementer

- `cargo build` before any PTY check (**`cargo test`/`clippy` do NOT produce the app binary**; a probe
  against a stale binary measures old code — that mistake has been made twice in this project).
- **Never pipe a battery or probe to `tail`** (`cmd | tail` returns *tail's* status and has produced both
  a false pass and a spurious FAIL here). Redirect to a file, then read `$?`.
- `pgrep -x -c timeout` before a battery (exits 1 at count 0 — never chain with `&&`). One battery at a
  time. Box: 48 G used / 0 MB swap with sglang resident — if something fails once, capture the name and
  assertion and re-run before concluding.
- Known intermittent flake: `issue-sweep-file-search-flake`.
- Fence: `src/model/buffer.rs`, the navigation symbol extraction, `search/references.rs`,
  `search/rg.rs`, the file view's word motion, and the language table in
  `crates/redline-syntax/src/language.rs`. Disclose anything else.
