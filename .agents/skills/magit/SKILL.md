---
name: magit
description: Authoritative reference for how Magit 4.7.1 (the Emacs Git interface) actually behaves — section-based buffers, point-addressed dwim keys, transients, status/staging/diff/commit/log/blame/branch/stash semantics with verified keybindings — so a Rust implementer can faithfully build the magit-like TUI surface for redline issues 07 (git status & staging) and 08 (log, blame, commit, branches) without relying on training-data guesses.
---

# magit

Reference for building a magit-like surface. Everything below was sourced from
the current Magit User Manual (4.7.1) at docs.magit.vc; keys are cited from the
manual pages listed in Sources. Items the manual pages did not settle are
marked **NOT VERIFIED** — do not implement them from memory.

## Manual version

Magit **4.7.1** User Manual (https://docs.magit.vc/magit), fetched 2026-09.
Note: the manual formerly lived at magit.vc/manual/magit (now 404). Key
bindings changed across magit releases (e.g. the `b` branch transient suffixes
below are 4.7.1's; older tutorials show a different layout) — trust this
document over muscle-memory of older versions.

Redline implements a **subset**: status/sections, file+hunk staging, diff
view, inline commit, log, blame, branch switch, stash. Rebase, cherry-pick,
worktrees, submodules, ediff, forges are deferred (see Mapping to redline).

## Core concepts

- **Every Magit buffer is a tree of sections** (manual 4.2). Sections are
  nested, collapsible/expandable (Org-like), each has a **type** (`hunk`,
  `file`, `commit`, ...) and often a **value** (e.g. a file section's value is
  the file name). The section under point determines what a key does — this is
  the "dwim" model: the same key (`s`, `u`, `RET`, ...) behaves differently
  depending on the section type at point. Commands can also act on a set of
  sections selected by an active region.
- **Section movement** (4.2.1, verified): `p`/`n` previous/next section,
  `M-p`/`M-n` previous/next *sibling* (falling back to parent), `^` parent.
  Movement runs `magit-section-movement-hook`, which in log/status buffers can
  auto-update a companion "revision" (commit/diff) buffer displayed in another
  window (`magit-update-other-window-delay` controls the debounce).
- **Section visibility** (4.2.2, verified): `TAB` toggle body of current
  section; `C-c TAB` cycle current section *and children*; `M-<tab>` cycle
  diff-related sections; `S-<tab>` cycle all sections in buffer; `1`–`4` show
  levels around point, `M-1`–`M-4` buffer-wide. Initial visibility is
  hardcoded per buffer type; **on refresh, previous visibility is preserved**.
  `H` (`magit-describe-section`) inspects the section at point.
- **Transients** (4.3; Getting Started 3): a transient prefix command is a
  temporary menu buffer shown at the bottom of the frame listing **infix
  arguments** (toggles/choices) and **suffix actions**, each with its key.
  You type the prefix, optionally set arguments, then the suffix. The top
  level is `h` (all commands) — globally bound `C-x M-g` (`magit-dispatch`);
  `C-c M-g` (`magit-file-dispatch`) is a file-oriented global transient; the
  manual recommends rebinding to `C-c g` / `C-c f`.
- **One buffer per repository per mode** (4.1): one status, one log, one diff,
  etc. buffer per repo; other buffers can be "locked" to a value (e.g. a
  specific commit). `q` buries the current Magit buffer.
- **Automatic refresh** (4.1.4, verified): after running any git command with
  side-effects, the **current Magit buffer and the status buffer** are
  refreshed automatically (option `magit-refresh-status-buffer` keeps the
  status buffer in sync even from other buffers). Refresh **re-creates the
  buffer contents from scratch** and preserves section visibility. Manual
  refresh: `g` (`magit-refresh`) = current buffer + status buffer; `G`
  (`magit-refresh-all`) = all Magit buffers of the repo + revert unmodified
  tracked file buffers.
- **Margins**: log/status/stash buffers can show a margin with commit age and
  author (`magit-log-margin`, `magit-status-margin`, form
  `(INIT STYLE WIDTH AUTHOR AUTHOR-WIDTH)`; toggle `L L`, cycle style `L l`,
  details `L d` in log buffers).
- **Header info** (status buffer): branch/upstream/push-branch/tag info is
  shown as header *sections* at the top of the status buffer, not in a
  mode-line. (Manual FAQ A.2.8 concedes "the mode-line information isn't
  always up-to-date" — magit's canonical status display is the status buffer
  header.)

## Status buffer

- Open with `C-x g` (`magit-status`) — the manual's recommended global
  binding. `magit-status-quick` avoids refreshing when the buffer exists but
  is not displayed; if it *is* displayed, it refreshes.
- Contents are built from the hook `magit-status-sections-hook`. Documented
  inserter functions:
  - **Headers** (inserted first, via `magit-status-headers-hook`): error
    header (last git error, disappears on next refresh), diff-filter header,
    **head-branch header** (current branch or detached HEAD), **upstream-branch
    header**, **push-branch header**, tags header (current/next tag + commit
    count to HEAD). Optional: repo path, remote, user.
  - **In-progress operation sections** (only when applicable): merge log,
    rebase sequence, am (patch-applying) sequence, cherry-pick/revert
    (sequencer) sequence, bisect output/rest/log.
  - **Untracked files** (list; `magit-status-show-untracked-files` = `nil`/`t`
    (directories) /`all`; each file-list section is capped at
    `magit-status-file-list-limit` entries for performance).
  - **Unstaged changes** (worktree vs index diff).
  - **Staged changes** (index vs HEAD diff).
  - **Stashes** (reflog of `refs/stash`).
  - **Unpulled from upstream** / **unpulled from push-remote** (commit lists).
  - **Unpushed to upstream, or recent commits**: if upstream exists and is
    behind, show unpushed commits; otherwise the last
    `magit-log-section-commit-count` commits (the "Recent commits" section).
  - (The exact default *order* of these hook members is set in magit's source,
    not enumerated in the manual — **NOT VERIFIED** here. The conventional
    order is headers → operation sections → untracked → unstaged → staged →
    stashes → unpulled → unpushed/recent; verify against
    `magit-status-sections-hook`'s default value before snapshotting.)
- **File lines**: each file in a diff section is a `file` section whose
  heading line carries a single-letter status indicator plus (in diff
  sections) a diffstat. Lines are formatted by `magit-format-file-function`
  with signature `(kind file face &optional status orig)` — kind is `diff`,
  `module`, `stat`, or `list`; `orig` is the pre-rename name. The conventional
  letters (U untracked, A added, M modified, D deleted, R renamed with orig
  name shown, C copied) are **NOT VERIFIED** from the manual pages fetched.
- **Hunk expansion**: expanding a file section (in unstaged/staged sections)
  reveals its `hunk` sections; a hunk heading shows the `@@` line range and
  context; the body is the diff text (added/removed/context lines).
- **RET behavior per element type** (verified in 5.8.2, 5.3.2, 5.9):
  - file section / hunk heading / diff line → `RET` = `magit-diff-visit-file`:
    visits the *blob* of the side that still contains the line — added or
    context line → new/right side; **removed line → old/left side** (option
    `magit-diff-visit-previous-blob`, default `t`). For unstaged changes the
    sides are index blob vs worktree file; for staged, HEAD blob vs index
    blob. Point lands at the corresponding location. `C-<return>` =
    `magit-diff-visit-worktree-file` always visits the real worktree file.
  - commit (in recent-commits/unpushed/unpulled sections) → shows that commit
    in the repository's revision (commit) buffer; in log buffers `SPC`/`DEL`
    (`magit-diff-show-or-scroll-up/down`) shows the commit at point or, if
    its commit buffer is already displayed in the frame, scrolls it instead.
  - stash entry → shows the stash's diffs (also `z v`).
- **Status buffer keybindings** (all verified from the manual):

| Key | Command | Effect |
|---|---|---|
| `C-x g` | `magit-status` | show/refresh status buffer |
| `TAB` | `magit-section-toggle` | fold/unfold section at point |
| `C-c TAB` | `magit-section-cycle` | cycle section + children visibility |
| `M-<tab>` | `magit-section-cycle-diffs` | cycle diff sections |
| `S-<tab>` | `magit-section-cycle-global` | cycle all sections |
| `1`–`4` / `M-1`–`M-4` | show-level-N | show sections up to level N (at point / buffer) |
| `p` / `n` | section-backward/forward | move between sections |
| `M-p` / `M-n` | sibling movement | previous/next sibling (else parent) |
| `^` | `magit-section-up` | move to parent section |
| `H` | `magit-describe-section` | inspect section at point |
| `s` / `S` | stage / stage-all | see Staging & unstaging |
| `u` / `U` | unstage / unstage-all | see Staging & unstaging |
| `k` | `magit-discard` | discard change at point (see below) |
| `a` / `v` | `magit-apply` / `magit-reverse` | apply/reverse change at point to worktree |
| `d` / `D` | diff transient / diff-args transient | see Diff viewing |
| `l` / `L` | log transient / log-args transient | see Log |
| `c` | commit transient | see Committing |
| `b` | branch transient | see Branches |
| `z` | stash transient | see Stashing |
| `h` | `magit-dispatch` | top-level transient (all commands) |
| `q` | `magit-mode-bury-buffer` | bury/kill current Magit buffer |
| `g` / `G` | refresh / refresh-all | see Refreshing |
| `Y` | `magit-cherry` | cherries view (deferred in redline) |

Note: discard is **lowercase `k`** in Magit 4.7.1 (not `K`). `k` in a stashes
buffer means `magit-stash-clear` (remove all stashes) — same key, different
section context.

## Staging & unstaging

Verified from 6.3 + 6.4 + Getting Started:

- `s` (`magit-stage`): "add the change **at point** to the staging area."
  Granularity is whatever section/region is current:
  - on a **file** section → stage whole file;
  - on a **hunk** section (file expanded) → stage that hunk;
  - with a **region inside a hunk body** (mark set with `C-SPC`, moved so the
    region covers some but not all added/removed lines) → stage only those
    lines ("hunk-internal region");
  - with a **region spanning the headings of sibling file/hunk sections** →
    stage all of them at once (both mark and point must be on sibling
    headings).
  - With a **prefix argument on an untracked file** → stage the file *without*
    its content (intent-to-add), enabling partial staging of new files.
- `S` (`magit-stage-modified`): stage all changes to files modified in the
  worktree (also stages new content of tracked files and removes tracked
  files deleted from the worktree from the index). With a prefix argument also
  stages previously untracked (non-ignored) files.
- `u` (`magit-unstage`): "remove the change at point from the staging area."
  Same granularities as `s` (file, hunk, hunk-internal region, multi-section
  region). Only **staged** changes can be unstaged. Exception: when point is on
  a **committed** change and `magit-unstage-committed` is non-`nil` (default),
  `u` instead **reverses the change in the index but not the working tree**
  (`magit-reverse-in-index`) — the documented workflow for extracting a
  change out of HEAD.
- `U` (`magit-unstage-all`): remove all changes from the staging area.
- `k` (`magit-discard`): remove the change at point from the working tree —
  on a staged change it removes it from **both** worktree and index; on an
  unstaged change, from the worktree only. On a hunk/file with unresolved
  conflicts it prompts which side to keep (no prompt if point is inside a
  side's text). Magit asks for confirmation before discarding (see manual 4.5
  "Completion, Confirmation and the Selection"; the exact prompt wording is
  **NOT VERIFIED** here).
- **Internal mechanics** (6.4, verified): stage, unstage, discard, reverse
  and apply are all "apply variants" and, **at least when operating on a
  hunk, are all implemented using `git apply`**. Concretely:
  - stage hunk = apply the hunk's patch **to the index** (worktree keeps the
    change);
  - unstage hunk = **reverse-apply** the hunk's patch **to the index**
    (worktree keeps the change);
  - discard = apply the reverse patch to the **worktree** (and index if the
    change was staged);
  - with a prefix argument all variants fall back to a 3-way merge.
  The manual explicitly notes (4.1.6) that discarding "is done by running
  `git apply --reverse ...`". Redline's git2 layer has no turnkey
  reverse-apply: see the git2 skill's "Hunk-level staging" section for the
  building-block recipe; the target semantics are exactly the above
  (patch → index only, forward for stage, reversed for unstage).
- From a file-visiting buffer (8.10/6.3.1): `magit-stage-files` /
  `magit-unstage-files` stage/unstage the whole visited file.

## Diff viewing

Verified from 5.4:

- `d` (`magit-diff`) transient — suffixes:
  - `d d` (`magit-diff-dwim`): show changes for the thing at point (commit →
    its changes; "Unstaged changes" section → those changes; region of two
    commits → diff between them). Explicitly described as "the smallest
    common denominator... there is no AI involved".
  - `d r` range (A..B / A…B; defaults to HEAD; single commit = worktree vs it)
  - `d w` working tree vs HEAD (prefix: vs a commit read from minibuffer)
  - `d s` index vs HEAD (staged)
  - `d u` worktree vs index (unstaged)
  - `d p` two arbitrary files on disk
  - `d c` show commit at point (prefix: prompt)
  - `d t` show all diffs of a stash
- **Structure**: a diff buffer contains a **diffstat section** and **file
  sections**, each file section containing **hunk sections**, each hunk body
  the added/removed/context lines. `j` jumps between the diffstat and the
  corresponding diff (file in stat → its diff; else → stat).
- **Unstaged vs staged vs commit diffs** are the same section hierarchy with
  different sides: unstaged = index (left) vs worktree (right); staged = HEAD
  (left) vs index (right); commit = parent (left) vs commit (right).
- **RET on hunk/line** = `magit-diff-visit-file` (blob of the containing
  side, point moved to the matching location); `C-<return>` = worktree file.
  (Details in Core concepts / Status buffer.)
- **Context size**: `-` decrease context lines, `+` increase, `0` reset to
  default (verified 5.4).
- **`D` (`magit-diff-refresh`) transient** changes diff arguments without
  changing which diff is shown (works in diff buffers *and* the status
  buffer): `D g` set local args; `D s` set defaults; `D w` save defaults;
  `D t` toggle hunk refinement (word-granularity diffs); `D T` toggle hunk
  fontification; `D r` switch range type (`..` ↔ `…`); `D f` flip revisions;
  `D F` toggle file filter (in a log buffer, toggles the restriction of the
  companion revision buffer).
- **Apply keys in diffs** (6.4, verified): `a` (`magit-apply`) apply the
  change at point to the working tree (prefix: 3-way fallback, which also
  applies to the index); `v` (`magit-reverse`) reverse it; `s`/`u` stage/
  unstage work in any diff buffer ("move point into the respective section
  inside a diff displayed in the status buffer **or a separate diff buffer**
  and type `s` or `u`" — 6.3).
- Other diff-buffer keys: `C-c C-d` show the changes about to be committed
  (toggles new-vs-all while amending); `C-c C-b`/`C-c C-f` move backward/
  forward in the buffer's revision history; `C-c C-t` trace the definition at
  point (log of the symbol); `C-c C-e` edit the commit of the hunk at point
  (interactive rebase — deferred in redline); `SPC`/`DEL` scroll.
- **Faces/colors**: the manual documents the *options* rather than a full
  face list: separate indicator faces (`magit-diff-removed-indicator` etc. via
  `magit-diff-use-indicator-faces`), whitespace painting
  (`magit-diff-paint-whitespace`, trailing-whitespace and indentation
  highlight), word-granularity refinement (`magit-diff-refine-hunk`), hunk
  fontification (`magit-diff-fontify-hunk`). The exact conventional face
  names (magit-diff-added/-removed/-hunk/-file/-header) live in the
  "Theming Faces" node (10.4.1) which was not fetched — **NOT VERIFIED** here.
  Conventional rendering: added lines green, removed red, hunk/file headings
  distinct — treat as **NOT VERIFIED** detail.

## Committing

Verified from 6.5:

- Magit runs `git commit` **without** `--message`: Git creates
  `.git/COMMIT_EDITMSG` and invokes the editor (magit arranges for that to be
  `emacsclient`), i.e. the commit happens when the editor session ends with
  exit status 0. (Redline replaces this with an inline editable buffer — see
  Mapping.)
- `c` (`magit-commit`) transient suffixes:
  - `c c` (`magit-commit-create`) — create a new commit.
  - `c e` (`magit-commit-extend`) — `git commit --amend --no-edit` (amend
    staged changes to HEAD, keep message; prefix arg keeps committer date).
  - `c a` (`magit-commit-amend`) — `git commit --amend --edit` (amend and edit
    the message).
  - `c w` (`magit-commit-reword`) — `--amend --only --edit` (edit message
    only; tree unchanged, staged changes stay staged).
  - `c f` fixup / `c s` squash / `c A` alter / `c n` augment / `c W` revise —
    create fixup/squash commits targeting the reachable commit at point
    (autosquash later).
  - `c F` / `c S` — instant fixup/squash: immediately run an `--autosquash`
    rebase (always require confirmation).
- **After initiating**: two buffers appear — the **commit message buffer** and
  a **diff buffer of the changes about to be committed** (auto-shown;
  `magit-commit-show-diff` controls it). `magit-commit-ask-to-stage` makes
  magit offer to stage all unstaged changes when nothing is staged.
- **Message buffer keys** (verified):
  - `C-c C-c` (`with-editor-finish`) — finish with exit code 0 → Git creates
    the commit.
  - `C-c C-k` (`with-editor-cancel`) — finish with exit code 1 → Git cancels
    the commit, the message file is left untouched, repo state unchanged.
  - `M-p`/`M-n` cycle the commit-message ring; `C-c M-s` save current message
    to the ring.
  - Pseudo-header inserters: `C-c C-a` Acked-by, `C-c C-r` Reviewed-by,
    `C-c C-s` Signed-off-by, `C-c C-t` Tested-by, `C-c C-o` Cc:, `C-c C-p`
    Reported-by, `C-c M-i` Suggested-by.
  - `C-c C-d` toggle new-changes vs all-changes diff while amending.
- **Conventions injected/enforced** (verified): `git-commit-mode` (minor mode
  over any major mode) sets up auto-fill, flyspell (optional), ChangeLog
  paragraph handling; the summary line beyond
  `git-commit-summary-max-length` characters is colorized;
  `git-commit-check-style-conventions` runs on finish and asks for
  confirmation on violations of `non-empty-second-line` and
  `overlong-summary-line` (prefix argument forces through).
- **On success**: the commit is created by git; magit's post-commit hooks
  refresh the status buffer and any visible log/revision buffers (refresh
  semantics per Core concepts). `git-commit-post-finish-hook` runs after
  `C-c C-c`-style commits.

## Log

Verified from 5.3:

- `l` (`magit-log`) transient: `l l` current branch (detached/prefix: read
  revs); `l h` HEAD; `l u` related (branch + upstream + push-target); `l o`
  other revs; `l L` all local branches + HEAD; `l b` all local+remote;
  `l a` all refs. Reflog: `l r` current branch, `l O` other ref, `l H` HEAD.
- **Structure**: one line per commit by default, each line a `commit`
  section. When a graph argument is used, git's graph characters appear in a
  left-hand column; **references are rendered by magit, not git**: local
  branches blue, remote branches green, the current branch (and a remote's
  HEAD branch) get a box around the refname; a local branch and its
  push-target on the same commit are combined into one refname cell.
  `magit-log-show-refname-after-summary` moves refnames after the summary.
  Signature validity is colorized (faces `magit-signature-*`) rather than
  using `--show-signature`. `magit-log-show-color-graph-limit` drops `--color`
  past a size threshold for performance.
- **Margin** (5.3.3): per-commit age and/or author in the margin
  (`magit-log-margin`); `L L` toggle, `L l` cycle style, `L d` details.
- **Keys** (5.3.2, verified): `SPC`/`DEL` show-or-scroll the commit at point
  in the revision buffer; `RET` shows the commit at point (revision buffer);
  `C-c C-n` move to a parent (numeric prefix selects which); `j` jump to a
  revision read from minibuffer; `=` toggle commit limit (on/off, default 256
  when set), `+` double limit, `-` half limit; `q` bury; `L` log-args
  transient (`L g` local args, `L s` defaults, `L w` save defaults, `L L`
  margin); `C-c C-b`/`C-c C-f` revision history back/forward. The `d`
  transient also works: `d d` on a commit shows its changes in the diff
  buffer. A commit-selection log mode adds `C-c C-c` pick / `C-c C-k` abort
  (used for rebase-start/squash-target selection — deferred in redline).
  A `.` mark command for logs is **NOT VERIFIED** (not present in the fetched
  pages).
- **Log ↔ status interconnection**: the status buffer's "Recent commits",
  "Unpushed to …" and "Unpulled from …" sections *are* log sections
  (`magit-insert-recent-commits`, `magit-insert-unpushed-to-upstream-or-recent`,
  `magit-insert-unpulled-from-upstream`, ...; counts from
  `magit-log-section-commit-count`). Moving among them updates the companion
  revision buffer (section-movement hooks, see Core concepts).

## Blame

Verified from 5.9 — **do not guess; this is how real magit does it**:

- Invoked **from a file-visiting (or blob-visiting) buffer**, via the
  file-dispatch transient: default binding **`C-c M-g`** (`magit-file-dispatch`),
  recommended binding **`C-c f`**. The blaming sub-prefix is `B`:
  - `C-c M-g B b` / `C-c f B b` — `magit-blame` (a.k.a. `magit-blame-addition`):
    blame each line/chunk for the commits that last touched it.
  - `... B r` — `magit-blame-removal` (blob buffers only): which revision
    removed each line.
  - `... B f` — `magit-blame-reverse`: last revision in which a line still
    existed.
  - `... B e` — `magit-blame-echo`: like blame but not read-only, uses
    `magit-blame-echo-style`.
  - `... B q` — quit blame.
  - The suffixes can also be invoked directly from the file-dispatch transient
    (skipping the `B` sub-prefix) when no infix arguments are needed.
- **Display is inline, not a separate buffer**: the current file/blob buffer
  is augmented per line/chunk with commit info; `magit-blame-mode` turns on
  and the buffer is made read-only by default
  (`magit-blame-read-only`).
- **Keys while blamed** (verified): `RET` show the commit that last touched
  the line at point; `SPC`/`DEL` update the companion commit buffer; `n`/`p`
  next/previous chunk, `N`/`P` next/previous chunk *from the same commit*;
  `b` re-blame **recursively** (blame the parent of the commit that added the
  current chunk — "blame the blame"); `q` (or `C-c M-g B q`) quit (kills the
  buffer if it was created for a recursive blame); `M-w` copy the chunk's
  commit hash; `c` cycle visualization style (`magit-blame-styles`).
- If the buffer visits a revision of the file, history is considered up to
  that revision; otherwise full history including uncommitted changes.

## Branches

Verified from 6.6 (Magit 4.7.1 — note these suffixes differ from older
versions):

- `b` (`magit-branch`) transient:
  - `b b` (`magit-checkout`) — checkout a revision (default: branch/revision
    at point); non-branch → detached HEAD; **fails if worktree or index have
    changes**.
  - `b n` (`magit-branch-create`) — create a branch (read starting point,
    then name; a branch starting point may become the new branch's upstream).
  - `b c` (`magit-branch-and-checkout`) — create **and** check out.
  - `b l` (`magit-branch-checkout`) — check out existing or new *local*
    branch (candidates: local branches + remote branches without a same-name
    local; remote pick creates a tracking local branch).
  - `b s` spinoff / `b S` spinout — create branch at current HEAD and reset
    the old branch to its shared upstream commit (or a region-selected FROM).
  - `b x` (`magit-branch-reset`) — reset branch at point to another branch/
    commit (hard reset for the current branch; confirmation required when
    uncommitted changes would be lost).
  - `b k` delete (region: multiple; dangerous deletes require confirmation);
    `b m` rename; `b r` checkout a remote ref (hidden by default);
    `b C` (`magit-branch-configure`) view/change branch git variables
    (`branch.NAME.merge/remote/rebase/pushRemote/description`,
    `pull.rebase`, `remote.pushDefault`, autoSetup*).
- Magit's "two remotes" model: every branch has an **upstream** (what you
  pull/merge from) and a **push-remote** (what you push to); the status
  buffer shows unpulled/unpushed sections for both (see Status buffer).

## Stashing

Verified from 6.12:

- `z` (`magit-stash`) transient:
  - `z z` (`magit-stash-both`) — stash index + worktree. One prefix arg ≡
    `--include-untracked`; two ≡ `--all`.
  - `z i` index only; `z w` worktree (unstaged) only; `z x` both but keep the
    index intact (`--keep-index`).
  - Snapshot variants (commit the changes instead of stashing): `z Z` both,
    `z I` index, `z W` worktree.
  - `z a` apply (Git ≥ 2.38: try `--index` first, fall back to `git apply`
    with `--3way` or `--reject`); `z p` pop (drops the stash **only on
    complete success** — no conflicts and index preserved).
  - `z k` drop (region: drop all contained); `k` in a stashes buffer clears
    all stashes (`magit-stash-clear`); `z v` show all diffs of a stash;
    `z b` create+checkout a branch from the stash (starting at the commit
    current when stashed); `z B` same but from current HEAD, applying and
    dropping if clean; `z f` format-patch; `z l` list stashes in a buffer.
- Stashes appear in the status buffer as the "Stashes:" section (reflog of
  `refs/stash`) with a margin (`magit-stashes-margin`).

## Refreshing

Verified from 4.1.4 + 10.3 context:

- `g` (`magit-refresh`): refresh the current buffer (if it derives from
  `magit-mode`) **and the status buffer**; also reverts unmodified tracked
  file buffers when `magit-revert-buffers` calls for it.
- `G` (`magit-refresh-all`): refresh **all** Magit buffers of the current
  repository and revert all unmodified tracked file buffers (revert happens
  even if `magit-revert-buffers` is `nil`).
- **Automatic**: after any git command with side-effects, the current Magit
  buffer is always refreshed, and the status buffer too
  (`magit-refresh-status-buffer`, non-`nil` by default). Other Magit buffers
  are deliberately *not* auto-refreshed (delay + sometimes undesirable).
- Refresh re-creates contents from scratch (can be slow in large repos) and
  **preserves section visibility**; initial (first-creation) visibility is
  hardcoded per section type, overridable via
  `magit-section-set-visibility-hook` / `magit-section-initial-visibility-alist`.
- Related buffer hygiene: `magit-save-repository-buffers` (save modified
  file-visiting buffers before running commands/refresh; `dontask` = silently)
  and auto-reverting of tracked file buffers after on-disk changes
  (`magit-auto-revert-mode`, immediate via `magit-auto-revert-immediately`).
  Redline's watcher bus (plan 04/07) is the equivalent of the auto-revert +
  auto-refresh machinery.

## Key quick-reference

Consolidated (all keys verified from the manual pages in Sources):

| Key | Command | Applies to (section under point) | Redline issue |
|---|---|---|---|
| `C-x g` | `magit-status` | — (repo at point) | 07 |
| `TAB` | `magit-section-toggle` | any section | 07 |
| `C-c TAB` | `magit-section-cycle` | section + children | 07 (stretch) |
| `M-<tab>` / `S-<tab>` | cycle diffs / all | buffer | 07 (stretch) |
| `p` / `n` / `M-p` / `M-n` / `^` | section movement | any | 07 |
| `RET` | `magit-diff-visit-file` | file/hunk/line → blob of containing side; commit → revision buffer; stash → stash diffs | 07 (file), 08 (commit) |
| `C-<return>` | `magit-diff-visit-worktree-file` | file/hunk/line → worktree file | 07 |
| `s` | `magit-stage` | file / hunk / hunk-internal region / multi-section region | 07 |
| `S` | `magit-stage-modified` | all modified files (+ untracked w/ prefix) | 07 |
| `u` | `magit-unstage` | staged file / hunk / region; committed → reverse-in-index | 07 |
| `U` | `magit-unstage-all` | whole index | 07 |
| `k` | `magit-discard` | file/hunk/region (worktree; +index if staged) | 07 (stretch: plan doesn't list it) |
| `a` / `v` | `magit-apply` / `magit-reverse` | hunk/commit change → worktree | 08 (stretch) |
| `d` (+`d d/r/w/s/u/p/c/t`) | `magit-diff` | thing at point | 07/08 |
| `D` | `magit-diff-refresh` | diff args of current buffer | deferred (v1: skip args editing) |
| `+` / `-` / `0` | diff context | diff buffer | 07 (stretch) |
| `j` | jump diffstat↔diff | diffstat file / diff | 07 (stretch) |
| `c c` / `c a` / `c e` / `c w` | commit / amend / extend / reword | staged changes | 08 (v1: `c c` + `c a` at least) |
| `c f/s/A/n/W/F/S` | fixup/squash/alter/augment/revise/instant | commit at point | deferred |
| `C-c C-c` / `C-c C-k` | finish / cancel commit message | commit message buffer | 08 |
| `l l/h/u/o/L/b/a` | `magit-log` variants | — | 08 (v1: `l l` + rev read) |
| `L` | `magit-log-refresh` (args) | log args | deferred |
| `SPC` / `DEL` | show-or-scroll commit | commit in log/status | 08 |
| `C-c C-n` | move to parent | commit in log | 08 (stretch) |
| `=` / `+` / `-` | commit limit toggle/double/half | log buffer | 08 (stretch) |
| `C-c M-g B b` (`C-c f B b`) | `magit-blame` | file-visiting buffer (inline) | 08 |
| `b` (in blame) | recursive blame | blamed buffer | 08 (stretch) |
| `b b` / `b l` | checkout / branch-checkout | — | 08 |
| `b n` / `b c` | create / create+checkout | — | 08 |
| `b k` / `b m` / `b x` | delete / rename / reset | branch at point | 08 (stretch) |
| `z z/i/w/x` | stash variants | — | 08 (v1: `z z`, pop, drop) |
| `z p` / `z a` / `z k` | pop / apply / drop | stash at point | 08 |
| `z v` / `z l` | show / list stashes | stash | 08 (stretch) |
| `g` / `G` | refresh / refresh-all | current / all buffers | 07 |
| `h` | `magit-dispatch` (all commands) | — | 08 (M-x-style Picker equivalent) |
| `q` | bury buffer | any Magit buffer | 07 |
| `H` | `magit-describe-section` | any section | 07 (debug aid) |

## Mapping to redline

**v1 implements** (issues 07/08): status buffer with the section tree (headers,
unstaged, staged, untracked, recent commits — drop stashes/unpulled sections
or keep them read-only), file+hunk stage/unstage, diff view (file → hunk →
line, diffstat optional), inline commit with an editable message buffer
(`C-c C-c` commit / `C-c C-k` abort), log (one-line entries, `RET`/`SPC` opens
commit diff read-only), blame (inline in FileView), branch switch
(checkout/create), stash (list/pop/drop + create).

- **Unstage → git2**: magit unstages a hunk by reverse-applying the hunk's
  patch to the **index only** (worktree untouched); staging applies it
  forward to the index only. git2 has no turnkey patch-apply/reverse-apply —
  see the git2 skill's "Hunk-level staging" section for the building-block
  recipe (compute the hunk's patch text, apply/reverse-apply against the
  index entries, write the index). Every operation must be verified against
  the git CLI in tests (per plan 07).
- **Adopt from magit**: (1) the **section model** — buffers as foldable,
  addressable section trees with per-section-type key handling; (2) **dwim
  keys** — `s`/`u`/`RET` dispatch on the section under point, plus
  region-granularity (hunk-internal region, multi-section region) can be
  deferred; (3) **refresh semantics** — refresh after every mutating
  operation (status + current view), visibility state preserved across
  refreshes, and watcher-driven refresh standing in for magit's
  after-save/auto-revert hooks; (4) the commit-message conventions
  (summary-length warning, confirm on empty second line / overlong summary).
- **Simplify / deviate**: (1) no transient *buffer* — magit's prefix menus
  can be an overlay/Picker over the command registry (same keys, same
  suffixes); (2) no `emacsclient`/`with-editor` round-trip — commit happens
  on `C-c C-c` in the inline message buffer, cancel on `C-c C-k` leaves repo
  state untouched; (3) no buffer-locking/one-buffer-per-repo-per-mode
  machinery — one view per repo is enough; (4) margins (age/author columns)
  optional; (5) no `D`/`L` argument-editing transients in v1 (fixed diff/log
  arguments); (6) no graph rendering in log v1 (plan 08 already defers it —
  note magit's default log *does* show a graph when requested, and renders
  refnames itself: blue local / green remote / boxed current).
- **Plan discrepancies to reconcile**: issue 08 binds the **branch picker to
  `y`** and **blame to `b`** — in real magit `b` is the *branch* transient
  (from status buffer) and blame is `C-c M-g B b` / `C-c f B b` *from a
  file-visiting buffer* (`Y` is the cherries view, not branches). Either
  follow magit (branch=`b`, blame via a file-view prefix) or keep redline's
  binding and document the deviation; don't assume both.
- **Deferred (out of v1)**: rebase (incl. `c F`/`c S` instant fixup/squash and
  `C-c C-e` edit-hunk-commit), cherry-pick/revert, `magit-cherry` (`Y`),
  worktrees, submodules, ediff, forges (forge/ghub), reflog, bisect,
  in-progress-operation sections (merge/rebase/am/sequencer/bisect headers),
  `k` discard (plan 07 doesn't list it; cheap to add since it shares the
  apply-variant machinery), apply/reverse (`a`/`v`).

## Sources

Fetched from the Magit 4.7.1 User Manual (docs.magit.vc/magit), 2026-09:

- https://docs.magit.vc/magit/index.html (TOC; manual version statement)
- https://docs.magit.vc/magit/Getting-Started.html (3 — workflow, `C-x g`, `h`, `C-x M-g`, `C-c M-g`, transient concept)
- https://docs.magit.vc/magit/Modes-and-Buffers.html (4.1 — buffers, refresh `g`/`G`, auto-refresh, save/revert)
- https://docs.magit.vc/magit/Sections.html (4.2 — section model, movement, visibility keys)
- https://docs.magit.vc/magit/Status-Buffer.html (5.1 — status sections, headers, file lists, log sections, options)
- https://docs.magit.vc/magit/Logging.html (5.3 — `l`/`L` transients, log buffer, margin, reflog, cherries)
- https://docs.magit.vc/magit/Diffing.html (5.4 — `d`/`D` transients, context keys, diff commands, options)
- https://docs.magit.vc/magit/Visiting-Files-and-Blobs.html (5.8 — `RET` / `C-<return>` visit semantics)
- https://docs.magit.vc/magit/Blaming.html (5.9 — blame invocation and keys)
- https://docs.magit.vc/magit/Staging-and-Unstaging.html (6.3 — `s`/`S`/`u`/`U`, reverse-in-index)
- https://docs.magit.vc/magit/Applying.html (6.4 — apply variants, `a`/`k`/`v`, `git apply` internals)
- https://docs.magit.vc/magit/Committing.html (6.5 — `c` transient, message buffer, `C-c C-c`/`C-c C-k`, conventions)
- https://docs.magit.vc/magit/Branching.html (6.6 — `b` transient, two remotes, branch variables)
- https://docs.magit.vc/magit/Stashing.html (6.12 — `z` transient, snapshots, apply/pop fallbacks)

Not fetched (out of budget): Transient-Commands.html (4.3),
Completion-Confirmation-and-the-Selection.html (4.5), Conventions/Theming
Faces (10.4.1), Keystroke-Index (App C). Behaviors resting only on those
nodes are marked NOT VERIFIED above. No magit source files were fetched.
