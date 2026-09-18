#!/usr/bin/env python3
"""Drive M-. (xref-find-definitions) — cursor-aware selection + resolver
fall-through (plan 006 issue 02) against the shared fixture.

Legs (fixture /tmp/redline_pyte_repo — a git repo WITHOUT a Cargo.toml, so
the cargo provider fails fast and offline):
  L1 same-file:   cursor at the END of the `target_one` call → jump to the
                  SAME-FILE definition (struct+impl-style case; the old
                  cross-file filter made this unjumpable).
  L2 cross-file:  cursor at the end of `target_lib` in a fresh src/leg.rs →
                  jump to src/lib.rs.
  L3 bare miss:   cursor at the end of `other` in `let other = 9;` — not in
                  the index (it is a let binding), no enclosing symbol →
                  resolver fall-through → a clear
                  "no provider resolution for `other`" message.
  L4 path token:  cursor at the end of `tokio` in `tokio::spawn(f);` — the
                  miss report names the PATH token `tokio::spawn` (the raw
                  token under the point is what the resolver gets).
  L5 ownership guard (006-02b item 1): a PATH dependency OUTSIDE the
                  fixture root lands via the same external-landing path as
                  a registry source (read-only `open_external_path`); the
                  C-x C-q override AND C-x C-s are refused there ("external
                  buffer is read-only (not project-owned)"). The temporary
                  Cargo.toml + external crate are removed afterwards — the
                  fixture baseline is explicitly cargo-less.
"""
import os
import shutil
import sys
import time

sys.path.insert(0, "/home/gary/dev/red/tools")
from fixture import reset
from pyte_driver import App

REPO = "/tmp/redline_pyte_repo"
LEG_RS = os.path.join(REPO, "src", "leg.rs")
EXT_CRATE = "/tmp/redline_xref_extcrate"
MINI = 22  # minibuffer row (content rows 0..20 in a 24-row PTY, 1-based
           # terminal row 23 is the status line)

results = []


def rec(tag, ok, detail=""):
    results.append(ok)
    print(f"{'OK  ' if ok else 'FAIL'} {tag}" + (f"  [{detail}]" if detail and not ok else ""))


def poll(app, needle, timeout=25.0):
    """Wait until `needle` appears in the minibuffer row."""
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        app.wait(0.4)
        last = app.row_text(MINI)
        if needle in last:
            return True, last
    return False, last


def open_file(app, rel):
    app.key("C-x C-f", 1.0)
    for ch in rel:
        app.key(ch, 0.15)
    app.key("RET", 1.0)


def goto(app, n):
    app.key("M-g g", 0.8)
    app.key(str(n), 0.4)
    app.key("RET", 1.0)


def top_content(app):
    return app.row_text(1)  # terminal row 1 = content row 0


reset()
print("=== M-. cursor-aware jump + resolver fall-through ===")
# Belt and braces: a crashed prior L5 run must not have left cargo state
# (the fixture baseline is explicitly cargo-less; reset() does not know
# about these files).
for stray in ("Cargo.toml", "Cargo.lock"):
    try:
        os.remove(os.path.join(REPO, stray))
    except FileNotFoundError:
        pass
shutil.rmtree(os.path.join(REPO, "target"), ignore_errors=True)
shutil.rmtree(EXT_CRATE, ignore_errors=True)
os.makedirs(os.path.join(REPO, "src"), exist_ok=True)
with open(LEG_RS, "w") as f:
    f.write("fn leg() {\n    target_lib();\n}\ntokio::spawn(f);\n")

app = App(REPO)
app.wait_ready()
try:
    # ── L1: same-file jump (the user's core complaint) ──────────────────
    open_file(app, "src/main.rs")
    goto(app, 4)  # "target_one();"
    app.key("M-f", 0.8)  # point to the END of the `target_one` run
    app.key("M-.", 1.5)
    ok, msg = poll(app, "jumped to src/main.rs", timeout=5.0)
    rec("L1 same-file: message", ok, f"minibuffer={msg!r}")
    ok = "fn target_one() {}" in top_content(app)
    rec("L1 same-file: window landed on the definition", ok,
        f"top={top_content(app)!r}")
    # Jump-back returns to the call site (the landing recorded a jump).
    app.key("M-,", 1.0)
    app.wait(0.5)
    back = "target_one();" in app.screen_text()
    rec("L1 same-file: M-, back to the call site", back,
        f"screen has call site={back}")

    # ── L2: cross-file jump ─────────────────────────────────────────────
    open_file(app, "src/leg.rs")
    goto(app, 2)  # "    target_lib();"
    app.key("M-f", 0.8)  # end of `target_lib` (skips `fn`... no: line 2 IS the call)
    app.key("M-.", 1.5)
    ok, msg = poll(app, "jumped to src/lib.rs", timeout=5.0)
    rec("L2 cross-file: message", ok, f"minibuffer={msg!r}")
    ok = "pub fn target_lib() {}" in top_content(app)
    rec("L2 cross-file: window landed on lib.rs definition", ok,
        f"top={top_content(app)!r}")

    # ── L3: bare (field-shaped) name → resolver fall-through ────────────
    open_file(app, "src/main.rs")
    goto(app, 6)  # "let other = 9;"
    app.key("M-f M-f", 0.8)  # end of `let`, then end of `other`
    app.key("M-.", 1.5)
    ok, msg = poll(app, "provider(s): rust", timeout=25.0)
    ok = ok and "`other`" in msg and "`tokio`" not in msg
    rec("L3 bare miss: graceful resolver report", ok, f"minibuffer={msg!r}")

    # ── L4: `::`-path token reaches the resolver raw ────────────────────
    open_file(app, "src/leg.rs")
    goto(app, 4)  # "tokio::spawn(f);"
    app.key("M-f", 0.8)  # end of `tokio`
    app.key("M-.", 1.5)
    ok, msg = poll(app, "provider(s): rust", timeout=25.0)
    ok = ok and "`tokio::spawn`" in msg
    rec("L4 path token: resolver gets the raw `tokio::spawn`", ok,
        f"minibuffer={msg!r}")

    # ── L5: the ownership guard on an external landing (006-02b item 1) ─
    # A PATH dependency outside the fixture root resolves (cargo metadata)
    # to a source dir OUTSIDE the project root → the resolver lands it via
    # open_external_path (read-only), exactly like a registry source.
    os.makedirs(os.path.join(EXT_CRATE, "src"), exist_ok=True)
    with open(os.path.join(EXT_CRATE, "Cargo.toml"), "w") as f:
        f.write('[package]\nname = "extdep"\nversion = "0.1.0"\nedition = "2021"\n')
    with open(os.path.join(EXT_CRATE, "src", "lib.rs"), "w") as f:
        f.write("pub fn ext_target() {}\n")
    # The dep is RENAMED (`extleg`) so the crate name `extdep` never
    # appears as a key the project symbol-indexer would index as a
    # definition (a bare `extdep` key makes M-. jump to Cargo.toml
    # instead of falling through — the drive_external_notes trick).
    with open(os.path.join(REPO, "Cargo.toml"), "w") as f:
        f.write('[package]\nname = "pyte_repo"\nversion = "0.1.0"\nedition = "2021"\n\n'
                f'[dependencies]\nextleg = {{ package = "extdep", path = "{EXT_CRATE}" }}\n')
    # Probe line: TOP-LEVEL call in leg.rs (no enclosing symbol → the M-.
    # selection falls through to the tooling resolver).
    with open(LEG_RS, "w") as f:
        f.write("fn leg() {\n    target_lib();\n}\ntokio::spawn(f);\nextdep::ext_target();\n")
    open_file(app, "src/leg.rs")
    goto(app, 5)  # "extdep::ext_target();"
    app.key("M-f", 0.8)  # point to the END of the `extdep` run
    app.key("M-.", 1.5)
    ok, msg = poll(app, "jumped to", timeout=45.0)
    rec("L5 external landing: M-. resolves the path dep OUTSIDE the root",
        ok and EXT_CRATE in msg, f"minibuffer={msg!r}")
    app.key("C-x C-q", 1.0)
    ok, msg = poll(app, "external buffer is read-only", timeout=10.0)
    rec("L5 ownership guard: C-x C-q is refused on the external buffer",
        ok, f"minibuffer={msg!r}")
    app.key("C-x C-s", 1.0)
    ok, msg = poll(app, "external buffer is read-only", timeout=10.0)
    rec("L5 ownership guard: C-x C-s is refused on the external buffer",
        ok, f"minibuffer={msg!r}")
finally:
    app.kill()
    # L5 leftovers: restore the cargo-less fixture baseline for the other
    # fixture suites and remove the temporary external crate.
    for stray in ("Cargo.toml", "Cargo.lock"):
        try:
            os.remove(os.path.join(REPO, stray))
        except FileNotFoundError:
            pass
    shutil.rmtree(os.path.join(REPO, "target"), ignore_errors=True)
    shutil.rmtree(EXT_CRATE, ignore_errors=True)

print(f"\n{sum(results)}/{len(results)} legs passed")
sys.exit(0 if all(results) else 1)
