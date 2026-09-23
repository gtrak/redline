# issue-redo-live-leg — no PTY leg covers redo or `C-x U`

**Found by:** the `016-04` gate (its item 3, which I asked it to confirm rather than assume).

## The gap

```
$ rg -n "C-x U|redo" tools/*.py
tools/fn_survey.py:8:Why this exists as a file rather than an inline heredoc: …
```

That single hit is a **false positive** ("he**redo**c"). The **undo** leg exists
(`tools/drive_redline_battery2.py:40` sends `\x1f` = `C-/`), so redo — a **new user-facing binding**
with unit tests and no drive — is exactly the *"a lane that changes behaviour owns the drives"* gap.
Neither `C-x U` nor the second binding has ever been exercised through a real terminal.

## The minimal leg (specified by the gate, with the oracle-measured expectation)

Launch → open an editable buffer → type a run (e.g. `abc`) → send `C-x u` → assert the text reverts →
send `C-x U` → assert the text is restored **byte-identically**.

Oracle (emacs 30.2, measured): for `abc <left> def` → undo → redo the text is `abdefc` and the point
stays at **3** — the start of the redone insertion, which is what the lane's
`land_point_at_char(key, start)` does. Pin that, not just "the text came back".

Then **register it**: `tools/gate.sh` `SHARED_SUITES` **and** `tools/pool.py` `BATTERY`. An
unregistered drive is outside the battery — the same mistake the symbol-precise lane made, where 27
legs were reproducible only by a manual run and "gate.sh full OK" never covered them.

## The `C-M-7` caveat (record this where a user would look)

`C-M-7` is the second redo binding, justified because crossterm 0.29 decodes `ESC 0x1F` as
`Char('7') + CONTROL` with ALT OR'd in — the gate verified this in the vendored parser and confirmed
the `0x1F` arm is **unconditional** (unlike bare `SHIFT`, which needed flags iocraft never sets, so
this is not that failure mode). But it **requires the terminal to send Alt as an ESC prefix**. Where
Meta is not "send escape", Alt+Ctrl+_ arrives as bare `0x1F` → `C-7` → which is **undo**, not redo.
The primary `C-x U` is unaffected, and on a CSI-u/kitty terminal the same physical key arrives as
`C-M-_` proper, deliberately unbound. A one-line note in the keymap docs and the README's key list
would save a user a confusing moment.

## Acceptance

- A PTY leg exists for redo, driven by `C-x U`, asserting both the text and the cursor position.
- It is registered in `SHARED_SUITES` and `BATTERY`, and **runs in `gate.sh full`** (verify by
  grepping the battery log for it, not by trusting the list).
- It fails if redo is unbound, or lands the text or the cursor wrong.
- The `C-M-7` meta-sends-escape caveat is recorded in user-facing docs.
