# Task: temporarily highlight the symbol you land on after a jump

**User-reported (hands-on, refined live):** *"just like i-search, I want jump back to
temporarily highlight the symbol that was jumped _from_"* — then, confirming: **"and
highlight when jumping to as well."** — then: **"actually I would like some kind of smooth
animation on the highlight."**

So the rule is settled and uniform: **on every jump landing, temporarily highlight the
symbol at the landing point, and animate it.** For `M-,` that symbol *is* the one you
jumped from; for `M-.` it is the definition you jumped to. Same machinery, one rule.

## Why landing-based and not origin-based (do not "fix" this)

An origin-based rule ("highlight where I jumped from") **fails the named case**: on `M-,`
the origin is the *definition you are leaving*, not the symbol you are returning to. The
landing-based rule satisfies both directions with one rule, which is why the user's two
messages are consistent.

## The machinery already exists (verified)

- **The origin and destination are already recorded.** `JumpEntry { buffer_key, line, col,
  label }` (`src/app/store/mod.rs:611`) — `col` is a **0-based CHAR index** (its doc says
  so, and a byte column would land off-by-N on multibyte lines).
- **Two choke points, and they are the only ones you need:**
  - `record_jump(&mut self, origin, label)` (`src/app/store/navigation/mod.rs:97`) — every
    **forward** jump funnels here (`M-.`, the xref/annotations pickers, search `RET`): the
    callers navigate first and then record, so by the time it runs the landing is complete
    and `current_jump_entry()` describes the destination. **Hook after the destination is
    captured.**
  - `navigate_to_entry(&mut self, entry)` (`src/app/store/navigation/mod.rs:~136`) — the
    landing path for `jump_back()` (`M-,`, `:110`) and `jump_forward()` (`C-i`, `:119`).
    The sequence is `set_current` → `bump_current_crate_recency` → `set_point(line, col,
    col)` → `recenter_landing` → `ensure_highlight` → (close the Search view). **Hook at
    the end of the buffer path.** Note the **early return** for the `SEARCH_JUMP_KEY`
    sentinel: that lands in the *results view*, where there is no buffer symbol — skip it.
- **Computing the extent**: `symbol_at_point(lang, text, col) -> Option<(String, String)>`
  (`src/app/store/navigation/xref.rs:235`, `pub(in crate::app::store)`) is the
  syntax-aware symbol at a point — reuse it. The word-boundary walk used by word motion
  (`src/app/store/file_view.rs:728-736`, over `is_word_char` at `src/model/buffer.rs:54`)
  is the fallback for a point with no syntax symbol. **If neither yields a non-empty
  extent, set no highlight** — never highlight an empty or whole-line range.
- **The rendering substrate is the in-flight `match-highlight` lane**: a second segment
  layer in `render_row` plus per-row ranges on `FileViewRow`. **This task depends on that
  one landing first** — and the dependency is hard, not stylistic: both features edit
  `src/ui/file_view.rs` and `src/app/store/mod.rs`, and **two lanes must never edit the
  same file**. Rebase on it.

## What to build

1. **A transient landing-highlight state** — the buffer key, line, and the **byte** range
   of the symbol. It describes the *current* buffer's landing (a cross-file highlight is
   not rendered anywhere, so do not pretend to keep a per-file map unless you can justify
   it).
2. **A helper** that, after a landing, computes the symbol at point and stores the range;
   call it from **both** choke points above (that is the whole point — one rule, all jumps,
   no per-command special cases).
3. **A distinct face** in `Theme` (e.g. `jump_highlight`) added to both constructors —
   **distinct from the search-match faces**, because the meaning differs and the user will
   want to tell them apart. Verify the colour is visible against the default background.
4. **The row overlay**: the landing range renders with that face through the same segment
   overlay the match-highlight lane added.

## The animation — a fade, and what is actually available (verified)

**There is no time-driven re-render anywhere today.** Every `tick.set` is event-driven
(store mutations and the five bus drains in `src/ui/root/hooks.rs`); nothing ticks on a
clock. So the animation needs a **new, self-stopping timer** — that is the main new
mechanism, and the part most likely to cause trouble.

**A genuine RGB fade IS feasible**: `truecolor_enabled()` (`src/ui/mod.rs:79`, a
`COLORTERM` check) already gates an RGB escape path (`bar_bg` at `:97` emits `Color::Rgb`
under truecolor and falls back to the palette otherwise), and `use_future` is already the
established pattern here (the bus drains).

Build it as:

1. **The driver**: when a landing highlight is set, a `use_future` sleeps one frame and
   bumps the revision tick until the animation's duration has elapsed, then **stops**. It
   must not spin when idle (no animation → no timer), and it must genuinely terminate — a
   60 fps loop left running would burn CPU and power forever. Duration and frame interval
   are named constants in one place (~250 ms and ~30 fps are sane defaults).
2. **The curve**: fade out, intensity `1.0 → 0.0` over the duration.
3. **Compute the intensity in the snapshot, not in the renderer**: the row carries
   `highlight: Option<(byte_range, intensity)>`. That makes the curve a **pure function you
   can unit-test without rendering anything**, and keeps `render_row` dumb (it maps an
   intensity to a color).
4. **The color**: interpolate the highlight face's RGB toward the base background by
   intensity, reusing the existing adaptive strategy. **Without truecolor, do not fake a
   fade** — hold the highlight for the full duration and then clear (a flash), and say so.
   A 16-color palette cannot do "smooth"; pretending otherwise looks like a bug.

**Flag this interaction, do not discover it in the battery**: the render loop's cursor
workaround spawns a task and sleeps per frame, and the known cursor CUP race (plan 013)
already fails ~50% under concurrent build load (80/80 idle). Animating multiplies the
frames that workaround runs on. So: keep the animation short, and **verify the cursor still
tracks during and after an animated jump** with `tools/check_cursor_stream.py`. If the
animation makes the cursor visibly misbehave, that is a finding for this lane and the
honest response is to shorten or drop the animation — **never to widen the sleep** (that is
explicitly rejected in plan 013).

## The traps

- **Multibyte**: `JumpEntry.col` is a char index and `set_point` consumes char indices, but
  the span layer is **byte** offsets. Convert explicitly (the recurring bug class here —
  the isearch column bug and the four column landings were all this). Pin it with a
  multibyte test.
- **Lifetime — "temporarily".** Recommended and simplest honest rule: **the highlight lives
  one command** — it is set during the jump and cleared at the start of the next command
  dispatch (a jump replaces it). That is genuinely transient, cannot go stale, and matches
  the isearch analogy (the highlight exists while the action is current). **Pin it with a
  test.** If the one-command rule feels too brief in practice, the relaxation is a single
  place — say so in your report rather than inventing a timer.
- **The sentinel path** (`SEARCH_JUMP_KEY`) must not set a buffer highlight.
- **A jump that lands where no buffer exists** (`navigate_to_entry`'s "no buffer" early
  return) must not set a highlight.
- **Do not confuse `ensure_highlight()`** (already in the landing sequence) — that is the
  *syntax* highlight refresh, not this feature.

## Files

`src/app/store/navigation/mod.rs` (the two hooks + the helper), `src/app/store/mod.rs`
(the transient state), `src/theme.rs` (the face + both constructors),
`src/app/store/file_view.rs` (put the range on the row), `src/ui/file_view.rs` (render it
through the overlay), tests.

## Verification

- `cargo build`; `cargo test --workspace` — reconcile against the **current** baseline
  (measure it; the match-highlight lane will have moved it) and account for every change.
- `cargo clippy --workspace --all-targets -- -D warnings` (`${PIPESTATUS[0]}`).
- **Tests to add** (each must discriminate):
  (a) `M-.` sets a landing highlight whose range covers the definition symbol;
  (b) `M-,` sets one covering the **origin** symbol (the named case);
  (c) the highlight is **cleared by the next command** and **replaced** by a second jump;
  (d) a **multibyte** line: the range is the right byte extent for the right chars;
  (e) the search sentinel landing (`SEARCH_JUMP_KEY`) sets **no** buffer highlight;
  (f) a landing at a point with no symbol (e.g. whitespace) sets none — no empty range;
  (g) the **intensity curve** is a pure function: monotone non-increasing, `1.0` at the
      start, `0.0` at or before the duration (no rendering needed);
  (h) the **timer terminates**: no tick bumps once the animation has completed, and no
      timer is started when no highlight is set;
  (i) with truecolor **disabled**, the highlight holds for the duration then clears (the
      honest degradation), rather than emitting a bogus RGB.
- **`timeout 900 tools/gate.sh full`** — this is a rendering change, so the PTY battery is
  the real check. Note other lanes may be running (the cursor CUP race fails under
  concurrent load); if the battery fails under load, report the load and label it rather
  than asserting a regression.
- **`python3 tools/check_cursor_stream.py`** specifically, with an animated jump exercised:
  the cursor must still land on the selected row. This is the check that would catch the
  animation's interaction with the known CUP race (see above) — report its observed output.
- Report: the two hook sites, how the extent is computed and the no-symbol fallback, the
  face and its colour, the lifetime rule with its test, the byte/char handling, and the
  gate output.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.
