#!/usr/bin/env python3
"""Deterministic cleanup inventory for redline (no LLM, no network, no build).

Why this exists: LLM/grep-based scans have a coverage ceiling set by the
patterns the scanner thought to grep for. This finds duplication and dead
public surface mechanically, over every .rs file in src/ + crates/.

Usage:
  python3 tools/cleanup_scan.py dup      # cross-file duplicate code blocks (window + fn level)
  python3 tools/cleanup_scan.py deadpub  # pub fns with no reference anywhere in-repo
  python3 tools/cleanup_scan.py all

Skips .agents/ (worktree slots are full-tree copies and would triple every hit)
and target/.
"""
import os, re, sys, hashlib, collections

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SKIP = ("/target", "/.git", "/.agents")


def rs_files():
    out = []
    for base in ("src", "crates"):
        for dp, dn, fn in os.walk(os.path.join(ROOT, base)):
            if any(s in dp for s in SKIP):
                continue
            dn[:] = [d for d in dn if d != "target" and d != ".git"]
            for f in fn:
                if f.endswith(".rs"):
                    out.append(os.path.join(dp, f))
    return out


def rel(p):
    return p.replace(ROOT + "/", "")


def strip(line):
    return re.sub(r"\s+", " ", re.sub(r"//.*", "", line)).strip()


def win_dups(files, width=6, min_chars=120):
    """Sliding normalized-window duplicates: catches copy-paste with any shape."""
    hits = collections.defaultdict(list)
    for p in files:
        lines = [strip(l) for l in open(p, encoding="utf8", errors="replace").read().splitlines()]
        for i in range(len(lines) - width + 1):
            win = lines[i:i + width]
            if any(not l for l in win) or sum(map(len, win)) < min_chars:
                continue
            hits[hashlib.md5("\n".join(win).encode()).hexdigest()].append((rel(p), i + 1, win[0]))
    return {k: v for k, v in hits.items() if len({p for p, _, _ in v}) >= 2}


def fn_dups(files):
    """Brace-matched whole-function duplicates: catches renamed/reshaped copies."""
    fns = []
    for p in files:
        raw = open(p, encoding="utf8", errors="replace").read().splitlines()
        for i, l in enumerate(raw):
            m = re.match(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_]\w*)", l)
            if not m:
                continue
            depth, started, body = 0, False, []
            for j in range(i, len(raw)):
                s = strip(raw[j])
                depth += s.count("{") - s.count("}")
                if "{" in s:
                    started = True
                body.append(s)
                if started and depth <= 0:
                    break
            fns.append((rel(p), m.group(1), i + 1, [l for l in body if l and not l.startswith("#[")]))
    groups = collections.defaultdict(list)
    for p, n, ln, body in fns:
        if len(body) < 6:
            continue
        key = hashlib.md5(("\n".join(body[1:-1] if len(body) > 2 else body)).encode()).hexdigest()
        groups[key].append((p, n, ln, len(body)))
    return len(fns), {k: v for k, v in groups.items() if len({p for p, _, _, _ in v}) >= 2}


def report_dup():
    files = rs_files()
    w = win_dups(files)
    nfns, f = fn_dups(files)
    print(f"# duplication scan: {len(files)} files, {nfns} functions parsed\n")
    print(f"## window duplicates (6 normalized lines, cross-file): {len(w)}")
    for k, v in sorted(w.items(), key=lambda kv: -len({p for p, _, _ in kv[1]}))[:15]:
        locs = sorted({f"{p}:{ln}" for p, ln, _ in v})
        print(f"\n[{len(locs)} sites] {v[0][2][:80]}")
        for l in locs[:8]:
            print("   ", l)
    print(f"\n## identical whole-function bodies (cross-file): {len(f)}")
    for k, v in sorted(f.items(), key=lambda kv: -len({p for p, _, _, _ in kv[1]})):
        locs = sorted({f"{p}:{ln} {n}()" for p, n, ln, _ in v})
        print(f"\n[{len(locs)} sites, ~{v[0][3]} lines each]")
        for l in locs:
            print("   ", l)


def report_deadpub():
    files = rs_files()
    alltext = "".join(open(p, encoding="utf8", errors="replace").read() for p in files)
    by_name = collections.defaultdict(list)
    for p in files:
        txt = open(p, encoding="utf8", errors="replace").read()
        for m in re.finditer(r"^\s*pub(?:\([^)]*\))?\s+(?:async\s+)?fn\s+([A-Za-z_]\w*)", txt, re.M):
            by_name[m.group(1)].append(rel(p))
    dead = [(n, ps[0]) for n, ps in by_name.items()
            if len(ps) == 1 and len(re.findall(r"\b" + re.escape(n) + r"\b", alltext)) <= 1]
    print(f"# dead pub surface: {len(dead)} pub fns with zero in-repo references")
    print("# (corpus/ fixture files under tests/corpus are expected to appear here)")
    for n, p in sorted(dead):
        print(f"   {p}: {n}()")


if __name__ == "__main__":
    what = sys.argv[1] if len(sys.argv) > 1 else "all"
    if what in ("dup", "all"):
        report_dup()
    if what in ("deadpub", "all"):
        print()
        report_deadpub()
