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
replaced, `open_external_path` (`src/app/store/navigation/mod.rs:23-29`) replaces
whatever buffer holds that key. The gate probed it directly: calling it on a key that is
an open, **dirty**, `Accurate` project buffer lost the unsaved text and reset the mode to
`Annotation` — the exact pre-fix failure mode, now guarded everywhere *except* here.

**User-path reachability is UNVERIFIED.** The gate tried to construct a landing whose
file lies inside the project root and could not get a picker to open at all
(`picker_open=false`) — inconclusive, not a disproof. Note the sibling tooling route
*does* handle in-project resolved sources explicitly
(`navigation/mod.rs:55-62`: `strip_prefix(project.root)` → `open_project_path`), which
shows the shape is anticipated. **Fix:** `is_none`-guard it, or use `reload_in_place` for
an existing key. Prove reachability or prove it unreachable — do not leave "probably
unreachable" as the justification, since that is the same reasoning that made this bug
latent until Accurate-mode editing existed.

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
