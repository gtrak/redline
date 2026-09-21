# Task: consolidate the golden-suite test helpers (T1)

## Why
`crates/redline-resolve/tests/golden_{go,js,python,rust}.rs` each re-declare the same
~10 helpers (~200+ duplicated lines). `assert_bless_stopped` is **byte-identical** across
go/python/rust (md5-verified by the audit); `normalize` is the same algorithm
parameterized by which roots get placeholder-substituted; `check_golden`/`summarize`/
`render_golden`/`copy_tree`/`parse_golden`/`sub_path`/`rel_to*` are near-identical.

## Target
One shared module: `crates/redline-resolve/tests/common/mod.rs` (the standard Rust
integration-test sharing pattern — each `golden_*.rs` declares `mod common;` and uses
`common::…`). Move the genuinely shared helpers there:
`parse_golden`, `sub_path`, `normalize` (parameterized by the placeholder roots, since
go/python/rust substitute different sets), `rel_to`/`rel_to_corpus`, `render_golden`,
`summarize`, `check_golden`, `copy_tree`, `assert_bless_stopped`, and the `bless_*`
helpers that are actually shared.
**Keep per-file what is genuinely per-language**: each `golden_*.rs` keeps its corpus,
its golden files, its `Probe`/`Outcome` shims if the types differ, and its test fns.

## Hard rules
- **No assertion may be weakened, deleted, or reworded.** This is a helper extraction;
  the per-language tests must keep asserting exactly what they assert today.
- **Bless behaviour must not change**: the bless path writes all goldens then FAILs
  (that is deliberate). Keep it exactly.
- If two helpers look similar but differ (e.g. js's `assert_bless_stopped` is a
  whitespace/atomic-path variant), either parameterize honestly or leave the variant in
  place with a comment saying why it differs. **Do not** collapse a real difference.
- Fence: `crates/redline-resolve/tests/*`. Nothing else.
- Honest-stop at half budget; a partial consolidation that is green is a valid landing.

## Gate
`cargo build`, `cargo test --workspace` (the four golden suites must run and pass —
state their per-suite counts before and after), `cargo clippy --workspace --all-targets`
(read `${PIPESTATUS[0]}`), `timeout 900 tools/gate.sh full`.
**Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` **and** swap before a
battery; if memory is tight, run the golden suites only and report the battery as
deferred (do not sleep-wait).
Budget ~35 tool calls. Report: the shared module's contents, what stayed per-file and
why, per-suite test counts before/after, gate counts.
