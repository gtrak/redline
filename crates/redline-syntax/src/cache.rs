//! Highlight cache: keyed by (path, mtime, theme), bounded memory.
//! Cache invalidation happens on reopen (mtime change) and theme switch
//! (theme name change). Live invalidation via a file watcher ships in
//! `src/app/watcher.rs`.

use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

use tree_sitter::InputEdit;

use crate::highlight::RetainedTree;
use crate::highlight::HighlightResult;

/// Maximum number of entries before the cache is cleared.
/// Each entry holds a `HighlightResult` (one `HighlightedLine` per line,
/// each with a small `Vec<LineSpan>`); for a 10k-line file this is
/// on the order of a few MB. 64 entries × ~5 MB ≈ 320 MB worst case,
/// which is the hard cap. In practice most entries are much smaller.
pub const MAX_ENTRIES: usize = 64;

/// A cache key: (absolute path string, mtime, theme name).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub path: String,
    /// File modification time; `SystemTime` is hashable via its
    /// internal representation. We use `duration_since(UNIX_EPOCH)`
    /// as a `u64` nanosecond count for a stable hash.
    pub mtime_nanos: u128,
    /// Theme name (e.g. "default", "dark"); switching themes invalidates
    /// all cached highlights.
    pub theme: String,
}

impl CacheKey {
    /// Build a key from a path, its mtime, and the theme name.
    pub fn new(path: &Path, mtime: SystemTime, theme: &str) -> Self {
        Self {
            path: path.to_string_lossy().into_owned(),
            mtime_nanos: mtime_to_nanos(mtime),
            theme: theme.to_string(),
        }
    }
}

/// The mtime as a stable nanosecond count (the derivation both
/// `CacheKey::new` and `TreeKey::new` use; `0` for pre-epoch times).
pub fn mtime_to_nanos(mtime: SystemTime) -> u128 {
    mtime
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// The maximum number of retained parse trees. Each entry is one
/// non-big buffer's parse tree; 8 keeps the worst case bounded while
/// covering the realistic edit workloads (one notes buffer, a few open
/// files).
pub const MAX_RETAINED_TREES: usize = 8;

/// A retained-tree key: (buffer key, file mtime). The mtime is part of
/// the key on purpose: a disk reload produces a new mtime, so the stale
/// tree (parsed from the pre-reload content) is simply never found and
/// the buffer falls back to a full parse.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TreeKey {
    pub path: String,
    pub mtime_nanos: u128,
}

impl TreeKey {
    pub fn new(path: &str, mtime: SystemTime) -> Self {
        Self {
            path: path.to_string(),
            mtime_nanos: mtime_to_nanos(mtime),
        }
    }
}

/// The retained parse trees for incremental reparsing, in insertion
/// order for eviction (oldest evicted first, like the highlight cache).
#[derive(Debug, Default)]
pub struct RetainedTrees {
    map: HashMap<TreeKey, RetainedTree>,
    order: Vec<TreeKey>,
}

impl RetainedTrees {
    pub fn get_mut(&mut self, key: &TreeKey) -> Option<&mut RetainedTree> {
        self.map.get_mut(key)
    }

    pub fn contains(&self, key: &TreeKey) -> bool {
        self.map.contains_key(key)
    }

    /// Remove a retained tree (007-04 review P2-1: content replacement
    /// without an edit event must not leave a hintable stale tree).
    pub fn remove(&mut self, key: &TreeKey) {
        self.map.remove(key);
        self.order.retain(|k| k != key);
    }

    /// Apply an `InputEdit` to the retained tree; no-op when the buffer
    /// has no retained tree yet (plain text, big file, or not yet
    /// highlighted).
    pub fn apply_edit(&mut self, key: &TreeKey, edit: &InputEdit) {
        if let Some(tree) = self.map.get_mut(key) {
            tree.apply_edit(edit);
        }
    }

    /// Store a freshly parsed tree under `key`, evicting the oldest
    /// entry beyond `MAX_RETAINED_TREES`.
    pub fn insert(&mut self, key: TreeKey, tree: RetainedTree) {
        // Fast path: key already present — update in place, no eviction.
        if let std::collections::hash_map::Entry::Occupied(mut e) = self.map.entry(key.clone()) {
            e.insert(tree);
            return;
        }
        while self.order.len() >= MAX_RETAINED_TREES {
            let oldest = self.order.remove(0);
            self.map.remove(&oldest);
        }
        self.order.push(key.clone());
        self.map.insert(key, tree);
    }

    #[allow(dead_code)] // used in tests
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[allow(dead_code)] // used in tests
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// A bounded highlight cache. Insertion order is tracked for eviction;
/// when the cache exceeds `MAX_ENTRIES`, the oldest entry is evicted.
#[derive(Debug, Default)]
pub struct HighlightCache {
    /// Fast lookup.
    map: HashMap<CacheKey, HighlightResult>,
    /// Insertion order (oldest first); used for eviction.
    order: Vec<CacheKey>,
    /// Retained parse trees for incremental reparsing (plan 007 issue 04).
    trees: RetainedTrees,
}

impl HighlightCache {
    pub fn new() -> Self {
        // Store-construction warmup (before any render): build the reuse
        // engines now so the first highlight is a map lookup, not a
        // lazy ~188ms debug-build build on the render critical path.
        crate::highlight::warm_reuse_engines();
        Self::default()
    }

    /// Look up a cached highlight result; `None` on a miss.
    pub fn get(&self, key: &CacheKey) -> Option<&HighlightResult> {
        self.map.get(key)
    }

    /// The number of cached entries.
    #[allow(dead_code)] // used in tests
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// `true` when the cache is empty.
    #[allow(dead_code)] // used in tests
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Remove all entries.
    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    /// Insert a highlight result; evicts the oldest entry when the
    /// cache exceeds `MAX_ENTRIES`.
    pub fn insert(&mut self, key: CacheKey, result: HighlightResult) {
        // Fast path: key already present — update in place, no eviction.
        {
            if let std::collections::hash_map::Entry::Occupied(mut e) =
                self.map.entry(key.clone())
            {
                e.insert(result);
                return;
            }
        }
        // Slow path: new key — evict oldest if at capacity, then insert.
        while self.order.len() >= MAX_ENTRIES {
            let oldest = self.order.remove(0);
            self.map.remove(&oldest);
        }
        self.order.push(key.clone());
        self.map.insert(key, result);
    }

    /// The buffer's retained parse tree (for an incremental reparse);
    /// `None` when the buffer has none.
    pub fn retain_tree(&mut self, key: &TreeKey) -> Option<&mut RetainedTree> {
        self.trees.get_mut(key)
    }

    /// `true` when the buffer has a retained parse tree.
    #[allow(dead_code)] // used in tests
    pub fn retain_contains(&self, key: &TreeKey) -> bool {
        self.trees.contains(key)
    }

    /// Record a rope edit on the buffer's retained tree (no-op without
    /// one).
    pub fn retain_apply_edit(&mut self, key: &TreeKey, edit: &InputEdit) {
        self.trees.apply_edit(key, edit);
    }

    /// Drop a buffer's retained tree (007-04 review P2-1): a content
    /// replacement WITHOUT an edit event must not leave a tree that could
    /// be hinted against unrelated content when the mtime is unchanged.
    pub fn retain_remove(&mut self, key: &TreeKey) {
        self.trees.remove(key);
    }

    /// Store a freshly parsed tree as the buffer's retained baseline.
    pub fn retain_insert(&mut self, key: TreeKey, tree: RetainedTree) {
        self.trees.insert(key, tree);
    }

    #[allow(dead_code)] // used in tests
    pub fn retain_len(&self) -> usize {
        self.trees.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn key(path: &str, mtime_secs: u64, theme: &str) -> CacheKey {
        let mtime = std::time::UNIX_EPOCH + Duration::from_secs(mtime_secs);
        CacheKey::new(std::path::Path::new(path), mtime, theme)
    }

    fn dummy_result(n_lines: usize) -> HighlightResult {
        HighlightResult {
            lines: vec![crate::highlight::HighlightedLine::default(); n_lines],
        }
    }

    #[test]
    fn same_key_hits_cache() {
        let mut cache = HighlightCache::new();
        let k = key("/a/b.rs", 1000, "default");
        cache.insert(k.clone(), dummy_result(10));
        assert!(cache.get(&k).is_some());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn changed_mtime_invalidates() {
        let mut cache = HighlightCache::new();
        let k1 = key("/a/b.rs", 1000, "default");
        let k2 = key("/a/b.rs", 1001, "default"); // mtime changed
        cache.insert(k1.clone(), dummy_result(10));
        assert!(cache.get(&k1).is_some());
        assert!(cache.get(&k2).is_none(), "new mtime must miss");
    }

    #[test]
    fn theme_switch_invalidates() {
        let mut cache = HighlightCache::new();
        let k1 = key("/a/b.rs", 1000, "default");
        let k2 = key("/a/b.rs", 1000, "dark"); // theme changed
        cache.insert(k1.clone(), dummy_result(10));
        assert!(cache.get(&k1).is_some());
        assert!(cache.get(&k2).is_none(), "different theme must miss");
    }

    #[test]
    fn different_path_is_distinct_key() {
        let mut cache = HighlightCache::new();
        let k1 = key("/a/b.rs", 1000, "default");
        let k2 = key("/a/c.rs", 1000, "default");
        cache.insert(k1.clone(), dummy_result(10));
        assert!(cache.get(&k1).is_some());
        assert!(cache.get(&k2).is_none());
    }

    #[test]
    fn bounded_size_evicts_oldest() {
        let mut cache = HighlightCache::new();
        // Insert MAX_ENTRIES entries.
        for i in 0..MAX_ENTRIES {
            cache.insert(key(&format!("/f{i}.rs"), i as u64, "default"), dummy_result(1));
        }
        assert_eq!(cache.len(), MAX_ENTRIES);

        // One more insert evicts the oldest (f0).
        cache.insert(key("/overflow.rs", 99999, "default"), dummy_result(1));
        assert_eq!(cache.len(), MAX_ENTRIES);
        assert!(cache.get(&key("/f0.rs", 0, "default")).is_none(), "f0 should be evicted");
        assert!(cache.get(&key("/overflow.rs", 99999, "default")).is_some());
    }

    #[test]
    fn reinsert_same_key_does_not_grow() {
        let mut cache = HighlightCache::new();
        let k = key("/a/b.rs", 1000, "default");
        cache.insert(k.clone(), dummy_result(5));
        cache.insert(k.clone(), dummy_result(10));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&k).unwrap().lines.len(), 10);
    }

    #[test]
    fn clear_drops_all() {
        let mut cache = HighlightCache::new();
        for i in 0..5 {
            cache.insert(key(&format!("/f{i}.rs"), i as u64, "default"), dummy_result(1));
        }
        cache.clear();
        assert!(cache.is_empty());
    }

    // ── retained trees (plan 007 issue 04) ─────────────────────────

    fn tree_key(path: &str, mtime_secs: u64) -> TreeKey {
        let mtime = std::time::UNIX_EPOCH + Duration::from_secs(mtime_secs);
        TreeKey::new(path, mtime)
    }

    fn dummy_tree() -> RetainedTree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter::Language::from(tree_sitter_rust::LANGUAGE))
            .unwrap();
        let tree = parser.parse(b"fn main() {}\n", None).unwrap();
        RetainedTree::new(tree)
    }

    #[test]
    fn retained_tree_roundtrip_same_key() {
        let mut cache = HighlightCache::new();
        let k = tree_key("/a/b.rs", 1000);
        cache.retain_insert(k.clone(), dummy_tree());
        assert!(cache.retain_contains(&k));
        assert!(cache.retain_tree(&k).is_some());
    }

    #[test]
    fn changed_mtime_is_a_stale_tree_key() {
        // A disk reload changes the mtime; the retained tree keyed by the
        // old mtime must not be found (stale-tree fallback = full parse).
        let mut cache = HighlightCache::new();
        let k1 = tree_key("/a/b.rs", 1000);
        let k2 = tree_key("/a/b.rs", 1001); // reloaded: new mtime
        cache.retain_insert(k1.clone(), dummy_tree());
        assert!(cache.retain_tree(&k2).is_none(), "new mtime must miss");
        assert!(cache.retain_tree(&k1).is_some());
    }

    #[test]
    fn retained_apply_edit_without_tree_is_noop() {
        let mut cache = HighlightCache::new();
        let k = tree_key("/a/b.rs", 1000);
        let edit = tree_sitter::InputEdit {
            start_byte: 0,
            old_end_byte: 0,
            new_end_byte: 1,
            start_position: tree_sitter::Point::new(0, 0),
            old_end_position: tree_sitter::Point::new(0, 0),
            new_end_position: tree_sitter::Point::new(0, 1),
        };
        cache.retain_apply_edit(&k, &edit); // no tree yet: must not panic
        assert!(!cache.retain_contains(&k));
    }

    #[test]
    fn retained_trees_are_bounded() {
        let mut cache = HighlightCache::new();
        for i in 0..MAX_RETAINED_TREES {
            cache.retain_insert(tree_key(&format!("/f{i}.rs"), i as u64), dummy_tree());
        }
        assert_eq!(cache.retain_len(), MAX_RETAINED_TREES);
        // One more insert evicts the oldest.
        cache.retain_insert(tree_key("/overflow.rs", 99999), dummy_tree());
        assert_eq!(cache.retain_len(), MAX_RETAINED_TREES);
        assert!(!cache.retain_contains(&tree_key("/f0.rs", 0)), "f0 should be evicted");
        assert!(cache.retain_contains(&tree_key("/overflow.rs", 99999)));
    }

    #[test]
    fn reinsert_same_key_does_not_grow_retained() {
        let mut cache = HighlightCache::new();
        let k = tree_key("/a/b.rs", 1000);
        cache.retain_insert(k.clone(), dummy_tree());
        cache.retain_insert(k.clone(), dummy_tree());
        assert_eq!(cache.retain_len(), 1);
    }
}
