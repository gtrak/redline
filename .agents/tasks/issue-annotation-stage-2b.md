# issue-annotation-stage-2b — recover same-scope name repeats with a validated ordinal

**Found by:** the 017 symbol-identity gate. The lane correctly refused the spec's raw ordinal (an
ordinal migrates a note to a **sibling** when an earlier same-scope occurrence is deleted — reproduced
in the cited fixture *and* in a non-empty scope: annotate the 2nd `foo()` in
`fn bar() { foo(); foo(); foo(); }`, delete the 1st, and ordinal 1 names the OLD 3rd).

So the shipped key is `(kind, name, enclosing-scope)`, and **two same-named symbols in one scope now
orphan on any drift** rather than being tracked: safe (never a wrong tie) but useless (the note stops
following a symbol that never moved).

## The fix

Key on the ordinal **only together with a content/stability validation**, so a *shift* orphans instead
of migrating. Either fingerprint works and they can be combined:

- validate the occurrence the ordinal resolves to against the **anchor text on its own line** (the
  record already stores `anchor`), or
- compare the **count of same-key occurrences before it** at capture vs at re-anchor.

`SymbolIdentity.ordinal` is already computed and shipped with **no production consumer**, so this is a
resolver change plus the validation — no new parsing.

## Acceptance

- A same-scope repeat **follows its own occurrence** across an insertion above it.
- Deleting an earlier same-scope occurrence **orphans** the note rather than migrating it — that is the
  property that makes the ordinal safe, and it must be pinned with the `fn bar() { foo(); foo(); foo(); }`
  fixture (annotate the middle, delete the first, assert `orphaned` and an unchanged line).
- Legacy records and the scope-only path behave exactly as today.

---

## Addendum 2026-09-23 — struct fields have NO scope, so common types collide constantly

Probing every plausible Rust field-type shape (`symbol_identity_at`) found the tie is always to the
**type**, never the field name — but also that:

```
struct A { name: String, }   ->  kind=type_identifier  name="String"  scope=[]     <-- empty!
struct A { x: u32, }         ->  kind=primitive_type   name="u32"     scope=[]
struct A { p: std::string::String, } -> kind=scoped_type_identifier name="std::string::String" scope=[]
struct A { r: &'a str, }     ->  None (no syntax anchor at all; line-tied)
```

**The scope is empty for a struct field.** The Rust scope walker covers impls/fns/mods but not struct
bodies, so a field's type identity carries no struct name. In a real file `String` (or `Vec`, or any
common type) appears many times, so those occurrences **collide** — and the resolver, refusing to
guess, degrades the note to the text rules. The feature is therefore much weaker than it looks for
exactly the case the user hit.

Two fixes, both here rather than in a new issue:

1. **Make struct bodies (and enum variants) contribute a scope element**, the way impls and functions
   already do. Then `(type_identifier, "String", ["A"])` disambiguates fields of different structs, and
   the cross-field collision largely disappears.
2. **`&'a str` captures nothing.** Establish whether the lifetime or the `str` leaf is responsible and
   make a reference type anchor like any other type. A `None` here is a *silent* degradation to
   line-following, which is the failure mode this whole issue is about.

**Acceptance for the addendum:** a struct field's type identity includes its struct; two structs with
the same field type resolve independently; `&'a str` (and `&mut T`, `[T; N]`, `dyn Trait`) capture an
anchor; and the pins from the main issue still pass.
