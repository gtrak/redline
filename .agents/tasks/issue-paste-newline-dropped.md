# issue-paste-newline-dropped: a pasted newline is silently dropped when editing a note

**Found by the `clipboard-selection` gate (P2-2), measured independently — pre-existing, not introduced by that lane.**

Pasting multi-line text into the notes buffer drops every newline. The gate measured it while checking
the lane's paste claim: after pasting two lines, `.redline-notes.md` ended `…e\xcc\x81asecond line` —
the two lines were concatenated, with no newline between them. The lane's probe asserted
`want_line1 in on_disk and b"second line" in on_disk`, which is true of the concatenated result, so the
defect passed a probe that was supposed to be answering "does paste work?".

**Cause (measured, not inferred):** `RET` is unbound while editing a note in Annotation mode —
`src/app/store/keys.rs:461-512` handles `Enter` only when the buffer is `accurate`. Notes are
single-line by design, so the *insert* path for a newline was never wired up.

**Why it is worth fixing rather than documenting:** the notes format is line-oriented
(`path:line:col` records with `note:` fields), so a multi-line note would corrupt the document — but
that is an argument for **splitting** a pasted multi-line string into the records it can represent, or
for refusing it **visibly**, not for silently gluing the lines together. Silent concatenation is the
worst option: the user pastes text and gets something they did not write, with no signal.

**Decide and state one of:**
- a pasted newline is represented as a space (visible, predictable), or
- the paste is refused with a message, or
- multi-line notes become representable (a format change — largest, needs its own issue).

**Acceptance:** whatever the rule is, it is asserted **byte-exactly** (the current probe's substring
`in` check must not be able to pass over a wrong result), the paste is verified end-to-end into the
file on disk, and the behaviour is the same for a paste as for typing the same characters by hand.
