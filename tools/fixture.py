#!/usr/bin/env python3
"""Materialize + reset the shared PTY fixtures so the sweep is reproducible
regardless of prior (possibly state-mutating) runs AND on a clean machine.

Two fixtures are built here (both addressed through `repo()`):

  * `<root>/redline_pyte_repo`  — the main shared fixture (30 tool refs).
  * `<root>/redline_tall_repo`  — 30 changed files so the magit status buffer
                                  exceeds the 19-row window
                                  (tools/drive_windowing.py, drive_all).

`create()` is the one-command REBUILD (idempotent: it resets an existing repo,
or `git init`s a fresh one when absent). `reset()` only RESETS an existing
repo (git checkout + marker re-apply); it cannot create one. The fixture is
defined ENTIRELY by the committed content spec below (files + commit sequence
+ strays live as data), so it is reproducible from the repo, not from a
machine-local `/tmp` artifact.

The fixture root is REDLINE_FIXTURE_ROOT (default /tmp): every tools/ fixture
path resolves as <root>/<basename> via repo(), so a battery invocation can run
against a private tree (tools/gate.sh defaults the root to /tmp/fx<pid>;
tools/pool.py points each lane at its own dir).

redline_pyte_repo baseline (git log + working tree):
  * 6 commits: `init`, `commit 1` .. `commit 5` (drive_log's expected subjects).
  * tracked files: README.md, src/lib.rs, src/main.rs.
  * 5 UNTRACKED strays under src/ (part of the de-facto baseline — the tree,
    magit and buffer-list drives count them): fieldtype.rs, indented_demo.rs,
    nestedtype.rs, sym_cjk.rs, sym_demo.rs.
  * working tree: src/lib.rs STAGED (git add), README.md UNSTAGED, with the
    markers `staged_change_marker` / `unstaged_change_marker`.
  * no untracked .redline-notes.md (the notes flow creates it; sweep removes it).

Run directly to rebuild + reset:  python3 tools/fixture.py
Or import:                        from fixture import create, reset
"""
import os
import shutil
import subprocess

# ─── content spec: redline_pyte_repo ─────────────────────────────────────
# The tracked seed, and the order it must be committed. Kept as DATA so the
# fixture is defined by the repo (see the file docstring).

# src/main.rs at the `init` commit: 5 `target`-bearing lines (the search's
# 5-hit file) + `target_one`/`target_two`/`target_three` symbols.
MAIN_RS_INIT = (
    "fn target_one() {}\n"
    "fn target_two() {}\n"
    "let target_three = 3;\n"
    "target_one();\n"
    "target_two();\n"
    "let other = 9;\n"
)
# src/lib.rs: 1 `target` hit (`pub fn target_lib`).
LIB_RS = "pub fn target_lib() {}\npub fn other_lib() {}\n"
# README.md: 2 `target` hits — `# target README` and `target docs here`
# (drive_search / drive_results_narrow assert `target README`; the results
# title is `8 matches in 3 files` = 2 README + 1 lib + 5 main). NOTE the
# path property drive_tree_narrow depends on: `ma` matches ONLY src/main.rs,
# so README's PATH (and every stray path) must hold no `m`…`a` chain.
README_MD = "# target README\ntarget docs here\n"

# The 5 machine-local untracked strays. Their NAMES carry the letter
# properties the tree-narrow drive pins (path-scored): `lib` -> only
# src/lib.rs; `ma` -> only src/main.rs; `m` -> README.md + indented_demo +
# main + sym_cjk + sym_demo (NOT fieldtype/lib/nestedtype). Contents are
# inert demo files; sym_cjk.rs carries a CJK identifier (the buffer-list
# drive opens it as the MRU-current buffer). None may contain `target`
# (the search count is exactly 8).
STRAY_FILES = {
    "src/fieldtype.rs":
        "pub struct Pt {\n"
        "    pub field_one: i32,\n"
        "    pub field_two: i32,\n"
        "}\n",
    "src/indented_demo.rs":
        "fn demo() {\n"
        "    if 1 > 0 {\n"
        "        let inner = 2;\n"
        "    }\n"
        "}\n",
    "src/nestedtype.rs":
        "struct Outer {\n"
        "    inner: Inner,\n"
        "}\n"
        "\n"
        "struct Inner {\n"
        "    value: i32,\n"
        "}\n",
    "src/sym_cjk.rs":
        "fn main() {\n"
        "    let \u4e16\u754c = 1;\n"
        "    let \u4e2d\u4e2d = 2;\n"
        "}\n",
    "src/sym_demo.rs":
        "pub fn demo_symbol() {}\n"
        "pub fn other_symbol() {}\n",
}

# The commit sequence: `init` takes the whole tracked seed; then 5 commits
# each append a blank line + `line N` to src/main.rs (drive_log's subjects).
COMMIT_COUNT = 5


# ─── content spec: redline_tall_repo ─────────────────────────────────────
# 30 tracked files (`f00.txt`..`f29.txt` = `base\n`), committed once, then
# each gets `change\n` appended in the working tree -> 30 dirty files.
TALL_FILES = 30


def fixture_root():
    """Fixture tree root: REDLINE_FIXTURE_ROOT, default /tmp. Keep roots SHORT
    (fixture paths land on the 80-col status line)."""
    return os.environ.get("REDLINE_FIXTURE_ROOT", "/tmp")


def repo(name):
    """<root>/<name>: the single path indirection for every fixture repo.
    Basenames are fixed — suites assert on them."""
    return os.path.join(fixture_root(), name)


REPO = repo("redline_pyte_repo")
TALL_REPO = repo("redline_tall_repo")


def _git(*args):
    subprocess.run(["git", "-C", REPO, *args], check=True,
                   capture_output=True, text=True)


def _git_at(path, *args):
    subprocess.run(["git", "-C", path, *args], check=True,
                   capture_output=True, text=True)


def _write(path, text):
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


def _append(path, text):
    with open(path, "a", encoding="utf-8") as f:
        f.write(text)


def _ensure_strays():
    """Materialize the 5 untracked strays (idempotent: rewrite to canonical
    content so a mutated stray cannot drift the baseline)."""
    for rel, text in STRAY_FILES.items():
        _write(os.path.join(REPO, rel), text)


def _create_pyte_repo():
    """Build redline_pyte_repo from scratch (assumes REPO is absent/empty)."""
    shutil.rmtree(REPO, ignore_errors=True)
    os.makedirs(os.path.join(REPO, "src"))
    _write(os.path.join(REPO, "README.md"), README_MD)
    _write(os.path.join(REPO, "src", "lib.rs"), LIB_RS)
    _write(os.path.join(REPO, "src", "main.rs"), MAIN_RS_INIT)
    _git("init", "-q", "-b", "main")
    _git("config", "user.name", "Test")
    _git("config", "user.email", "test@example.com")
    _git("add", "-A")
    _git("commit", "-q", "-m", "init")
    for i in range(1, COMMIT_COUNT + 1):
        _append(os.path.join(REPO, "src", "main.rs"), "\nline %d\n" % i)
        _git("add", "-A")
        _git("commit", "-q", "-m", "commit %d" % i)
    _ensure_strays()


def _create_tall_repo():
    """Build redline_tall_repo from scratch: 30 files committed at `base`,
    then all 30 changed in the working tree -> the magit status buffer
    (~34 rows) exceeds the 19-row magit window."""
    shutil.rmtree(TALL_REPO, ignore_errors=True)
    os.makedirs(TALL_REPO)
    for i in range(TALL_FILES):
        _write(os.path.join(TALL_REPO, "f%02d.txt" % i), "base\n")
    _git_at(TALL_REPO, "init", "-q", "-b", "main")
    _git_at(TALL_REPO, "config", "user.name", "Test")
    _git_at(TALL_REPO, "config", "user.email", "test@example.com")
    _git_at(TALL_REPO, "add", "-A")
    _git_at(TALL_REPO, "commit", "-q", "-m", "init")
    for i in range(TALL_FILES):
        _append(os.path.join(TALL_REPO, "f%02d.txt" % i), "change\n")


def create():
    """Idempotently materialize BOTH shared fixtures from the content spec.

    * absent (no `.git`) -> build from scratch (files + commit sequence).
    * present            -> reset to the baseline (git checkout + markers).

    Safe to run repeatedly; the one-command rebuild for a fresh machine.
    """
    if not os.path.isdir(os.path.join(REPO, ".git")):
        _create_pyte_repo()
    # Always (re)assert the untracked strays: they are baseline, not leg output.
    _ensure_strays()
    reset()
    if not os.path.isdir(os.path.join(TALL_REPO, ".git")):
        _create_tall_repo()


def reset():
    """Reset redline_pyte_repo's working tree to the baseline (repo must
    already exist — see create())."""
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
    create()
    out = subprocess.run(["git", "-C", REPO, "status", "--porcelain"],
                         capture_output=True, text=True).stdout
    print("fixture created/reset to:\n" + out.strip())
    print("fixtures: %s, %s" % (REPO, TALL_REPO))
