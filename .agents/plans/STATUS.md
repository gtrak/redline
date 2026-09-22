# STATUS — the work tracker (what is open, what is landed, what is unknown)

Single authoritative status table for every spec in `.agents/tasks/*.md` and
every issue in `.agents/plans/*/PLAN.md` (active plans; the archived plans keep their full issue texts in `.agents/plans/archive/*.md`). When a lane lands, its state here
must change in the same commit as the code (`.agents/skills/plan-process/
SKILL.md` § "Landing a lane includes updating the tracker").

**The evidence rule:** every state carries checkable evidence — a
`file:line`, a symbol, a test name, or a commit hash. A state inferred from
a spec's existence, its date, or the wording of a worklist is not a state.
When the evidence cannot be found, the row is **UNKNOWN** — an honest
UNKNOWN is a success, a confident wrong LANDED is how work gets dropped.

States, used strictly:

- **LANDED** — implemented and on main.
- **IN FLIGHT** — specced, and a lane is actively working it right now
  (branch/worktree named). The original failure this file prevents was an
  OPEN row nobody was working; IN FLIGHT is the state that makes that
  distinguishable from a forgotten one.
- **OPEN** — specced, not implemented.
- **SUPERSEDED** — replaced by another spec (successor named).
- **BLOCKED** — cannot start (blocker named: a decision, or another lane's file).
- **UNKNOWN** — could not be determined from evidence.

Audited at main `dba6c82` (tracker-repair lane, 2026-09-21); re-verified
against main `1a82f02` after the tracker gate pass (row states, the
`fn_survey`-reproducible numbers, and the stale citations below were all
re-checked at `1a82f02`).

## The short answer (open + in-flight work)

| Spec | State | Why it is open |
|---|---|---|
| `015-02` honest point + edit modes | OPEN | the keystone: `current_point_byte` still returns the line start |
| `015-03` accurate editing | OPEN | depends on 015-02 |
| `015-04` yank semantics | OPEN | depends on 015-02 |
| `016-undo` (plan, design only) | OPEN | no undo anywhere in the codebase |
| `013-01` iocraft upstream PR | OPEN | user decision on plan 013 not made |
| `013-02` vendored patch | OPEN | contingent on that decision |
| `013-03` pre-frame harness | OPEN | neither deliverable is present — see the 013 section |
| `014-02` extract `redline-git` | OPEN | staged, not done |
| `014-03` extract `redline-model` | OPEN | not specced; gated on the `sections.rs` decision |
| `010-04` in-process RA (Shape B) | OPEN (contingent) | "only if user picks Shape B" — not scheduled; the row exists so a contingent spec cannot go invisible |
| `issue-guardrails` | OPEN | no `SAFETY:` comments, no `[workspace.lints]`, no `.githooks` |
| `issue-hygiene-sweep` | OPEN | all 4 dead `pub` constructors still present; 20 files carry `allow(dead_code)` |
| `issue-git-test-harness` | OPEN | the second `fn git()` cluster — **5** copies: blame/commit/log/refs/repo-tests — still hand-copied |
| `issue-provider-ident-char` | OPEN | 3 separate `is_ident_char` definitions remain |
| `issue-dependency-dir-guard` | OPEN | no detection/disclosure of non-ignored dependency dirs |
| `issue-ignore-single-engine` | OPEN | two hand-rolled ignore decisions still exist |
| `issue-input-latency` | OPEN (parked) | landed once, reverted on user request (`0ddaf46`) — re-land is a user call |
| `issue-jump-highlight` | IN FLIGHT | worktree slot 2, branch `jump-highlight` — a lane is working it (uncommitted edits to `src/theme.rs`, `src/ui/file_view.rs`, …; nothing committed yet). **The missed user-requested feature** — the spec that sat unworked for a session while looking current; now held by a lane |
| `issue-deflake-timing` | BLOCKED | root-cause fix is plan 013's iocraft decision; items 2–3 already landed |

## Plan 011 — resolver parity (archived; all 8 issues landed)

| Spec | State | Evidence | Notes |
|---|---|---|---|
| 011-01 language dispatch + provider registration (`issue-011-01-impl`) | LANDED | `resolve`/provider chain (`crates/redline-resolve/src/lib.rs:275`) | spot-checked |
| 011-02 per-language scope hints (`issue-011-02-impl`) | LANDED | `scope_qualified` use (`crates/redline-resolve/src/cargo.rs:155`) | spot-checked |
| 011-03 node-at-point per language (`issue-011-03-impl`) | LANDED | `crates/redline-syntax/src/node/{mod,paths,scopes}.rs` | — |
| 011-04 per-language source index (`issue-011-04-impl`) | LANDED | `source_extensions_for` (`src/app/store/index_wiring.rs:526`) | — |
| 011-05 resolver parity harness + matrix (`issue-011-05-impl`) | LANDED | `docs/provider-matrix.md` | spot-checked |
| 011-06 path-token extraction (`issue-011-06-impl`) | LANDED | `dotted_path_container` call site (`src/app/store/navigation/xref.rs:355`) | — |
| 011-07 walk sets (`issue-011-07-impl`) | LANDED | `source_extensions_for` (`src/app/store/index_wiring.rs:526`) + `docs/provider-matrix.md` C/C++/Markdown columns | — |
| 011-08 golden-corpus suites (`issue-011-08-impl`) | LANDED | `crates/redline-resolve/tests/golden_{go,js,python,rust}.rs` | spot-checked |

## Plan 013 — the hardware-cursor race (active; decision open with the user)

| Spec | State | Evidence | Notes |
|---|---|---|---|
| 013-01 upstream PR | OPEN | the race still exists in code: `sleep(12ms)` + detached `MoveTo` (`src/ui/root/hooks.rs:269`); no PR filed (nothing in-repo to evidence one; plan 013 §5 decision unmade) | recommended option; needs the user's go |
| 013-02 vendored patch | OPEN | iocraft still lock-pinned to 0.9.1 unmodified (`Cargo.toml`); no `[patch]` table | contingent on the user choosing B |
| 013-03 pre-frame harness sub-class | OPEN | **neither deliverable is present.** The pre-frame legs the suite carries (`?25l` counted / no-lingering asserts, `tools/check_cursor_stream.py:8–12,249`) came from plan 004 issue 05 (commit `c964a07`) — *before plan 013 existed* — so they are not evidence this lane ran; and the spec's second deliverable (record the outcome/decision in the 012 worklist § Gate reliability class 1 — now `archive/012-project-organization.md` § `00-worklist`) was never written. The evidence points to not-run | `cup_settle` (`check_cursor_stream.py:135`) covers a starved CUP, which the spec explicitly does NOT conflate with the pre-frame class |

## Plan 014 — crate extraction (active; stage 1 landed)

| Spec | State | Evidence | Notes |
|---|---|---|---|
| 014-01 extract `redline-syntax` | LANDED | commit `56efe19`; `crates/redline-syntax/` (154-test split of the 867, verified in the plan's success criteria) | stage 1 |
| 014-02 extract `redline-git` | OPEN | `crates/` holds only `redline-syntax` + `redline-resolve` (no `redline-git`, no `redline-testutil`) | staged after 01; includes promoting the git test harness |
| 014-03 extract `redline-model` | OPEN | not specced (PLAN.md §6: "write the issue when the decision is made"); blocked on the `model/sections.rs` view-model decision + 02 | — |

## Plan 015 — editing modes (active; issue 01 landed, the keystone is open)

| Spec | State | Evidence | Notes |
|---|---|---|---|
| 015-01 annotations picker (`issue-015-01-annotations-picker`) | LANDED | `annotations_candidates` (`src/app/store/picker.rs:112`), `open_picker(PickerKind::Annotations, …)` (`:158`) | **silently landed** — the PLAN.md status said "Nothing implemented yet" until this repair |
| 015-02 honest point + modes (`issue-015-02-point-and-modes`) | OPEN | `current_point_byte` still returns the line start — `try_line_to_byte` with no column (`src/app/store/buffers.rs:362–367`); no edit-mode type anywhere in `src/app/store/` | the keystone; blocks 03/04 |
| 015-03 accurate editing (`issue-015-03-accurate-editing`) | OPEN | insert still appends at buffer end (`notes_edit_key_event` — printable appends, Backspace deletes; RET unhandled); depends on 02 | — |
| 015-04 yank semantics (`issue-015-04-yank-semantics`) | OPEN | yank still inserts at the line start (same `current_point_byte` lie); depends on 02 | — |

## Plan 016 — undo (active; design only)

| Spec | State | Evidence | Notes |
|---|---|---|---|
| `016-01` undo-stack · `016-02` edit-coverage · `016-03` dirty-flag-and-reload · `016-04` redo-and-coalescing (the plan's task-order rows; not yet specced as files) | OPEN | no undo: zero hits for `fn undo` in `src/` + `crates/`; no `C-x u`/`C-/` binding in the keymap tables (`src/app/store/mod.rs`) | design settled (inverse-edit stack; BOTH `C-x u` and `C-/` per user directive); pick-up = spec the four issues |

## Standalone task specs (not owned by a numbered plan issue)

| Spec | State | Evidence | Notes |
|---|---|---|---|
| `issue-coverage-tracker` | LANDED | `docs/language-coverage.md` exists (the language × capability grid, 34,745 B at `1a82f02`); commit `7433956` | `coverage-rows` + `coverage-fixes` (below) are the lanes that kept it current |
| `issue-coverage-rows` | LANDED | commit `ece5ab9`; the C/Cpp/Toml/Json cells carry the new test pins (e.g. `c_member_path_comes_back_whole` cited in the C row of the doc) | docs |
| `issue-coverage-fixes` | LANDED | commit `546a199`; the doc's 6th cell value "not applicable" (`docs/language-coverage.md:43`) and the corrected TS tier (`:96`) | 1 P1 + 6 P2 review fixes, all doc-accuracy |
| `issue-deflake-timing` | BLOCKED | Item 1 (root fix: cursor write inside the sync region) is **verified impossible in-fence** on iocraft 0.9.1 (worklist § Gate reliability class 1; plan 013 §2.3/§5) — the fix is an iocraft-side decision the user has not made; the sleep is still there (`src/ui/root/hooks.rs:269`). Items 2+3 ARE landed: `cup_settle` (`tools/check_cursor_stream.py:135`, commit `b47e03c`) and the no-retry/swap policy (`docs/ux-testing-plan.md:465`) | blocker = plan 013's user decision (upstream PR vs vendored patch vs accept) |
| `issue-dependency-dir-guard` | OPEN | no detection/disclosure: zero production references to the guard dir list (`node_modules`/`vendor`/`Pods`…) in `src/model/files.rs` or `src/nav/index/`; `FileList::build` (`src/model/files.rs:65`) has no guard | specced 2026-09-21 (commit `7adacd5`) after the 56-second startup. **Cross-reference for the implementer**: `under_node_modules` (`src/app/store/index_wiring.rs:619`, used at `:603`, from 011-04 item 4) already prunes `node_modules` from the *external-dependency* index walk — this spec is about the *project* walk; do not re-implement that prune |
| `issue-doc-refresh` | LANDED | commit `9711ee9` (skill + 3 queued P2s) | — |
| `issue-doc-refresh2` | LANDED | commit `c85a093` (six stale doc/comment sites) | — |
| `issue-external-change-reload` | OPEN | `project.rs:33-48` re-opens an externally-changed file via `insert_rope(..., false)`, justified by a comment claiming "Issue-03 buffers are read-only, so this is safe" — false for edit-mode buffers: it REPLACES the Buffer, resetting the mode to `Annotation` and dropping unsaved in-memory edits | specced 2026-09-21 from the 015-02 gate's P2; a latent DATA-LOSS path (pre-existing), and the policy chosen here is what plan 016's dirty-flag/reload contract must build on |
| `issue-final-polish` | LANDED | python `src_layout_root` canonicalize (`crates/redline-resolve/src/providers/python_provider.rs:378–384`); control-char degrade (`:274`); field-level `#[expect(dead_code)]` on `golden_go.rs` `Probe.bail` (`crates/redline-resolve/tests/golden_go.rs:61–63`) | all six items |
| `issue-fixture-isolation` | LANDED | commit `62357cb`; `REDLINE_FIXTURE_ROOT=${REDLINE_FIXTURE_ROOT:-/tmp/fx$$}` (`tools/gate.sh:79–80`) | — |
| `issue-grammar-bumps` | LANDED | the 9 pin moves are in `Cargo.toml` (e.g. `tree-sitter-c-sharp =0.23.5`); `docs/tree-sitter-runtime-matrix.md:89` — "RESOLVED by the grammar-bumps lane"; commit `2508ec3` | **the task file still reads "the queued per-grammar drift decision"** — do not re-spec it from the file; the residual list of deliberately-not-bumped grammars is recorded at `tree-sitter-runtime-matrix.md:178` |
| `issue-git-test-harness` | OPEN | the second cluster is still hand-copied — **5** `fn git(dir, args)` copies: `src/git/blame.rs:81`, `src/git/commit.rs:68`, `src/git/log.rs:173`, `src/git/refs.rs:111`, `src/git/repo/tests.rs:14`; no `test_support.rs`/`redline-testutil` anywhere | the store-side dedup (`a1c2e47`) was fenced to `store/tests/*` |
| `issue-guardrails` | OPEN | 6 production `unsafe` in `src/main.rs` (lines 148, 174, 188, 192, 199, 279) with **zero** `SAFETY:` comments; no `[workspace.lints]`/`[lints]` table in any `Cargo.toml`; no `.githooks/`, no `core.hooksPath` | — |
| `issue-hint-relative` | LANDED | `resolver_scope_js_relative_import_carries_sibling_path` (`src/app/store/tests/navigation/imports.rs:143`) | 011-08 js review P2-7 contract pin |
| `issue-hygiene-sweep` | OPEN | all 4 dead `pub` constructors still present: `with_cargo_bin` (`crates/redline-resolve/src/cargo.rs:70`), `with_go_bin` (`providers/go_provider.rs:70`), `with_npm_bin` (`providers/js_provider.rs:98`), `with_providers` (`lib.rs:250`); `allow(dead_code)` in 20 files; the `file()` twins still at `src/model/files.rs:440` + `src/model/project.rs:214` | — |
| `issue-ignore-predicate` | LANDED | `is_gitignored` shared predicate (`src/model/files.rs:232`); `.git/info/exclude` + global excludes (`:376–396`); agreement tests (`walk_and_filter_agree_on_dot_ignore_files`, `:845`); commit `ca4e743` | successor of `issue-index-gitignore` |
| `issue-ignore-predicate-2` | LANDED | P2a: the global matcher now gets the **absolute** path — "the same input the walk's matcher sees, P2a" (`src/model/files.rs:388–394`); F5 dependency-walk exception (`src/app/store/index_wiring.rs:557–603`) | commit `2503740` era + F5 follow-up |
| `issue-ignore-single-engine` | OPEN | two hand-rolled decisions still exist: `is_gitignored` (`src/model/files.rs:232`) + `git_ignored_by_ancestors` (`src/search/rg.rs:435`, own memo); the `require_git(false)` single-engine shape from the spec is not there | they now share the `load_gitignore`/`gitignore_matches` helpers but the engine decision is still duplicated |
| `issue-index-file-budget` | LANDED | `FILE_BUDGET_BASE`/`FILE_BUDGET_PER_KB`/`default_file_budget` + `FileDeadline` in `crates/redline-syntax/src/queries.rs:505–523`; commit `d15b397` (doc corrections in `1a82f02`) | was OPEN at the `dba6c82` audit; the budget lane landed on main afterwards — updated at the gate pass |
| `issue-index-gitignore` | SUPERSEDED | replaced by `issue-ignore-predicate` (whose header: "Follow-up to `ca4e743` (the incremental index path now respects `.gitignore`)") — the core fix landed, the follow-up work was re-specced there | successor: `issue-ignore-predicate` |
| `issue-index-profiler` | LANDED | `src/index_profile.rs`; `--index-profile[=PATH]` flag (`src/main.rs:42–52`) | headless fine-grained index profiler |
| `issue-input-latency` | OPEN (parked) | landed (`1f0b4ef` + `b80cf86`), then **reverted on user request** (`0ddaf46` "Revert … may re-land"); no motion-key coalescing in `src/app/store/keys.rs` today; worklist row: "parked, recipe in the commit" | re-land is a user call, not an open lane |
| `issue-js-polish` | LANDED | `local_path_dep_bails_on_malformed_intermediate_manifest` (`crates/redline-resolve/src/providers/js_provider.rs:1021`) | matrix refresh done with it |
| `issue-json-yaml-no-symbols` | IN FLIGHT | worktree slot 1, branch `json-yaml-no-symbols` (commit `42cd743`) — a lane is working it; no row existed until now because the spec was written AFTER this table was built | user directive: JSON/YAML keys are no longer symbols (90% of a real project's 252,985 symbols were JSON+YAML keys). Verified safe for resolution: the resolver reads `package.json` from disk (`crates/redline-resolve/src/cargo.rs:402`, `providers/js_provider.rs:470`), not via symbols |
| `issue-jump-ambiguity` | LANDED | commit `6a80b41`; "jump silently iff one candidate AND in the current file" (`src/app/store/navigation/definitions.rs:129–160`); `M->` force-list (`src/app/store/mod.rs:322`) | — |
| `issue-jump-highlight` | IN FLIGHT | worktree slot 2, branch `jump-highlight` — a lane is working it (uncommitted edits to `src/theme.rs`, `src/ui/file_view.rs`, …; nothing committed yet). At the `dba6c82` audit there was no `jump_highlight` face in `src/theme.rs` and no transient highlight in `src/app/store/navigation/` | **the missed user-requested feature** — the spec that sat unworked for a session while looking current; its only hard dependency (the match-highlight overlay) landed, and it is now held by a lane |
| `issue-lang-predicates` | LANDED | `c_member_path_comes_back_whole` (`crates/redline-syntax/src/node/mod.rs:725`) + scope walks for C/Cpp/Markdown/Bash/Toml/Json; doc rows refreshed (commit `ece5ab9`) | — |
| `issue-loop-01-parallel-sweep` | LANDED | `gate.sh pooled` (`tools/gate.sh:19–20`) + `tools/pool.py`; stale-binary pin (`gate.sh:29–33`) | — |
| `issue-loop-02-fast-quiet` | LANDED | `PTY_QUIET` default 0.06 (`tools/pyte_driver.py:42`); commit `498de7f` (root causes + discrimination) | — |
| `issue-loop-03-demote-pyramid` | LANDED | 92 `unit_flow_*` twins in `src/app/flow_tests.rs`; e.g. `unit_flow_ext_crate_in_crate_mdot` (`:2955`) | — |
| `issue-loop-04-other-suites` | LANDED | commit `4cebd9e` ("all 8 suites demoted; pooled 82.7s") | — |
| `issue-provider-ident-char` | OPEN | three separate `is_ident_char` definitions remain: `crates/redline-resolve/src/cargo.rs:499`, `crates/redline-resolve/src/providers/go_provider.rs:624`, `crates/redline-resolve/src/providers/js_provider.rs:792` | the app-side rule was already unified (`is_word_char`, `src/model/buffer.rs:54`); the resolver providers each keep their own |
| `issue-py-roots` | LANDED | `src_layout_root` (`crates/redline-resolve/src/providers/python_provider.rs:370`), used at `:217`; test `src_layout_root_walks_up_for_pyproject` | python src/ layout honesty (011-08 python finding 2) |
| `issue-runtime-bump` | LANDED | `Cargo.toml:37 tree-sitter = "=0.25.10"`; commit `d912c7d` (Clojure landed on the new runtime) | — |
| `issue-samefile-jumpback` | LANDED | commit `c39b96b` ("M-, lands at the actual jump origin"); regression pin `jump_back_restores_recorded_column` (`src/app/store/tests/navigation/jump.rs:111`) | — |
| `issue-watchlist-fixes` | LANDED | all five U-K items checked in `docs/ux-testing-plan.md:445–460`: uppercase M-. (`xref_uppercase_type_and_const_shapes_jump_directly`, `src/app/store/tests/navigation/definitions.rs:11`), imenu impl grouping (`src/app/store/navigation/xref.rs:483`), search_jump keeps the view (`src/app/store/search.rs:400`), M-, sentinel pop (`src/app/store/navigation/mod.rs:161`), 2-line overlap (`src/app/store/file_view.rs:38`) | — |
| `issue-window-splits` | LANDED | per the spec's own honest-stop: C-x 0 (`src/app/store/mod.rs:360`), C-x 1 (`src/app/command.rs:406`), C-x 2 bound with the scoped follow-up recorded + DECIDED OUT (`src/app/command.rs:411–414`; U-K row, `docs/ux-testing-plan.md:484`) | real splits are a recorded non-goal, not open work |
| `issue-word-char` | LANDED | see the 012 table above (single `is_word_char`, `src/model/buffer.rs:54`) | listed here for completeness — it is the 012 lane |
| `issue-wt-convention` | LANDED | "Lane worktrees (numbered slots)" section (`.agents/skills/plan-process/SKILL.md:35`) | docs |
| `review-gate` | LANDED | standing reviewer protocol (no implementation target); in use — `VERDICT:` lines are quoted throughout the PLAN outcome records | protocol doc, not a work item |

## Open items that live OUTSIDE this tracker (report, not rows)

These are not specced in `.agents/tasks/` — they are watchlist notes in
`docs/ux-testing-plan.md` (U-K, unchecked) + the 012 worklist § Gate
reliability (the worklist is archived at
`archive/012-project-organization.md` § `00-worklist`). They are listed so the "anything else we forgot to work?"
question is answered from one place:

- U-K: `C-s`/`C-r` swallowed during isearch + picker latch; `n`/`N` cannot
  appear in an isearch query; EOFNL "(no newline)" cue not rendered;
  `unstage_hunk` on a fully staged-added file writes an empty blob;
  goto-line 0-based vs its 1-based error message; `Theme::name()` hardcoded;
  watcher late-publish race after stop/replace.
- the 012 worklist § Gate reliability (archived at `archive/012-project-organization.md` § `00-worklist`): the `git::repo` swap-exhaustion flake
  (policy recorded, no fix owed); the unattributed signal-kill flake class
  (signal-specific retry is a gate-lane follow-up); the PTY flake of
  unattributed identity (descriptor-table gate).
- 013: the user decision itself (upstream PR / vendored patch / accept).

## Archived plans 001–012 — full records in `.agents/plans/archive/`



The archive is a flat list of one `.md` per completed plan — the original
folder's full content (PLAN + issue files, verbatim), not a deletion of it.
The rows here come in two classes, and the class decides what a LANDED state
means:

**Verified at landing (004, 007, 010, 012 — archived 2026-09-21).** Every
issue in the full tables below was checked when its lane landed (gate +
review + the evidence citation in the row); the tables move here from the
active sections unchanged. "PLAN.md" citations in those tables mean the
corresponding archive file, which embeds the original `PLAN.md` verbatim.
These rows are NOT part of the sampled group below and carry no caveat.

> **CAVEAT — sampled, not audited (the grouped rows 001, 002, 003, 005,
> 006, 008, 011 only).** These rows are **inference, not audit**: ~39
> archived specs collapse into 7 group rows, and for 002 / 003 / 006 a
> single verified anchor stretches across specs that were never individually
> checked. The residual risk is one-directional and real: **an unlanded
> archived spec could hide inside a LANDED group.** If a behavior attributed
> to one of these groups is missing from the code, re-verify that spec
> individually before believing this table.

### Plan 004 — emacs parity (archived 2026-09-21; all 16 issues verified at landing)


| Spec | State | Evidence | Notes |
|---|---|---|---|
| 004-01 isearch interception (task `issue-004-01-impl`; issue text in `archive/004-emacs-parity.md`) | LANDED | `isearch_key_event` (`src/app/store/keys.rs:251` — defined; dispatched at `:62`), `register_isearch` (`src/app/command.rs:378`) | — |
| 004-02 parity adopts batch 1 (`issue-004-02-impl`) | LANDED | `docs/emacs-parity-log.md` adopted rows; PLAN.md outcome record | C-v 2-line overlap also pinned by `scroll_page_down_keeps_two_line_overlap` (watchlist lane) |
| 004-03 mark & kill/yank (`issue-004-03-impl`) | LANDED | `kill_ring` (`src/app/store/buffers.rs:493`), yank-pop (`:558`) | — |
| 004-04 quit save-prompt (`issue-004-04-impl`) | LANDED | commit `0c5dfc6` (PLAN.md outcome record); `quit_prompt_key` (`src/app/store/keys.rs:540`) | — |
| 004-05 cursor visibility (`issue-004-05-impl`) | LANDED | `install_cursor_effect` (`src/ui/root/mod.rs:92`); `tools/check_cursor_stream.py` 05/05d/05e/05g legs | hardware cursor + truecolor bar |
| 004-05b file-view point + motion (`issue-004-05b-cursor-point`, `issue-004-05b-flows`) | LANDED | commit `220208c` (PLAN.md); `point_line`/`point_col` (`src/app/store/file_view.rs:553,558`) | — |
| 004-05c mouse/recenter/word (`issue-004-05c-mouse-recenter-words`) | LANDED | commit `197967e`; `mouse_recenter_word_checks` in `tools/check_cursor_stream.py` | — |
| 004-05d wide-char columns (`issue-004-05d-wide-chars`) | LANDED | commit `fc505aa`; check_cursor_stream 05d leg | — |
| 004-05e tree click offset (`issue-004-05e-tree-click-offset`) | LANDED | commit `95f24b1`; check_cursor_stream 05e leg | — |
| 004-05f menu readability (`issue-004-05f-menu-readability`) | LANDED | commit `dbb1885` + `dba535c` (PLAN.md) | — |
| 004-05g tree cursor offset (`issue-004-05g-tree-cursor-offset`) | LANDED | commit `badaf9b`; check_cursor_stream 05g leg | — |
| 004-05h buffer-list keys (`issue-004-05h-buffer-list-keys`) | LANDED | commit `679c007`; `buffer_list_n_p_d_keys` test (flow_tests) | — |
| 004-06 discovery home (`issue-004-06-impl`) | LANDED | `src/ui/home_view.rs`; `ViewId::Home` in `src/app/store/mod.rs` | — |
| 004-06a empty table + HOME (`issue-004-06a-impl`) | LANDED | `BufferTable::new()` starts empty (`src/model/buffer.rs`); anti-drift test | — |
| 004-06b remove scratch (`06b-remove-scratch.md`) | LANDED | scratch no longer auto-created; `open-scratch` kept as the non-default entry the spec allowed (`src/app/command.rs:177`) | decision stated per spec |
| 004-07 recenter after jump (`issue-004-07-jump-recenter`) | LANDED | commit `ea532ac`; `recenter_landing` calls (`src/app/store/navigation/mod.rs:76,185`) | — |

### Plan 007 — syntax-aware linking (archived 2026-09-21; all 4 issues verified at landing)


| Spec | State | Evidence | Notes |
|---|---|---|---|
| 007-01 node-at-point + scope (`issue-007-01-impl`) | LANDED | `node_at` (`crates/redline-syntax/src/node/mod.rs:63`); commit `c94a315` | — |
| 007-02 syntax anchors (`issue-007-02-impl`) | LANDED | `SyntaxAnchor` (`src/app/store/notes_doc.rs`); commit `512f1eb`; `tools/drive_syntax_notes.py` | — |
| 007-03 scope-aware resolution (`issue-007-03-impl`) | LANDED | `SymbolContext.scope` (`crates/redline-resolve/src/lib.rs:142`); commit `c015366` | — |
| 007-04 incremental parse reuse (`issue-007-04-impl`) | LANDED | `rope_edit_to_input_edit` (`crates/redline-syntax/src/highlight.rs:497`); `retain_apply_edit` (`crates/redline-syntax/src/cache.rs:219`); commit `825edae` (on main) | **The PLAN.md status line ("04 unscheduled") was stale** — the outcome record and the code both show it merged; corrected in this repair |

### Plan 010 — bundled static analysis (archived 2026-09-21; Shape A verified at landing — Shape B / 010-04 stays OPEN-contingent)


| Spec | State | Evidence | Notes |
|---|---|---|---|
| Rung 1 impl/field tables (`010-01`, task `issue-010-01-impl`) | LANDED | `RustTables` in `src/nav/index/builder.rs:10`; commit `2111d03`+`53f0965` | — |
| Rung 2 use-path/mod-tree (`010-02`) | SUPERSEDED | subsumed by plan 011's scope hints + walks (PLAN.md outcome record) | not a dropped item — the work landed under 011 |
| Rung 3 local binding types (`010-03`, task `issue-010-03-impl`) | LANDED | `rust_binding_type_at` (`crates/redline-syntax/src/queries.rs:1065` at main `1a82f02`); commit `171deca` | — |
| Rung 4 find-implementations + whole-path enumeration (`issue-rung4-and-paths`) | LANDED | `find-implementations` (`src/app/command.rs:714`); `dotted_path_container` (`src/app/store/navigation/xref.rs:415`); commit `0126adc` | — |
| 010-04 in-process RA — Shape B (the task-order row) | OPEN (contingent) | not scheduled: PLAN.md task order reads `04 in-process RA (only if user picks Shape B)`; no work started | the contingent spec is on the table so the decision cannot go invisible |
| new languages Java/C#/Ruby/Scheme/Clojure (`issue-new-languages`) | LANDED | 19-language `LANGUAGES` table (`crates/redline-syntax/src/language.rs`); commits `d1789a5`…`d912c7d`, merged `3ddb824` | registry at 19 |
| whole-path M-. for Java/C#/Ruby (`issue-newlang-paths`) | LANDED | commit `7aa2787`; Java/CSharp arms in `crates/redline-syntax/src/node/paths.rs:116,144` | — |

### Plan 012 — project organization (archived 2026-09-21; structurally complete, verified at landing)


Plan issues (each staged, gate-green):

| Spec | State | Evidence | Notes |
|---|---|---|---|
| 012-01 inventory | LANDED | `docs/architecture.md` (module map + split plan) | — |
| 012-02 split pattern + buffers/views | LANDED | `src/app/store/{mod,buffers,views}.rs` | — |
| 012-03 search | LANDED | `src/app/store/search.rs` | — |
| 012-04 magit + commit/log/blame | LANDED | `src/app/store/{magit,commit}.rs` | — |
| 012-05 notes + annotations | LANDED | `src/app/store/{notes,notes_doc}.rs` | — |
| 012-06 navigation + index wiring | LANDED | `src/app/store/navigation/`, `src/app/store/index_wiring.rs` | — |
| 012-07 picker + minibuffer + project | LANDED | `src/app/store/{picker,minibuffer,project}.rs` | — |
| 012-08 tests split | LANDED | `src/app/store/tests/` (15 concern files) | — |
| 012-09 other oversized files + cleanup | LANDED | `crates/redline-syntax/src/node/{mod,paths,scopes}.rs`, `src/git/repo/index_ops.rs`, `src/ui/root/`, `src/nav/index/` | line-bar exceptions (test files, cohesive `store/mod.rs`) are the recorded audit judgments in PLAN.md |

Post-012 cleanup/organization lanes (task specs):

| Spec | State | Evidence | Notes |
|---|---|---|---|
| `issue-a1-root-decompose` | LANDED | `Root` is now 67 lines (`src/ui/root/mod.rs:34`), jobs moved to `src/ui/root/{geometry,hooks,render}.rs` | was "SPECD" in the worklist |
| `issue-a2-keymap-table` | LANDED | `GLOBAL_BINDINGS`/per-view binding tables (`src/app/store/mod.rs:231`+), `load_bindings`/`parse_sequence` | was "SPECD" |
| `issue-a3-modal-chain` | LANDED | `key_event` reads as a priority chain of named modals (`src/app/store/keys.rs:11`); each guard routes to a named handler | was "SPECD" |
| `issue-a4-xref-decompose` | LANDED | `xref_find_definitions` is 23 lines at `:151` — reproducible via `python3 tools/fn_survey.py --min 1 src/app/store/navigation/definitions.rs` (so is `xref_mdot_candidates`, 128 lines at `:206`) — split along its phase labels | measurement dated at main `1a82f02`; an undated measurement in this table goes stale when another lane reshapes the file (the `jump-ambiguity` lane did exactly that to `definitions.rs`) |
| `issue-a7-navigation-split` | LANDED | `src/app/store/navigation/{mod,definitions,imports,xref}.rs` — 1,035/902/195/614 lines, all under the 1,500 bar | was "NEW — criterion unmet" in the worklist |
| `issue-cleanup-tail` | LANDED | `src/ui/views/` inlined (gone); store test-helper dedup (commit `a1c2e47`); `Snapshot::from_store` CLOSED with the gate-verified justification (worklist § What remains) | was "in flight" |
| `issue-mechanical-sweep-1` | LANDED | M6: 20 `register_*` fns + `registry_has_the_seed_commands` (`src/app/command.rs:904`); M4: one `text_style` (`src/ui/mod.rs:47`); R2: `blob_side_ends_with_newline`/`find_hunk_in` (`src/git/repo/index_ops.rs:88,109`); M7: `Section::render` (`src/model/sections.rs:78`), `Node::collect_command_pairs` (`src/app/keymap.rs:322`) | — |
| `issue-invariant-pins` | LANDED | C13: `supports_reuse_agrees_with_registry_locals_queries` (`crates/redline-syntax/src/highlight.rs:736`); F-8: `vendored_highlight_queries_match_copy_time_sha256` (`crates/redline-syntax/src/language.rs:750`); C14/C8: graft asymmetry pinned both sides (`src/model/files.rs:558,577` + `src/search/rg.rs:1081`) | — |
| `issue-word-char` | LANDED | single `is_word_char` (`src/model/buffer.rs:54`), consumed by `rg.rs:592` + `references.rs:69` | C15 closed |
| `issue-xref-deletion` | LANDED | no `trait Xref` remains in `src/` or `crates/` (grep = zero) | D2 closed |
| `issue-magit-section-movement` | LANDED | `"No next section"`/`"No previous section"` (`src/app/store/magit.rs:40,57`) + pins (`src/app/store/tests/magit.rs:344–421`) | C5 closed |
| `issue-isearch-column` | LANDED | `Buffer::try_byte_to_line_col` (`src/model/buffer.rs:195`) + multibyte pin (`:500`); isearch lands at the match column (`src/app/store/search.rs:179`); `jump_back_restores_recorded_column` (`src/app/store/tests/navigation/jump.rs:111`) | — |
| `issue-column-landings` | LANDED | all four callers now column-accurate: C-x C-x (`src/app/store/buffers.rs:459`), isearch cancel `pre_search_col` (`src/app/store/search.rs:26`), search RET (`search.rs` Hit.col path), unique-def (`src/app/store/navigation/definitions.rs:435`) | was "SPECD first" in the worklist |
| `issue-match-highlight` | LANDED | second-pass match overlay (`src/ui/file_view.rs:219`), `RowFace::Match`/`MatchCurrent` (`:240–241`) | **was silently landed** — the worklist said "No highlighting exists today" until this repair (state corrected); the spec was refreshed in `1a82f02` |
| `issue-picker-density` | LANDED | `label`/`detail` name-first rows + `truncate_left` (`src/ui/picker.rs`), `picker_empty_preview_uses_full_width` (`src/ui/root/mod.rs:372`); commit `6d01393` | — |
| `issue-picker-followup` | LANDED | the eight remaining pickers converted (label/detail builders in `src/app/store/picker.rs`); commit `5d53a35` | — |
| `issue-golden-helpers` | LANDED | `crates/redline-resolve/tests/common/mod.rs` + `mod common;` in `golden_{go,python,rust}` (`:38/:48/:37`); `golden_js.rs:44` **deliberately opts out** ("the js suite deliberately does NOT share from `tests/common/mod.rs`" — its probes come from a `probes.toml` manifest, not `*.golden` files) | T1 |
| `issue-descriptor-table` | LANDED | one `LANGUAGES`/`LanguageSpec` table replaces the 8 sync sites (commit `6229039`); `supports_reuse` is a thin wrapper over the row (`crates/redline-syntax/src/highlight.rs:258`) | D1 |
| `issue-nav-index-split` | LANDED | `src/nav/index/{mod,builder,progress,symbol_index}.rs` | M1/M2 + S4 |
| `issue-node-split` | LANDED | `crates/redline-syntax/src/node/{mod,paths,scopes}.rs` | — |
| `issue-repo-split` | LANDED | `src/git/repo/index_ops.rs` (hunk math + index ops) | — |
| `issue-root-split` | LANDED | `src/ui/root/{mod,geometry,hooks,render}.rs` | — |
| `issue-store-module-pattern` | LANDED | `src/app/store/mod.rs` + first concern extractions | 012-02 |
| `issue-store-concern-moves` | LANDED | 13 concern files under `src/app/store/` | 012-03+ |
| `issue-store-test-split` | LANDED | `src/app/store/tests/` per concern | phase 3 |

### Grouped rows — sampled at archiving (001, 002, 003, 005, 006, 008, 011), plus one 012 cross-reference (NOT sampled)

Grouped as LANDED per plan-process archiving (the plan folders are in
`.agents/plans/archive/` and the features are live on main).

Spot-checks named, per the task's instruction:

| Group | Specs covered | State | Evidence (spot-checks named) |
|---|---|---|---|
| 001 (code browser) | `issue-01-impl`, `issue-02-impl`, `issue-03-impl`, `issue-04-impl`, `issue-05-impl`, `issue-06-impl`, `issue-07-impl`, `issue-08-impl`, `issue-09-impl` (9 specs) | LANDED | spot-checked: the app skeleton + command registry (`src/app/command.rs`), the project layer (`src/model/project.rs`), file watching (`src/app/watcher.rs`) — all live |
| 002 (magit depth) | `issue-002-01-impl`, `issue-002-02-impl`, `issue-002-03-impl`, `issue-002-04-impl`, `issue-002-05-impl` (5) | LANDED | spot-checked 002-01: the transient menu (`src/ui/transient_menu.rs:1` "issue 002"); the armed-discard + full verbs are the `discard_armed`/`menu_key_event` guards in `src/app/store/keys.rs` |
| 003 (scroll & watcher) | `issue-003-01-impl`, `issue-003-02-impl`, `issue-003-03-impl` (3) | LANDED | spot-checked 003-01: the watcher publishes to the app (`src/app/watcher.rs`); shared windowing `window_slice` lives in `src/app/store/helpers.rs` |
| 005 (edit modes) | `issue-005-01-impl`, `issue-005-02-impl`, `issue-005-02b-marker-gutter`, `issue-005-02c-span-fill`, `issue-005-03-impl` (5) | LANDED | spot-checked 005-01: `Buffer.editable` ("plan 005 issue 01", `src/model/buffer.rs:67`) + the `C-x C-q` toggle/confirm (`src/app/store/buffers.rs:345–354`); 02c: canvas-span fill (commit `eb5be7c`); 03: the quit dump reroutes stdout (`src/main.rs` tty reroute) |
| 006 (tooling-aware jump) | `issue-006-01-impl`, `issue-006-01b-js-provider`, `issue-006-01c-python-provider`, `issue-006-01d-go-provider`, `issue-006-02-impl`, `issue-006-02b-review-followups`, `issue-006-03-impl`, `issue-006-03b-review-followups` (8) | LANDED | spot-checked: the four providers (`crates/redline-resolve/src/{cargo,providers/{js,python,go}}_provider.rs`), the M-. fall-through (`apply_resolve_event`, `src/app/store/navigation/mod.rs`), the external-crate index (`EXT_INDEX_FILE_CAP`, `src/app/store/index_wiring.rs:481`), the 006-02b/03b follow-ups pinned in-code (`src/app/store/navigation/xref.rs:257`, `mod.rs:31`) |
| 008 (external annotations) | `issue-008-01-impl` | LANDED | spot-checked: annotations on external buffers store the absolute path (`src/app/store/notes.rs:104`) |
| 011 (resolver parity) | all (see the 011 table — every task spec named there) | LANDED | full table above |
| 012 (project organization) | all (see the 012 tables — every spec named there) | LANDED | full tables in this section — verified at landing, not sampled |

## Governing criterion added 2026-09-21 (user directive)

**"ALL symbols should jump to the right definition."** Recorded as
`.agents/tasks/issue-all-symbol-jumps.md`: right file, right symbol, right **column**,
checkable as a **symbol-kind × jump-path matrix** with a test per cell. It exists because
the same defect class was reported twice (line-start jumps, then field jumps landing at
column 0) while every gate stayed green — the PTY tier has **one** column pin in the
whole suite, so a col-0 landing on an `M-.` path could not fail the battery.

It **supersedes** one-off fixes for individual jump paths: if a cell is wrong, fix it
there. The justified col-0 exceptions (goto-line, annotations-anchored, impl-block
headers) must be listed and defended in that issue; everything else is a bug.
