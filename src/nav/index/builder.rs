//! The background indexing machinery: the per-file parse, the rayon
//! parallel full build, the incremental in-place refresh, the
//! which-function cursor lookup, and the second `impl SymbolIndex`
//! block (the `set_file_tables` map-maintenance method).

use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::syntax::queries::{extract_all, ImplKind, RustTables, Symbol};
use crate::syntax::registry::resolve_language;

use super::progress::IndexProgress;
use super::symbol_index::{SymbolIndex, TraitImpl};

/// Parse one project file and extract its definition symbols AND Rust
/// tables (010-01 — one parse, same tree). Plain text and unreadable files
/// yield the empty result.
pub fn extract_file(root: &Path, rel: &str) -> (Vec<Symbol>, RustTables) {
    let abs = root.join(rel);
    let text = match std::fs::read_to_string(&abs) {
        Ok(t) => t,
        Err(_) => return (Vec::new(), RustTables::default()),
    };
    let lang = resolve_language(rel);
    extract_all(lang, &text)
}

/// Build a full index for `root` over the (project-relative) `files` list
/// using a rayon-parallel parse. **Blocking** — call from a background
/// thread (the app wraps this in `spawn_blocking`). `progress` (if given) is
/// advanced to the file count as each file finishes.
pub fn build_index(root: &Path, files: &[String], progress: Option<&IndexProgress>) -> SymbolIndex {
    // Rayon fan-in: parse every file in parallel. Each worker owns its own
    // thread-local parser (see `queries::extract_all`); `progress` is
    // shared and cheap to bump from any worker.
    let entries: Vec<(String, Vec<Symbol>, RustTables)> = files
        .par_iter()
        .map(|rel| {
            let (syms, tables) = extract_file(root, rel);
            if let Some(p) = progress {
                p.note_file_done();
            }
            (rel.clone(), syms, tables)
        })
        .collect();

    let mut index = SymbolIndex::new();
    for (rel, syms, tables) in entries {
        if !syms.is_empty() {
            index.set_file(&rel, syms);
        }
        // 010-01: the Rust tables ride the same pass (same tree).
        index.set_file_tables(&rel, tables);
    }
    index
}

/// Re-parse ONLY the `changed` (absolute) files and update `self` in place;
/// deleted/unreadable files have their entries dropped. Returns the set of
/// project-relative paths actually touched — this is the O(changed-files)
/// incremental refresh (every other file's outline is left untouched).
pub fn refresh_in_place(root: &Path, changed: &[PathBuf], index: &mut SymbolIndex) -> Vec<String> {
    let mut touched = Vec::new();
    for abs in changed {
        let Some(rel) = abs.strip_prefix(root).ok() else {
            continue;
        };
        let rel = rel.to_string_lossy().into_owned();
        match std::fs::read_to_string(abs) {
            Ok(text) => {
                let lang = resolve_language(&rel);
                let (syms, tables) = extract_all(lang, &text);
                index.set_file(&rel, syms);
                // 010-01: the Rust tables ride the same reparse (same tree).
                index.set_file_tables(&rel, tables);
            }
            // Deleted / unreadable: drop its outline (no-op if absent).
            Err(_) => {
                index.remove_file(&rel);
            }
        }
        touched.push(rel);
    }
    touched
}

/// The innermost (narrowest) symbol whose `[line, end_line]` extent contains
/// `line`; `None` when the cursor line is outside every definition. Used for
/// which-function. Pure (testable in isolation).
pub fn enclosing_symbol(outline: &[Symbol], line: usize) -> Option<&Symbol> {
    let mut best: Option<&Symbol> = None;
    for s in outline {
        if s.line <= line && line <= s.end_line {
            let span = s.end_line.saturating_sub(s.line);
            let take = match best {
                None => true,
                Some(b) => {
                    let bspan = b.end_line.saturating_sub(b.line);
                    span < bspan || (span == bspan && s.line > b.line)
                }
            };
            if take {
                best = Some(s);
            }
        }
    }
    best
}

impl SymbolIndex {
    /// (010-01) Replace a file's Rust tables, keeping the name-keyed field
    /// map consistent (the same discipline as `set_file`). Empty tables
    /// store no entry. Returns `true` when the stored tables actually
    /// changed.
    pub fn set_file_tables(&mut self, path: &str, tables: RustTables) -> bool {
        // 010-03: bindings count as content — a file whose ONLY table
        // content is local bindings (no structs / impls) still stores an
        // entry, and the unchanged-content shortcut below (010-01 review
        // P1: the same-check runs BEFORE any removal) restores it rather
        // than stripping it.
        let empty = tables.fields.is_empty()
            && tables.impls.is_empty()
            && tables.bindings.is_empty();
        let old = self.rust_tables.remove(path);
        // 010-01 review P1: the unchanged-content shortcut MUST run before
        // any field-map removal — the removal is unconditional below, so a
        // no-op refresh (touch / linter rewrite / editor no-save) would
        // otherwise silently drop the file's field contributions and
        // `self.<field>` M-. resolution would degrade until the next real
        // change (mirrors set_file's order, index.rs:165).
        let same = old.as_ref() == Some(&tables);
        if same && !empty {
            // Restore the unchanged entry and bail.
            self.rust_tables.insert(path.to_string(), tables);
            return false;
        }
        // Drop this file's contributions from the field map.
        if let Some(old_tables) = &old {
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
        }
        // 010-04: drop this file's trait-map contributions (the same
        // removal discipline as the field map — it only runs when the
        // content really changed; the same-content shortcut above
        // already returned before any removal).
        if let Some(old_tables) = &old {
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
        if empty {
            return old.is_some();
        }
        for (struct_name, fields) in &tables.fields {
            let by_field = self
                .rust_fields
                .entry(struct_name.clone())
                .or_default();
            for f in fields {
                by_field
                    .entry(f.field.clone())
                    .or_default()
                    .entry(path.to_string())
                    .or_default()
                    .push(f.line);
            }
        }
        // 010-04: the trait-keyed map — one entry per (impl block, self
        // type) per trait; deduped so an impl's N methods do not multiply
        // the block's location.
        for (self_type, methods) in &tables.impls {
            for m in methods {
                if let ImplKind::Trait(t) = &m.kind {
                    let entries = self
                        .rust_traits
                        .entry(t.clone())
                        .or_default()
                        .entry(path.to_string())
                        .or_default();
                    let entry = TraitImpl {
                        impl_line: m.impl_line,
                        self_type: self_type.clone(),
                    };
                    if !entries.contains(&entry) {
                        entries.push(entry);
                    }
                }
            }
        }
        self.rust_tables.insert(path.to_string(), tables);
        true
    }
}
