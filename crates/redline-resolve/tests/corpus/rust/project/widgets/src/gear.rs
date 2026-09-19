//! Gears.

use crate::GearRatio;

/// A physical gear with a tooth count and a module.
#[derive(Debug, Clone, Copy)]
pub struct Gear {
    pub teeth: u32,
    pub module: u32,
}

impl Gear {
    /// Construct a gear with `teeth` at `module`.
    pub fn new(teeth: u32, module: u32) -> Self {
        Self { teeth, module }
    }

    /// The gear's ratio as a `GearRatio`.
    pub fn ratio(&self) -> f64 {
        GearRatio {
            teeth: self.teeth,
            module: self.module,
        }
        .as_f64()
    }

    /// Is this gear smaller than `other`?
    pub fn is_smaller_than(&self, other: &Gear) -> bool {
        self.teeth < other.teeth
    }
}
