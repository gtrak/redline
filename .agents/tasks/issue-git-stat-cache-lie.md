# issue-git-stat-cache-lie — an unstage can make a modified file read as clean

**Found by:** the 017 symbol-identity gate, which reproduced what **four earlier gates** had dismissed
as an unreproducible `git::repo` "flake". **Severity: P2**, pre-existing, out of that lane's fence.

## Reproduced, root-caused, proven deterministic

`stage_file_then_unstage_matches_cli` (`src/git/repo/tests.rs:159`) fails in a plain loop of the
**unmodified** binary: 2/400, 2/600, 1/1200. Isolation passes because the trigger is a **timing
boundary**, not load.

State captured at failure:

```
workdir            = "a\nB\nc\n"                    // the change IS on disk
diff --cached      = ""                             // index == HEAD (correct)
diff               = ""                             // WRONG: git says the workdir is unmodified
status --porcelain = ""                             // WRONG: git says the tree is clean
ls-files -s        = "100644 de98044… 0\ta.txt"     // HEAD's blob
ls-files --debug   = ctime: 1790182294:999154986  mtime: …:999154986  ino: 94395199  size: 6
hash-object a.txt  = 7be73ce3…                      // the modified content
```

The index entry **names HEAD's blob but carries the workdir file's stat cache**. Both failures land at
mtime `…:999003975` / `…:999049069` — the last millisecond of a second, so the index write crosses into
the next second and git's racy-timestamp re-hash never fires.

**Deterministic proof:** forcing the workdir file's mtime 5 s into the past before staging →
**200/200** failures (`status=""` while the file on disk is modified).

**A/B against the CLI** (same scenario, same helper):

```
A (wrapper unstage_file): ctime: …:696867334  mtime: …:696867334  ino: 94505128  size: 6
B (git reset -- a.txt):   ctime: 0:0          mtime: 0:0          ino: 0         size: 0
```

The CLI **zeroes the stat** so git must re-hash; the wrapper copies the workdir stat, producing a
stat-cache lie.

## Root cause and fix

`reset_index_entry_to_head` (`src/git/repo/index_ops.rs:412-435`) does
`index.get_path(...).map(copy_index_entry)` — carrying ctime/mtime/dev/ino/file_size from
`stage_file`'s `add_path` — and then overwrites only `id`/`mode` via `add_frombuffer`.

**Fix:** zero the stat fields on `new_entry` before `add_frombuffer` (or build from
`default_entry(path, mode)` and set `id`) — exactly what `git reset` does.

## Why it matters to a user

The app's Magit status can show a **modified file as clean** after an unstage, and `git status` agrees
with it — so the user sees nothing to review. Same class as the data-loss path the dirty-flag work
guarded: the tool asserts a state that is false.

## Acceptance

- The stat fields are zeroed on the unstage path, pinned (assert `ctime == 0`/`mtime == 0` on the
  entry, or carry the CLI A/B as the test).
- The loop reproducer is green: run `stage_file_then_unstage_matches_cli` in a loop (the gate used
  400–1200 iterations) as the regression net, since a single run passed even while broken.
- **Audit every other `copy_index_entry` caller** for the same lie.

## Process note — this is the lesson, not the bug

Four gates saw this and filed it as an "unreproducible watch-list flake", because a single failing run
followed by passing isolation runs looks exactly like noise. It was a real defect the whole time. The
rule this project already has — *do not call a failure flaky without reproducing it on the base, and
prefer preventing a flake over retrying through it* — is what would have caught it; the missing step
was **deliberately reproducing it in a loop** rather than accepting the isolation pass as an
explanation.
