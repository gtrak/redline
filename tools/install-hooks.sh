#!/usr/bin/env bash
# Install the repo's tracked git hooks for this repository.
#
# `core.hooksPath` is the only way to share hooks across clones
# (.git/hooks/ is not versioned), so this points the repository at the
# tracked .githooks/ directory. core.hooksPath is REPO config shared by
# every worktree of this repository (a write from one worktree lands in
# the shared .git/config), so one install covers all worktrees — a push
# from any of them then runs .githooks/pre-push with that worktree's
# checkout (it gates the branch being pushed).
#
# Run it once per CLONE (a fresh clone that has not run it has no hooks).
# It is NOT run automatically by any build step on purpose: rewriting a
# contributor's git config from cargo is intrusive.
set -eu
cd "$(git rev-parse --show-toplevel)"
if [ ! -f .githooks/pre-push ]; then
  echo "install-hooks: .githooks/pre-push not found at $(pwd)" >&2
  exit 1
fi
git config core.hooksPath .githooks
echo "installed: core.hooksPath = $(git config core.hooksPath) (pre-push runs tools/gate.sh fast)"
