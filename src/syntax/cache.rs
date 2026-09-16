//! Highlight cache: keyed by (path, mtime, theme), bounded memory.
//! Cache invalidation happens on reopen (mtime change) and theme switch
//! (theme name change). Live invalidation via a file watcher arrives in
//! issue 04.

use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

use crate::syntax::highlight::HighlightResult;

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
        let mtime_nanos = mtime
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        Self {
            path: path.to_string_lossy().into_owned(),
            mtime_nanos,
            theme: theme.to_string(),
        }
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
}

impl HighlightCache {
    pub fn new() -> Self {
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
            lines: vec![crate::syntax::highlight::HighlightedLine::default(); n_lines],
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
}
