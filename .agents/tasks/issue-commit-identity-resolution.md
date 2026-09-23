# issue-commit-identity-resolution — redline's commit ignores git's identity resolution

**The user's report:** committing through redline's magit fails with
`commit failed: no author identity: set user.name and user.email in git config`, while `git commit`
and **emacs magit** (which shells out to `git commit`) work in the same repository.
*"It should use the same config resolution that git and emacs magit uses."*

## What the code does, and why it diverges

`src/git/commit.rs::author_from_config` reads only the config snapshot:

```rust
let config = self.inner.config()?;
let name = config.get_string("user.name").map_err(|_| GitError::NoAuthor)?;
let email = config.get_string("user.email").map_err(|_| GitError::NoAuthor)?;
```

with a deliberate comment saying it reads config *"rather than the auto-detecting `signature()`, which
falls back to username@hostname and would mask a genuinely unset identity"*.

**That reasoning is the bug.** Git's resolution is a *chain*, and this implements one link of it:

1. `user.name` / `user.email` from the config chain (repo → global → system) — **the only level read**;
2. **`GIT_AUTHOR_NAME` / `GIT_AUTHOR_EMAIL` / `GIT_COMMITTER_*`** — honoured by git, **ignored here**;
3. the **`EMAIL` env / `user@hostname` invention** when nothing is configured — git does it, redline
   errors instead (the very fallback the comment calls "masking");
4. **`user.useConfigOnly = true`** — git's *explicit* way to demand a real identity; unhonoured here.

So any user whose identity arrives by (2) or (3) — a very common setup — sees redline fail where
`git` and magit succeed. A repo with a **local** identity works, which is why this box's own repo
(and the existing tests, which set `user.name` locally) never caught it.

## Requirement

**For the same environment, redline's author and committer identity must equal git's.** Not "read the
same config file" — the same *resolution*.

The oracle is `git var GIT_AUTHOR_IDENT` / `GIT_COMMITTER_IDENT`, which implements exactly the chain
above (and is what magit gets by delegation). Either delegate to it, or implement the chain and
**verify against it**.

Keep the honest failure: when the identity is genuinely unavailable — which git defines as
`useConfigOnly` set with nothing configured — the existing clear `NoAuthor` error is right.

## Acceptance

- **A measured oracle test, per level**: for (a) config-only, (b) `GIT_AUTHOR_*` only, (c) nothing
  configured, and (d) `useConfigOnly` + nothing configured, redline's resolution equals
  `git var GIT_AUTHOR_IDENT` — including (c), where git *invents* an identity and redline must
  produce the same one, and (d), where git fails and redline must fail with `NoAuthor`.
- The `GIT_COMMITTER_*` variables are honoured too (the code uses one signature for both).
- A repository with **only** a global identity commits successfully (the user's case).
- Existing pins keep passing: `commit_moves_head_and_records_message_and_author` and
  `commit_without_author_fails` (the latter must be re-expressed against `useConfigOnly`, since
  "no config" is no longer an error in git).
- `cargo test --workspace`, clippy, `tools/gate.sh full`.

## Fence

`src/git/commit.rs`, `src/git/error.rs` if the error gains a case, tests.

## Note — the process failure that let this ship

`commit_without_author_fails` isolates `HOME`/`GIT_CONFIG_*` and asserts `NoAuthor`, which encoded the
*divergence* as intended behaviour: the test pins what the code does, not what git does. Every commit
test also sets `user.name` **locally** via `git config`, so the global-env and env-var paths were
never exercised. A test whose oracle is the implementation cannot find this class of bug.

---

## Addendum — the cause is probably NOT only the env-var level (investigate before fixing)

Measured on this box: `~/.gitconfig` sets `user.name`/`user.email` **unconditionally** (its only
`includeIf` is `gitdir:~/dev/arena/`), and the repo redline is developed in also sets them locally.
So here `repo.config()` finds an identity and commits work — which means the reporter's failure is
**not** explained by the chain alone. If libgit2 could see their global config, it would have found
`user.name`.

**So the more likely cause is that libgit2 does not read their global config at all** — the app's
process having a different `$HOME`, or `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_NOSYSTEM` set in the launching
environment, or libgit2's **cached** resolved config path (the code's own test comment already warns
about exactly this: *"libgit2 … can also cache the resolved path … regardless of the host's
`~/.gitconfig`"*). `git` in a shell and magit inherit the shell's environment; the app may not.

**Therefore, before implementing the chain, reproduce and identify which level actually fails:**

1. Reproduce the reported case: a repository with **only** a global identity (no local `user.*`) — if
   it commits fine on this box, the bug is environment/config-file selection, not the chain, and the
   fix must address that instead (or as well). Say which it turned out to be.
2. Make the failure **self-diagnosing**, whatever the cause: the current message says only
   *"no author identity: set user.name and user.email in git config"*, which names neither what was
   searched nor what git would have done. It should distinguish at least:
   *identity not in the config chain* from *`useConfigOnly` forbids inventing one*, and — where
   cheap — name the config files consulted. A message that cannot be acted on is how this survived.
3. Then implement the chain (items 1–4 above) and verify against `git var GIT_AUTHOR_IDENT`.

The oracle test stays the same either way: for the same environment redline and git must agree.
