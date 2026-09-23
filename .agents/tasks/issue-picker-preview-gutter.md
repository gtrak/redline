# issue-picker-preview-gutter — the preview pane touches the candidate column

**Found by:** a UX sweep of my own (live PTY frames), not from the backlog.

## Measured

At 80 columns, opening a file (the find-file picker) with a previewable selection:

```
 README.md                                   # target README
 indented_demo.rs            src/indented_demo.rstarget docs here
 lib.rs                                 src/lib.rs
 main.rs                      src/main.rsunstaged_change_marker
```

The candidate column's right edge and the preview pane's first cell are **adjacent**:

- `cand_w = split` where `split = w * 2 / 3`
- the candidate's exclusive right boundary is `right_edge = 1 + cand_w`
- the preview starts at `preview_x = split + 1`

so there is **no separating column**, and any candidate whose label+detail reaches the column edge
runs straight into the preview text — `src/main.rs` + `unstaged_change_marker` reads as one string.
That happens exactly when the user is scanning paths, which is the worst moment for it.

## What this is NOT (correcting my own first reading)

**Not an overlap, and not data loss.** The arithmetic is right, the truncation is right
(`truncate_left(&detail, right_edge - label_end)`), and nothing is drawn over anything. I first read
the frame as a column *collision*; the code showed the columns are exactly adjacent. **Adjacency is
not collision** — a dump is a claim, the code is the evidence.

## Fix

One blank column between the panes: `preview_x = split + 2` (or shrink `cand_w` by one), keeping the
preview's own truncation width consistent with its new start.

**Pin it, do not just fix it:**
- a candidate whose label+detail reaches the column edge must still leave the **gutter blank**;
- the preview must start at the **same column on every row** (the panes must stay scannable), which
  is the property the current layout already gets right and the fix must not break;
- and the *narrow* case must still collapse sensibly — `has_preview == false` already gives the
  candidate rows the full width, and a 1-column gutter must not reintroduce a dead preview strip at
  widths where the preview is suppressed.

## Acceptance

- A live frame at 80 cols with a long path shows at least one blank column between the panes.
- The preview column's start is identical on every row.
- No candidate row's content ever shares a cell with preview text.
- The existing picker tests and `cargo test --workspace` / clippy / `tools/gate.sh full` stay green.

## Meta — the measurement lesson from finding this

The sweep that found it **first produced three false alarms** (`C-x C-f` "doesn't open the picker",
`C-s` "doesn't search", `C-c n a` "doesn't open") because I dumped only **12 of 24 rows**, and the
picker and the isearch prompt both render in rows 13–22. A measurement that cannot see the thing
measured is worse than no measurement — and it was one command to re-run with full frames, which
showed all three working correctly.
