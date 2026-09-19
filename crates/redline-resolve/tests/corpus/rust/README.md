# Rust golden corpus (plan 011, issue 08)

A small, genuine cargo workspace that `tests/golden_rust.rs` copies to a
tempdir and probes through the REAL `CargoProvider` (live `cargo metadata`
subprocesses, isolated `CARGO_HOME`, no network, no PTY).

Layout (probe workspace root defaults to `project/`):

- `project/Cargo.toml` — workspace root: members `gearbox` and `widgets`.
- `project/gearbox/` — a bin+lib member: module tree (`core/`, `engine/`),
  `use … as` alias (`DriveGear`), prelude bare names (`String`, `Vec`),
  turbofish (`HashMap::<u32, f64>::new()`), generic args
  (`piston::describe::<f64>(…)`), deep `::` chains
  (`gearbox::core::math::sum_sq`, `widgets::gear::Gear::new`).
- `project/widgets/` — a lib member: module tree (`gear`, `spindle`),
  structs, impl blocks.
- `frobnicator/` — a vendored crate kept OUTSIDE the workspace root
  (a sibling of `project/`, reached via a plain path dependency) so probes
  landing there exercise `external = true` without any registry fetch.
- `not_a_crate/` — a directory without `Cargo.toml`, probe target for the
  no-cargo-project bail.
- `*.golden` — one file per probe: the probe spec (symbol, from_file,
  scope hint, workspace root) AND the expected outcome pinned
  byte-for-byte — either a `ResolvedSource` (file/source_root relative to
  this corpus root, line, external) or the exact bail message with
  `{root}` / `{corpus}` / `{home}` placeholders for the dynamic absolute
  paths.

Probe 003/016 pin an observation, not a promise: path-shaped DEEP chains
(`a::b::c`) use the historical second-segment rule, so they land on the
second segment (the module declaration), while the bare+scope-hint twins
(004, 006, 017) land on the leaf item.

## Suite coverage note (011-08 review P2-2)

The golden suite runs `cargo metadata` only — it does NOT type-check the
sample sources. A corpus edit that breaks compilation passes the gate
silently; keep the samples compiling (out-of-band `cargo check` on a
scratch copy when you edit them).
