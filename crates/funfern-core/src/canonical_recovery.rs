//! The complementary field a point probe reads, recovered from the samples
//! around it.
//!
//! A point's field was the quadratic through its element's six samples. They
//! sit 0.09 or more in from every edge in barycentric terms, so a point nearer
//! an edge than that was read by extrapolation, up to 1.8% of the amplitude
//! off. The samples' own error alternates from element to element, and a fit
//! over several elements cancels much of it. This is Zienkiewicz and Zhu's
//! patch recovery: each vertex has a least-squares quadratic over the samples
//! of its elements of one medium, and a point reads the quadratic through its
//! element's corner values of those fits and, at each edge's midpoint, the
//! mean of the edge's two. A vertex on another medium's edge fits over a
//! second ring of elements, so that its fit does not reach into the jump from
//! one side only. Over uniform, graded and sliver meshes, walls, slits,
//! interfaces and the gallery it is nowhere worse than the element's own fit,
//! 2.2 to 6 times better in the bulk, 1.6 to 4.7 at interfaces and 1.13 to 4
//! at walls (`docs/spikes/funfern-point-recovery-spike.md`).
//!
//! The reading is linear in the samples' physical fields, so its weights are
//! fixed when a stencil is built. Each sample's field is still its own
//! inverse applied at its own site, as `canonical_consumer` requires; only
//! the combination of those fields changes.
//!
//! A line probe keeps its element's own fit. It integrates its points, and
//! that fit's errors alternate from element to element and cancel along the
//! line, where the recovered field's smaller error is smooth and adds up. On
//! the travelling packet the recovered field read a segment's flux better at
//! 10 to 20 elements to the wavelength, and ten times worse at 40; on the
//! `observables` channel it read the energy down the wall 2.0% high, where
//! the own fit reads 1.5%.

use std::collections::{BTreeMap, HashMap};

use crate::{
    CanonicalWaveOperator, MaterialFrame, MaterialId, Point2, SymmetricTensor2, TopologyWaveModel,
    TriMesh, WaveError, canonical_consumer::solve_six,
};

const LOCAL_NODES: usize = 7;
const COMPLEMENTARY_SAMPLES: usize = 6;

/// One sample a reading combines: its index, its weight, and the
/// constitutive inverse the solver owns there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecoveredSample {
    pub sample: u32,
    pub weight: f64,
    pub inverse: SymmetricTensor2,
}

/// A generation's elements around each vertex, by medium, from which a point
/// probe's reading is fitted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComplementaryRecovery {
    /// Each element's medium, an index. Elements whose materials have the
    /// same coefficients and frame share one, whatever the materials are
    /// called.
    element_media: Vec<u32>,
    /// The elements with a corner at each node:
    /// `node_elements[node_offsets[n]..node_offsets[n + 1]]`.
    node_offsets: Vec<u32>,
    node_elements: Vec<u32>,
    /// Whether each node ends an edge between two media.
    interface: Vec<bool>,
}

impl ComplementaryRecovery {
    /// Media from each element's region: its material and frame, with the
    /// material's id, name and colour left out. A material that differs in
    /// anything else, a temporal law included, is another medium, so a
    /// driven region's fits stop at its edge even where its base
    /// coefficients match its neighbour's.
    pub fn new(
        mesh: &TriMesh,
        element_nodes: &[[u32; LOCAL_NODES]],
        node_count: usize,
        model: TopologyWaveModel<'_>,
    ) -> Result<Self, WaveError> {
        if mesh.triangles.len() != element_nodes.len() {
            return Err(WaveError::InvalidMesh(
                "the recovery's mesh does not match the canonical elements",
            ));
        }
        let mut media = Vec::new();
        let mut region_media = BTreeMap::new();
        let mut element_media = Vec::with_capacity(element_nodes.len());
        for triangle in &mesh.triangles {
            let index = match region_media.get(&triangle.region) {
                Some(index) => *index,
                None => {
                    let region = model.region(triangle.region).ok_or(WaveError::InvalidMesh(
                        "an element's region is not in the model",
                    ))?;
                    let mut material = model
                        .material(region.material)
                        .ok_or(WaveError::InvalidMesh(
                            "an element's material is not in the model",
                        ))?
                        .clone();
                    material.id = MaterialId(0);
                    material.name.clear();
                    material.color = [0; 3];
                    let medium: (_, MaterialFrame) = (material, region.frame);
                    let index = match media.iter().position(|known| *known == medium) {
                        Some(index) => index,
                        None => {
                            media.push(medium);
                            media.len() - 1
                        }
                    } as u32;
                    region_media.insert(triangle.region, index);
                    index
                }
            };
            element_media.push(index);
        }

        let mut node_offsets = vec![0_u32; node_count + 1];
        for nodes in element_nodes {
            for node in &nodes[..3] {
                let slot =
                    node_offsets
                        .get_mut(*node as usize + 1)
                        .ok_or(WaveError::InvalidMesh(
                            "an element's corner is out of range",
                        ))?;
                *slot += 1;
            }
        }
        for node in 0..node_count {
            node_offsets[node + 1] += node_offsets[node];
        }
        let mut filled = node_offsets.clone();
        let mut node_elements = vec![0_u32; node_offsets[node_count] as usize];
        for (element, nodes) in element_nodes.iter().enumerate() {
            for node in &nodes[..3] {
                let slot = &mut filled[*node as usize];
                node_elements[*slot as usize] = element as u32;
                *slot += 1;
            }
        }

        let mut interface = vec![false; node_count];
        let mut edges = HashMap::<(u32, u32), u32>::new();
        for (element, nodes) in element_nodes.iter().enumerate() {
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                let key = (nodes[a].min(nodes[b]), nodes[a].max(nodes[b]));
                match edges.insert(key, element as u32) {
                    Some(other) if element_media[other as usize] != element_media[element] => {
                        interface[key.0 as usize] = true;
                        interface[key.1 as usize] = true;
                    }
                    _ => {}
                }
            }
        }
        Ok(Self {
            element_media,
            node_offsets,
            node_elements,
            interface,
        })
    }

    /// The samples a reading at `barycentric` in `element` combines, with
    /// their weights, in sample order.
    pub fn weights(
        &self,
        operator: &CanonicalWaveOperator,
        element: usize,
        barycentric: [f64; 3],
    ) -> Result<Vec<RecoveredSample>, WaveError> {
        let medium = *self
            .element_media
            .get(element)
            .ok_or(WaveError::InvalidMesh(
                "the point's element is not recovered",
            ))?;
        let corners = operator
            .element_nodes()
            .get(element)
            .map(|nodes| [nodes[0], nodes[1], nodes[2]])
            .ok_or(WaveError::InvalidMesh(
                "the point's element is not recovered",
            ))?;
        let points = corners.map(|node| operator.node_points()[node as usize]);
        let mut fits = Vec::with_capacity(3);
        for node in corners {
            fits.push(self.star_fit(operator, node, medium)?);
        }
        // The quadratic through the corners' fits there and, at each edge's
        // midpoint, the mean of its two ends' fits: `4 l_i l_j` on the mean,
        // so `2 l_i l_j` on each.
        let mut weights = BTreeMap::<u32, f64>::new();
        for local in 0..3 {
            let next = (local + 1) % 3;
            let l = barycentric[local];
            let middle = (points[local] + points[next]) * 0.5;
            let edge = 2.0 * l * barycentric[next];
            fits[local].add(points[local], l * (2.0 * l - 1.0), &mut weights)?;
            fits[local].add(middle, edge, &mut weights)?;
            fits[next].add(middle, edge, &mut weights)?;
        }
        let samples = operator.constitutive_samples();
        weights
            .into_iter()
            .map(|(sample, weight)| {
                Ok(RecoveredSample {
                    sample,
                    weight,
                    inverse: samples[sample as usize].complementary_inverse,
                })
            })
            .collect()
    }

    /// The least-squares quadratic over the samples of `node`'s elements of
    /// `medium`, and of their neighbours of it where the node is on an
    /// interface.
    fn star_fit(
        &self,
        operator: &CanonicalWaveOperator,
        node: u32,
        medium: u32,
    ) -> Result<StarFit, WaveError> {
        let of_medium = |node: u32| {
            let range = self.node_offsets[node as usize] as usize
                ..self.node_offsets[node as usize + 1] as usize;
            self.node_elements[range]
                .iter()
                .copied()
                .filter(move |element| self.element_media[*element as usize] == medium)
        };
        let star = of_medium(node).collect::<Vec<_>>();
        if star.is_empty() {
            return Err(WaveError::InvalidMesh(
                "a corner has no element of its medium",
            ));
        }
        let nodes = operator.element_nodes();
        let points = operator.node_points();
        let size = |element: u32| {
            let [a, b, c] = [0, 1, 2].map(|local| points[nodes[element as usize][local] as usize]);
            (0.5 * (b - a).cross(c - a)).sqrt()
        };
        let scale = star.iter().map(|element| size(*element)).sum::<f64>() / star.len() as f64;
        let mut elements = star;
        if self.interface[node as usize] {
            let ring = elements
                .iter()
                .flat_map(|element| nodes[*element as usize][..3].iter().copied())
                .flat_map(of_medium)
                .collect::<Vec<_>>();
            elements.extend(ring);
            elements.sort_unstable();
            elements.dedup();
        }
        let center = points[node as usize];
        let samples = operator.constitutive_samples();
        let mut rows = Vec::with_capacity(COMPLEMENTARY_SAMPLES * elements.len());
        for element in elements {
            for sample in
                COMPLEMENTARY_SAMPLES as u32 * element..COMPLEMENTARY_SAMPLES as u32 * (element + 1)
            {
                let site = samples
                    .get(sample as usize)
                    .filter(|site| site.element == element);
                let site = site.ok_or(WaveError::InvalidMesh(
                    "the canonical samples are not six to an element in order",
                ))?;
                rows.push((sample, monomials(site.point, center, scale)));
            }
        }
        let mut normal = [[0.0; COMPLEMENTARY_SAMPLES]; COMPLEMENTARY_SAMPLES];
        for (_, row) in &rows {
            for i in 0..COMPLEMENTARY_SAMPLES {
                for j in 0..COMPLEMENTARY_SAMPLES {
                    normal[i][j] += row[i] * row[j];
                }
            }
        }
        Ok(StarFit {
            center,
            scale,
            rows,
            normal,
        })
    }
}

/// One vertex's fit: its value anywhere is `Σ_s (r_s · N⁻¹ m(p)) f_s`, `r_s`
/// sample `s`'s monomials, `N` their normal matrix and `m(p)` the point's.
struct StarFit {
    center: Point2,
    scale: f64,
    rows: Vec<(u32, [f64; COMPLEMENTARY_SAMPLES])>,
    normal: [[f64; COMPLEMENTARY_SAMPLES]; COMPLEMENTARY_SAMPLES],
}

impl StarFit {
    /// Adds `factor` times the weights the fit's value at `point` gives its
    /// samples.
    fn add(
        &self,
        point: Point2,
        factor: f64,
        weights: &mut BTreeMap<u32, f64>,
    ) -> Result<(), WaveError> {
        if factor == 0.0 {
            return Ok(());
        }
        let solved = solve_six(self.normal, monomials(point, self.center, self.scale))?;
        for (sample, row) in &self.rows {
            let weight = row.iter().zip(solved).map(|(m, z)| m * z).sum::<f64>();
            *weights.entry(*sample).or_default() += factor * weight;
        }
        Ok(())
    }
}

/// The quadratic monomials at `point`, about `center` over `scale`.
fn monomials(point: Point2, center: Point2, scale: f64) -> [f64; COMPLEMENTARY_SAMPLES] {
    let x = (point.x - center.x) / scale;
    let y = (point.y - center.y) / scale;
    [1.0, x, y, x * x, x * y, y * y]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_scenes::{REFLECTING_WALLS, add_inclusion, prepare, set_spans};
    use crate::{CurveId, Material, Region, RegionId, ScalarField, SpanBehavior, TopologyScene};

    /// Adds region `id` of its own material: the default medium, its
    /// stiffness slot `stiffness`.
    fn add_medium(scene: &mut TopologyScene, id: u64, stiffness: f64) -> RegionId {
        let mut material = Material::default_medium();
        material.id = MaterialId(id);
        material.name = format!("medium {id}");
        material.stiffness = ScalarField::constant(stiffness);
        scene.materials.push(material);
        scene.regions.push(Region {
            id: RegionId(id),
            material: MaterialId(id),
            frame: MaterialFrame::world(),
        });
        RegionId(id)
    }

    /// A quadratic field, one per side.
    fn quadratic(side: usize, point: Point2) -> Point2 {
        let [a, b, c, d, e, f] = [
            [0.3, -1.1, 0.7, 0.9, -0.4, 1.3],
            [-0.8, 0.5, 1.2, -0.6, 0.8, 0.2],
            [1.4, 0.2, -0.9, 0.3, 1.1, -0.7],
        ][side];
        let (x, y) = (point.x, point.y);
        let value = a + b * x + c * y + d * x * x + e * x * y + f * y * y;
        Point2::new(value, 0.5 * value - x * y)
    }

    /// A different quadratic outside, inside a drum walled off by a
    /// reflecting rim but of the same medium, and inside a disc of a stiffer
    /// one, is read exactly anywhere in every element: the recovery
    /// reproduces a quadratic, and fits neither across a wall, whose two
    /// sides hold different nodes, nor across another medium's edge, though
    /// beside one it reaches a second ring. The element's own six samples
    /// would read this exactly too; a fit that crossed would not.
    #[test]
    fn a_quadratic_on_each_side_is_read_exactly() {
        let mut scene = TopologyScene {
            outer_boundaries: REFLECTING_WALLS,
            ..TopologyScene::default()
        };
        let drum = add_medium(&mut scene, 2, 1.0);
        let disc = add_medium(&mut scene, 3, 4.0);
        add_inclusion(&mut scene, CurveId(1), Point2::new(-0.45, 0.3), 0.3, drum);
        set_spans(&mut scene, CurveId(1), SpanBehavior::REFLECTING);
        add_inclusion(&mut scene, CurveId(2), Point2::new(0.4, -0.3), 0.35, disc);
        let prepared = prepare(scene, 0.15);
        let operator = &prepared.canonical;
        let side = |element: usize| match prepared.mesh.triangles[element].region {
            region if region == drum => 1,
            region if region == disc => 2,
            _ => 0,
        };
        let recovery = operator.recovery();
        assert!(recovery.interface.iter().any(|on| *on));
        let samples = operator.constitutive_samples();
        let positions = [
            [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0],
            [0.498, 0.498, 0.004],
            [0.0, 0.5, 0.5],
            [0.05, 0.05, 0.9],
            [1.0, 0.0, 0.0],
            [0.6, 0.25, 0.15],
        ];
        let mut sides = [0; 3];
        for (element, nodes) in operator.element_nodes().iter().enumerate() {
            sides[side(element)] += 1;
            let corners = [0, 1, 2].map(|local| operator.node_points()[nodes[local] as usize]);
            for barycentric in positions {
                let point = corners[0] * barycentric[0]
                    + corners[1] * barycentric[1]
                    + corners[2] * barycentric[2];
                let read = recovery
                    .weights(operator, element, barycentric)
                    .unwrap()
                    .iter()
                    .fold(Point2::default(), |sum, entry| {
                        let site = &samples[entry.sample as usize];
                        sum + quadratic(side(site.element as usize), site.point) * entry.weight
                    });
                let error = (read - quadratic(side(element), point)).norm();
                assert!(
                    error < 1.0e-10,
                    "element {element} at {barycentric:?} reads {error:.2e} off"
                );
            }
        }
        assert!(sides.iter().all(|count| *count > 20), "{sides:?}");
    }
}
