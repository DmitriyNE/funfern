//! How the device examples judge a run. `error > tolerance` is false for a
//! value that is not a number, and `f64::max` drops one, so a run whose device,
//! readback or reference produced a NaN passed every check written that way.
//! These fail it instead. Each example includes this file as a module of its
//! own; not every one uses both.

/// Whether `error` is a number no larger than `tolerance`. A NaN is not, nor
/// an infinity.
#[allow(dead_code)]
pub fn within(error: f64, tolerance: f64) -> bool {
    error.is_finite() && error <= tolerance
}

/// The worse of two errors, and not a number if either is not, where
/// `f64::max` keeps the other.
#[allow(dead_code)]
pub fn worse(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}
