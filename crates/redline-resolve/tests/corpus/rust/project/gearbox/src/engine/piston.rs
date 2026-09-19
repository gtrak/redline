//! A piston helper: generic reporting for the sample binary.

/// Describe a piston event as a string.
///
/// `T: Display` is the generic argument the binary pins with turbofish.
pub fn describe<T: std::fmt::Display>(teeth: u32, ratio: T) -> String {
    format!("piston-{teeth} @ {ratio}")
}
