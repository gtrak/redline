//! The in-memory symbol index: per-file outlines, name-keyed lookup maps,
//! and the per-file Rust tables (010-01), with their same-name-keyed
//! field / trait maps.

use std::collections::HashMap;

use crate::syntax::queries::{ImplKind, RustTables, Symbol};

/// A definition's location in the project: the (project-relative) file and
/// the symbol itself. The tree-sitter `Xref` backend's `find_definition`
/// returns these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub file: String,
    pub symbol: Symbol,
}

/// The in-memory symbol index: per-file outlines plus a
/// name → (file → symbol) map for cross-file definition lookup. Plain data;
/// `Clone` (the app hands a clone of the current index to a background
/// incremental job, and `watch` retains the latest result).
#[derive(Clone, Debug, Default)]
pub struct SymbolIndex {
    /// project-relative path → outline (files with ≥1 symbol only).
    files: HashMap<String, Vec<Symbol>>,
    /// symbol name → (project-relative file → Vec of same-name symbols).
    /// Cross-file lookup; the inner `Vec` preserves all same-file duplicates
    /// (e.g. two `fn f` in different mods of one file).
    by_name: HashMap<String, HashMap<String, Vec<Symbol>>>,
    /// Total symbol count across all files.
    total: usize,
    /// 010-01: per-file Rust tables (Rust files with impls/structs, or
    /// local binding annotations (010-03) only), built in the same pass
    /// as the outlines (same tree — zero extra parse
    /// cost). Same-file self-receiver / local-binding method + field
    /// lookup. `pub(super)` so the builder's second `impl SymbolIndex`
    /// block (`set_file_tables`) can maintain the maps alongside.
    pub(super) rust_tables: HashMap<String, RustTables>,
    /// 010-01: name-keyed struct fields for the CROSS-file self-receiver
    /// field lookup: struct name → field name → (file → [lines]).
    /// `pub(super)` so the builder's second `impl SymbolIndex`
    /// block (`set_file_tables`) can maintain the maps alongside.
    pub(super) rust_fields: HashMap<String, HashMap<String, HashMap<String, Vec<usize>>>>,
    /// 010-04 (plan 010 Shape A, rung 4): the name-keyed TRAIT map for
    /// find-implementations: trait name (the impl's captured `trait:` field
    /// text) → (file → that file's `impl Trait for Type` blocks) — built
    /// from the SAME per-file tables in `set_file_tables` (zero extra
    /// parse cost, same tree; the 010-01 same-check-BEFORE-removal
    /// discipline applies to this map verbatim — the shortcut above the
    /// removals guards it). `pub(super)` so the builder's second
    /// `impl SymbolIndex` block (`set_file_tables`) can maintain the
    /// maps alongside.
    pub(super) rust_traits: HashMap<String, HashMap<String, Vec<TraitImpl>>>,
}

/// (010-04) One `impl <Trait> for <Type>` block recorded by a file's
/// tables: the impl header's line (0-based) and the impl'd (self) type.
/// The read-only view's building block (the find-implementations picker).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitImpl {
    pub impl_line: usize,
    pub self_type: String,
}

/// (010-04) A trait-impl location across the index: the (project-relative,
/// or crate-relative for an external index) file plus the impl block's
/// line and self type — one find-implementations candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitImplLocation {
    pub file: String,
    pub impl_line: usize,
    pub self_type: String,
}

impl SymbolIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` when the file has an outline entry.
    #[allow(dead_code)]
    pub fn has(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    /// The outline (symbols) for a project-relative file.
    pub fn outline(&self, path: &str) -> &[Symbol] {
        self.files.get(path).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// (010-01) The per-file Rust tables (struct fields + impl methods);
    /// `None` for files without tables (non-Rust, or no impls/structs).
    /// The same-file seam of the M-. self-receiver consumption.
    pub fn tables(&self, path: &str) -> Option<&RustTables> {
        self.rust_tables.get(path)
    }

    /// (010-01) Every location of struct `struct_name`'s field `field`
    /// (all files, same file included), in deterministic (file, line) order
    /// — the cross-file self-receiver field lookup; the caller orders the
    /// same-file hit first.
    pub fn field_locations(&self, struct_name: &str, field: &str) -> Vec<(String, usize)> {
        let Some(per_file) = self.rust_fields.get(struct_name).and_then(|m| m.get(field)) else {
            return Vec::new();
        };
        let mut out: Vec<(String, usize)> = per_file
            .iter()
            .flat_map(|(file, lines)| lines.iter().map(move |&l| (file.clone(), l)))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        out.dedup();
        out
    }

    /// (010-04) Every `impl <trait_name> for <Type>` location in the
    /// index (all files), in deterministic (file, impl line, self type)
    /// order — the read-only data for find-implementations; the caller
    /// orders the same-file hits first (the M-. same-file-first
    /// discipline, shared with `field_locations`).
    pub fn trait_impl_locations(&self, trait_name: &str) -> Vec<TraitImplLocation> {
        let Some(per_file) = self.rust_traits.get(trait_name) else {
            return Vec::new();
        };
        let mut out: Vec<TraitImplLocation> = per_file
            .iter()
            .flat_map(|(file, entries)| entries.iter().map(|e| TraitImplLocation {
                file: file.clone(),
                impl_line: e.impl_line,
                self_type: e.self_type.clone(),
            }))
            .collect();
        out.sort_by(|a, b| {
            (a.file.as_str(), a.impl_line, &a.self_type)
                .cmp(&(b.file.as_str(), b.impl_line, &b.self_type))
        });
        out.dedup();
        out
    }

    /// Total symbol count across all indexed files.
    pub fn total(&self) -> usize {
        self.total
    }

    /// Every definition location for `name` (all files, all symbols of that
    /// name), in a deterministic (file, line, name) order.
    pub fn definitions_of(&self, name: &str) -> Vec<Location> {
        let Some(map) = self.by_name.get(name) else {
            return Vec::new();
        };
        let mut out: Vec<Location> = map
            .iter()
            .flat_map(|(file, syms)| syms.iter().map(move |sym| Location {
                file: file.clone(),
                symbol: sym.clone(),
            }))
            .collect();
        out.sort_by(|a, b| {
            (a.file.as_str(), a.symbol.line, &a.symbol.name).cmp(&(b.file.as_str(), b.symbol.line, &b.symbol.name))
        });
        out
    }

    /// Number of files in which `name` is defined (cross-file).
    #[allow(dead_code)]
    pub fn definition_count(&self, name: &str) -> usize {
        self.by_name.get(name).map(|m| m.len()).unwrap_or(0)
    }

    /// Number of files with ≥1 symbol.
    #[allow(dead_code)]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Every definition location in the index (all files, all symbols).
    pub fn all_locations(&self) -> Vec<Location> {
        let mut out: Vec<Location> = self
            .files
            .iter()
            .flat_map(|(file, syms)| syms.iter().map(|s| Location {
                file: file.clone(),
                symbol: s.clone(),
            }))
            .collect();
        out.sort_by(|a, b| {
            (a.file.as_str(), a.symbol.line, &a.symbol.name).cmp(&(
                b.file.as_str(),
                b.symbol.line,
                &b.symbol.name,
            ))
        });
        out
    }

    /// Replace a file's outline, keeping the cross-file name map consistent.
    /// Returns `true` when the stored outline actually changed.
    pub fn set_file(&mut self, path: &str, symbols: Vec<Symbol>) -> bool {
        let old = self.files.remove(path);
        let same = old.as_ref() == Some(&symbols);
        if same {
            // Restore the unchanged entry and bail.
            self.files.insert(path.to_string(), symbols);
            return false;
        }
        // Drop this file's contributions from the name map.
        if let Some(old_syms) = &old {
            for s in old_syms {
                if let Some(m) = self.by_name.get_mut(&s.name) {
                    m.remove(path);
                    if m.is_empty() {
                        self.by_name.remove(&s.name);
                    }
                }
            }
        }
        self.total = self.total.saturating_sub(old.map(|v| v.len()).unwrap_or(0));
        if symbols.is_empty() {
            return true;
        }
        for s in &symbols {
            self.by_name
                .entry(s.name.clone())
                .or_default()
                .entry(path.to_string())
                .or_default()
                .push(s.clone());
        }
        self.files.insert(path.to_string(), symbols);
        self.total = self.total.saturating_add(self.files[path].len());
        true
    }

    /// Remove a file's outline (its symbols) from the index. Returns `true`
    /// when the file had entries.
    pub fn remove_file(&mut self, path: &str) -> bool {
        let Some(old) = self.files.remove(path) else {
            return false;
        };
        for s in &old {
            if let Some(m) = self.by_name.get_mut(&s.name) {
                m.remove(path);
                if m.is_empty() {
                    self.by_name.remove(&s.name);
                }
            }
        }
        self.total = self.total.saturating_sub(old.len());
        // 010-01: the file's Rust tables (a file with tables always has an
        // outline — every struct/impl contributes a symbol — so this never
        // orphans entries, but the cleanup keeps the invariant explicit).
        if let Some(old_tables) = self.rust_tables.remove(path) {
            for (struct_name, fields) in &old_tables.fields {
                if let Some(by_field) = self.rust_fields.get_mut(struct_name) {
                    for f in fields {
                        if let Some(file_map) = by_field.get_mut(&f.field) {
                            file_map.remove(path);
                            if file_map.is_empty() {
                                by_field.remove(&f.field);
                            }
                        }
                    }
                    if by_field.is_empty() {
                        self.rust_fields.remove(struct_name);
                    }
                }
            }
            // 010-04: the trait map leaves with the tables (the same
            // invariant as the field map: a file with trait impls always
            // had a stored entry).
            for methods in old_tables.impls.values() {
                for m in methods {
                    if let ImplKind::Trait(t) = &m.kind
                        && let Some(by_file) = self.rust_traits.get_mut(t)
                    {
                        by_file.remove(path);
                        if by_file.is_empty() {
                            self.rust_traits.remove(t);
                        }
                    }
                }
            }
        }
        true
    }
}
