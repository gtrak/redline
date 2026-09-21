use super::{Annotation, NotesEntry, SyntaxAnchor, NOTES_BEGIN, NOTES_END, NOTES_RECORD_START};

/// The parsed `.redline-notes.md` document (plan 005 issue 02): free text
/// BEFORE the structured annotation section, the section's entries, and
/// free text AFTER it. Everything outside the section is preserved
/// verbatim on re-serialization.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotesDoc {
    pub before: Vec<String>,
    pub entries: Vec<NotesEntry>,
    pub after: Vec<String>,
}

/// Parse a notes file's text into a `NotesDoc` (plan 005 issue 02):
/// free text outside the structured section is preserved verbatim; a
/// record block (`[annotation]` + `key: value` lines) parses into an
/// `Annotation` when the required fields (`path`, `line`, `anchor`,
/// `note`) are present and well-formed; every other block (malformed
/// records, stray lines) is kept verbatim as a `NotesEntry::Raw`, never
/// dropped. No markers at all → the whole file is `before` (untouched).
pub fn parse_notes(text: &str) -> NotesDoc {
    let mut lines: Vec<&str> = text.split('\n').collect();
    // A trailing newline is a line terminator, not an empty last line:
    // drop the final empty element so round-trips don't accumulate
    // blank lines.
    if text.ends_with('\n') && lines.last() == Some(&"") {
        lines.pop();
    }
    let begin = lines
        .iter()
        .position(|l| l.trim() == NOTES_BEGIN)
        .and_then(|b| {
            lines[b + 1..]
                .iter()
                .position(|l| l.trim() == NOTES_END)
                .map(|e| (b, b + 1 + e))
        });
    let Some((b, e)) = begin else {
        return NotesDoc {
            before: lines.iter().map(|s| s.to_string()).collect(),
            entries: Vec::new(),
            after: Vec::new(),
        };
    };
    NotesDoc {
        before: lines[..b].iter().map(|s| s.to_string()).collect(),
        entries: parse_notes_section(&lines[b + 1..e]),
        after: lines[e + 1..].iter().map(|s| s.to_string()).collect(),
    }
}

/// Parse the section body into entries: record blocks (`[annotation]`
/// through the next `[annotation]` / end) that carry every required field
/// become `Record`s; anything else is a verbatim `Raw` block (malformed
/// records are kept, never dropped).
fn parse_notes_section(lines: &[&str]) -> Vec<NotesEntry> {
    // Split into (is_record_start, verbatim text) blocks: a block begins at
    // every record start and runs to the next record start.
    let starts: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.trim() == NOTES_RECORD_START)
        .map(|(i, _)| i)
        .collect();
    let mut entries = Vec::new();
    // Stray lines before the first record block → one verbatim block.
    if !starts.is_empty() && starts[0] > 0 {
        entries.push(NotesEntry::Raw(lines[..starts[0]].join("\n")));
    }
    for (i, start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(lines.len());
        let block = &lines[*start..end];
        match parse_record_block(block) {
            Some(rec) => entries.push(NotesEntry::Record(rec)),
            None => entries.push(NotesEntry::Raw(block.join("\n"))),
        }
    }
    entries
}

/// Parse one `[annotation]` block into an `Annotation`. `None` when a
/// required field (`path`, `line`, `anchor`, `note`) is missing or
/// ill-formed (the caller keeps the block verbatim).
fn parse_record_block(block: &[&str]) -> Option<Annotation> {
    let mut rec = Annotation::default();
    let mut have = [false; 4]; // path, line, anchor, note
    // The syntax anchor's keys (plan 007 issue 02) are OPTIONAL: both must
    // be present for `syntax: Some`; either missing → `None` (legacy).
    let mut syntax_kind: Option<String> = None;
    let mut syntax_name: Option<String> = None;
    for line in &block[1..] {
        // Lines without a `:` (stray text, blank lines) are skipped, not
        // fatal: a record stays valid as long as the required fields are
        // present (tolerant parse — hand-edited blocks survive).
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        // Value: the remainder after the first ':' with one leading space
        // stripped (anchor text is otherwise exact — it is the re-anchor
        // key, so it must round-trip byte-for-byte).
        let value = value.strip_prefix(' ').unwrap_or(value);
        match key {
            "path" => {
                rec.path = value.to_string();
                have[0] = true;
            }
            "line" => {
                rec.line = value.trim().parse::<usize>().ok()?;
                have[1] = true;
            }
            "col" => {
                // `col` is optional metadata (not an anchor field): a
                // malformed value defaults to 0 rather than demoting the
                // whole record to raw.
                rec.col = value.trim().parse::<usize>().unwrap_or(0);
            }
            "anchor" => {
                rec.anchor = value.to_string();
                have[2] = true;
            }
            "note" => {
                rec.text = value.to_string();
                have[3] = true;
            }
            "orphaned" => {
                rec.orphaned = value.trim() == "true";
            }
            "syntax_kind" => {
                syntax_kind = Some(value.to_string());
            }
            "syntax_name" => {
                syntax_name = Some(value.to_string());
            }
            // Unknown keys inside a record block: the block stays valid
            // (forward compatibility), they are simply not re-emitted.
            _ => {}
        }
    }
    if have.iter().all(|h| *h) {
        // The syntax anchor is OPTIONAL: absent keys (legacy records) and a
        // half-written pair (a hand-edited `syntax_kind` without a
        // `syntax_name`) both degrade to `None` — the record stays valid
        // (tolerant parse), the anchor simply does not half-fire.
        rec.syntax = match (syntax_kind, syntax_name) {
            (Some(kind), Some(name)) => Some(SyntaxAnchor { kind, name }),
            _ => None,
        };
        Some(rec)
    } else {
        None
    }
}

/// Serialize a `NotesDoc` back to file text (plan 005 issue 02): free text
/// outside the section verbatim, records in canonical form, raw blocks
/// verbatim, in the entries' order.
pub fn serialize_notes(doc: &NotesDoc) -> String {
    let mut out = String::new();
    for line in &doc.before {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(NOTES_BEGIN);
    out.push('\n');
    for entry in &doc.entries {
        match entry {
            NotesEntry::Record(a) => {
                out.push_str(NOTES_RECORD_START);
                out.push('\n');
                out.push_str(&format!("path: {}\n", a.path));
                out.push_str(&format!("line: {}\n", a.line));
                out.push_str(&format!("col: {}\n", a.col));
                out.push_str(&format!("anchor: {}\n", a.anchor));
                out.push_str(&format!("note: {}\n", a.text));
                out.push_str(&format!("orphaned: {}\n", a.orphaned));
                // The syntax keys are emitted ONLY when present, appended
                // after `orphaned` (additive): a record without a syntax
                // anchor serializes byte-identically to the pre-007-02
                // shape (no migration of legacy files).
                if let Some(sa) = &a.syntax {
                    out.push_str(&format!("syntax_kind: {}\n", sa.kind));
                    out.push_str(&format!("syntax_name: {}\n", sa.name));
                }
            }
            NotesEntry::Raw(s) => {
                out.push_str(s);
                out.push('\n');
            }
        }
    }
    out.push_str(NOTES_END);
    out.push('\n');
    for line in &doc.after {
        out.push_str(line);
        out.push('\n');
    }
    out
}
