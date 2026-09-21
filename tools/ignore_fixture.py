#!/usr/bin/env python3
"""Generate the F2 perf fixture: a marker-only tree for measuring the
memoized ignore-chain walk (`src/model/files.rs`, via
`redline --index-profile=OUT`).

Shape (fixed by the F2 record):
  - marker-only: NO `.git` (the native gitignore sources are inert —
    `require_git` — so the walk runs the shared `is_gitignored`
    predicate, the code path the memo optimizes);
  - depth-4 directory tree, 20,000 files: 20 x 10 x 10 leaf dirs,
    10 files each;
  - a 33-line root `.gitignore`;
  - a small `.gitignore` every 5th directory (index % 5 == 0) at each
    level, including the leaves.

Usage:  ignore_fixture.py [OUT_DIR]     (default: /tmp/ignore_fixture)
OUT_DIR is created if missing; an existing one is wiped.

Profile (named, per P3c): `redline --index-profile=OUT_DIR` on the
RELEASE build, cold (fresh page-cache reads not forced; report both
the measured walk ms and the fixture command). The F2 memo record in
`src/model/files.rs` (test (d)) restates the pre-memo vs memoized
WALK time as a ratio for this fixture + profile — machine absolutes
are not comparable across boxes, so the record carries the ratio,
the profile, and this command, not an absolute.
"""

import shutil
import sys
from pathlib import Path

L1, L2, L3, FILES_PER_LEAF = 20, 10, 10, 10  # 20*10*10*10 = 20,000 files

# 33 lines: file rules, directory rules, and negations, as a real
# project's root `.gitignore` would mix them.
ROOT_GITIGNORE = [
    "*.log",
    "*.tmp",
    "target/",
    "dist/",
    "build/",
    "cache/",
    "node_modules/",
    "coverage/",
    "*.min.js",
    "*.map",
    "out/",
    "gen/",
    "tmp/",
    "scratch/",
    "*.swp",
    "*.swo",
    "*~",
    ".DS_Store",
    "Thumbs.db",
    "*.pid",
    "*.lock",
    "vendor/",
    "bundle/",
    "artifacts/",
    "output/",
    "staging/",
    "releases/",
    "reports/",
    "snapshots/",
    "fixtures/",
    "samples/",
    "prof/",
    "!keep.log",
]


def main() -> None:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ignore_fixture")
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)

    assert len(ROOT_GITIGNORE) == 33, "the F2 fixture pins a 33-line root .gitignore"
    out.joinpath(".gitignore").write_text("\n".join(ROOT_GITIGNORE) + "\n")
    out.joinpath("README.md").write_text("F2 perf fixture (marker-only)\n")

    files = 0
    for a in range(L1):
        for b in range(L2):
            for c in range(L3):
                leaf = out / f"d{a}" / f"e{b}" / f"f{c}"
                leaf.mkdir(parents=True)
                if c % 5 == 0:
                    leaf.joinpath(".gitignore").write_text("s0.rs\ns1.rs\n")
                for i in range(FILES_PER_LEAF):
                    leaf.joinpath(f"s{i}.rs").write_text(
                        f"// fixture {a}/{b}/{c}\nfn f{i}() {{}}\n"
                    )
                    files += 1
                # "every 5th directory" at the upper levels too (their
                # name-matched rules reach the files below them):
            if b % 5 == 0:
                (out / f"d{a}" / f"e{b}").joinpath(".gitignore").write_text("s7.rs\n")
        if a % 5 == 0:
            (out / f"d{a}").joinpath(".gitignore").write_text("s8.rs\n")
    print(
        f"wrote {out}: {files} source files (2,000 leaf dirs, "
        f"{L1}x{L2}x{L3} tree), marker-only, 33-line root .gitignore"
    )


if __name__ == "__main__":
    main()
