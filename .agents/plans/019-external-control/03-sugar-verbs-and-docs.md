# Issue 019-03 — Sugar verbs, docs, and the frozen surface

**Objective:** make the CLI pleasant for an agent to drive: ergonomic verbs
over the primitive set, and docs that name the surface. Freeze the JSON
schema and the command-name surface as an API.

**Why now:** 019-02 made the wire work. This issue is what an agent actually
types, plus the contract it relies on.

## Key decisions

- **Sugar is composition, not new state.** `open`, `goto`, `jump`, `search`,
  `isearch`, `select`, `confirm`, `cancel`, `annotate`, `magit` are thin
  sequences over `do`/`key`/`text` (or direct store ops where no command
  exists — e.g. picker select, see W3). Each sugar verb must be expressible
  in one round trip for the attached case.
- **`open PATH[:LINE[:COL]]`** — define exactly how the suffix is parsed and
  that it lands the point, not merely opens the file.
- **Docs are the contract.** README gets a "Driving redline from outside"
  section (attached + headless examples); `redline.1` gets the verbs and the
  socket env/flag; the JSON schema is documented in the plan or a new
  `docs/control-api.md` and **pinned by a test** (a golden-ish assertion that
  the key set is stable).

## Files

| File | Change |
|------|--------|
| `src/session.rs` or `src/control/verbs.rs` | sugar verbs. |
| `src/main.rs` | surface the sugar verbs in the parser/usage. |
| `README.md` | new section + cheat sheet row(s). |
| `redline.1` | verbs, `--socket`, env var. |
| `docs/control-api.md` | **new** — the JSON schema + verb table. |
| `tools/drive_control.py` | extend with one leg per sugar verb. |

## Steps

1. Implement the sugar verbs; document each verb's exact semantics.
2. Freeze the schema: a test asserting the top-level snapshot keys and each
   nested object's keys, so an accidental rename reddens.
3. README + man page + `docs/control-api.md`.
4. Extend the drive: `open file:line`, `goto`, `search`, `select`+`confirm`
   on a picker, `annotate`, `magit`, each asserted via a following `quote`
   or `state`.
5. Update the tracker (`.agents/plans/STATUS.md`) and archive the plan when
   all three issues land.

## Verification

- All gates green; the drive legs registered in both `gate.sh` and
  `pool.py`; README examples run verbatim in a smoke test.
- A developer can copy a README example and get the documented result.
