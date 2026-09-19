//! widgets: mechanical parts used by the gearbox sample.
//!
//! Deep-module corpus target: `widgets::gear::Gear` and friends.

pub mod gear;
pub mod spindle;

/// A gear ratio expressed as `teeth` over `module`.
#[derive(Debug, Clone, Copy)]
pub struct GearRatio {
    pub teeth: u32,
    pub module: u32,
}

impl GearRatio {
    /// The ratio as a plain f64.
    pub fn as_f64(&self) -> f64 {
        self.teeth as f64 / self.module as f64
    }

    /// Drive ratio against a mating gear's `ratio`.
    pub fn against(&self, ratio: GearRatio) -> f64 {
        self.as_f64() / ratio.as_f64()
    }
}

/// Default spindle speed for the sample (rpm).
pub const DEFAULT_RPM: u32 = 1800;
