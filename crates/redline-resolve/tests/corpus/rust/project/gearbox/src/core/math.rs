//! Flat math helpers.

/// Sum of squares over `values`.
pub fn sum_sq(values: &[f64]) -> f64 {
    values.iter().map(|v| v * v).sum()
}

/// Product of `values`, 1.0 for an empty slice.
pub fn prod(values: &[f64]) -> f64 {
    values.iter().product()
}
