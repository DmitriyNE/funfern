use crate::Point2;

/// The exact sign of a geometric predicate evaluated on finite `f64` inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PredicateSign {
    Negative,
    Zero,
    Positive,
}

impl PredicateSign {
    pub const fn reversed(self) -> Self {
        match self {
            Self::Negative => Self::Positive,
            Self::Zero => Self::Zero,
            Self::Positive => Self::Negative,
        }
    }

    pub const fn is_positive(self) -> bool {
        matches!(self, Self::Positive)
    }

    pub const fn is_negative(self) -> bool {
        matches!(self, Self::Negative)
    }
}

fn sign_of(expansion: &[f64]) -> PredicateSign {
    match expansion.last().copied().unwrap_or(0.0) {
        value if value > 0.0 => PredicateSign::Positive,
        value if value < 0.0 => PredicateSign::Negative,
        _ => PredicateSign::Zero,
    }
}

fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let x = a + b;
    let b_virtual = x - a;
    let a_virtual = x - b_virtual;
    let b_roundoff = b - b_virtual;
    let a_roundoff = a - a_virtual;
    (a_roundoff + b_roundoff, x)
}

fn fast_two_sum(a: f64, b: f64) -> (f64, f64) {
    let x = a + b;
    (b - (x - a), x)
}

fn split(a: f64) -> (f64, f64) {
    // 2^27 + 1 splits a binary64 significand into non-overlapping halves.
    const SPLITTER: f64 = 134_217_729.0;
    let c = SPLITTER * a;
    let high = c - (c - a);
    (high, a - high)
}

fn two_product(a: f64, b: f64) -> (f64, f64) {
    let x = a * b;
    let (a_high, a_low) = split(a);
    let (b_high, b_low) = split(b);
    let error = a_low * b_low - (((x - a_high * b_high) - a_low * b_high) - a_high * b_low);
    (error, x)
}

fn product(a: f64, b: f64) -> Vec<f64> {
    let (low, high) = two_product(a, b);
    if low == 0.0 {
        if high == 0.0 { vec![] } else { vec![high] }
    } else {
        vec![low, high]
    }
}

/// Adds non-overlapping expansions, eliminating zero components.
fn expansion_sum(left: &[f64], right: &[f64]) -> Vec<f64> {
    if left.is_empty() {
        return right.to_vec();
    }
    if right.is_empty() {
        return left.to_vec();
    }

    let mut output = Vec::with_capacity(left.len() + right.len());
    let mut left_index = 0;
    let mut right_index = 0;
    let mut accumulator = if left[0].abs() < right[0].abs() {
        left_index += 1;
        left[0]
    } else {
        right_index += 1;
        right[0]
    };

    while left_index < left.len() && right_index < right.len() {
        let next = if left[left_index].abs() < right[right_index].abs() {
            let value = left[left_index];
            left_index += 1;
            value
        } else {
            let value = right[right_index];
            right_index += 1;
            value
        };
        let (roundoff, sum) = two_sum(accumulator, next);
        if roundoff != 0.0 {
            output.push(roundoff);
        }
        accumulator = sum;
    }

    for next in left[left_index..]
        .iter()
        .chain(&right[right_index..])
        .copied()
    {
        let (roundoff, sum) = two_sum(accumulator, next);
        if roundoff != 0.0 {
            output.push(roundoff);
        }
        accumulator = sum;
    }
    if accumulator != 0.0 || output.is_empty() {
        output.push(accumulator);
    }
    output
}

fn expansion_negate(value: &[f64]) -> Vec<f64> {
    value.iter().map(|component| -*component).collect()
}

fn scale_expansion(value: &[f64], scale: f64) -> Vec<f64> {
    if value.is_empty() || scale == 0.0 {
        return vec![];
    }
    let mut output = Vec::with_capacity(value.len() * 2);
    let (low, mut accumulator) = two_product(value[0], scale);
    if low != 0.0 {
        output.push(low);
    }
    for component in &value[1..] {
        let (product_low, product_high) = two_product(*component, scale);
        let (sum_low, sum) = two_sum(accumulator, product_low);
        if sum_low != 0.0 {
            output.push(sum_low);
        }
        let (roundoff, next) = fast_two_sum(product_high, sum);
        if roundoff != 0.0 {
            output.push(roundoff);
        }
        accumulator = next;
    }
    if accumulator != 0.0 || output.is_empty() {
        output.push(accumulator);
    }
    output
}

fn expansion_product(left: &[f64], right: &[f64]) -> Vec<f64> {
    let mut output = vec![];
    for component in right {
        output = expansion_sum(&output, &scale_expansion(left, *component));
    }
    output
}

fn exact_sum(terms: impl IntoIterator<Item = Vec<f64>>) -> Vec<f64> {
    terms
        .into_iter()
        .fold(vec![], |sum, term| expansion_sum(&sum, &term))
}

/// Returns the exact orientation sign of `a`, `b`, `c`.
///
/// Positive means counter-clockwise. Products and their sum are represented as
/// non-overlapping floating-point expansions, so a nearly collinear input is not
/// classified by an epsilon. Inputs must be finite.
pub fn orient2d(a: Point2, b: Point2, c: Point2) -> PredicateSign {
    debug_assert!(a.finite() && b.finite() && c.finite());
    let acx = a.x - c.x;
    let bcx = b.x - c.x;
    let acy = a.y - c.y;
    let bcy = b.y - c.y;
    let left = acx * bcy;
    let right = acy * bcx;
    let determinant = left - right;
    let error_bound = 3.330_669_073_875_471_6e-16 * (left.abs() + right.abs());
    if determinant.abs() > error_bound {
        return if determinant > 0.0 {
            PredicateSign::Positive
        } else {
            PredicateSign::Negative
        };
    }
    // This un-translated form retains subtraction tails automatically because
    // each product is exact before the expansion sum.
    let terms = [
        product(a.x, b.y),
        expansion_negate(&product(a.y, b.x)),
        product(b.x, c.y),
        expansion_negate(&product(b.y, c.x)),
        product(c.x, a.y),
        expansion_negate(&product(c.y, a.x)),
    ];
    sign_of(&exact_sum(terms))
}

fn permutation_parity(permutation: [usize; 4]) -> bool {
    let inversions = (0..4)
        .flat_map(|i| (i + 1..4).map(move |j| (i, j)))
        .filter(|(i, j)| permutation[*i] > permutation[*j])
        .count();
    inversions % 2 == 0
}

/// Returns the exact in-circle sign for the oriented triangle `a`, `b`, `c`.
///
/// For a counter-clockwise triangle, positive means `d` lies inside its
/// circumcircle. The result reverses for a clockwise triangle.
pub fn incircle(a: Point2, b: Point2, c: Point2, d: Point2) -> PredicateSign {
    debug_assert!(a.finite() && b.finite() && c.finite() && d.finite());
    let adx = a.x - d.x;
    let ady = a.y - d.y;
    let bdx = b.x - d.x;
    let bdy = b.y - d.y;
    let cdx = c.x - d.x;
    let cdy = c.y - d.y;
    let bdxcdy = bdx * cdy;
    let cdxbdy = cdx * bdy;
    let cdxady = cdx * ady;
    let adxcdy = adx * cdy;
    let adxbdy = adx * bdy;
    let bdxady = bdx * ady;
    let alift = adx * adx + ady * ady;
    let blift = bdx * bdx + bdy * bdy;
    let clift = cdx * cdx + cdy * cdy;
    let determinant =
        alift * (bdxcdy - cdxbdy) + blift * (cdxady - adxcdy) + clift * (adxbdy - bdxady);
    let permanent = (bdxcdy.abs() + cdxbdy.abs()) * alift
        + (cdxady.abs() + adxcdy.abs()) * blift
        + (adxbdy.abs() + bdxady.abs()) * clift;
    let error_bound = 1.110_223_024_625_156_5e-15 * permanent;
    if determinant.abs() > error_bound {
        return if determinant > 0.0 {
            PredicateSign::Positive
        } else {
            PredicateSign::Negative
        };
    }
    let points = [a, b, c, d];
    let lifts: [Vec<f64>; 4] =
        points.map(|point| expansion_sum(&product(point.x, point.x), &product(point.y, point.y)));
    let mut determinant = vec![];

    // Expand the 4x4 determinant [x, y, x^2+y^2, 1]. The final column is one,
    // so every term is an exact product of x, y, and one exact lift expansion.
    for x_row in 0..4 {
        for y_row in 0..4 {
            if y_row == x_row {
                continue;
            }
            for (lift_row, lift) in lifts.iter().enumerate() {
                if lift_row == x_row || lift_row == y_row {
                    continue;
                }
                let one_row = (0..4)
                    .find(|row| *row != x_row && *row != y_row && *row != lift_row)
                    .unwrap();
                let permutation = [x_row, y_row, lift_row, one_row];
                let xy = product(points[x_row].x, points[y_row].y);
                let mut term = expansion_product(&xy, lift);
                if !permutation_parity(permutation) {
                    term = expansion_negate(&term);
                }
                determinant = expansion_sum(&determinant, &term);
            }
        }
    }
    sign_of(&determinant)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_orientation_resolves_roundoff_scale() {
        let a = Point2::new(0.0, 0.0);
        let b = Point2::new(1.0, 1.0e-20);
        let c = Point2::new(2.0, 2.0e-20 + 1.0e-35);
        assert_eq!(orient2d(a, b, c), PredicateSign::Positive);
        assert_eq!(orient2d(a, c, b), PredicateSign::Negative);
        assert_eq!(orient2d(a, b, b), PredicateSign::Zero);
    }

    #[test]
    fn exact_incircle_handles_cocircular_and_nearby_points() {
        let a = Point2::new(0.0, 0.0);
        let b = Point2::new(1.0, 0.0);
        let c = Point2::new(0.0, 1.0);
        assert_eq!(
            incircle(a, b, c, Point2::new(1.0, 1.0)),
            PredicateSign::Zero
        );
        assert_eq!(
            incircle(a, b, c, Point2::new(1.0, 1.0 - f64::EPSILON)),
            PredicateSign::Positive
        );
        assert_eq!(
            incircle(a, b, c, Point2::new(1.0, 1.0 + f64::EPSILON)),
            PredicateSign::Negative
        );
        assert_eq!(
            incircle(a, c, b, Point2::new(1.0, 1.0 - f64::EPSILON)),
            PredicateSign::Negative
        );
    }

    #[test]
    fn predicates_agree_with_integer_arithmetic() {
        let mut state = 0x1234_5678_9abc_def0_u64;
        let mut coordinate = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            ((state >> 32) as i32 % 2_001 - 1_000) as i64
        };
        for _ in 0..5_000 {
            let values: [(i64, i64); 4] = std::array::from_fn(|_| (coordinate(), coordinate()));
            let points = values.map(|(x, y)| Point2::new(x as f64, y as f64));
            let [(ax, ay), (bx, by), (cx, cy), (dx, dy)] = values;
            let orientation =
                (ax - cx) as i128 * (by - cy) as i128 - (ay - cy) as i128 * (bx - cx) as i128;
            assert_eq!(
                orient2d(points[0], points[1], points[2]),
                int_sign(orientation)
            );

            let adx = (ax - dx) as i128;
            let ady = (ay - dy) as i128;
            let bdx = (bx - dx) as i128;
            let bdy = (by - dy) as i128;
            let cdx = (cx - dx) as i128;
            let cdy = (cy - dy) as i128;
            let determinant = (adx * adx + ady * ady) * (bdx * cdy - bdy * cdx)
                + (bdx * bdx + bdy * bdy) * (cdx * ady - cdy * adx)
                + (cdx * cdx + cdy * cdy) * (adx * bdy - ady * bdx);
            assert_eq!(
                incircle(points[0], points[1], points[2], points[3]),
                int_sign(determinant)
            );
        }
    }

    fn int_sign(value: i128) -> PredicateSign {
        match value {
            value if value > 0 => PredicateSign::Positive,
            value if value < 0 => PredicateSign::Negative,
            _ => PredicateSign::Zero,
        }
    }
}
