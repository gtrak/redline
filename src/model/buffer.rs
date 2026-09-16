//! The open-buffer set: which buffers are open, their plain-text
//! contents (the ropey text model arrives with issue 03), and the
//! current buffer. `kill` removes a buffer from the set.

use std::collections::HashMap;
use std::path::PathBuf;

/// Display name of the scratch buffer.
pub const SCRATCH_NAME: &str = "*scratch*";

/// One open buffer. `path` is `None` for the scratch buffer; the text
/// is plain `String` for now (read files are immutable until issue 03
/// makes buffers editable).
#[derive(Clone, Debug)]
pub struct Buffer {
    pub path: Option<PathBuf>,
    pub text: String,
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
    /// A table holding only a fresh `*scratch*` buffer (current).
    pub fn new() -> Self {
        let mut t = Self {
            by_key: HashMap::new(),
            order: Vec::new(),
            current: None,
        };
        let key = t.insert(None, String::new());
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

    /// Insert (or replace) a buffer and make it most-recently-used.
    pub fn insert(&mut self, path: Option<PathBuf>, text: String) -> String {
        let key = Self::key_for(&path);
        let is_new = !self.by_key.contains_key(&key);
        self.by_key
            .insert(key.clone(), Buffer { path, text });
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
        assert!(t.current_buffer().unwrap().path.is_none());
    }

    #[test]
    fn insert_and_get_buffers() {
        let mut t = BufferTable::new();
        let k = t.insert(key("/p/src/a.rs"), "fn a() {}\n".into());
        assert_eq!(k, "/p/src/a.rs");
        assert_eq!(t.get(&k).unwrap().text, "fn a() {}\n");
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn switching_buffers_updates_current_and_mru() {
        let mut t = BufferTable::new();
        let a = t.insert(key("/p/a.rs"), "a".into());
        let b = t.insert(key("/p/b.rs"), "b".into());
        // MRU order right after opening: b, a, *scratch*.
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
        // Killing an unknown buffer reports false.
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
        assert_eq!(t.get(&k).unwrap().text, "new");
    }

    #[test]
    fn same_relative_path_different_projects_are_distinct() {
        let mut t = BufferTable::new();
        let a = t.insert(key("/p1/src/main.rs"), "one".into());
        let b = t.insert(key("/p2/src/main.rs"), "two".into());
        assert_ne!(a, b);
        assert_eq!(t.len(), 3);
    }
}
