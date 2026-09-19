# JS/TS golden corpus (011-08, js lane)

18+1 probes against the JS provider: 12 deterministic offline probes (zero
shell-outs, byte-stable), 1 online-deterministic, and 6 live probes (real
`npm install` of left-pad / dot-prop / pascal-case in a per-run tempdir;
registry goldens flip loudly on caret-range drift — review the diff and
re-bless deliberately).

`GOLDEN_BLESS=1 cargo test -p redline-resolve --test golden_js` re-captures
ALL goldens then FAILS the run (never ends green — an accidental bless
cannot slip through); rerun without the env var to verify.

## Post-011-08 history

- fix-jsrel (`0905a18`): relative specifiers LAND against `from_file`'s
  directory (a real go-to-definition feature); absolute / file-ish names
  get dedicated bails; `node_modules/.` is refused at both layers; the
  online funnel into `npm install "."` is closed by construction (review
  P2-4: the absent-node_modules case is closed by construction, pinned
  offline by `relative-import-whole-bail`).
- fix-jsrel review P2-7: the relative probes carry a scope hint the app
  does not yet emit (the app never hints relative specifiers — pinned by
  `resolver_scope_js_no_import_and_relative_are_not_guessed`); the goldens
  are a contract pin for the follow-up that makes the app emit them.
