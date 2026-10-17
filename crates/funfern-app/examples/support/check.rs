//! How the device examples judge a run, and how long they give it.
//! `error > tolerance` is false for a value that is not a number, and
//! `f64::max` drops one, so a run whose device, readback or reference produced
//! a NaN passed every check written that way. These fail it instead. Each
//! example includes this file as a module of its own; not every one uses all
//! of it.

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

/// When a run gives up: `FUNFERN_E2E_DEADLINE` seconds from now where it is
/// set, as `scripts/e2e-fixture.sh` sets it on Linux a minute inside its own
/// limit, and `seconds` otherwise. The clock starts before the app does, so
/// it counts the device's start and its pipelines' compilation: on lavapipe
/// the temporal handoff took its first step 155 s in, past its 90.
#[allow(dead_code)]
pub fn deadline(seconds: u64) -> std::time::Instant {
    let seconds = std::env::var("FUNFERN_E2E_DEADLINE")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(seconds);
    std::time::Instant::now() + std::time::Duration::from_secs(seconds)
}
