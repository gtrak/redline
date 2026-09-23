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
