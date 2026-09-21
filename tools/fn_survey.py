#!/usr/bin/env python3
"""fn_survey — production function sizes, nesting depth, and locations.

Built for the "complex logic organized for maintainability" agenda: it answers
"which production functions are long / deeply nested / where exactly?" so a lane
can be pointed at a real target.

Why this exists as a file rather than an inline heredoc: the first version of this
scan (used to find `Root` 504, `key_event` 345, `xref_find_definitions` 177) stripped
comments and string literals for brace matching **and dropped the newlines inside
them**, so every reported `file:line` drifted *earlier* by the number of newlines
inside block comments and multi-line strings. In `src/syntax/queries.rs` — whose
tests are full of multi-line raw strings — that was ~115 lines: the scan said
`extract_all` was at `:360` when it is at `:475`. The SIZES were correct (brace
matched); the LOCATIONS were not, and a spec that says "this function at file:line"
must not be wrong about the line.

This version preserves newlines while stripping, so locations are exact.

Usage:
    python3 tools/fn_survey.py [--min N] [--top N] [--nest N] [paths...]

Default: production functions >= 80 lines, top 25, under `src/` and `crates/`.
`--nest N` instead lists the deepest-nesting functions.
Test modules (`#[cfg(test)]`) and `tests/` directories are excluded — test file
size is explicitly not a criterion in this repo.
"""
from __future__ import annotations

import argparse
import os
import re
import sys

DEFAULT_MIN = 80
DEFAULT_TOP = 25


def strip_preserving_newlines(src: str) -> str:
    """Remove comments and string/char literal *content*, keeping every newline.

    Keeping the newlines is the whole point: line numbers computed from the
    result must match the original file.
    """
    out: list[str] = []
    i = 0
    n = len(src)
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            if j < 0:
                break
            i = j  # the newline is appended by the next iteration
            continue
        if src.startswith("/*", i):
            j = src.find("*/", i + 2)
            seg = src[i:n] if j < 0 else src[i : j + 2]
            out.append("\n" * seg.count("\n"))
            i = n if j < 0 else j + 2
            continue
        if c in ('"',) or (c in "br" and i + 1 < n and src[i + 1] == '"'):
            # ordinary/byte string literal (raw strings `r#"…"#` are handled
            # approximately: scanning from the first `"` to the next unescaped
            # `"`, which is exact unless the raw string itself contains a quote)
            if c != '"':
                out.append(c)
                i += 1
            i += 1  # skip the opening quote
            start = i
            while i < n:
                if src[i] == "\\":
                    i += 2
                    continue
                if src[i] == '"':
                    break
                i += 1
            out.append("\n" * src[start:i].count("\n"))
            i += 1
            continue
        if c == "'":
            m = re.match(r"'(?:\\.|[^\\'])'", src[i:])
            if m:  # char literal: no newlines possible
                out.append(" ")
                i += m.end()
                continue
            # else a lifetime (`'a`) — fall through and emit it
        out.append(c)
        i += 1
    return "".join(out)


def cfg_test_regions(s: str) -> list[tuple[int, int]]:
    """Byte ranges of `#[cfg(test)]` items (so tests are excluded)."""
    regs: list[tuple[int, int]] = []
    for m in re.finditer(r"#\[cfg\(test\)\]", s):
        b = s.find("{", m.end())
        if b < 0:
            continue
        depth = 0
        i = b
        while i < len(s):
            if s[i] == "{":
                depth += 1
            elif s[i] == "}":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        regs.append((m.start(), i))
    return regs


FN_RE = re.compile(r"\bfn\s+([A-Za-z0-9_]+)\s*(?:<[^>]*>)?\s*\(")


def survey_file(path: str) -> list[tuple[int, int, int, str, str]]:
    raw = open(path, encoding="utf-8", errors="ignore").read()
    s = strip_preserving_newlines(raw)
    regs = cfg_test_regions(s)
    found = []
    for m in FN_RE.finditer(s):
        if any(a <= m.start() <= b for a, b in regs):
            continue
        b = s.find("{", m.end())
        if b < 0 or ";" in s[m.end() : b]:
            continue  # trait decl / extern block
        depth = 0
        i = b
        maxd = 0
        while i < len(s):
            if s[i] == "{":
                depth += 1
                maxd = max(maxd, depth)
            elif s[i] == "}":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        body = s[b:i]
        nargs = len([a for a in s[m.end() : b].split(",") if a.strip() and a.strip() != ")"])
        line = s[: m.start()].count("\n") + 1
        found.append((body.count("\n"), maxd, nargs, m.group(1), f"{path}:{line}"))
    return found


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("paths", nargs="*", default=["src", "crates"])
    ap.add_argument("--min", type=int, default=DEFAULT_MIN)
    ap.add_argument("--top", type=int, default=DEFAULT_TOP)
    ap.add_argument("--nest", type=int, default=0, help="list deepest nesting instead")
    args = ap.parse_args()

    roots = args.paths or ["src", "crates"]
    results = []
    for root in roots:
        if os.path.isfile(root):
            results += survey_file(root)
            continue
        for dirpath, dirnames, filenames in os.walk(root):
            dirnames[:] = [d for d in dirnames if d not in ("target", ".git", "tests")]
            for f in filenames:
                if f.endswith(".rs"):
                    results += survey_file(os.path.join(dirpath, f))

    if args.nest:
        results.sort(key=lambda r: -r[1])
        print(f"{'lines':>5} {'nest':>4} {'args':>4}  function")
        for lines, maxd, nargs, name, loc in results[: args.top]:
            print(f"{lines:>5} {maxd:>4} {nargs:>4}  {name:<40} {loc}")
        return 0

    big = [r for r in results if r[0] >= args.min]
    big.sort(key=lambda r: -r[0])
    print(f"# production functions >= {args.min} lines: {len(big)}  (of {len(results)} surveyed)")
    print(f"{'lines':>5} {'nest':>4} {'args':>4}  function")
    for lines, maxd, nargs, name, loc in big[: args.top]:
        print(f"{lines:>5} {maxd:>4} {nargs:>4}  {name:<40} {loc}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
