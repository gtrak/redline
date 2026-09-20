# Task: plan 010 Rung 4 (find-implementations) + app-side whole-path enumeration for C/Cpp/Toml/Json

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `.agents/plans/010-bundled-static-analysis/PLAN.md` (Shape A, Rung 4)
and the landed Rung 1 machinery (`.agents/tasks/issue-010-01-impl.md`,
merged `53f0965` + review fixes).

## Item 1 — Rung 4: find-implementations (read-only view)

The seam is stored: `ImplMethod.impl_line` + `ImplKind::Trait` (the
Rung 1 tables, `src/nav/index.rs`). Build the read-only view:
- A command (M-x palette entry + a keybind IF a natural one exists —
  judge; emacs has no standard find-implementations key) that takes the
  trait/type at point (reuse the M-. extraction) and opens a PICKER of
  impl candidates (file + line, kind: inherent/trait name).
- Data access: the Rung 1 reviewer judged a full scan of every file's
  tables OR a new name-keyed trait map. Decide honestly: a trait-keyed
  map built in the same index pass (the Rung 1 discipline: zero extra
  parse, same tree) is likely right — same-content-refresh discipline
  applies (same-check BEFORE removal, the 010-01 P1 pattern, pinned).
- Degrades: trait not at point / no table entry / generic trait → the
  existing bare-symbol lookup (byte-for-byte).

## Item 2 — the app-side whole-path upgrade for C/Cpp/Toml/Json (Known corner)

`dotted_path_container` (store.rs:9849-9861) enumerates only the
pre-lane container kinds (member_expression, attribute,
selector_expression, qualified_type). The lang-pred lane landed the
SYNTAX layer for C (`field_expression`), Cpp (`field_expression` +
`qualified_identifier`), Toml (`dotted_key`) — but the app-side upgrade
never fires for them, so M-. stays bare. Extend the container
enumeration honestly:
- C/Cpp: `field_expression` (note Cpp `::` is already covered by the
  byte-scan — do not double-handle).
- Toml/Json: dotted KEYS — judge whether key-path M-. is useful enough
  vs the index fall-through (a toml dotted key at M-. would look up the
  outline; probably yes for table headers). State the decision.
- Rust self.* stays special-cased (Rung 1) — untouched.
- Pin the new containers per language (M-. on `o.x` in C lands via the
  index fall-through with the whole path; degradation byte-for-byte
  where the container text doesn't match an indexed symbol).

## Constraints

- Gate: `cargo test --workspace` + clippy (read exit) + `tools/gate.sh
  full` (flock, `cargo build` first). Budget ~50 tool calls; honest-stop
  at half.
- Scope fence: `src/app/store.rs`, `src/app/command.rs` + `keymap.rs`
  (the Rung 4 command + binding), `src/nav/index.rs` (trait map),
  `docs/provider-matrix.md` (both features' rows).
  NO queries.rs/node.rs changes (a parallel lane owns the syntax layer);
  NO provider changes.
