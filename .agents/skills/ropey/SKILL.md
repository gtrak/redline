---
name: ropey
description: >-
  Reference for ropey 1.6.1, the utf8 text rope used as redline's buffer text
  model. Covers the Rope/RopeSlice core types, char/byte/line index conversion,
  insert/remove, slicing, iteration (Lines/Chars/Chunks), error types,
  cheap-clone/Cow semantics, and large-file performance characteristics.
  All APIs verified against docs.rs (ropey 1.6.1).
---

# ropey

`ropey` is a utf8 text rope. Its atomic unit is a Unicode scalar (`char`); all
editing and slicing is done in **char indices**, which prevents creating
invalid utf8. Core components: `Rope` (main type), `RopeSlice` (immutable view
into a `Rope`), `iter` (Lines/Chars/Chunks/Bytes iterators), `RopeBuilder`
(incremental builder), and `str_utils`.

## Version

`ropey = "1.6.1"` (pinned in `Cargo.toml` under `# ── text model ──`). It is the
buffer text model backing redline's editable buffers (light editing only) and
the read-only file text. Pinned because ropey's API is stable (100% documented,
O(log N) guarantees) and the plan targets a single known version.

## Core API

All verified against https://docs.rs/ropey/1.6.1/ropey/.

### Construction & I/O
- `Rope::new() -> Rope` — empty rope.
- `Rope::from_str(text: &str) -> Rope` — O(N).
- `Rope::from_reader(reader: impl Read) -> Result<Rope>` — O(N); returns an
  `std::io::Error` with kind `InvalidData` on non-utf8 input.
- `rope.write_to(writer: impl Write) -> Result<()>` — O(N) `std::io::Result`.
- `From` impls: `Rope` from `&str`, `String`, `Cow<'a, str>`; `Rope` into
  `String`/`Cow<'a, str>`; `RopeSlice<'a>` into `Rope`/`String`/`Cow<'a, str>`.

### Lengths (all O(1))
- `len_bytes()`, `len_chars()`, `len_lines()`, `len_utf16_cu() -> usize`.
  Note: `len_lines()` counts lines, so a file ending in `\n` has a trailing
  empty final line (e.g. `"a\n"` → 2 lines).

### Index conversions (all O(log N), panic when out of bounds)
- `line_to_char(line_idx) -> usize` / `line_to_byte(line_idx) -> usize`
- `char_to_line(char_idx) -> usize` / `char_to_byte(char_idx) -> usize`
- `byte_to_char(byte_idx) -> usize` / `byte_to_line(byte_idx) -> usize`
- `char_to_utf16_cu` / `utf16_cu_to_char` (utf16 interop, rarely needed).
- Non-panicking twins: `try_*` return `Result<usize>` (error = `ropey::Error`),
  and `get_*` return `Option<_>`.

### Editing (char-index based)
- `insert(char_idx: usize, text: &str)` — O(M + log N); panics if
  `char_idx > len_chars()`.
- `insert_char(char_idx: usize, ch: char)` — O(log N); same bound.
- `remove<R: RangeBounds<usize>>(char_range: R)` — O(M + log N); panics if the
  range is reversed or `end > len_chars()`.
- `split_off(char_idx) -> Rope` — returns the right part, O(log N).
- `append(other: Rope)` — O(log N).
- Non-panicking: `try_insert`, `try_insert_char`, `try_remove`, `try_split_off`
  all return `ropey::Result<()>` / `Result<Rope>`.

### Slicing & line access
- `slice<R: RangeBounds<usize>>(char_range) -> RopeSlice<'_>` — O(log N).
- `byte_slice(byte_range) -> RopeSlice<'_>` — also panics if not char-aligned.
- `line(line_idx) -> RopeSlice<'_>` — O(log N); returns the line **including**
  its trailing line break. `get_line(line_idx) -> Option<RopeSlice<'_>>`.
- `get_slice(char_range) -> Option<RopeSlice<'_>>` — non-panicking.

### Iterators (`ropey::iter`)
- `lines() -> Lines` / `lines_at(line_idx) -> Lines` — each item is a
  `RopeSlice` (includes trailing line break).
- `chars() -> Chars` / `chars_at(char_idx) -> Chars`.
- `bytes() -> Bytes`, `chunks() -> Chunks` (contiguous `&str` chunks).
- Iterators are bidirectional cursors: `next()`/`prev()`, and `reverse()`
  (in-place, unlike std's `rev()`). `*_at(pos)` positions so `next()` yields
  the element at `pos`, `prev()` the one before.

### Cheap clone & Cow semantics
- `Rope: Clone` is O(1) with data sharing (thread-safe; clones diverge
  incrementally on edit). Ideal for spawning async saves while the UI edits.
- `RopeSlice<'a>` is `Copy` (O(1), borrows the rope).
- `as_str(&self) -> Option<&'a str>` — O(1); returns `None` for large slices
  that cross chunk boundaries.
- `impl From<RopeSlice<'a>> for Cow<'a, str>` — borrows when contiguous, else
  allocates (best O(1), worst O(N)). Prefer this over `to_string()` to avoid
  unnecessary copies.

### Getting a line efficiently (FileView pattern)
```rust
use ropey::Rope;

fn line_text(rope: &Rope, line: usize) -> Option<&str> {
    // get_line returns None if line >= len_lines()
    // as_str returns None only when the line spans multiple chunks (very long lines)
    rope.get_line(line).and_then(|ls| ls.as_str())
}

fn line_range(rope: &Rope, line: usize) -> Option<(usize, usize)> {
    // char offsets of the line's [start, end)
    match (rope.try_line_to_char(line), rope.try_line_to_char(line + 1)) {
        (Ok(s), Ok(e)) => Some((s, e)),
        _ => None,
    }
}
```
For a visible viewport, iterate `rope.lines_at(top_line)` and take N lines
rather than re-resolving each line by index.

## Usage in redline

- **`src/model/buffer.rs` text model**: wrap a `Rope` per open buffer. Keep
  `len_lines()`/`len_chars()` for status; use `get_line` + `as_str`/`Cow` to
  materialize lines for rendering. An `editable: bool` flag gates whether
  `insert`/`remove` are exposed (commit messages, notes, explicitly-opened
  files only).
- **FileView line cache**: cache materialized line text keyed by line number;
  invalidate a range after an edit by recomputing
  `char_to_line(old_char)`..`char_to_line(new_char)` and refetching those
  lines. Because `Rope` clones are O(1), a background save can clone the rope
  and `write_to` in another thread without blocking edits.
- Scroll anchor on reload: record `char_to_line`/`line_to_char` before
  replacing the rope, re-resolve the same line after.

## Gotchas

- **Indexing units**: `insert`/`remove`/`slice`/`line_to_char` are all
  **char-index** based, not byte. `byte_slice` is the only byte-based slicing
  and additionally panics if the range is not on a char boundary.
- **Line count vs. trailing newline**: `len_lines()` counts a final empty line
  when the text ends in a line break. Don't assume `len_lines() ==` the number
  of `\n` + content lines.
- **Panic on out-of-bounds**: `line(i)` panics when `i >= len_lines()`;
  `insert`/`remove` panic when past `len_chars()`. Use `get_*`/`try_*` in any
  path that can be fed untrusted indices (e.g. scroll restoration after a
  file shrank).
- **`line()` includes the trailing newline**: when stripping for display, drop
  the trailing `\n`/`\r\n`; the line break belongs to the line it ends.
- **Line-break recognition**: default features recognize LF and CRLF; the
  default `unicode_lines` feature (which implies `cr_lines`) also recognizes
  VT, FF, NEL, U+2028/U+2029. Disabling default features to tune line breaks
  also disables the `simd` feature — re-enable `simd` explicitly if desired.
- **Error type**: `ropey::Error` is `#[non_exhaustive]` (variants like
  `CharIndexOutOfBounds`, `CharRangeOutOfBounds`, `ByteRangeNotCharBoundary`);
  always add a wildcard arm when matching. `ropey::Result<T>` is
  `Result<T, ropey::Error>`.
- **Large-file performance**: worst case O(log N) for nearly all edits/queries,
  designed for GB-scale and pathological single-line files. `chunk_at_*`
  methods are the fastest calls (sub-100ns for ~200kB docs) — use them (or
  `str_utils`) for bulk low-level work instead of per-char loops.
- **`shrink_to_fit`** on a clone stops data sharing and can *increase* total
  memory; don't call it on a clone you still want to share.
