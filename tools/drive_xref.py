#!/usr/bin/env python3
"""M-. xref thin tier (loop-04): the same-file jump smoke + the jump-column
pins (jump-column-pty / issue-all-symbol-jumps).

One leg of the original six stays PTY: L1 (M-. at the END of the `target_one`
call jumps to the SAME-FILE definition — the user's core complaint — lands
on it, and M-, returns to the call site). The state halves of the rest are
unit twins (src/app/flow_tests.rs, loop-04):

  * L2 cross-file jump          -> unit_flow_xref_l2
  * L3 bare-miss report         -> unit_flow_xref_l3
  * L4 raw `::` path token      -> unit_flow_xref_l4
  * L5 external landing guard   -> unit_flow_xref_l5 (the path-dep's cargo
                                   metadata resolution itself is the
                                   resolver corpus's)
  * L6 middle-landing recenter  -> unit_flow_xref_l6

What stays PTY-only here is the live terminal: the real M-. input through
the terminal encoder and the live landing/repaint.

L7 (jump-column-pty): the COLUMN pins. The user's second report was "jump
to a field name -> beginning of line": a col-0 landing on any M-. path
used to pass this whole drive (it asserted rows only). Each L7 pin asserts
the CUP the stream emits after the landing — the position the user
actually sees — and every fixture's symbol sits at a NONZERO column, so a
col-0 regression (CUP col 1) fails the battery:

  * L7a same-file unique M-. (`definitions.rs` silent path, outline
        symbol): `alpha_target` at char col 7 -> CUP (2, 8); the M-> forced
        list's RET re-asserts it (the picker's outline re-read arm).
  * L7b cross-file field -> Xref picker -> RET: `beta_field` at char col 20
        in col_b.rs -> CUP (2, 21). Pre-fix: the field table carried a
        line only, so the landing was col 0 by construction -> CUP (2, 1).
  * L7c same-file method via the local-binding pre-step: `gamma_method` at
        char col 7 of line 2 -> CUP (4, 8). Pre-fix: CUP (4, 1).
  * L7d multibyte field (the byte->(line,col) conversion): a CJK comment
        before `delta_field` puts its char column (29) FOUR cells behind a
        naive byte reading would suggest only in the wide-char sense — the
        CUP is the DISPLAY column 31 -> (2, 32). A col-0 regression shows
        (2, 1); landing a raw byte column instead of the char column would
        show a shifted CUP.

L1 also gains a CUP pin: `target_one` at char col 3 -> CUP (2, 4) (the
outline-symbol silent path, already carrying start_byte since fcde521 —
this is the regression pin that keeps it there).

Fixture: /tmp/redline_pyte_repo (a git repo WITHOUT a Cargo.toml — the
fixture baseline is explicitly cargo-less; the leg files are removed after).
"""
import os
import shutil
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fixture import reset
from pyte_driver import App
from fixture import repo

REPO = repo("redline_pyte_repo")
LEG_RS = os.path.join(REPO, "src", "leg.rs")
MINI = 22  # minibuffer row (content rows 0..20 in a 24-row PTY, 1-based
           # terminal row 23 is the status line)

# L7 fixtures: written BEFORE the App starts so the startup index covers
# them (M-. resolves off the index); removed in the finally. Every symbol
# pinned below sits at a NONZERO column.
COL_A_RS = os.path.join(REPO, "src", "col_a.rs")
COL_B_RS = os.path.join(REPO, "src", "col_b.rs")
COL_BU_RS = os.path.join(REPO, "src", "col_b_use.rs")
COL_C_RS = os.path.join(REPO, "src", "col_c.rs")
COL_D_RS = os.path.join(REPO, "src", "col_d.rs")
COL_A_CONTENT = "pub fn alpha_target() {}\nfn caller() {\n    alpha_target();\n}\n"
COL_B_CONTENT = "pub struct Pt { pub beta_field: i32 }\n"
COL_BU_CONTENT = (
    "use crate::col_b::Pt;\n"
    "fn use_it() {\n"
    "    let p = Pt { beta_field: 1 };\n"
    "    let _ = p.beta_field;\n"
    "}\n"
)
COL_C_CONTENT = (
    "pub struct Pt { pub x: i32 }\n"
    "impl Pt {\n"
    "    fn gamma_method(&self) -> i32 { self.x }\n"
    "}\n"
    "fn use_it() {\n"
    "    let p = Pt { x: 1 };\n"
    "    let _ = p.gamma_method();\n"
    "}\n"
)
COL_D_CONTENT = (
    "pub struct Pt { /* 中文 */ pub delta_field: i32 }\n"
    "fn use_it() {\n"
    "    let p = Pt { delta_field: 1 };\n"
    "    let _ = p.delta_field;\n"
    "}\n"
)
L7_FILES = (COL_A_RS, COL_B_RS, COL_BU_RS, COL_C_RS, COL_D_RS)

results = []


def rec(tag, ok, detail=""):
    results.append(ok)
    print(f"{'OK  ' if ok else 'FAIL'} {tag}" + (f"  [{detail}]" if detail and not ok else ""))


def poll(app, needle, timeout=25.0):
    """Wait until `needle` appears in the minibuffer row (transient
    messages: the status-line row)."""
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        app.wait(0.4)
        last = app.row_text(MINI)
        if needle in last:
            return True, last
    return False, last


def poll_screen(app, needle, timeout=25.0):
    """Wait until `needle` appears ANYWHERE on the screen (the picker's
    "Definition: " prompt row, which is not the status line)."""
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        app.wait(0.4)
        last = app.screen_text()
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


def cup(app):
    """The surviving CUP (1-based row, col) — the cursor position the user
    sees. cup_settle keeps the read window open until the chunk's last
    synchronized frame is closed, so a (None, ...) read under load is a
    protocol finding, not a scheduling artifact. 013-02: the CUP now
    rides inside the frame (after the park, before the close) — see
    pyte_driver.App.cup_after_sync for the re-derived contract."""
    return app.cup_settle()


reset()
print("=== M-. same-file jump (thin tier) ===")
os.makedirs(os.path.join(REPO, "src"), exist_ok=True)
with open(LEG_RS, "w") as f:
    f.write("fn leg() {\n    target_lib();\n}\ntokio::spawn(f);\n")
for path, content in zip(L7_FILES,
                         (COL_A_CONTENT, COL_B_CONTENT, COL_BU_CONTENT,
                          COL_C_CONTENT, COL_D_CONTENT)):
    with open(path, "w") as f:
        f.write(content)

app = App(REPO)
app.wait_ready()
try:
    open_file(app, "src/main.rs")
    goto(app, 4)  # "target_one();"
    app.key("M-f", 0.8)  # point to the END of the `target_one` run
    app.key("M-.", 1.5)
    ok, msg = poll(app, "jumped to src/main.rs", timeout=5.0)
    rec("L1 same-file: message", ok, f"minibuffer={msg!r}")
    ok = "fn target_one() {}" in top_content(app)
    rec("L1 same-file: window landed on the definition", ok,
        f"top={top_content(app)!r}")
    # jump-column-pty: the CUP lands ON the name (char col 3 -> 1-based
    # col 4: `fn ` is three chars), not the line start. A col-0
    # regression shows CUP (2, 1).
    r, c = cup(app)
    rec("L1 same-file: CUP on the name column", (r, c) == (2, 4),
        f"cup=({r},{c}) want (2,4)")
    # Jump-back returns to the call site (the landing recorded a jump).
    app.key("M-,", 1.0)
    app.wait(0.5)
    back = "target_one();" in app.screen_text()
    rec("L1 same-file: M-, back to the call site", back,
        f"screen has call site={back}")

    # ── L7a: same-file unique M-. (outline symbol) + M-> forced list ────
    print("=== M-. column pins (jump-column-pty) ===")
    open_file(app, "src/col_a.rs")
    goto(app, 3)  # "    alpha_target();"
    app.key("M-f", 0.8)  # end of the `alpha_target` run
    app.key("M-.", 1.5)
    ok, msg = poll(app, "jumped to src/col_a.rs", timeout=5.0)
    rec("L7a same-file unique: silent jump message", ok, f"minibuffer={msg!r}")
    r, c = cup(app)
    rec("L7a same-file unique: CUP on `alpha_target` (char col 7)",
        (r, c) == (2, 8), f"cup=({r},{c}) want (2,8); col-0 regression = (2,1)")
    app.key("M-,", 1.0)
    app.wait(0.5)
    app.key("M->", 1.0)  # force the candidate list for the same target
    ok, msg = poll_screen(app, "Definition:", timeout=5.0)
    rec("L7a M-> forced list: picker opened", ok, f"screen has prompt={ok}")
    app.key("RET", 1.0)
    ok, msg = poll(app, "jumped to src/col_a.rs", timeout=5.0)
    rec("L7a M-> RET: landed", ok, f"minibuffer={msg!r}")
    r, c = cup(app)
    rec("L7a M-> RET: CUP on `alpha_target` (picker outline re-read arm)",
        (r, c) == (2, 8), f"cup=({r},{c}) want (2,8); col-0 regression = (2,1)")

    # ── L7b: cross-file field -> Xref picker -> RET (the user's repro) ──
    open_file(app, "src/col_b_use.rs")
    goto(app, 4)  # "    let _ = p.beta_field;"
    app.key("M-f M-f M-f M-f", 0.8)  # end of the `beta_field` run
    app.key("M-.", 1.5)
    ok, msg = poll_screen(app, "Definition:", timeout=5.0)
    rec("L7b cross-file field: picker opened", ok, f"screen has prompt={ok}")
    app.key("RET", 1.0)  # the cross-file field row is preselected
    ok, msg = poll(app, "jumped to src/col_b.rs", timeout=5.0)
    rec("L7b cross-file field: RET landed in the struct's file",
        ok, f"minibuffer={msg!r}")
    r, c = cup(app)
    rec("L7b cross-file field: CUP on `beta_field` (char col 20)",
        (r, c) == (2, 21),
        f"cup=({r},{c}) want (2,21); pre-fix col-0 landing = (2,1)")

    # ── L7c: same-file method via the local-binding pre-step ────────────
    open_file(app, "src/col_c.rs")
    goto(app, 7)  # "    let _ = p.gamma_method();"
    app.key("M-f M-f M-f M-f", 0.8)  # end of the `gamma_method` run
    app.key("M-.", 1.5)
    ok, msg = poll(app, "jumped to src/col_c.rs", timeout=5.0)
    rec("L7c method pre-step: silent jump message", ok, f"minibuffer={msg!r}")
    r, c = cup(app)
    # The method's `fn` is on line 2 of col_c.rs -> terminal row 4 (the
    # title row is row 1); `gamma_method` is char col 7 -> 1-based col 8.
    rec("L7c method pre-step: CUP on `gamma_method` (char col 7)",
        (r, c) == (4, 8),
        f"cup=({r},{c}) want (4,8); pre-fix col-0 landing = (4,1)")

    # ── L7d: multibyte — the landing column is a CHAR column ────────────
    open_file(app, "src/col_d.rs")
    goto(app, 4)  # "    let _ = p.delta_field;"
    app.key("M-f M-f M-f M-f", 0.8)  # end of the `delta_field` run
    app.key("M-.", 1.5)
    ok, msg = poll(app, "jumped to src/col_d.rs", timeout=5.0)
    rec("L7d multibyte field: silent jump message", ok, f"minibuffer={msg!r}")
    r, c = cup(app)
    # `delta_field` is char col 29 of "pub struct Pt { /* 中文 */ pub
    # delta_field: i32 }" — the CJK pair before it takes 2 EXTRA display
    # cells, so the display column is 31 -> 1-based CUP col 32. A col-0
    # regression shows (2, 1); landing a raw byte column (33) as a char
    # index would show a shifted CUP (char 33 = 'a' of `delta_field` at
    # display col 35 -> (2, 36)).
    rec("L7d multibyte field: CUP is the DISPLAY column of the char col",
        (r, c) == (2, 32),
        f"cup=({r},{c}) want (2,32); col-0 regression = (2,1), byte-as-char = (2,36)")
finally:
    app.kill()
    for stray in (LEG_RS, *L7_FILES):
        try:
            os.remove(stray)
        except FileNotFoundError:
            pass

print(f"\n{sum(results)}/{len(results)} legs passed")
sys.exit(0 if all(results) else 1)
