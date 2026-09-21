//! Synchronized consumers of the canonical direct `(Q, b)` state.
//!
//! These adapters deliberately start from the accepted integrated primary
//! flux, the independent six-sample complementary flux, and physical
//! auxiliaries. They never reconstruct a bulk potential or reinterpret old
//! displacement levels.
//!
//! One rule governs every consumer here and its GPU counterpart. A
//! constitutive inverse is applied only where the solver owns one: the
//! assembled nodal map at a node, and the element's own six samples for the
//! complementary flux. Physical fields recovered there are then interpolated
//! to wherever a consumer reports them. Densities and flow are evaluated from
//! those interpolated fields through forward maps alone. Interpolating the
//! flux first and inverting once at an arbitrary point would create a
//! constitutive evaluation site the solver does not have, which a nonlinear
//! law cannot serve without an extra uncached bracketed solve, and it is why
//! the order here is worth the extra stencil data.

use crate::{
    CanonicalWaveOperator, Point2, QuadraticAreaElement, QuadraticAreaStencil,
    QuadraticPointStencil, SymmetricTensor2, WaveError, enriched_quadratic_basis,
};

const LOCAL_NODES: usize = 7;
const COMPLEMENTARY_SAMPLES: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalPointStencil {
    pub nodes: [u32; LOCAL_NODES],
    pub primary_weights: [f64; LOCAL_NODES],
    /// Inverse immutable generation mass at each primary node. Canonical
    /// storage owns integrated flux `Q`; consumers display `u = Q / M`.
    pub primary_inverse_mass: [f64; LOCAL_NODES],
    pub complementary_samples: [u32; COMPLEMENTARY_SAMPLES],
    pub complementary_weights: [f64; COMPLEMENTARY_SAMPLES],
    /// The constitutive inverse at each of the element's own samples, where
    /// the physical complementary field is recovered before interpolation.
    /// These are immutable for the life of a generation, so holding them here
    /// cannot go stale the way a copied law value would.
    pub sample_inverses: [SymmetricTensor2; COMPLEMENTARY_SAMPLES],
    pub primary_reference: f64,
    /// Forward constitutive coefficient at the probe point. The complementary
    /// energy density is `w = 0.5 e . C e`, evaluated from the interpolated
    /// field without inverting anything a second time.
    pub complementary_reference: SymmetricTensor2,
    pub orientation: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanonicalPointSample {
    pub primary: f64,
    pub primary_rate: f64,
    pub complementary: Point2,
    pub energy_density: f64,
    pub energy_flow: Point2,
}

impl CanonicalPointStencil {
    pub fn from_quadratic(
        stencil: QuadraticPointStencil,
        operator: &CanonicalWaveOperator,
    ) -> Result<Self, WaveError> {
        let element = stencil.element as usize;
        if operator.element_nodes().get(element) != Some(&stencil.nodes) {
            return Err(WaveError::InvalidMesh(
                "the point stencil does not match the canonical element",
            ));
        }
        let samples = operator
            .constitutive_samples()
            .get(element * COMPLEMENTARY_SAMPLES..(element + 1) * COMPLEMENTARY_SAMPLES)
            .ok_or(WaveError::InvalidMesh(
                "the canonical element has no complementary samples",
            ))?;
        let complementary_weights = complementary_interpolation_weights(
            samples
                .iter()
                .map(|sample| sample.barycentric)
                .collect::<Vec<_>>()
                .try_into()
                .map_err(|_| WaveError::InvalidMesh("invalid complementary sample count"))?,
            stencil.barycentric,
        )?;
        let complementary_reference = rotate_tensor(stencil.stiffness)
            .inverse()
            .ok_or(WaveError::InvalidCoefficients)?;
        Ok(Self {
            nodes: stencil.nodes,
            primary_weights: stencil.value_weights,
            primary_inverse_mass: stencil
                .nodes
                .map(|node| operator.primary_mass()[node as usize].recip()),
            complementary_samples: std::array::from_fn(|local| {
                (element * COMPLEMENTARY_SAMPLES + local) as u32
            }),
            complementary_weights,
            sample_inverses: std::array::from_fn(|local| samples[local].complementary_inverse),
            primary_reference: stencil.mass_density,
            complementary_reference,
            orientation: operator.orientation(),
        })
    }

    /// The physical complementary field at the probe point. Each of the
    /// element's samples is inverted where the solver owns that inverse, and
    /// the resulting fields are interpolated.
    pub fn complementary_field(&self, complementary_flux: &[Point2]) -> Result<Point2, WaveError> {
        let mut field = Point2::default();
        for local in 0..COMPLEMENTARY_SAMPLES {
            let Some(flux) = complementary_flux.get(self.complementary_samples[local] as usize)
            else {
                return Err(WaveError::InvalidState);
            };
            field = field
                + self.sample_inverses[local].apply(*flux) * self.complementary_weights[local];
        }
        Ok(field)
    }

    pub fn sample(
        &self,
        primary: &[f64],
        primary_rate: &[f64],
        complementary_flux: &[Point2],
    ) -> Result<CanonicalPointSample, WaveError> {
        let mut u = 0.0;
        let mut rate = 0.0;
        for local in 0..LOCAL_NODES {
            let node = self.nodes[local] as usize;
            let (Some(value), Some(derivative)) = (primary.get(node), primary_rate.get(node))
            else {
                return Err(WaveError::InvalidState);
            };
            let inverse_mass = self.primary_inverse_mass[local];
            u += self.primary_weights[local] * value * inverse_mass;
            rate += self.primary_weights[local] * derivative * inverse_mass;
        }
        let complementary = self.complementary_field(complementary_flux)?;
        let energy_density = 0.5
            * (self.primary_reference * u * u
                + self.complementary_reference.quadratic_form(complementary));
        let energy_flow = rotate(complementary) * (self.orientation * u);
        if [u, rate, energy_density, energy_flow.x, energy_flow.y]
            .into_iter()
            .all(f64::is_finite)
            && complementary.finite()
        {
            Ok(CanonicalPointSample {
                primary: u,
                primary_rate: rate,
                complementary,
                energy_density,
                energy_flow,
            })
        } else {
            Err(WaveError::InvalidState)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanonicalEnergyBreakdown {
    pub primary: f64,
    pub complementary: f64,
    pub thin_gap: f64,
    pub outgoing: f64,
}

impl CanonicalEnergyBreakdown {
    pub fn total(self) -> f64 {
        self.primary + self.complementary + self.thin_gap + self.outgoing
    }
}

pub fn canonical_energy_breakdown(
    operator: &CanonicalWaveOperator,
    primary_flux: &[f64],
    complementary_flux: &[Point2],
    auxiliary: &[f64],
) -> Result<CanonicalEnergyBreakdown, WaveError> {
    if primary_flux.len() != operator.degrees_of_freedom()
        || complementary_flux.len() != operator.complementary_degrees_of_freedom()
    {
        return Err(WaveError::InvalidState);
    }
    let gap_count = operator.thin_gap_samples().len();
    let outgoing_count = operator
        .outgoing_boundary()
        .map_or(0, |boundary| boundary.auxiliary_count());
    if auxiliary.len() != gap_count + outgoing_count {
        return Err(WaveError::InvalidState);
    }
    let primary = primary_flux
        .iter()
        .zip(operator.primary_mass())
        .map(|(flux, mass)| 0.5 * flux * flux / mass)
        .sum();
    let complementary = complementary_flux
        .iter()
        .zip(operator.constitutive_samples())
        .map(|(flux, sample)| {
            0.5 * sample.integration_weight * flux.dot(sample.complementary_inverse.apply(*flux))
        })
        .sum();
    let thin_gap = auxiliary[..gap_count]
        .iter()
        .zip(operator.thin_gap_samples())
        .map(|(jump, sample)| 0.5 * sample.stiffness * jump * jump)
        .sum();
    let outgoing = auxiliary[gap_count..]
        .iter()
        .map(|value| 0.5 * value * value)
        .sum();
    let result = CanonicalEnergyBreakdown {
        primary,
        complementary,
        thin_gap,
        outgoing,
    };
    if [
        result.primary,
        result.complementary,
        result.thin_gap,
        result.outgoing,
        result.total(),
    ]
    .into_iter()
    .all(f64::is_finite)
    {
        Ok(result)
    } else {
        Err(WaveError::InvalidState)
    }
}

/// Field statistics come from the smooth twelve-point rule over the covered
/// area; `total_energy` and `mean_energy_density` come from the canonical
/// decomposition instead, so a probe covering everything reports exactly the
/// solver's own bulk energy.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanonicalAreaSample {
    pub mean_primary: f64,
    pub rms_primary: f64,
    pub rms_complementary: f64,
    pub mean_energy_density: f64,
    pub total_energy: f64,
    pub covered_area: f64,
    pub coverage: f64,
}

/// One physical quadrature record used by CPU and GPU area consumers.
///
/// It carries no constitutive coefficient. Field statistics are moments of
/// the interpolated fields, and the reported energy comes from the canonical
/// decomposition on the parent element, so nothing here needs a material law.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanonicalAreaQuadraturePoint {
    pub primary_weights: [f64; LOCAL_NODES],
    pub complementary_weights: [f64; COMPLEMENTARY_SAMPLES],
    pub physical_weight: f64,
}

/// One clipped piece of a parent element, compiled for both the f64 oracle
/// and the recorder shader.
///
/// The displayed field statistics use the smooth twelve-point rule over
/// `quadrature`, while the reported energy uses the canonical decomposition
/// below. Those are different quantities on purpose: the mass-lumped nodal
/// energy the solver conserves is not the integral of a pointwise density,
/// and an energy probe that cannot close against the solver's own total is
/// not worth much as a diagnostic.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalAreaContribution {
    /// Constitutive inverse at the parent element's own samples.
    pub sample_inverses: [SymmetricTensor2; COMPLEMENTARY_SAMPLES],
    /// Integration weight carried by each of those samples.
    pub sample_weights: [f64; COMPLEMENTARY_SAMPLES],
    /// This element's contribution to each local node's assembled lumped
    /// mass, before the node's inverse mass is applied. Summed over every
    /// element incident on a node these are exactly that node's mass, so a
    /// probe covering the whole domain reports the solver's own energy.
    /// Storing the contribution rather than the finished share keeps the
    /// record independent of time: a driven material changes the mass, not
    /// this weight.
    pub node_references: [f64; LOCAL_NODES],
    /// Fraction of the parent element covered by this clipped piece. The
    /// canonical terms are element-wide, so a partly covered element
    /// contributes in proportion to its covered area.
    pub covered_fraction: f64,
    pub quadrature: [CanonicalAreaQuadraturePoint; 12],
}

/// Compiles the canonical reconstruction used by an area contribution.
/// Keeping this in core gives recorder shaders and the f64 oracle exactly the
/// same interpolation and energy rules.
pub fn canonical_area_contribution(
    element: QuadraticAreaElement,
    operator: &CanonicalWaveOperator,
) -> Result<CanonicalAreaContribution, WaveError> {
    let canonical_nodes = operator
        .element_nodes()
        .get(element.element as usize)
        .ok_or(WaveError::InvalidMesh(
            "area element is outside canonical state",
        ))?;
    if canonical_nodes != &element.nodes {
        return Err(WaveError::InvalidMesh(
            "area element does not match canonical numbering",
        ));
    }
    let samples = operator
        .constitutive_samples()
        .get(
            element.element as usize * COMPLEMENTARY_SAMPLES
                ..(element.element as usize + 1) * COMPLEMENTARY_SAMPLES,
        )
        .ok_or(WaveError::InvalidMesh(
            "area element has no canonical complementary samples",
        ))?;
    let sample_points: [[f64; 3]; COMPLEMENTARY_SAMPLES] = samples
        .iter()
        .map(|sample| sample.barycentric)
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| WaveError::InvalidMesh("invalid complementary sample count"))?;
    let mut compiled = [CanonicalAreaQuadraturePoint::default(); 12];
    for (slot, (local, weight)) in area_quadrature().into_iter().enumerate() {
        let barycentric = std::array::from_fn(|coordinate| {
            element.barycentric_vertices[0][coordinate] * local[0]
                + element.barycentric_vertices[1][coordinate] * local[1]
                + element.barycentric_vertices[2][coordinate] * local[2]
        });
        compiled[slot] = CanonicalAreaQuadraturePoint {
            primary_weights: enriched_quadratic_basis(barycentric),
            complementary_weights: complementary_interpolation_weights(sample_points, barycentric)?,
            physical_weight: element.area * weight,
        };
    }

    // The parent's own contributions are emitted in element order, seven per
    // element, so the share of each node is a direct index rather than a scan.
    let parent = element.element as usize;
    let contributions = operator.primary_contributions();
    let mut node_references = [0.0; LOCAL_NODES];
    for (local, reference) in node_references.iter_mut().enumerate() {
        let contribution = contributions
            .get(parent * LOCAL_NODES + local)
            .filter(|contribution| {
                contribution.element as usize == parent
                    && contribution.local_node as usize == local
                    && contribution.node == element.nodes[local]
            })
            .ok_or(WaveError::InvalidMesh(
                "area element does not match the canonical primary contributions",
            ))?;
        *reference = contribution.geometric_weight * contribution.reference_coefficient;
    }

    // Area of the clipped triangle in the parent's own barycentric frame, so
    // it is the covered fraction whatever the parent's physical size.
    let [origin, second, third] = element.barycentric_vertices;
    let covered_fraction = ((second[1] - origin[1]) * (third[2] - origin[2])
        - (third[1] - origin[1]) * (second[2] - origin[2]))
        .abs();
    if !covered_fraction.is_finite() || covered_fraction <= 0.0 {
        return Err(WaveError::InvalidMesh("area element covers no parent area"));
    }

    Ok(CanonicalAreaContribution {
        sample_inverses: std::array::from_fn(|local| samples[local].complementary_inverse),
        sample_weights: std::array::from_fn(|local| samples[local].integration_weight),
        node_references,
        covered_fraction,
        quadrature: compiled,
    })
}

pub fn sample_canonical_area(
    stencil: &QuadraticAreaStencil,
    operator: &CanonicalWaveOperator,
    primary: &[f64],
    complementary_flux: &[Point2],
) -> Result<CanonicalAreaSample, WaveError> {
    if primary.len() != operator.degrees_of_freedom()
        || complementary_flux.len() != operator.complementary_degrees_of_freedom()
        || stencil.covered_area <= 0.0
        || stencil.target_area <= 0.0
    {
        return Err(WaveError::InvalidState);
    }
    let mut primary_integral = 0.0;
    let mut primary_squared = 0.0;
    let mut complementary_squared = 0.0;
    let mut total_energy = 0.0;
    for element in &stencil.elements {
        sample_area_element(
            *element,
            operator,
            primary,
            complementary_flux,
            &mut primary_integral,
            &mut primary_squared,
            &mut complementary_squared,
            &mut total_energy,
        )?;
    }
    let result = CanonicalAreaSample {
        mean_primary: primary_integral / stencil.covered_area,
        rms_primary: (primary_squared / stencil.covered_area).max(0.0).sqrt(),
        rms_complementary: (complementary_squared / stencil.covered_area)
            .max(0.0)
            .sqrt(),
        mean_energy_density: total_energy / stencil.covered_area,
        total_energy,
        covered_area: stencil.covered_area,
        coverage: (stencil.covered_area / stencil.target_area).clamp(0.0, 1.0),
    };
    if [
        result.mean_primary,
        result.rms_primary,
        result.rms_complementary,
        result.mean_energy_density,
        result.total_energy,
        result.covered_area,
        result.coverage,
    ]
    .into_iter()
    .all(f64::is_finite)
    {
        Ok(result)
    } else {
        Err(WaveError::InvalidState)
    }
}

#[allow(clippy::too_many_arguments)]
fn sample_area_element(
    element: QuadraticAreaElement,
    operator: &CanonicalWaveOperator,
    primary: &[f64],
    complementary_flux: &[Point2],
    primary_integral: &mut f64,
    primary_squared: &mut f64,
    complementary_squared: &mut f64,
    total_energy: &mut f64,
) -> Result<(), WaveError> {
    let contribution = canonical_area_contribution(element, operator)?;
    let start = element.element as usize * COMPLEMENTARY_SAMPLES;

    // The element's own samples carry the only complementary inverses the
    // solver owns, so recover the physical field there once and interpolate
    // it at each quadrature point below.
    let mut sample_fields = [Point2::default(); COMPLEMENTARY_SAMPLES];
    for (local, field) in sample_fields.iter_mut().enumerate() {
        let Some(flux) = complementary_flux.get(start + local) else {
            return Err(WaveError::InvalidState);
        };
        *field = contribution.sample_inverses[local].apply(*flux);
    }

    for point in contribution.quadrature {
        let u = point
            .primary_weights
            .iter()
            .zip(element.nodes)
            .map(|(basis, node)| {
                basis * primary[node as usize] / operator.primary_mass()[node as usize]
            })
            .sum::<f64>();
        let complementary = point
            .complementary_weights
            .iter()
            .zip(sample_fields)
            .fold(Point2::default(), |sum, (weight, field)| {
                sum + field * *weight
            });
        *primary_integral += point.physical_weight * u;
        *primary_squared += point.physical_weight * u * u;
        *complementary_squared += point.physical_weight * complementary.dot(complementary);
    }

    // Reported energy is the solver's own discrete energy restricted to this
    // piece: the lumped nodal share plus the element's sample energies.
    let mut energy = 0.0;
    for (local, reference) in contribution.node_references.into_iter().enumerate() {
        let node = element.nodes[local] as usize;
        let (Some(flux), Some(mass)) = (primary.get(node), operator.primary_mass().get(node))
        else {
            return Err(WaveError::InvalidState);
        };
        if *mass <= 0.0 {
            return Err(WaveError::InvalidState);
        }
        energy += 0.5 * reference * flux * flux / (mass * mass);
    }
    for (local, field) in sample_fields.into_iter().enumerate() {
        let Some(flux) = complementary_flux.get(start + local) else {
            return Err(WaveError::InvalidState);
        };
        energy += 0.5 * contribution.sample_weights[local] * flux.dot(field);
    }
    *total_energy += contribution.covered_fraction * energy;
    Ok(())
}

fn rotate(value: Point2) -> Point2 {
    Point2::new(-value.y, value.x)
}

fn rotate_tensor(tensor: SymmetricTensor2) -> SymmetricTensor2 {
    SymmetricTensor2::new(tensor.yy, -tensor.xy, tensor.xx)
}

fn monomials([_l0, l1, l2]: [f64; 3]) -> [f64; 6] {
    [1.0, l1, l2, l1 * l1, l1 * l2, l2 * l2]
}

pub(crate) fn complementary_interpolation_weights(
    samples: [[f64; 3]; COMPLEMENTARY_SAMPLES],
    target: [f64; 3],
) -> Result<[f64; COMPLEMENTARY_SAMPLES], WaveError> {
    let mut matrix = [[0.0; COMPLEMENTARY_SAMPLES]; COMPLEMENTARY_SAMPLES];
    for (row, sample) in samples.into_iter().enumerate() {
        let values = monomials(sample);
        for column in 0..COMPLEMENTARY_SAMPLES {
            matrix[column][row] = values[column];
        }
    }
    solve_six(matrix, monomials(target))
}

fn solve_six(
    mut matrix: [[f64; COMPLEMENTARY_SAMPLES]; COMPLEMENTARY_SAMPLES],
    mut right: [f64; COMPLEMENTARY_SAMPLES],
) -> Result<[f64; COMPLEMENTARY_SAMPLES], WaveError> {
    for pivot in 0..COMPLEMENTARY_SAMPLES {
        let best = (pivot..COMPLEMENTARY_SAMPLES)
            .max_by(|left, right_row| {
                matrix[*left][pivot]
                    .abs()
                    .total_cmp(&matrix[*right_row][pivot].abs())
            })
            .ok_or(WaveError::InvalidMesh(
                "the complementary reconstruction is singular",
            ))?;
        if matrix[best][pivot].abs() <= 1.0e-14 {
            return Err(WaveError::InvalidMesh(
                "the complementary reconstruction is singular",
            ));
        }
        matrix.swap(pivot, best);
        right.swap(pivot, best);
        let pivot_values = matrix[pivot];
        for row in pivot + 1..COMPLEMENTARY_SAMPLES {
            let factor = matrix[row][pivot] / matrix[pivot][pivot];
            for (column, value) in matrix[row].iter_mut().enumerate().skip(pivot) {
                *value -= factor * pivot_values[column];
            }
            right[row] -= factor * right[pivot];
        }
    }
    let mut solution = [0.0; COMPLEMENTARY_SAMPLES];
    for row in (0..COMPLEMENTARY_SAMPLES).rev() {
        solution[row] = (right[row]
            - (row + 1..COMPLEMENTARY_SAMPLES)
                .map(|column| matrix[row][column] * solution[column])
                .sum::<f64>())
            / matrix[row][row];
    }
    solution
        .iter()
        .all(|value| value.is_finite())
        .then_some(solution)
        .ok_or(WaveError::InvalidState)
}

fn area_quadrature() -> [([f64; 3], f64); 12] {
    const A: f64 = 0.873_821_971_016_996;
    const B: f64 = 0.063_089_014_491_502;
    const C: f64 = 0.501_426_509_658_179;
    const D: f64 = 0.249_286_745_170_910;
    const E: f64 = 0.636_502_499_121_399;
    const F: f64 = 0.310_352_451_033_785;
    const G: f64 = 0.053_145_049_844_816;
    const W0: f64 = 0.050_844_906_370_207;
    const W1: f64 = 0.116_786_275_726_379;
    const W2: f64 = 0.082_851_075_618_374;
    [
        ([A, B, B], W0),
        ([B, A, B], W0),
        ([B, B, A], W0),
        ([C, D, D], W1),
        ([D, C, D], W1),
        ([D, D, C], W1),
        ([E, F, G], W2),
        ([E, G, F], W2),
        ([F, E, G], W2),
        ([F, G, E], W2),
        ([G, E, F], W2),
        ([G, F, E], W2),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AreaProbeShape, BACKGROUND_REGION, CanonicalWaveState, MeshingOptions,
        OuterBoundaryCondition, QuadraticAreaStencil, QuadraticPointStencil, QuadraticWaveOperator,
        Scene, TriMesh, mesh_scene,
    };

    fn scene_fixture() -> (TriMesh, QuadraticWaveOperator, CanonicalWaveOperator, Scene) {
        let scene = Scene::initial();
        let mesh = mesh_scene(
            &scene,
            1,
            MeshingOptions {
                target_edge_length: 0.3,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let scalar = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let canonical = CanonicalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1).unwrap();
        (mesh, scalar, canonical, scene)
    }

    fn fixture() -> (CanonicalWaveOperator, QuadraticPointStencil) {
        let (mesh, scalar, canonical, scene) = scene_fixture();
        let point = canonical.node_points()[canonical.element_nodes()[0][6] as usize];
        let stencil = QuadraticPointStencil::build(&mesh, &scalar, &scene, point).unwrap();
        (canonical, stencil)
    }

    /// What the canonical decomposition buys: an area probe covering every
    /// face reports the solver's own bulk energy, so an energy balance can be
    /// closed against the accounting lanes. A pointwise density integral
    /// cannot do this, because the primary energy the solver conserves is
    /// mass-lumped.
    #[test]
    fn area_probe_over_every_face_reports_the_solver_bulk_energy() {
        let (mesh, scalar, operator, scene) = scene_fixture();
        let stencil = QuadraticAreaStencil::build(
            &mesh,
            &scalar,
            &scene,
            AreaProbeShape::Region(BACKGROUND_REGION),
        )
        .unwrap();
        let primary = operator
            .node_points()
            .iter()
            .map(|point| 0.17 + 0.23 * (1.3 * point.x - 0.8 * point.y).sin())
            .collect::<Vec<_>>();
        let potential = operator
            .node_points()
            .iter()
            .map(|point| 0.09 * (0.7 * point.x + 1.1 * point.y).cos())
            .collect::<Vec<_>>();
        let state = CanonicalWaveState::from_primary_and_potential(
            &operator,
            0.5 * operator.maximum_time_step(),
            &primary,
            &potential,
        )
        .unwrap();
        let sample = sample_canonical_area(
            &stencil,
            &operator,
            state.primary_flux(),
            state.complementary_flux(),
        )
        .unwrap();
        let breakdown = canonical_energy_breakdown(
            &operator,
            state.primary_flux(),
            state.complementary_flux(),
            &[],
        )
        .unwrap();
        let expected = breakdown.primary + breakdown.complementary;
        assert!((sample.coverage - 1.0).abs() < 1.0e-9);
        assert!(
            (sample.total_energy - expected).abs() < 1.0e-9 * expected,
            "area probe reported {} against the solver's {expected}",
            sample.total_energy
        );
    }

    #[test]
    fn point_consumer_reconstructs_constant_physical_fields_and_flow() {
        let (operator, quadratic) = fixture();
        let stencil = CanonicalPointStencil::from_quadratic(quadratic, &operator).unwrap();
        let primary = operator
            .primary_mass()
            .iter()
            .map(|mass| 2.0 * mass)
            .collect::<Vec<_>>();
        let rate = operator
            .primary_mass()
            .iter()
            .map(|mass| 0.25 * mass)
            .collect::<Vec<_>>();
        let flux = Point2::new(0.4, -0.2);
        let complementary = vec![flux; operator.complementary_degrees_of_freedom()];
        let sampled = stencil.sample(&primary, &rate, &complementary).unwrap();
        let field = stencil.sample_inverses[0].apply(flux);
        assert!((sampled.primary - 2.0).abs() < 1.0e-12);
        assert!((sampled.primary_rate - 0.25).abs() < 1.0e-12);
        assert!((sampled.complementary - field).norm() < 1.0e-12);
        assert!(sampled.energy_density > 0.0);
        assert!(
            (sampled.energy_flow - rotate(field) * (2.0 * operator.orientation())).norm() < 1.0e-12
        );
    }

    #[test]
    fn energy_breakdown_matches_the_state_oracle_including_auxiliaries() {
        let (operator, _) = fixture();
        let primary = operator
            .node_points()
            .iter()
            .map(|point| 0.2 + 0.1 * point.x)
            .collect::<Vec<_>>();
        let potential = operator
            .node_points()
            .iter()
            .map(|point| 0.03 * point.y)
            .collect::<Vec<_>>();
        let state = CanonicalWaveState::from_primary_and_potential(
            &operator,
            0.5 * operator.maximum_time_step(),
            &primary,
            &potential,
        )
        .unwrap();
        let auxiliary = match state.auxiliaries() {
            crate::CanonicalAuxiliaryState::None => vec![],
            crate::CanonicalAuxiliaryState::Linear(values) => values
                .thin_gap_jump()
                .iter()
                .chain(values.outgoing_z())
                .copied()
                .collect(),
        };
        let breakdown = canonical_energy_breakdown(
            &operator,
            state.primary_flux(),
            state.complementary_flux(),
            &auxiliary,
        )
        .unwrap();
        assert!((breakdown.total() - state.energy(&operator).unwrap()).abs() < 1.0e-12);
    }
}
