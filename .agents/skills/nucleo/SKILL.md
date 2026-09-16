---
name: nucleo
description: >-
  Reference for nucleo-matcher 0.3.1, the low-level fuzzy matcher that powers
  redline's helm-style Picker filtering. Covers Config, Matcher, the pattern
  API (Pattern/parse/new), CaseMatching/Normalization/AtomKind, Utf32Str/
  Utf32String haystack input, scores and match indices, and a verified
  filter-and-sort snippet. All APIs verified against docs.rs (0.3.1).
---

# nucleo

`nucleo-matcher` is the low-level fuzzy matcher implementation used by the
high-level `nucleo` crate (which helix uses). It is fast and outperforms
fzf/skim, but the API is less convenient by design.

> **`nucleo` 0.5.0** is the streaming *picker engine* that wraps this crate
> (parallel/streamed scoring for very large or streaming candidate sets). It is
> **not pinned** in `Cargo.toml` yet — add `nucleo = "0.5"` later only if a
> picker must match a very large (> ~thousands) or streaming list. For
> redline's current pickers (files, symbols, commands, branches — bounded,
> in-memory) `nucleo-matcher` directly is sufficient. The docs warn that
> driving the matcher in a UI loop for large sets is slow; `match_list`/
> `score` below are fine for the bounded lists redline keeps in memory.

## Version

`nucleo-matcher = "0.3.1"` (pinned in `Cargo.toml` under
`# ── fuzzy matching (helm-style pickers) ──`). Pinned because the plan's
helm-style Picker (issue 02/05) needs a stable, fast matcher and 0.3.x is the
release helix ships.

## Core API

All verified against https://docs.rs/nucleo-matcher/0.3.1/nucleo_matcher/.

### `Config`
```rust
#[non_exhaustive] pub struct Config {
    pub normalize: bool,      // normalize latin script to ASCII (default on)
    pub ignore_case: bool,
    pub prefer_prefix: bool,  // small bonus for matches near the start
    /* private fields */
}
```
- `Config::DEFAULT` — associated const (can't use `Default::default()` in const
  contexts).
- `Config::match_paths(self) -> Config` / `set_match_paths(&mut self)` —
  bonuses appropriate for file paths (use for the file picker).

### `Matcher`
```rust
pub struct Matcher { pub config: Config, /* private fields */ }
```
- `Matcher::new(config: Config) -> Matcher` — **eagerly allocates ~135KB** of
  reused scratch so matching never allocates (except growing an `indices`
  vec). Reuse one matcher; do not build one per keystroke.
- `Matcher: Default` (uses default config).
- Direct match calls (rarely needed — prefer `Pattern`): `fuzzy_match`,
  `substring_match`, `exact_match`, `prefix_match`, `postfix_match` (each has
  a `_indices` twin and a `_greedy` fuzzy variant). All take
  `haystack: Utf32Str, needle: Utf32Str` and return `Option<u16>` (score) —
  **needle must be pre-normalized/case-folded by the caller**.
- **Panics** if a haystack exceeds 2^32-1 codepoints.

### `pattern` module (preferred API)
- `Pattern::parse(pattern: &str, case: CaseMatching, normalize: Normalization)
  -> Pattern` — splits on whitespace into per-word atoms; supports the special
  word-boundary markers `^` (prefix), `$` (postfix), `!` (exact), and plain
  fuzzy otherwise (see `AtomKind`). Escape with backslash.
- `Pattern::new(pattern, case, normalize, kind: AtomKind) -> Pattern` — each
  word matched individually with a fixed `AtomKind`, no special-char parsing.
- `pattern.reparse(...)` — re-parse in place (reuses allocations).

Matching (all take `&mut Matcher`; `ignore_case` is overwritten per-atom to
match each atom's casing):
- `pattern.score(haystack: Utf32Str, matcher: &mut Matcher) -> Option<u32>` —
  ranking score, `None` if no match.
- `pattern.indices(haystack, matcher, indices: &mut Vec<u32>) -> Option<u32>` —
  score + matched char indices **appended** to `indices` (not sorted/deduped;
  `indices.sort_unstable(); indices.dedup();` for highlighting).
- `pattern.match_list<T: AsRef<str>>(items: impl IntoIterator<Item = T>,
  matcher: &mut Matcher) -> Vec<(T, u32)>` — match **and sort** a small list,
  returned best-first (score descending). Convenient, but blocks the calling
  thread — fine for redline's bounded lists, not for huge/streaming sets.

### Supporting enums (`pattern` module)
- `CaseMatching` — `Ignore`, `Explicit`, `Prefer` (how to treat case mismatch).
- `Normalization` — `Ignore`, `Smart`, `Deep` (unicode normalization level).
- `AtomKind` — `Fuzzy`, `Exact`, `Prefix`, `Postfix`, `Substring`.

### `Utf32Str` / `Utf32String` (haystack input)
```rust
pub enum Utf32Str<'a> { Ascii(&'a [u8]), Unicode(&'a [char]) }
```
- `Utf32Str::new(str: &'a str, buf: &'a mut Vec<char>) -> Utf32Str<'a>` — build
  from a utf8 `str`; ascii-only text needs no copy, non-ascii fills `buf` with
  presegmented codepoints. `Utf32Str` is `Copy`. Methods: `len()`, `is_empty()`,
  `is_ascii()`, `get(n: u32) -> char`, `slice(range)`, `slice_u32(range)`
  (u32 range matches matcher indices), `chars()`.
- `Utf32String` is the **owned** version (holds a `Vec<char>`), for when you
  need to cache a presegmented haystack across matches.

> **Column options**: `nucleo-matcher` 0.3.1 has **no column/multi-field API**
> (no `MatchColumn`/`Column` type). That lives in the higher-level `nucleo`
> crate. To match against several fields in redline, build a single haystack
> string per candidate (e.g. `format!("{name} {path}")`) or run separate
> `Pattern` matches and combine scores manually.

## Usage in redline

Helm-style `Picker` (plan decision #3) filters a bounded candidate list:

```rust
use nucleo_matcher::{
    Config, Matcher,
    pattern::{CaseMatching, Normalization, Pattern},
};

/// Filter `candidates` by `query`, returning (candidate, score) best-first.
/// `matcher` should be a long-lived `Matcher` (store on app state), built once
/// with `Matcher::new(Config::DEFAULT.match_paths())` for files or
/// `Matcher::new(Config::DEFAULT)` for symbols/commands.
fn filter<'a>(matcher: &mut Matcher, query: &str, candidates: &'a [String])
    -> Vec<(&'a String, u32)> {
    if query.is_empty() {
        return candidates.iter().map(|c| (c, u32::MAX)).collect();
    }
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    // match_list takes items: AsRef<str>, returns them sorted by score desc.
    pattern
        .match_list(candidates.iter().collect::<Vec<_>>(), matcher)
        .into_iter()
        .map(|(s, score)| (s, score))
        .collect()
}
```
For highlighting, call `pattern.indices(haystack, matcher, &mut idx)` with
`haystack = Utf32Str::new(s, &mut buf)`, then `sort_unstable`+`dedup`. Reuse
one `Matcher` and one `Vec<char>` buffer across keystrokes.

## Gotchas

- **Reuse the matcher**: `Matcher::new` allocates ~135KB; constructing one per
  keystroke in the picker hot path is a perf bug. Store it in app state.
- **`match_list` blocks the thread** and is only recommended for "relatively
  small" lists. If a picker ever matches tens of thousands of files, switch to
  the high-level `nucleo` crate (streamed/parallel) rather than `match_list`.
- **Needle normalization is your job** when calling `Matcher` methods
  directly (`fuzzy_match` etc.); the `pattern` API handles it — prefer that.
- **`Utf32Str::new` borrows `buf`**: the returned `Utf32Str` is only valid as
  long as the `Vec<char>` outlives it (for non-ascii input). Keep the buffer
  alive for the duration of the match.
- **Indices are char (codepoint) offsets, not utf8 bytes** — matches
  `Utf32Str`'s codepoint representation. Map to utf8 offsets via the source
  string when you need byte positions for rendering.
- **`Pattern::indices` does not sort/dedup** — indices from multiple atoms are
  appended; sort and dedup before highlighting.
- **`Config`/`Pattern` are `#[non_exhaustive]`** — don't construct with struct
  literal syntax; use the documented constructors and builders.
- **`match_list` requires `T: AsRef<str>`** and returns the same `T` you
  passed in; pass `.iter()` to get references back rather than cloning.
