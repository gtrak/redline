# issue-non-rust-receiver-resolution — receiver resolution is Rust-only, and the non-Rust path hands local variables to providers as package names

**Found by:** the per-language audit (the user asked for one after the annotation anchor turned out to
be Rust-only). Findings F1 + F2 — **the same root cause**, so one issue.

## The asymmetry (measured from the code)

**Rust** has receiver pre-steps in `src/app/store/navigation/definitions.rs:356` (`self.<member>`)
and `:377` (`x.<member>` via `rust_dotted_receiver`), mirrored for external buffers at
`src/app/store/navigation/xref.rs:142,159`. They consume the Rust-only index tables
(`rust_tables_query`, `field_locations`, `trait_impl_locations`). So in Rust a dotted path's
receiver is *resolved* before anything else happens.

**Every other language** gets no receiver step. Worse, it gets the opposite treatment —
`xref.rs:358` keeps Rust `.` access **bare** ("Every other Rust `.` access stays bare"), while
`xref.rs:375` **upgrades** the token for every non-Rust language:

```rust
let path_token = if lang != LanguageId::Rust
    && … Self::dotted_path_container(lang, &info.kind)
    && info.text.split('.').all(|seg| !seg.is_empty() && seg.chars().all(is_ident))
```

## Why this is a P1 and not a wrong lookup

The all-`is_ident` guard was added by the 011-06 review, and its **own comment** says what it is for:
to prevent *"an unintended `npm install "a?"` / `pip install "foo()"` shell-out in online
projects"*. It rejects `a?.b`, `foo().bar`, `(*p).field` — but **`df.head` is all-identifier
segments**, so it passes, and `df` is a **local variable**, not a module.

Both ends verified:

- **The app** upgrades the token to `df.head` (the guard passes; `node_at` returns the container's
  text — `node/mod.rs:437` already pins `node_at(JavaScript, …)` returning `"this.ref"`).
- **The provider** takes the first segment as the package:
  `python_provider.rs:152-164` builds `module_chain = segments[..n-1]` and
  `js_provider.rs:341,358` splits on the first `.` for `base_package_name`.
- **The fetch is silent**: `python_provider.rs:230-238` — *"Not stdlib: try `pip install`
  (sanctioned fetch-on-demand)"* → `run_pip_install`, 120 s network timeout, **no confirmation**.

So M-. on a local variable's attribute can silently fetch a package named after that variable. It is
reachable **inside third-party sources** (a point not enclosed by an indexed symbol, e.g. module-level
code), and redline deliberately supports jumping into library sources — so a dependency can choose
the name that gets installed. That is the guard's stated hazard, reached through the input class it
does not cover.

**I did not execute the install path** — running it fetches a package from the network. The two ends
are established by reading the code; the reachability precondition (point outside an indexed symbol)
needs one store-level test, which is part of the acceptance below.

## Requirement

1. **Never hand a receiver the app cannot classify to a provider as a package/module.** If the
   dotted path's first segment is not a **known import binding** (or the language's path prefix is
   not established for it), keep the token **bare** — the in-project index lookup is the honest
   answer, and the provider's bail is better than a wrong fetch.
2. **Generalise the receiver step** so the non-Rust languages resolve `receiver.member` through
   their own scope/import machinery instead of skipping to the bare name. Python/JS/TS/Tsx/Go
   already have scope and import support (`resolver_scope_for`, `imports.rs`) — this is *using* it,
   not building it.
3. **Confirm before fetching** (defence in depth, and a policy decision to state explicitly): a
   provider that wants to `pip install` / `npm install` a name derived from *source text* should ask
   first, showing the exact command and the file that implied it. Do not leave this implicit.
4. **Do not regress Rust.** Its `.` handling stays byte-for-byte bare except the documented `self.`
   case, and the receiver behaviour it has today must be unchanged.

## Acceptance

- **The leak is closed, with a test that fails today**: M-. on `df.head` in a Python file at a point
  **outside any indexed symbol** must NOT produce `df.head` as a provider symbol; it stays bare (or
  resolves through scope). Same for `this.x` in JS/TS and `x.Field` in Go. The audit confirms these
  shapes are currently **untested** — the existing pins (`tests/navigation/definitions.rs:1352-1411`)
  cover only `a?.b`, `a.b?.c`, `foo().bar`, `(*p).field`, `a[b]`.
- **A genuine module path still upgrades**: `json.dumps` / `os.path.join` (the existing test) must
  keep working, in-project and cross-project.
- **F1's capability**: a receiver that *is* a known import binding resolves for the non-Rust
  languages, pinned per language, with the honest bail when scope is unknown.
- **No network in tests**: the tests assert the *token* / the resolution decision, never an actual
  install. Any test that could fetch is `#[ignore]`d with the reason, following the Go-toolchain
  precedent.
- `cargo test --workspace`, clippy `--workspace --all-targets -- -D warnings`, `tools/gate.sh full`.

## Fence

`src/app/store/navigation/{definitions,xref}.rs`, `src/app/store/*/imports.rs` (scope/hints),
`crates/redline-resolve/src/providers/*` (the package assumption + the confirm), tests and drives.
Disclose anything else with before/after.

## Related, from the same audit (not this issue)

F4: the Rust field/trait tables are a whole capability with **no row in `docs/language-coverage.md`**,
and `rust_fields` is keyed by the **bare** struct name, so same-named structs in different modules
collapse (recorded in the tracker, absent from every `docs/` file). F5: imenu impl-parent grouping is
Rust-only. F6/F7/F9: `M-?`, `type_globs` and `injections_query` have no grid coverage (F7's
`type_globs` also claims to mirror the registry and misses 6 languages, though it is latent — never
set in production). F8: stale doc claims (`notes.rs:303` points at the deleted `src/syntax/`;
coverage doc "all 17 non-Plain"; README "all 14 registry languages", "Rust: cargo metadata" only).
Those are a separate docs-truth pass.
