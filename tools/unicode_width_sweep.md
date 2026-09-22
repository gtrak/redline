# unicode-width 0.1.14 → 0.2.2 — full-domain width sweep (deps batch B)

Recorded by the `annot-polish` lane (commit that lands this file supersedes the
range summary in `e17ec1a`, which listed truncated ranges — use the ranges
below). Regenerate the numbers from the repo:

    cargo run --example unicode_width_sweep

The example sweeps every Unicode scalar value through the OLD table
(unicode-width 0.1.14, kept as the dev-only `unicode-width-old` manifest
entry — dev-dependency only, never linked into the shipped binary) and the
current table (unicode-width 0.2.2, Unicode 17.0 — see `UNICODE_VERSION` in
the 0.2.2 `tables.rs`), and prints the totals, the per-bucket counts, and
every changed codepoint.

## Why full-domain

The fixed 7-character sample (one unambiguous narrow, one CJK wide, one
emoji, one combining mark, three ambiguous-width chars) showed **zero**
width changes across the bump. The full domain showed **458**. A
sample-only measurement would have concluded "no behaviour change" — the
sweep is full-domain (all 1,112,064 scalars) precisely so that cannot
happen again.

## Summary

    total_scalars=1112064
    changed=458
    0->1: 2
    1->0: 99
    1->2: 355
    2->0: 2

## What the delta is (and is not)

The 458 deltas are **East Asian Width reclassifications** under the
Unicode 17.0 tables, not ambiguous-width handling:

* **Neutral → Wide** (`1 -> 2`, 355 scalars) — e.g. musical symbols
  U+1D300-U+1D356 / U+1D360-U+1D376, Cyrillic U+4DC0-U+4DFF, Mongolian
  U+187F8-U+187FF / U+18CFF / U+18D09-U+18D1E / U+18D80-U+18DF2, Symbols
  for Legacy Computing (U+1FA8x-U+1FAFx), misc symbols U+2630-U+2637 /
  U+268A-U+268F.
* **Mark → combining/zero-width** (`1 -> 0`, 99 scalars; `2 -> 0`, 2
  scalars) — e.g. Kangxi radicals supplement U+1ACF-U+1ADD /
  U+1AE0-U+1AEB, select Hangul jamo, Greek U+1D166 / U+1D16D, and the
  Kanbun pou/tou marks U+16FF0-U+16FF1.
* **Reclassified spacing** (`0 -> 1`, 2 scalars) — U+1171E, U+11A3A.

**0 of the 458 deltas are in the EAW "Ambiguous" class**, and that class
changed **zero** widths across the bump (n=138,197 in 0.1.14 → 138,232 in
0.2.2, all still 1-cell in the default non-CJK table — `width()` treats
Ambiguous as 1 column in both versions). All 458 changes track the newer
Unicode properties (more correct), and none of the 458 scalars appears in
any test fixture, so no expectation encoded the old table.

### Corrected per-bucket ranges (superseding the `e17ec1a` summary)

* `1->2` (355): U+2630-U+2637, U+268A-U+268F, U+31E4-U+31E5, U+4DC0-U+4DFF,
  U+16FF2-U+16FF6, U+187F8-U+187FF, U+18CFF, U+18D09-U+18D1E,
  U+18D80-U+18DF2, U+1D300-U+1D356, U+1D360-U+1D376, U+1F6D8,
  U+1FA89-U+1FA8A, U+1FA8E-U+1FA8F, U+1FABE, U+1FAC6, U+1FAC8, U+1FACD,
  U+1FADC, U+1FADF, U+1FAE9-U+1FAEA, U+1FAEF
* `1->0` (99): U+0897, U+1715, U+1734, U+1ACF-U+1ADD, U+1AE0-U+1AEB,
  U+1B44, U+1BAA, U+1BF2-U+1BF3, U+A953, U+A9C0, U+10D69-U+10D6D,
  U+10EFA-U+10EFC, U+111C0, U+11235, U+1134D, U+113B8-U+113C0, U+113C2,
  U+113C5, U+113C7-U+113C9, U+113CE-U+113D2, U+113E1-U+113E2, U+116B6,
  U+1193D, U+11B60, U+11B62-U+11B64, U+11B66, U+11F41, U+11F5A,
  U+1611E-U+16129, U+1612D-U+1612F, U+1D166, U+1D16D, U+1E5EE-U+1E5EF,
  U+1E6E3, U+1E6E6, U+1E6EE-U+1E6EF, U+1E6F5
* `2->0` (2): U+16FF0-U+16FF1
* `0->1` (2): U+1171E, U+11A3A

## Fixed sample (no change across the bump)

| char | class | 0.1.14 | 0.2.2 |
|---|---|---|---|
| `a` U+0061 | unambiguous narrow | 1 | 1 |
| `中` U+4E2D | CJK wide | 2 | 2 |
| `\u{1F980}` (crab) | emoji | 2 | 2 |
| U+0301 | combining mark | 0 | 0 |
| `©` U+00A9 | ambiguous | 1 | 1 |
| `Ā` U+0100 | ambiguous | 1 | 1 |
| `≡` U+2261 | ambiguous | 1 | 1 |

## Effect on this app

All width math funnels through `src/model/text_width.rs`
(`char_display_width`), so cursor placement, truncation
(`truncate_ellipsis`), and the gutter/click mapping all use the new table
consistently: a buffer line containing one of the 458 scalars now computes
its width per the (more correct) Unicode 17.0 properties everywhere in the
app. iocraft 0.9.1 is held on unicode-width 0.1.14 (its manifest pins
`^0.1.13`; both versions coexist in the lock), so if iocraft
measures text width internally, only these 458 exotic scalars could
disagree between the two tables.

Pins: `src/model/text_width.rs::width_table_pins_022_delta_and_stable_anchors`
anchors one scalar per changed bucket (U+2630→2, U+1ACF→0, U+16FF0→0,
U+1171E→1) plus the stable narrow/wide/emoji/combining/ambiguous anchors.

## Recorded raw output

Full output of `cargo run --example unicode_width_sweep` at the time of
recording (re-running now must reproduce it byte-for-byte):

total_scalars=1112064
changed=458
0->1: 2
1->0: 99
1->2: 355
2->0: 2
00897: 1 -> 0
01715: 1 -> 0
01734: 1 -> 0
01ACF: 1 -> 0
01AD0: 1 -> 0
01AD1: 1 -> 0
01AD2: 1 -> 0
01AD3: 1 -> 0
01AD4: 1 -> 0
01AD5: 1 -> 0
01AD6: 1 -> 0
01AD7: 1 -> 0
01AD8: 1 -> 0
01AD9: 1 -> 0
01ADA: 1 -> 0
01ADB: 1 -> 0
01ADC: 1 -> 0
01ADD: 1 -> 0
01AE0: 1 -> 0
01AE1: 1 -> 0
01AE2: 1 -> 0
01AE3: 1 -> 0
01AE4: 1 -> 0
01AE5: 1 -> 0
01AE6: 1 -> 0
01AE7: 1 -> 0
01AE8: 1 -> 0
01AE9: 1 -> 0
01AEA: 1 -> 0
01AEB: 1 -> 0
01B44: 1 -> 0
01BAA: 1 -> 0
01BF2: 1 -> 0
01BF3: 1 -> 0
02630: 1 -> 2
02631: 1 -> 2
02632: 1 -> 2
02633: 1 -> 2
02634: 1 -> 2
02635: 1 -> 2
02636: 1 -> 2
02637: 1 -> 2
0268A: 1 -> 2
0268B: 1 -> 2
0268C: 1 -> 2
0268D: 1 -> 2
0268E: 1 -> 2
0268F: 1 -> 2
031E4: 1 -> 2
031E5: 1 -> 2
04DC0: 1 -> 2
04DC1: 1 -> 2
04DC2: 1 -> 2
04DC3: 1 -> 2
04DC4: 1 -> 2
04DC5: 1 -> 2
04DC6: 1 -> 2
04DC7: 1 -> 2
04DC8: 1 -> 2
04DC9: 1 -> 2
04DCA: 1 -> 2
04DCB: 1 -> 2
04DCC: 1 -> 2
04DCD: 1 -> 2
04DCE: 1 -> 2
04DCF: 1 -> 2
04DD0: 1 -> 2
04DD1: 1 -> 2
04DD2: 1 -> 2
04DD3: 1 -> 2
04DD4: 1 -> 2
04DD5: 1 -> 2
04DD6: 1 -> 2
04DD7: 1 -> 2
04DD8: 1 -> 2
04DD9: 1 -> 2
04DDA: 1 -> 2
04DDB: 1 -> 2
04DDC: 1 -> 2
04DDD: 1 -> 2
04DDE: 1 -> 2
04DDF: 1 -> 2
04DE0: 1 -> 2
04DE1: 1 -> 2
04DE2: 1 -> 2
04DE3: 1 -> 2
04DE4: 1 -> 2
04DE5: 1 -> 2
04DE6: 1 -> 2
04DE7: 1 -> 2
04DE8: 1 -> 2
04DE9: 1 -> 2
04DEA: 1 -> 2
04DEB: 1 -> 2
04DEC: 1 -> 2
04DED: 1 -> 2
04DEE: 1 -> 2
04DEF: 1 -> 2
04DF0: 1 -> 2
04DF1: 1 -> 2
04DF2: 1 -> 2
04DF3: 1 -> 2
04DF4: 1 -> 2
04DF5: 1 -> 2
04DF6: 1 -> 2
04DF7: 1 -> 2
04DF8: 1 -> 2
04DF9: 1 -> 2
04DFA: 1 -> 2
04DFB: 1 -> 2
04DFC: 1 -> 2
04DFD: 1 -> 2
04DFE: 1 -> 2
04DFF: 1 -> 2
0A953: 1 -> 0
0A9C0: 1 -> 0
10D69: 1 -> 0
10D6A: 1 -> 0
10D6B: 1 -> 0
10D6C: 1 -> 0
10D6D: 1 -> 0
10EFA: 1 -> 0
10EFB: 1 -> 0
10EFC: 1 -> 0
111C0: 1 -> 0
11235: 1 -> 0
1134D: 1 -> 0
113B8: 1 -> 0
113BB: 1 -> 0
113BC: 1 -> 0
113BD: 1 -> 0
113BE: 1 -> 0
113BF: 1 -> 0
113C0: 1 -> 0
113C2: 1 -> 0
113C5: 1 -> 0
113C7: 1 -> 0
113C8: 1 -> 0
113C9: 1 -> 0
113CE: 1 -> 0
113CF: 1 -> 0
113D0: 1 -> 0
113D1: 1 -> 0
113D2: 1 -> 0
113E1: 1 -> 0
113E2: 1 -> 0
116B6: 1 -> 0
1171E: 0 -> 1
1193D: 1 -> 0
11A3A: 0 -> 1
11B60: 1 -> 0
11B62: 1 -> 0
11B63: 1 -> 0
11B64: 1 -> 0
11B66: 1 -> 0
11F41: 1 -> 0
11F5A: 1 -> 0
1611E: 1 -> 0
1611F: 1 -> 0
16120: 1 -> 0
16121: 1 -> 0
16122: 1 -> 0
16123: 1 -> 0
16124: 1 -> 0
16125: 1 -> 0
16126: 1 -> 0
16127: 1 -> 0
16128: 1 -> 0
16129: 1 -> 0
1612D: 1 -> 0
1612E: 1 -> 0
1612F: 1 -> 0
16FF0: 2 -> 0
16FF1: 2 -> 0
16FF2: 1 -> 2
16FF3: 1 -> 2
16FF4: 1 -> 2
16FF5: 1 -> 2
16FF6: 1 -> 2
187F8: 1 -> 2
187F9: 1 -> 2
187FA: 1 -> 2
187FB: 1 -> 2
187FC: 1 -> 2
187FD: 1 -> 2
187FE: 1 -> 2
187FF: 1 -> 2
18CFF: 1 -> 2
18D09: 1 -> 2
18D0A: 1 -> 2
18D0B: 1 -> 2
18D0C: 1 -> 2
18D0D: 1 -> 2
18D0E: 1 -> 2
18D0F: 1 -> 2
18D10: 1 -> 2
18D11: 1 -> 2
18D12: 1 -> 2
18D13: 1 -> 2
18D14: 1 -> 2
18D15: 1 -> 2
18D16: 1 -> 2
18D17: 1 -> 2
18D18: 1 -> 2
18D19: 1 -> 2
18D1A: 1 -> 2
18D1B: 1 -> 2
18D1C: 1 -> 2
18D1D: 1 -> 2
18D1E: 1 -> 2
18D80: 1 -> 2
18D81: 1 -> 2
18D82: 1 -> 2
18D83: 1 -> 2
18D84: 1 -> 2
18D85: 1 -> 2
18D86: 1 -> 2
18D87: 1 -> 2
18D88: 1 -> 2
18D89: 1 -> 2
18D8A: 1 -> 2
18D8B: 1 -> 2
18D8C: 1 -> 2
18D8D: 1 -> 2
18D8E: 1 -> 2
18D8F: 1 -> 2
18D90: 1 -> 2
18D91: 1 -> 2
18D92: 1 -> 2
18D93: 1 -> 2
18D94: 1 -> 2
18D95: 1 -> 2
18D96: 1 -> 2
18D97: 1 -> 2
18D98: 1 -> 2
18D99: 1 -> 2
18D9A: 1 -> 2
18D9B: 1 -> 2
18D9C: 1 -> 2
18D9D: 1 -> 2
18D9E: 1 -> 2
18D9F: 1 -> 2
18DA0: 1 -> 2
18DA1: 1 -> 2
18DA2: 1 -> 2
18DA3: 1 -> 2
18DA4: 1 -> 2
18DA5: 1 -> 2
18DA6: 1 -> 2
18DA7: 1 -> 2
18DA8: 1 -> 2
18DA9: 1 -> 2
18DAA: 1 -> 2
18DAB: 1 -> 2
18DAC: 1 -> 2
18DAD: 1 -> 2
18DAE: 1 -> 2
18DAF: 1 -> 2
18DB0: 1 -> 2
18DB1: 1 -> 2
18DB2: 1 -> 2
18DB3: 1 -> 2
18DB4: 1 -> 2
18DB5: 1 -> 2
18DB6: 1 -> 2
18DB7: 1 -> 2
18DB8: 1 -> 2
18DB9: 1 -> 2
18DBA: 1 -> 2
18DBB: 1 -> 2
18DBC: 1 -> 2
18DBD: 1 -> 2
18DBE: 1 -> 2
18DBF: 1 -> 2
18DC0: 1 -> 2
18DC1: 1 -> 2
18DC2: 1 -> 2
18DC3: 1 -> 2
18DC4: 1 -> 2
18DC5: 1 -> 2
18DC6: 1 -> 2
18DC7: 1 -> 2
18DC8: 1 -> 2
18DC9: 1 -> 2
18DCA: 1 -> 2
18DCB: 1 -> 2
18DCC: 1 -> 2
18DCD: 1 -> 2
18DCE: 1 -> 2
18DCF: 1 -> 2
18DD0: 1 -> 2
18DD1: 1 -> 2
18DD2: 1 -> 2
18DD3: 1 -> 2
18DD4: 1 -> 2
18DD5: 1 -> 2
18DD6: 1 -> 2
18DD7: 1 -> 2
18DD8: 1 -> 2
18DD9: 1 -> 2
18DDA: 1 -> 2
18DDB: 1 -> 2
18DDC: 1 -> 2
18DDD: 1 -> 2
18DDE: 1 -> 2
18DDF: 1 -> 2
18DE0: 1 -> 2
18DE1: 1 -> 2
18DE2: 1 -> 2
18DE3: 1 -> 2
18DE4: 1 -> 2
18DE5: 1 -> 2
18DE6: 1 -> 2
18DE7: 1 -> 2
18DE8: 1 -> 2
18DE9: 1 -> 2
18DEA: 1 -> 2
18DEB: 1 -> 2
18DEC: 1 -> 2
18DED: 1 -> 2
18DEE: 1 -> 2
18DEF: 1 -> 2
18DF0: 1 -> 2
18DF1: 1 -> 2
18DF2: 1 -> 2
1D166: 1 -> 0
1D16D: 1 -> 0
1D300: 1 -> 2
1D301: 1 -> 2
1D302: 1 -> 2
1D303: 1 -> 2
1D304: 1 -> 2
1D305: 1 -> 2
1D306: 1 -> 2
1D307: 1 -> 2
1D308: 1 -> 2
1D309: 1 -> 2
1D30A: 1 -> 2
1D30B: 1 -> 2
1D30C: 1 -> 2
1D30D: 1 -> 2
1D30E: 1 -> 2
1D30F: 1 -> 2
1D310: 1 -> 2
1D311: 1 -> 2
1D312: 1 -> 2
1D313: 1 -> 2
1D314: 1 -> 2
1D315: 1 -> 2
1D316: 1 -> 2
1D317: 1 -> 2
1D318: 1 -> 2
1D319: 1 -> 2
1D31A: 1 -> 2
1D31B: 1 -> 2
1D31C: 1 -> 2
1D31D: 1 -> 2
1D31E: 1 -> 2
1D31F: 1 -> 2
1D320: 1 -> 2
1D321: 1 -> 2
1D322: 1 -> 2
1D323: 1 -> 2
1D324: 1 -> 2
1D325: 1 -> 2
1D326: 1 -> 2
1D327: 1 -> 2
1D328: 1 -> 2
1D329: 1 -> 2
1D32A: 1 -> 2
1D32B: 1 -> 2
1D32C: 1 -> 2
1D32D: 1 -> 2
1D32E: 1 -> 2
1D32F: 1 -> 2
1D330: 1 -> 2
1D331: 1 -> 2
1D332: 1 -> 2
1D333: 1 -> 2
1D334: 1 -> 2
1D335: 1 -> 2
1D336: 1 -> 2
1D337: 1 -> 2
1D338: 1 -> 2
1D339: 1 -> 2
1D33A: 1 -> 2
1D33B: 1 -> 2
1D33C: 1 -> 2
1D33D: 1 -> 2
1D33E: 1 -> 2
1D33F: 1 -> 2
1D340: 1 -> 2
1D341: 1 -> 2
1D342: 1 -> 2
1D343: 1 -> 2
1D344: 1 -> 2
1D345: 1 -> 2
1D346: 1 -> 2
1D347: 1 -> 2
1D348: 1 -> 2
1D349: 1 -> 2
1D34A: 1 -> 2
1D34B: 1 -> 2
1D34C: 1 -> 2
1D34D: 1 -> 2
1D34E: 1 -> 2
1D34F: 1 -> 2
1D350: 1 -> 2
1D351: 1 -> 2
1D352: 1 -> 2
1D353: 1 -> 2
1D354: 1 -> 2
1D355: 1 -> 2
1D356: 1 -> 2
1D360: 1 -> 2
1D361: 1 -> 2
1D362: 1 -> 2
1D363: 1 -> 2
1D364: 1 -> 2
1D365: 1 -> 2
1D366: 1 -> 2
1D367: 1 -> 2
1D368: 1 -> 2
1D369: 1 -> 2
1D36A: 1 -> 2
1D36B: 1 -> 2
1D36C: 1 -> 2
1D36D: 1 -> 2
1D36E: 1 -> 2
1D36F: 1 -> 2
1D370: 1 -> 2
1D371: 1 -> 2
1D372: 1 -> 2
1D373: 1 -> 2
1D374: 1 -> 2
1D375: 1 -> 2
1D376: 1 -> 2
1E5EE: 1 -> 0
1E5EF: 1 -> 0
1E6E3: 1 -> 0
1E6E6: 1 -> 0
1E6EE: 1 -> 0
1E6EF: 1 -> 0
1E6F5: 1 -> 0
1F6D8: 1 -> 2
1FA89: 1 -> 2
1FA8A: 1 -> 2
1FA8E: 1 -> 2
1FA8F: 1 -> 2
1FABE: 1 -> 2
1FAC6: 1 -> 2
1FAC8: 1 -> 2
1FACD: 1 -> 2
1FADC: 1 -> 2
1FADF: 1 -> 2
1FAE9: 1 -> 2
1FAEA: 1 -> 2
1FAEF: 1 -> 2
