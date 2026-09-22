//! The in-memory symbol index: per-file outlines, name-keyed lookup maps,
//! and the per-file Rust tables (010-01), with their same-name-keyed
//! field / trait maps.

use std::collections::HashMap;

use redline_syntax::queries::{ImplKind, RustTables, Symbol};

/// The field-map shape (clippy's type_complexity factored out): struct
/// name → field name → (file → [(line, name start byte)]).
type FieldMap = HashMap<String, HashMap<String, HashMap<String, Vec<(usize, usize)>>>>;

/// A definition's location in the project: the (project-relative) file and
/// the symbol itself. The tree-sitter backend's `definitions_of` returns
/// these.
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
    /// field lookup: struct name → field name → (file → [(line, name start
    /// byte)]). The byte (jump-column-pty) is what the landing converts to
    /// a char column — the pre-fix line-only shape landed at col 0.
    /// `pub(super)` so the builder's second `impl SymbolIndex`
    /// block (`set_file_tables`) can maintain the maps alongside.
    pub(super) rust_fields: FieldMap,
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

/// (010-01 / jump-column-pty) One struct field's location in the index:
/// the (project-relative) file, the field declaration's 0-based line, and
/// the field name node's start byte (the landing's char-column source —
/// the pre-fix `Vec<(String, usize)>` shape had no byte to carry, so a
/// field jump landed at column 0 by construction).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldLocation {
    pub file: String,
    pub line: usize,
    pub start_byte: usize,
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
    /// same-file hit first. Each entry carries the field name's start byte
    /// (jump-column-pty): the landing converts it to the char column via
    /// the byte→(line,col) conversion, the way the outline symbols do.
    pub fn field_locations(&self, struct_name: &str, field: &str) -> Vec<FieldLocation> {
        let Some(per_file) = self.rust_fields.get(struct_name).and_then(|m| m.get(field)) else {
            return Vec::new();
        };
        let mut out: Vec<FieldLocation> = per_file
            .iter()
            .flat_map(|(file, locs)| {
                locs.iter().map(move |&(line, start_byte)| FieldLocation {
                    file: file.clone(),
                    line,
                    start_byte,
                })
            })
            .collect();
        out.sort_by(|a, b| (a.file.as_str(), a.line).cmp(&(b.file.as_str(), b.line)));
        out.dedup();
        out
    }

    /// (jump-column-pty) The name-node start byte of the struct field
    /// `field` declared at `(file, line)` — the picker-landing fallback for
    /// a field row (fields are not outline symbols). The field-table map is
    /// keyed struct-first and the picker's RET row carries only the field
    /// name + (file, line) (no struct), so when SEVERAL structs declare a
    /// same-named field on the SAME line this answers the MINIMUM byte —
    /// the earliest declaration on that line (the same "land on the first"
    /// convention the outline path applies to a line hosting the same name
    /// twice). Deterministic by construction: the minimum over the matching
    /// entries never depends on the map's iteration order (pre-fix this was
    /// `find_map` over `rust_fields.values()` — a per-process-seeded
    /// `HashMap` order, so the same input answered different columns in
    /// different processes).
    /// `None` when no field of that name is recorded at that (file, line)
    /// — the caller degrades to col 0.
    pub fn field_start_byte(&self, file: &str, line: usize, field: &str) -> Option<usize> {
        self.rust_fields
            .values()
            .filter_map(|by_field| by_field.get(field))
            .filter_map(|by_file| by_file.get(file))
            .filter_map(|locs| locs.iter().find(|&&(l, _)| l == line).map(|&(_, b)| b))
            .min()
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
