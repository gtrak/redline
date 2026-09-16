//! The open-buffer set: ropey-backed text buffers, the open-buffer
//! table (MRU order, current buffer), and cheap line/byte access.
//!
//! `Buffer.rope` is a `ropey::Rope` (O(1) clone, O(log N) line
//! access). `BufferTable` keeps the same public API from issue 02
//! (insert / get / get_mut / list / set_current / current /
//! current_buffer / kill / len).
//!
//! `mtime` is the file's modification time at load; used for highlight
//! cache invalidation on reopen (issue 04 adds live watcher invalidation).

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ropey::Rope;

/// Display name of the scratch buffer.
pub const SCRATCH_NAME: &str = "*scratch*";

/// Files larger than this (in bytes) skip highlighting and render as
/// plain text (graceful degradation, never a hang).
pub const BIG_FILE_THRESHOLD: usize = 10 * 1024 * 1024; // 10 MiB

/// One open buffer. `path` is `None` for the scratch buffer; the text
/// is ropey-backed (O(1) clone, O(log N) line access).
#[derive(Clone)]
pub struct Buffer {
    pub path: Option<PathBuf>,
    pub rope: Rope,
    /// File modification time at load (used for cache invalidation).
    /// `UNIX_EPOCH` for the scratch buffer or on stat failure.
    pub mtime: SystemTime,
    /// Whether the buffer is editable (issue 03: read-only for files;
    /// scratch is editable).
    pub editable: bool,
}

impl std::fmt::Debug for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Buffer")
            .field("path", &self.path)
            .field("lines", &self.rope.len_lines())
            .field("bytes", &self.rope.len_bytes())
            .field("editable", &self.editable)
            .finish()
    }
}

impl Buffer {
    /// Create a buffer from a `Rope` and optional path/mtime.
    pub fn new(path: Option<PathBuf>, rope: Rope, mtime: SystemTime, editable: bool) -> Self {
        Self {
            path,
            rope,
            mtime,
            editable,
        }
    }

    /// The number of lines in the buffer.
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// The number of bytes in the buffer.
    #[allow(dead_code)] // public API: spec-required byte↔line conversion
    pub fn byte_count(&self) -> usize {
        self.rope.len_bytes()
    }

    /// True when the file is large enough to skip highlighting.
    pub fn is_big(&self) -> bool {
        self.rope.len_bytes() > BIG_FILE_THRESHOLD
    }

    /// The text of line `line_idx` (without the trailing newline).
    /// Returns `None` when the line index is out of bounds.
    ///
    /// Fast path: when the line is contiguous within a single ropey chunk
    /// (~1 KiB), the result borrows from the rope (O(1)). When the line
    /// crosses a chunk boundary, it allocates (O(line length)).
    pub fn line_text(&self, line_idx: usize) -> Option<Cow<'_, str>> {
        let ls = self.rope.get_line(line_idx)?;
        // Fast path: contiguous slice → borrow directly.
        if let Some(s) = ls.as_str() {
            let stripped = s
                .strip_suffix("\r\n")
                .or_else(|| s.strip_suffix('\n'))
                .unwrap_or(s);
            return Some(Cow::Borrowed(stripped));
        }
        // Slow path: line crosses a chunk boundary → allocate.
        let s: String = ls.into();
        let stripped = s
            .strip_suffix("\r\n")
            .or_else(|| s.strip_suffix('\n'))
            .unwrap_or(&s);
        Some(Cow::Owned(stripped.to_string()))
    }

    /// The byte offset of the start of line `line_idx`.
    /// Panics when `line_idx >= len_lines()`; use `try_line_to_byte`
    /// in any path that can be fed untrusted indices.
    #[allow(dead_code)] // public API: spec-required byte↔line conversion
    pub fn line_to_byte(&self, line_idx: usize) -> usize {
        self.rope.line_to_byte(line_idx)
    }

    /// The line number of the byte at `byte_idx`.
    /// Panics when `byte_idx > len_bytes()`; use `try_byte_to_line` in
    /// any path that can be fed untrusted indices.
    #[allow(dead_code)] // public API: spec-required byte↔line conversion
    pub fn byte_to_line(&self, byte_idx: usize) -> usize {
        self.rope.byte_to_line(byte_idx)
    }

    /// Non-panicking `line_to_byte`; `None` when out of bounds.
    pub fn try_line_to_byte(&self, line_idx: usize) -> Option<usize> {
        self.rope.try_line_to_byte(line_idx).ok()
    }

    /// Non-panicking `byte_to_line`; `None` when out of bounds.
    pub fn try_byte_to_line(&self, byte_idx: usize) -> Option<usize> {
        self.rope.try_byte_to_line(byte_idx).ok()
    }

    /// The full text as a `String` (O(N); prefer `line_text` for
    /// per-line access).
    pub fn text(&self) -> String {
        self.rope.to_string()
    }
}

/// The open-buffer set. Keys are `*scratch*` or the buffer's absolute
/// path as a string (absolute paths keep two projects' same-named
/// files distinct). `order` is most-recently-used first.
#[derive(Debug)]
pub struct BufferTable {
    by_key: HashMap<String, Buffer>,
    order: Vec<String>,
    current: Option<String>,
}

impl BufferTable {
    /// A table holding only a fresh `*scratch*` buffer (current, editable).
    pub fn new() -> Self {
        let mut t = Self {
            by_key: HashMap::new(),
            order: Vec::new(),
            current: None,
        };
        let key = t.insert_rope(
            None,
            Rope::new(),
            SystemTime::UNIX_EPOCH,
            true,
        );
        t.current = Some(key);
        t
    }

    /// The key of a buffer: the scratch sentinel or its absolute path.
    pub fn key_for(path: &Option<PathBuf>) -> String {
        match path {
            Some(p) => p.to_string_lossy().into_owned(),
            None => SCRATCH_NAME.to_string(),
        }
    }

    /// Insert (or replace) a buffer from a `String` and make it
    /// most-recently-used. `editable` is `false` for files, `true`
    /// for scratch.
    pub fn insert(&mut self, path: Option<PathBuf>, text: String) -> String {
        let editable = path.is_none();
        let mtime = path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        self.insert_rope(path, Rope::from_str(&text), mtime, editable)
    }

    /// Insert (or replace) a buffer from a `Rope` and make it
    /// most-recently-used.
    pub fn insert_rope(
        &mut self,
        path: Option<PathBuf>,
        rope: Rope,
        mtime: SystemTime,
        editable: bool,
    ) -> String {
        let key = Self::key_for(&path);
        let is_new = !self.by_key.contains_key(&key);
        self.by_key
            .insert(key.clone(), Buffer::new(path, rope, mtime, editable));
        if is_new {
            self.order.push(key.clone());
        }
        self.touch_internal(&key);
        key
    }

    /// Move an existing buffer to the MRU front (a visit).
    pub fn touch(&mut self, key: &str) {
        self.touch_internal(key);
    }

    fn touch_internal(&mut self, key: &str) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
            self.order.insert(0, key.to_string());
        }
    }

    pub fn get(&self, key: &str) -> Option<&Buffer> {
        self.by_key.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Buffer> {
        self.by_key.get_mut(key)
    }

    /// All buffers, most-recently-used first.
    pub fn list(&self) -> Vec<(&str, &Buffer)> {
        self.order
            .iter()
            .filter_map(|k| self.by_key.get(k).map(|b| (k.as_str(), b)))
            .collect()
    }

    /// Make the buffer with `key` current (also a visit); ignored for
    /// unknown keys.
    pub fn set_current(&mut self, key: &str) {
        if self.by_key.contains_key(key) {
            self.current = Some(key.to_string());
            self.touch(key);
        }
    }

    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    pub fn current_buffer(&self) -> Option<&Buffer> {
        self.current.as_deref().and_then(|k| self.by_key.get(k))
    }

    /// Remove a buffer from the set; `true` if it existed. If it was
    /// current, there is no current buffer afterwards.
    pub fn kill(&mut self, key: &str) -> bool {
        if self.by_key.remove(key).is_some() {
            self.order.retain(|k| k != key);
            if self.current.as_deref() == Some(key) {
                self.current = None;
            }
            true
        } else {
            false
        }
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }
}

/// Load a file into a `Rope` via `Rope::from_reader` (O(N), streaming).
/// Returns the rope and the file's mtime.
pub fn load_file(path: &Path) -> std::io::Result<(Rope, SystemTime)> {
    let metadata = std::fs::metadata(path)?;
    let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let file = std::fs::File::open(path)?;
    let rope = Rope::from_reader(file)?;
    Ok((rope, mtime))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(path: &str) -> Option<PathBuf> {
        Some(PathBuf::from(path))
    }

    #[test]
    fn new_table_has_current_scratch() {
        let t = BufferTable::new();
        assert_eq!(t.len(), 1);
        assert_eq!(t.current(), Some(SCRATCH_NAME));
        let buf = t.current_buffer().unwrap();
        assert!(buf.path.is_none());
        assert!(buf.editable);
        assert_eq!(buf.line_count(), 1); // empty rope → 1 line
    }

    #[test]
    fn insert_and_get_buffers() {
        let mut t = BufferTable::new();
        let k = t.insert(key("/p/src/a.rs"), "fn a() {}\n".into());
        assert_eq!(k, "/p/src/a.rs");
        let buf = t.get(&k).unwrap();
        assert_eq!(buf.line_text(0).as_deref(), Some("fn a() {}"));
        assert_eq!(buf.line_count(), 2); // "fn a() {}\n" → 2 lines
    }

    #[test]
    fn switching_buffers_updates_current_and_mru() {
        let mut t = BufferTable::new();
        let a = t.insert(key("/p/a.rs"), "a".into());
        let b = t.insert(key("/p/b.rs"), "b".into());
        let order: Vec<&str> = t.list().iter().map(|(k, _)| *k).collect();
        assert_eq!(order, vec![b.as_str(), a.as_str(), SCRATCH_NAME]);

        t.set_current(&a);
        assert_eq!(t.current(), Some(a.as_str()));
        let order: Vec<&str> = t.list().iter().map(|(k, _)| *k).collect();
        assert_eq!(order, vec![a.as_str(), b.as_str(), SCRATCH_NAME]);
    }

    #[test]
    fn kill_removes_from_set_and_clears_current() {
        let mut t = BufferTable::new();
        let a = t.insert(key("/p/a.rs"), "a".into());
        t.set_current(&a);
        assert!(t.kill(&a));
        assert!(t.get(&a).is_none());
        assert_eq!(t.current(), None);
        assert_eq!(t.len(), 1);
        assert!(!t.kill("/p/nope.rs"));
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn killing_non_current_buffer_keeps_current() {
        let mut t = BufferTable::new();
        let a = t.insert(key("/p/a.rs"), "a".into());
        let b = t.insert(key("/p/b.rs"), "b".into());
        t.set_current(&a);
        t.kill(&b);
        assert_eq!(t.current(), Some(a.as_str()));
    }

    #[test]
    fn insert_replaces_existing_buffer_text() {
        let mut t = BufferTable::new();
        let k = t.insert(key("/p/a.rs"), "old".into());
        t.insert(key("/p/a.rs"), "new".into());
        assert_eq!(t.len(), 2, "scratch + one buffer");
        assert_eq!(t.get(&k).unwrap().line_text(0).as_deref(), Some("new"));
    }

    #[test]
    fn same_relative_path_different_projects_are_distinct() {
        let mut t = BufferTable::new();
        let a = t.insert(key("/p1/src/main.rs"), "one".into());
        let b = t.insert(key("/p2/src/main.rs"), "two".into());
        assert_ne!(a, b);
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn line_count_and_line_text() {
        let mut t = BufferTable::new();
        let k = t.insert(key("/p/a.rs"), "line1\nline2\nline3\n".into());
        let buf = t.get(&k).unwrap();
        assert_eq!(buf.line_count(), 4); // trailing \n → 4 lines
        assert_eq!(buf.line_text(0).as_deref(), Some("line1"));
        assert_eq!(buf.line_text(1).as_deref(), Some("line2"));
        assert_eq!(buf.line_text(2).as_deref(), Some("line3"));
        assert_eq!(buf.line_text(3).as_deref(), Some("")); // trailing empty line
        assert_eq!(buf.line_text(4).as_deref(), None); // out of bounds
    }

    #[test]
    fn byte_to_line_roundtrip() {
        let mut t = BufferTable::new();
        let k = t.insert(key("/p/a.rs"), "abc\ndef\nghi\n".into());
        let buf = t.get(&k).unwrap();
        // "abc\ndef\nghi\n"
        //  012345678...
        // byte 0 → line 0, byte 3 → line 0 (the \n), byte 4 → line 1
        assert_eq!(buf.byte_to_line(0), 0);
        assert_eq!(buf.byte_to_line(3), 0); // '\n' belongs to line 0
        assert_eq!(buf.byte_to_line(4), 1); // 'd'
        assert_eq!(buf.byte_to_line(7), 1); // '\n' of line 1
        assert_eq!(buf.byte_to_line(8), 2); // 'g'
    }

    #[test]
    fn line_to_byte_roundtrip() {
        let mut t = BufferTable::new();
        let k = t.insert(key("/p/a.rs"), "abc\ndef\nghi\n".into());
        let buf = t.get(&k).unwrap();
        assert_eq!(buf.line_to_byte(0), 0);
        assert_eq!(buf.line_to_byte(1), 4);
        assert_eq!(buf.line_to_byte(2), 8);
        // Roundtrip: byte_to_line(line_to_byte(i)) == i for valid lines.
        for line in 0..3 {
            assert_eq!(buf.byte_to_line(buf.line_to_byte(line)), line);
        }
    }

    #[test]
    fn try_line_to_byte_out_of_bounds() {
        let mut t = BufferTable::new();
        let k = t.insert(key("/p/a.rs"), "abc\n".into());
        let buf = t.get(&k).unwrap();
        // "abc\n" has len_lines() == 2; ropey allows index up to len_lines()
        // (the trailing empty line's start is at the end-of-file byte).
        assert_eq!(buf.try_line_to_byte(0), Some(0));
        assert_eq!(buf.try_line_to_byte(1), Some(4));
        assert_eq!(buf.try_line_to_byte(2), Some(4)); // == len_lines(), end-of-file
        assert_eq!(buf.try_line_to_byte(3), None);     // truly out of bounds
    }

    #[test]
    fn try_byte_to_line_out_of_bounds() {
        let mut t = BufferTable::new();
        let k = t.insert(key("/p/a.rs"), "abc\n".into());
        let buf = t.get(&k).unwrap();
        assert_eq!(buf.try_byte_to_line(0), Some(0));
        assert_eq!(buf.try_byte_to_line(3), Some(0)); // the \n
        assert_eq!(buf.try_byte_to_line(4), Some(1)); // start of line 1
        assert_eq!(buf.try_byte_to_line(100), None);  // out of bounds
    }

    #[test]
    fn large_file_detection() {
        // Build a rope > 10 MiB in memory (no file I/O).
        let chunk = "x".repeat(1024);
        let mut rope = Rope::new();
        for _ in 0..(BIG_FILE_THRESHOLD / 1024 + 1) {
            rope.insert(rope.len_chars(), &chunk);
            rope.insert(rope.len_chars(), "\n");
        }
        let buf = Buffer::new(
            None,
            rope,
            SystemTime::UNIX_EPOCH,
            false,
        );
        assert!(buf.is_big(), "should be flagged as big");
        assert!(buf.byte_count() > BIG_FILE_THRESHOLD);

        // A small buffer is not big.
        let small = Buffer::new(
            None,
            Rope::from_str("hello"),
            SystemTime::UNIX_EPOCH,
            false,
        );
        assert!(!small.is_big());
    }

    #[test]
    fn load_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let (rope, mtime) = load_file(&path).unwrap();
        assert_eq!(Rope::from_str("fn main() {}\n"), rope);
        assert!(mtime >= SystemTime::UNIX_EPOCH);
    }

    #[test]
    fn load_file_missing_path_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.rs");
        assert!(load_file(&path).is_err());
    }

    #[test]
    fn buffer_text_roundtrip() {
        let mut t = BufferTable::new();
        let k = t.insert(key("/p/a.rs"), "hello\nworld\n".into());
        let buf = t.get(&k).unwrap();
        assert_eq!(buf.text(), "hello\nworld\n");
    }

    #[test]
    fn crlf_lines_are_stripped() {
        let mut t = BufferTable::new();
        let k = t.insert(key("/p/a.rs"), "line1\r\nline2\r\n".into());
        let buf = t.get(&k).unwrap();
        assert_eq!(buf.line_text(0).as_deref(), Some("line1"));
        assert_eq!(buf.line_text(1).as_deref(), Some("line2"));
    }

    #[test]
    fn multi_chunk_lines_not_blank() {
        // Regression: lines that cross a ropey ~1 KiB chunk boundary must
        // not render as empty. Build a buffer > 2 KiB so that at least one
        // line boundary falls inside a chunk, then verify every line is
        // non-empty (except the trailing empty line from the final \n).
        let line_content = "x".repeat(997); // 997 bytes per line
        let num_lines = 4; // 4 * (997+1) = 3992 bytes > 2 KiB
        let text: String = (0..num_lines)
            .map(|i| format!("line{}:{}", i, line_content))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let rope = Rope::from_str(&text);
        let buf = Buffer::new(None, rope, SystemTime::UNIX_EPOCH, false);
        for line in 0..num_lines {
            let lt = buf.line_text(line).expect("in-bounds line must be Some");
            assert!(!lt.is_empty(), "line {} must not be blank", line);
            assert!(
                lt.starts_with(&format!("line{}:", line)),
                "line {} should start with its label, got: {}",
                line,
                &lt[..20]
            );
        }
        // The trailing empty line after the final \n.
        assert_eq!(buf.line_text(num_lines).as_deref(), Some(""));
    }
}
