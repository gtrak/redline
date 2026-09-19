//! Spindles.

/// A spindle with a diameter (mm) and a rated speed (rpm).
#[derive(Debug, Clone, Copy)]
pub struct Spindle {
    pub diameter_mm: f32,
    pub rated_rpm: u32,
}

impl Spindle {
    /// A spindle at `diameter_mm` rated for `rpm`.
    pub fn new(diameter_mm: f32, rpm: u32) -> Self {
        Self {
            diameter_mm,
            rated_rpm: rpm,
        }
    }

    /// Surface speed in m/min at `rpm`.
    pub fn surface_speed(&self, rpm: u32) -> f32 {
        std::f32::consts::PI * self.diameter_mm * rpm as f32 / 1000.0
    }
}
