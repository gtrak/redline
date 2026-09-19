//! gearbox: assembles widgets into drivetrains (corpus sample).
//!
//! Exercises the redline rust golden corpus: a module tree, deep `::`
//! chains, turbofish, generic arguments, use-aliases, and prelude names.

pub mod core;
pub mod engine;

use widgets::gear::Gear as DriveGear;
use widgets::GearRatio;

/// A drivetrain: a gearbox with one gear per stage.
pub struct Drivetrain {
    pub stages: Vec<DriveGear>,
    pub name: String,
}

impl Drivetrain {
    /// Build a drivetrain stage at `stage` with `gear`.
    pub fn add_stage(&mut self, stage: usize, gear: DriveGear) {
        self.stages.insert(stage, gear);
    }

    /// Total ratio across all stages (1.0 for an empty drivetrain).
    pub fn total_ratio(&self) -> f64 {
        self.stages.iter().fold(1.0, |acc, g| acc * g.ratio())
    }
}

/// Index gear ratios by tooth count (the turbofish pins the value type).
pub fn gear_lookup(ratios: &[GearRatio]) -> std::collections::HashMap<u32, f64> {
    use std::collections::HashMap;
    let mut table: HashMap<u32, f64> = HashMap::<u32, f64>::new();
    for ratio in ratios {
        table.insert(ratio.teeth, ratio.as_f64());
    }
    table
}
