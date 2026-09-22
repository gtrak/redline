# issue-external-change-reload — an externally-changed file silently discards unsaved edits

**Found by the 015-02 gate (P2), pre-existing, not introduced by that lane.**

## The bug

`src/app/store/project.rs:33-48` re-opens a file when the on-disk mtime has changed,
via `insert_rope(..., false)` on the **existing** key. The call is justified by a
comment claiming *"Issue-03 buffers are read-only, so this is safe"*.

That justification is **false**, and was already false before plan 015 for any
buffer in edit mode:

* `insert_rope` **replaces the `Buffer`**, so the per-buffer mode resets to
  `Annotation` — a buffer you had put in `Accurate` mode silently leaves it; and
* it **drops unsaved in-memory edits** — the rope is replaced wholesale, so
  anything typed but not saved is gone, with no prompt and no record.

So: edit a file, leave it unsaved, let something touch the file on disk (a build, a
formatter, a `git checkout`, a background agent), and your edits vanish silently.
That is a **data-loss path**, and it is exactly the class the `locally_modified` /
`changed_on_disk` machinery exists to handle — it is already tracked, so the fix is
a policy decision rather than new bookkeeping.

## What to decide and implement (state the choice in the commit)

Emacs's behaviour is the reference: when the file changed on disk and the buffer is
modified, it **refuses to revert silently** and asks (`revert-buffer` →
`file has changed on disk; really edit the buffer?`). Pick a policy and justify it:

* **(a) Ask** — arm a confirm ("file changed on disk; discard your edits?"), the
  same shape as the existing quit/discard-confirm state machine, and only replace
  the rope on an explicit yes. Most faithful; reuses a mechanism that already
  exists.
* **(b) Refuse while dirty** — keep the buffer and surface the conflict
  (`changed_on_disk` already exists for this), reverting only when the buffer is
  clean. Cheap and safe, but the user must resolve it manually.
* **(c) Auto-revert only when clean** — i.e. make the existing comment *true* by
  gating on `locally_modified`, and leave the dirty case alone with a visible
  marker. The minimum that removes the data-loss path.

Whichever: **preserve the mode across the reopen** (the buffer identity did not
change, only its contents), and **correct the comment** so it stops asserting a
safety property that does not hold.

## Acceptance

* A test proving **unsaved edits are not silently discarded** when the on-disk
  mtime changes — the gate's reproduction is: edit → touch the file → let the
  watcher fire → assert the text (or an explicit prompt) survived.
* A test proving a **clean** buffer still picks up the new on-disk content (the
  behaviour that must keep working).
* A test proving the **mode survives** the reopen (a buffer in `Accurate` is still
  `Accurate` afterwards), since that is the second half of the same defect.
* The comment's false justification is gone.
* `cargo test --workspace` + clippy `-- -D warnings` clean; `tools/gate.sh full` green.

## Fence

`src/app/store/project.rs` (the reopen path), `src/app/store/index_wiring.rs` (the
reload paths, which the 015-02 gate verified touch only
`rope/mtime/locally_modified/changed_on_disk`), the watcher plumbing if the policy
needs it, and tests. Disclose anything else.

## Interaction to respect

Plan 016 (undo) defines a **dirty-flag contract** — "undoing back to the saved state
must clear `locally_modified`" — and its own design notes say a file reloaded from
disk must **clear the undo history** because recorded offsets become invalid. So the
policy chosen here is the one 016 must build on: if this lands first, 016 inherits a
defined reload semantics; if 016 lands first, this must honour its history-clearing
rule. Say which you assumed.

## Follow-ups from the gate (all after the fix landed as `28aa9e9`)

**F1 — `open_external_path` calls `insert_rope` UNCONDITIONALLY (a guard is wanted).**
Unlike `open_notes`, which is `is_none`-guarded so an already-open buffer is never
replaced, `open_external_path` (`src/app/store/navigation/mod.rs:39`) replaces
whatever buffer holds that key. The gate probed it directly: calling it on a key that is
an open, **dirty**, `Accurate` project buffer lost the unsaved text and reset the mode to
`Annotation` — the exact pre-fix failure mode, now guarded everywhere *except* here.

**User-path reachability — PROVEN REACHABLE** (was UNVERIFIED; the gate's own
attempt — a landing whose file lies inside the project root — could not get a
picker open, `picker_open=false`, which was inconclusive). The route that
reaches it: a **project root nested inside an indexed external crate root**
(a project opened inside a crate source tree the tooling resolver has
indexed — exactly the in-project shape the sibling tooling route anticipates
with its `strip_prefix(project.root)` → `open_project_path` branch at
`navigation/mod.rs:86-92`, which is why that route is safe: it routes
in-project files to the guarded project open). The picker's crate-relative
index rows do **not** have that route: RET on an Xref index row of a
crate-rooted picker takes the `Some(crate_root)` arm
(`picker.rs`: `open_external_path(&root.join(file))`, `navigation/xref.rs`
silent-jump arm the same) with NO in-project check. Sequence:
(1) crate root R indexed (006-03 `start_crate_indexing` event);
(2) project P rooted at `R/sub`; `R/sub/src/app.rs` open, Accurate, dirty;
(3) land in external `R/src/lib.rs` (M-. into a registry/tooling source,
or any external open);
(4) M-. on a symbol defined in `R/sub/src/app.rs` → cross-file → crate-
rooted Xref picker;
(5) RET → `open_external_path(R.join("sub/src/app.rs"))` → the held key of
the open, dirty project buffer → pre-fix: edits and mode both lost. Pinning
tests (all in `src/app/store/tests/navigation/landings.rs`):
`crate_index_landing_inside_project_root_keeps_dirty_buffer` (this exact
sequence — RED pre-fix, GREEN post-fix) and
`external_landing_on_dirty_project_buffer_keeps_edits_and_mode` (the gate's
direct probe — RED pre-fix, GREEN post-fix). The fix: `open_external_path`
no longer calls `insert_rope` unconditionally — on a HELD key, a DIRTY
buffer is left content-wise alone (just made current; its conflict belongs
to the watcher / `open_project_path` machinery) and a CLEAN one is refreshed
IN PLACE via the `reload_in_place` chokepoint only when the on-disk mtime
moved (`external_reland_refreshes_clean_cache_in_place` pins the preserved
cache-refresh behaviour; `external_landing_on_clean_accurate_project_
buffer_refreshes_in_place` pins identity survival on the clean path).
`external_buffers` registration now happens only on CREATION, so a project
buffer can no longer be mis-classified as an external cache entry.

**F2 — one thin PTY leg for the new confirm.** No PTY flow exercises it, so its on-screen
rendering is uncovered. The store tests drive the same entry point the PTY path uses
(`src/ui/root/input.rs::to_app_key` → the same `AppKey`/`Key` that `AppKey::key_event`
consumes), so the *logic* is proven; what is not is that the prompt actually renders in
the live flow. `sweep_flows.flow_banner_hint` is the closest leg and exercises only the
*clean* watcher path. Given this project just spent a day on exactly "user-visible
behaviour, no PTY assertion, two bug reports with every gate green", and this prompt is
destructive-adjacent, add one leg: edit → external write → reopen → assert the prompt
text → `n` → assert the edits and marker survive → `y` → assert the disk text.

**F3 — `toggle_ro_accept` is the fifth rope-replacing route (for plan 016).** It assigns
`buf.rope` directly (`src/app/store/buffers.rs:990-1009`) and so bypasses
`reload_in_place`. It is an explicit user command, so the *requirement* holds, but 016's
"a disk reload clears the undo history" rule must hook **both** sites — the chokepoint and
this one. Recorded in the `reload_in_place` doc comment too.

**F4 — the `locally_modified` gate depends on an invariant.** The fix treats
`locally_modified == false` as "holds no in-memory work", which the gate verified by
enumerating all 12 rope-mutation sites (each sets the flag). 016's dirty-flag contract
("undoing to the saved state clears `locally_modified`") is what keeps that meaning true,
so 016 must preserve it — a flag that can be false while the text differs from disk would
re-open this hole.
