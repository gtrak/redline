//! frobnicator: a vendored utility crate kept OUTSIDE the workspace root
//! (a sibling of the workspace) so the corpus can exercise the external
//! source-root path without any registry fetch.

/// A frobnicator with a strength and a phase.
#[derive(Debug, Clone, Copy)]
pub struct Frobnicator {
    pub strength: u32,
    pub phase: f64,
}

impl Frobnicator {
    /// Apply the frobnicator to `value`.
    pub fn apply(&self, value: f64) -> f64 {
        value * (self.strength as f64 + self.phase)
    }
}
