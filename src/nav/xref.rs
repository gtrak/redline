//! Navigation behind an [`Xref`] trait (issue 05): find a definition's
//! location(s), a file's outline, and all project symbols. The tree-sitter
//! backend is [`SymbolIndex`]; an LSP backend can implement the same trait
//! later without touching the app's command handlers.

use super::index::{Location, Symbol, SymbolIndex};

/// Cross-reference for symbol navigation. Methods are read-only (the backend
/// owns the index); the tree-sitter backend is in-memory and synchronous.
#[allow(dead_code)] // used by tests; LSP backend will use it later
pub trait Xref {
    /// Every definition location for `name` (all files, all symbols of that
    /// name), in a deterministic (file, line, name) order.
    fn find_definition(&self, name: &str) -> Vec<Location>;

    /// The outline (symbols) for a project-relative file.
    fn outline(&self, file: &str) -> Vec<Symbol>;

    /// Every definition location in the index (all files).
    fn all_symbols(&self) -> Vec<Location>;

    /// How many files contribute symbols.
    fn file_count(&self) -> usize;
}

impl Xref for SymbolIndex {
    fn find_definition(&self, name: &str) -> Vec<Location> {
        SymbolIndex::definitions_of(self, name)
    }

    fn outline(&self, file: &str) -> Vec<Symbol> {
        SymbolIndex::outline(self, file).to_vec()
    }

    fn all_symbols(&self) -> Vec<Location> {
        SymbolIndex::all_locations(self)
    }

    fn file_count(&self) -> usize {
        SymbolIndex::file_count(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a tiny in-memory index by hand (no filesystem) and confirm the
    /// Xref trait routes to the tree-sitter backend correctly.
    #[test]
    fn xref_trait_finds_definitions() {
        let mut idx = SymbolIndex::new();
        let sym = Symbol {
            name: "foo".into(),
            kind: crate::syntax::queries::SymbolKind::Function,
            line: 3,
            start_byte: 0,
            end_byte: 10,
            end_line: 3,
        };
        idx.set_file("a.rs", vec![sym.clone()]);
        idx.set_file("b.rs", vec![sym.clone()]);

        let xref: &dyn Xref = &idx;
        let defs = xref.find_definition("foo");
        assert_eq!(defs.len(), 2, "foo defined in a.rs and b.rs");
        assert_eq!(defs[0].file, "a.rs");
        assert_eq!(defs[1].file, "b.rs");
        assert_eq!(xref.outline("a.rs").len(), 1);
        assert_eq!(xref.all_symbols().len(), 2);
        assert_eq!(xref.file_count(), 2);
        assert!(xref.find_definition("missing").is_empty());
    }

    // (Kept as a smoke test that the trait is object-safe.)
    #[test]
    fn xref_is_object_safe() {
        let idx = SymbolIndex::new();
        let boxed: Box<dyn Xref> = Box::new(idx);
        assert!(boxed.find_definition("x").is_empty());
    }
}
