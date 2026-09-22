#!/usr/bin/env python3
"""Reset the <root>/redline_pyte_repo fixture to a known baseline so the sweep is
reproducible regardless of prior (possibly state-mutating) runs.

The fixture root is REDLINE_FIXTURE_ROOT (default /tmp): every tools/ fixture
path resolves as <root>/<basename> via repo(), so a battery invocation can run
against a private tree (tools/gate.sh defaults the root to /tmp/fx<pid>;
tools/pool.py points each lane at its own dir).

Baseline:
  * src/lib.rs  — working-tree change, STAGED   (git diff --cached shows it)
  * README.md   — working-tree change, UNSTAGED
  * no untracked .redline-notes.md (the notes flow creates it; sweep removes it)

Run directly to reset:  python3 tools/fixture.py
Or import:               from fixture import reset; reset()
"""
import os
import shutil
import subprocess

def fixture_root():
    """Fixture tree root: REDLINE_FIXTURE_ROOT, default /tmp. Keep roots SHORT
    (fixture paths land on the 80-col status line)."""
    return os.environ.get("REDLINE_FIXTURE_ROOT", "/tmp")


def repo(name):
    """<root>/<name>: the single path indirection for every fixture repo.
    Basenames are fixed — suites assert on them."""
    return os.path.join(fixture_root(), name)


REPO = repo("redline_pyte_repo")


def _git(*args):
    subprocess.run(["git", "-C", REPO, *args], check=True,
                   capture_output=True, text=True)


def reset():
    # Restore ALL tracked files to HEAD first: an edit-mode PTY leg (or a
    # manual probe) can save real edits into a fixture file, and a stale
    # tracked file silently changes what every later flow renders. The
    # markers below are re-applied afterwards (they are working-tree edits
    # on top of HEAD, not commits).
    _git("checkout", "--", ".")
    # Drop untracked leg/scratch files the PTY suites create, so the tree is
    # exactly the baseline (see docs/ux-testing-plan.md backlog #8/#12).
    for stray in ("src/leg.rs", "src/wordleg.rs", "src/cursorleg.rs",
                  "src/whichfn.rs", "src/wideleg.rs", "wideleg.rs",
                  # drive_xref's jump legs (L5 external landing, L6 mid-window
                  # landing) create these; a `timeout`-killed run skips their
                  # finally, and untracked leftovers perturb every later
                  # suite's tree/status/file-listing renders (backlog-#12).
                  "src/call.rs", "src/long.rs",
                  # drive_xref's L7 jump-column pins (jump-column-pty):
                  "src/col_a.rs", "src/col_b.rs", "src/col_b_use.rs",
                  "src/col_c.rs", "src/col_d.rs",
                  # drive_xref's external-landing leg (L5) adds a path-dep
                  # Cargo.toml/Cargo.lock to the SHARED fixture; if that run is
                  # killed by the mandated `timeout` (SIGTERM/SIGKILL skips its
                  # finally), the leftovers change every later suite's
                  # tree/status/file-listing renders (backlog-#12 class).
                  "Cargo.toml", "Cargo.lock",
                  # The external-crate suites' probe buffer: a `timeout`-killed
                  # run leaves src/probe.rs untracked, and its `target_one`
                  # symbol makes drive_xref L1's M-. see TWO same-named
                  # definitions -> the ambiguity picker opens instead of the
                  # silent jump (observed live 2026-09-22: every lane's gate
                  # failed drive_xref identically until the stray was removed).
                  "src/probe.rs"):
        p = os.path.join(REPO, stray)
        try:
            os.remove(p)
        except FileNotFoundError:
            pass
    # ...and the target/ dir that Cargo.toml's presence can pull in.
    shutil.rmtree(os.path.join(REPO, "target"), ignore_errors=True)
    # Ensure the working-tree changes exist (idempotent: only writes markers if
    # the files lost them).
    lib = os.path.join(REPO, "src", "lib.rs")
    readme = os.path.join(REPO, "README.md")
    with open(lib) as f:
        if "staged_change_marker" not in f.read():
            with open(lib, "a") as f:
                f.write("\nstaged_change_marker\n")
    with open(readme) as f:
        if "unstaged_change_marker" not in f.read():
            with open(readme, "a") as f:
                f.write("\nunstaged_change_marker\n")

    # Staged: src/lib.rs ; unstaged: README.md ; drop the untracked notes file.
    _git("add", "src/lib.rs")
    _git("restore", "--staged", "README.md")
    try:
        os.remove(os.path.join(REPO, ".redline-notes.md"))
    except FileNotFoundError:
        pass


if __name__ == "__main__":
    reset()
    out = subprocess.run(["git", "-C", REPO, "status", "--porcelain"],
                         capture_output=True, text=True).stdout
    print("fixture reset to:\n" + out.strip())
