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
- fix-jsrel review P2-7 (RESOLVED, `e2281a2`): the relative probes' scope
  hints are now EMITTED by the app — `js_ts_specifier` accepts `./`/`../`
  specifiers (every binding shape the provider's relative branch lands:
  named / aliased / default / destructured carry the item, whole-module
  and namespace carry the specifier alone), so M-. on a relative use
  site lands in the sibling file (`external = false`, editable project
  buffer — live leg `xref_relative_import_lands_in_sibling_file_editable`;
  hint shapes in `resolver_scope_js_relative_import_carries_sibling_path`).
  The honest negatives stay negative: absolute / bare-`.` / side-effect
  specs still never hint (`resolver_scope_js_no_import_absolute_and_
  side_effect_are_not_guessed`). The goldens keep pinning the provider's
  landed behavior byte-for-byte, now fed by the app's own hint.
