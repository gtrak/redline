# Task: plan 004 issue 06a — empty buffer table + HOME view (the first half)

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/iocraft/SKILL.md` is authoritative for rendering;
`.agents/skills/*.md` generally are ground truth.

## Why this is SPLIT into 06a + 06b

The original 004-06 spec (`.agents/tasks/issue-004-06-impl.md`) is the largest
and riskiest item left in the repo, and its own text says: *"If this proves
larger than a single issue, STOP and report — do not silently leave half the
invariants; I will split it."* Orchestrator pre-check (2026-09-19, read-only)
confirms the split is warranted:

- `BufferTable::new()` (`src/model/buffer.rs:188`) inserts a fresh `*scratch*`
  and sets it current — so boot ALWAYS has a buffer.
- `src/app/store.rs` has **37** `current_buffer()` uses and **40** existing
  `None`-guards (`unwrap_or(…)` / `current().map(…)` / `let Some(key) = …`).
  The `None` path is *partly* prepared but each boot-path consumer must be
  audited for a `None` that used to be impossible.
- `ViewId` has 96 references across the store; adding `Home` touches name(),
  keymap(), and the root render switch.
- `*scratch*` is asserted at boot by existing tests + PTY flows.

Two halves make the risk tractable and each is independently verifiable:

- **06a (this task)**: the table starts EMPTY, `current` is `None`, `ViewId::Home`
  exists and RENDERS the derived-groups home. `*scratch*` is no longer
  auto-created. Home is NOT a buffer.
- **06b (later)**: remove the remaining scratch affordances, rework the flows'
  key expectations, parity-log row, and the `q`-on-home rules.

If, after reading the code, 06a ALSO cannot land coherently, STOP and report
with exactly which invariant resists — do not half-land it.

## What to build (06a only)

1. **`BufferTable::new()` starts empty**: no scratch insert, `current: None`.
   Keep `SCRATCH_NAME` and `insert_rope(None, …)` available — 06b may still
   create scratch explicitly; this task only removes the AUTO-create.
2. **`ViewId::Home`** — a new variant with a `name()` arm (e.g. `"home"`), a
   keymap (see rules below), and a **root render arm**. Home renders:
   - header: project name + dirty counts + "redline";
   - the derived groups from the live keymap × command registry — REUSE the
     existing machinery (`menu_entries_for_path` / `menu_entries` /
     `menu_rows` / `menu_height` / `menu_bindings` in `src/app/store.rs`);
     add an "all top-level groups" query rather than a new formatter. NEVER a
     hand-maintained string list — an anti-drift TEST must prove it (mutate the
     registry in a test; home's content changes).
   - footer: the standard help line (`C-x C-c` quits, `?` opens the menu).
3. **Render switch**: when there is no current buffer (or the view is Home),
   render Home instead of the file view. Do NOT create a buffer to satisfy the
   render.
4. **Keys on Home**: `q` UNBOUND (no buffer to close — verify Home does not
   inherit anything that quits or closes); `C-x C-c` quits immediately (no
   buffers ⇒ nothing to prompt — the 004-04 semantics); `?` opens the
   descendable transient menu (already works); every existing entry point
   (`C-x C-f`, `C-x g`, `C-x n`, `C-x b`, `C-c p …`) works FROM home and
   REPLACES home with the opened view.
5. **Audit the boot-path `None`s**: every store path reachable with no current
   buffer must behave sanely (no panic, no accidental buffer creation). The
   40 existing guards are a starting point — verify each boot-path consumer,
   and add guards where a `None` was previously impossible.
6. **Update the tests/flows that assert `*scratch*` at boot.** Counts will
   change. Be explicit in the report about each one you changed and why.

## Explicit non-goals (these are 06b)

- Do not remove the `open-scratch` COMMAND (it may stay as an explicit command).
- Do not rework unrelated flows' key expectations beyond what boot state forces.
- Do not touch the parity log (06b adds the splash-divergence row).
- No dependency changes; no undo; do not alter 004-05b point motion, kill/yank,
  or 004-04 quit machinery.

## Constraints

- Skills are truth (iocraft for render; no registry/docs.rs/fetch).
  Write-first; compile early.
- Gate: `tools/gate.sh fast` inner loop, `tools/gate.sh full` final. The gate
  is `--workspace` — do not regress the resolver crate's coverage. Progress
  streams to stderr; do NOT pipe stdout through `tail`.
- PTY flock: "shared PTY fixture is busy" + exit 3 ⇒ wait and retry; NEVER two
  suites concurrently; wrap EVERY python PTY invocation in `timeout`.
- BUDGET: land within ~80 tool calls; no new investigations after the tests
  pass. This is a large issue — if you are at 80 calls with a coherent partial
  state, report rather than thrash.
- Scope fence: `src/app/store.rs`, `src/model/buffer.rs`,
  `src/ui/root.rs` and/or a new home view module, `src/ui/transient_menu.rs`
  (reuse), tests, `tools/` flows, `docs/`. Nothing else.
- Plain `git commit`; do not `git add -A` other sessions' files.

## Verification (iterate until ALL pass)

- `tools/gate.sh full` green with HONEST counts; every changed count explained.
- **Anti-drift test**: mutate the command registry in a test and assert home's
  derived content changes — proving it is generated, not hardcoded.
- PTY flow(s): boot renders home (assert a SAMPLE of real group labels/keys are
  present exactly once); from home `C-x C-f` opens a file (home replaced);
  `C-x g` opens magit; `q` on home is a no-op (app alive); `C-x C-c` from home
  exits immediately with NO prompt; `?` opens the menu.
- Assert `*scratch*` no longer appears in the boot frames.
- No panics reachable from an empty table (grep for the boot-path consumers and
  exercise them via the flows).

## Report format

Home render design (data flow from keymap×registry, and the all-groups query);
the boot-`None` audit table (consumer → guard added/verified); every
`*scratch*`-asserting test/flow you changed and why; per-key behavior on home;
gate counts; skill corrections; deviations; what you deliberately left for 06b.
