//! Tests for `SymbolIndex`'s cross-reference methods (issue 05): a
//! definition's location(s), a file's outline, and all project symbols.
//! These are inherent methods on `SymbolIndex` (defined in
//! [`crate::nav::index`]); the app's command handlers call them directly.
//! There is no `Xref` trait / LSP seam — none is planned, so the earlier
//! trait indirection was a pass-through to itself and has been removed.

#[cfg(test)]
mod tests {
    use crate::nav::index::{Symbol, SymbolIndex};

    /// Build a tiny in-memory index by hand (no filesystem) and confirm the
    /// cross-reference methods return the expected definition locations,
    /// outline, and file count.
    #[test]
    fn symbol_index_finds_definitions() {
        let mut idx = SymbolIndex::new();
        let sym = Symbol {
            name: "foo".into(),
            kind: redline_syntax::queries::SymbolKind::Function,
            line: 3,
            start_byte: 0,
            end_byte: 10,
            end_line: 3,
        };
        idx.set_file("a.rs", vec![sym.clone()]);
        idx.set_file("b.rs", vec![sym.clone()]);

        let defs = idx.definitions_of("foo");
        assert_eq!(defs.len(), 2, "foo defined in a.rs and b.rs");
        assert_eq!(defs[0].file, "a.rs");
        assert_eq!(defs[1].file, "b.rs");
        assert_eq!(idx.outline("a.rs").len(), 1);
        assert_eq!(idx.all_locations().len(), 2);
        assert_eq!(idx.file_count(), 2);
        assert!(idx.definitions_of("missing").is_empty());
    }
}
