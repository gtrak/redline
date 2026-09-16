---
name: projectile-ux
description: Authoritative reference for how the Emacs package **projectile** (project management/navigation) actually behaves — project root detection order, `.projectile` marker/ignore semantics, the three indexing methods and file caching, the full documented command/keybinding map, search backend selection (grep/ripgrep/ag), known-projects persistence, and per-project-type compile/test commands. Use this when implementing or reviewing redline's project layer (plan issues 02 and 06) so behavior mirrors current upstream projectile rather than training-data memory of old versions.
---

# projectile-ux

In-repo reference for redline's project layer. Every claim below was verified
against the official projectile documentation site on the fetch dates noted in
**Sources**; anything not present in the fetched pages is explicitly marked
**NOT VERIFIED** and must not be treated as fact.

## Docs version

- Source: **docs.projectile.mx**, the "**default**" channel (current master,
  i.e. post-3.4 — the version selector on every page lists 3.4, 3.3, … 2.2 and
  "default" as the latest). Copyright line reads 2011–2025.
- The docs write all keybindings with an `s-p` prefix and state verbatim:
  "Projectile doesn't have a default key prefix for its commands, but all the
  examples in the manual assume you've opted for `s-p` (`super`-p)." The
  configuration example in the docs binds the command map to `C-c p`
  (`(global-set-key (kbd "C-c p") 'projectile-command-map)`), so every `s-p X`
  entry below reads `C-c p X` for redline.
- Features from specific releases are flagged in the docs: frecency ranking
  (3.1+), automatic cache watches (3.2, "experimental"), gitignore-pattern
  ignore semantics (3.3+). The documented behavior below is the current
  master behavior, which **differs from many older projectile versions**
  (see "differs from common assumptions" in the output summary).

## Project detection

### The documented detection order

Projectile runs a list of root functions in order
(`projectile-project-root-functions`); the first to return a directory wins.
Default list, in precedence order:

1. **`projectile-root-local`** — buffer-local variable
   `projectile-project-root`, typically set via `.dir-locals.el`
   (`;; -*- projectile-project-root: "/path/to/other/project/" -*-`).
   Takes precedence over everything. Read fresh on every lookup, **not**
   cached, so two buffers in the same directory can resolve to different roots.
2. **`projectile-root-marked`** — looks for a `.projectile` file (or the
   value of `projectile-dirconfig-file`). "The idea is that normally if you
   have a `.projectile` file you'd like it to override the normal project root
   discovery logic." Runs **before** VCS discovery: an outer `.projectile`
   makes the whole tree one project even if nearer `.git` directories exist.
3. **`projectile-root-bottom-up`** — walks **up** from `default-directory`,
   matching only **VCS markers** (the list
   `projectile-project-root-files-bottom-up` holds only VCS markers). The
   **nearest (bottom-most)** match wins. Matches both files and directories —
   `.git` is a directory in normal repos but a *file* in worktrees/submodules,
   so it belongs on this list. In a monorepo where a `.git` sits at the top and
   a language manifest lives in a subdirectory, **the enclosing repository
   wins**.
4. **`projectile-root-top-down`** — walks **down** from the filesystem root;
   returns the **top-most (farthest)** match. Configured by
   `projectile-project-root-files`, auto-populated from the registered project
   types (each type's `:project-file`, or first marker file). This is where
   per-language manifests (`deps.edn`, `Cargo.toml`, `pom.xml`, …) actually
   get matched. Matches **regular files only** — directories with listed names
   are skipped.
5. **`projectile-root-top-down-recurring`** — markers that can appear at every
   level (e.g. `Makefile`, `.svn`); returns the top-most match.

So the effective order is: **buffer-local override → `.projectile` → nearest
VCS root → outermost language manifest → recurring markers**.

### VCSes recognized out of the box

Git, Mercurial, Bazaar, Subversion, CVS, Fossil, Darcs, Sapling, Jujutsu.
Markers live in `projectile-vcs-markers` (ordered alist, customizable since
3.1); during the upward search the **nearest marker directory always wins**;
the alist order only breaks ties for markers in the *same* directory (e.g.
`jj git init` creates both `.jj` and `.git`; default detects `git`).

### The `.projectile` file: marker vs ignore-file semantics

- An **empty** `.projectile` is a pure project marker (the docs call the file
  a "project marker").
- The same file "serves both as a project marker and a configuration file" —
  when it contains patterns it is a **dirconfig** (ignore/keep rules, see
  Caching/Ignoring below). It is treated as an ignore list whenever it has
  `-`/`+`/`!` pattern lines (all three indexing methods honor them); lines
  with **no prefix** are still accepted as ignore patterns for backward
  compatibility, but "the implicit form is being phased out and Projectile now
  warns about it once per project."
- Because `projectile-root-marked` runs before VCS detection, `.projectile`
  wins over `.git` at the same or enclosing level.

### Nested repositories

A repo checked out inside another project (vendored dep, submodule, second
clone) is ambiguous; the documented default behavior follows the ordering:
the outer `.projectile` wins over any inner `.git`. To make the nested repo
its own project: drop an empty `.projectile` into it (the **nearest**
`.projectile` wins), or reorder
`projectile-project-root-functions` to put `projectile-root-bottom-up` before
`projectile-root-marked` (then `.projectile` only takes effect where no VCS
root sits closer).

### Outside any project

Default: invoking a projectile command outside a project **prompts you for a
project to switch to** (controlled by `projectile-require-project-root`).
`t` → raise an error outside project folders; `nil` → the current directory
is treated as the project root.

### Project-root cache

`projectile-project-root-cache` memoizes every root function's result, keyed
on the search start directory. Positive entries are revalidated against the
filesystem (`file-exists-p`) on every lookup. **Negative entries (no project
found) are also memoized** — "the main source of confusion when adding a
marker file: Projectile remembers that the directory was rootless and won't
notice the new marker until you invalidate the cache." Cleared by
`projectile-invalidate-cache` (`s-p i`), `projectile-invalidate-cache-all`,
`projectile-discard-root-cache` (root cache only), or Emacs restart.

## Caching

### Indexing methods (how the file list is produced)

Three methods, set by `projectile-indexing-method`:

| | `native` | `hybrid` | `alien` |
|---|---|---|---|
| What it is | pure-Elisp directory walk | external command + post-process pass | external command only |
| Speed | Slow | Fast | Fastest |
| Honors `.projectile` `-`/`+`/`!` | yes | yes | yes |
| Honors global ignore files/dirs/suffixes | yes | yes | yes |
| Honors `projectile-globally-ignored-file-regexps` (Emacs regexps) | yes | no | no |
| Re-adds files the VCS itself ignores | n/a | yes (VCS-aware unignore pass) | no |
| Sorts via `projectile-sort-order` | yes | yes | no (tool's order) |
| File caching enabled by default | **yes** | no | no |
| Windows out of the box | yes | needs Unix utils | needs Unix utils |

- **Default: `alien` on all operating systems except Windows** (native there).
- **Alien**: shells out; for VCS projects it uses the VCS itself. Git command:
  `git ls-files -zco --exclude-standard` (relative, 0-delimited output).
  Non-VCS projects: `fd` if installed
  (`fd . -0 --type f --color=never --strip-cwd-prefix`), else `find`
  (`find . -type f | cut -c3- | tr '\n' '\0'`; the `find` fallback does **not**
  exclude build dirs itself). By default **`fd` is also used inside git
  repos** instead of `git ls-files` (because `git ls-files` lists deleted
  files until staged); disable with `projectile-git-use-fd` nil.
- Ignore rules under `alien` are **pushed into the tool** where possible:
  git via `:(exclude,glob)` pathspecs, `fd` via repeated `--exclude` globs;
  tools that can't take exclusions (hg, svn, fossil, bzr, darcs, pijul, plain
  find) get their output filtered in Elisp. A project with `!` unignore
  entries gives up the push-down and is filtered in Elisp.
  `projectile-alien-honors-ignores` nil → raw listing, defer to the tool's own
  `.gitignore` etc. Ignore matching is **case-sensitive** in every method.
- **Hybrid**: same command as alien plus a second pass applying dirconfig,
  global ignore/unignore variables, and sort order.
- **Native**: the only method that honors Emacs-regex global ignores.

### File-list caching

- Caching is **enabled by default only for native indexing**; force with
  `(setq projectile-enable-caching t)`.
- `C-u s-p f` invalidates the cache before prompting.
- `s-p z` (`projectile-cache-file`) adds the currently visited file to the
  cache; files created outside Emacs are added automatically the first time
  they're opened (ignored files are never added).
- With `projectile-mode`, the cache **auto-updates via file hooks** when
  files are added/deleted *inside Emacs* (`projectile-auto-update-cache`);
  this does **not** see external changes (git pull, other editors) —
  **NOT VERIFIED that any cache invalidation is automatic on VCS/branch
  switch**; the only documented automatic external mechanism is the
  experimental watch feature below, otherwise the cache stays stale until
  manual invalidation.
- The project cache auto-invalidates if `.projectile`'s mtime is newer than
  the cache file's mtime.
- TTL: `projectile-files-cache-expire` (default `nil` = never expires).
- Persistent mode: `projectile-enable-caching` `'persistent` → one cache file
  per project in the project root, named `.projectile-cache.eld` by default
  (since 2.9; previously one shared file).
- Purge: `projectile-purge-file-from-cache`, `projectile-purge-dir-from-cache`;
  `projectile-invalidate-cache-all` for many stale projects at once.
- **Experimental (3.2+)**: `projectile-auto-update-cache-with-watches`
  registers `file-notify` watches per cached directory; events are debounced
  (a branch switch touching hundreds of files folds into one update),
  unapplyable events fall back to full invalidation. Limits: one watch per
  directory (512-directory default cap), no TRAMP, alien/hybrid also consult
  the VCS so `.gitignore`d files never appear.
- **Background indexing**: alien/hybrid index asynchronously by default
  (`projectile-async-indexing`), C-g-live and abortable; native indexes
  synchronously. `projectile-index-project-async` warms the cache in the
  background (docs suggest hooking it to `projectile-after-switch-project-hook`).

### Sort order & frecency

- `projectile-sort-order`: `default` (none), `recentf`, `recently-active`,
  `modification-time`, `access-time`, or a function. Not applied under alien.
- **Frecency (3.1+)**: `projectile-find-file` ranks candidates by visit
  frequency + recency with a **half-life of two weeks**; never-visited files
  keep their original order after the ranked ones. Persisted across sessions
  in `projectile-frecency-file`, capped per project by
  `projectile-frecency-max-files` (200 default), applied via completion
  metadata so it works under every indexing method. Disable:
  `projectile-enable-frecency` nil.

### `.projectile` (dirconfig) ignore syntax — documented exactly

All three indexing methods honor the three entry kinds. Patterns are
**gitignore patterns matched against paths relative to the project root**;
matching is case-sensitive.

- Ignore: `-` prefix. `- /log` (leading slash) anchors at project root;
  `-tmp`, `-*.rb`, `-models` match at any depth.
- Keep (restrict to subdirectories only — not file patterns): `+` prefix,
  e.g. `+/src/foo`, `+/tests/foo`. Keep entries apply **first**, restricting
  the file set; ignore entries then apply to that set.
- Unignore: `!` prefix, e.g. `!/src/foo`, `!*.yml`. Overriding a directory
  leaves its contents subject to ignore patterns; use the full path with `!`
  to rescue files too.
- Pattern language (one shared matcher): no-slash pattern matches a file name
  or any directory segment at any depth (won't match `xlog` for `-log`);
  a slash anchors at the root; `**/` prefix matches at any depth; trailing
  `/` restricts to directories; a matched directory covers its whole
  subtree; `**` matches within one segment while `*` crosses `/`; `?` one
  char; `[...]`/`[!...]` classes supported.
- Comments: set `projectile-dirconfig-comment-prefix` (e.g. `#`); leading
  whitespace before `+`/`-`/`!` is skipped.

Documented worked examples (verbatim from the docs):

```
-/log
-/tmp
-/vendor
-/public/uploads
```
→ ignores those folders only at the project root.

```
-tmp
-*.rb
-*.yml
-models
```
→ matches at any depth.

Monorepo example: `+/packages/core`, `+/packages/web`, `-dist/`,
`-node_modules/` yields only the two packages' files (keep first, then
any-depth ignores).

### Global ignore/unignore variables

- `projectile-globally-ignored-files` (names, any depth),
  `projectile-globally-ignored-file-suffixes`,
  `projectile-globally-ignored-directories` (gitignore patterns; defaults
  cover editor/VCS dirs plus common dependency/build output: `node_modules`,
  `target`, `*pycache*`, `.venv`, `.next`, `.terraform`, …; `vendor`, `dist`,
  `public`, `build` deliberately excluded),
  `projectile-globally-ignored-file-regexps` (Emacs regexps on absolute
  names — **native indexing only**),
  `projectile-globally-unignored-files` / `-directories`.

## Commands & keybindings

Full table as documented (prefix `s-p` = redline's `C-c p`). This is the
current master map; several keys **moved** relative to old projectile
versions (noted where redline's plan assumed the old key).

| Key | Command | Documented behavior |
|---|---|---|
| `s-p f` | `projectile-find-file` | List all files in the project (frecency-ranked); prefix arg clears cache first |
| `s-p F` | `projectile-find-file-in-known-projects` | All files in all known projects |
| `s-p g` | `projectile-find-file-dwim` | Files at point in the project; prefix clears cache |
| `s-p d` | `projectile-find-dir` | List all directories in the project; prefix clears cache |
| `s-p T` | `projectile-find-test-files` | List all test files (specs, features, etc.) |
| `s-p l` | `projectile-find-file-in-directory` | All files in a (not necessarily project) directory |
| `s-p C` | `projectile-find-changed-file` | Staged/unstaged/untracked files; prefix: diff vs a chosen revision. Git only |
| `s-p e` | `projectile-recentf` | **Recently visited project files** (note: recent files is `e`, NOT `r`) |
| `s-p b` | `projectile-switch-to-buffer` | List all project buffers currently open |
| `s-p o` | `projectile-multi-occur` | multi-occur on all open project buffers |
| `s-p a` | `projectile-find-other-file` | Same base name, different extension (`foo.h` ↔ `foo.c`) |
| `s-p t` | `projectile-toggle-between-implementation-and-test` | Toggle impl ↔ test file |
| `s-p j` / `s-p J` | `projectile-find-file-of-kind` / `projectile-toggle-related-file` | Find file of a kind (Rails model/controller) / jump to related files |
| `s-p s s` | `projectile-search` | **Search with the configured backend** (see Search integration); prefix arg = regexp search |
| `s-p s g` | `projectile-grep` | grep on project files (M-prefix: grep `projectile-grep-default-files` only) |
| `s-p s r` | `projectile-ripgrep` | Run `rg` on the project, literal; prefix = regex. Requires `rg.el`/`ripgrep.el` |
| `s-p s a` | `projectile-ag` | Run `ag` on the project, literal; prefix = regex. Requires `ag.el` |
| `s-p s x` | `projectile-find-references` | References to symbol at point (cheatsheet: "uses internally the xref library"; usage page describes it as a backend-agnostic **textual** search honoring ignore config) |
| `s-p s R` / `s-p s X` | `projectile-search-review` / `-regexp-review` | Read-only reviewable search buffer (see below) |
| `s-p s t` | `projectile-todos` | Collect `TODO`/`FIXME`/… annotations (keywords: TODO FIXME HACK XXX BUG NOTE; prefix prompts which) |
| `s-p r` | `projectile-replace` | **Interactive query-replace on all project files** (note: `r` is replace, not recent) |
| `s-p R` | `projectile-replace-review` | Reviewable replace with preview/toggle; `s-p u` undoes the last applied replace |
| `s-p i` | `projectile-invalidate-cache` | Invalidate the project (and root) cache |
| `s-p k` | `projectile-kill-buffers` | Kill all project buffers (filter: `projectile-kill-buffers-filter`) |
| `s-p D` | `projectile-dired` | Open the project root in dired |
| `s-p E` | `projectile-edit-dir-locals` | Open/create the root `.dir-locals.el` |
| `s-p !` / `s-p &` | `projectile-shell` / `projectile-async-shell` | `shell-command` / `async-shell-command` in the project root |
| `s-p c o` | `projectile-configure-project` | Standard configure command for the project type |
| `s-p c c` | `projectile-compile-project` | Standard compile command for the project type |
| `s-p c t` | `projectile-test-project` | Standard **test command** for the project type |
| `s-p c .` | `projectile-run-test-at-point` | Run the test at point (tree-sitter; Emacs 29+) |
| `s-p c i` | `projectile-install-project` | Standard install command |
| `s-p c p` | `projectile-package-project` | Standard package command |
| `s-p c r` | `projectile-run-project` | Standard run command |
| `s-p c x` / `s-p c X` | `projectile-run-task` / `projectile-repeat-last-task` | Run a named task / re-run last task |
| `s-p c m …` | subproject variants | `c m f` find file in subproject; `c m o/c/t/i/p/r` run the lifecycle command in the **nearest** subproject (monorepos) |
| `s-p p` | `projectile-switch-project` | List **known projects** to switch to (see switcher behavior) |
| `s-p q` | `projectile-switch-open-project` | List **open** projects |
| `s-p W` | `projectile-switch-worktree` | Other checkouts of the current repo (git worktrees, jj workspaces, clones) |
| `s-p n p` | `projectile-switch-sibling-project` | Projects related to the current one |
| `s-p S` | `projectile-save-project-buffers` | Save all project buffers |
| `s-p m` | `projectile-dispatch` | `transient` dispatch menu mirroring the command map |
| `s-p P` | `projectile-dashboard` | Project summary (name, type, file count, branch, recent files, tasks, lifecycle commands), all actionable |
| `s-p H` | `projectile-doctor` | Diagnostic report on how Projectile sees the project |
| `s-p x r` | `projectile-run` | Shell/REPL/terminal in project root via pluggable backend (default `eshell`; also shell, term, vterm, eat, ghostel, gdb, ielm — `s-p x e/i/t/s/g/v/x/G`) |
| `s-p B s/j/d` | project bookmarks | Set/jump/delete bookmarks scoped to the project (built on `bookmark.el`, names prefixed `project-name: `) |
| `s-p w …` | sessions | Save/restore per-project sessions (separate `sessions` doc page) |
| `s-p left` / `s-p right` | previous/next project buffer | Buffer cycling |
| `s-p 4 4` / `s-p 5 5` | other-window/frame prefix | Next command displays in another window/frame (like `C-x 4 4`) |
| `s-p z` | `projectile-cache-file` | Add current file to the project cache |
| `s-p ESC` | `projectile-switch-to-recently-selected-buffer` | Most recently selected projectile buffer |
| `s-p C-h` | help | Keybinding help |

### Project switcher behavior (`s-p p`)

- Prompts over **known projects** (all projects ever visited, persisted —
  see Behaviors worth copying); the current project is excluded by default
  (`projectile-current-project-on-switch` to include it).
- On selection, runs `projectile-switch-project-action` — **default is
  `projectile-find-file`**, i.e. you stay in completion and pick a file to
  visit in the new project (not dired, not recent files). Alternatives
  documented: `projectile-dispatch` (the menu), `projectile-dashboard`,
  `projectile-dired`, `projectile-find-dir`,
  `projectile-find-file-in-known-projects`, `projectile-find-file-dwim`.
- `C-u s-p p` opens the dispatch menu after selecting a project.
- `s-p 4 p` / `s-p 5 p` switch and show in another window/frame (running
  `projectile-switch-project-other-window-action` / `-other-frame-action`,
  default find-file there).

## Search integration

### Backend selection

`projectile-search` (`s-p s s`) uses a **pluggable backend** selected by
`projectile-search-backend`. Three built-in backends: `grep`, `ripgrep`, `ag`.
Default `'auto`: "picks the first available backend, **favouring ripgrep,
then grep (which is always available)**". You can name a specific backend or
use `prompt'`. So the documented availability order today is **rg → grep** —
`ag` is *not* in the auto chain; it's only reached via the dedicated
`s-p s a` command. (Custom backends register via
`projectile-register-search-backend`.) The dedicated commands
`projectile-grep` (`s-p s g`), `projectile-ripgrep` (`s-p s r`),
`projectile-ag` (`s-p s a`) are "just `projectile-search` with a forced
backend".

- `grep` backend: in Git projects, `projectile-use-git-grep` t uses
  `vc-git-grep` instead of `rgrep` (faster, respects `.gitignore`).
- `rg` requires the Emacs package `rg.el` or `ripgrep.el`; `ag` requires
  `ag.el`. How their results are displayed: the docs say these commands "run
  `rg`/`ag` on the project" and rely on those packages; the packages'
  grep-mode results machinery is implied but the display details are **NOT
  VERIFIED** from the fetched pages.
- All search commands apply the same ignore patterns as indexing, translated
  to the tool's exclusion syntax (the one inexact translation: under a
  `find`-based grep, a `*` in a root-anchored pattern crosses `/`).

### Reviewable search buffer (`s-p s R` / `s-p s X`)

- Prompts for the term (defaulting to symbol/region at point), gathers every
  match into a **read-only `*projectile-search*` buffer**, **grouped by
  file**, one `LINE:COL: CONTEXT` line per match, matched span highlighted.
- Keys: `RET` visit match in another window; `n`/`p` next/prev match;
  `M-n`/`M-p` next/prev file; `c` toggle case; `x` toggle literal/regexp;
  `k`/`d` keep/flush matches by line regexp; `K`/`D` by file regexp; `g`
  re-run (undoes filtering); `r` hand to the replace reviewer; `e` export to
  a `grep-mode` buffer (wgrep / `grep-edit-mode` bridge); `q` quit.
- **Fast path**: when `projectile-search-use-ripgrep` is non-nil (default)
  and `rg` is installed, a *literal* search runs through ripgrep, streaming
  into the same buffer; it follows ripgrep's own ignore rules plus
  Projectile's ignore globs passed via `--glob`. Regexp searches always use
  the pure-Elisp scan (ripgrep regex ≠ Emacs regexp); non-UTF-8 files are
  skipped by the rg path but found by the elisp path. Scan is asynchronous,
  cancelable (`q`, `C-g`, kill buffer), capped by
  `projectile-search-max-matches`.

### References & symbols

- `projectile-find-references` (`s-p ?` or `s-p s x`): "backend-agnostic
  **textual** search: it greps the project for the symbol, scoped to the
  project root and honouring Projectile's ignore configuration". The
  cheatsheet says it "uses internally the `xref` library". Docs advise: use
  it when you don't have a language server or tags table; otherwise
  `xref-find-references` (`M-?`) gives semantic results (and is scoped to the
  projectile project via the project.el integration).
- **ctags/etags / `projectile-regenerate-tags`**: NOT VERIFIED — no tags
  command appears anywhere on the fetched pages (index, projects, usage,
  tasks, configuration, indexing, ignoring, cheatsheet). Do not assume the
  old tags backend exists in current projectile.
- **Test at point** (`s-p c .`): uses the buffer's **tree-sitter** parse tree
  to find the test enclosing point (Emacs 29+ with tree-sitter,
  tree-sitter major mode). Documented per-language rules:
  python-ts (`python -m pytest FILE::NAME`), go-ts (`go test -run '^NAME$'
  ./DIR`), js/ts/tsx-ts (jest: `npx jest FILE -t 'NAME'`), ruby-ts
  (RSpec `bundle exec rspec FILE -e 'NAME'` or Minitest, per project type),
  rust-ts (`#[test]` → `cargo test -- --exact NAME`), elixir-ts (`mix test
  FILE:LINE`), java-ts (`mvn test -Dtest=Class#NAME` or `./gradlew --tests`),
  erlang-ts (`rebar3 eunit --test=module:NAME`), fsharp-ts
  (`dotnet test --filter FullyQualifiedName~NAME`). Extensible via
  `projectile-test-at-point-rules` (alist keyed by major mode with
  `:node-types`, `:name-fn`, `:command-fn`).

## Behaviors worth copying

- **Per-project configuration**: standard `.dir-locals.el` at the project
  root; documented per-project vars include `projectile-project-configure-cmd`,
  `projectile-project-compilation-cmd`, `projectile-project-test-cmd`,
  `-install-cmd`, `-package-cmd`, `-run-cmd`, test prefix/suffix,
  `projectile-project-name`, `projectile-enable-caching`,
  `projectile-tasks`, `projectile-project-root`. `s-p E` opens/creates the
  file. Commands are cached per project (`projectile-discard-command-cache`
  to re-read `.dir-locals.el`).
- **Known-projects persistence**: stored in a file on disk,
  `projectile-known-projects-file` (default
  `~/.emacs.d/projectile-bookmarks.eld` — a bookmarks-file name, but *not*
  Emacs bookmarks). Projects are added automatically when you visit a file in
  a project (`projectile-track-known-projects-automatically`); exclusion via
  `projectile-ignored-projects` (exact paths),
  `projectile-ignored-project-patterns` (regexps),
  `projectile-ignored-project-function` (predicate). A corrupt file is moved
  aside with a `.corrupt` suffix rather than overwritten.
- **Automatic discovery**: `projectile-project-search-path` (list of dirs,
  cons cell `(DIR . DEPTH)` for recursive depth) is scanned **the first time
  a project-switching command runs in a session** (not on every switch);
  `projectile-auto-discover-projects` (default on); TRAMP entries skipped.
  `projectile-cleanup-known-projects` (alias
  `projectile-forget-zombie-projects`) removes missing projects;
  `projectile-auto-cleanup-known-projects` for automatic.
- **Compile/test/run commands per project type**: each registered project
  type carries `:compile`, `:test`, `:run`, `:configure`, `:install`,
  `:package`, `:compilation-dir`. Mappings shown in the fetched docs (the full
  built-in table for all ~90 types is NOT VERIFIED from these pages):
  - npm: compile `npm install`, test `npm test`, run `npm start`
  - ruby-rspec: compile `bundle exec rake`, test `bundle exec rspec`
  - ruby-test: compile `bundle exec rake`, test `bundle exec rake test`
  - rails-test: compile `bundle exec rails server`, test `bundle exec rake test`
  - rails-rspec: compile `bundle exec rails server`, test `bundle exec rspec`
  - dotnet: compile `dotnet build`, run `dotnet run`, test `dotnet test`
  - Per-type `:test-suffix`/`:test-prefix`/`:src-dir`/`:test-dir` drive
    impl/test toggling (defaults: `src/` and `test/`).
- **Test heuristics**: per-type test command (above) + tree-sitter
  test-at-point rules (above) + test-file listing (`s-p T`).
- **Frecency ranking** of find-file candidates (half-life 2 weeks, persisted,
  200-file cap per project).
- **Mode-line indicator**: ` Projectile[ProjectName:ProjectType]`
  (`projectile-mode-line-prefix`, `projectile-dynamic-mode-line`,
  `projectile-mode-line-function`); suppressed for remote/non-file buffers.
- **Command history**: per-type (configure/compile/test/install/package/run)
  command histories scoped to the **repository** (shared across worktrees),
  `M-p` cycles them; consecutive duplicates ignored by default.
- **Dashboard** (`s-p P`): cheap project summary, never indexes; usable as a
  switch action.
- **project.el integration**: projectile registers as a `project.el` backend
  (root, files, name, buffers, ignores), so `xref` and LSP tooling see the
  same project.

## Key quick-reference

Redline-relevant subset, verified keys → commands → redline issue:

| Key (redline) | Projectile command | Verified behavior | Redline issue |
|---|---|---|---|
| `C-c p f` | `projectile-find-file` | Project file list, frecency-ranked, ignore-aware; prefix = re-index | 02 |
| `C-c p p` | `projectile-switch-project` | Known-projects prompt; on select → find-file in new project (default action) | 02 |
| `C-c p q` | `projectile-switch-open-project` | Prompt over currently open projects | 02 |
| `C-c p e` | `projectile-recentf` | Recently visited project files | 02 |
| `C-c p b` | `projectile-switch-to-buffer` | List open project buffers | 02 |
| `C-c p k` | `projectile-kill-buffers` | Kill all project buffers | 02 |
| `C-c p d` | `projectile-find-dir` | Project directory list | 02 |
| `C-c p D` | `projectile-dired` | Open project root (dired) | 02 (optional) |
| `C-c p i` | `projectile-invalidate-cache` | Rebuild file cache + root cache | 02 |
| `C-c p s s` | `projectile-search` | Project-wide search, backend auto = ripgrep → grep; prefix = regexp | 06 |
| `C-c p s g` | `projectile-grep` | grep backend (git-grep in git repos if enabled) | 06 |
| `C-c p s r` | `projectile-ripgrep` | rg literal (prefix = regex) | 06 |
| `C-c p s a` | `projectile-ag` | ag literal (prefix = regex) | 06 (skip — redline embeds rg) |
| `C-c p s x` | `projectile-find-references` | Textual symbol search at point, ignore-aware | 06 (`M-?`) |
| `C-c p s R` | `projectile-search-review` | Read-only grouped results buffer, cancelable | 06 (results view) |
| `C-c p r` | `projectile-replace` | Query-replace across project files | out of v1 scope |
| `C-c p E` | `projectile-edit-dir-locals` | Edit root `.dir-locals.el` | out of v1 scope |
| `C-c p c c` / `c t` / `c r` | compile/test/run project | Per-type lifecycle commands | out of v1 scope |
| `M-?` | `xref-find-references` | Semantic refs (via LSP/tags); projectile's textual fallback is `s-p s x` | 06 |

## Mapping to redline

Redline adopts (from this doc):

- **`C-c p` prefix** with `f` (find-file), `p` (switch-project), `s s`
  (search), `e` (recent files), `b` (buffers), `q` (open projects) — all
  keys verified against the current docs above.
- **Switch-project semantics**: prompt over the persisted known-project
  registry; after selection, land in the file picker for that project
  (projectile's default `projectile-switch-project-action` is
  `projectile-find-file` — redline's picker is the analogue).
- **Ignore-aware file listing** with gitignore-pattern semantics
  (anchored vs any-depth patterns, `+` keep, `!` unignore).
- **Frecency-style ranking** of file candidates (visit frequency + recency,
  persisted) — matches the plan's recents persistence under
  `~/.cache/redline/`.

Explicit deviations (redline vs projectile, documented here on purpose):

1. **Root detection order**. Projectile: buffer-local → `.projectile`
   (marked, wins over VCS) → nearest VCS marker (bottom-up, VCS-only) →
   outermost language manifest (top-down) → recurring markers. Redline (plan
   02): **innermost git root first**, else marker-file scan
   (`.git`, `Cargo.toml`, `package.json`, `pyproject.toml`, `.projectile`,
   …). Consequences: in redline a git repo always wins over a `.projectile`
   in a subdirectory (opposite of projectile's marked-first rule), and
   redline prefers the *nearest* git root rather than projectile's
   top-down manifest matching.
2. **Outside a project**: projectile's default prompts for a project to
   switch to (or errors with `projectile-require-project-root` t; with nil
   treats cwd as root). Redline **requires a root** and falls back to marker
   scan — no "current dir is the project" mode.
3. **No file-list cache of projectile's shape**: redline walks with the
   `ignore` crate (`.gitignore`-aware) and caches per project under
   `~/.cache/redline/`, refreshed on demand (auto in issue 04), instead of
   projectile's in-project `.projectile-cache.eld` + file hooks + optional
   watches. Redline does not implement alien/hybrid/native methods or
   `git ls-files`-based listing.
4. **Search backend**: projectile's documented auto order is
   **ripgrep → grep** (ag only via the dedicated `s-p s a` command; the
   "ag → rg → grep" order is an older-version assumption and is NOT what
   current docs say). Redline embeds the ripgrep crates, so redline's
   rg-first behavior matches projectile's current auto default; redline has
   no grep/rgs PATH dependence and no ag backend.
5. **No ctags/etags**: the current docs contain no tags backend (NOT
   VERIFIED as existing at all); redline uses tree-sitter for symbols
   (issue 05) and embedded-rg + tree-sitter token filtering for references
   (issue 06), mirroring projectile's *textual* `projectile-find-references`
   rather than any tags table.
6. **Recent-files key**: current docs bind recent files to `s-p e` and
   replace to `s-p r`. If redline historically planned `C-c p r` = recent
   files, that is a deviation — the verified current mapping is `C-c p e`
   (recent) / `C-c p r` (replace).
7. **No Emacs-specific machinery**: dired, multi-occur, dispatch/transient
   menu, sessions, bookmarks, per-project compilation buffers, and the
   project.el integration have no redline equivalents in v1; redline's
   Picker (nucleo) replaces `completing-read`+vertico, and the mode-line
   indicator becomes the status-line project name (plan 02 step 1).
8. **Magit key conflict check**: the in-repo magit skill's only `C-c C-p`
   binding is the `Reported-by` pseudo-header inserter inside the `git
   with-editor` commit-message buffer — a context-local key in a commit
   editor, a different sequence from the global `C-c p` prefix. **No
   conflict** with redline's `C-c p` project prefix.

## Sources

Fetched 2026-09-16 from the official docs site (all "default" channel pages):

- https://docs.projectile.mx/projectile/index.html (home/TOC; version selector shows 3.4…2.2 + default)
- https://docs.projectile.mx/projectile/projects.html (detection order, VCS list, marker tables, `.projectile` marker role, nested repos, root cache, dir-locals, project buffers)
- https://docs.projectile.mx/projectile/usage.html (basic usage keys, known-project discovery/cleanup, reviewable replace/search buffers, todos, bookmarks, dispatch menu, dashboard, project.el/xref integration, `C-c p` binding example)
- https://docs.projectile.mx/projectile/ignoring.html (dirconfig `-`/`+`/`!` syntax, pattern language, worked examples, global ignore/unignore variables)
- https://docs.projectile.mx/projectile/indexing.html (native/hybrid/alien methods, git/fd/find commands, alien ignore push-down, sorting, frecency, caching, watches, background indexing)
- https://docs.projectile.mx/projectile/cheatsheet.html (full keybinding reference)
- https://docs.projectile.mx/projectile/tasks.html (lifecycle commands, per-type command examples, named tasks, subprojects, test-at-point tree-sitter rules)
- https://docs.projectile.mx/projectile/configuration.html (outside-project behavior, switch actions, known-projects file, completion, search backends incl. auto = ripgrep → grep, shells, mode line, project name, hooks)

Not fetched (out of budget / not needed): sessions, across_repositories,
faq, troubleshooting, installation, getting_started, extensions,
configuration_index, migrating, upgrading pages, and the GitHub README.
