//! The outgoing trace's eigensystem from its sparsity.
//!
//! The second-order wall's tangential operator couples each trace node to
//! the nodes of the one or two boundary edges it sits on: at most five
//! nonzeros a row, in pieces that are the walls, a cycle where four outgoing
//! sides meet at the corners. The dense solve it replaced ([`SymmetricEigenJob`]
//! in `canonical_wave`, kept under test as the oracle) cost the cube of the
//! trace in its eigenvector work alone, 1.0 s at 1,062 nodes, of which the
//! Householder accumulation took 42% and the QL rotations 33%.
//!
//! This one never forms a dense matrix of the operator. Each connected
//! component is ordered by reverse Cuthill-McKee, which gives the cycle a
//! bandwidth of 4, and solved on its own:
//!
//! 1. The band is reduced to a tridiagonal by Givens rotations that chase
//!    each bulge off the end, Schwarz's algorithm, without accumulating the
//!    rotations: quadratic in the component, linear in the band.
//! 2. The tridiagonal's eigenvalues come from implicit QL on the scalars
//!    alone, EISPACK's `tql1`, under the same whole-solve budget of 30·n
//!    iterations as before.
//! 3. Each eigenvector comes from inverse iteration on the banded operator
//!    itself, LAPACK's `dstein` rules on a band instead of a tridiagonal: a
//!    banded LU with partial pivoting at the eigenvalue, shifts of close
//!    eigenvalues kept ten units apart, and explicit orthogonalization within
//!    a group of eigenvalues closer than a thousandth of the component's norm.
//!    Doing it on the band rather than on the tridiagonal is what keeps the
//!    whole solve quadratic: there is no back-transformation.
//!
//! The arithmetic is `sqrt` and the four operations, the start vectors come
//! from an integer generator, and every tie is broken by index, so the basis
//! is the same on every platform. Eigenpairs come out ascending over the
//! whole trace, the vectors column-major in trace order, which is what the
//! outgoing boundary job reads.

use crate::WaveError;

/// Inner iterations one work unit may spend. A unit is a run of rotations,
/// one QL iteration, or one eigenvector, so a unit's time grows with the
/// trace at most linearly, never with its square.
pub(crate) const EIGEN_UNIT_WORK: usize = 1 << 13;

/// Implicit QL iterations the whole solve may take, per eigenvalue: LAPACK's
/// `dsteqr` budget of 30·n, under which an eigenvalue that stalls borrows from
/// the others, where EISPACK's `tql2` refuses the solve at 30 on any one. The
/// trace operators here take under two on average and five at most.
pub(crate) const EIGEN_ITERATIONS_PER_EIGENVALUE: usize = 30;

/// Why a refused solve was refused, and the way out. The solve is
/// deterministic, so the same trace fails every time it is prepared: the
/// document holds the change, the running generation keeps running, and the
/// trace is only prepared again when something along the boundary changes it.
pub(crate) const EIGENSOLVE_DID_NOT_CONVERGE: &str = "the outgoing trace eigensolve did not converge; the document keeps the change, but the solver runs without it until a new mesh edge, other walls or an edit along them change the outgoing boundary";

/// Inverse-iteration passes one eigenvector may take, `dstein`'s `MAXITS`.
const INVERSE_ITERATIONS: usize = 5;

/// A vector is orthogonalized explicitly against every earlier vector whose
/// eigenvalue lies within this share of the component's norm below its own,
/// `dstein`'s `ODM3`. Further apart, the gap alone keeps two vectors
/// orthogonal to about the unit roundoff over this share. `dstein` chains
/// such neighbours into groups and orthogonalizes against the whole group;
/// on a trace spectrum, whose mean gap is about this share, the chains ran
/// through most of the spectrum and made the vectors cubic again, 389 ms of
/// 453 at 1,062 nodes. A window holds a few vectors in the bulk of the
/// spectrum and a few dozen at its dense low end, and gives the same bound.
const GROUP_SHARE: f64 = 1.0e-3;

/// Consecutive shifts are kept at least this many units of roundoff of the
/// eigenvalue apart, `dstein`'s `PERTOL`, so a repeated eigenvalue gets a
/// factorization of its own and the orthogonalization picks out its partner.
const SHIFT_SEPARATION: f64 = 10.0;

/// A vector is accepted when `|A v - σ v|` is within this share of the
/// component's norm, after the second pass at the earliest: the first pass
/// from a random start still carries the start's share of every other
/// eigenvector, scaled by the eigenvalue's roundoff over the gap, 2e-13 of
/// the norm on a ring of 160, and the second pass takes that out, which is
/// `dstein`'s extra iteration. The solve then reaches a few units of
/// roundoff, 3e-15 on a trace of 144 nodes; the bound is far above that and
/// far below what single precision can tell, so a vector refused here is not
/// an eigenvector.
const RESIDUAL_ACCEPTANCE: f64 = 1.0e-9;

/// `sqrt(a^2 + b^2)` without overflow, from `sqrt` alone.
pub(crate) fn portable_hypot(a: f64, b: f64) -> f64 {
    let (a, b) = (a.abs(), b.abs());
    let (large, small) = if a > b { (a, b) } else { (b, a) };
    if large == 0.0 {
        return 0.0;
    }
    let ratio = small / large;
    large * (1.0 + ratio * ratio).sqrt()
}

/// The eigenvalues ascending and the vectors column-major,
/// `vectors[position * count + trace_row]`.
pub(crate) type TraceEigenResult = (Vec<f64>, Vec<f64>);

/// A symmetric band, both triangles stored: `data[row * (2w + 1) + (column -
/// row + w)]` for `|column - row| <= w`. Entries outside read as zero.
#[derive(Clone)]
struct SymmetricBand {
    size: usize,
    half_width: usize,
    data: Vec<f64>,
}

impl SymmetricBand {
    fn new(size: usize, half_width: usize) -> Self {
        Self {
            size,
            half_width,
            data: vec![0.0; size * (2 * half_width + 1)],
        }
    }

    fn stride(&self) -> usize {
        2 * self.half_width + 1
    }

    fn index(&self, row: usize, column: usize) -> Option<usize> {
        if row >= self.size || column >= self.size {
            return None;
        }
        let offset = column as isize - row as isize + self.half_width as isize;
        (offset >= 0 && offset < self.stride() as isize)
            .then(|| row * self.stride() + offset as usize)
    }

    fn get(&self, row: usize, column: usize) -> f64 {
        self.index(row, column)
            .map_or(0.0, |index| self.data[index])
    }

    fn set(&mut self, row: usize, column: usize, value: f64) {
        match self.index(row, column) {
            Some(index) => self.data[index] = value,
            None => debug_assert_eq!(value, 0.0, "a band entry outside its storage"),
        }
    }

    /// Stores an operator entry. A zero outside the storage is nothing to
    /// store; a nonzero there is refused, in every build: the ordering bounds
    /// the band from the same entries, so one it does not hold is a defect
    /// in the numbering, and a band missing an entry solves some other
    /// operator with no residual to show for it.
    fn place(&mut self, row: usize, column: usize, value: f64) -> Result<(), WaveError> {
        match self.index(row, column) {
            Some(index) => {
                self.data[index] = value;
                Ok(())
            }
            None if value == 0.0 => Ok(()),
            None => Err(WaveError::InvalidMesh(
                "the outgoing trace ordering left an entry outside its band",
            )),
        }
    }

    /// `A x`.
    fn apply(&self, x: &[f64], y: &mut [f64]) {
        let w = self.half_width;
        for (row, out) in y.iter_mut().enumerate() {
            let first = row.saturating_sub(w);
            let last = (row + w).min(self.size - 1);
            let base = row * self.stride() + w - (row - first);
            *out = self.data[base..base + last - first + 1]
                .iter()
                .zip(&x[first..=last])
                .map(|(a, b)| a * b)
                .sum();
        }
    }

    /// The largest row sum of magnitudes.
    fn one_norm(&self) -> f64 {
        self.data
            .chunks(self.stride())
            .map(|row| row.iter().map(|value| value.abs()).sum::<f64>())
            .fold(0.0, f64::max)
    }

    /// The similarity rotation in the plane `(p, q)`, `q = p + 1`: rows `p`
    /// and `q` then columns `p` and `q`, which keeps both triangles equal.
    /// Entry `(p, k)` sits one place after `(q, k)` in the stride, and the
    /// two rows' storage overlaps except at its ends, where the entry
    /// outside is zero and stays so.
    fn rotate(&mut self, p: usize, q: usize, c: f64, s: f64) {
        let w = self.half_width;
        let stride = self.stride();
        let size = self.size;
        // Rows p and q: columns k with both in storage are p - w + 1 ..= p + w.
        let first = (p + 1).saturating_sub(w);
        let last = (p + w).min(size - 1);
        if p >= w {
            // Column p - w: row p only.
            let index = p * stride;
            self.data[index] *= c;
        }
        for k in first..=last {
            let at_p = p * stride + k + w - p;
            let at_q = at_p + stride - 1;
            let a = self.data[at_p];
            let b = self.data[at_q];
            self.data[at_p] = c * a + s * b;
            self.data[at_q] = c * b - s * a;
        }
        if q + w < size {
            // Column q + w: row q only.
            let index = q * stride + 2 * w;
            self.data[index] *= c;
        }
        // Columns p and q: rows k over the same range; (k, p) sits one place
        // before (k, q).
        if p >= w {
            let index = (p - w) * stride + 2 * w;
            self.data[index] *= c;
        }
        for k in first..=last {
            let at_p = k * stride + p + w - k;
            let at_q = at_p + 1;
            let a = self.data[at_p];
            let b = self.data[at_q];
            self.data[at_p] = c * a + s * b;
            self.data[at_q] = c * b - s * a;
        }
        if q + w < size {
            let index = (q + w) * stride;
            self.data[index] *= c;
        }
    }
}

/// `P (A - σ I) = L U` on a band of `b` lower and `b` upper diagonals, in
/// LAPACK's layout: column `j` holds rows `j - 2b ..= j + b` contiguously,
/// the extra `b` upper diagonals being the fill the row interchanges make.
/// `U`'s diagonal is stored reciprocal. The storage is the caller's, reused
/// from one shift to the next.
struct BandLu<'a> {
    size: usize,
    bandwidth: usize,
    data: &'a mut [f64],
    pivots: &'a mut [usize],
}

impl<'a> BandLu<'a> {
    fn width(bandwidth: usize) -> usize {
        3 * bandwidth + 1
    }

    /// Where column `column`'s entry for row `column - 2b` sits; row `r` of
    /// that column is `base + r - (column - 2b)`.
    fn base(&self, column: usize) -> usize {
        column * Self::width(self.bandwidth)
    }

    fn factor(
        band: &SymmetricBand,
        bandwidth: usize,
        shift: f64,
        tiny: f64,
        data: &'a mut Vec<f64>,
        pivots: &'a mut Vec<usize>,
    ) -> Self {
        let size = band.size;
        let width = Self::width(bandwidth);
        data.clear();
        data.resize(size * width, 0.0);
        pivots.clear();
        pivots.resize(size, 0);
        let lu = Self {
            size,
            bandwidth,
            data,
            pivots,
        };
        // The band's rows into the columns of the factor's layout.
        let b = bandwidth;
        let stride = band.stride();
        for row in 0..size {
            let first = row.saturating_sub(b);
            let last = (row + b).min(size - 1);
            let source = row * stride + band.half_width - (row - first);
            for (offset, column) in (first..=last).enumerate() {
                let index = lu.base(column) + row + 2 * b - column;
                lu.data[index] = band.data[source + offset];
            }
            let index = lu.base(row) + 2 * b;
            lu.data[index] -= shift;
        }
        let reach = 2 * b;
        for j in 0..size {
            let below = b.min(size - 1 - j);
            let diagonal_index = lu.base(j) + 2 * b;
            // The pivot among the diagonal and the `below` entries under it.
            let mut pivot = 0;
            let mut best = lu.data[diagonal_index].abs();
            for t in 1..=below {
                let candidate = lu.data[diagonal_index + t].abs();
                if candidate > best {
                    best = candidate;
                    pivot = t;
                }
            }
            lu.pivots[j] = j + pivot;
            let columns_end = (j + reach).min(size - 1);
            if pivot != 0 {
                for column in j..=columns_end {
                    let at_j = lu.base(column) + j + 2 * b - column;
                    lu.data.swap(at_j, at_j + pivot);
                }
            }
            if lu.data[diagonal_index].abs() < tiny {
                lu.data[diagonal_index] = tiny;
            }
            let diagonal = lu.data[diagonal_index];
            for t in 1..=below {
                lu.data[diagonal_index + t] /= diagonal;
            }
            // The back substitution multiplies by the reciprocal, which does
            // not wait on a division at every row.
            lu.data[diagonal_index] = 1.0 / diagonal;
            if below == 0 {
                continue;
            }
            let (before, after) = lu.data.split_at_mut(lu.base(j + 1));
            let multipliers = &before[diagonal_index + 1..=diagonal_index + below];
            for column in j + 1..=columns_end {
                let at_j = column * width + j + 2 * b - column - before.len();
                let upper = after[at_j];
                if upper == 0.0 {
                    continue;
                }
                for (t, multiplier) in multipliers.iter().enumerate() {
                    after[at_j + 1 + t] -= multiplier * upper;
                }
            }
        }
        lu
    }

    /// Solves in place.
    fn solve(&self, x: &mut [f64]) {
        let size = self.size;
        let b = self.bandwidth;
        for j in 0..size {
            let pivot = self.pivots[j];
            if pivot != j {
                x.swap(j, pivot);
            }
            let below = b.min(size - 1 - j);
            let value = x[j];
            if value != 0.0 {
                let diagonal_index = self.base(j) + 2 * b;
                for t in 1..=below {
                    x[j + t] -= self.data[diagonal_index + t] * value;
                }
            }
        }
        for j in (0..size).rev() {
            let base = self.base(j);
            let diagonal_index = base + 2 * b;
            x[j] *= self.data[diagonal_index];
            let value = x[j];
            if value != 0.0 {
                let first = j.saturating_sub(2 * b);
                let column = &self.data[base + first + 2 * b - j..base + 2 * b];
                for (xi, entry) in x[first..j].iter_mut().zip(column) {
                    *xi -= entry * value;
                }
            }
        }
    }
}

/// One connected piece of the trace, in its reverse Cuthill-McKee order.
struct Component {
    /// Local position to trace row.
    nodes: Vec<usize>,
    /// The largest `|row - column|` of a nonzero.
    bandwidth: usize,
    /// The operator, kept for the vectors.
    band: SymmetricBand,
    /// The reduction's working copy, a half-width wider for the bulge.
    work: SymmetricBand,
    one_norm: f64,
    /// The tridiagonal's diagonal, then the eigenvalues as QL settles them.
    diagonal: Vec<f64>,
    /// The tridiagonal's subdiagonal, `off_diagonal[k]` between `k` and `k + 1`.
    off_diagonal: Vec<f64>,
    /// QL's accumulated shift `f` and its running negligibility scale `tst1`.
    shift: f64,
    negligible: f64,
    /// The eigenvalues ascending.
    values: Vec<f64>,
    /// The vectors computed so far whose eigenvalues may still lie within
    /// the window of the next, oldest first, each with its eigenvalue, in
    /// the component's own order.
    recent: std::collections::VecDeque<(f64, Vec<f64>)>,
    /// The shift the previous vector was computed at.
    previous_shift: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Order,
    /// Zero `(row, column)` of `component`'s working band, the `element`th
    /// entry of the diagonal at distance `level`, or its bulge further down.
    Reduce {
        component: usize,
        level: usize,
        element: usize,
        row: usize,
        column: usize,
    },
    /// Settle `eigenvalue` of `component`, one QL iteration at a time.
    Values {
        component: usize,
        eigenvalue: usize,
        end: Option<usize>,
    },
    Rank,
    Vectors {
        position: usize,
    },
    Done,
}

/// The cooperative solve: `new` with the operator's nonzeros, then `advance`
/// until it answers.
pub(crate) struct BandedEigenJob {
    count: usize,
    entries: Vec<(u32, u32, f64)>,
    components: Vec<Component>,
    phase: Phase,
    /// QL iterations spent so far, over the whole solve.
    iterations: usize,
    /// Inverse-iteration passes spent so far, over every vector.
    vector_passes: usize,
    /// Global position to `(component, rank)`.
    order: Vec<(usize, usize)>,
    values: Vec<f64>,
    vectors: Vec<f64>,
    /// The banded factor's storage, reused from one vector to the next.
    factor: Vec<f64>,
    pivots: Vec<usize>,
}

impl BandedEigenJob {
    /// `entries` are `(row, column, value)` over `count` trace rows, both
    /// triangles present. Entries that disagree with their transpose beyond
    /// roundoff of the matrix's magnitude are refused, as the dense solve
    /// refused them.
    pub(crate) fn new(entries: Vec<(u32, u32, f64)>, count: usize) -> Result<Self, WaveError> {
        if entries.iter().any(|(row, column, value)| {
            *row as usize >= count || *column as usize >= count || !value.is_finite()
        }) {
            return Err(WaveError::InvalidMesh(
                "the outgoing trace matrix is invalid",
            ));
        }
        let scale = entries
            .iter()
            .map(|(_, _, value)| value.abs())
            .sum::<f64>()
            .max(1.0);
        let mut sorted = entries
            .iter()
            .map(|&(row, column, value)| ((row, column), value))
            .collect::<Vec<_>>();
        sorted.sort_by_key(|entry| entry.0);
        for (index, &((row, column), value)) in sorted.iter().enumerate() {
            if index > 0 && sorted[index - 1].0 == (row, column) {
                return Err(WaveError::InvalidMesh(
                    "the outgoing trace matrix repeats an entry",
                ));
            }
            let mirror = sorted
                .binary_search_by(|probe| probe.0.cmp(&(column, row)))
                .map(|found| sorted[found].1)
                .unwrap_or(0.0);
            if (value - mirror).abs() > 2.0e-11 * scale {
                return Err(WaveError::InvalidMesh(
                    "the outgoing normalized trace operator is not symmetric",
                ));
            }
        }
        Ok(Self {
            count,
            entries,
            components: Vec::new(),
            phase: if count == 0 {
                Phase::Done
            } else {
                Phase::Order
            },
            iterations: 0,
            vector_passes: 0,
            order: Vec::new(),
            values: Vec::new(),
            vectors: Vec::new(),
            factor: Vec::new(),
            pivots: Vec::new(),
        })
    }

    pub(crate) fn advance(&mut self, budget: usize) -> Option<Result<TraceEigenResult, WaveError>> {
        for _ in 0..budget {
            if self.phase == Phase::Done {
                return Some(self.finish());
            }
            if let Err(error) = self.unit() {
                self.phase = Phase::Done;
                return Some(Err(error));
            }
        }
        None
    }

    /// QL iterations spent so far, over the whole solve.
    #[cfg(test)]
    pub(crate) fn iterations(&self) -> usize {
        self.iterations
    }

    /// Inverse-iteration passes spent so far, over every vector.
    #[cfg(test)]
    pub(crate) fn vector_passes(&self) -> usize {
        self.vector_passes
    }

    fn finish(&mut self) -> Result<TraceEigenResult, WaveError> {
        let values = std::mem::take(&mut self.values);
        let vectors = std::mem::take(&mut self.vectors);
        if values.len() != self.count || vectors.len() != self.count * self.count {
            return Err(WaveError::InvalidMesh(EIGENSOLVE_DID_NOT_CONVERGE));
        }
        Ok((values, vectors))
    }

    /// One bounded unit of whichever phase is current.
    fn unit(&mut self) -> Result<(), WaveError> {
        match self.phase {
            Phase::Order => {
                self.order_components()?;
                self.phase = self.after_reduction(0);
            }
            Phase::Reduce {
                component,
                level,
                element,
                row,
                column,
            } => {
                self.phase = self.reduce(component, level, element, row, column);
            }
            Phase::Values {
                component,
                eigenvalue,
                end,
            } => {
                self.phase = self.settle_value(component, eigenvalue, end)?;
            }
            Phase::Rank => {
                self.rank();
                self.phase = if self.count == 0 {
                    Phase::Done
                } else {
                    Phase::Vectors { position: 0 }
                };
            }
            Phase::Vectors { position } => {
                self.vector(position)?;
                self.phase = if position + 1 < self.count {
                    Phase::Vectors {
                        position: position + 1,
                    }
                } else {
                    Phase::Done
                };
            }
            Phase::Done => {}
        }
        Ok(())
    }

    /// Splits the trace into its connected pieces and orders each by reverse
    /// Cuthill-McKee from a node of least degree, neighbours by degree then
    /// index, so the order is a function of the graph alone. The pieces come
    /// from the nonzero entries; an entry between two pieces is a zero the
    /// mesh pattern holds between trace nodes no wall element couples, as a
    /// wall's end beside another wall has, and it is not stored: its
    /// column's position belongs to the other piece's numbering.
    fn order_components(&mut self) -> Result<(), WaveError> {
        let count = self.count;
        let mut adjacency = vec![Vec::new(); count];
        for &(row, column, value) in &self.entries {
            if row != column && value != 0.0 {
                adjacency[row as usize].push(column as usize);
                adjacency[column as usize].push(row as usize);
            }
        }
        for list in &mut adjacency {
            list.sort_unstable();
            list.dedup();
        }
        let degree = |node: usize| adjacency[node].len();
        let mut seen = vec![false; count];
        let mut visited = vec![false; count];
        let mut position = vec![usize::MAX; count];
        for start in 0..count {
            if seen[start] {
                continue;
            }
            // The piece, by a plain search; then its least-degree node.
            let mut members = vec![start];
            seen[start] = true;
            let mut next = 0;
            while next < members.len() {
                let node = members[next];
                next += 1;
                for &neighbour in &adjacency[node] {
                    if !seen[neighbour] {
                        seen[neighbour] = true;
                        members.push(neighbour);
                    }
                }
            }
            let root = members
                .iter()
                .copied()
                .min_by_key(|&node| (degree(node), node))
                .expect("a piece has a member");
            let mut nodes = vec![root];
            visited[root] = true;
            let mut next = 0;
            while next < nodes.len() {
                let node = nodes[next];
                next += 1;
                let mut neighbours = adjacency[node]
                    .iter()
                    .copied()
                    .filter(|&neighbour| !visited[neighbour])
                    .collect::<Vec<_>>();
                neighbours.sort_by_key(|&neighbour| (degree(neighbour), neighbour));
                for neighbour in neighbours {
                    visited[neighbour] = true;
                    nodes.push(neighbour);
                }
            }
            nodes.reverse();
            for (index, &node) in nodes.iter().enumerate() {
                position[node] = index;
            }
            let bandwidth = nodes
                .iter()
                .map(|&node| {
                    adjacency[node]
                        .iter()
                        .map(|&neighbour| position[node].abs_diff(position[neighbour]))
                        .max()
                        .unwrap_or(0)
                })
                .max()
                .unwrap_or(0);
            let size = nodes.len();
            let band = SymmetricBand::new(size, bandwidth);
            let work = SymmetricBand::new(size, bandwidth + 1);
            self.components.push(Component {
                nodes,
                bandwidth,
                band,
                work,
                one_norm: 0.0,
                diagonal: vec![0.0; size],
                off_diagonal: vec![0.0; size],
                shift: 0.0,
                negligible: 0.0,
                values: Vec::new(),
                recent: std::collections::VecDeque::new(),
                previous_shift: 0.0,
            });
        }
        let mut owner = vec![usize::MAX; count];
        for (index, component) in self.components.iter().enumerate() {
            for &node in &component.nodes {
                owner[node] = index;
            }
        }
        for &(row, column, value) in &self.entries {
            if owner[row as usize] != owner[column as usize] {
                debug_assert_eq!(value, 0.0, "a nonzero entry between pieces");
                continue;
            }
            let component = &mut self.components[owner[row as usize]];
            let (r, c) = (position[row as usize], position[column as usize]);
            component.band.place(r, c, value)?;
            component.work.place(r, c, value)?;
        }
        for component in &mut self.components {
            component.one_norm = component.band.one_norm();
        }
        Ok(())
    }

    /// The phase after `component`'s reduction, or the values phase.
    fn after_reduction(&mut self, component: usize) -> Phase {
        for (index, piece) in self.components.iter().enumerate().skip(component) {
            if piece.bandwidth >= 2 {
                return Phase::Reduce {
                    component: index,
                    level: piece.bandwidth,
                    element: 0,
                    row: 0,
                    column: piece.bandwidth,
                };
            }
        }
        self.start_values(0)
    }

    /// Schwarz's reduction: rotations in the plane `(column - 1, column)`
    /// zero `(row, column)`, each one pushing a bulge `level` rows down, until
    /// the bulge leaves the matrix. A unit does a bounded run of rotations
    /// and may stop inside a chase.
    fn reduce(
        &mut self,
        component: usize,
        mut level: usize,
        mut element: usize,
        mut row: usize,
        mut column: usize,
    ) -> Phase {
        let piece = &mut self.components[component];
        let size = piece.work.size;
        let cost = 4 * piece.work.stride();
        let mut work = 0;
        while work < EIGEN_UNIT_WORK {
            let (x, y) = (piece.work.get(row, column - 1), piece.work.get(row, column));
            if y != 0.0 {
                let r = portable_hypot(x, y);
                let (c, s) = (x / r, y / r);
                piece.work.rotate(column - 1, column, c, s);
                piece.work.set(row, column, 0.0);
                piece.work.set(column, row, 0.0);
                work += cost;
                if column + level < size {
                    row = column - 1;
                    column += level;
                    continue;
                }
            }
            element += 1;
            if element + level < size {
                row = element;
                column = element + level;
                continue;
            }
            level -= 1;
            if level >= 2 {
                element = 0;
                row = 0;
                column = level;
                continue;
            }
            for k in 0..size {
                piece.diagonal[k] = piece.work.get(k, k);
                piece.off_diagonal[k] = if k + 1 < size {
                    piece.work.get(k, k + 1)
                } else {
                    0.0
                };
            }
            piece.work.data = Vec::new();
            return self.after_reduction(component + 1);
        }
        Phase::Reduce {
            component,
            level,
            element,
            row,
            column,
        }
    }

    /// The values phase of `component`, whose tridiagonal is ready; pieces
    /// of bandwidth under 2 take theirs straight from the band.
    fn start_values(&mut self, component: usize) -> Phase {
        if component >= self.components.len() {
            return Phase::Rank;
        }
        let piece = &mut self.components[component];
        if piece.bandwidth < 2 {
            let size = piece.band.size;
            for k in 0..size {
                piece.diagonal[k] = piece.band.get(k, k);
                piece.off_diagonal[k] = if k + 1 < size {
                    piece.band.get(k, k + 1)
                } else {
                    0.0
                };
            }
        }
        Phase::Values {
            component,
            eigenvalue: 0,
            end: None,
        }
    }

    /// Implicit QL on the scalars, as EISPACK's `tql1`: eigenvalue `l` is
    /// settled once the subdiagonal next to it is negligible, `end` bounding
    /// the unreduced block it sits in. One iteration a unit.
    fn settle_value(
        &mut self,
        component: usize,
        l: usize,
        end: Option<usize>,
    ) -> Result<Phase, WaveError> {
        let piece = &mut self.components[component];
        let n = piece.diagonal.len();
        if l == n {
            let mut ranks = (0..n).collect::<Vec<_>>();
            ranks.sort_by(|&a, &b| {
                piece.diagonal[a]
                    .total_cmp(&piece.diagonal[b])
                    .then(a.cmp(&b))
            });
            piece.values = ranks.iter().map(|&k| piece.diagonal[k]).collect();
            return Ok(self.start_values(component + 1));
        }
        let m = match end {
            Some(m) => m,
            None => {
                piece.negligible = piece
                    .negligible
                    .max(piece.diagonal[l].abs() + piece.off_diagonal[l].abs());
                let threshold = f64::EPSILON * piece.negligible;
                (l..n)
                    .find(|&m| piece.off_diagonal[m].abs() <= threshold)
                    .unwrap_or(n - 1)
            }
        };
        if m == l {
            piece.diagonal[l] += piece.shift;
            piece.off_diagonal[l] = 0.0;
            return Ok(Phase::Values {
                component,
                eigenvalue: l + 1,
                end: None,
            });
        }
        self.iterations += 1;
        if self.iterations > EIGEN_ITERATIONS_PER_EIGENVALUE * self.count {
            return Err(WaveError::InvalidMesh(EIGENSOLVE_DID_NOT_CONVERGE));
        }
        let d = &mut piece.diagonal;
        let e = &mut piece.off_diagonal;
        let mut g = d[l];
        let mut p = (d[l + 1] - g) / (2.0 * e[l]);
        let mut r = portable_hypot(p, 1.0);
        if p < 0.0 {
            r = -r;
        }
        d[l] = e[l] / (p + r);
        d[l + 1] = e[l] * (p + r);
        let dl1 = d[l + 1];
        let mut h = g - d[l];
        for value in &mut d[l + 2..] {
            *value -= h;
        }
        piece.shift += h;
        p = d[m];
        let mut c = 1.0;
        let mut c2 = c;
        let mut c3 = c;
        let el1 = e[l + 1];
        let mut s = 0.0;
        let mut s2 = 0.0;
        for i in (l..m).rev() {
            c3 = c2;
            c2 = c;
            s2 = s;
            g = c * e[i];
            h = c * p;
            r = portable_hypot(p, e[i]);
            e[i + 1] = s * r;
            s = e[i] / r;
            c = p / r;
            p = c * d[i] - s * g;
            d[i + 1] = h + s * (c * g + s * d[i]);
        }
        p = -s * s2 * c3 * el1 * e[l] / dl1;
        e[l] = s * p;
        d[l] = c * p;
        if e[l].abs() > f64::EPSILON * piece.negligible {
            Ok(Phase::Values {
                component,
                eigenvalue: l,
                end: Some(m),
            })
        } else {
            d[l] += piece.shift;
            e[l] = 0.0;
            Ok(Phase::Values {
                component,
                eigenvalue: l + 1,
                end: None,
            })
        }
    }

    /// The global order: ascending over every component, ties by component
    /// then rank.
    fn rank(&mut self) {
        let mut order = Vec::with_capacity(self.count);
        for (index, component) in self.components.iter().enumerate() {
            order.extend((0..component.values.len()).map(|rank| (index, rank)));
        }
        order.sort_by(|&(ca, ra), &(cb, rb)| {
            self.components[ca].values[ra]
                .total_cmp(&self.components[cb].values[rb])
                .then((ca, ra).cmp(&(cb, rb)))
        });
        self.values = order
            .iter()
            .map(|&(component, rank)| self.components[component].values[rank])
            .collect();
        self.order = order;
        self.vectors = vec![0.0; self.count * self.count];
    }

    /// The eigenvector at global `position` by inverse iteration on its
    /// component's band.
    fn vector(&mut self, position: usize) -> Result<(), WaveError> {
        let (index, rank) = self.order[position];
        let count = self.count;
        let piece = &mut self.components[index];
        let size = piece.band.size;
        let value = piece.values[rank];
        let tiny = (f64::EPSILON * piece.one_norm).max(f64::MIN_POSITIVE);
        // The shift, kept apart from the previous one as dstein keeps it.
        let mut shift = value;
        if rank > 0 {
            let separation = SHIFT_SEPARATION * f64::EPSILON * value.abs();
            if shift - piece.previous_shift < separation {
                shift = piece.previous_shift + separation;
            }
        }
        piece.previous_shift = shift;
        let lu = BandLu::factor(
            &piece.band,
            piece.bandwidth,
            shift,
            tiny,
            &mut self.factor,
            &mut self.pivots,
        );
        let mut x = start_vector(position, size);
        let mut y = vec![0.0; size];
        // The earlier vectors still within the window.
        while piece
            .recent
            .front()
            .is_some_and(|(earlier, _)| value - earlier >= GROUP_SHARE * piece.one_norm)
        {
            piece.recent.pop_front();
        }
        let acceptance = RESIDUAL_ACCEPTANCE * piece.one_norm;
        let mut accepted = false;
        for pass in 0..INVERSE_ITERATIONS {
            self.vector_passes += 1;
            lu.solve(&mut x);
            for (_, z) in &piece.recent {
                let projection = dot(&x, z);
                for (xi, zi) in x.iter_mut().zip(z) {
                    *xi -= projection * zi;
                }
            }
            // The solve at a near-singular shift grows the vector by about
            // the reciprocal of a tiny pivot; scaling by the largest entry
            // first keeps the norm from overflowing.
            let largest = x.iter().fold(0.0_f64, |largest, xi| largest.max(xi.abs()));
            if !(largest.is_finite() && largest > 0.0) {
                return Err(WaveError::InvalidMesh(EIGENSOLVE_DID_NOT_CONVERGE));
            }
            for xi in &mut x {
                *xi /= largest;
            }
            let norm = dot(&x, &x).sqrt();
            for xi in &mut x {
                *xi /= norm;
            }
            if pass == 0 {
                continue;
            }
            piece.band.apply(&x, &mut y);
            let residual = x
                .iter()
                .zip(&y)
                .map(|(xi, yi)| (yi - shift * xi).powi(2))
                .sum::<f64>()
                .sqrt();
            if residual <= acceptance {
                accepted = true;
                break;
            }
        }
        if !accepted {
            return Err(WaveError::InvalidMesh(EIGENSOLVE_DID_NOT_CONVERGE));
        }
        // The largest entry positive, so the basis does not depend on the
        // start vector's sign.
        let largest = x.iter().copied().fold(0.0_f64, |largest, xi| {
            if xi.abs() > largest.abs() {
                xi
            } else {
                largest
            }
        });
        if largest < 0.0 {
            for xi in &mut x {
                *xi = -*xi;
            }
        }
        for (local, &node) in piece.nodes.iter().enumerate() {
            self.vectors[position * count + node] = x[local];
        }
        piece.recent.push_back((value, x));
        Ok(())
    }
}

fn dot(left: &[f64], right: &[f64]) -> f64 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

/// A start vector of entries in `(-1, 1)` from an integer generator seeded by
/// the vector's position, the same on every platform.
fn start_vector(position: usize, size: usize) -> Vec<f64> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64 ^ ((position as u64 + 1) * 0xD1B5_4A32_D192_ED03);
    (0..size)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 52) as f64 - 1.0
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Runs the cooperative solve to completion, one unit at a time: the
    /// eigenpairs, the units the solve took, and the QL iterations it spent.
    pub(crate) fn eigensolve(matrix: &[f64], count: usize) -> (Vec<f64>, Vec<f64>, usize, usize) {
        let entries = (0..count)
            .flat_map(|row| (0..count).map(move |column| (row, column)))
            .filter(|&(row, column)| matrix[row * count + column] != 0.0)
            .map(|(row, column)| (row as u32, column as u32, matrix[row * count + column]))
            .collect();
        eigensolve_entries(entries, count)
    }

    /// The same solve from the operator's entries as the assembly hands them
    /// over, explicit zeros included.
    pub(crate) fn eigensolve_entries(
        entries: Vec<(u32, u32, f64)>,
        count: usize,
    ) -> (Vec<f64>, Vec<f64>, usize, usize) {
        let mut job = BandedEigenJob::new(entries, count).unwrap();
        let mut units = 1;
        loop {
            if let Some(result) = job.advance(1) {
                let (values, vectors) = result.unwrap();
                eprintln!(
                    "{count}: {units} units, {} QL iterations, {} inverse-iteration passes",
                    job.iterations(),
                    job.vector_passes()
                );
                return (values, vectors, units, job.iterations());
            }
            units += 1;
        }
    }

    /// The largest eigen-residual `|A v - lambda v|` over the matrix's largest
    /// row sum, and the largest departure of the vectors from orthonormality.
    pub(crate) fn eigen_errors(
        matrix: &[f64],
        count: usize,
        values: &[f64],
        vectors: &[f64],
    ) -> [f64; 2] {
        let norm = (0..count)
            .map(|row| {
                (0..count)
                    .map(|column| matrix[row * count + column].abs())
                    .sum::<f64>()
            })
            .fold(0.0, f64::max)
            .max(f64::MIN_POSITIVE);
        let column = |mode: usize| &vectors[mode * count..(mode + 1) * count];
        let mut residual = 0.0_f64;
        for (mode, value) in values.iter().enumerate() {
            for row in 0..count {
                let product = (0..count)
                    .map(|entry| matrix[row * count + entry] * column(mode)[entry])
                    .sum::<f64>();
                residual = residual.max((product - value * column(mode)[row]).abs());
            }
        }
        let mut orthogonality = 0.0_f64;
        for left in 0..count {
            for right in left..count {
                let dot = column(left)
                    .iter()
                    .zip(column(right))
                    .map(|(a, b)| a * b)
                    .sum::<f64>();
                let expected = if left == right { 1.0 } else { 0.0 };
                orthogonality = orthogonality.max((dot - expected).abs());
            }
        }
        [residual / norm, orthogonality]
    }

    /// `A` rebuilt from its eigenpairs, `sum lambda v v^T`, against itself,
    /// over its largest row sum.
    pub(crate) fn reconstruction_error(
        matrix: &[f64],
        count: usize,
        values: &[f64],
        vectors: &[f64],
    ) -> f64 {
        let norm = (0..count)
            .map(|row| {
                (0..count)
                    .map(|column| matrix[row * count + column].abs())
                    .sum::<f64>()
            })
            .fold(0.0, f64::max)
            .max(f64::MIN_POSITIVE);
        let mut worst = 0.0_f64;
        for row in 0..count {
            for column in 0..count {
                let rebuilt = (0..count)
                    .map(|mode| {
                        values[mode] * vectors[mode * count + row] * vectors[mode * count + column]
                    })
                    .sum::<f64>();
                worst = worst.max((rebuilt - matrix[row * count + column]).abs());
            }
        }
        worst / norm
    }

    pub(crate) fn ring(count: usize) -> Vec<f64> {
        let mut matrix = vec![0.0; count * count];
        for row in 0..count {
            let next = (row + 1) % count;
            matrix[row * count + row] = 2.0;
            matrix[row * count + next] = -1.0;
            matrix[next * count + row] = -1.0;
        }
        matrix
    }

    /// A closed trace is a ring: its tangential operator has one zero mode and
    /// every other eigenvalue twice, `2 - 2 cos(2 pi k / n)` for a uniform one.
    /// The solve finds them to roundoff, with orthonormal vectors, in more
    /// units than there are eigenvectors, and spends a tenth of its iteration
    /// budget, which is counted over the whole solve.
    #[test]
    fn the_trace_eigensolve_finds_a_rings_spectrum_in_bounded_units() {
        let count = 160;
        let matrix = ring(count);
        let (values, vectors, units, iterations) = eigensolve(&matrix, count);
        let mut expected = (0..count)
            .map(|k| 2.0 - 2.0 * (std::f64::consts::TAU * k as f64 / count as f64).cos())
            .collect::<Vec<_>>();
        expected.sort_by(f64::total_cmp);
        for (value, expected) in values.iter().zip(&expected) {
            assert!(
                (value - expected).abs() < 1.0e-13,
                "{value} against {expected}"
            );
        }
        assert!(values.windows(2).all(|pair| pair[0] <= pair[1]));
        let [residual, orthogonality] = eigen_errors(&matrix, count, &values, &vectors);
        let rebuilt = reconstruction_error(&matrix, count, &values, &vectors);
        assert!(residual < 1.0e-14, "{residual:e}");
        assert!(orthogonality < 1.0e-13, "{orthogonality:e}");
        assert!(rebuilt < 1.0e-13, "{rebuilt:e}");
        assert!(units > count, "{units}");
        assert!(
            iterations <= 3 * count,
            "{iterations} iterations for {count}"
        );
    }

    /// A trace in pieces, with a node coupled to nothing, is solved piece by
    /// piece and merged ascending. A general symmetric matrix, whose band is
    /// the whole matrix, a diagonal one out of order, and one and two nodes
    /// take the same path.
    #[test]
    fn the_trace_eigensolve_takes_disconnected_general_and_small_matrices() {
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut random = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        let count = 57;
        let mut pieces = vec![0.0; count * count];
        // A chain of 30, a ring of 26, and node 56 alone.
        for row in 0..29 {
            pieces[row * count + row] += 1.0;
            pieces[(row + 1) * count + row + 1] += 1.0;
            pieces[row * count + row + 1] -= 1.0;
            pieces[(row + 1) * count + row] -= 1.0;
        }
        for k in 0..26 {
            let row = 30 + k;
            let next = 30 + (k + 1) % 26;
            pieces[row * count + row] += 2.0;
            pieces[row * count + next] -= 1.0;
            pieces[next * count + row] -= 1.0;
        }
        let mut general = vec![0.0; count * count];
        for row in 0..count {
            for column in 0..=row {
                let value = random();
                general[row * count + column] = value;
                general[column * count + row] = value;
            }
        }
        let mut diagonal = vec![0.0; count * count];
        for row in 0..count {
            diagonal[row * count + row] = ((row * 7) % count) as f64;
        }
        let one = vec![3.0];
        let two = vec![2.0, -1.0, -1.0, 2.0];
        for (matrix, count) in [
            (pieces, count),
            (general, count),
            (diagonal, count),
            (one, 1),
            (two, 2),
        ] {
            let (values, vectors, _, _) = eigensolve(&matrix, count);
            assert!(values.windows(2).all(|pair| pair[0] <= pair[1]));
            let [residual, orthogonality] = eigen_errors(&matrix, count, &values, &vectors);
            let rebuilt = reconstruction_error(&matrix, count, &values, &vectors);
            assert!(residual < 1.0e-14, "{count}: {residual:e}");
            assert!(orthogonality < 1.0e-13, "{count}: {orthogonality:e}");
            assert!(rebuilt < 1.0e-13, "{count}: {rebuilt:e}");
        }
    }

    /// The trace's entries follow the mesh pattern, which holds a zero
    /// between trace nodes that share an element but no wall element, as a
    /// wall's end beside another wall does. Such an entry joins two pieces
    /// whose numberings have nothing to do with each other; written into a
    /// piece's band at the other piece's position it landed on whatever
    /// entry sat there, a diagonal of 1172 on the first scene that had one,
    /// which read as an eigenvalue of -443. Zeros between the pieces at
    /// every pair of positions leave the solve exactly as it is without
    /// them.
    #[test]
    fn zeros_between_pieces_leave_the_solve_unchanged() {
        let count = 23;
        let mut matrix = vec![0.0; count * count];
        // A chain of 10 coupled over one and two steps, as a quadratic
        // trace is, and a ring of 13.
        for row in 0..10 {
            matrix[row * count + row] += 3.0;
            for (step, coupling) in [(1, -1.0), (2, -0.5)] {
                if row + step < 10 {
                    matrix[row * count + row + step] += coupling;
                    matrix[(row + step) * count + row] += coupling;
                }
            }
        }
        for k in 0..13 {
            let row = 10 + k;
            let next = 10 + (k + 1) % 13;
            matrix[row * count + row] += 2.0;
            matrix[row * count + next] -= 1.0;
            matrix[next * count + row] -= 1.0;
        }
        let entries = |zeros_between: bool| {
            let mut entries = (0..count)
                .flat_map(|row| (0..count).map(move |column| (row, column)))
                .filter(|&(row, column)| matrix[row * count + column] != 0.0)
                .map(|(row, column)| (row as u32, column as u32, matrix[row * count + column]))
                .collect::<Vec<_>>();
            if zeros_between {
                for chain in 0..10_u32 {
                    for ring in 10..23_u32 {
                        entries.push((chain, ring, 0.0));
                        entries.push((ring, chain, 0.0));
                    }
                }
            }
            entries
        };
        let (plain_values, plain_vectors, _, _) = eigensolve_entries(entries(false), count);
        let (values, vectors, _, _) = eigensolve_entries(entries(true), count);
        assert_eq!(values, plain_values);
        assert_eq!(vectors, plain_vectors);
        let [residual, orthogonality] = eigen_errors(&matrix, count, &values, &vectors);
        assert!(residual < 1.0e-14, "{residual:e}");
        assert!(orthogonality < 1.0e-13, "{orthogonality:e}");
    }

    /// An empty trace answers at once; an asymmetric one and one with a
    /// repeated entry are refused, and so is a nonzero outside a band's
    /// storage, where a zero is nothing to store.
    #[test]
    fn the_trace_eigensolve_refuses_what_is_not_a_symmetric_operator() {
        let mut empty = BandedEigenJob::new(Vec::new(), 0).unwrap();
        let (values, vectors) = empty.advance(1).unwrap().unwrap();
        assert!(values.is_empty() && vectors.is_empty());
        assert!(BandedEigenJob::new(vec![(0, 1, 1.0), (1, 0, 1.5)], 2).is_err());
        assert!(BandedEigenJob::new(vec![(0, 0, 1.0), (0, 0, 1.0)], 2).is_err());
        assert!(BandedEigenJob::new(vec![(0, 2, 1.0)], 2).is_err());
        let mut band = SymmetricBand::new(3, 1);
        assert!(band.place(0, 2, 0.0).is_ok());
        assert!(band.place(0, 2, 1.0).is_err());
        assert!(band.place(0, 1, 1.0).is_ok());
        assert_eq!(band.get(0, 1), 1.0);
    }
}
