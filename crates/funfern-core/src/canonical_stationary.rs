//! The stationary part of a complementary flux, solved for cooperatively.
//!
//! A flux splits into the flux of a potential, which moves the field, and a
//! part `C^T W J` takes to no force, which never moves and never leaves: no
//! wall absorbs it and the grid filter keeps it on purpose. A fixed
//! generation started from rest holds none, since its flux only ever grows by
//! `C u`. A handoff makes some: its L2 projection carries each old element's
//! flux onto the new elements, and on one that straddles old elements, as an
//! adaptive refinement's edge flips and every remesh leave, the projection of
//! a potential's flux is the flux of no potential the new mesh can hold
//! (2026-11-03). Being inert, the part is the same at every step of its
//! generation, so it can be found from any readback and taken away whenever
//! that lands, and taking it away changes nothing of how the field moves.
//!
//! [`CanonicalWaveOperator::stationary_complementary_component`] is the
//! oracle: it solves the same system in one call, unpreconditioned, to
//! `2e-12`. This job takes the same operator, `C^T W J C`, through
//! Jacobi-preconditioned conjugate gradients in slices of one element each.

use crate::{CanonicalWaveOperator, Point2, SymmetricTensor2, WaveError};
use std::sync::Arc;

/// Where the solve stops: the residual of the potential's equation against
/// the flux's own force. Measured on the default box at the smallest element
/// adaptation allows, 45,203 triangles, `1e-4` already left 1e-8 of the
/// stationary energy behind and `1e-6` 1e-12, in 1,224 and 1,665 iterations.
/// What the solve leaves is the flux of a potential, and moves: at `1e-6`,
/// taken away after a handoff, it moved the box's field 4e-7 off its course
/// over 400 steps, under the f32 device's own departure from the f64
/// reference.
///
/// That is an isotropic medium's. What a residual leaves behind is bounded
/// by its square times the operator's condition number, which anisotropy
/// multiplies: at `1e-6` the box at axis ratios of 1e4 to 1e6, a tensor's
/// eigenvalues 1e8 to 1e12 apart, left 1e-7 to 5e-5 of a potential's flux
/// behind as stationary (C8.1, 2026-11-10). A job divides this by the root
/// of the largest axis ratio its medium has, up to `TIGHTENED_TO_AXIS_RATIO`.
/// At 1e4 the box then left 2e-13 at edge 0.1 and 7e-14 at 0.025.
pub const STATIONARY_FLUX_TOLERANCE: f64 = 1.0e-6;

/// The axis ratio past which the tolerance tightens no further. At 1e6 the
/// box held 4.8e-10 of its flux stationary at edge 0.1 and 1.4e-10 at 0.05
/// at any tolerance from `3e-9` to `1e-10`, a stationary part the run's own
/// rounding made rather than the solve's; at edge 0.025 `1e-9` did not
/// converge in four times the node count, where `1e-8` took 2.74 times it
/// and left 3e-8 (C8.1, 2026-11-10).
const TIGHTENED_TO_AXIS_RATIO: f64 = 1.0e4;

/// What a [`CanonicalStationaryFluxJob`] found.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalStationaryFlux {
    /// The stationary part, sample by sample.
    pub stationary: Vec<Point2>,
    /// The part's stored energy, and the whole flux's.
    pub energy: f64,
    pub flux_energy: f64,
    pub iterations: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// The preconditioner and the flux's force, element by element.
    Gather(usize),
    Start,
    /// `C^T W J C p`, element by element.
    Apply(usize),
    Update,
    Finish,
    Done,
}

impl Phase {
    const fn label(self) -> &'static str {
        match self {
            Self::Gather(_) => "Gathering the flux's force",
            Self::Start | Self::Apply(_) | Self::Update => "Solving for the flux's potential",
            Self::Finish => "Separating the stationary flux",
            Self::Done => "Finished",
        }
    }
}

/// Finds the stationary part of a complementary flux on `operator`, one
/// element or one vector update per work unit.
pub struct CanonicalStationaryFluxJob {
    operator: Arc<CanonicalWaveOperator>,
    flux: Vec<Point2>,
    phase: Phase,
    diagonal: Vec<f64>,
    potential: Vec<f64>,
    residual: Vec<f64>,
    preconditioned: Vec<f64>,
    direction: Vec<f64>,
    product: Vec<f64>,
    residual_dot: f64,
    force_norm: f64,
    tolerance: f64,
    iterations: usize,
    limit: usize,
}

impl CanonicalStationaryFluxJob {
    pub fn new(operator: Arc<CanonicalWaveOperator>, flux: Vec<Point2>) -> Result<Self, WaveError> {
        if flux.len() != operator.complementary_degrees_of_freedom()
            || flux.iter().any(|value| !value.finite())
        {
            return Err(WaveError::InvalidState);
        }
        let nodes = operator.degrees_of_freedom();
        // Conjugate gradients take some square root of the condition number,
        // which grows as the mesh's node count does: 1,665 iterations on
        // 136,131 nodes. Twenty of that root and a floor are far past it on
        // an isotropic medium and an even mesh, but not on an anisotropic
        // one or a graded one: the box at axis ratio 64 took 4,233 on 5,485
        // nodes, at 1e4 45,279 on 21,931, and a channel 3e-4 wide past the
        // root's 2,980 (C8.1, 2026-11-10). Past a few hundred, the count
        // runs to a few times the node count, where conjugate gradients in
        // exact arithmetic would have finished: at the tolerance anisotropy
        // tightens to, the box at axis ratio 1e4 took 2.73 times it at edge
        // 0.05 and 3.30 at 0.025, 87,451 nodes. Four times it bounds every
        // mesh measured; a finer one at that ratio may pass it, as Jacobi
        // preconditioning does nothing for anisotropy.
        let limit = (20 * (nodes as f64).sqrt().ceil() as usize + 1_000).max(4 * nodes);
        // A medium's axis ratio is the root of its tensor's eigenvalue ratio.
        let axis_ratio = operator
            .constitutive_samples()
            .iter()
            .map(|sample| eigenvalue_ratio(sample.complementary_inverse))
            .fold(1.0, f64::max)
            .sqrt();
        Ok(Self {
            operator,
            flux,
            phase: Phase::Gather(0),
            diagonal: vec![0.0; nodes],
            potential: vec![0.0; nodes],
            residual: vec![0.0; nodes],
            preconditioned: vec![0.0; nodes],
            direction: vec![0.0; nodes],
            product: vec![0.0; nodes],
            residual_dot: 0.0,
            force_norm: 0.0,
            tolerance: STATIONARY_FLUX_TOLERANCE / axis_ratio.min(TIGHTENED_TO_AXIS_RATIO).sqrt(),
            iterations: 0,
            limit,
        })
    }

    pub fn phase(&self) -> &'static str {
        self.phase.label()
    }

    /// The residual the solve stops at, against the flux's force.
    pub fn tolerance(&self) -> f64 {
        self.tolerance
    }

    /// The iterations the solve may take.
    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn advance(&mut self, budget: usize) -> Option<Result<CanonicalStationaryFlux, WaveError>> {
        for _ in 0..budget.max(1) {
            match self.step() {
                Ok(Some(found)) => return Some(Ok(found)),
                Ok(None) => {}
                Err(error) => {
                    self.phase = Phase::Done;
                    return Some(Err(error));
                }
            }
            if self.phase == Phase::Done {
                return None;
            }
        }
        None
    }

    fn step(&mut self) -> Result<Option<CanonicalStationaryFlux>, WaveError> {
        let elements = self.operator.element_nodes().len();
        match self.phase {
            Phase::Gather(element) if element == elements => self.phase = Phase::Start,
            Phase::Gather(element) => {
                self.gather(element);
                self.phase = Phase::Gather(element + 1);
            }
            Phase::Start => {
                project_component_means(
                    &mut self.residual,
                    self.operator.component_labels(),
                    self.operator.component_count(),
                );
                self.force_norm = dot(&self.residual, &self.residual).sqrt();
                if !self.force_norm.is_finite() {
                    return Err(WaveError::InvalidState);
                }
                if self.force_norm == 0.0 {
                    self.phase = Phase::Finish;
                    return Ok(None);
                }
                self.precondition();
                self.direction.clone_from(&self.preconditioned);
                self.residual_dot = dot(&self.residual, &self.preconditioned);
                self.product.fill(0.0);
                self.phase = Phase::Apply(0);
            }
            Phase::Apply(element) if element == elements => self.phase = Phase::Update,
            Phase::Apply(element) => {
                self.apply(element);
                self.phase = Phase::Apply(element + 1);
            }
            Phase::Update => self.update()?,
            Phase::Finish => return self.finish().map(Some),
            Phase::Done => {}
        }
        Ok(None)
    }

    /// One element's share of the Jacobi preconditioner and of the flux's
    /// force, as [`CanonicalWaveOperator::force`] gathers it.
    fn gather(&mut self, element: usize) {
        let operator = &self.operator;
        let orientation = operator.orientation();
        let nodes = operator.element_nodes()[element];
        for index in element * SAMPLES..(element + 1) * SAMPLES {
            let sample = &operator.constitutive_samples()[index];
            let field = sample.complementary_inverse.apply(self.flux[index]);
            for (local, node) in nodes.iter().enumerate() {
                let curl = sample.curls()[local];
                self.diagonal[*node as usize] +=
                    sample.integration_weight * curl.dot(sample.complementary_inverse.apply(curl));
                self.residual[*node as usize] +=
                    orientation * sample.integration_weight * curl.dot(field);
            }
        }
    }

    /// One element's share of `C^T W J C` applied to the search direction:
    /// the compatible flux in its difference form, then the force, as the
    /// operator's own two calls form them.
    fn apply(&mut self, element: usize) {
        let operator = &self.operator;
        let orientation = operator.orientation();
        let nodes = operator.element_nodes()[element];
        let reference = self.direction[nodes[0] as usize];
        for index in element * SAMPLES..(element + 1) * SAMPLES {
            let sample = &operator.constitutive_samples()[index];
            let mut curl = Point2::default();
            for (local, node) in nodes.iter().enumerate().skip(1) {
                curl = curl + sample.curls()[local] * (self.direction[*node as usize] - reference);
            }
            let field = sample.complementary_inverse.apply(curl * orientation);
            for (local, node) in nodes.iter().enumerate() {
                self.product[*node as usize] +=
                    orientation * sample.integration_weight * sample.curls()[local].dot(field);
            }
        }
    }

    fn update(&mut self) -> Result<(), WaveError> {
        let curvature = dot(&self.direction, &self.product);
        if !curvature.is_finite() || curvature <= 0.0 {
            return Err(WaveError::InvalidState);
        }
        let step = self.residual_dot / curvature;
        for node in 0..self.potential.len() {
            self.potential[node] += step * self.direction[node];
            self.residual[node] -= step * self.product[node];
        }
        project_component_means(
            &mut self.residual,
            self.operator.component_labels(),
            self.operator.component_count(),
        );
        self.iterations += 1;
        let remaining = dot(&self.residual, &self.residual).sqrt();
        if !remaining.is_finite() {
            return Err(WaveError::InvalidState);
        }
        if remaining <= self.tolerance * self.force_norm {
            self.phase = Phase::Finish;
            return Ok(());
        }
        if self.iterations >= self.limit {
            return Err(WaveError::NotConverged {
                solve: "solve for the flux's potential",
                limit: self.limit,
            });
        }
        self.precondition();
        let next = dot(&self.residual, &self.preconditioned);
        let ratio = next / self.residual_dot;
        self.residual_dot = next;
        for node in 0..self.direction.len() {
            self.direction[node] = self.preconditioned[node] + ratio * self.direction[node];
        }
        self.product.fill(0.0);
        self.phase = Phase::Apply(0);
        Ok(())
    }

    fn precondition(&mut self) {
        for ((value, residual), diagonal) in self
            .preconditioned
            .iter_mut()
            .zip(&self.residual)
            .zip(&self.diagonal)
        {
            *value = if *diagonal > 0.0 {
                residual / diagonal
            } else {
                0.0
            };
        }
    }

    fn finish(&mut self) -> Result<CanonicalStationaryFlux, WaveError> {
        let compatible = self.operator.compatible_flux(&self.potential)?;
        let stationary = self
            .flux
            .iter()
            .zip(compatible)
            .map(|(flux, compatible)| *flux - compatible)
            .collect::<Vec<_>>();
        let energy_of = |flux: &[Point2]| {
            self.operator
                .constitutive_samples()
                .iter()
                .zip(flux)
                .map(|(sample, value)| {
                    0.5 * sample.integration_weight
                        * value.dot(sample.complementary_inverse.apply(*value))
                })
                .sum::<f64>()
        };
        let found = CanonicalStationaryFlux {
            energy: energy_of(&stationary),
            flux_energy: energy_of(&self.flux),
            stationary,
            iterations: self.iterations,
        };
        self.phase = Phase::Done;
        if !found.energy.is_finite() || !found.flux_energy.is_finite() {
            return Err(WaveError::InvalidState);
        }
        Ok(found)
    }
}

const SAMPLES: usize = 6;

/// A symmetric positive tensor's largest eigenvalue over its smallest, as
/// the largest's square over the determinant, which has no cancellation:
/// exactly 1 for an isotropic one.
fn eigenvalue_ratio(tensor: SymmetricTensor2) -> f64 {
    let largest = tensor.eigenvalues()[1];
    (largest * largest / tensor.determinant()).max(1.0)
}

fn dot(left: &[f64], right: &[f64]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

/// Each connected component's mean taken out, as the oracle's solve does:
/// the potential is free by a constant on each, and the force of any flux
/// sums to nothing over one but for rounding.
fn project_component_means(values: &mut [f64], labels: &[u32], count: usize) {
    let mut sums = vec![0.0; count];
    let mut sizes = vec![0usize; count];
    for (value, label) in values.iter().zip(labels) {
        sums[*label as usize] += value;
        sizes[*label as usize] += 1;
    }
    for (value, label) in values.iter_mut().zip(labels) {
        *value -= sums[*label as usize] / sizes[*label as usize] as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_scenes::*;
    use crate::{MeshingOptions, OuterBoundaryCondition, TopologyWaveModel};

    fn operator(edge: f64) -> Arc<CanonicalWaveOperator> {
        let scene = initial();
        let mesh = meshed(
            &scene,
            7,
            MeshingOptions {
                target_edge_length: edge,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let scalar = assembled(&mesh, &scene, OuterBoundaryCondition::FirstOrderOutgoing).unwrap();
        Arc::new(
            CanonicalWaveOperator::compile(
                &mesh,
                &scalar,
                TopologyWaveModel::from_topology_scene(&scene),
                7,
            )
            .unwrap(),
        )
    }

    fn smooth(operator: &CanonicalWaveOperator) -> Vec<Point2> {
        operator
            .constitutive_samples()
            .iter()
            .map(|sample| {
                let p = sample.point;
                Point2::new(
                    (5.0 * p.x).sin() * (3.0 * p.y).cos(),
                    (4.0 * p.y).sin() + p.x,
                )
            })
            .collect()
    }

    fn solved(
        operator: &Arc<CanonicalWaveOperator>,
        flux: Vec<Point2>,
        budget: usize,
    ) -> CanonicalStationaryFlux {
        let mut job = CanonicalStationaryFluxJob::new(operator.clone(), flux).unwrap();
        loop {
            if let Some(found) = job.advance(budget) {
                return found.unwrap();
            }
        }
    }

    fn energy(operator: &CanonicalWaveOperator, flux: &[Point2]) -> f64 {
        operator
            .constitutive_samples()
            .iter()
            .zip(flux)
            .map(|(sample, value)| {
                0.5 * sample.integration_weight
                    * value.dot(sample.complementary_inverse.apply(*value))
            })
            .sum()
    }

    /// The job's stationary part is the oracle's, to what the tolerance
    /// leaves, whatever slices it is taken in: a third of a smooth field's
    /// energy here, found to 1e-12 of it.
    #[test]
    fn the_stationary_flux_is_the_oracles_in_any_slices() {
        let operator = operator(0.1);
        let flux = smooth(&operator);
        let exact = operator.stationary_complementary_component(&flux).unwrap();
        let whole = solved(&operator, flux.clone(), usize::MAX);
        let sliced = solved(&operator, flux.clone(), 3);
        assert_eq!(whole, sliced, "the slices changed the answer");
        let share = whole.energy / whole.flux_energy;
        assert!((0.1..0.9).contains(&share), "a stationary share of {share}");
        let gap = whole
            .stationary
            .iter()
            .zip(&exact)
            .map(|(found, exact)| *found - *exact)
            .collect::<Vec<_>>();
        let left = energy(&operator, &gap) / energy(&operator, &exact);
        assert!(
            left < 1.0e-10,
            "the job is {left:e} of the oracle's part off it"
        );
        assert_eq!(whole.energy, energy(&operator, &whole.stationary));
    }

    /// The flux of a potential has no stationary part, and a stationary
    /// part has nothing but: what the job finds is the whole of it, and it
    /// pushes no node.
    #[test]
    fn a_potentials_flux_is_kept_and_a_stationary_one_found_whole() {
        let operator = operator(0.1);
        let potential = operator
            .node_points()
            .iter()
            .map(|point| (3.0 * point.x).sin() + 0.4 * point.y * point.y)
            .collect::<Vec<_>>();
        let compatible = operator.compatible_flux(&potential).unwrap();
        let found = solved(&operator, compatible.clone(), usize::MAX);
        assert!(
            found.energy < 1.0e-12 * found.flux_energy,
            "a potential's flux was {:e} stationary",
            found.energy / found.flux_energy
        );

        let stationary = operator
            .stationary_complementary_component(&smooth(&operator))
            .unwrap();
        let found = solved(&operator, stationary.clone(), usize::MAX);
        let kept = found.energy / energy(&operator, &stationary);
        assert!(
            (kept - 1.0).abs() < 1.0e-9,
            "found {kept} of a stationary part"
        );
        let force = operator.force(&found.stationary).unwrap();
        let pushed = force.iter().map(|value| value * value).sum::<f64>().sqrt();
        let scale = operator
            .force(&smooth(&operator))
            .unwrap()
            .iter()
            .map(|value| value * value)
            .sum::<f64>()
            .sqrt();
        assert!(pushed < 1.0e-9 * scale, "the part found pushes {pushed:e}");
    }

    #[test]
    fn a_flux_of_the_wrong_shape_is_refused() {
        let operator = operator(0.2);
        let mut flux = smooth(&operator);
        flux.pop();
        assert!(CanonicalStationaryFluxJob::new(operator.clone(), flux).is_err());
        let mut flux = smooth(&operator);
        flux[0].x = f64::NAN;
        assert!(CanonicalStationaryFluxJob::new(operator, flux).is_err());
    }
}
