# Task: picker rendering density — name-first rows, viewport sizing, no dead preview

You are the implementation worker. Repo root is your cwd. Self-contained.

## Origin (user report)

Jump-to-definition (and every other picker) renders wastefully: a FIXED
12-row canvas (src/ui/picker.rs:71) in an 80×24 popup leaves half the box
blank when few candidates and shows at most 9 rows when many; the row
string is pre-baked `path:line  [kind] name` (store.rs:3877) and
left-truncated to 2/3 width — so the SYMBOL NAME (at the end) is what gets
chopped; the right third is the preview pane, blank when the selected
candidate has no preview; and every row repeats the same long path prefix.
`PickerCandidate` carries name/display/docs/category and the renderer uses
only `display`.

## What to build (A + B + D from the design discussion)

**A. Name-first rows with a right-aligned detail column.**
- Render the candidate's NAME at the left; a compact detail string
  (`[fn] src/app/store.rs:1234`) right-aligned in the candidate column
  (fixed column at ~62% of the candidate width) so paths form a scannable
  column and the name never truncates.
- Prefer structured fields over the pre-baked `display`: add a `detail`
  field to `PickerCandidate` (or render `name` + a constructed detail at
  the UI); keep `display` working for other pickers, or migrate all
  builders in this lane (judge; the palette/find-file/buffers pickers
  should look at least as good as today).
- Keep the selected-row invert/bar exactly as today.

**B. Size the canvas to content and viewport.**
- `height = min(candidates + 2, viewport_rows - 1)` (prompt row + rows +
  count row), so 3 candidates = a 5-row box and 50 candidates = nearly the
  full popup. The viewport height is available in the root snapshot —
  thread it to `PickerProps`.
- Keep the count row (`N of M`) — it is pinned by the df95113/loop-03
  width-chain tests (the off-screen count-line bug class).

**D. No dead preview space.** When the selected candidate's preview is
empty, give the candidate rows the FULL width (no 2/3 split).

## Constraints / pins to respect

- `src/ui/picker.rs` layout is pinned by `render_at_width_catches_offscreen_picker_count_line`
  (root.rs) and the df95113 width chain — those tests must still pass
  (update only if the assertion's real intent is preserved and stated).
- Row-format changes affect PTY legs: `tools/sweep_flows.py` picker flows and
  `tools/drive_xref.py` (which assert `N of M` and row content). Mirror any
  assertion change; the "jumped to file:line" assertions are out of scope.
- Palette/row-count assertions in store tests + one root.rs test.
- Gate: `cargo test --workspace` (loop-03 twins are the density regression net
  — add render80 twins for the new layout: 3 candidates → 5 rows; long path +
  name not truncated; empty preview → full width), clippy (PIPESTATUS), and
  `tools/gate.sh full`.
- Budget ~45 tool calls; honest-stop at half.
- Fence: `src/ui/picker.rs`, `src/ui/root.rs`, `src/app/store.rs`
  (PickerCandidate + candidate builders + picker state as strictly needed),
  `tools/sweep_flows.py`, `tools/drive_xref.py`, docs (a note if any claim
  changes). NO other modules.
