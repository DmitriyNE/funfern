//! Can a point probe read the transverse field closer to the truth by
//! fitting the samples of the elements around it, rather than only its own?
//! (`docs/spikes/funfern-point-recovery-spike.md`.)
//!
//! A point's transverse field is today the quadratic through the six
//! constitutive samples of the element holding it, each sample's physical
//! field `J b` taken where the solver owns `J`. The samples sit 0.09 or more
//! in from every edge, so a point nearer an edge than that is read by
//! extrapolation. The study reads the same field by nine rules:
//! - `own`: today's quadratic through the element's six samples;
//! - `averaged`: the error estimator's recovery, each vertex the
//!   area-weighted mean of its elements' own fits there, read linearly
//!   across the element;
//! - `edge patch`: a least-squares quadratic over the element and those
//!   sharing an edge with it, up to 24 samples;
//! - `spr`: Zienkiewicz and Zhu's patch recovery, a least-squares quadratic
//!   over each vertex's elements read at the vertex and, averaged between an
//!   edge's two, at its midpoint, read quadratically across the element;
//! - `vertex patch`: one least-squares quadratic over every element touching
//!   the element's vertices;
//! - `edge+ring` and `spr+ring`: `edge patch` and `spr` with a second ring of
//!   elements where the patch meets a wall, a slit or an interface, so that
//!   it does not reach only one way;
//! - `edge+iring` and `spr+iring`: the same, the second ring only where the
//!   patch meets another material.
//!
//! None reaches past an element of another material (or frame), and none
//! across a slit, whose two sides hold different nodes.
//!
//! Each is read at up to three levels, to tell the reconstruction's error
//! from the samples' own:
//! - L1: the exact field at each sample;
//! - L2: the field of the exact potential's interpolant, `compatible_flux`;
//! - L3: the solver's own samples, from a run whose solution is known.
//!
//! Errors are relative to the field's amplitude, by where the point lies in
//! its element (within 0.01 of an edge in barycentric terms, between that
//! and the samples' 0.0916, or among the samples) and by its element: one
//! with an edge on a wall, a slit or a hole ("wall"), one with an edge on
//! another material ("interface"), or neither ("bulk").
//!
//! `cargo run --release -p funfern-app --example point_recovery_spike`, or
//! with `--only <part>` for one of `box`, `channel`, `packet`, `meshes`,
//! `interface` and `gallery`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

use funfern_core::*;

/// Where each element is read, in barycentric coordinates: its centroid,
/// 0.004 off each edge's middle, 0.06 off it (past the samples, short of the
/// edge), near each vertex, on each edge's middle, and among the samples.
const POSITIONS: [[f64; 3]; 16] = [
    [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0],
    [0.498, 0.498, 0.004],
    [0.004, 0.498, 0.498],
    [0.498, 0.004, 0.498],
    [0.47, 0.47, 0.06],
    [0.06, 0.47, 0.47],
    [0.47, 0.06, 0.47],
    [0.9, 0.05, 0.05],
    [0.05, 0.9, 0.05],
    [0.05, 0.05, 0.9],
    [0.5, 0.5, 0.0],
    [0.0, 0.5, 0.5],
    [0.5, 0.0, 0.5],
    [0.6, 0.25, 0.15],
    [0.15, 0.6, 0.25],
    [0.25, 0.15, 0.6],
];

/// The samples' smallest barycentric coordinate, the stiffness rule's
/// vertex orbit.
const SAMPLE_MARGIN: f64 = 0.091_576_213_509_771;

const METHODS: [&str; 9] = [
    "own",
    "averaged",
    "edge patch",
    "spr",
    "vertex patch",
    "edge+ring",
    "spr+ring",
    "edge+iring",
    "spr+iring",
];
const RULES: usize = METHODS.len();
/// The rules the summary judges, against `own`.
const JUDGED: [usize; 6] = [2, 3, 5, 6, 7, 8];

const ELEMENTS: [&str; 3] = ["bulk", "wall", "interface"];
const PLACES: [&str; 3] = ["at an edge", "past the samples", "among them"];

const DRIVE: TimeSignal = TimeSignal::Pulsed {
    offset: 0.0,
    amplitude: 0.5,
    frequency_hz: 2.0,
    phase_radians: 0.0,
    envelope: PulseEnvelope::FlatTop {
        duration: 8.0,
        edge: 0.5,
    },
    start: 0.0,
    repeat: 0.0,
};
const UPSTREAM: Point2 = Point2::new(-0.45, 0.61);
const DOWNSTREAM: Point2 = Point2::new(0.35, 0.62);
const CHANNEL: DomainRect = DomainRect {
    min_x: -3.0,
    max_x: 3.0,
    min_y: -0.5,
    max_y: 0.5,
};

fn tm() -> PhysicsModel {
    PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    }
}

/// A scene meshed and assembled as the app's runtime prepares it.
struct Prepared {
    scene: TopologyScene,
    mesh: TriMesh,
    canonical: CanonicalWaveOperator,
    quadratic: QuadraticWaveOperator,
}

fn prepare(scene: TopologyScene, edge: f64) -> Result<Prepared, String> {
    let domain = scene.geometry.domain;
    let options = MeshingOptions {
        target_edge_length: edge,
        ..MeshingOptions::default()
    }
    .sized_for_area(domain.width() * domain.height());
    let compiled = scene
        .compile(1)
        .map_err(|issue| format!("scene: {issue}"))?;
    let plan = compiled
        .plan
        .coarsened(&compiled.topology, AtomCoarsening::from_meshing(options))
        .map_err(|issue| format!("plan: {issue}"))?;
    let mesh = mesh_topology_plan(&plan, 1, options).map_err(|error| format!("mesh: {error}"))?;
    let base = TopologyWaveModel::from_topology_scene(&scene)
        .to_owned()
        .without_temporal_laws();
    let quadratic = QuadraticWaveOperator::assemble_topology(&mesh, &plan, base.as_model())
        .map_err(|error| format!("scalar operator: {error}"))?;
    let canonical = CanonicalWaveOperator::compile(&mesh, &quadratic, base.as_model(), 1)
        .map_err(|error| format!("canonical operator: {error}"))?;
    Ok(Prepared {
        scene,
        mesh,
        canonical,
        quadratic,
    })
}

/// The quadratic monomials at `point`, about `center` over `scale`.
fn monomials(point: Point2, center: Point2, scale: f64) -> [f64; 6] {
    let x = (point.x - center.x) / scale;
    let y = (point.y - center.y) / scale;
    [1.0, x, y, x * x, x * y, y * y]
}

fn invert6(mut matrix: [[f64; 6]; 6]) -> [[f64; 6]; 6] {
    let mut inverse = [[0.0; 6]; 6];
    for (row, values) in inverse.iter_mut().enumerate() {
        values[row] = 1.0;
    }
    for pivot in 0..6 {
        let best = (pivot..6)
            .max_by(|a, b| matrix[*a][pivot].abs().total_cmp(&matrix[*b][pivot].abs()))
            .unwrap();
        assert!(matrix[best][pivot].abs() > 1.0e-12, "a singular fit");
        matrix.swap(pivot, best);
        inverse.swap(pivot, best);
        let scale = matrix[pivot][pivot];
        for column in 0..6 {
            matrix[pivot][column] /= scale;
            inverse[pivot][column] /= scale;
        }
        for row in 0..6 {
            if row != pivot {
                let factor = matrix[row][pivot];
                if factor != 0.0 {
                    for column in 0..6 {
                        matrix[row][column] -= factor * matrix[pivot][column];
                        inverse[row][column] -= factor * inverse[pivot][column];
                    }
                }
            }
        }
    }
    inverse
}

/// A least-squares quadratic through a set of samples: the coefficients are
/// `Σ_j columns_j f_j`, so a value anywhere is linear in the samples.
struct Fit {
    center: Point2,
    scale: f64,
    columns: Vec<(usize, [f64; 6])>,
}

impl Fit {
    fn new(points: &[Point2], samples: Vec<usize>, center: Point2, scale: f64) -> Self {
        let rows = samples
            .iter()
            .map(|sample| monomials(points[*sample], center, scale))
            .collect::<Vec<_>>();
        let mut normal = [[0.0; 6]; 6];
        for row in &rows {
            for i in 0..6 {
                for j in 0..6 {
                    normal[i][j] += row[i] * row[j];
                }
            }
        }
        let inverse = invert6(normal);
        let columns = samples
            .into_iter()
            .zip(rows)
            .map(|(sample, row)| {
                let mut column = [0.0; 6];
                for (i, value) in column.iter_mut().enumerate() {
                    *value = (0..6).map(|j| inverse[i][j] * row[j]).sum();
                }
                (sample, column)
            })
            .collect();
        Self {
            center,
            scale,
            columns,
        }
    }

    fn coefficients(&self, values: &[Point2]) -> [Point2; 6] {
        let mut coefficients = [Point2::default(); 6];
        for (sample, column) in &self.columns {
            for i in 0..6 {
                coefficients[i] = coefficients[i] + values[*sample] * column[i];
            }
        }
        coefficients
    }

    fn evaluate(&self, coefficients: &[Point2; 6], point: Point2) -> Point2 {
        monomials(point, self.center, self.scale)
            .iter()
            .zip(coefficients)
            .fold(Point2::default(), |sum, (m, c)| sum + *c * *m)
    }

    /// Adds `factor` times the weights a value at `point` takes.
    fn weights(&self, point: Point2, factor: f64, into: &mut HashMap<usize, f64>) {
        let m = monomials(point, self.center, self.scale);
        for (sample, column) in &self.columns {
            *into.entry(*sample).or_default() +=
                factor * (0..6).map(|i| m[i] * column[i]).sum::<f64>();
        }
    }
}

/// The mesh as the recovery sees it, and every fit the rules use.
struct Layout {
    orientation: f64,
    sample_points: Vec<Point2>,
    inverses: Vec<SymmetricTensor2>,
    corners: Vec<[u32; 3]>,
    node_points: Vec<Point2>,
    /// Each element's material and frame, as an index.
    groups: Vec<usize>,
    areas: Vec<f64>,
    /// 0 bulk, 1 wall, 2 interface.
    kinds: Vec<usize>,
    own: Vec<Fit>,
    edge_patch: Vec<Fit>,
    vertex_patch: Vec<Fit>,
    edge_ring: Vec<Fit>,
    edge_iring: Vec<Fit>,
    stars: BTreeMap<(u32, usize), Vec<usize>>,
    star_fits: BTreeMap<(u32, usize), Fit>,
    star_ring_fits: BTreeMap<(u32, usize), Fit>,
    star_iring_fits: BTreeMap<(u32, usize), Fit>,
}

impl Layout {
    fn new(prepared: &Prepared) -> Self {
        let operator = &prepared.canonical;
        let samples = operator.constitutive_samples();
        let sample_points = samples
            .iter()
            .map(|sample| sample.point)
            .collect::<Vec<_>>();
        let inverses = samples
            .iter()
            .map(|sample| sample.complementary_inverse)
            .collect::<Vec<_>>();
        let node_points = operator.node_points().to_vec();
        let corners = operator
            .element_nodes()
            .iter()
            .map(|nodes| [nodes[0], nodes[1], nodes[2]])
            .collect::<Vec<_>>();
        assert_eq!(corners.len(), prepared.mesh.triangles.len());
        assert_eq!(samples.len(), 6 * corners.len());
        for (element, triangle) in prepared.mesh.triangles.iter().enumerate() {
            for local in 0..3 {
                let mesh_point = prepared.mesh.vertices[triangle.vertices[local]].point;
                let node_point = node_points[corners[element][local] as usize];
                assert!((mesh_point - node_point).norm() < 1.0e-12);
            }
        }
        // A group is a material's coefficients in a frame: two regions of the
        // same medium fit together whatever their material is called.
        let mut laws = Vec::<(Material, MaterialFrame)>::new();
        let groups = prepared
            .mesh
            .triangles
            .iter()
            .map(|triangle| {
                let region = prepared
                    .scene
                    .region(triangle.region)
                    .expect("an element's region");
                let mut material = prepared
                    .scene
                    .material(region.material)
                    .expect("a region's material")
                    .clone();
                material.id = MaterialId(0);
                material.name.clear();
                material.color = [0; 3];
                let law = (material, region.frame);
                laws.iter()
                    .position(|known| *known == law)
                    .unwrap_or_else(|| {
                        laws.push(law);
                        laws.len() - 1
                    })
            })
            .collect::<Vec<_>>();
        let areas = corners
            .iter()
            .map(|corner| {
                let [a, b, c] = corner.map(|node| node_points[node as usize]);
                0.5 * (b - a).cross(c - a)
            })
            .collect::<Vec<_>>();
        assert!(areas.iter().all(|area| *area > 0.0));
        let mut edges = HashMap::<(u32, u32), Vec<usize>>::new();
        for (element, corner) in corners.iter().enumerate() {
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                let key = (corner[a].min(corner[b]), corner[a].max(corner[b]));
                edges.entry(key).or_default().push(element);
            }
        }
        let mut neighbours = vec![vec![]; corners.len()];
        let mut kinds = vec![0; corners.len()];
        let mut open = BTreeSet::<u32>::new();
        // The elements and nodes on another material's edge.
        let mut facing = vec![false; corners.len()];
        let mut interface_nodes = BTreeSet::<u32>::new();
        for (key, elements) in &edges {
            match elements.as_slice() {
                [only] => {
                    kinds[*only] = 1;
                    open.extend([key.0, key.1]);
                }
                [a, b] if groups[*a] == groups[*b] => {
                    neighbours[*a].push(*b);
                    neighbours[*b].push(*a);
                }
                [a, b] => {
                    for element in [*a, *b] {
                        if kinds[element] == 0 {
                            kinds[element] = 2;
                        }
                        facing[element] = true;
                    }
                    open.extend([key.0, key.1]);
                    interface_nodes.extend([key.0, key.1]);
                }
                _ => panic!("an edge of three elements"),
            }
        }
        let mut stars = BTreeMap::<(u32, usize), Vec<usize>>::new();
        for (element, corner) in corners.iter().enumerate() {
            for node in corner {
                stars
                    .entry((*node, groups[element]))
                    .or_default()
                    .push(element);
            }
        }
        let samples_of = |elements: &[usize]| {
            elements
                .iter()
                .flat_map(|element| 6 * element..6 * element + 6)
                .collect::<Vec<_>>()
        };
        let centroid = |element: usize| {
            let [a, b, c] = corners[element].map(|node| node_points[node as usize]);
            (a + b + c) / 3.0
        };
        let size = |element: usize| areas[element].sqrt();
        let fit = |elements: &[usize], center: Point2, scale: f64| {
            Fit::new(&sample_points, samples_of(elements), center, scale)
        };
        let unique = |mut elements: Vec<usize>| {
            elements.sort_unstable();
            elements.dedup();
            elements
        };
        // Every element of a group touching `elements`' corners.
        let touching = |elements: &[usize], group: usize| {
            unique(
                elements
                    .iter()
                    .flat_map(|element| corners[*element])
                    .flat_map(|node| stars[&(node, group)].iter().copied())
                    .collect(),
            )
        };
        let mut own = Vec::with_capacity(corners.len());
        let mut edge_patch = Vec::with_capacity(corners.len());
        let mut vertex_patch = Vec::with_capacity(corners.len());
        let mut edge_ring = Vec::with_capacity(corners.len());
        let mut edge_iring = Vec::with_capacity(corners.len());
        for element in 0..corners.len() {
            let (center, scale) = (centroid(element), size(element));
            own.push(fit(&[element], center, scale));
            let mut patch = vec![element];
            patch.extend(&neighbours[element]);
            edge_patch.push(fit(&patch, center, scale));
            vertex_patch.push(fit(&touching(&[element], groups[element]), center, scale));
            let ring = unique(
                patch
                    .iter()
                    .flat_map(|near| {
                        std::iter::once(*near).chain(neighbours[*near].iter().copied())
                    })
                    .collect(),
            );
            edge_ring.push(fit(
                if kinds[element] == 0 { &patch } else { &ring },
                center,
                scale,
            ));
            edge_iring.push(fit(
                if facing[element] { &ring } else { &patch },
                center,
                scale,
            ));
        }
        let mut star_fits = BTreeMap::new();
        let mut star_ring_fits = BTreeMap::new();
        let mut star_iring_fits = BTreeMap::new();
        for (key, elements) in &stars {
            let scale =
                elements.iter().map(|element| size(*element)).sum::<f64>() / elements.len() as f64;
            let vertex = node_points[key.0 as usize];
            star_fits.insert(*key, fit(elements, vertex, scale));
            let ring = touching(elements, key.1);
            let pick = |wide: bool| if wide { &ring } else { elements };
            star_ring_fits.insert(*key, fit(pick(open.contains(&key.0)), vertex, scale));
            star_iring_fits.insert(
                *key,
                fit(pick(interface_nodes.contains(&key.0)), vertex, scale),
            );
        }
        Self {
            orientation: operator.orientation(),
            sample_points,
            inverses,
            corners,
            node_points,
            groups,
            areas,
            kinds,
            own,
            edge_patch,
            vertex_patch,
            edge_ring,
            edge_iring,
            stars,
            star_fits,
            star_ring_fits,
            star_iring_fits,
        }
    }

    fn corner_points(&self, element: usize) -> [Point2; 3] {
        self.corners[element].map(|node| self.node_points[node as usize])
    }

    fn centroid(&self, element: usize) -> Point2 {
        let [a, b, c] = self.corner_points(element);
        (a + b + c) / 3.0
    }

    fn point_at(&self, element: usize, barycentric: [f64; 3]) -> Point2 {
        let [a, b, c] = self.corner_points(element);
        a * barycentric[0] + b * barycentric[1] + c * barycentric[2]
    }

    fn barycentric(&self, element: usize, point: Point2) -> [f64; 3] {
        let [a, b, c] = self.corner_points(element);
        let area = (b - a).cross(c - a);
        let l1 = (point - a).cross(c - a) / area;
        let l2 = (b - a).cross(point - a) / area;
        [1.0 - l1 - l2, l1, l2]
    }

    fn locate(&self, point: Point2) -> usize {
        let low = |element: usize| {
            self.barycentric(element, point)
                .into_iter()
                .fold(f64::INFINITY, f64::min)
        };
        (0..self.corners.len())
            .max_by(|a, b| low(*a).total_cmp(&low(*b)))
            .unwrap()
    }

    /// The physical field `J b` at every sample.
    fn physical(&self, flux: &[Point2]) -> Vec<Point2> {
        flux.iter()
            .zip(&self.inverses)
            .map(|(flux, inverse)| inverse.apply(*flux))
            .collect()
    }

    /// The constitutive inverse of an element, its first sample's: constant
    /// over the element in every medium here but the gallery's.
    fn inverse(&self, element: usize) -> SymmetricTensor2 {
        self.inverses[6 * element]
    }

    fn smallest_angle(&self) -> f64 {
        (0..self.corners.len())
            .map(|element| {
                let p = self.corner_points(element);
                (0..3)
                    .map(|i| {
                        let a = p[(i + 1) % 3] - p[i];
                        let b = p[(i + 2) % 3] - p[i];
                        (a.dot(b) / (a.norm() * b.norm())).acos().to_degrees()
                    })
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(f64::INFINITY, f64::min)
    }
}

/// Every fit's coefficients for one set of sample values, from which each
/// rule reads any point.
struct Readout<'a> {
    layout: &'a Layout,
    own: Vec<[Point2; 6]>,
    edge_patch: Vec<[Point2; 6]>,
    vertex_patch: Vec<[Point2; 6]>,
    edge_ring: Vec<[Point2; 6]>,
    edge_iring: Vec<[Point2; 6]>,
    stars: BTreeMap<(u32, usize), [Point2; 6]>,
    star_rings: BTreeMap<(u32, usize), [Point2; 6]>,
    star_irings: BTreeMap<(u32, usize), [Point2; 6]>,
    averaged: BTreeMap<(u32, usize), Point2>,
}

impl<'a> Readout<'a> {
    fn new(layout: &'a Layout, values: &[Point2]) -> Self {
        let all = |fits: &[Fit]| {
            fits.iter()
                .map(|fit| fit.coefficients(values))
                .collect::<Vec<_>>()
        };
        let keyed = |fits: &BTreeMap<(u32, usize), Fit>| {
            fits.iter()
                .map(|(key, fit)| (*key, fit.coefficients(values)))
                .collect::<BTreeMap<_, _>>()
        };
        let own = all(&layout.own);
        let averaged = layout
            .stars
            .iter()
            .map(|(key, elements)| {
                let vertex = layout.node_points[key.0 as usize];
                let (sum, weight) =
                    elements
                        .iter()
                        .fold((Point2::default(), 0.0), |(sum, weight), element| {
                            let area = layout.areas[*element];
                            let value = layout.own[*element].evaluate(&own[*element], vertex);
                            (sum + value * area, weight + area)
                        });
                (*key, sum / weight)
            })
            .collect();
        Self {
            layout,
            edge_patch: all(&layout.edge_patch),
            vertex_patch: all(&layout.vertex_patch),
            edge_ring: all(&layout.edge_ring),
            edge_iring: all(&layout.edge_iring),
            stars: keyed(&layout.star_fits),
            star_rings: keyed(&layout.star_ring_fits),
            star_irings: keyed(&layout.star_iring_fits),
            own,
            averaged,
        }
    }

    fn read(&self, method: usize, element: usize, barycentric: [f64; 3]) -> Point2 {
        let layout = self.layout;
        let point = layout.point_at(element, barycentric);
        let group = layout.groups[element];
        let keys = layout.corners[element].map(|node| (node, group));
        let recovered = |fits: &BTreeMap<(u32, usize), Fit>,
                         values: &BTreeMap<(u32, usize), [Point2; 6]>| {
            let corners = layout.corner_points(element);
            let star =
                |local: usize, at: Point2| fits[&keys[local]].evaluate(&values[&keys[local]], at);
            let mut value = Point2::default();
            for local in 0..3 {
                let l = barycentric[local];
                value = value + star(local, corners[local]) * (l * (2.0 * l - 1.0));
                let next = (local + 1) % 3;
                let middle = (corners[local] + corners[next]) * 0.5;
                let mean = (star(local, middle) + star(next, middle)) * 0.5;
                value = value + mean * (4.0 * l * barycentric[next]);
            }
            value
        };
        match method {
            0 => layout.own[element].evaluate(&self.own[element], point),
            1 => keys
                .iter()
                .zip(barycentric)
                .fold(Point2::default(), |sum, (key, weight)| {
                    sum + self.averaged[key] * weight
                }),
            2 => layout.edge_patch[element].evaluate(&self.edge_patch[element], point),
            3 => recovered(&layout.star_fits, &self.stars),
            4 => layout.vertex_patch[element].evaluate(&self.vertex_patch[element], point),
            5 => layout.edge_ring[element].evaluate(&self.edge_ring[element], point),
            6 => recovered(&layout.star_ring_fits, &self.star_rings),
            7 => layout.edge_iring[element].evaluate(&self.edge_iring[element], point),
            _ => recovered(&layout.star_iring_fits, &self.star_irings),
        }
    }
}

/// The sum of the absolute weights rule `method` gives the samples at
/// `barycentric`, the factor by which it can grow a rounding error in them,
/// and how many samples it reads.
fn weight_sum(
    layout: &Layout,
    method: usize,
    element: usize,
    barycentric: [f64; 3],
) -> (f64, usize) {
    let point = layout.point_at(element, barycentric);
    let group = layout.groups[element];
    let corners = layout.corner_points(element);
    let mut weights = HashMap::<usize, f64>::new();
    let recovered = |fits: &BTreeMap<(u32, usize), Fit>, weights: &mut HashMap<usize, f64>| {
        for local in 0..3 {
            let key = (layout.corners[element][local], group);
            let next = (local + 1) % 3;
            let next_key = (layout.corners[element][next], group);
            let l = barycentric[local];
            let middle = (corners[local] + corners[next]) * 0.5;
            let edge = 4.0 * l * barycentric[next];
            fits[&key].weights(corners[local], l * (2.0 * l - 1.0), weights);
            fits[&key].weights(middle, 0.5 * edge, weights);
            fits[&next_key].weights(middle, 0.5 * edge, weights);
        }
    };
    match method {
        0 => layout.own[element].weights(point, 1.0, &mut weights),
        1 => {
            for local in 0..3 {
                let key = (layout.corners[element][local], group);
                let elements = &layout.stars[&key];
                let total = elements.iter().map(|e| layout.areas[*e]).sum::<f64>();
                for other in elements {
                    layout.own[*other].weights(
                        corners[local],
                        barycentric[local] * layout.areas[*other] / total,
                        &mut weights,
                    );
                }
            }
        }
        2 => layout.edge_patch[element].weights(point, 1.0, &mut weights),
        3 => recovered(&layout.star_fits, &mut weights),
        4 => layout.vertex_patch[element].weights(point, 1.0, &mut weights),
        5 => layout.edge_ring[element].weights(point, 1.0, &mut weights),
        6 => recovered(&layout.star_ring_fits, &mut weights),
        7 => layout.edge_iring[element].weights(point, 1.0, &mut weights),
        _ => recovered(&layout.star_iring_fits, &mut weights),
    }
    (
        weights.values().map(|weight| weight.abs()).sum(),
        weights.len(),
    )
}

fn class(layout: &Layout, element: usize, barycentric: [f64; 3]) -> (usize, usize) {
    let low = barycentric.into_iter().fold(f64::INFINITY, f64::min);
    let place = if low < 0.01 {
        0
    } else if low < SAMPLE_MARGIN {
        1
    } else {
        2
    };
    (layout.kinds[element], place)
}

#[derive(Clone, Copy, Default)]
struct Stat {
    worst: f64,
    squares: f64,
    count: usize,
}

impl Stat {
    fn add(&mut self, error: f64) {
        self.worst = self.worst.max(error);
        self.squares += error * error;
        self.count += 1;
    }

    fn rms(&self) -> f64 {
        (self.squares / self.count.max(1) as f64).sqrt()
    }

    fn merge(&mut self, other: Stat) {
        self.worst = self.worst.max(other.worst);
        self.squares += other.squares;
        self.count += other.count;
    }
}

/// Worst and RMS error per rule, element kind and place, and the samples'
/// own per element kind.
#[derive(Clone)]
struct Errors {
    stats: [[[Stat; 3]; 3]; RULES],
    samples: [Stat; 3],
}

impl Errors {
    fn new() -> Self {
        Self {
            stats: [[[Stat::default(); 3]; 3]; RULES],
            samples: [Stat::default(); 3],
        }
    }

    /// Reads every element at every position against `exact`, which takes
    /// the element as well, for fields that differ by side.
    fn read(
        &mut self,
        layout: &Layout,
        values: &[Point2],
        exact: impl Fn(usize, Point2) -> Point2,
        amplitude: f64,
    ) {
        let readout = Readout::new(layout, values);
        for element in 0..layout.corners.len() {
            for barycentric in POSITIONS {
                let truth = exact(element, layout.point_at(element, barycentric));
                let (kind, place) = class(layout, element, barycentric);
                for method in 0..RULES {
                    let read = readout.read(method, element, barycentric);
                    self.stats[method][kind][place].add((read - truth).norm() / amplitude);
                }
            }
            let samples = 6 * element..6 * element + 6;
            for (value, point) in values[samples.clone()]
                .iter()
                .zip(&layout.sample_points[samples])
            {
                let error = (*value - exact(element, *point)).norm();
                self.samples[layout.kinds[element]].add(error / amplitude);
            }
        }
    }

    /// One rule's worst over every place for one element kind.
    fn worst(&self, method: usize, kind: usize) -> Stat {
        let mut stat = Stat::default();
        for place in 0..3 {
            stat.merge(self.stats[method][kind][place]);
        }
        stat
    }

    fn print(&self, title: &str) {
        println!("  {title}");
        print!("    samples themselves:");
        for (kind, name) in ELEMENTS.iter().enumerate() {
            if self.samples[kind].count > 0 {
                print!(
                    " {name} {:.2e} (rms {:.1e})",
                    self.samples[kind].worst,
                    self.samples[kind].rms()
                );
            }
        }
        println!();
        print!("    {:<28}", "worst (rms)");
        for method in METHODS {
            print!("{method:>19}");
        }
        println!();
        for (kind, element) in ELEMENTS.iter().enumerate() {
            for (place, name) in PLACES.iter().enumerate() {
                if self.stats[0][kind][place].count == 0 {
                    continue;
                }
                print!("    {:<28}", format!("{element}, {name}"));
                for method in 0..RULES {
                    let stat = self.stats[method][kind][place];
                    print!("{:>19}", format!("{:.2e} ({:.1e})", stat.worst, stat.rms()));
                }
                println!();
            }
        }
    }
}

/// What the summary keeps per case: each element kind's worst under each
/// rule.
struct Summary {
    rows: Vec<(String, Errors)>,
}

impl Summary {
    fn add(&mut self, name: impl Into<String>, errors: &Errors) {
        self.rows.push((name.into(), errors.clone()));
    }

    fn print(&self) {
        println!("== Summary: worst per element kind, today against the judged rules ==");
        print!("  {:<44}{:<10}{:>10}", "case", "elements", "own");
        for method in JUDGED {
            print!("{:>19}", METHODS[method]);
        }
        println!();
        for (name, errors) in &self.rows {
            for (kind, element) in ELEMENTS.iter().enumerate() {
                let own = errors.worst(0, kind);
                if own.count == 0 {
                    continue;
                }
                print!("  {name:<44}{element:<10}{:>10.2e}", own.worst);
                for method in JUDGED {
                    let stat = errors.worst(method, kind);
                    print!(
                        "{:>19}",
                        format!("{:.2e} x{:.2}", stat.worst, own.worst / stat.worst)
                    );
                }
                println!();
            }
        }
    }
}

/// A plane wave's potential `ψ = sin(k d·p + φ)/k`, whose gradient has
/// amplitude one.
#[derive(Clone, Copy)]
struct PlaneWave {
    k: f64,
    direction: Point2,
    phase: f64,
}

impl PlaneWave {
    fn oblique(k: f64) -> Self {
        Self {
            k,
            direction: Point2::new(0.3_f64.cos(), 0.3_f64.sin()),
            phase: 0.7,
        }
    }

    fn potential(&self, point: Point2) -> f64 {
        (self.k * self.direction.dot(point) + self.phase).sin() / self.k
    }

    /// The flux `b = orientation·R∇ψ`, `R` a quarter turn.
    fn flux(&self, orientation: f64, point: Point2) -> Point2 {
        let slope = (self.k * self.direction.dot(point) + self.phase).cos();
        let gradient = self.direction * slope;
        Point2::new(-gradient.y, gradient.x) * orientation
    }
}

/// L2 of `wave`: the physical field of its interpolant's flux against
/// `J·b`, each element's `J` its own.
fn second_level(prepared: &Prepared, layout: &Layout, wave: PlaneWave) -> Errors {
    let potential = layout
        .node_points
        .iter()
        .map(|point| wave.potential(*point))
        .collect::<Vec<_>>();
    let flux = prepared
        .canonical
        .compatible_flux(&potential)
        .expect("compatible flux");
    let mut errors = Errors::new();
    errors.read(
        layout,
        &layout.physical(&flux),
        |element, point| {
            layout
                .inverse(element)
                .apply(wave.flux(layout.orientation, point))
        },
        1.0,
    );
    errors
}

/// L1 of `wave`: the exact physical field at every sample.
fn first_level(layout: &Layout, wave: PlaneWave) -> Errors {
    let exact = |element: usize, point: Point2| {
        layout
            .inverse(element)
            .apply(wave.flux(layout.orientation, point))
    };
    let values = (0..layout.sample_points.len())
        .map(|sample| exact(sample / 6, layout.sample_points[sample]))
        .collect::<Vec<_>>();
    let mut errors = Errors::new();
    errors.read(layout, &values, exact, 1.0);
    errors
}

/// Each rule's largest weight sum, per element kind.
fn weight_sums(layout: &Layout) {
    let mut worst = [[0.0_f64; 3]; RULES];
    let mut reads = [(0_usize, 0_usize); RULES];
    let mut points = 0;
    for element in 0..layout.corners.len() {
        for barycentric in POSITIONS {
            points += 1;
            for method in 0..RULES {
                let kind = layout.kinds[element];
                let (sum, count) = weight_sum(layout, method, element, barycentric);
                worst[method][kind] = worst[method][kind].max(sum);
                reads[method].0 = reads[method].0.max(count);
                reads[method].1 += count;
            }
        }
    }
    println!("  weight sums bulk/wall/interface, samples read most/mean:");
    for method in 0..RULES {
        println!(
            "    {:<14}{:.2}/{:.2}/{:.2}  {}/{:.1}",
            METHODS[method],
            worst[method][0],
            worst[method][1],
            worst[method][2],
            reads[method].0,
            reads[method].1 as f64 / points as f64
        );
    }
}

fn describe(name: &str, prepared: &Prepared, layout: &Layout) {
    let count = |kind: usize| layout.kinds.iter().filter(|k| **k == kind).count();
    println!(
        "{name}: {} vertices, {} elements ({} wall, {} interface), {} DOFs, smallest angle {:.1}°",
        prepared.mesh.vertices.len(),
        layout.corners.len(),
        count(1),
        count(2),
        prepared.canonical.degrees_of_freedom(),
        layout.smallest_angle()
    );
}

/// The default medium in TM on `domain`, its walls reflecting.
fn box_scene(domain: DomainRect) -> TopologyScene {
    let mut scene = TopologyScene {
        physics: tm(),
        outer_boundaries: OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting),
        ..TopologyScene::default()
    };
    scene.geometry.domain = domain;
    scene
}

fn curve(id: u64, spline: CurveSpline, behavior: SpanBehavior) -> TopologyCurve {
    let spans = (0..spline.span_count() as u64)
        .map(|index| CurveSpan {
            id: CurveSpanId(10 * id + index),
            behavior,
        })
        .collect();
    TopologyCurve::new(CurveId(id), spline, spans).unwrap()
}

/// A rounded closed curve whose inside is `region`, or a hole where `None`.
fn add_closed(
    scene: &mut TopologyScene,
    id: u64,
    center: Point2,
    radius: f64,
    region: Option<RegionId>,
) {
    let behavior = if region.is_some() {
        SpanBehavior::Transmitting
    } else {
        SpanBehavior::REFLECTING
    };
    let curve = curve(
        id,
        CurveSpline::Closed(PeriodicCubicSpline::rounded(center, radius)),
        behavior,
    );
    let [start, end] = curve.spline.span_bounds(0).unwrap();
    scene.face_assignments.push(AuthoredFaceAssignment {
        anchor: FaceAnchor::Curve {
            curve: curve.id,
            span: curve.spans[0].id,
            side: CurveTraceSide::Left,
            parameter: 0.5 * (start + end),
        },
        region,
    });
    scene.geometry.curves.push(curve);
}

/// The default medium, its stiffness slot `stiffness` where given, as region
/// `region`.
fn add_medium(scene: &mut TopologyScene, region: RegionId, stiffness: Option<f64>) {
    let mut material = Material::default_medium();
    material.id = MaterialId(region.0);
    material.name = format!("medium {}", region.0);
    if let Some(stiffness) = stiffness {
        material.stiffness = ScalarField::constant(stiffness);
    }
    scene.materials.push(material);
    scene.regions.push(Region {
        id: region,
        material: material_id(region),
        frame: MaterialFrame::world(),
    });
}

fn material_id(region: RegionId) -> MaterialId {
    MaterialId(region.0)
}

/// A straight curve across the domain at `x`, welded to the bottom and top
/// walls, its two faces `left` and `right`.
fn add_divider(
    scene: &mut TopologyScene,
    x: f64,
    behavior: SpanBehavior,
    left: RegionId,
    right: RegionId,
) {
    let domain = scene.geometry.domain;
    let spline = OpenCubicSpline::polyline(vec![
        Point2::new(x, domain.min_y),
        Point2::new(x, domain.max_y),
    ])
    .unwrap();
    let mut divider = curve(5, CurveSpline::Open(spline), behavior);
    let last = divider.nodes.len() - 1;
    divider.nodes[0].vertex = Some(TopologyVertexId(5));
    divider.nodes[last].vertex = Some(TopologyVertexId(6));
    let fraction = (x - domain.min_x) / domain.width();
    scene.geometry.vertices.push(TopologyVertex {
        id: TopologyVertexId(5),
        location: TopologyVertexLocation::Outer {
            side: OuterSide::Bottom,
            fraction,
        },
    });
    scene.geometry.vertices.push(TopologyVertex {
        id: TopologyVertexId(6),
        location: TopologyVertexLocation::Outer {
            side: OuterSide::Top,
            fraction: 1.0 - fraction,
        },
    });
    scene.geometry.curves.push(divider);
    scene.geometry.synchronize_vertices().unwrap();
    scene.face_assignments = vec![
        AuthoredFaceAssignment {
            anchor: FaceAnchor::Outer {
                side: OuterSide::Bottom,
                fraction: 0.5 * fraction,
            },
            region: Some(left),
        },
        AuthoredFaceAssignment {
            anchor: FaceAnchor::Outer {
                side: OuterSide::Bottom,
                fraction: 0.5 * (1.0 + fraction),
            },
            region: Some(right),
        },
    ];
}

/// The `observables` fixture's channel: the cavity made TM, its left side
/// driven by a 2 Hz burst, its right a first-order outgoing wall, a circle of
/// the same medium transmitting inside it.
fn channel_scene(radius: f64) -> TopologyScene {
    let mut scene = box_scene(DomainRect::UNIT);
    scene.regions.push(Region {
        id: RegionId(2),
        material: DEFAULT_MATERIAL,
        frame: MaterialFrame::world(),
    });
    add_closed(
        &mut scene,
        1,
        Point2::new(0.3, -0.35),
        radius,
        Some(RegionId(2)),
    );
    let mut walls = OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
    walls.sides[OuterSide::Left.index()] = OuterBoundaryCondition::Dirichlet { signal: DRIVE };
    walls.sides[OuterSide::Right.index()] = OuterBoundaryCondition::FirstOrderOutgoing;
    scene.outer_boundaries = walls;
    scene
}

/// L3 on the channel: the plane wave `u = g(t − (x + 1))` and its transverse
/// field `orientation·(0, −u)`, read at every element a hundred times over
/// the 8 s run, and at the fixture's two points every step.
fn channel_run(prepared: &Prepared, layout: &Layout, report_points: bool) -> Errors {
    let operator = &prepared.canonical;
    let forcing =
        CanonicalForcing::from_legacy_boundaries(operator, &prepared.quadratic, SOURCE_ANCHOR_TIME)
            .expect("forcing");
    let dt = operator.recommended_time_step();
    let steps = (8.0 / dt).round() as usize;
    let mut state = CanonicalWaveState::zero(operator, dt).expect("state");
    let probes = [UPSTREAM, DOWNSTREAM].map(|point| {
        let element = layout.locate(point);
        (point, element, layout.barycentric(element, point))
    });
    let exact = |point: Point2, t: f64| {
        let u = DRIVE.value(t - (point.x + 1.0));
        Point2::new(0.0, -u) * layout.orientation
    };
    let mut errors = Errors::new();
    let mut magnitude = [[0.0_f64; RULES]; 2];
    let mut bias = [[0.0_f64; RULES]; 2];
    let every = (steps / 100).max(1);
    for step in 1..=steps {
        state.step_with_forcing(operator, &forcing).expect("step");
        let t = state.time();
        let values = layout.physical(state.complementary_flux());
        if step % every == 0 {
            errors.read(layout, &values, |_, point| exact(point, t), 0.5);
        }
        if report_points {
            let readout = Readout::new(layout, &values);
            for (index, (point, element, barycentric)) in probes.iter().enumerate() {
                let truth = exact(*point, t);
                for method in 0..RULES {
                    let read = readout.read(method, *element, *barycentric);
                    magnitude[index][method] =
                        magnitude[index][method].max((read.norm() - truth.norm()).abs());
                    bias[index][method] += (read.norm() - truth.norm()) / steps as f64;
                }
            }
        }
    }
    if report_points {
        for (index, name) in ["upstream", "downstream"].iter().enumerate() {
            print!("  {name:<10} ||c|-|u|| worst");
            for method in 0..RULES {
                print!("  {} {:.2e}", METHODS[method], magnitude[index][method]);
            }
            println!();
            print!("  {:<10} mean |c|-|u|  ", "");
            for method in 0..RULES {
                print!("  {} {:+.2e}", METHODS[method], bias[index][method]);
            }
            println!();
        }
    }
    errors
}

/// The travelling packet of `canonical_consumer`'s tests: `P` a Gaussian of
/// width 0.3 at −1.2 times `sin 2π(x + 1.2)`, released moving right in the
/// left medium of `[−3, 3] × [−½, ½]` with `u = −c₁P′(x)`. Where the
/// channel holds a second medium beyond `x = 0` it splits there by the
/// impedances `Z = A/c`: reflected by `R = (Z₁ − Z₂)/(Z₁ + Z₂)`, transmitted
/// by `T = 1 + R`. The transverse field is `orientation·(0, A ∂ₓψ)`, `ψ`
/// the time integral of `u`: `∓(A/c)u` for a wave moving right or left.
/// Read sixty times over 2.2 s.
fn packet_run(prepared: &Prepared, layout: &Layout) -> Errors {
    let (x0, sigma, k) = (-1.2, 0.3, std::f64::consts::TAU);
    let envelope = |s: f64| (-(s * s) / (2.0 * sigma * sigma)).exp();
    let p_of = |x: f64| envelope(x - x0) * (k * (x - x0)).sin();
    let dp_of = |x: f64| {
        let s = x - x0;
        envelope(s) * (k * (k * s).cos() - s / (sigma * sigma) * (k * s).sin())
    };
    let peak = (-4000..=4000)
        .map(|i| dp_of(x0 + f64::from(i) * 1.0e-3).abs())
        .fold(0.0, f64::max);
    // The stiffness each side holds, read off its samples; both masses are
    // the default medium's, one.
    let side = |right: bool| {
        let element = (0..layout.corners.len())
            .find(|element| (layout.centroid(*element).x > 0.05) == right)
            .unwrap();
        layout.inverse(element).xx
    };
    let (a1, a2) = (side(false), side(true));
    let (c1, c2) = (a1.sqrt(), a2.sqrt());
    let (z1, z2) = (a1 / c1, a2 / c2);
    let reflected = (z1 - z2) / (z1 + z2);
    let transmitted = 1.0 + reflected;
    // The incident wave as a signal at the interface: `u = f(t − x/c₁)`.
    let f = |tau: f64| -c1 * dp_of(-c1 * tau);
    let field = |point: Point2, t: f64| {
        let flux = if point.x <= 0.0 {
            -(a1 / c1) * f(t - point.x / c1) + (a1 / c1) * reflected * f(t + point.x / c1)
        } else {
            -(a2 / c2) * transmitted * f(t - point.x / c2)
        };
        Point2::new(0.0, flux * layout.orientation)
    };
    let operator = &prepared.canonical;
    let primary = layout
        .node_points
        .iter()
        .map(|p| -c1 * dp_of(p.x))
        .collect::<Vec<_>>();
    let potential = layout
        .node_points
        .iter()
        .map(|p| p_of(p.x))
        .collect::<Vec<_>>();
    let h = operator.recommended_time_step();
    let mut state =
        CanonicalWaveState::from_primary_and_potential(operator, h, &primary, &potential)
            .expect("packet state");
    let steps = (2.2 / h).ceil() as usize;
    let every = (steps / 60).max(1);
    let mut errors = Errors::new();
    for step in 1..=steps {
        state.step(operator).expect("step");
        if step % every == 0 {
            let t = state.time();
            let values = layout.physical(state.complementary_flux());
            errors.read(layout, &values, |_, point| field(point, t), peak * a1 / c1);
        }
    }
    if a1 != a2 {
        println!("  stiffness {a1:.3} | {a2:.3}, R {reflected:.4}, T {transmitted:.4}");
    }
    errors
}

/// L2 on a box split at `x` by a reflecting wall, a different wave on each
/// side: what a fit across the slit would mix.
fn slit_level(prepared: &Prepared, layout: &Layout, x: f64) -> Errors {
    let left = PlaneWave::oblique(std::f64::consts::TAU);
    let right = PlaneWave {
        k: 1.5 * std::f64::consts::TAU,
        direction: Point2::new(0.2_f64.cos(), -0.2_f64.sin()),
        phase: 2.1,
    };
    let mut node_side = vec![None; layout.node_points.len()];
    for element in 0..layout.corners.len() {
        let is_right = layout.centroid(element).x > x;
        for node in prepared.canonical.element_nodes()[element] {
            let known = node_side[node as usize].get_or_insert(is_right);
            assert_eq!(*known, is_right, "a node on both sides of the slit");
        }
    }
    let potential = layout
        .node_points
        .iter()
        .zip(&node_side)
        .map(|(point, side)| {
            if side.unwrap() {
                right.potential(*point)
            } else {
                left.potential(*point)
            }
        })
        .collect::<Vec<_>>();
    let flux = prepared
        .canonical
        .compatible_flux(&potential)
        .expect("compatible flux");
    let mut errors = Errors::new();
    errors.read(
        layout,
        &layout.physical(&flux),
        |element, point| {
            let wave = if layout.centroid(element).x > x {
                right
            } else {
                left
            };
            layout
                .inverse(element)
                .apply(wave.flux(layout.orientation, point))
        },
        1.0,
    );
    errors
}

/// L2 on a gallery scene: the flux of a plane wave's interpolant against the
/// wave's own, `J` left out because a graded medium has no exact `J` at a
/// point between its samples. Ten elements to the wavelength.
fn gallery_level(prepared: &Prepared, layout: &Layout, edge: f64) -> Errors {
    let wave = PlaneWave::oblique(0.6 / edge);
    let potential = layout
        .node_points
        .iter()
        .map(|point| wave.potential(*point))
        .collect::<Vec<_>>();
    let flux = prepared
        .canonical
        .compatible_flux(&potential)
        .expect("compatible flux");
    let mut errors = Errors::new();
    errors.read(
        layout,
        &flux,
        |_, point| wave.flux(layout.orientation, point),
        1.0,
    );
    errors
}

fn run_static(name: &str, scene: TopologyScene, edge: f64, wave: PlaneWave, summary: &mut Summary) {
    let prepared = prepare(scene, edge).expect(name);
    let layout = Layout::new(&prepared);
    describe(name, &prepared, &layout);
    let errors = second_level(&prepared, &layout, wave);
    errors.print("L2, the field of the potential's interpolant");
    weight_sums(&layout);
    summary.add(format!("{name}, L2"), &errors);
}

fn main() {
    let started = Instant::now();
    let arguments = std::env::args().collect::<Vec<_>>();
    let only = arguments
        .iter()
        .position(|argument| argument == "--only")
        .and_then(|index| arguments.get(index + 1))
        .cloned();
    let part = |name: &str| only.as_deref().is_none_or(|only| only == name);
    let mut summary = Summary { rows: vec![] };
    let tau = std::f64::consts::TAU;

    if part("box") {
        println!("== Uniform meshes of the box, a plane wave of wavelength 1 at 0.3 rad ==");
        for edge in [0.1, 0.05, 0.025] {
            let name = format!("box at {edge}");
            let prepared = prepare(box_scene(DomainRect::UNIT), edge).expect("box");
            let layout = Layout::new(&prepared);
            describe(&name, &prepared, &layout);
            first_level(&layout, PlaneWave::oblique(tau))
                .print("L1, the exact field at the samples");
            let errors = second_level(&prepared, &layout, PlaneWave::oblique(tau));
            errors.print("L2, the field of the potential's interpolant");
            weight_sums(&layout);
            summary.add(format!("{name}, L2"), &errors);
        }
        println!();
    }

    if part("channel") {
        println!("== The observables channel and four meshes beside it ==");
        for (name, edge, radius) in [
            ("channel", 0.07, 0.3),
            ("channel at 0.065", 0.065, 0.3),
            ("channel at 0.075", 0.075, 0.3),
            ("channel, loop 0.297", 0.07, 0.297),
            ("channel, loop 0.303", 0.07, 0.303),
        ] {
            let prepared = prepare(channel_scene(radius), edge).expect("channel");
            let layout = Layout::new(&prepared);
            describe(name, &prepared, &layout);
            println!("  step {:.4e}", prepared.canonical.recommended_time_step());
            let first = name == "channel";
            if first {
                let errors = second_level(
                    &prepared,
                    &layout,
                    PlaneWave {
                        k: 2.0 * tau,
                        direction: Point2::new(1.0, 0.0),
                        phase: 0.4,
                    },
                );
                errors.print("L2, the field of the potential's interpolant");
                weight_sums(&layout);
            }
            let errors = channel_run(&prepared, &layout, true);
            errors.print("L3, the solver's samples over the run");
            summary.add(format!("{name}, L3"), &errors);
        }
        println!();
    }

    if part("packet") {
        println!("== The travelling packet, uniform meshes of [-3, 3] x [-1/2, 1/2] ==");
        for edge in [0.1, 0.05, 0.025] {
            let name = format!("packet at {edge}");
            let prepared = prepare(box_scene(CHANNEL), edge).expect("packet");
            let layout = Layout::new(&prepared);
            describe(&name, &prepared, &layout);
            let errors = packet_run(&prepared, &layout);
            errors.print("L3, the packet's samples over its run");
            summary.add(format!("{name}, L3"), &errors);
        }
        println!();
    }

    if part("meshes") {
        println!("== Graded, curved, slit and sliver meshes, L2 ==");
        let oblique = PlaneWave::oblique(tau);
        let mut small_hole = box_scene(DomainRect::UNIT);
        add_closed(&mut small_hole, 1, Point2::new(0.1, -0.05), 0.04, None);
        run_static(
            "small hole, graded, at 0.1",
            small_hole,
            0.1,
            oblique,
            &mut summary,
        );
        let mut holes = box_scene(DomainRect::UNIT);
        add_closed(&mut holes, 1, Point2::new(-0.4, 0.3), 0.03, None);
        add_closed(&mut holes, 2, Point2::new(0.45, -0.35), 0.02, None);
        run_static(
            "two small holes, graded, at 0.05",
            holes,
            0.05,
            oblique,
            &mut summary,
        );
        let mut hole = box_scene(DomainRect::UNIT);
        add_closed(&mut hole, 1, Point2::new(0.0, 0.0), 0.35, None);
        run_static("curved hole at 0.1", hole, 0.1, oblique, &mut summary);
        let mut baffle = box_scene(DomainRect::UNIT);
        baffle.geometry.curves.push(curve(
            3,
            CurveSpline::Open(
                OpenCubicSpline::uniform(vec![
                    Point2::new(-0.6, -0.1),
                    Point2::new(-0.25, 0.05),
                    Point2::new(0.05, 0.1),
                    Point2::new(0.4, 0.15),
                ])
                .unwrap(),
            ),
            SpanBehavior::REFLECTING,
        ));
        run_static("free baffle at 0.1", baffle, 0.1, oblique, &mut summary);
        for (gap, edge) in [(0.012, 0.18), (0.006, 0.12)] {
            let mut sliver = box_scene(DomainRect::UNIT);
            let tip = Point2::new(0.05, 0.25);
            let mut hanging = curve(
                60,
                CurveSpline::Open(
                    OpenCubicSpline::polyline(vec![
                        Point2::new(0.0, 1.0),
                        tip + Point2::new(0.0, gap),
                        tip,
                    ])
                    .unwrap(),
                ),
                SpanBehavior::REFLECTING,
            );
            hanging.nodes[0].vertex = Some(TopologyVertexId(60));
            sliver.geometry.curves.push(hanging);
            sliver.geometry.vertices.push(TopologyVertex {
                id: TopologyVertexId(60),
                location: TopologyVertexLocation::Outer {
                    side: OuterSide::Top,
                    fraction: 0.5,
                },
            });
            sliver.geometry.synchronize_vertices().unwrap();
            run_static(
                &format!("baffle tip span {gap} at {edge}"),
                sliver,
                edge,
                oblique,
                &mut summary,
            );
        }
        // Two faces cannot share a region, so the right one has its own, of
        // the same medium: only the slit's own nodes keep the sides apart.
        let mut slit = box_scene(DomainRect::UNIT);
        add_medium(&mut slit, RegionId(2), None);
        add_divider(
            &mut slit,
            0.1,
            SpanBehavior::REFLECTING,
            BACKGROUND_REGION,
            RegionId(2),
        );
        let name = "slit wall to wall at 0.1";
        let prepared = prepare(slit, 0.1).expect(name);
        let layout = Layout::new(&prepared);
        describe(name, &prepared, &layout);
        let errors = slit_level(&prepared, &layout, 0.1);
        errors.print("L2, a different wave on each side");
        summary.add(format!("{name}, L2"), &errors);
        println!();
    }

    if part("interface") {
        println!("== Material interfaces ==");
        let mut disc = box_scene(DomainRect::UNIT);
        add_medium(&mut disc, RegionId(2), Some(4.0));
        add_closed(&mut disc, 1, Point2::new(0.1, 0.0), 0.45, Some(RegionId(2)));
        run_static(
            "disc of stiffness slot 4 at 0.08",
            disc,
            0.08,
            PlaneWave::oblique(tau),
            &mut summary,
        );
        for edge in [0.05, 0.025] {
            let mut channel = box_scene(CHANNEL);
            add_medium(&mut channel, RegionId(2), Some(4.0));
            add_divider(
                &mut channel,
                0.0,
                SpanBehavior::Transmitting,
                BACKGROUND_REGION,
                RegionId(2),
            );
            let name = format!("packet through an interface at {edge}");
            let prepared = prepare(channel, edge).expect("interface");
            let layout = Layout::new(&prepared);
            describe(&name, &prepared, &layout);
            let errors = packet_run(&prepared, &layout);
            errors.print("L3, the packet's samples over its run");
            summary.add(format!("{name}, L3"), &errors);
        }
        println!();
    }

    if part("gallery") {
        println!("== Gallery scenes at their own edge, L2 of a plane wave on b ==");
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let mut files = std::fs::read_dir(&directory)
            .expect("the gallery")
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        files.sort();
        let mut merged = Errors::new();
        for path in files {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let bytes = std::fs::read(&path).unwrap();
            let document = match funfern_app::topology_persistence::parse_document(&bytes) {
                Ok(document) => document,
                Err(error) => {
                    println!("{name}: not read: {error}");
                    continue;
                }
            };
            let edge = document.presentation.mesh_edge;
            let prepared = match prepare(document.model.accepted, edge) {
                Ok(prepared) => prepared,
                Err(error) => {
                    println!("{name}: not prepared: {error}");
                    continue;
                }
            };
            let layout = Layout::new(&prepared);
            describe(&format!("{name} at {edge}"), &prepared, &layout);
            let errors = gallery_level(&prepared, &layout, edge);
            print!("  worst, bulk/wall/interface:");
            for method in [0, 2, 3, 5, 6] {
                print!(" {}", METHODS[method]);
                for kind in 0..3 {
                    let stat = errors.worst(method, kind);
                    if stat.count > 0 {
                        print!(" {:.1e}", stat.worst);
                    } else {
                        print!(" -");
                    }
                }
            }
            println!();
            for method in 0..RULES {
                for kind in 0..3 {
                    for place in 0..3 {
                        merged.stats[method][kind][place].merge(errors.stats[method][kind][place]);
                    }
                }
            }
            for kind in 0..3 {
                merged.samples[kind].merge(errors.samples[kind]);
            }
        }
        merged.print("Every gallery scene together");
        summary.add("gallery, L2 on b", &merged);
        println!();
    }

    summary.print();
    println!("{:.1} s", started.elapsed().as_secs_f64());
}
