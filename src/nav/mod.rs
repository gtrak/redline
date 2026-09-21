//! Symbol navigation (issue 05): the background symbol index the commands
//! run against. Plain Rust (tree-sitter + rayon + tokio; no iocraft), per
//! the plan's layering rule.

pub mod index;
pub mod xref;
