# Task: plan 011 issue 01 — language dispatch + register the non-Rust providers

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth.

## Origin (orchestrator investigation, 2026-09-19)

`M-.` in a JS/Python/Go project falls through to a cargo-only provider chain
and dies on "no Cargo.toml under workspace root". But
`crates/redline-resolve/src/providers/{js,python,go}_provider.rs` are fully
implemented and unit-tested (they shell out to real npm/python3/go). They are
simply **never registered**:

- `src/app/store.rs:7684` — `chain.add(CargoProvider::new());` is the ONLY
  `chain.add` in the codebase (the comment above it reads "Adding more
  languages is a one-line `chain.add(…)` here").
- `crates/redline-resolve/src/lib.rs` `resolve_traced` (~line 212) walks every
  provider in order and **never consults the trait's `languages()`** — so a
  naive registration would probe all four toolchains on every miss.
- `SymbolContext` (`lib.rs:121`) has no language field, so a provider cannot
  know what it is resolving.

## What to build

1. **Give the chain a language.** Two acceptable shapes — pick one and
   justify:
   (a) add `pub language: Option<String>` (lowercase, e.g. "python", matching
   the trait's `languages()` strings) to `SymbolContext`, and have
   `resolve_traced` skip providers whose `languages()` does not contain it;
   or (b) keep `SymbolContext` unchanged and add an explicit
   `Resolver::resolve_for_language(&self, lang, ctx)`. (a) is simpler for
   callers; (b) keeps the context strictly about the symbol. Whichever you
   choose, the SKIP must be in `resolve_traced` (the single chain walk), and it
   must be recorded in the trace (an attempt with a "skipped: language
   mismatch" reason, or simply not attempted — say which, and keep the trace
   honest/useful).
2. **Backward compatibility**: a `None`/unset language must preserve today's
   behavior (try providers in order) — the existing Rust path and all existing
   tests must keep passing byte-for-byte. Verify `store.rs` passes the language
   it already knows (`self.grammar_registry.language_for(&path)` — see
   `store.rs:~5263` for the existing pattern, and `LanguageId::name()` for the
   lowercase string).
3. **Register the three providers** in `src/app/store.rs` (the `chain.add`
   site), in a sensible order. Consider: should the chain be built per-jump
   (it currently is, inside the `spawn_blocking` closure) or should language
   dispatch make order mostly irrelevant? Answer briefly.
4. **Fix the "no Cargo.toml" confusion**: with dispatch in place, a Python
   buffer must never reach the cargo provider. Confirm by test, not by
   inspection.
5. **Make a miss cheap and honest.** No miss may probe a non-matching
   provider. Add a test asserting the trace contains no attempt for
   mismatched providers.

## Explicit non-goals

- No syntax work (node-at-point/scope hints are issues 02–03).
- No crate/module index changes (issue 04).
- No LSP, no new dependencies.
- Do not change any provider's internals (they are already tested) — only the
  chain, the context, and the app wiring.

## Constraints

- Skills are truth (`.agents/skills/*.md`; no registry/docs.rs/fetch).
  Write-first; compile early. Keep `crates/redline-resolve` app-free (plain
  data only — a lowercase language STRING is fine; do not import app types).
- Gate runner: `tools/gate.sh fast` (inner loop), `tools/gate.sh full`
  (final). There is also `tools/gate.sh pooled` (parallel, ~2.5 min) — either
  is acceptable for the final gate; progress streams to stderr, do NOT pipe
  stdout through `tail`.
- PTY flock: "shared PTY fixture is busy" + exit 3 ⇒ wait and retry; NEVER two
  suites concurrently; wrap EVERY python PTY invocation in `timeout`.
- **NOTE: 007-03 may be landing in this same tree (it also edits
  `SymbolContext` + `store.rs`).** Check `git log --oneline -3` and
  `git status` FIRST. If 007-03's scope field is present, build on it; if the
  tree is mid-edit by another lane, STOP and report rather than fighting it.
- Plain `git commit`; do not `git add -A` other sessions' files.

## Verification (iterate until ALL pass)

- `tools/gate.sh full` (or `pooled`) green with HONEST counts; all existing
  suites unchanged.
- Resolver-crate tests:
  - a chain with cargo + python, context language "python" → only python is
    attempted (assert on the trace / a spy provider);
  - language "rust" → only cargo;
  - language None → existing in-order behavior (regression pin);
  - a provider whose `languages()` is empty (check if any exists) behaves
    sanely.
- App-side tests: the context built for a `.py`/`.js`/`.go` buffer carries the
  right language string; a Rust buffer still carries "rust".
- If practical, a PTY leg: in a small Node or Python project, M-. on a
  path-shaped symbol (`lodash.map` / `os.path.join`) lands in real installed
  source. Reuse the `drive_external_crate.py` pattern (own repo + own flock).
  If the toolchain is unavailable in the sandbox, say so explicitly and rely
  on the unit tests — do NOT fake a pass.
- Live check in this repo: confirm the Rust path is unchanged (M-. on a
  dependency symbol still resolves as before).

## Report format

The language-dispatch shape chosen and why; the chain order + per-jump vs
per-language reasoning; the trace behavior for skipped providers; the language
string mapping (`LanguageId` → `languages()`); test list with discriminating
tests called out; gate counts (honest); whether the PTY leg ran or why not;
skill corrections; deviations.
