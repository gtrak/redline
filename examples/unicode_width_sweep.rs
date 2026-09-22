//! The full-domain unicode-width table sweep behind deps batch B
//! (0.1.14 -> 0.2.2).
//!
//! Sweeps ALL 1,112,064 Unicode scalar values (U+0000..=U+10FFFF minus the
//! surrogate gap) through both the OLD table (unicode-width 0.1.14, kept as
//! the dev-only `unicode-width-old` dependency) and the current table
//! (unicode-width 0.2.2, Unicode 17.0), and prints:
//!
//!   total_scalars=... changed=...
//!   <old width -> new width>: <count>   (one line per bucket)
//!   <codepoint>: <old> -> <new>         (one line per changed scalar)
//!
//! Regenerate / re-verify the recorded numbers (tools/unicode_width_sweep.md):
//!
//!     cargo run --example unicode_width_sweep
//!
//! Why FULL-DOMAIN rather than a sample: the fixed 7-character sample
//! (narrow / CJK-wide / emoji / combining / 3 ambiguous-width) showed ZERO
//! changes, while the full domain showed 458 — the sample alone would have
//! concluded "no behaviour change". The 458 deltas are East Asian Width
//! RECLASSIFICATIONS (Neutral -> Wide, and mark -> combining) under the
//! Unicode 17.0 tables; NONE of them are in the EAW "Ambiguous" class,
//! which changed zero widths (n=138,197 -> 138,232, all still 1-cell in
//! the default non-CJK table). The pins in src/model/text_width.rs anchor
//! the deltas and the stable classes.

use std::collections::BTreeMap;

use unicode_width::UnicodeWidthChar;

fn main() {
    let mut total = 0usize;
    let mut changed = 0usize;
    let mut buckets: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut diffs: Vec<String> = Vec::new();
    for cp in 0u32..=0x10FFFF {
        if (0xD800..=0xDFFF).contains(&cp) {
            continue; // surrogate gap — not a scalar value
        }
        let Some(c) = char::from_u32(cp) else { continue };
        total += 1;
        let old = unicode_width_old::UnicodeWidthChar::width(c).unwrap_or(1);
        let new = c.width().unwrap_or(1);
        if old != new {
            changed += 1;
            *buckets.entry((old, new)).or_insert(0) += 1;
            diffs.push(format!("{cp:05X}: {old} -> {new}"));
        }
    }
    println!("total_scalars={total}");
    println!("changed={changed}");
    for ((old, new), count) in &buckets {
        println!("{old}->{new}: {count}");
    }
    for d in &diffs {
        println!("{d}");
    }
}
