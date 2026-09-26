# 017 — worklist (from the 00-audit matrix)

The **matrix** in `00-audit-matrix.md` is the evidence; this file is the plan's order and the decisions
taken from it. Every figure below was measured by the audit, not assumed.

## Measured baseline

- **19** registered languages (18 with grammars + Plain). **4** tooling providers (`cargo`, `python`,
  `js`, `go`) covering **6 language rows** (Rust, JS, TS, TSX, Python, Go). Scope-hint extractors exist for
  those same 6 rows only.
- **The grammar is not the problem.** All 18 grammars dumped clean (`has_error = false`), and Clojure's
  `jwks/fetch-issuer-info` is **one `sym_lit`** (`sym_ns` + `/` + `sym_name`) with `::jwks/local-opt` as one
  `kwd_lit`. The split is **redline's word rule** (`is_word_char`), confirming `issue-language-aware-symbols`.
- **The user's reported case fails in two places**: the index stores `fetch-issuer-info` **whole**, but
  extraction yields `issuer` (the word-rule split), and there is **no Clojure provider**. Plain Clojure vars
  do resolve through the picker.
- The Clojure convention was **verified against real layouts on this box** (`kaocha.core-ext` ↔
  `src/kaocha/core_ext.clj`, jar layouts, `.cljc`), not copied from memory.

## Order (strict value-ordering after the mechanism is free)

| # | issue | why here |
|---|---|---|
| 01 | **The mechanism** — import/alias extraction queries + a **conventions table** (name → relative path) + the resolution order (tooling → convention → syntax) | Makes every later language a **table row + a query** instead of a branch in `xref`. Offline-testable, so it lands without any toolchain. |
| 02 | **Clojure** alias/namespace wiring | The user's own report; end-to-end breakage measured; pure convention. **Depends on the sibling lane** (`issue-language-aware-symbols`, slot 1) for the constituent rule — re-verify against the pinned grammar when wiring. |
| 03 | **Java** `a.b.C` → `a/b/C.java` | Unambiguous by the JLS, pure convention, no toolchain. |
| 04 | **C/C++ quoted `#include`** (relative) | The cheapest real resolution, and it **carries the B5 decision below**. |
| 05 | **Go** in-module path→dir | Toolchain-gated (F4: no go on this box — the legs must be unit-level or loudly skipped). |
| 06 | **Ruby** `require_relative` | After the `?`/`!` constituent bug (B3) is fixed, since the name is extracted through the same rule. |
| 07 | **Bash** relative `source` / `.` | Same shape as 04, smaller payoff. |
| 08 | **The flag rows** | Last **on purpose**: C#, Scheme bare `require`, Ruby bare `require`, C++ semantic resolution, Python relative imports. These document the **residual** — what we have decided *cannot* be resolved statically — so they must be written after the resolvable set is known and the matrix is final. |

## Decisions taken

**B5 — the enclosing-symbol fallback misroutes an unresolvable name (accepted; the audit's recommendation
plus a precision).** Measured: `M-.` on a C function called inside `main` — declared in a header that this
project cannot resolve — does **not** bail. It opens a **picker on `main`**. That is a *wrong jump* where
the project's own rule is that a wrong jump is worse than an honest miss, and it affects **every
language without a provider**.

The rule to implement:

- If the token at the point **is** the enclosing symbol, the fallback is correct — picking it is the
  intended behaviour. Keep it.
- If the token is a **different** symbol and resolution failed, the fallback must **not** fire: report the
  name as **unresolved** (a named flag, like the annotations' `orphaned`) rather than showing the enclosing
  function, which is a plausible-looking lie about where the definition is.

**F2 — the per-provider miss detail is swallowed** (accept as a small, separate fix): the refusal reason
from the fetch confirmation **never reaches the minibuffer**. The confirmation gate landed, but when it
declines, the operator sees a refusal with no reason. Security-adjacent and cheap.

## Bugs found by the audit (filed, not fixed there)

- **B1–B4** — symbol-constituent splits: Clojure hyphen and `/`; **JS/TS `$`**; **Ruby `?`/`!`**; **Rust
  lifetimes `'`**. All are `issue-language-aware-symbols` (slot 1's scope) — the audit independently
  confirmed the same class the issue predicts.
- **B5** — the enclosing-fallback misroute (above).
- **F1** — the js provider's `"jsx"` dispatch string is dead.
- **F3** — `.mdx` maps to the Markdown grammar, so MDX imports are invisible.
- **F4** — no `go` toolchain on this box (go paths untestable offline).
- **F5** — a predicted go spawn-error shape (unverified).
- **F6** — Scheme's `(foo core)` library form is indexed under `foo` only.

## Method note (reusable)

The audit's evidence standard is the one to keep for the rest of this plan: **dump the grammar and quote
it**, **measure `M-.` rather than infer it from a provider's presence**, **verify conventions against real
layouts on the box**, and **file what you find instead of fixing it in an audit**. Probes must be bounded
by construction — one-shot, print, return.
