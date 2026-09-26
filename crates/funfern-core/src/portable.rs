//! Transcendental functions that give the same bits on every platform.
//!
//! `f64::acos`, `sin`, `cos`, `tan`, `atan2` and `hypot` call the platform's
//! maths library, which is not correctly rounded and differs in the last bit
//! between macOS, glibc (with or without FMA) and the browser build. The
//! mesher ranks triangles by their smallest angle and sorts directions by
//! angle, so a last bit reordered its refinement and the same geometry meshed
//! differently on Linux and macOS. These are fdlibm's algorithms, as musl and
//! the `libm` crate carry them, written with the four operations and `sqrt`
//! only, which IEEE 754 fixes exactly; Rust never fuses them. Their constants
//! are fdlibm's, given by their bits.

const PIO2_HI: f64 = f64::from_bits(0x3FF9_21FB_5444_2D18);
const PIO2_LO: f64 = f64::from_bits(0x3C91_A626_3314_5C07);

/// `acos`'s rational approximation of `(asin(√z) − √z)/√z` in `z`.
fn asin_ratio(z: f64) -> f64 {
    const PS0: f64 = f64::from_bits(0x3FC5_5555_5555_5555);
    const PS1: f64 = f64::from_bits(0xBFD4_D612_03EB_6F7D);
    const PS2: f64 = f64::from_bits(0x3FC9_C155_0E88_4455);
    const PS3: f64 = f64::from_bits(0xBFA4_8228_B568_8F3B);
    const PS4: f64 = f64::from_bits(0x3F49_EFE0_7501_B288);
    const PS5: f64 = f64::from_bits(0x3F02_3DE1_0DFD_F709);
    const QS1: f64 = f64::from_bits(0xC003_3A27_1C8A_2D4B);
    const QS2: f64 = f64::from_bits(0x4000_2AE5_9C59_8AC8);
    const QS3: f64 = f64::from_bits(0xBFE6_066C_1B8D_0159);
    const QS4: f64 = f64::from_bits(0x3FB3_B8C5_B12E_9282);
    let p = z * (PS0 + z * (PS1 + z * (PS2 + z * (PS3 + z * (PS4 + z * PS5)))));
    let q = 1.0 + z * (QS1 + z * (QS2 + z * (QS3 + z * QS4)));
    p / q
}

/// `acos(x)` in radians, the same on every platform.
pub fn portable_acos(x: f64) -> f64 {
    let high = (x.to_bits() >> 32) as u32;
    let magnitude = high & 0x7fff_ffff;
    if magnitude >= 0x3ff0_0000 {
        let low = x.to_bits() as u32;
        if (magnitude - 0x3ff0_0000) | low == 0 {
            return if high >> 31 != 0 { 2.0 * PIO2_HI } else { 0.0 };
        }
        return f64::NAN;
    }
    if magnitude < 0x3fe0_0000 {
        if magnitude <= 0x3c60_0000 {
            return PIO2_HI;
        }
        return PIO2_HI - (x - (PIO2_LO - x * asin_ratio(x * x)));
    }
    if high >> 31 != 0 {
        let z = (1.0 + x) * 0.5;
        let s = z.sqrt();
        let w = asin_ratio(z) * s - PIO2_LO;
        return 2.0 * (PIO2_HI - (s + w));
    }
    let z = (1.0 - x) * 0.5;
    let s = z.sqrt();
    let df = f64::from_bits(s.to_bits() & 0xffff_ffff_0000_0000);
    let c = (z - df * df) / (s + df);
    let w = asin_ratio(z) * s + c;
    2.0 * (df + w)
}

/// fdlibm's `sin` on `[−π/4, π/4]`, of `x + y` with `y` a tail below `x`'s
/// last bit.
fn kernel_sin(x: f64, y: f64, tail: bool) -> f64 {
    const S1: f64 = f64::from_bits(0xBFC5_5555_5555_5549);
    const S2: f64 = f64::from_bits(0x3F81_1111_1110_F8A6);
    const S3: f64 = f64::from_bits(0xBF2A_01A0_19C1_61D5);
    const S4: f64 = f64::from_bits(0x3EC7_1DE3_57B1_FE7D);
    const S5: f64 = f64::from_bits(0xBE5A_E5E6_8A2B_9CEB);
    const S6: f64 = f64::from_bits(0x3DE5_D93A_5ACF_D57C);
    let z = x * x;
    let w = z * z;
    let r = S2 + z * (S3 + z * S4) + z * w * (S5 + z * S6);
    let v = z * x;
    if tail {
        x - ((z * (0.5 * y - v * r) - y) - v * S1)
    } else {
        x + v * (S1 + z * r)
    }
}

/// fdlibm's `cos` on `[−π/4, π/4]`, of `x + y`.
fn kernel_cos(x: f64, y: f64) -> f64 {
    const C1: f64 = f64::from_bits(0x3FA5_5555_5555_554C);
    const C2: f64 = f64::from_bits(0xBF56_C16C_16C1_5177);
    const C3: f64 = f64::from_bits(0x3EFA_01A0_19CB_1590);
    const C4: f64 = f64::from_bits(0xBE92_7E4F_809C_52AD);
    const C5: f64 = f64::from_bits(0x3E21_EE9E_BDB4_B1C4);
    const C6: f64 = f64::from_bits(0xBDA8_FAE9_BE88_38D4);
    let z = x * x;
    let w = z * z;
    let r = z * (C1 + z * (C2 + z * C3)) + w * w * (C4 + z * (C5 + z * C6));
    let hz = 0.5 * z;
    let w = 1.0 - hz;
    w + (((1.0 - w) - hz) + (z * r - x * y))
}

/// `x` less the nearest multiple `n` of `π/2`, as a head and a tail, and
/// `n`; fdlibm's reduction for arguments up to about `2²⁰ π/2`, which every
/// angle the geometry forms is well inside.
fn reduce_half_pi(x: f64) -> (i64, f64, f64) {
    const TO_INT: f64 = 1.5 / f64::EPSILON;
    const INV_PIO2: f64 = f64::from_bits(0x3FE4_5F30_6DC9_C883);
    const PIO2_1: f64 = f64::from_bits(0x3FF9_21FB_5440_0000);
    const PIO2_1T: f64 = f64::from_bits(0x3DD0_B461_1A62_6331);
    const PIO2_2: f64 = f64::from_bits(0x3DD0_B461_1A60_0000);
    const PIO2_2T: f64 = f64::from_bits(0x3BA3_198A_2E03_7073);
    const PIO2_3: f64 = f64::from_bits(0x3BA3_198A_2E00_0000);
    const PIO2_3T: f64 = f64::from_bits(0x397B_839A_2520_49C1);
    let exponent = |value: f64| ((value.to_bits() >> 52) & 0x7ff) as i64;
    let multiple = x * INV_PIO2 + TO_INT - TO_INT;
    let n = multiple as i64;
    let mut r = x - multiple * PIO2_1;
    let mut w = multiple * PIO2_1T;
    let mut head = r - w;
    let first = exponent(x);
    if first - exponent(head) > 16 {
        // Cancellation took more than 16 bits: a second, finer round.
        let t = r;
        w = multiple * PIO2_2;
        r = t - w;
        w = multiple * PIO2_2T - ((t - r) - w);
        head = r - w;
        if first - exponent(head) > 49 {
            let t = r;
            w = multiple * PIO2_3;
            r = t - w;
            w = multiple * PIO2_3T - ((t - r) - w);
            head = r - w;
        }
    }
    let tail = (r - head) - w;
    (n, head, tail)
}

/// `(sin x, cos x)`, the same on every platform, for `|x|` up to about a
/// million.
pub fn portable_sin_cos(x: f64) -> (f64, f64) {
    let magnitude = ((x.to_bits() >> 32) as u32) & 0x7fff_ffff;
    if magnitude <= 0x3fe9_21fb {
        // |x| ≤ π/4.
        if magnitude < 0x3e46_a09e {
            // |x| < 2⁻²⁷·√2: sin x = x and cos x = 1 to the last bit.
            return (x, 1.0);
        }
        return (kernel_sin(x, 0.0, false), kernel_cos(x, 0.0));
    }
    if !x.is_finite() {
        return (f64::NAN, f64::NAN);
    }
    let (n, head, tail) = reduce_half_pi(x);
    let (sin, cos) = (kernel_sin(head, tail, true), kernel_cos(head, tail));
    match n & 3 {
        0 => (sin, cos),
        1 => (cos, -sin),
        2 => (-sin, -cos),
        _ => (-cos, sin),
    }
}

/// `tan x` as `sin x / cos x`, the same on every platform.
pub fn portable_tan(x: f64) -> f64 {
    let (sin, cos) = portable_sin_cos(x);
    sin / cos
}

/// A key that orders directions as `atan2(y, x)` does, over `(−2, 2]` for
/// `(−π, π]`: the diamond angle, `y/(|x| + |y|)` laid quadrant after
/// quadrant, which is monotone in the true angle and needs one division.
/// The zero vector keys to zero.
pub fn pseudo_angle(x: f64, y: f64) -> f64 {
    let sum = x.abs() + y.abs();
    if sum == 0.0 {
        return 0.0;
    }
    let p = y / sum;
    if x >= 0.0 {
        // From −1 at −y through 0 at +x to 1 at +y.
        p
    } else if y >= 0.0 {
        // From 1 at +y to 2 at −x.
        2.0 - p
    } else {
        // From −2, just past −x, to −1 at −y.
        -2.0 - p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ulps(a: f64, b: f64) -> u64 {
        if a == b {
            return 0;
        }
        (a.to_bits() as i64 - b.to_bits() as i64).unsigned_abs()
    }

    /// A fixed, dependency-free sequence over `[0, 1)`.
    fn samples(count: usize) -> impl Iterator<Item = f64> {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        (0..count).map(move |_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        })
    }

    /// Within a last bit of the platform's `acos` over the whole domain, and
    /// exact at its ends.
    #[test]
    fn portable_acos_matches_the_platform_to_a_last_bit() {
        for value in samples(200_000).map(|u| 2.0 * u - 1.0) {
            let (ours, theirs) = (portable_acos(value), value.acos());
            assert!(
                ulps(ours, theirs) <= 1,
                "acos({value:e}): {ours:e} against {theirs:e}"
            );
        }
        assert_eq!(portable_acos(1.0), 0.0);
        assert_eq!(portable_acos(-1.0), std::f64::consts::PI);
        assert!(portable_acos(1.5).is_nan());
    }

    /// Within a last bit of the platform's `sin` and `cos` from −100 to 100,
    /// and exact where they are small.
    #[test]
    fn portable_sine_and_cosine_match_the_platform_to_a_last_bit() {
        for value in samples(200_000).map(|u| 200.0 * u - 100.0) {
            let (sin, cos) = portable_sin_cos(value);
            let (platform_sin, platform_cos) = value.sin_cos();
            // Near a zero a last bit is a large share of a tiny value, so
            // those are judged absolutely.
            let close =
                |ours: f64, theirs: f64| ulps(ours, theirs) <= 1 || (ours - theirs).abs() <= 1e-16;
            assert!(
                close(sin, platform_sin),
                "sin({value:e}): {sin:e} against {platform_sin:e}"
            );
            assert!(
                close(cos, platform_cos),
                "cos({value:e}): {cos:e} against {platform_cos:e}"
            );
        }
        assert_eq!(portable_sin_cos(0.0), (0.0, 1.0));
        assert_eq!(portable_sin_cos(1e-10), (1e-10, 1.0));
    }

    /// The pseudo-angle orders directions exactly as `atan2`, including the
    /// axes and the half-turn at `−x`.
    #[test]
    fn the_pseudo_angle_orders_directions_as_atan2() {
        let mut directions = samples(4_000)
            .zip(samples(8_000).skip(4_000))
            .map(|(u, v)| (2.0 * u - 1.0, 2.0 * v - 1.0))
            .collect::<Vec<_>>();
        directions.extend([(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0), (3.0, 3.0)]);
        let mut by_atan2 = directions.clone();
        by_atan2.sort_by(|a, b| a.1.atan2(a.0).total_cmp(&b.1.atan2(b.0)));
        let mut by_key = directions.clone();
        by_key.sort_by(|a, b| pseudo_angle(a.0, a.1).total_cmp(&pseudo_angle(b.0, b.1)));
        assert_eq!(by_atan2, by_key);
        assert_eq!(pseudo_angle(-1.0, 0.0), 2.0);
        assert_eq!(pseudo_angle(1.0, 0.0), 0.0);
    }
}
