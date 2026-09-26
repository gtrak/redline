# 017-00 — the capability audit: which languages can resolve an import/alias/module today, and what would it take

**Read `PLAN.md` first.** This is the plan's entry point and it decides the order of everything after it.

## Why an audit

The plan's three layers (syntax / convention / tooling) exist, but **nobody has measured coverage**.
There are providers for `python`, `js`, `go` and `cargo`, and 19 registered languages — but "19" and "4"
are both **claims**, and this project's rule is that a number is a claim until measured. Do not assume
either figure: **enumerate the languages from `crates/redline-syntax/src/language.rs` and the providers
from `crates/redline-resolve/src/providers/` plus `cargo.rs`, and report the counts you measured.**

## The deliverable: a matrix, one row per language

For **every** registered language, establish and record — with evidence, not from memory:

| column | what it means |
|---|---|
| import/alias/module **forms** | the actual syntax: `import`/`require`/`use`/`#include`/`ns` forms, aliases, `as`/`use`/`refer`, package declarations |
| **grammar evidence** | the **node kinds** the grammar produces for a real import statement. **Dump the sexp** (write a throwaway example against `redline-syntax` and print the tree, then delete it) — this is how the Clojure lane proved `sym_lit` covers a namespaced symbol whole, and it is the only way to be sure. Quote the dump. |
| **resolves today?** | what actually happens on `M-.` on an imported/aliased name right now: lands correctly / lands wrong / bails with no provider. **Measure it**; do not infer from the presence of a provider. |
| **convention rule** | the language's own published name → path mapping (if any), with a **citation** (the language's own docs/tooling behaviour, or the reader/spec). Examples of the *shape*: Clojure namespace `foo.bar-baz` → `foo/bar_baz.clj` (dots → directories, hyphens → underscores, plus `.cljc`/`.cljs` variants); Java `a.b.C` → `a/b/C.java`; C `#include "x/y.h"` → the header. Verify each rather than copying these. |
| **decidable?** | yes / partly / **no**. Be explicit about languages where static resolution is genuinely impossible (C++ ADL and templates, Ruby metaprogramming, dynamic `require`/`importlib`, Java without a source root). Those must end up as **flag, don't guess**. |
| **plan-013 work** | the concrete first step for this language: a query + a conventions-table entry, a provider, or "cannot be resolved — flag it". |

## Rules for the audit itself

- **Both directions per language**: show a name that *should* resolve (and where it should land), and a
  name that *should not* (and that it is flagged, not mis-resolved).
- **No guessing at the forms.** If you cannot dump a grammar node for a language's import form, say so —
  a language whose import syntax you could not establish is a finding, not a gap to fill with an
  assumption.
- **Do not change behaviour** in this issue unless a genuine bug is trivial and in-fence; the audit's job
  is to produce the matrix. If you find a bug (e.g. the Clojure hyphen split's siblings — `$` in JS/TS,
  Ruby `?`/`!`), record it and file it rather than silently fixing it here.
- **Offline and side-effect-free.** No installs, no network, no subprocess that isn't already in-tree and
  sanctioned. Layer 3's existing fetch gate stays exactly as landed.

## Output

- `00-audit-matrix.md` in this plan directory with the table above filled for every language, the
  measured counts, the grammar dumps (quoted), and a **ranked list of the gaps** — ranked by
  *user-visible value × evidence of breakage × cheapness to verify*, with the ranking justified.
- The matrix decides the plan's issue order. State the proposed order and why.

## Verification notes

- `cargo build` before any PTY check (**`cargo test`/`clippy` do NOT produce the app binary**). Redirect
  batteries to a file and read `$?` — **never pipe to `tail`**. `pgrep -x -c timeout` before a battery
  (exits 1 at 0). Box: 48 G used / 0 MB swap — capture a failing assertion before re-running.
- Known intermittent flake: `issue-sweep-file-search-flake`.
