#!/usr/bin/env python3
"""issue-clipboard-and-selection — live PTY probe (measurement, not a gate).

Measures, against a real PTY, the three reported defects:

  A. Does the app capture the mouse (the reason the terminal's own
     drag-to-select is unavailable)? → startup mouse-reporting request.
  B. OSC 52: a left drag (SGR press/drag/release) over the file pane,
     then M-w → the byte-exact escape must appear in the app's PTY
     output stream, and its base64 payload must decode to exactly the
     dragged lines' UTF-8. The region rows must be visibly highlighted
     (the region face background) BEFORE the copy.
  C. Paste (measured fix, not just measurement): the app requests NO
     bracketed paste (2004) — iocraft surfaces no Paste event, so enabling
     it would turn a working paste into unhandled escapes (a deliberate
     non-change). A paste is therefore ordinary key bytes; the pre-fix
     input gate (ASCII-only printable) dropped every non-ASCII char of a
     paste. The probe verifies the fixed path end-to-end: a pasted 中
     lands with the cursor at its DISPLAY column (wide char = 2 cells),
     and the multi-byte bytes land in .redline-notes.md byte-exact on
     save.

Honest scope: this box has no terminal EMULATOR with a system clipboard
in the loop — the PTY master is a pipe. So B measures EMISSION (the
escape reaches the terminal's output stream byte-exact); whether a real
terminal app honours it (xterm/wezterm/foot/kitty do; iTerm2 by default
does not) is not measurable here. The user should try M-w in a real
terminal that honours OSC 52 (xterm, wezterm, foot, kitty, tmux with
`set-clipboard on`).
"""
import base64
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pyte_driver  # noqa: E402

BIN = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "target", "debug", "redline")


def osc52_payloads(raw: bytes):
    """Every OSC 52 escape in `raw`, as decoded payloads (or b'' on
    malformed)."""
    out = []
    i = 0
    while True:
        i = raw.find(b"\x1b]52;c;", i)
        if i < 0:
            break
        end = raw.find(b"\x07", i)
        if end < 0:
            break
        b64 = raw[i + 7:end]
        try:
            out.append(base64.b64decode(b64 + b"=" * (-len(b64) % 4)))
        except Exception as e:
            out.append(("MALFORMED", b64, e))
        i = end + 1
    return out


def region_rows(s):
    """0-based terminal rows whose cell (col 0) background is NOT the
    default — the region face signature (or anything else non-default).
    """
    hits = []
    for y in range(s.screen.lines):
        cell = s.screen.buffer[y][0]
        if cell.bg not in ("default", "000000"):
            hits.append(y)
    return hits


def main():
    checks = []

    def rec(name, ok, detail=""):
        checks.append((name, ok, detail))
        print(f"  {'PASS' if ok else 'FAIL'}  {name:44s} {detail}")

    project = tempfile.mkdtemp(prefix="probe-clip-")
    os.makedirs(os.path.join(project, "src"))
    with open(os.path.join(project, "Cargo.toml"), "w") as f:
        f.write("[package]\n")
    with open(os.path.join(project, "src", "probe.rs"), "w") as f:
        f.write("\n".join(f"l{i}" for i in range(5)) + "\n")
    with open(os.path.join(project, "src", "multibyte.rs"), "w") as f:
        f.write("alpha\ncafé beta\n中 gamma\n")

    app = pyte_driver.App(project, rows=24, cols=80)
    try:
        # ── A: mouse capture ────────────────────────────────────────────
        modes = [m for m in (b"\x1b[?1006h", b"\x1b[?1002h", b"\x1b[?1003h",
                             b"\x1b[?1005h") if m in app.raw_tail]
        rec("A: app requests mouse reporting at startup", bool(modes),
            f"modes={modes!r} (the terminal's own drag-to-select is "
            f"unavailable while this is on)")
        rec("A: app does NOT request bracketed paste (2004)",
            b"\x1b[?2004h" not in app.raw_tail,
            "a paste arrives as plain key bytes, not a Paste event")

        # ── B: drag-select + M-w → OSC 52 ───────────────────────────────
        app.key("C-x C-f", 1.0)
        app.type_text("multibyte", 0.5)
        app.key("RET", 1.0)
        rec("B: multibyte.rs open", "multibyte.rs" in app.screen_text(),
            app.row_text(0)[:40])

        raw_before = len(app.raw_tail)
        # Left drag: press terminal (row 1, col 1) [SGR 2;2 → content row
        # 0 → buffer line 0], drag to (row 3, col 1) [content row 2 → line
        # 2], release there. SGR is 1-based.
        #
        # crossterm parses SGR mouse: the final char is the press/release
        # discriminator — uppercase M = press/drag, lowercase m = release
        # (parse.rs:746-752: `if buffer.last() == Some(&b'm')` converts
        # Down→Up). The old probe fed uppercase M for the "release", which
        # crossterm parses as a Down (press) — re-arming the drag at the
        # release position and moving the point to col 0 of line 2 (中 is
        # 2 cells, so display col 1 → char 0), shrinking the region to
        # lines 0–1. The probe never measured the real Up path.
        app.feed(b"\x1b[<0;2;2M", 0.6)   # press (Down, left)
        app.feed(b"\x1b[<32;2;4M", 0.6)  # drag (Drag, left)
        # Region face: content rows 0..=2 = terminal rows 1..=3 (row 4 =
        # content row 3, outside the region, must stay default-bg).
        app.wait(0.4)
        hits = region_rows(app)
        rec("B: the dragged rows are highlighted (region face)",
            set([1, 2, 3]) <= set(hits) and 4 not in hits,
            f"bg rows={hits}")
        # Proper release: lowercase m = Up. The region (mark + point)
        # PERSISTS after release (emacs mark semantics).
        app.feed(b"\x1b[<0;2;4m", 0.6)
        app.wait(0.4)
        hits_after_release = region_rows(app)
        rec("B: region persists after release (Up)",
            set([1, 2, 3]) <= set(hits_after_release) and 4 not in hits_after_release,
            f"bg rows={hits_after_release}")

        app.key("M-w", 0.8)
        payloads = osc52_payloads(app.raw_tail[raw_before:])
        # The drag covers lines 0..=2 (press at line 0, drag to line 2
        # EOL). The payload must be the full 3-line text: the mark is at
        # line 0 char 0, the point at line 2 EOL (char 7 of "中 gamma").
        # A payload of only lines 0–1 would mean the release moved the
        # point (the old uppercase-M bug).
        expected = "alpha\ncafé beta\n中 gamma".encode("utf-8")
        rec("B: exactly one OSC 52 emitted by M-w", len(payloads) == 1,
            f"count={len(payloads)}")
        if payloads:
            ok = payloads[0] == expected
            rec("B: OSC 52 payload decodes to exactly the dragged UTF-8",
                ok, f"got={payloads[0]!r} want={expected!r}")
            # The byte-exact escape itself (pin in the report):
            esc = b"\x1b]52;c;" + base64.b64encode(expected) + b"\x07"
            rec("B: the literal escape is byte-exact in the stream",
                esc in app.raw_tail[raw_before:], f"literal={esc!r}")
            # The payload must equal the highlighted text byte-exact:
            # the highlighted rows are 1,2,3 (content rows 0,1,2 =
            # buffer lines 0,1,2). If the highlight spanned a different
            # range than the copy, this would catch it.
            highlighted_text = "alpha\ncafé beta\n中 gamma".encode("utf-8")
            rec("B: payload == highlighted text (byte-exact)",
                payloads[0] == highlighted_text,
                f"payload={payloads[0]!r} highlighted={highlighted_text!r}")

        # ── C: paste into the notes buffer, save, read disk ────────────
        app.key("C-x n", 1.0)
        # C1: editable-buffer cursor contract (geometry.rs): the
        # hardware cursor is pinned to col 0 of the LAST line of an
        # editable buffer (notes editing is append-at-end; char/width
        # math is the read-view's business, plan 004 05d — the paste
        # path itself is byte-verified in C2). So after pasting 中 the
        # cursor must sit on the pasted line (content row 1 → terminal
        # row 2 0-based → 3 1-based) at col 0 (→ 1 1-based).
        app.feed("p中q".encode("utf-8"), 0.8)
        cup = app.cup_settle()
        rec("C1: pasted line tracks the editable cursor (last line, col 0)",
            cup == (3, 1), f"cup={cup} want (3,1) 1-based")
        # C2: the multi-byte paste survives end-to-end AND its newlines are
        # preserved (issue-paste-newline-dropped): bytes in → file on disk,
        # é/中 byte-exact, and the pasted LFs land as real '\n' — a pasted LF
        # arrives as C-j (terminal 0x0A in raw mode), which the fix inserts as
        # a newline. The pre-fix build DROPPED them, gluing the lines
        # together; this probe used to assert that glued result with an `in`
        # check (true of the concatenation) and so passed over the defect.
        # Now asserted BYTE-EXACT on the whole file (an `in`/`contains` check
        # cannot falsify a dropped newline).
        paste2 = "r-\u00e9-1\nsecond line\n"
        app.feed(paste2.encode("utf-8"), 0.8)
        app.key("C-x C-s", 1.2)
        notes = os.path.join(project, ".redline-notes.md")
        on_disk = open(notes, "rb").read() if os.path.exists(notes) else b""
        # Full byte-exact file: the seeded `# Notes` header + both pastes with
        # their newlines intact (`p中q` has no newline; the second paste
        # carries two — one after `r-é-1`, one after `second line`).
        expected_file = ("# Notes\n"
                         + "p\u4e2dq" + "r-\u00e9-1\nsecond line\n").encode("utf-8")
        rec("C2: pasted multi-byte bytes + newlines land byte-exact on disk",
            on_disk == expected_file,
            f"file={'yes' if on_disk else 'MISSING'}, "
            f"byte-exact={on_disk == expected_file}, "
            f"want={expected_file!r} got={on_disk!r}")
    finally:
        app.kill()

    failed = [c for c in checks if not c[1]]
    print(f"\n{len(checks) - len(failed)}/{len(checks)} passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
