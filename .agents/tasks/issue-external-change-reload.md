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
