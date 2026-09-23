//! The open-buffer set: ropey-backed text buffers, the open-buffer
//! table (MRU order, current buffer), and cheap line/byte access.
//!
//! `Buffer.rope` is a `ropey::Rope` (O(1) clone, O(log N) line
//! access). `BufferTable` keeps the same public API from issue 02
//! (insert / get / get_mut / list / set_current / current /
//! current_buffer / kill / len).
//!
//! `mtime` is the file's modification time at load; used for highlight
//! cache invalidation on reopen and for live watcher invalidation (issue 04).
//!
//! Light-editing flag path (plan decision #6): a buffer carries an
//! `editable` flag and a derived `locally_modified` (plan 016 issue 03:
//! DERIVED from the saved-state marker + the undo history, not stored —
//! the marker is the single source of truth, and the tie-break is
//! conservative: a buffer that cannot PROVE it matches the disk state
//! reports modified, because a false "clean" is a data-loss path). A
//! buffer is *locally owned* (never auto-clobbered by a disk change) when
//! it has no on-disk path (the scratch buffer), it has unsaved local edits
//! (`locally_modified()`), or it is a file buffer in edit mode
//! (`editable`, plan 005 issue 01: a file the user is actively editing is
//! guarded even before the first keystroke, so a disk change can never
//! silently rewrite it under the cursor mid-edit-session). When a disk
//! change lands on a locally-owned buffer, `changed_on_disk` is set so the
//! view can show a conflict marker instead of overwriting; `g` forces a
//! reload.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ropey::Rope;

/// Display name of the scratch buffer.
pub const SCRATCH_NAME: &str = "*scratch*";

/// The per-buffer edit mode (plan 015 issue 02): selects HOW the buffer's
/// text may be modified. `Annotation` is the default for every buffer (the
/// coarse editing shape); `Accurate` is the per-buffer opt-in (emacs
/// `C-x C-q`). The invariant is **`Accurate` ⟹ `editable`**: `editable`
/// remains the gate for whether text may be modified at all, the mode only
/// selects the shape of that modification, and `Annotation` does NOT imply
/// read-only (the notes buffer is editable + Annotation — that is how
/// annotations are typed).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BufferMode {
    /// The default: the coarse editing shape (every buffer opens here).
    #[default]
    Annotation,
    /// The per-buffer opt-in (`C-x C-q`): the cursor-accurate editing shape.
    Accurate,
}

/// Files larger than this (in bytes) skip highlighting and render as
/// plain text (graceful degradation, never a hang).
pub const BIG_FILE_THRESHOLD: usize = 10 * 1024 * 1024; // 10 MiB

/// Word-constituent for word motion and symbol extraction (C15, the
/// single word-char rule of the redline crate): a Unicode alphanumeric
/// or `_`. This is a fixed rule, NOT the emacs syntax table: emacs
/// decides word-ness per buffer from its syntax table (where e.g. `?`
/// and `!` can be word-constituents and whitespace, symbol, and word
/// categories are distinct). Redline treats every other character —
/// punctuation AND whitespace — as one "non-word" class; newlines are
/// non-word.
///
/// This is the crate-wide definition: word motion
/// (`app::store::file_view`), M-? symbol extraction
/// (`search::references::symbol_under_point`), the ripgrep word-boundary
/// sink (`search::rg::is_word_boundary`), and identifier scanning in
/// `app::store::navigation` all use it. The `redline-resolve` crate has
/// no dependency on `redline` and duplicates this rule (see its
/// `is_ident_char` in `crates/redline-resolve/src/cargo.rs`).
pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// One recorded text edit, kept as the INVERSE needed to undo it
/// (plan 016 issue 01). It is NOT a document snapshot: the history grows
/// with the *edit*, not with the document (a per-step `Rope` clone would
/// grow with the file). Char indices throughout — the same units
/// `retain_rope_edit` takes (a byte/char mix-up here is this project's
/// most recurring defect class, so the ranges are never bytes).
///
/// `range` is the char range the forward edit's inserted text now OCCUPIES
/// in the post-edit rope; `removed` is the text to delete to undo (the
/// forward edit's inserted text); `inserted` is the text to reinsert (the
/// forward edit's original text). Undoing applies `removed`-then-`inserted`
/// over `range` back through the edit path (`retain_rope_edit` +
/// `invalidate_highlight_for_key`).
///
/// `id` (plan 016 issue 03) is the step's unique monotonically increasing
/// identity, assigned by the buffer's counter when the step is recorded. It
/// is the unit the saved-state marker speaks in: the marker stores the
/// identity of the top-of-history position at the last save/load, and the
/// buffer is clean iff the position's identity still equals it. Ids never
/// repeat for a buffer's lifetime (the counter survives cap eviction AND
/// history clears), so an evicted marker can never be matched again —
/// which is exactly what makes an unreachable saved state read as modified
/// instead of clean-by-default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoStep {
    /// The step's unique monotonically increasing id (see the struct docs).
    pub id: usize,
    pub range: Range<usize>,
    pub removed: String,
    pub inserted: String,
}

/// The per-buffer undo stack (plan 016 issue 01): a bounded LIFO of
/// inverse edits, most recent LAST (`pop()` yields the next thing to
/// undo). One per buffer — emacs is buffer-local, so undo in buffer A
/// never touches buffer B, and killing a buffer drops its history with
/// the `Buffer` (a reopened file must not inherit stale offsets). Capped
/// at `MAX_ENTRIES`; when the cap is hit the OLDEST step is dropped, so
/// undo simply stops earlier (the newest steps always win).
///
/// Plan 016 issue 04: this same type backs the per-buffer REDO stack
/// (`Buffer.redo`), so the redo history obeys the undo history's discipline
/// exactly — per buffer, dropped with the buffer, cleared wherever
/// `drop_undo_history` clears the undo stack, bounded by this same cap.
/// A redone step re-enters the undo stack carrying its ORIGINAL id (the
/// step's own `id` is moved, never re-minted): the saved-state marker
/// speaks in these ids, and re-minting would silently break the
/// clean/modified reading (plan 016 issue 03's seam).
#[derive(Clone, Debug, Default)]
pub struct UndoStack {
    steps: Vec<UndoStep>,
}

impl UndoStack {
    /// The undo-history cap (entries) — shared by the redo stack (plan
    /// 016 issue 04: the redo history is bounded by the same cap and must
    /// not become an unbounded second history). A long session cannot grow
    /// the history without limit; the byte footprint of one step is bounded
    /// by the size of the single edit it records.
    pub const MAX_ENTRIES: usize = 100;

    /// Record an inverse edit (most recent). Over the cap, the oldest is
    /// dropped (undo stops earlier).
    pub fn push(&mut self, step: UndoStep) {
        self.steps.push(step);
        if self.steps.len() > Self::MAX_ENTRIES {
            self.steps.remove(0); // drop the oldest → undo stops earlier
        }
    }

    /// The most recent inverse edit (the next thing to undo), or `None`
    /// when the stack is empty.
    pub fn pop(&mut self) -> Option<UndoStep> {
        self.steps.pop()
    }

    /// The most recent step WITHOUT removing it (plan 016 issue 04): the
    /// self-insert-run coalescing decision peeks the run's top step before
    /// committing to a merge — peeking, not popping, in the condition is
    /// what keeps a non-qualifying decision from silently dropping the
    /// peeked step (the side-effect trap 02's M-y helper avoids with an
    /// explicit restore).
    pub fn last(&self) -> Option<&UndoStep> {
        self.steps.last()
    }

    /// Whether the stack holds no steps (test-only: production drives the
    /// stack through `push`/`pop` and reads positions via `position_id`).
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// The identity of the current top-of-history position (plan 016 issue
    /// 03): the most recent step's id, or `0` when the history is empty. The
    /// `0` is the "empty history / freshly loaded" sentinel the saved-state
    /// marker uses.
    ///
    /// Deliberately NOT `steps.len()`: the cap drops the OLDEST step (the
    /// length shrinks without the position moving), and issue 04's redo will
    /// move the position FORWARD again (a re-pushed step re-enters the top
    /// with the same id, whatever the length does). The step id is stable
    /// under both; the length is not.
    pub fn position_id(&self) -> usize {
        self.steps.last().map(|s| s.id).unwrap_or(0)
    }

    /// Number of recorded steps (test-only; the production code drives the
    /// stack through `push`/`pop` and the cap, so this is gated rather than
    /// carried as dead public API).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.steps.len()
    }
}

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
    /// scratch is editable). The gate for whether text may be modified at
    /// all (plan 015 issue 02: `Accurate` mode ⟹ `editable`, and the mode
    /// toggle is what flips this for file buffers).
    pub editable: bool,
    /// The per-buffer edit mode (plan 015 issue 02): `Annotation` default,
    /// `Accurate` opt-in via `C-x C-q`. See `BufferMode` for the invariant.
    pub mode: BufferMode,
    /// Whether this buffer IS the per-project notes document (plan 015
    /// issue 03, folded-in baseline fix): the notes buffer's baseline
    /// editability is BUFFER-relative — it stays inherently editable even
    /// after `switch_project_root` changes the root-relative `notes_key()`
    /// and the old notes buffer survives in the table. Set in `open_notes`.
    pub is_notes: bool,
    /// The saved-state marker (plan 016 issue 03): the identity of the
    /// top-of-history position at the last save/load — the most recent
    /// step's id, or `Some(0)` for "saved/loaded with an EMPTY history"
    /// (the sentinel `UndoStack::position_id` uses). `None` means NO
    /// EVIDENCE: the history was cleared without the content being
    /// re-established from a trusted state, so the buffer must report
    /// modified until the next save/load (see `locally_modified` for the
    /// tie-break). Set by `mark_saved` (a save) and `mark_fresh` (a
    /// reload / accepted discard / notes sync re-read the content from
    /// disk truth); reset to `None` by the store's `drop_undo_history`.
    pub saved_marker: Option<usize>,
    /// The per-buffer undo-step id counter (plan 016 issue 03): the next
    /// id a recorded step takes. Monotonic for the buffer's LIFETIME — it
    /// deliberately survives cap eviction and history clears (a cleared
    /// history must never re-issue an id, or a stale marker could match a
    /// new step and report a false clean). Lives on the buffer, not the
    /// stack, so a `undo = UndoStack::default()` clear cannot reset it.
    pub undo_seq: usize,
    /// A disk change arrived for this buffer while it was locally owned:
    /// the user must reconcile manually (`g` forces a reload; the view
    /// shows a "changed on disk" marker while this is set).
    pub changed_on_disk: bool,
    /// The mark position (byte offset in the rope). `None` when no mark is
    /// set (plan 004 issue 03: set by C-SPC, cleared by C-g / C-w / C-y / M-y).
    pub mark: Option<usize>,
    /// The per-buffer undo stack (plan 016 issue 01): inverse edits of the
    /// text made here, most recent last. Dropped with the buffer on kill;
    /// cleared on a disk reload (issue 03 owns that hook — see the
    /// `reload_in_place` / `toggle_ro_accept` rope-assign sites).
    pub undo: UndoStack,
    /// The per-buffer redo stack (plan 016 issue 04): the inverses of edits
    /// that were undone, most recent last. One undo pushes the inverse onto
    /// this stack; redo re-applies it and pushes the step (its ORIGINAL id,
    /// never re-minted) back onto `undo`. Any new edit clears this stack
    /// (the emacs rule — a stale redo branch after an edit would redo onto
    /// content that no longer exists). Same discipline as `undo`: per
    /// buffer, dropped with the buffer, cleared by the store's
    /// `drop_undo_history` at the three rope-assigning sites, capped at
    /// `UndoStack::MAX_ENTRIES`.
    pub redo: UndoStack,
}

impl std::fmt::Debug for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Buffer")
            .field("path", &self.path)
            .field("lines", &self.rope.len_lines())
            .field("bytes", &self.rope.len_bytes())
            .field("editable", &self.editable)
            .field("is_notes", &self.is_notes)
            .field("locally_modified", &self.locally_modified())
            .field("changed_on_disk", &self.changed_on_disk)
            .finish()
    }
}

impl Buffer {
    /// Create a buffer from a `Rope` and optional path/mtime. The light-
    /// editing flags start clear.
    pub fn new(path: Option<PathBuf>, rope: Rope, mtime: SystemTime, editable: bool) -> Self {
        Self {
            path,
            rope,
            mtime,
            editable,
            mode: BufferMode::default(),
            is_notes: false,
            // Fresh: the content just came from disk (or is empty scratch),
            // the history is empty → the marker is the empty-history
            // sentinel, so the buffer reads clean until the first edit.
            saved_marker: Some(0),
            undo_seq: 0,
            changed_on_disk: false,
            mark: None,
            undo: UndoStack::default(),
            redo: UndoStack::default(),
        }
    }

    /// Whether this buffer is locally owned (never auto-clobbered by a disk
    /// change): it has no on-disk path (scratch), it carries unsaved local
    /// edits, or it is a file buffer in edit mode (plan 005 issue 01: edit
    /// mode guards the reload even before the first edit lands).
    pub fn is_locally_owned(&self) -> bool {
        self.path.is_none()
            || self.locally_modified()
            || (self.editable && self.path.is_some())
    }

    /// The light-editing flag (plan decision #6), DERIVED (plan 016 issue
    /// 03) from the saved-state marker and the undo history rather than
    /// stored: `true` when the buffer's current position cannot be PROVEN
    /// to be the last saved/loaded position.
    ///
    /// **The tie-break (the data-loss direction):** a false "modified" is
    /// an annoyance (the quit prompt asks one extra question, a save writes
    /// identical bytes); a false "clean" is DATA LOSS — `C-x C-c` stops
    /// asking and the unsaved edits are gone. So whenever the marker cannot
    /// prove the buffer matches the disk state, this reports modified:
    /// - `None` (no evidence: the history was cleared without a re-load)
    ///   → always modified;
    /// - `Some(0)` (saved/loaded at an EMPTY history) → clean only while the
    ///   history is STILL empty (any new step moved the position away);
    /// - `Some(id)` → clean only while the top step's id is still `id`.
    ///   When the cap evicts the saved step, no top can carry that id again
    ///   (ids are unique and monotonic), so the saved state is unreachable
    ///   and the buffer reads modified, never clean-by-default.
    ///
    /// Undoing back to the saved position restores cleanliness without any
    /// flag bookkeeping at the edit/undo sites: every recorded edit moves
    /// the position to a fresh id, and every pop moves it back to the
    /// predecessor's id.
    pub fn locally_modified(&self) -> bool {
        match self.saved_marker {
            None => true,
            Some(marker) => self.undo.position_id() != marker,
        }
    }

    /// Record the CURRENT top-of-history position as the saved state (the
    /// store's save paths call this once the write lands): the buffer is
    /// clean from here on iff the position stays where the save found it.
    pub fn mark_saved(&mut self) {
        self.saved_marker = Some(self.undo.position_id());
    }

    /// Re-establish the saved state after the buffer's content was replaced
    /// with a TRUSTED state read from disk truth (the reload chokepoint, the
    /// read-only accept's re-read, the notes sync's write-then-reflect): the
    /// history is gone (or about to be dropped by the caller) and the saved
    /// state is the freshly loaded content — the empty-history sentinel.
    /// Distinct from the `None` evidence-free state a bare
    /// `drop_undo_history` leaves: this one proves clean, that one does not.
    pub fn mark_fresh(&mut self) {
        self.saved_marker = Some(0);
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

    /// The 0-based line and 0-based CHAR column (chars from the line's
    /// start — the same units the point's `col` uses) of the byte at
    /// `byte_idx`. Non-panicking; `None` when out of bounds. The
    /// byte→char step is required, not a convenience: using the raw byte
    /// offset as a column is off-by-N on any line that starts with a
    /// multibyte character.
    pub fn try_byte_to_line_col(&self, byte_idx: usize) -> Option<(usize, usize)> {
        let line = self.try_byte_to_line(byte_idx)?;
        let char_idx = self.rope.try_byte_to_char(byte_idx).ok()?;
        let line_start = self.rope.try_line_to_char(line).ok()?;
        Some((line, char_idx - line_start))
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
    /// An EMPTY table (plan 004 issue 06a): boot starts on the home view,
    /// so no buffer exists yet and `current` is `None`. `*scratch*` is no
    /// longer auto-created — the explicit `open-scratch` command (M-o /
    /// C-x o / M-x) still inserts it on demand, and `SCRATCH_NAME` +
    /// `insert_rope(None, …)` stay available for that.
    pub fn new() -> Self {
        Self {
            by_key: HashMap::new(),
            order: Vec::new(),
            current: None,
        }
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
    fn new_table_starts_empty() {
        // 06a: boot is the home view — no auto-created scratch, no current.
        let t = BufferTable::new();
        assert_eq!(t.len(), 0);
        assert_eq!(t.current(), None);
        assert!(t.current_buffer().is_none());
    }

    #[test]
    fn insert_scratch_explicitly_still_works() {
        // 06a affordance: the open-scratch path (insert with path None)
        // still yields the *scratch* key, editable.
        let mut t = BufferTable::new();
        let k = t.insert(None, String::new());
        assert_eq!(k, SCRATCH_NAME.to_string());
        t.set_current(&k);
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
        assert_eq!(order, vec![b.as_str(), a.as_str()]); // 06a: no scratch row

        t.set_current(&a);
        assert_eq!(t.current(), Some(a.as_str()));
        let order: Vec<&str> = t.list().iter().map(|(k, _)| *k).collect();
        assert_eq!(order, vec![a.as_str(), b.as_str()]); // 06a: no scratch row
    }

    #[test]
    fn kill_removes_from_set_and_clears_current() {
        let mut t = BufferTable::new();
        let a = t.insert(key("/p/a.rs"), "a".into());
        t.set_current(&a);
        assert!(t.kill(&a));
        assert!(t.get(&a).is_none());
        assert_eq!(t.current(), None);
        assert_eq!(t.len(), 0, "06a: killing the only buffer leaves an empty table");
        assert!(!t.kill("/p/nope.rs"));
        assert_eq!(t.len(), 0);
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
        assert_eq!(t.len(), 1, "06a: exactly the one inserted buffer");
        assert_eq!(t.get(&k).unwrap().line_text(0).as_deref(), Some("new"));
    }

    #[test]
    fn same_relative_path_different_projects_are_distinct() {
        let mut t = BufferTable::new();
        let a = t.insert(key("/p1/src/main.rs"), "one".into());
        let b = t.insert(key("/p2/src/main.rs"), "two".into());
        assert_ne!(a, b);
        assert_eq!(t.len(), 2, "06a: two buffers, no scratch");
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
    fn try_byte_to_line_col_multibyte() {
        let mut t = BufferTable::new();
        let k = t.insert(key("/p/a.rs"), "café omega\nxx\n".into());
        let buf = t.get(&k).unwrap();
        // Line 0 "café omega" in BYTES: c=0 a=1 f=2 é=3,4 ' '=5 o=6 m=7
        // e=8 g=9 a=10; in CHARS: c=0 a=1 f=2 é=3 ' '=4 o=5 …
        assert_eq!(buf.try_byte_to_line_col(0), Some((0, 0)));
        assert_eq!(
            buf.try_byte_to_line_col(6),
            Some((0, 5)),
            "byte 6 is char 5 (é occupies 2 bytes)"
        );
        // Line 1 "xx" starts at byte 12 (11 bytes + the '\n' at 11).
        assert_eq!(buf.try_byte_to_line_col(12), Some((1, 0)));
        assert_eq!(buf.try_byte_to_line_col(13), Some((1, 1)));
        assert_eq!(buf.try_byte_to_line_col(999), None, "out of bounds");
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

    /// plan 005 issue 01: `is_locally_owned` covers the three guards —
    /// pathless (scratch), locally modified, and edit-mode file buffer
    /// (even before the first edit lands).
    #[test]
    fn is_locally_owned_edit_mode_file_buffer() {
        let mut t = BufferTable::new();
        let k = t.insert(key("/p/a.rs"), "fn a() {}\n".into());
        let buf = t.get(&k).unwrap();
        assert!(!buf.is_locally_owned(), "read-only file buffer is not locally owned");

        // Edit mode ON (a mid-session `editable` flip): locally owned even
        // with no edits yet.
        t.get_mut(&k).unwrap().editable = true;
        assert!(
            t.get(&k).unwrap().is_locally_owned(),
            "an edit-mode file buffer must be locally owned"
        );

        // Back to read-only with no edits: no longer locally owned.
        t.get_mut(&k).unwrap().editable = false;
        assert!(!t.get(&k).unwrap().is_locally_owned());

        // Scratch (no path) is locally owned regardless of the flag.
        // 06a: scratch no longer exists at boot — insert it explicitly.
        let scratch_key = t.insert(None, String::new());
        let scratch = t.get(&scratch_key).unwrap();
        assert!(scratch.is_locally_owned());
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
