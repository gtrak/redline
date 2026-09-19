//! Engine assembly (a directory module with a `piston` submodule).

pub mod piston;

use crate::core::math;
use widgets::gear::Gear;

/// A two-stage engine: an input gear driving an output gear.
pub struct Engine {
    pub input: Gear,
    pub output: Gear,
}

impl Engine {
    /// Assemble an engine from an `input` and `output` gear.
    pub fn assemble(input: Gear, output: Gear) -> Self {
        Self { input, output }
    }

    /// Overall ratio (output teeth / input teeth), folded through the
    /// deep chain `gearbox::core::math::prod` for the corpus sample.
    pub fn ratio(&self) -> f64 {
        math::prod(&[
            self.output.teeth as f64 / self.input.teeth as f64,
        ])
    }
}
