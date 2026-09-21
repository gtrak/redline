# Plan 013 — the hardware-cursor race (write-up + decision)

**Status:** analysis complete, decision OPEN (user). Nothing implemented.
**Origin:** found during plan 012's deflake work (lane `deflake-cursor`, `b47e03c`),
re-measured by its gate (`5d25dc0`), refined by the final 012 battery (`61ad9bd`).

---

## 1. Why this exists (the short version)

Redline draws a hardware cursor (emacs `-nw` parity: the terminal cursor sits on
*point* — the selected row for list views, the top visible line for the buffer
view). iocraft hides the cursor and re-parks it at the status line every frame, so
redline has to write its own cursor position **after** each frame.

It does that from a **detached task that sleeps 12 ms first**. That 12 ms is a
*guess* at landing between two points inside iocraft's frame, and it is the whole
bug: when a frame takes longer than 12 ms (a loaded machine), the guess is wrong.

Two consequences, one cosmetic and one serious:

- **Cosmetic:** on a busy machine the cursor can sit at the status line instead of
  on point, until the next frame re-asserts it. Frame *content* is never affected.
- **Serious:** the PTY cursor suite (`tools/check_cursor_stream.py`) fails ~50% of
  the time under load and passes 100% idle. **A verification gate whose signal
  depends on machine load is a hazard** — it trains you to re-run until green,
  which is how real failures get waved through.

Everything below is the mechanism, the evidence, and the fix options. If you read
one section, read §2 and §3.

---

## 2. The mechanism, exactly

### 2.1 What iocraft does with the cursor

All references are iocraft **0.9.1** (`~/.cargo/registry/src/*/iocraft-0.9.1/`).

| Step | Where | What it emits |
|---|---|---|
| once, at startup | `src/backend/crossterm.rs:767` | `cursor::Hide` → `?25l` |
| open a frame | `src/backend/crossterm.rs:671` (`begin_frame`) | `BeginSynchronizedUpdate` → `?2026h` |
| **park the cursor** | `src/backend/crossterm.rs:583-586` | `MoveTo(0, top_row + canvas.height() - 1)` — the **bottom row**, i.e. the status line |
| close a frame | `src/backend/crossterm.rs:677` (`end_frame`) | `EndSynchronizedUpdate` → `?2026l` |

and the render loop wraps the whole thing:

```rust
// iocraft-0.9.1/src/render.rs:481-497  (terminal_render_loop)
term.synchronized_update(|mut term| {
    let output = self.render(terminal_size, Some(&mut term));   // components update HERE
    if ... { term.write_canvas(prev, &output.canvas)?; }        // ...and the park happens HERE
    Ok(())
})?;                                                            // ?2026l
```

Note the order: **the park is inside the synchronized region, and it is the last
cursor movement of the frame.**

### 2.2 Why redline cannot just write the cursor in its component

iocraft's `use_effect` fires *during* `self.render(...)` — which is **inside**
`synchronized_update` and **before** `write_canvas` runs. So a direct write there is
overwritten by the park microseconds later. Redline's own source documents this
(`src/ui/root/mod.rs:368-372`):

> iocraft's own effect hook fires mid-frame (after `?2026h`, before the content
> draw), so a direct write here would be clobbered by the frame's status-line park
> (`24;1`).

### 2.3 The workaround, and why it is a race

```rust
// src/ui/root/mod.rs:381-394
hooks.use_effect(move || {
    if !cursor_live { return; }
    let cell = cursor_cell_opt;
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(12)).await;   // ← the guess
        if let Some((col, row)) = cell {
            let _ = crossterm::execute!(std::io::stdout(), Show, MoveTo(col, row));
        }
    });
}, (&revision,));
```

`12 ms` was chosen because "the measured frame flush is ~5 ms" (same comment).
That makes correctness depend on frame duration, which depends on machine load.
There are now **two writers to the same stdout** (iocraft's render loop and this
task), ordered only by that sleep.

The bet fails in both directions:

| Direction | Condition | Result |
|---|---|---|
| **too early** | frame takes > 12 ms (loaded box) → CUP lands mid-frame | clobbered by the park → cursor stays at the status line for that frame |
| **too late** | task scheduled late (starved runtime) | CUP not in the chunk the test read → `cup=None` |

Plus a third, related sub-class observed by the gate: **pre-frame starvation**
(`?25l=0` — even the startup hide missed the read window, along with stale-content
reads). Same family: timing assumptions between a writer and a reader. The harness
gate in §3 structurally *cannot* cover this one, because it only guards the CUP.

### 2.4 What is NOT broken

This is important for scoping the fix:

- **The computed position is correct.** `cursor_cell(&snap)` reads the snapshot,
  i.e. the motion result. The drive's own expected values (`want=4`, `want (6,11)`)
  confirm the app's state was right — the app knew where the cursor belonged.
- **Frame content is never wrong.** It is written inside `?2026h … ?2026l` and is
  unaffected by the cursor race.
- **Motion is never wrong.** The failing tests are *motion* tests only because
  those are the tests that assert the cursor. `check_cursor_stream.py` tracks the
  CUP across `n`/`p`/`M-b`/`M-f`/`C-f`/tree-click, so any missing CUP surfaces
  under a motion leg's name.

So the defect is precisely: **a stale cursor for one frame on a loaded machine,
plus a load-dependent test signal.**

---

## 3. Evidence

| Measurement | Result |
|---|---|
| `check_cursor_stream.py`, **idle** box (final 012 battery on main `61ad9bd`) | **80/80 PASS**, 71 s |
| Same suite, **under confirmed concurrent load** | **7/10, then 3/8 FAIL** — dominated by `cup=None` *after* the full 5 s wait (`5d25dc0`) |
| App-side fix inside the current fence | **verified impossible** on 0.9.1 — no post-canvas/post-frame seam |
| Harness gate landed (`b47e03c`) | converts a silent mis-read into a **loud protocol failure** (`Session.cup_settle`: the read stays open until a CUP after the final `?2026l`, hard deadline `CUP_WAIT_CAP`) — it does **not** fix the race |
| Load multiplier | swap exhaustion (8,191/8,191 MB, 0 free) — a correlate, not a deterministic cause |

The suite's own header states the contract it is enforcing
(`tools/check_cursor_stream.py:10-11`): *"The CUP that survives to the end of a
frame (the last CUP after the last `?2026l`) lands on the selected (blue-bar) row
and tracks it across n/p."*

### How to reproduce (diagnosis only)

```bash
# Idle — expect PASS:
timeout 300 python3 tools/check_cursor_stream.py

# Under load — expect the race to show. In another shell:
export CARGO_BUILD_JOBS=4 && cargo test --workspace      # induce load
timeout 300 python3 tools/check_cursor_stream.py         # now expect cup=None
```

**Do not do this while a gate or another lane is running** — the point is to
reproduce the condition, and it will also disturb any concurrent PTY suite. This
is a diagnostic recipe for a scratch tree, never a gate step.

---

## 4. What "fixed" means

The CUP must be written by the **same writer**, into the **same queue**, inside the
**same synchronized region**, immediately after the park and before `?2026l`.
Then the ordering is **program order**, not a timing bet.

**Success criteria**

1. `check_cursor_stream.py` passes N/N **under concurrent build load** — the
   condition that reproduces the bug. (Passing idle proves nothing; it already does.)
2. The `sleep(12ms)` and the detached `spawn` are **deleted**; app code emits no
   raw cursor escapes at all.
3. The frame path is otherwise unchanged; no new flake class appears.
4. The PTY gate's signal no longer depends on machine load.

---

## 5. Options (the open decision)

| # | Option | What it involves | Cost | When it is right |
|---|---|---|---|---|
| **A** | **Upstream PR** (recommended) | Propose `use_cursor_position(col, row)` — the app declares where the hardware cursor goes; the backend emits `Show` + `MoveTo` after the park, inside the sync region. Precedent: ratatui's `Frame::set_cursor_position`. | PR may sit; using it immediately needs B anyway | **Always worth doing** — cheap, the right API, defensible on its own merits |
| **B** | **Vendored patch** | ~20-30 lines in `write_canvas`'s fullscreen branch (right after the park at `crossterm.rs:583-586`) reading a cursor position held on the backend, plus a public setter the hook calls. Pin via `[patch.crates-io]` / path dep. | You own a fork across **every** iocraft bump (the pin is 0.9.1; this cycle alone did tree-sitter 0.24.7→0.25.10 + 9 grammar bumps) | If you want the fix **now** and the PTY gate load-independent |
| **C** | **Accept** | Keep the harness gate loud; document that the cursor suite must run on a quiet box. | Cursor lags one frame on a busy machine; the PTY suite stays load-dependent | If the cursor is judged cosmetic and the gate is always run idle |
| **D** | Widen the sleep / auto-retry | — | Guessing harder; retry hides a real failure | **Never** (recorded rule: a test failure is never retried; only a signal-kill is) |

### Why option A is not a wild ask

iocraft **already has** the pieces:

- an app→terminal write channel: `use_output` → `Passthrough { stream, content, newline }`
  → `TerminalBackend::print_above` (`src/backend/mod.rs:52-70`);
- terminal access during the update phase: `render()` receives
  `Option<&mut Terminal>` and puts it in `UpdateContext.terminal`
  (`src/render.rs:379-390`) — that is how `use_output` works today;
- a single frame boundary to hook: the `synchronized_update` closure
  (`src/render.rs:481-497`).

What is missing is only that the channel is defined as *"lines above the canvas,
reflowed"* rather than *"a cursor directive after it"*. So the proposal is an
**extension of an existing mechanism**, not a new subsystem.

---

## 6. Task order

| Issue | Depends on | What |
|---|---|---|
| `01-upstream-pr.md` | — | Write and file the iocraft proposal (`use_cursor_position`) with a minimal repro. |
| `02-vendored-patch.md` | 01 rejected/slow **and** user chooses B | The ~20-30-line patch + pinning + the redline-side call site that deletes the sleep. |
| `03-harness-preframe.md` | — | Cover the **pre-frame starvation** sub-class (`?25l=0`, stale-content reads) that `cup_settle` cannot: the gate guards the CUP only. Independent of the app fix. |

---

## 7. Cross-references

- `.agents/plans/012-project-organization/00-worklist.md` § **Gate reliability** — the
  four flake classes this belongs to (class 1), with the measurements.
- Commits: `b47e03c` (harness gate — diagnosis, not a deflake), `5d25dc0` (re-measured,
  root-caused, "app-side fix impossible in-fence"), `daeac06` (three flake classes),
  `61ad9bd` (final battery: 80/80 idle).
- `src/ui/root/mod.rs:361-394` — the effect and its (accurate) comment.
- `tools/check_cursor_stream.py` — the suite; header lines 10-11 and 37-43 state the
  CUP contract and the `cup_settle` gate.
