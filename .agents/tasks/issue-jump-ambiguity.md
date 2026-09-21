# Task: tighten `M-.` jump ambiguity, and add a hotkey to force the candidate list

**User request (verbatim):** *"I would like to tighen up the jump ambiguity as much as
possible and add a hotkey to force the list, M-Shift .?"* — `M-Shift .` is `M->` in a
terminal (Shift+`.` sends `>`).

## The diagnosis (verified — this is what "weird choice" means)

The current rule in `xref_find_definitions` (`src/app/store/navigation/definitions.rs:160`) is
**purely by count**:

```rust
if defs.len() == 1 { self.xref_jump_unique_definition(defs); }
else               { self.xref_open_ambiguous_picker(lookup_name, defs); }
```

So there are two independent sources of an unexplained landing:

1. **The tooling fall-through is structurally silent.** When the workspace index has no
   candidate, `M-.` asks the language tooling, and `apply_resolve_event` →
   `open_resolved_source` (`src/app/store/navigation/mod.rs:38`) **always** jumps — there is
   no picker path at all. It lands via `set_point_line(line)` with **no column**, and when
   the provider cannot pin a line it does `.unwrap_or(0)` — **the top of the file**. So
   `M-.` on a module path like `use crate::foo;` silently lands at line 1 of `foo.rs`.
2. **A single index candidate is trusted on count alone** — including a cross-file
   same-named symbol, and including the **enclosing-symbol fallback**
   (`xref_enclosing_symbol_fallback`), which is a guess *by line*, not by the symbol under
   the point.

The picker only ever appears for **index** ambiguity, so neither of the above can reach it.

## What to build

### 1. Silent jump only when the candidate is provably the one under the point

**Jump silently iff: exactly one candidate AND it is in the current file.** Everything else
opens the `PickerKind::Xref` picker, with the best candidate **preselected** — `open_picker`
already starts at index 0 and `defs` is already **same-file-first** ordered, so `RET` still
accepts the top guess in one keystroke. That is what makes an aggressive rule affordable.

Concretely:
- exactly one candidate, **same file** → silent jump (unchanged; you can see the target);
- exactly one candidate, **cross-file** → picker, best preselected;
- 2+ candidates → picker (already);
- **enclosing-symbol fallback** → picker, never a silent jump (it is a by-line guess);
- **tooling fall-through** → see §2.

### 2. The tooling fall-through stops being silent

When the tooling resolve lands, its location must **join the candidate list in the picker**
rather than jumping. A tooling resolve is the weakest kind of answer, so it must be
*visible*: mark the row so a "top of file" landing reads as a weak resolve rather than a
mystery — e.g. a `tooling:` marker in the `detail` (the row shape is
`detail = "[kind] file:line"`, `display = "file:line  [kind] name"`). Preselect it so `RET`
accepts.

**Two mechanics to get right:**
- `apply_resolve_event` / `open_resolved_source` currently take a `ResolvedSource` and jump.
  You need a path that turns it into a `PickerCandidate` instead. Keep the **external-buffer
  registration** (`start_crate_indexing` for `source.external`) and the read-only external
  open — those must not be lost.
- **Capture the jump origin when `M-.` is pressed, not when the picker opens.** The tooling
  path is **asynchronous**, so by the time the resolve lands the point may have moved; if the
  picker records the origin at open time, `M-,` returns to the wrong place. Check how the
  synchronous index picker records its origin and make the async path consistent with it.

### 3. The forced-list hotkey: `M->`

A command that **always** opens the candidate picker for the symbol at point — bypassing the
rule in §1 entirely, including for a same-file unique candidate. This is the user's "when I
know I want to search for the jump point" escape hatch.

**Binding, and the rebinding it requires (verified against the parity reference):**
- `M->` is **currently bound to `point-buffer-end`** (`src/app/store/mod.rs:322`), so it must
  be freed. **Verified with vanilla `emacs -Q` 30.2:** `M->` = `end-of-buffer`, but
  **`M-<end>` = `end-of-buffer-other-window`** — a *window-splitting* command, meaningless
  under redline's locked single-pane design. So **move `point-buffer-end` to `M-End`**: no
  parity loss (its emacs meaning does not apply here), and `G` still binds
  `point-buffer-end` (line 319), so nothing becomes unreachable. State this reasoning in the
  commit.
- Optional, your call: `point-buffer-start` could gain `M-Home` for symmetry (`M-<home>` is
  `beginning-of-buffer-other-window` in emacs -Q, the same situation). Do **not** remove
  `M-<`.
- Register the command (e.g. `xref-find-definitions-picker` / "force the definition list")
  in `src/app/command.rs` **and** the palette, and expect the count assertions to move:
  global bindings 23 → 24, and `load_bindings_equivalence` in `src/app/keymap.rs` asserts
  both the global count and the per-view total. **Restate the history rather than renumber
  it** — that file carries a comment explaining that the pre-change count was 142 and the
  current total is 143; keep that convention and update it honestly.

### 4. Make the *reason* visible in the picker

Since the whole point is that the user can now see the choice, the rows should make the
source legible: same-file vs other-file, and tooling vs index. Extend `detail`/`display`
(do not invent a new picker kind). Keep it terse.

## Constraints

- `M-.` must still find definitions and `M-,` must still return (parity is a standing goal);
  the picker's `RET` must still record the jump with the `M-.` label so the stack works.
- Do not break the external-buffer path (`xref_in_external_buffer`) or
  `find_implementations`, which **reuses the Xref jump path** via `xref_crate_root`.
- The tooling path's async generation/staleness discipline (006-02b) must survive: a stale
  resolve must not open a picker.
- Do not remove the "no symbol under point" / "no definition for X" messages.

## Files

`src/app/store/navigation/definitions.rs` (the rule, the picker, the fallback),
`src/app/store/navigation/mod.rs` (`open_resolved_source`), `src/app/command.rs`,
`src/app/store/mod.rs` (the binding tables), `src/app/keymap.rs` (the count assertions),
plus the tests in `src/app/store/tests/` and `src/app/flow_tests.rs`.

## Verification

- `cargo build`; `cargo test --workspace` — reconcile against the **current** baseline
  (measure it; other lanes have landed: the bin is at **732** passing, and the picker
  convention is that every count assertion is updated honestly) and account for every
  change.
- `cargo clippy --workspace --all-targets -- -D warnings` (`${PIPESTATUS[0]}`).
- **Tests to add** (each must discriminate):
  (a) same-file unique → **silent** jump (assert no picker opened);
  (b) cross-file unique → **picker opens** with the best candidate preselected (assert
      `picker_open()` and that `RET` lands on that candidate);
  (c) the **enclosing-symbol fallback** → picker, not a silent jump;
  (d) a **tooling** resolve → picker with the tooling row present and marked (not a silent
      jump), and `RET` lands;
  (e) `M->` forces the list for a **same-file unique** candidate (the case where `M-.` would
      have jumped);
  (f) `point-buffer-end` still reachable via `M-End` **and** `G`;
  (g) the keymap count assertions (global 24) updated and passing.
- **`timeout 900 tools/gate.sh full`** — the picker renders and this changes the
  most-used navigation command, so the PTY battery is the real check. Other lanes may be
  running (the cursor CUP race fails under concurrent load); if it fails under load, report
  `uptime`/`free -g` and label it rather than asserting a regression.
- Report: the new decision rule and where it lives, how the tooling path reaches the picker
  (and how the origin is captured for the async case), the rebinding with the emacs -Q
  evidence, the picker's source markers, the tests with why each discriminates, and the gate
  output. Budget ~60 tool calls. `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.
