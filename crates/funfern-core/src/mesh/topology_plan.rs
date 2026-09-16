//! Mesh-facing projection of a compiled curve arrangement.
//!
//! The projection resolves each geometric arrangement vertex into one or more
//! finite-element trace vertices. It is deliberately independent from the old
//! obstacle/interface/baffle labels so the triangulator and solver can migrate
//! without reconstructing sectors from coordinates.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    BoundaryEdge, BoundaryLabel, BoundaryPoint, BridgeSearch, EarSearch, EarStage, MeshBuilder,
    MeshError, MeshQuality, MeshTriangle, MeshingStats, OpenConstraintKind, PolygonLocation,
    SegmentRelation, TriMesh, TriangulationDomain, boundary_adjacency, edge_key, point_in_triangle,
    segment_relation, valid_options,
};
use crate::{
    CompiledBoundaryStep, CompiledEdge, CompiledEdgeSource, CurveId, CurveSpanId, FaceId, Point2,
    PredicateSign, RegionId, SpanBehavior, TopologySnapshot, TraceVertexId, orient2d,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceRegionAssignment {
    pub face: FaceId,
    /// `None` excludes the face from the simulation, as for a hole.
    pub region: Option<RegionId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CurveTraceSide {
    Left,
    Right,
}

impl CurveTraceSide {
    pub fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct ExpandedChain {
    pub(super) traces: [TraceVertexId; 2],
    pub(super) vertices: Vec<usize>,
    pub(super) parameters: Vec<f64>,
}

/// Builds a constrained triangular mesh directly from the compiled topology
/// contract. This is the robust full-rebuild baseline for the unified model;
/// it never reconstructs legacy obstacle, divider, or baffle objects.
pub fn mesh_topology_plan(
    plan: &TopologyMeshPlan,
    mesh_revision: u64,
    options: super::MeshingOptions,
) -> Result<TriMesh, MeshError> {
    let mut job = TopologyMeshingJob::new(plan.clone(), mesh_revision, options);
    loop {
        if let Some(result) = job.advance(4096) {
            return result;
        }
    }
}

enum TopologyMeshingState {
    Initialize,
    Bridge(BridgeSearch),
    Clip(EarSearch),
    Legalize,
    Refine,
    CutSlits { index: usize },
    SplitSlitVertices,
    ApplySlitLineage,
    LegalizeFinal,
    VerifyTriangles { index: usize, quality: MeshQuality },
    VerifyBoundary { index: usize, quality: MeshQuality },
    Done,
}

/// Cooperative full meshing for an immutable topology plan.
pub struct TopologyMeshingJob {
    plan: TopologyMeshPlan,
    mesh_revision: u64,
    builder: MeshBuilder,
    trace_vertices: BTreeMap<TraceVertexId, usize>,
    slit_runs: Vec<(RegionId, Vec<PlannedFaceStep>)>,
    slit_steps: Vec<PlannedFaceStep>,
    legalization_work: usize,
    state: TopologyMeshingState,
    /// Triangles below this index were imported by a repair and are verified
    /// for orientation and adjacency only; their regions were settled by the
    /// repair against the compiled topology.
    region_check_from: usize,
}

impl TopologyMeshingJob {
    pub fn new(plan: TopologyMeshPlan, mesh_revision: u64, options: super::MeshingOptions) -> Self {
        let builder = MeshBuilder::new(options, plan.domain);
        Self {
            plan,
            mesh_revision,
            builder,
            trace_vertices: BTreeMap::new(),
            slit_runs: vec![],
            slit_steps: vec![],
            legalization_work: 0,
            state: TopologyMeshingState::Initialize,
            region_check_from: 0,
        }
    }

    /// Continues meshing from a builder that a repair has prepared: imported
    /// triangles below `region_check_from` are frozen, and the builder's
    /// domains are the cavity polygons to bridge, clip, legalize and refine
    /// before the slit runs are cut and the whole mesh is verified.
    pub(super) fn resume(
        plan: TopologyMeshPlan,
        mesh_revision: u64,
        builder: MeshBuilder,
        trace_vertices: BTreeMap<TraceVertexId, usize>,
        slit_runs: Vec<(RegionId, Vec<PlannedFaceStep>)>,
        region_check_from: usize,
    ) -> Self {
        let state = match builder.domains.first().cloned() {
            Some(domain) => TopologyMeshingState::Bridge(new_bridge_search(domain, 0)),
            None => TopologyMeshingState::Legalize,
        };
        Self {
            plan,
            mesh_revision,
            builder,
            trace_vertices,
            slit_runs,
            slit_steps: vec![],
            legalization_work: 0,
            state,
            region_check_from,
        }
    }

    pub fn stats(&self) -> MeshingStats {
        self.builder.stats
    }

    pub fn phase(&self) -> &'static str {
        match self.state {
            TopologyMeshingState::Initialize => "Preparing topology",
            TopologyMeshingState::Bridge(_) => "Connecting holes",
            TopologyMeshingState::Clip(_) => "Triangulating",
            TopologyMeshingState::Legalize | TopologyMeshingState::LegalizeFinal => {
                "Legalizing edges"
            }
            TopologyMeshingState::Refine => "Refining",
            TopologyMeshingState::CutSlits { .. }
            | TopologyMeshingState::SplitSlitVertices
            | TopologyMeshingState::ApplySlitLineage => "Recovering separated curves",
            TopologyMeshingState::VerifyTriangles { .. }
            | TopologyMeshingState::VerifyBoundary { .. } => "Checking mesh",
            TopologyMeshingState::Done => "Finished",
        }
    }

    pub fn advance(&mut self, budget: usize) -> Option<Result<TriMesh, MeshError>> {
        for _ in 0..budget {
            if matches!(self.state, TopologyMeshingState::Done) {
                return None;
            }
            self.builder.stats.work_units += 1;
            let result = self.step();
            match result {
                Ok(Some(mesh)) => return Some(Ok(mesh)),
                Ok(None) => {}
                Err(error) => {
                    self.state = TopologyMeshingState::Done;
                    return Some(Err(error));
                }
            }
        }
        None
    }

    fn step(&mut self) -> Result<Option<TriMesh>, MeshError> {
        if !valid_options(self.builder.options) {
            return Err(MeshError::InvalidOptions);
        }
        if self.builder.stats.work_units > 50_000_000 {
            return Err(MeshError::Topology("topology meshing work limit reached"));
        }
        let state = std::mem::replace(&mut self.state, TopologyMeshingState::Done);
        let b = &mut self.builder;
        self.state = match state {
            TopologyMeshingState::Initialize => {
                let prepared = prepare_topology_builder(&self.plan, b)?;
                self.trace_vertices = prepared.trace_vertices;
                self.slit_runs = prepared.slit_runs;
                let domain = b
                    .domains
                    .first()
                    .cloned()
                    .ok_or(MeshError::Topology("no topology domain to triangulate"))?;
                TopologyMeshingState::Bridge(new_bridge_search(domain, 0))
            }
            TopologyMeshingState::Bridge(search) => self.step_bridge(search)?,
            TopologyMeshingState::Clip(search) => self.step_clip(search)?,
            TopologyMeshingState::Legalize => {
                if b.dirty_edges.is_empty() {
                    self.legalization_work = 0;
                    TopologyMeshingState::Refine
                } else {
                    self.legalization_work += 1;
                    if self.legalization_work > b.options.max_triangles.saturating_mul(128) {
                        return Err(MeshError::Topology("edge legalization work limit reached"));
                    }
                    b.legalize_one()?;
                    TopologyMeshingState::Legalize
                }
            }
            TopologyMeshingState::Refine => {
                if !b.bad_triangles.is_empty()
                    && b.stats.refinement_insertions >= b.options.max_refinement_steps
                {
                    return Err(MeshError::RefinementLimit(b.quality()));
                }
                if b.refine_once()? {
                    TopologyMeshingState::CutSlits { index: 0 }
                } else {
                    TopologyMeshingState::Legalize
                }
            }
            TopologyMeshingState::CutSlits { index } => {
                if let Some((region, run)) = self.slit_runs.get(index) {
                    let separated = run.first().is_some_and(|step| {
                        matches!(step.boundary.behavior, Some(SpanBehavior::Separated { .. }))
                    });
                    if separated {
                        cut_free_slit(b, *region, run, &mut self.trace_vertices)?;
                        self.slit_steps.extend(run.iter().copied());
                    } else {
                        label_transmitting_slit(b, *region, run, &mut self.trace_vertices)?;
                    }
                    TopologyMeshingState::CutSlits { index: index + 1 }
                } else {
                    TopologyMeshingState::SplitSlitVertices
                }
            }
            TopologyMeshingState::SplitSlitVertices => {
                split_slit_trace_vertices(b, &self.slit_steps, &mut self.trace_vertices)?;
                TopologyMeshingState::ApplySlitLineage
            }
            TopologyMeshingState::ApplySlitLineage => {
                apply_slit_trace_lineage(b, &self.slit_steps, &mut self.trace_vertices)?;
                self.legalization_work = 0;
                TopologyMeshingState::LegalizeFinal
            }
            TopologyMeshingState::LegalizeFinal => {
                if b.dirty_edges.is_empty() {
                    TopologyMeshingState::VerifyTriangles {
                        index: 0,
                        quality: MeshQuality {
                            minimum_angle_degrees: 180.0,
                            maximum_edge_length: 0.0,
                        },
                    }
                } else {
                    self.legalization_work += 1;
                    if self.legalization_work > b.options.max_triangles.saturating_mul(128) {
                        return Err(MeshError::Topology("edge legalization work limit reached"));
                    }
                    b.legalize_one()?;
                    TopologyMeshingState::LegalizeFinal
                }
            }
            TopologyMeshingState::VerifyTriangles { index, mut quality } => {
                if index == b.triangles.len() {
                    TopologyMeshingState::VerifyBoundary { index: 0, quality }
                } else {
                    verify_topology_triangle(
                        b,
                        index,
                        &mut quality,
                        index >= self.region_check_from,
                    )?;
                    TopologyMeshingState::VerifyTriangles {
                        index: index + 1,
                        quality,
                    }
                }
            }
            TopologyMeshingState::VerifyBoundary { index, quality } => {
                if index == b.boundary_edges.len() {
                    return Ok(Some(TriMesh {
                        geometry_revision: self.plan.geometry_revision,
                        mesh_revision: self.mesh_revision,
                        vertices: std::mem::take(&mut b.vertices),
                        triangles: std::mem::take(&mut b.triangles),
                        boundary_edges: std::mem::take(&mut b.boundary_edges),
                        quality,
                        requested_sizes: vec![],
                    }));
                }
                verify_topology_boundary(b, index)?;
                TopologyMeshingState::VerifyBoundary {
                    index: index + 1,
                    quality,
                }
            }
            TopologyMeshingState::Done => TopologyMeshingState::Done,
        };
        Ok(None)
    }

    fn step_bridge(&mut self, mut search: BridgeSearch) -> Result<TopologyMeshingState, MeshError> {
        let b = &self.builder;
        if search.hole == search.holes.len() {
            return Ok(TopologyMeshingState::Clip(EarSearch {
                polygon: search.polygon,
                domain: search.domain,
                region: search.region,
                index: 0,
                degenerate_pass: false,
                stage: EarStage::Start,
            }));
        }
        let hole_len = search.holes[search.hole].len();
        if search.seeding {
            if search.outer_index == search.polygon.len() {
                let (length, outer_index, hole_index) = search
                    .best
                    .take()
                    .ok_or(MeshError::Topology("no topology bridge candidate"))?;
                search.outer_index = outer_index;
                search.hole_index = hole_index;
                search.visibility = Some((0, 0, length));
                search.seeding = false;
                search.testing_seed = true;
            } else {
                let length = (b.point(search.polygon[search.outer_index])
                    - b.point(search.holes[search.hole][search.hole_index]))
                .norm();
                if search.best.is_none_or(|best| length < best.0) {
                    search.best = Some((length, search.outer_index, search.hole_index));
                }
                advance_bridge_pair(&mut search, hole_len);
            }
        } else if search.outer_index == search.polygon.len() {
            let (_, outer_index, hole_index) = search
                .best
                .ok_or(MeshError::Topology("no visible bridge to topology cycle"))?;
            let hole = search.holes[search.hole].clone();
            let mut splice = Vec::with_capacity(hole.len() + 2);
            for offset in 0..hole.len() {
                splice.push(hole[(hole_index + offset) % hole.len()]);
            }
            splice.push(hole[hole_index]);
            splice.push(search.polygon[outer_index]);
            search
                .polygon
                .splice(outer_index + 1..outer_index + 1, splice);
            search.hole += 1;
            search.outer_index = 0;
            search.hole_index = 0;
            search.best = None;
            search.seeding = true;
        } else {
            step_bridge_visibility(b, &mut search)?;
        }
        Ok(TopologyMeshingState::Bridge(search))
    }

    fn step_clip(&mut self, mut search: EarSearch) -> Result<TopologyMeshingState, MeshError> {
        let b = &mut self.builder;
        let polygon = &mut search.polygon;
        if polygon.len() == 3 {
            // Three points on one line enclose nothing. A polygon that has
            // collapsed to a slit - what is left of a transmitting chain the
            // boundary walked out along and back - contributes no triangle, and
            // its edges are already constraints.
            if orient2d(
                b.point(polygon[0]),
                b.point(polygon[1]),
                b.point(polygon[2]),
            ) != PredicateSign::Zero
            {
                b.push_triangle(
                    b.ccw_triangle([polygon[0], polygon[1], polygon[2]], search.region)?,
                )?;
            }
            let next_domain = search.domain + 1;
            return Ok(if let Some(domain) = b.domains.get(next_domain).cloned() {
                TopologyMeshingState::Bridge(new_bridge_search(domain, next_domain))
            } else {
                TopologyMeshingState::Legalize
            });
        }
        if search.index == polygon.len() {
            if search.degenerate_pass {
                return Err(MeshError::Topology("topology ear clipping stalled"));
            }
            search.degenerate_pass = true;
            search.index = 0;
            search.stage = EarStage::Start;
            return Ok(TopologyMeshingState::Clip(search));
        }
        let index = search.index;
        let previous = (index + polygon.len() - 1) % polygon.len();
        let next = (index + 1) % polygon.len();
        let [a, v, c] = [polygon[previous], polygon[index], polygon[next]];
        let mut reject = false;
        let mut remove = false;
        match search.stage {
            EarStage::Start => {
                let sign = orient2d(b.point(a), b.point(v), b.point(c));
                let candidate = if search.degenerate_pass {
                    a == v || v == c || a == c || sign == PredicateSign::Zero
                } else {
                    a != v && v != c && a != c && sign == PredicateSign::Positive
                };
                if candidate {
                    search.stage = EarStage::Diagonal(0);
                } else {
                    reject = true;
                }
            }
            EarStage::Diagonal(other) => {
                if other == polygon.len() {
                    if search.degenerate_pass {
                        remove = true;
                    } else {
                        search.stage = EarStage::Contains(0);
                    }
                } else {
                    let [x, y] = [polygon[other], polygon[(other + 1) % polygon.len()]];
                    reject = x != a
                        && x != c
                        && y != a
                        && y != c
                        && segment_relation(b.point(a), b.point(c), b.point(x), b.point(y))
                            != SegmentRelation::Disjoint;
                    search.stage = EarStage::Diagonal(other + 1);
                }
            }
            EarStage::Contains(other_index) => {
                if other_index == polygon.len() {
                    remove = true;
                } else {
                    let other = polygon[other_index];
                    reject = ![previous, index, next].contains(&other_index)
                        && ![a, v, c].contains(&other)
                        && point_in_triangle(b.point(other), [b.point(a), b.point(v), b.point(c)])
                            != PolygonLocation::Outside;
                    search.stage = EarStage::Contains(other_index + 1);
                }
            }
        }
        if remove {
            if !search.degenerate_pass {
                b.push_triangle(MeshTriangle {
                    vertices: [a, v, c],
                    region: search.region,
                })?;
            }
            polygon.remove(index);
            search.index = 0;
            search.degenerate_pass = false;
            search.stage = EarStage::Start;
        } else if reject {
            search.index += 1;
            search.stage = EarStage::Start;
        }
        Ok(TopologyMeshingState::Clip(search))
    }
}

struct PreparedTopology {
    trace_vertices: BTreeMap<TraceVertexId, usize>,
    slit_runs: Vec<(RegionId, Vec<PlannedFaceStep>)>,
}

/// Ends one stretch of a face cycle. The stretch has to come back to where it
/// started, or the cycle walked out of the face and never returned, which the
/// plan is not allowed to produce. The two ends are compared by position, not
/// by identity: a junction carries one trace vertex per sector, so a stretch
/// that leaves and returns through the same junction ends on a different trace
/// at the same point - which is how `cut_free_slit` pairs the two sides of a
/// slit as well.
fn close_face_cycle(
    points: &BTreeMap<TraceVertexId, Point2>,
    polygon: &mut Vec<usize>,
    ends: &mut Option<[TraceVertexId; 2]>,
    cycles: &mut Vec<Vec<usize>>,
) -> Result<(), MeshError> {
    let Some(ends) = ends.take() else {
        return Ok(());
    };
    let polygon = std::mem::take(polygon);
    if polygon.is_empty() {
        return Ok(());
    }
    let [first, last] = ends.map(|trace| points.get(&trace));
    let (Some(first), Some(last)) = (first, last) else {
        return Err(MeshError::Topology("topology trace vertex is missing"));
    };
    if polygon.len() < 3 || first != last {
        return Err(MeshError::Topology("topology face cycle is incomplete"));
    }
    cycles.push(polygon);
    Ok(())
}

fn signed_polygon_area(builder: &MeshBuilder, polygon: &[usize]) -> f64 {
    let area = (0..polygon.len())
        .map(|index| {
            let from = builder.point(polygon[index]);
            let to = builder.point(polygon[(index + 1) % polygon.len()]);
            from.cross(to)
        })
        .sum::<f64>();
    0.5 * area
}

fn prepare_topology_builder(
    plan: &TopologyMeshPlan,
    builder: &mut MeshBuilder,
) -> Result<PreparedTopology, MeshError> {
    let trace_points = plan
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.point))
        .collect::<BTreeMap<_, _>>();
    let mut trace_vertices = BTreeMap::new();
    let mut chains = BTreeMap::<(usize, Option<CurveTraceSide>), ExpandedChain>::new();
    let mut free_slits = BTreeMap::<(RegionId, CurveId), Vec<PlannedFaceStep>>::new();

    for domain in &plan.domains {
        let mut edge_counts = BTreeMap::<usize, usize>::new();
        for step in domain.steps.iter().flatten() {
            *edge_counts.entry(step.edge).or_default() += 1;
        }
        let is_slit = |step: &PlannedFaceStep| {
            edge_counts.get(&step.edge).copied().unwrap_or(0) > 1
                && matches!(step.boundary.behavior, Some(SpanBehavior::Separated { .. }))
        };
        for step in domain.steps.iter().flatten().filter(|step| is_slit(step)) {
            let PlannedBoundarySource::Curve { curve, .. } = step.boundary.source else {
                continue;
            };
            free_slits
                .entry((domain.region, curve))
                .or_default()
                .push(*step);
        }
        let mut cycles = Vec::with_capacity(domain.steps.len());
        for cycle in &domain.steps {
            // A slit is left out of the polygon here and cut back into the
            // triangulated face afterwards. Removing it can break one cycle
            // into several: a slit that bridges two holes, joins a hole to the
            // outer boundary, or meets itself is traversed twice by a cycle
            // that runs through both of the loops it joins. Each stretch
            // between two removals closes on the trace vertex the slit left
            // from, so the stretches are collected as separate polygons.
            // Concatenating them instead leaves a polygon that jumps across
            // the slit - a zero-width channel with no interior, which no
            // refinement can ever make well shaped.
            //
            // Starting just after the first removal keeps a stretch that spans
            // the cycle's own start in one piece.
            let start = cycle.iter().position(&is_slit).map_or(0, |index| index + 1);
            let mut polygon = vec![];
            let mut ends: Option<[TraceVertexId; 2]> = None;
            for offset in 0..cycle.len() {
                let step = &cycle[(start + offset) % cycle.len()];
                if is_slit(step) {
                    close_face_cycle(&trace_points, &mut polygon, &mut ends, &mut cycles)?;
                    continue;
                }
                let separated =
                    matches!(step.boundary.behavior, Some(SpanBehavior::Separated { .. }));
                let side = match step.boundary.source {
                    PlannedBoundarySource::Curve { side, .. } if separated => Some(side),
                    _ => None,
                };
                // A stretch that leaves a junction through one sector and
                // returns through another names two traces at one point of one
                // face - a curve that meets itself does this, and so does a
                // slit's attachment. They are one mesh vertex: seeding it
                // before the chain is expanded keeps the boundary edges and the
                // polygon on the same vertex instead of leaving the arriving
                // one with no triangle to belong to. The sectors are separated
                // again by `split_slit_trace_vertices` once the slits are cut.
                if let Some([first, _]) = ends
                    && step.boundary.traces[1] != first
                    && trace_points.get(&step.boundary.traces[1]) == trace_points.get(&first)
                    && let Some(vertex) = trace_vertices.get(&first).copied()
                {
                    trace_vertices
                        .entry(step.boundary.traces[1])
                        .or_insert(vertex);
                }
                let key = (step.edge, side);
                if let std::collections::btree_map::Entry::Vacant(entry) = chains.entry(key) {
                    let chain =
                        expand_step_chain(builder, &trace_points, &mut trace_vertices, step)?;
                    entry.insert(chain);
                }
                let chain = &chains[&key];
                let (oriented, parameters) = if chain.traces == step.boundary.traces {
                    (chain.vertices.clone(), chain.parameters.clone())
                } else if chain.traces == [step.boundary.traces[1], step.boundary.traces[0]] {
                    (
                        chain.vertices.iter().rev().copied().collect(),
                        chain.parameters.iter().rev().copied().collect(),
                    )
                } else {
                    return Err(MeshError::Topology("topology trace chain is inconsistent"));
                };
                polygon.extend_from_slice(&oriented[..oriented.len() - 1]);

                let label = topology_label(step.boundary);
                if let Some((_, regions)) = builder
                    .boundary_regions
                    .iter_mut()
                    .find(|(candidate, _)| *candidate == label)
                {
                    regions.insert(domain.region);
                } else {
                    builder
                        .boundary_regions
                        .push((label, BTreeSet::from([domain.region])));
                }
                for (pair, parameter) in oriented.windows(2).zip(parameters.windows(2)) {
                    let key = edge_key(pair[0], pair[1]);
                    if !builder.boundary_keys.contains(&key) {
                        builder.add_boundary_edge(BoundaryEdge {
                            vertices: [pair[0], pair[1]],
                            label,
                            parameters: [parameter[0], parameter[1]],
                        });
                    }
                }
                ends = Some([
                    ends.map_or(step.boundary.traces[0], |ends| ends[0]),
                    step.boundary.traces[1],
                ]);
            }
            close_face_cycle(&trace_points, &mut polygon, &mut ends, &mut cycles)?;
        }
        // Every cycle runs with the face on its left, so the outer boundary
        // turns positively and encloses every hole, which turns the other way.
        // Order no longer settles which is which: a slit joining a hole to the
        // outer boundary splits the face's first cycle into one of each. The
        // largest signed area is the outer boundary - taking every positive one
        // would instead trip over a cycle that walks a transmitting chain out
        // and back, whose area is zero up to rounding.
        let mut outer = None;
        let mut holes = Vec::with_capacity(cycles.len());
        for polygon in cycles {
            let area = signed_polygon_area(builder, &polygon);
            match outer.take() {
                Some((best_area, best)) if best_area >= area => {
                    outer = Some((best_area, best));
                    holes.push(polygon);
                }
                Some((_, best)) => {
                    outer = Some((area, polygon));
                    holes.push(best);
                }
                None => outer = Some((area, polygon)),
            }
        }
        let Some(outer) = outer
            .filter(|(area, _)| *area > 0.0)
            .map(|(_, polygon)| polygon)
        else {
            return Err(MeshError::Topology("topology face has no outer cycle"));
        };
        builder.domains.push(TriangulationDomain {
            region: domain.region,
            outer,
            holes,
        });
    }

    let mut slit_runs = vec![];
    for ((region, _), steps) in free_slits {
        slit_runs.extend(
            split_slit_runs(&steps)?
                .into_iter()
                .map(|run| (region, run)),
        );
    }
    Ok(PreparedTopology {
        trace_vertices,
        slit_runs,
    })
}

pub(super) fn new_bridge_search(domain: TriangulationDomain, index: usize) -> BridgeSearch {
    BridgeSearch {
        polygon: domain.outer,
        holes: domain.holes,
        hole: 0,
        domain: index,
        region: domain.region,
        outer_index: 0,
        hole_index: 0,
        best: None,
        visibility: None,
        seeding: true,
        testing_seed: false,
    }
}

fn advance_bridge_pair(search: &mut BridgeSearch, hole_len: usize) {
    search.hole_index += 1;
    if search.hole_index == hole_len {
        search.hole_index = 0;
        search.outer_index += 1;
    }
}

fn step_bridge_visibility(
    builder: &MeshBuilder,
    search: &mut BridgeSearch,
) -> Result<(), MeshError> {
    let a_index = search.polygon[search.outer_index];
    let hole_len = search.holes[search.hole].len();
    let v_index = search.holes[search.hole][search.hole_index];
    let a = builder.point(a_index);
    let v = builder.point(v_index);
    let mut next_candidate = false;
    if let Some((stage, index, length)) = search.visibility {
        let edge = if stage == 0 {
            builder.boundary_edges.get(index).map(|edge| {
                if builder.boundary_relevant_to_region(edge.label, search.region) {
                    edge.vertices
                } else {
                    [a_index, v_index]
                }
            })
        } else if index < search.polygon.len() {
            Some([
                search.polygon[index],
                search.polygon[(index + 1) % search.polygon.len()],
            ])
        } else {
            None
        };
        if let Some(edge) = edge {
            if !edge.contains(&a_index) && !edge.contains(&v_index) {
                let relation =
                    segment_relation(a, v, builder.point(edge[0]), builder.point(edge[1]));
                next_candidate = if stage == 0 {
                    relation != SegmentRelation::Disjoint
                } else {
                    relation == SegmentRelation::ProperIntersection
                };
            }
            search.visibility = Some((stage, index + 1, length));
        } else if stage == 0 {
            search.visibility = Some((1, 0, length));
        } else {
            if builder.region_at(a.lerp(v, 0.5)) == Some(search.region) {
                search.best = Some((length, search.outer_index, search.hole_index));
            }
            next_candidate = true;
        }
    } else {
        let length = (a - v).norm();
        if a != v && search.best.is_none_or(|best| length < best.0) {
            search.visibility = Some((0, 0, length));
        } else {
            next_candidate = true;
        }
    }
    if next_candidate {
        search.visibility = None;
        if search.testing_seed {
            search.testing_seed = false;
            search.outer_index = if search.best.is_some() {
                search.polygon.len()
            } else {
                0
            };
            search.hole_index = 0;
        } else {
            advance_bridge_pair(search, hole_len);
        }
    }
    Ok(())
}

pub(super) fn verify_topology_triangle(
    builder: &MeshBuilder,
    index: usize,
    quality: &mut MeshQuality,
    check_region: bool,
) -> Result<(), MeshError> {
    let triangle = builder.triangles[index];
    let [a, b, c] = builder.triangle_points(triangle);
    if orient2d(a, b, c) != PredicateSign::Positive {
        return Err(MeshError::Topology("mesh contains an inverted triangle"));
    }
    if check_region && builder.region_at((a + b + c) / 3.0) != Some(triangle.region) {
        return Err(MeshError::Topology(
            "triangle has the wrong topology face region",
        ));
    }
    for opposite in 0..3 {
        let edge = edge_key(
            triangle.vertices[(opposite + 1) % 3],
            triangle.vertices[(opposite + 2) % 3],
        );
        let sides = builder
            .adjacency
            .get(&edge)
            .ok_or(MeshError::Topology("missing adjacency"))?;
        let expected = if builder.boundary_keys.contains(&edge) {
            builder
                .boundary_edges
                .iter()
                .find(|boundary| edge_key(boundary.vertices[0], boundary.vertices[1]) == edge)
                .map_or(2, |boundary| boundary_adjacency(boundary.label))
        } else {
            2
        };
        if sides.len() != expected || !sides.contains(&(index, triangle.vertices[opposite])) {
            return Err(MeshError::Topology("mesh has a crack or non-manifold edge"));
        }
    }
    let triangle_quality = builder.triangle_quality(triangle);
    let twice_area = (b - a).cross(c - a).abs();
    if twice_area
        <= triangle_quality.maximum_edge_length * triangle_quality.maximum_edge_length * 1.0e-10
    {
        return Err(MeshError::Topology(
            "mesh contains a scale-degenerate triangle near a constraint",
        ));
    }
    quality.minimum_angle_degrees = quality
        .minimum_angle_degrees
        .min(triangle_quality.minimum_angle_degrees);
    quality.maximum_edge_length = quality
        .maximum_edge_length
        .max(triangle_quality.maximum_edge_length);
    Ok(())
}

fn verify_topology_boundary(builder: &MeshBuilder, index: usize) -> Result<(), MeshError> {
    let boundary = builder.boundary_edges[index];
    let expected = boundary_adjacency(boundary.label);
    if builder
        .adjacency
        .get(&edge_key(boundary.vertices[0], boundary.vertices[1]))
        .is_none_or(|sides| sides.len() != expected)
    {
        return Err(MeshError::Topology(
            "constrained edge has incorrect adjacency",
        ));
    }
    Ok(())
}

pub(super) fn split_slit_runs(
    steps: &[PlannedFaceStep],
) -> Result<Vec<Vec<PlannedFaceStep>>, MeshError> {
    let mut left = steps
        .iter()
        .filter(|step| {
            matches!(
                step.boundary.source,
                PlannedBoundarySource::Curve {
                    side: CurveTraceSide::Left,
                    ..
                }
            )
        })
        .copied()
        .collect::<Vec<_>>();
    left.sort_by(|a, b| a.boundary.parameter[0].total_cmp(&b.boundary.parameter[0]));
    if left.is_empty() {
        return Err(MeshError::Topology("free slit has no left trace"));
    }
    let right = steps
        .iter()
        .filter_map(|step| {
            matches!(
                step.boundary.source,
                PlannedBoundarySource::Curve {
                    side: CurveTraceSide::Right,
                    ..
                }
            )
            .then_some((step.edge, *step))
        })
        .collect::<BTreeMap<_, _>>();
    if left.iter().any(|step| !right.contains_key(&step.edge)) {
        return Err(MeshError::Topology("free slit is missing its right trace"));
    }
    let mut runs = vec![];
    let mut run_edges = vec![left[0].edge];
    let mut previous = left[0];
    for step in &left[1..] {
        let previous_right = right[&previous.edge];
        let next_right = right[&step.edge];
        let parameter_continues = previous.boundary.parameter[1] == step.boundary.parameter[0];
        let left_trace_continues = previous.boundary.traces[1] == step.boundary.traces[0];
        // Right steps run opposite to curve parameter, hence endpoint 0 is the
        // preceding span's end and endpoint 1 is the following span's start.
        let right_trace_continues =
            previous_right.boundary.traces[0] == next_right.boundary.traces[1];
        if !(parameter_continues && left_trace_continues && right_trace_continues) {
            runs.push(std::mem::take(&mut run_edges));
        }
        run_edges.push(step.edge);
        previous = *step;
    }
    runs.push(run_edges);
    Ok(runs
        .into_iter()
        .map(|edges| {
            let edges = edges.into_iter().collect::<BTreeSet<_>>();
            steps
                .iter()
                .filter(|step| edges.contains(&step.edge))
                .copied()
                .collect()
        })
        .collect())
}

/// One slit's chain, installed into the triangulation.
struct RecoveredSlit {
    curve: CurveId,
    constraint: usize,
    /// Mesh vertex and curve parameter of every sample along the chain.
    chain: Vec<(usize, f64)>,
    /// The left-hand steps in parameter order, for reading a segment's span.
    left: Vec<PlannedFaceStep>,
}

/// Installs one slit's chain into the triangulation: samples it and recovers
/// every segment as a constrained edge. What happens next depends on whether
/// the slit separates its two sides.
fn recover_slit_chain(
    builder: &mut MeshBuilder,
    region: RegionId,
    steps: &[PlannedFaceStep],
    trace_vertices: &mut BTreeMap<TraceVertexId, usize>,
) -> Result<RecoveredSlit, MeshError> {
    let mut left = steps
        .iter()
        .filter(|step| {
            matches!(
                step.boundary.source,
                PlannedBoundarySource::Curve {
                    side: CurveTraceSide::Left,
                    ..
                }
            )
        })
        .copied()
        .collect::<Vec<_>>();
    left.sort_by(|a, b| a.boundary.parameter[0].total_cmp(&b.boundary.parameter[0]));
    let first = left
        .first()
        .ok_or(MeshError::Topology("free slit has no left trace"))?;
    let curve = match first.boundary.source {
        PlannedBoundarySource::Curve { curve, .. } => curve,
        PlannedBoundarySource::Outer(_) => unreachable!(),
    };
    if left.iter().any(|step| {
        !matches!(step.boundary.source, PlannedBoundarySource::Curve { curve: candidate, .. } if candidate == curve)
    }) {
        return Err(MeshError::Topology("a free slit cycle contains multiple curves"));
    }
    let mut samples = vec![];
    for step in &left {
        let edge = step.boundary;
        let length = (edge.points[1] - edge.points[0]).norm();
        let pieces = (length / builder.chain_target(edge.points)).ceil().max(1.0) as usize;
        for piece in 0..pieces {
            let fraction = piece as f64 / pieces as f64;
            samples.push((
                crate::Sample {
                    t: edge.parameter[0] + (edge.parameter[1] - edge.parameter[0]) * fraction,
                    point: edge.points[0].lerp(edge.points[1], fraction),
                },
                (piece == 0).then_some(edge.traces[0]),
            ));
        }
    }
    let last = left.last().unwrap().boundary;
    samples.push((
        crate::Sample {
            t: last.parameter[1],
            point: last.points[1],
        },
        Some(last.traces[1]),
    ));
    if samples.len() == 2 {
        samples.insert(
            1,
            (
                crate::Sample {
                    t: (samples[0].0.t + samples[1].0.t) * 0.5,
                    point: samples[0].0.point.lerp(samples[1].0.point, 0.5),
                },
                None,
            ),
        );
    }

    let constraint = builder.internal_samples.len();
    builder
        .internal_samples
        .push(samples.iter().map(|(sample, _)| *sample).collect());
    builder.constraint_target_regions.push(region);
    builder.constraint_kinds.push(OpenConstraintKind::Baffle {
        id: crate::InternalBoundaryId(curve.0),
        region,
    });
    builder.internal_chains.push(vec![]);
    for (sample, trace) in samples {
        let vertex = if let Some(vertex) = trace.and_then(|trace| trace_vertices.get(&trace)) {
            *vertex
        } else {
            builder.insert_constraint_point(constraint, sample.point)?
        };
        builder.internal_chains[constraint].push((vertex, sample.t));
    }
    let mut segment = 0;
    let mut attempts = 0usize;
    while segment + 1 < builder.internal_chains[constraint].len() {
        let pair = [
            builder.internal_chains[constraint][segment],
            builder.internal_chains[constraint][segment + 1],
        ];
        let requested = [pair[0].0, pair[1].0];
        // A vertex sitting on the segment leaves no edge to flip - a slit along
        // a line of symmetry of the domain collects them - so it joins the
        // chain and the two halves are recovered instead.
        if let Some((vertex, fraction)) = builder.vertex_inside_segment(requested) {
            let parameter = pair[0].1 + (pair[1].1 - pair[0].1) * fraction;
            builder.internal_chains[constraint].insert(segment + 1, (vertex, parameter));
            continue;
        }
        if builder.recover_constraint_edge(requested)? {
            segment += 1;
            attempts = 0;
            continue;
        }
        attempts += 1;
        if attempts > builder.options.max_triangles.saturating_mul(8) {
            return Err(MeshError::Topology("free-slit recovery work limit reached"));
        }
    }
    let chain = builder.internal_chains[constraint].clone();
    Ok(RecoveredSlit {
        curve,
        constraint,
        chain,
        left,
    })
}

/// A slit whose two sides transmit: a divider that encloses nothing, which the
/// face walks out along and back. Nothing is cut - both sides are the same
/// medium and share their nodes - so the recovered chain only has to be labelled
/// as the curve it belongs to, one segment at a time against the span it came
/// from.
fn label_transmitting_slit(
    builder: &mut MeshBuilder,
    region: RegionId,
    steps: &[PlannedFaceStep],
    trace_vertices: &mut BTreeMap<TraceVertexId, usize>,
) -> Result<(), MeshError> {
    let RecoveredSlit {
        curve, chain, left, ..
    } = recover_slit_chain(builder, region, steps, trace_vertices)?;
    for pair in chain.windows(2) {
        let parameter = 0.5 * (pair[0].1 + pair[1].1);
        let planned = left
            .iter()
            .find(|step| {
                let [a, b] = step.boundary.parameter;
                parameter >= a.min(b) && parameter <= a.max(b)
            })
            .ok_or(MeshError::Topology("free-slit span lineage is missing"))?;
        let PlannedBoundarySource::Curve { span, .. } = planned.boundary.source else {
            return Err(MeshError::Topology("slit trace has a non-curve label"));
        };
        // One edge, labelled by the side the face's first traversal names; the
        // other side is registered as carrying the region but adds no second
        // edge, exactly as the full rebuild's deduplication leaves it.
        let label = BoundaryLabel::Curve {
            curve,
            span,
            side: CurveTraceSide::Left,
            separated: false,
        };
        let key = edge_key(pair[0].0, pair[1].0);
        if !builder.boundary_keys.contains(&key) {
            builder.add_boundary_edge(BoundaryEdge {
                vertices: [pair[0].0, pair[1].0],
                label,
                parameters: [pair[0].1, pair[1].1],
            });
        }
        for side in [CurveTraceSide::Left, CurveTraceSide::Right] {
            let sided = BoundaryLabel::Curve {
                curve,
                span,
                side,
                separated: false,
            };
            if let Some((_, regions)) = builder
                .boundary_regions
                .iter_mut()
                .find(|(candidate, _)| *candidate == sided)
            {
                regions.insert(region);
            } else {
                builder
                    .boundary_regions
                    .push((sided, BTreeSet::from([region])));
            }
        }
        // A vertex the recovery created carries no lineage of its own, and an
        // adaptation checks every constrained edge against the plan: an
        // interval's endpoint has to hold its trace, and everything between
        // them the label and parameter it sits at.
        for (vertex, parameter) in [pair[0], pair[1]] {
            let planned = left.iter().find_map(|step| {
                step.boundary
                    .parameter
                    .iter()
                    .zip(step.boundary.traces)
                    .find_map(|(end, trace)| (*end == parameter).then_some(trace))
            });
            match planned {
                Some(trace) => {
                    if builder.vertices[vertex]
                        .trace
                        .is_some_and(|existing| existing != trace)
                    {
                        return Err(MeshError::Topology("slit trace lineage is inconsistent"));
                    }
                    builder.vertices[vertex].trace = Some(trace);
                    trace_vertices.insert(trace, vertex);
                }
                None => {
                    if builder.vertices[vertex].trace.is_none() {
                        builder.vertices[vertex].boundary =
                            Some(BoundaryPoint { label, parameter });
                    }
                }
            }
        }
    }
    Ok(())
}

/// A slit that separates its two sides: the mesh is cut along the recovered
/// chain, giving each side its own vertices, and the cut's internal-boundary
/// labels become the curve's own.
fn cut_free_slit(
    builder: &mut MeshBuilder,
    region: RegionId,
    steps: &[PlannedFaceStep],
    trace_vertices: &mut BTreeMap<TraceVertexId, usize>,
) -> Result<(), MeshError> {
    let RecoveredSlit {
        curve,
        constraint,
        chain,
        left,
    } = recover_slit_chain(builder, region, steps, trace_vertices)?;
    builder.cut_internal_boundary(constraint)?;

    let legacy_id = crate::InternalBoundaryId(curve.0);
    for endpoint in [0, chain.len() - 1] {
        let left_vertex = chain[endpoint].0;
        let point = builder.point(left_vertex);
        let right_trace = steps.iter().find_map(|step| {
            let PlannedBoundarySource::Curve {
                side: CurveTraceSide::Right,
                ..
            } = step.boundary.source
            else {
                return None;
            };
            step.boundary
                .points
                .iter()
                .zip(step.boundary.traces)
                .find_map(|(candidate, trace)| (*candidate == point).then_some(trace))
        });
        if let Some(replacement) = right_trace.and_then(|trace| trace_vertices.get(&trace).copied())
            && replacement != left_vertex
        {
            rewire_attached_slit_endpoint(builder, legacy_id, left_vertex, replacement)?;
        }
    }
    for edge in &mut builder.boundary_edges {
        let BoundaryLabel::InternalBoundary { id, side } = edge.label else {
            continue;
        };
        if id != legacy_id {
            continue;
        }
        let parameter = 0.5 * (edge.parameters[0] + edge.parameters[1]);
        let planned = left
            .iter()
            .find(|step| {
                let [a, b] = step.boundary.parameter;
                parameter >= a.min(b) && parameter <= a.max(b)
            })
            .ok_or(MeshError::Topology("free-slit span lineage is missing"))?;
        let span = match planned.boundary.source {
            PlannedBoundarySource::Curve { span, .. } => span,
            PlannedBoundarySource::Outer(_) => unreachable!(),
        };
        edge.label = BoundaryLabel::Curve {
            curve,
            span,
            side: match side {
                super::InternalBoundarySide::Left => CurveTraceSide::Left,
                super::InternalBoundarySide::Right => CurveTraceSide::Right,
            },
            separated: true,
        };
        for endpoint in 0..2 {
            let vertex = edge.vertices[endpoint];
            if builder.vertices[vertex].trace.is_none() {
                builder.vertices[vertex].boundary = Some(BoundaryPoint {
                    label: edge.label,
                    parameter: edge.parameters[endpoint],
                });
            }
        }
    }
    builder.boundary_regions.retain(|(label, _)| {
        !matches!(label, BoundaryLabel::InternalBoundary { id, .. } if *id == legacy_id)
    });
    for side in [CurveTraceSide::Left, CurveTraceSide::Right] {
        for step in &left {
            let PlannedBoundarySource::Curve { span, .. } = step.boundary.source else {
                unreachable!()
            };
            builder.boundary_regions.push((
                BoundaryLabel::Curve {
                    curve,
                    span,
                    side,
                    separated: true,
                },
                BTreeSet::from([region]),
            ));
        }
    }
    Ok(())
}

fn apply_slit_trace_lineage(
    builder: &mut MeshBuilder,
    steps: &[PlannedFaceStep],
    trace_vertices: &mut BTreeMap<TraceVertexId, usize>,
) -> Result<(), MeshError> {
    for step in steps {
        let label = topology_label(step.boundary);
        if !matches!(label, BoundaryLabel::Curve { .. }) {
            return Err(MeshError::Topology("slit trace has a non-curve label"));
        }
        for (point, trace) in step.boundary.points.into_iter().zip(step.boundary.traces) {
            let vertices = builder
                .boundary_edges
                .iter()
                .filter(|edge| edge.label == label)
                .flat_map(|edge| edge.vertices)
                .filter(|vertex| {
                    (builder.point(*vertex) - point).norm() <= builder.options.curve_tolerance
                })
                .collect::<BTreeSet<_>>();
            let vertex = trace_vertices
                .get(&trace)
                .copied()
                .filter(|mapped| vertices.contains(mapped))
                .or_else(|| {
                    vertices
                        .iter()
                        .find(|vertex| builder.vertices[**vertex].trace.is_none())
                        .copied()
                })
                .or_else(|| vertices.iter().next().copied())
                .ok_or(MeshError::Topology(
                    "slit trace lineage endpoint is missing",
                ))?;
            if builder.vertices[vertex]
                .trace
                .is_some_and(|existing| existing != trace)
            {
                return Err(MeshError::Topology("slit trace lineage is inconsistent"));
            }
            builder.vertices[vertex].trace = Some(trace);
            if trace_vertices
                .insert(trace, vertex)
                .is_some_and(|mapped| mapped != vertex)
            {
                return Err(MeshError::Topology("slit trace lineage is duplicated"));
            }
        }
    }
    Ok(())
}

fn split_slit_trace_vertices(
    builder: &mut MeshBuilder,
    steps: &[PlannedFaceStep],
    trace_vertices: &mut BTreeMap<TraceVertexId, usize>,
) -> Result<(), MeshError> {
    let separated_keys = builder
        .boundary_edges
        .iter()
        .filter(|edge| {
            matches!(
                edge.label,
                BoundaryLabel::Curve {
                    separated: true,
                    ..
                }
            )
        })
        .map(|edge| edge_key(edge.vertices[0], edge.vertices[1]))
        .collect::<BTreeSet<_>>();
    let planned_trace = |label: BoundaryLabel, point: Point2| {
        steps.iter().find_map(|step| {
            (topology_label(step.boundary) == label).then(|| {
                step.boundary
                    .points
                    .iter()
                    .zip(step.boundary.traces)
                    .find_map(|(candidate, trace)| {
                        ((*candidate - point).norm() <= builder.options.curve_tolerance)
                            .then_some(trace)
                    })
            })?
        })
    };
    let mut seeds = BTreeMap::<usize, BTreeMap<TraceVertexId, BTreeSet<usize>>>::new();
    for edge in &builder.boundary_edges {
        if !matches!(
            edge.label,
            BoundaryLabel::Curve {
                separated: true,
                ..
            }
        ) {
            continue;
        }
        let Some([(triangle, _)]) = builder
            .adjacency
            .get(&edge_key(edge.vertices[0], edge.vertices[1]))
            .map(Vec::as_slice)
        else {
            return Err(MeshError::Topology(
                "separated trace edge has invalid adjacency",
            ));
        };
        for vertex in edge.vertices {
            let Some(trace) = planned_trace(edge.label, builder.point(vertex)) else {
                continue;
            };
            seeds
                .entry(vertex)
                .or_default()
                .entry(trace)
                .or_default()
                .insert(*triangle);
        }
    }

    for (vertex, trace_seeds) in seeds {
        let incident = builder.incident[vertex]
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if incident.is_empty() {
            return Err(MeshError::Topology(
                "separated trace endpoint has no incident element",
            ));
        }
        let mut remaining = incident.clone();
        let mut components = vec![];
        while let Some(seed) = remaining.pop_first() {
            let mut component = BTreeSet::from([seed]);
            let mut pending = vec![seed];
            while let Some(triangle_index) = pending.pop() {
                let triangle = builder.triangles[triangle_index];
                for other in triangle
                    .vertices
                    .iter()
                    .copied()
                    .filter(|candidate| *candidate != vertex)
                {
                    let radial = edge_key(vertex, other);
                    if separated_keys.contains(&radial) {
                        continue;
                    }
                    if let Some(sides) = builder.adjacency.get(&radial) {
                        for (neighbor, _) in sides {
                            if incident.contains(neighbor) && component.insert(*neighbor) {
                                remaining.remove(neighbor);
                                pending.push(*neighbor);
                            }
                        }
                    }
                }
            }
            components.push(component);
        }

        let components = components
            .into_iter()
            .map(|component| {
                let traces = trace_seeds
                    .iter()
                    .filter(|(_, triangles)| !triangles.is_disjoint(&component))
                    .map(|(trace, _)| *trace)
                    .collect::<BTreeSet<_>>();
                let mut traces = traces.into_iter();
                let Some(trace) = traces.next() else {
                    return Err(MeshError::Topology(
                        "separated junction sector has inconsistent lineage",
                    ));
                };
                if traces.next().is_some() {
                    return Err(MeshError::Topology(
                        "separated junction sector has inconsistent lineage",
                    ));
                }
                let boundaries = builder
                    .boundary_edges
                    .iter()
                    .enumerate()
                    .filter_map(|(index, edge)| {
                        if !edge.vertices.contains(&vertex) {
                            return None;
                        }
                        builder
                            .adjacency
                            .get(&edge_key(edge.vertices[0], edge.vertices[1]))
                            .is_some_and(|sides| {
                                sides
                                    .iter()
                                    .any(|(triangle, _)| component.contains(triangle))
                            })
                            .then_some(index)
                    })
                    .collect::<Vec<_>>();
                Ok((component, trace, boundaries))
            })
            .collect::<Result<Vec<_>, MeshError>>()?;
        let retained = builder.vertices[vertex]
            .trace
            .and_then(|trace| {
                components
                    .iter()
                    .position(|(_, candidate, _)| *candidate == trace)
            })
            .unwrap_or(0);
        for (index, (component, trace, boundaries)) in components.into_iter().enumerate() {
            let replacement = if index == retained {
                vertex
            } else {
                let replacement = builder.add_vertex(builder.point(vertex), None)?;
                builder.internal_trace_vertices.insert(replacement);
                replacement
            };
            builder.vertices[replacement].trace = Some(trace);
            if trace_vertices
                .insert(trace, replacement)
                .is_some_and(|existing| existing != replacement && existing != vertex)
            {
                return Err(MeshError::Topology(
                    "separated junction trace is represented twice",
                ));
            }
            if replacement == vertex {
                continue;
            }
            for triangle_index in component {
                let mut triangle = builder.triangles[triangle_index];
                if triangle.vertices.contains(&replacement) {
                    return Err(MeshError::Topology(
                        "separated junction split would degenerate an element",
                    ));
                }
                for candidate in &mut triangle.vertices {
                    if *candidate == vertex {
                        *candidate = replacement;
                    }
                }
                builder.replace_triangle(triangle_index, triangle);
            }
            for boundary_index in boundaries {
                let old = builder.boundary_edges[boundary_index].vertices;
                builder.boundary_keys.remove(&edge_key(old[0], old[1]));
                for candidate in &mut builder.boundary_edges[boundary_index].vertices {
                    if *candidate == vertex {
                        *candidate = replacement;
                    }
                }
                let new = builder.boundary_edges[boundary_index].vertices;
                builder.boundary_keys.insert(edge_key(new[0], new[1]));
            }
        }
    }
    Ok(())
}

fn rewire_attached_slit_endpoint(
    builder: &mut MeshBuilder,
    id: crate::InternalBoundaryId,
    endpoint: usize,
    replacement: usize,
) -> Result<(), MeshError> {
    let boundary_index = builder
        .boundary_edges
        .iter()
        .position(|edge| {
            edge.vertices.contains(&endpoint)
                && edge.label
                    == BoundaryLabel::InternalBoundary {
                        id,
                        side: super::InternalBoundarySide::Right,
                    }
        })
        .ok_or(MeshError::Topology(
            "attached slit has no right endpoint edge",
        ))?;
    let boundary = builder.boundary_edges[boundary_index];
    let seed = *builder
        .adjacency
        .get(&edge_key(boundary.vertices[0], boundary.vertices[1]))
        .and_then(|sides| sides.first())
        .map(|(triangle, _)| triangle)
        .ok_or(MeshError::Topology("attached slit endpoint has no element"))?;
    let mut sector = BTreeSet::from([seed]);
    let mut pending = vec![seed];
    while let Some(triangle_index) = pending.pop() {
        let triangle = builder.triangles[triangle_index];
        for other in triangle
            .vertices
            .iter()
            .copied()
            .filter(|candidate| *candidate != endpoint)
        {
            let edge = edge_key(endpoint, other);
            if builder.boundary_keys.contains(&edge) {
                continue;
            }
            if let Some(sides) = builder.adjacency.get(&edge) {
                for (neighbor, _) in sides {
                    if *neighbor != triangle_index
                        && builder.triangles[*neighbor].vertices.contains(&endpoint)
                        && sector.insert(*neighbor)
                    {
                        pending.push(*neighbor);
                    }
                }
            }
        }
    }
    for triangle_index in sector {
        let mut triangle = builder.triangles[triangle_index];
        if triangle.vertices.contains(&replacement) {
            return Err(MeshError::Topology(
                "attached slit endpoint would create a degenerate element",
            ));
        }
        for vertex in &mut triangle.vertices {
            if *vertex == endpoint {
                *vertex = replacement;
            }
        }
        builder.replace_triangle(triangle_index, triangle);
    }
    builder
        .boundary_keys
        .remove(&edge_key(boundary.vertices[0], boundary.vertices[1]));
    for vertex in &mut builder.boundary_edges[boundary_index].vertices {
        if *vertex == endpoint {
            *vertex = replacement;
        }
    }
    let rewired = builder.boundary_edges[boundary_index].vertices;
    builder
        .boundary_keys
        .insert(edge_key(rewired[0], rewired[1]));
    Ok(())
}

pub(super) fn topology_label(boundary: PlannedBoundaryEdge) -> BoundaryLabel {
    match boundary.source {
        PlannedBoundarySource::Outer(side) => BoundaryLabel::Outer(side),
        PlannedBoundarySource::Curve { curve, span, side } => {
            let separated = matches!(boundary.behavior, Some(SpanBehavior::Separated { .. }));
            BoundaryLabel::Curve {
                curve,
                span,
                side: if separated {
                    side
                } else {
                    CurveTraceSide::Left
                },
                separated,
            }
        }
    }
}

pub(super) fn trace_mesh_vertex(
    builder: &mut MeshBuilder,
    points: &BTreeMap<TraceVertexId, Point2>,
    vertices: &mut BTreeMap<TraceVertexId, usize>,
    trace: TraceVertexId,
) -> Result<usize, MeshError> {
    if let Some(vertex) = vertices.get(&trace) {
        return Ok(*vertex);
    }
    let point = *points
        .get(&trace)
        .ok_or(MeshError::Topology("topology trace vertex is missing"))?;
    let vertex = builder.add_vertex(point, None)?;
    builder.vertices[vertex].trace = Some(trace);
    vertices.insert(trace, vertex);
    Ok(vertex)
}

pub(super) fn expand_step_chain(
    builder: &mut MeshBuilder,
    points: &BTreeMap<TraceVertexId, Point2>,
    trace_vertices: &mut BTreeMap<TraceVertexId, usize>,
    step: &PlannedFaceStep,
) -> Result<ExpandedChain, MeshError> {
    let boundary = step.boundary;
    let label = topology_label(boundary);
    let length = (boundary.points[1] - boundary.points[0]).norm();
    let pieces = (length / builder.chain_target(boundary.points))
        .ceil()
        .max(1.0) as usize;
    // The two sides of a separated span are expanded as separate chains that
    // run in opposite directions and must subdivide identically, so the
    // interior pieces are computed from the lower parameter to the higher one
    // and reversed afterwards when the step runs the other way.
    let forward = boundary.parameter[0] <= boundary.parameter[1];
    let (low_point, high_point, low, high) = if forward {
        (
            boundary.points[0],
            boundary.points[1],
            boundary.parameter[0],
            boundary.parameter[1],
        )
    } else {
        (
            boundary.points[1],
            boundary.points[0],
            boundary.parameter[1],
            boundary.parameter[0],
        )
    };
    let mut interior = (1..pieces)
        .map(|piece| {
            let fraction = piece as f64 / pieces as f64;
            (
                low_point.lerp(high_point, fraction),
                low + (high - low) * fraction,
            )
        })
        .collect::<Vec<_>>();
    if !forward {
        interior.reverse();
    }
    let mut vertices = Vec::with_capacity(pieces + 1);
    let mut parameters = Vec::with_capacity(pieces + 1);
    vertices.push(trace_mesh_vertex(
        builder,
        points,
        trace_vertices,
        boundary.traces[0],
    )?);
    parameters.push(boundary.parameter[0]);
    for (point, parameter) in interior {
        vertices.push(builder.add_vertex(point, Some(BoundaryPoint { label, parameter }))?);
        parameters.push(parameter);
    }
    vertices.push(trace_mesh_vertex(
        builder,
        points,
        trace_vertices,
        boundary.traces[1],
    )?);
    parameters.push(boundary.parameter[1]);
    for (vertex, parameter) in vertices.iter().copied().zip(parameters.iter().copied()) {
        if builder.vertices[vertex].boundary.is_none() {
            builder.vertices[vertex].boundary = Some(BoundaryPoint { label, parameter });
        }
    }
    Ok(ExpandedChain {
        traces: boundary.traces,
        vertices,
        parameters,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PlannedBoundarySource {
    Outer(crate::OuterSide),
    Curve {
        curve: CurveId,
        span: CurveSpanId,
        side: CurveTraceSide,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlannedBoundaryEdge {
    pub source: PlannedBoundarySource,
    pub behavior: Option<SpanBehavior>,
    pub face: FaceId,
    pub region: RegionId,
    pub traces: [TraceVertexId; 2],
    pub points: [Point2; 2],
    pub parameter: [f64; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlannedFaceStep {
    /// Snapshot-local compiled edge index. Both active sides of a transmitting
    /// edge use the same index, allowing the mesher to share its entire chain.
    pub edge: usize,
    pub boundary: PlannedBoundaryEdge,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlannedFaceDomain {
    pub face: FaceId,
    pub region: RegionId,
    /// Directed trace-vertex cycles with the active face on the left. The first
    /// is the outer cycle; later cycles are holes or slit components.
    pub cycles: Vec<Vec<TraceVertexId>>,
    /// Directed edge steps matching `cycles`, with this face on the left.
    pub steps: Vec<Vec<PlannedFaceStep>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlannedTraceVertex {
    pub id: TraceVertexId,
    pub point: Point2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedJunctionTraces {
    pub vertex: crate::TopologyVertexId,
    /// One entry per sector trace. Repeated face IDs are meaningful for a
    /// separated same-face junction.
    pub faces: Vec<FaceId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyMeshPlan {
    pub geometry_revision: u64,
    pub domain: crate::DomainRect,
    pub vertices: Vec<PlannedTraceVertex>,
    pub junctions: Vec<PlannedJunctionTraces>,
    pub domains: Vec<PlannedFaceDomain>,
    pub boundaries: Vec<PlannedBoundaryEdge>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyMeshPlanError {
    DuplicateFace(FaceId),
    MissingFace(FaceId),
    UnknownFace(FaceId),
    InvalidRegion(RegionId),
    DuplicateRegion(RegionId),
    NoActiveFaces,
    MissingTrace(TraceVertexId),
    BrokenCycle(FaceId),
    MissingCurve(CurveSpanId),
    CoupledExcludedFace(CurveSpanId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyMeshUpdateAction {
    /// Geometry and topology are identical; the current discretization remains valid.
    Reuse,
    /// The plan differs only where carving can follow: the changed atoms are
    /// carved out of the active mesh and refilled, the rest is kept.
    Repair(TopologyRepairReason),
    FullRebuild(TopologyFullRebuildReason),
}

/// What differs between two plans that carving can follow. The kinds are
/// diagnostic: the carve itself works from the atoms, whatever the edit was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyRepairReason {
    FaceAssignmentsChanged,
    CurveOrSpanTopologyChanged,
    SpanBehaviorChanged,
    TraceEquivalenceChanged,
    CurveOrJunctionMoved,
}

impl TopologyRepairReason {
    pub const fn label(self) -> &'static str {
        match self {
            Self::FaceAssignmentsChanged => "face assignments changed",
            Self::CurveOrSpanTopologyChanged => "curve or span topology changed",
            Self::SpanBehaviorChanged => "span behavior changed",
            Self::TraceEquivalenceChanged => "trace equivalence changed",
            Self::CurveOrJunctionMoved => "curve or junction moved",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyFullRebuildReason {
    DomainChanged,
    /// The plan is unchanged but the meshing options, such as the target edge
    /// length, are not those the active mesh was built with.
    MeshingOptionsChanged,
    /// The user asked for a rebuild of an unchanged plan, for instance to
    /// return from an adapted mesh to the base resolution.
    Requested,
    /// A repair was attempted and the carve failed; the runtime keeps the
    /// message alongside.
    RepairFailed,
    /// A mesh the plans agreed to reuse carries trace ids the new plan
    /// renumbered, and the two numberings could not be matched up.
    TraceIdentityChanged,
}

impl TopologyFullRebuildReason {
    pub const fn label(self) -> &'static str {
        match self {
            Self::DomainChanged => "outer domain changed",
            Self::MeshingOptionsChanged => "mesh resolution changed",
            Self::Requested => "rebuild requested",
            Self::RepairFailed => "mesh repair failed",
            Self::TraceIdentityChanged => "boundary trace identity changed",
        }
    }
}

/// Classifies reuse before starting mesh work. Identical plans reuse the
/// mesh; a changed outer domain rebuilds it; every other difference is a
/// repair by carving, named by the kind of change for the diagnostics.
pub fn topology_mesh_update_action(
    previous: &TopologyMeshPlan,
    next: &TopologyMeshPlan,
) -> TopologyMeshUpdateAction {
    use TopologyRepairReason as Reason;
    if previous.domain != next.domain {
        return TopologyMeshUpdateAction::FullRebuild(TopologyFullRebuildReason::DomainChanged);
    }
    let face_signature = |plan: &TopologyMeshPlan| {
        plan.domains
            .iter()
            .map(|domain| (domain.face, domain.region))
            .collect::<Vec<_>>()
    };
    if face_signature(previous) != face_signature(next) {
        return TopologyMeshUpdateAction::Repair(Reason::FaceAssignmentsChanged);
    }
    let source_signature = |plan: &TopologyMeshPlan| {
        plan.boundaries
            .iter()
            .map(|boundary| (boundary.source, boundary.face, boundary.region))
            .collect::<BTreeSet<_>>()
    };
    if source_signature(previous) != source_signature(next) {
        return TopologyMeshUpdateAction::Repair(Reason::CurveOrSpanTopologyChanged);
    }
    let behavior_signature = |plan: &TopologyMeshPlan| {
        plan.boundaries
            .iter()
            .map(|boundary| {
                (
                    boundary.source,
                    matches!(boundary.behavior, Some(SpanBehavior::Separated { .. })),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    if behavior_signature(previous) != behavior_signature(next) {
        return TopologyMeshUpdateAction::Repair(Reason::SpanBehaviorChanged);
    }
    if previous.junctions != next.junctions {
        return TopologyMeshUpdateAction::Repair(Reason::TraceEquivalenceChanged);
    }
    let same_geometry = previous.boundaries.len() == next.boundaries.len()
        && previous
            .boundaries
            .iter()
            .zip(&next.boundaries)
            .all(|(a, b)| a.points == b.points && a.parameter == b.parameter)
        && previous
            .vertices
            .iter()
            .map(|vertex| vertex.point)
            .eq(next.vertices.iter().map(|vertex| vertex.point));
    if same_geometry {
        TopologyMeshUpdateAction::Reuse
    } else {
        TopologyMeshUpdateAction::Repair(Reason::CurveOrJunctionMoved)
    }
}

/// Carries a reused mesh's trace ids forward to a renumbered plan.
///
/// The arrangement hands out one trace id per sector a vertex has, so a span
/// that stops dividing its faces - transmit switched on against an excluded
/// face, say - renumbers every trace from there on even though nothing moved
/// and every atom came out identical. A carve derives its traces from the plan
/// it is handed, but a reused mesh keeps the numbering it was built with, and
/// an adaptation checks each one against the plan it is given.
///
/// The atoms carry the correspondence: two plans that agree to reuse a mesh
/// hold the same boundary in the same place, so the traces at its ends name
/// the same two topological points under either numbering. `None` when the
/// lists do not line up that way, which the caller answers with a rebuild.
pub fn topology_trace_remap(
    previous: &TopologyMeshPlan,
    next: &TopologyMeshPlan,
) -> Option<BTreeMap<TraceVertexId, TraceVertexId>> {
    if previous.boundaries.len() != next.boundaries.len() {
        return None;
    }
    let mut remap = BTreeMap::new();
    for (before, after) in previous.boundaries.iter().zip(&next.boundaries) {
        if before.source != after.source
            || before.face != after.face
            || before.region != after.region
            || before.points != after.points
            || before.parameter != after.parameter
        {
            return None;
        }
        for (from, to) in before.traces.into_iter().zip(after.traces) {
            if remap
                .insert(from, to)
                .is_some_and(|existing| existing != to)
            {
                return None;
            }
        }
    }
    // Both directions matter: every trace the old mesh can carry has to arrive
    // somewhere, and what they arrive at has to be the new plan's own list,
    // which is what an adaptation checks the mesh against.
    let image = remap.values().copied().collect::<BTreeSet<_>>();
    if image.len() != remap.len()
        || previous
            .vertices
            .iter()
            .any(|vertex| !remap.contains_key(&vertex.id))
        || image != next.vertices.iter().map(|vertex| vertex.id).collect()
    {
        return None;
    }
    Some(remap)
}

impl std::fmt::Display for TopologyMeshPlanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateFace(_) => {
                formatter.write_str("two mesh regions claim the same subdomain")
            }
            Self::MissingFace(_) => formatter.write_str("a subdomain is missing from the mesh"),
            Self::UnknownFace(_) => {
                formatter.write_str("the mesh names a subdomain that does not exist")
            }
            Self::InvalidRegion(_) => {
                formatter.write_str("the mesh names a material that does not exist")
            }
            Self::DuplicateRegion(_) => {
                formatter.write_str("two mesh regions claim the same material")
            }
            Self::NoActiveFaces => formatter.write_str("there is nothing left to mesh"),
            Self::MissingTrace(_) => formatter.write_str("a boundary trace is missing"),
            Self::BrokenCycle(_) => formatter.write_str("a subdomain boundary does not close"),
            Self::MissingCurve(span) => {
                write!(formatter, "span {} is missing from the mesh", span.0)
            }
            Self::CoupledExcludedFace(span) => write!(
                formatter,
                "span {} couples to a subdomain that is excluded",
                span.0
            ),
        }
    }
}

impl std::error::Error for TopologyMeshPlanError {}

impl TopologyMeshPlan {
    pub fn new(
        topology: &TopologySnapshot,
        assignments: &[FaceRegionAssignment],
    ) -> Result<Self, TopologyMeshPlanError> {
        let topology_faces = topology
            .faces
            .iter()
            .map(|face| face.id)
            .collect::<BTreeSet<_>>();
        let mut assigned = BTreeMap::new();
        let mut regions = BTreeSet::new();
        for assignment in assignments {
            if !topology_faces.contains(&assignment.face) {
                return Err(TopologyMeshPlanError::UnknownFace(assignment.face));
            }
            if assigned
                .insert(assignment.face, assignment.region)
                .is_some()
            {
                return Err(TopologyMeshPlanError::DuplicateFace(assignment.face));
            }
            if let Some(region) = assignment.region {
                if region.0 == 0 {
                    return Err(TopologyMeshPlanError::InvalidRegion(region));
                }
                if !regions.insert(region) {
                    return Err(TopologyMeshPlanError::DuplicateRegion(region));
                }
            }
        }
        for face in &topology.faces {
            if !assigned.contains_key(&face.id) {
                return Err(TopologyMeshPlanError::MissingFace(face.id));
            }
        }
        if regions.is_empty() {
            return Err(TopologyMeshPlanError::NoActiveFaces);
        }

        validate_span_sides(topology, &assigned)?;

        let mut trace_points = BTreeMap::<TraceVertexId, Point2>::new();
        for vertex in &topology.vertices {
            for trace in &vertex.traces {
                if let Some(existing) = trace_points.insert(trace.id, vertex.point)
                    && existing != vertex.point
                {
                    return Err(TopologyMeshPlanError::MissingTrace(trace.id));
                }
            }
        }

        let mut domains = vec![];
        for face in &topology.faces {
            let Some(region) = assigned[&face.id] else {
                continue;
            };
            let compiled = face
                .boundaries
                .iter()
                .map(|cycle| compile_cycle(topology, &assigned, face.id, region, cycle))
                .collect::<Result<Vec<_>, _>>()?;
            let cycles = compiled
                .iter()
                .map(|(cycle, _)| cycle.clone())
                .collect::<Vec<_>>();
            let steps = compiled
                .into_iter()
                .map(|(_, steps)| steps)
                .collect::<Vec<_>>();
            if cycles.first().is_none_or(|cycle| cycle.len() < 3) {
                return Err(TopologyMeshPlanError::BrokenCycle(face.id));
            }
            domains.push(PlannedFaceDomain {
                face: face.id,
                region,
                cycles,
                steps,
            });
        }

        let mut boundaries = vec![];
        for edge in &topology.edges {
            append_boundary_sides(edge, &assigned, &mut boundaries)?;
        }
        let used_traces = domains
            .iter()
            .flat_map(|domain| domain.cycles.iter().flatten().copied())
            .chain(boundaries.iter().flat_map(|boundary| boundary.traces))
            .collect::<BTreeSet<_>>();
        let vertices = trace_points
            .into_iter()
            .filter(|(id, _)| used_traces.contains(id))
            .map(|(id, point)| PlannedTraceVertex { id, point })
            .collect();
        let junctions = topology
            .vertices
            .iter()
            .filter_map(|vertex| {
                vertex.authored.map(|authored| {
                    let mut faces = vertex
                        .traces
                        .iter()
                        .filter(|trace| assigned.get(&trace.face).copied().flatten().is_some())
                        .map(|trace| trace.face)
                        .collect::<Vec<_>>();
                    faces.sort();
                    PlannedJunctionTraces {
                        vertex: authored,
                        faces,
                    }
                })
            })
            .collect();
        Ok(Self {
            geometry_revision: topology.revision,
            domain: topology.domain,
            vertices,
            junctions,
            domains,
            boundaries,
        })
    }

    pub fn region_for_face(&self, face: FaceId) -> Option<RegionId> {
        self.domains
            .iter()
            .find(|domain| domain.face == face)
            .map(|domain| domain.region)
    }

    pub fn trace_point(&self, trace: TraceVertexId) -> Option<Point2> {
        self.vertices
            .binary_search_by_key(&trace, |vertex| vertex.id)
            .ok()
            .map(|index| self.vertices[index].point)
    }

    /// Resolves one planned boundary interval by its stable source label.
    /// Endpoints are excluded so a shared breakpoint cannot select both spans.
    pub fn boundary_at(
        &self,
        source: PlannedBoundarySource,
        parameter: f64,
    ) -> Option<PlannedBoundaryEdge> {
        if !parameter.is_finite() {
            return None;
        }
        let mut matches = self.boundaries.iter().copied().filter(|boundary| {
            let [a, b] = boundary.parameter;
            boundary.source == source && parameter > a.min(b) && parameter < a.max(b)
        });
        let boundary = matches.next()?;
        matches.next().is_none().then_some(boundary)
    }
}

/// How arrangement segments merge into mesh atoms. The tolerance is the chord
/// deviation the mesher already promises for curves, and the cap keeps an atom
/// no longer than the target edge, so subdividing an atom linearly, as the
/// mesher and adaptation do, stays within that tolerance of the curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtomCoarsening {
    pub tolerance: f64,
    pub maximum_chord: f64,
}

impl AtomCoarsening {
    pub fn from_meshing(options: super::MeshingOptions) -> Self {
        Self {
            tolerance: options.curve_tolerance,
            maximum_chord: options.target_edge_length,
        }
    }

    fn active(self) -> bool {
        self.tolerance.is_finite()
            && self.tolerance > 0.0
            && self.maximum_chord.is_finite()
            && self.maximum_chord > 0.0
    }
}

/// The span or outer side an arrangement segment belongs to, independent of
/// which face's side is being planned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum AtomKey {
    Outer(crate::OuterSide),
    Span(CurveSpanId),
}

impl AtomKey {
    fn of(edge: &CompiledEdge) -> Self {
        match edge.source {
            CompiledEdgeSource::Outer(side) => Self::Outer(side),
            CompiledEdgeSource::Curve(span) => Self::Span(span),
        }
    }
}

/// Whether every interior joint of `edges`, a run in increasing parameter
/// order, lies within the tolerance of the chord from the run's start to its
/// end, and the chord is no longer than the cap.
fn chord_within(topology: &TopologySnapshot, edges: &[usize], coarsening: AtomCoarsening) -> bool {
    let start = topology.edges[edges[0]].points[0];
    let end = topology.edges[edges[edges.len() - 1]].points[1];
    let chord = end - start;
    let length_squared = chord.dot(chord);
    if length_squared > coarsening.maximum_chord * coarsening.maximum_chord {
        return false;
    }
    edges[..edges.len() - 1].iter().all(|&edge| {
        let point = topology.edges[edge].points[1];
        let offset = point - start;
        let along = if length_squared > 0.0 {
            (offset.dot(chord) / length_squared).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (offset - chord * along).norm() <= coarsening.tolerance
    })
}

impl TopologyMeshPlan {
    /// Merges runs of arrangement segments into mesh atoms. The arrangement
    /// samples curves finely enough to intersect them robustly, and this plan
    /// used every one of those segments as a boundary edge, which pinned the
    /// boundary resolution and the time step far below the target. Runs merge
    /// while their joints stay within the chord tolerance and the chord under
    /// the cap; they never cross an authored vertex, a span boundary, an outer
    /// corner, a change of the faces or behaviour beside them, or any trace
    /// vertex shared with another source. Both sides of a span merge over the
    /// same runs, so paired traces stay paired. Face cycles, boundary atoms and
    /// trace vertices are rewritten consistently; junctions are unchanged.
    pub fn coarsened(
        &self,
        topology: &TopologySnapshot,
        coarsening: AtomCoarsening,
    ) -> Result<Self, TopologyMeshPlanError> {
        if !coarsening.active() {
            return Ok(self.clone());
        }
        let assigned = topology
            .faces
            .iter()
            .map(|face| (face.id, self.region_for_face(face.id)))
            .collect::<BTreeMap<_, _>>();

        let mut protected = BTreeSet::new();
        for vertex in &topology.vertices {
            if vertex.authored.is_some() {
                protected.extend(vertex.traces.iter().map(|trace| trace.id));
            }
        }
        let mut keys_by_trace = BTreeMap::<TraceVertexId, BTreeSet<AtomKey>>::new();
        for edge in &topology.edges {
            for traces in edge.traces {
                for id in [traces.left, traces.right] {
                    keys_by_trace
                        .entry(id)
                        .or_default()
                        .insert(AtomKey::of(edge));
                }
            }
        }
        protected.extend(
            keys_by_trace
                .iter()
                .filter(|(_, keys)| keys.len() > 1)
                .map(|(id, _)| *id),
        );

        let mut by_key = BTreeMap::<AtomKey, Vec<usize>>::new();
        for (index, edge) in topology.edges.iter().enumerate() {
            by_key.entry(AtomKey::of(edge)).or_default().push(index);
        }
        let signature = |index: usize| {
            let edge = &topology.edges[index];
            (
                edge.left,
                edge.right,
                effective_curve_behavior(edge, &assigned),
            )
        };
        let mut runs: Vec<Vec<usize>> = vec![];
        for (_, mut edges) in by_key {
            edges.sort_by(|a, b| {
                topology.edges[*a].parameter[0].total_cmp(&topology.edges[*b].parameter[0])
            });
            let mut start = 0;
            while start < edges.len() {
                let mut end = start;
                while end + 1 < edges.len() {
                    let joint = &topology.edges[edges[end]];
                    let next = &topology.edges[edges[end + 1]];
                    let continues = joint.parameter[1] == next.parameter[0]
                        && signature(edges[end]) == signature(edges[end + 1])
                        && !protected.contains(&joint.traces[1].left)
                        && !protected.contains(&joint.traces[1].right);
                    if !continues || !chord_within(topology, &edges[start..=end + 1], coarsening) {
                        break;
                    }
                    end += 1;
                }
                runs.push(edges[start..=end].to_vec());
                start = end + 1;
            }
        }
        runs.sort_by_key(|run| run.iter().copied().min());
        let mut run_of_edge = vec![usize::MAX; topology.edges.len()];
        for (index, run) in runs.iter().enumerate() {
            for &edge in run {
                run_of_edge[edge] = index;
            }
        }

        let mut boundaries = vec![];
        for run in &runs {
            let first = &topology.edges[run[0]];
            let last = &topology.edges[run[run.len() - 1]];
            let side = |face: FaceId| assigned.get(&face).copied().flatten();
            let points = [first.points[0], last.points[1]];
            let parameter = [first.parameter[0], last.parameter[1]];
            match first.source {
                CompiledEdgeSource::Outer(outer) => {
                    if let Some(region) = side(first.left) {
                        boundaries.push(PlannedBoundaryEdge {
                            source: PlannedBoundarySource::Outer(outer),
                            behavior: None,
                            face: first.left,
                            region,
                            traces: [first.traces[0].left, last.traces[1].left],
                            points,
                            parameter,
                        });
                    }
                }
                CompiledEdgeSource::Curve(span) => {
                    let curve = first
                        .curve
                        .ok_or(TopologyMeshPlanError::MissingCurve(span))?;
                    let behavior = effective_curve_behavior(first, &assigned);
                    if let Some(region) = side(first.left) {
                        boundaries.push(PlannedBoundaryEdge {
                            source: PlannedBoundarySource::Curve {
                                curve,
                                span,
                                side: CurveTraceSide::Left,
                            },
                            behavior,
                            face: first.left,
                            region,
                            traces: [first.traces[0].left, last.traces[1].left],
                            points,
                            parameter,
                        });
                    }
                    // Both sides, even when they name the same face: see
                    // `append_boundary_sides`.
                    if let Some(region) = side(first.right) {
                        boundaries.push(PlannedBoundaryEdge {
                            source: PlannedBoundarySource::Curve {
                                curve,
                                span,
                                side: CurveTraceSide::Right,
                            },
                            behavior,
                            face: first.right,
                            region,
                            traces: [last.traces[1].right, first.traces[0].right],
                            points: [last.points[1], first.points[0]],
                            parameter: [last.parameter[1], first.parameter[0]],
                        });
                    }
                }
            }
        }

        let mut domains = Vec::with_capacity(self.domains.len());
        for domain in &self.domains {
            let mut cycles = Vec::with_capacity(domain.steps.len());
            let mut steps = Vec::with_capacity(domain.steps.len());
            for cycle in &domain.steps {
                let key = |step: &PlannedFaceStep| {
                    let side = match step.boundary.source {
                        PlannedBoundarySource::Outer(_) => None,
                        PlannedBoundarySource::Curve { side, .. } => Some(side),
                    };
                    (run_of_edge[step.edge], side)
                };
                let count = cycle.len();
                if count == 0 {
                    return Err(TopologyMeshPlanError::BrokenCycle(domain.face));
                }
                // Start where a run begins so a run straddling the cycle's
                // first step is not cut in two.
                let rotation = (0..count)
                    .find(|&index| key(&cycle[index]) != key(&cycle[(index + count - 1) % count]))
                    .unwrap_or(0);
                let mut merged: Vec<PlannedFaceStep> = vec![];
                for offset in 0..count {
                    let step = &cycle[(rotation + offset) % count];
                    match merged.last_mut() {
                        Some(last) if key(last) == key(step) => {
                            last.boundary.traces[1] = step.boundary.traces[1];
                            last.boundary.points[1] = step.boundary.points[1];
                            last.boundary.parameter[1] = step.boundary.parameter[1];
                        }
                        _ => merged.push(PlannedFaceStep {
                            edge: runs[run_of_edge[step.edge]].iter().copied().min().unwrap(),
                            boundary: step.boundary,
                        }),
                    }
                }
                for pair in 0..merged.len() {
                    let next = (pair + 1) % merged.len();
                    if merged[pair].boundary.traces[1] != merged[next].boundary.traces[0] {
                        return Err(TopologyMeshPlanError::BrokenCycle(domain.face));
                    }
                }
                cycles.push(merged.iter().map(|step| step.boundary.traces[0]).collect());
                steps.push(merged);
            }
            domains.push(PlannedFaceDomain {
                face: domain.face,
                region: domain.region,
                cycles,
                steps,
            });
        }

        let used = domains
            .iter()
            .flat_map(|domain| domain.cycles.iter().flatten().copied())
            .chain(boundaries.iter().flat_map(|boundary| boundary.traces))
            .collect::<BTreeSet<_>>();
        let vertices = self
            .vertices
            .iter()
            .copied()
            .filter(|vertex| used.contains(&vertex.id))
            .collect();
        Ok(Self {
            geometry_revision: self.geometry_revision,
            domain: self.domain,
            vertices,
            junctions: self.junctions.clone(),
            domains,
            boundaries,
        })
    }
}

fn validate_span_sides(
    topology: &TopologySnapshot,
    assigned: &BTreeMap<FaceId, Option<RegionId>>,
) -> Result<(), TopologyMeshPlanError> {
    for edge in &topology.edges {
        let CompiledEdgeSource::Curve(span) = edge.source else {
            continue;
        };
        let left = assigned.get(&edge.left).copied().flatten();
        let right = assigned.get(&edge.right).copied().flatten();
        match edge.behavior {
            Some(SpanBehavior::Separated { coupling, .. })
                if !matches!(coupling, crate::InternalBoundaryCoupling::Independent)
                    && left.is_some() != right.is_some() =>
            {
                return Err(TopologyMeshPlanError::CoupledExcludedFace(span));
            }
            Some(_) => {}
            None => return Err(TopologyMeshPlanError::MissingCurve(span)),
        }
    }
    Ok(())
}

fn compile_cycle(
    topology: &TopologySnapshot,
    assigned: &BTreeMap<FaceId, Option<RegionId>>,
    face: FaceId,
    region: RegionId,
    steps: &[CompiledBoundaryStep],
) -> Result<(Vec<TraceVertexId>, Vec<PlannedFaceStep>), TopologyMeshPlanError> {
    let mut cycle = vec![];
    let mut planned = vec![];
    let mut expected = None;
    for step in steps {
        let edge = topology
            .edges
            .get(step.edge)
            .ok_or(TopologyMeshPlanError::BrokenCycle(face))?;
        let [start, end] = oriented_face_traces(edge, face, step.reversed)
            .ok_or(TopologyMeshPlanError::BrokenCycle(face))?;
        if expected.is_some_and(|expected| expected != start) {
            return Err(TopologyMeshPlanError::BrokenCycle(face));
        }
        cycle.push(start);
        planned.push(PlannedFaceStep {
            edge: step.edge,
            boundary: planned_face_edge(edge, assigned, face, region, step.reversed)?,
        });
        expected = Some(end);
    }
    if cycle.first().copied() != expected {
        return Err(TopologyMeshPlanError::BrokenCycle(face));
    }
    Ok((cycle, planned))
}

fn planned_face_edge(
    edge: &CompiledEdge,
    assigned: &BTreeMap<FaceId, Option<RegionId>>,
    face: FaceId,
    region: RegionId,
    reversed: bool,
) -> Result<PlannedBoundaryEdge, TopologyMeshPlanError> {
    let [start, end] = oriented_face_traces(edge, face, reversed)
        .ok_or(TopologyMeshPlanError::BrokenCycle(face))?;
    let (source, points, parameter) = match (edge.source, reversed) {
        (CompiledEdgeSource::Outer(side), false) => (
            PlannedBoundarySource::Outer(side),
            edge.points,
            edge.parameter,
        ),
        (CompiledEdgeSource::Curve(span), false) => (
            PlannedBoundarySource::Curve {
                curve: edge
                    .curve
                    .ok_or(TopologyMeshPlanError::MissingCurve(span))?,
                span,
                side: CurveTraceSide::Left,
            },
            edge.points,
            edge.parameter,
        ),
        (CompiledEdgeSource::Curve(span), true) => (
            PlannedBoundarySource::Curve {
                curve: edge
                    .curve
                    .ok_or(TopologyMeshPlanError::MissingCurve(span))?,
                span,
                side: CurveTraceSide::Right,
            },
            [edge.points[1], edge.points[0]],
            [edge.parameter[1], edge.parameter[0]],
        ),
        (CompiledEdgeSource::Outer(_), true) => {
            return Err(TopologyMeshPlanError::BrokenCycle(face));
        }
    };
    Ok(PlannedBoundaryEdge {
        source,
        behavior: effective_curve_behavior(edge, assigned),
        face,
        region,
        traces: [start, end],
        points,
        parameter,
    })
}

/// A transmitting trace against an excluded face is the wall of the active
/// domain. Compile it as a homogeneous Neumann boundary without changing the
/// authored span, so transmission returns if that face is activated later.
fn effective_curve_behavior(
    edge: &CompiledEdge,
    assigned: &BTreeMap<FaceId, Option<RegionId>>,
) -> Option<SpanBehavior> {
    let active = |face: FaceId| assigned.get(&face).copied().flatten().is_some();
    match edge.behavior {
        Some(SpanBehavior::Transmitting) if active(edge.left) != active(edge.right) => {
            Some(SpanBehavior::REFLECTING)
        }
        behavior => behavior,
    }
}

fn oriented_face_traces(
    edge: &CompiledEdge,
    face: FaceId,
    reversed: bool,
) -> Option<[TraceVertexId; 2]> {
    match (reversed, edge.left == face, edge.right == face) {
        (false, true, _) => Some([edge.traces[0].left, edge.traces[1].left]),
        (true, _, true) => Some([edge.traces[1].right, edge.traces[0].right]),
        _ => None,
    }
}

fn append_boundary_sides(
    edge: &CompiledEdge,
    assigned: &BTreeMap<FaceId, Option<RegionId>>,
    output: &mut Vec<PlannedBoundaryEdge>,
) -> Result<(), TopologyMeshPlanError> {
    let side = |face: FaceId| assigned.get(&face).copied().flatten();
    match edge.source {
        CompiledEdgeSource::Outer(outer) => {
            if let Some(region) = side(edge.left) {
                output.push(PlannedBoundaryEdge {
                    source: PlannedBoundarySource::Outer(outer),
                    behavior: None,
                    face: edge.left,
                    region,
                    traces: [edge.traces[0].left, edge.traces[1].left],
                    points: edge.points,
                    parameter: edge.parameter,
                });
            }
        }
        CompiledEdgeSource::Curve(span) => {
            let curve = edge
                .curve
                .ok_or(TopologyMeshPlanError::MissingCurve(span))?;
            let behavior = effective_curve_behavior(edge, assigned);
            if let Some(region) = side(edge.left) {
                output.push(PlannedBoundaryEdge {
                    source: PlannedBoundarySource::Curve {
                        curve,
                        span,
                        side: CurveTraceSide::Left,
                    },
                    behavior,
                    face: edge.left,
                    region,
                    traces: [edge.traces[0].left, edge.traces[1].left],
                    points: edge.points,
                    parameter: edge.parameter,
                });
            }
            // Both sides, even when they name the same face. A transmitting
            // edge interior to one face - a divider bridging two loops, or one
            // that ends in open space - is walked twice by that face, and each
            // direction needs its own atom: carving follows the second one back
            // around the free tip, and a probe on the second side has a trace
            // to sample. The two carry the same trace vertices in the opposite
            // order, so nothing new is introduced by the pair; every reader of
            // this list asks whether some atom matches rather than summing it.
            if let Some(region) = side(edge.right) {
                output.push(PlannedBoundaryEdge {
                    source: PlannedBoundarySource::Curve {
                        curve,
                        span,
                        side: CurveTraceSide::Right,
                    },
                    behavior,
                    face: edge.right,
                    region,
                    traces: [edge.traces[1].right, edge.traces[0].right],
                    points: [edge.points[1], edge.points[0]],
                    parameter: [edge.parameter[1], edge.parameter[0]],
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CurveNode, CurveSpan, CurveSpline, DomainRect, OpenCubicSpline, PeriodicCubicSpline,
        TopologyCurve, TopologyGeometry, TopologyVertex, TopologyVertexId, TopologyVertexLocation,
        compile_topology,
    };

    fn spans(start: u64, count: usize, behavior: SpanBehavior) -> Vec<CurveSpan> {
        (0..count)
            .map(|index| CurveSpan {
                id: CurveSpanId(start + index as u64),
                behavior,
            })
            .collect()
    }

    fn square(behavior: SpanBehavior) -> TopologyCurve {
        let spline = PeriodicCubicSpline::polygon(vec![
            Point2::new(-0.5, -0.5),
            Point2::new(0.5, -0.5),
            Point2::new(0.5, 0.5),
            Point2::new(-0.5, 0.5),
        ])
        .unwrap();
        TopologyCurve::new(
            CurveId(1),
            CurveSpline::Closed(spline),
            spans(1, 4, behavior),
        )
        .unwrap()
    }

    fn assign_each_face(topology: &TopologySnapshot) -> Vec<FaceRegionAssignment> {
        topology
            .faces
            .iter()
            .enumerate()
            .map(|(index, face)| FaceRegionAssignment {
                face: face.id,
                region: Some(RegionId(index as u64 + 1)),
            })
            .collect()
    }

    fn mesh_options() -> super::super::MeshingOptions {
        super::super::MeshingOptions {
            target_edge_length: 0.35,
            minimum_angle_degrees: 8.0,
            max_vertices: 20_000,
            max_triangles: 40_000,
            max_refinement_steps: 20_000,
            ..super::super::MeshingOptions::default()
        }
    }

    fn mesh_snapshot(topology: &TopologySnapshot, assignments: &[FaceRegionAssignment]) -> TriMesh {
        let plan = TopologyMeshPlan::new(topology, assignments).unwrap();
        mesh_topology_plan(&plan, 77, mesh_options()).unwrap()
    }

    /// The same, at an edge length the caller picks. Some arrangements only
    /// come apart once the boundary is sampled finely enough to put several
    /// vertices on each span.
    fn mesh_snapshot_at(
        topology: &TopologySnapshot,
        assignments: &[FaceRegionAssignment],
        target_edge_length: f64,
    ) -> Result<TriMesh, MeshError> {
        let plan = TopologyMeshPlan::new(topology, assignments).unwrap();
        mesh_topology_plan(
            &plan,
            77,
            super::super::MeshingOptions {
                target_edge_length,
                ..mesh_options()
            },
        )
    }

    fn run_topology_job(plan: TopologyMeshPlan, budget: usize) -> TriMesh {
        let mut job = TopologyMeshingJob::new(plan, 77, mesh_options());
        loop {
            if let Some(result) = job.advance(budget) {
                return result.unwrap();
            }
        }
    }

    fn mesh_area(mesh: &TriMesh) -> f64 {
        mesh.triangles
            .iter()
            .map(|triangle| {
                let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
                (b - a).cross(c - a) * 0.5
            })
            .sum()
    }

    fn rounded_hole_topology(behavior: SpanBehavior) -> TopologySnapshot {
        let spline = PeriodicCubicSpline::rounded(Point2::new(0.1, -0.05), 0.3);
        let curve = TopologyCurve::new(
            CurveId(4),
            CurveSpline::Closed(spline),
            spans(40, 8, behavior),
        )
        .unwrap();
        compile_topology(
            &TopologyGeometry {
                curves: vec![curve],
                ..TopologyGeometry::default()
            },
            19,
        )
        .unwrap()
    }

    /// Largest distance from the arrangement's own sample points inside an
    /// atom's parameter range to the atom's chord.
    fn chord_deviation(topology: &TopologySnapshot, atom: &PlannedBoundaryEdge) -> f64 {
        let [a, b] = atom.parameter;
        let (low, high) = (a.min(b), a.max(b));
        let chord = atom.points[1] - atom.points[0];
        let length_squared = chord.dot(chord);
        topology
            .edges
            .iter()
            .filter(|edge| match atom.source {
                PlannedBoundarySource::Outer(side) => {
                    edge.source == CompiledEdgeSource::Outer(side)
                }
                PlannedBoundarySource::Curve { span, .. } => {
                    edge.source == CompiledEdgeSource::Curve(span)
                }
            })
            .filter(|edge| {
                edge.parameter[0] >= low - 1.0e-12 && edge.parameter[1] <= high + 1.0e-12
            })
            .flat_map(|edge| edge.points)
            .map(|point| {
                let offset = point - atom.points[0];
                let along = (offset.dot(chord) / length_squared).clamp(0.0, 1.0);
                (offset - chord * along).norm()
            })
            .fold(0.0, f64::max)
    }

    /// Number of arrangement segments an atom covers; more than one means the
    /// atom is a merged run, which is what the chord cap bounds.
    fn segments_in(topology: &TopologySnapshot, atom: &PlannedBoundaryEdge) -> usize {
        let [a, b] = atom.parameter;
        let (low, high) = (a.min(b), a.max(b));
        topology
            .edges
            .iter()
            .filter(|edge| match atom.source {
                PlannedBoundarySource::Outer(side) => {
                    edge.source == CompiledEdgeSource::Outer(side)
                }
                PlannedBoundarySource::Curve { span, .. } => {
                    edge.source == CompiledEdgeSource::Curve(span)
                }
            })
            .filter(|edge| {
                edge.parameter[0] >= low - 1.0e-12 && edge.parameter[1] <= high + 1.0e-12
            })
            .count()
    }

    /// Traces of authored vertices that no atom of the plan ends at.
    fn unused_authored_traces(
        topology: &TopologySnapshot,
        plan: &TopologyMeshPlan,
    ) -> BTreeSet<TraceVertexId> {
        let endpoints = plan
            .boundaries
            .iter()
            .flat_map(|boundary| boundary.traces)
            .collect::<BTreeSet<_>>();
        topology
            .vertices
            .iter()
            .filter(|vertex| vertex.authored.is_some())
            .flat_map(|vertex| vertex.traces.iter().map(|trace| trace.id))
            .filter(|trace| !endpoints.contains(trace))
            .collect()
    }

    fn region_areas(mesh: &TriMesh) -> BTreeMap<RegionId, f64> {
        let mut areas = BTreeMap::new();
        for triangle in &mesh.triangles {
            let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
            *areas.entry(triangle.region).or_insert(0.0) += (b - a).cross(c - a) * 0.5;
        }
        areas
    }

    /// The arrangement cuts a curve into segments fine enough to intersect it
    /// robustly; the mesh plan merges them back into atoms within the curve
    /// tolerance. Both sides of a separated span merge over the same runs, the
    /// face cycles stay continuous, and the mesh keeps its region areas while
    /// its shortest boundary edge and the solver's time step grow several times.
    #[test]
    fn coarsening_merges_arrangement_segments_within_the_curve_tolerance() {
        let topology = rounded_hole_topology(SpanBehavior::REFLECTING);
        let assignments = assign_each_face(&topology);
        let fine = TopologyMeshPlan::new(&topology, &assignments).unwrap();
        let coarsening = AtomCoarsening {
            tolerance: 5.0e-4,
            maximum_chord: 0.15,
        };
        let coarse = fine.coarsened(&topology, coarsening).unwrap();
        let curve_atoms = |plan: &TopologyMeshPlan| {
            plan.boundaries
                .iter()
                .filter(|boundary| matches!(boundary.source, PlannedBoundarySource::Curve { .. }))
                .count()
        };
        assert!(
            curve_atoms(&fine) >= 4 * curve_atoms(&coarse),
            "{} fine against {} coarse atoms",
            curve_atoms(&fine),
            curve_atoms(&coarse)
        );
        let mut merged_atoms = 0;
        for atom in &coarse.boundaries {
            assert!(chord_deviation(&topology, atom) <= coarsening.tolerance + 1.0e-12);
            if segments_in(&topology, atom) > 1 {
                merged_atoms += 1;
                assert!(
                    (atom.points[1] - atom.points[0]).norm() <= coarsening.maximum_chord + 1.0e-12
                );
            }
        }
        assert!(merged_atoms > 0);
        assert_eq!(
            unused_authored_traces(&topology, &coarse),
            unused_authored_traces(&topology, &fine)
        );
        let ranges = |side: CurveTraceSide| {
            coarse
                .boundaries
                .iter()
                .filter(|boundary| {
                    matches!(boundary.source, PlannedBoundarySource::Curve { side: candidate, .. } if candidate == side)
                })
                .map(|boundary| {
                    let [a, b] = boundary.parameter;
                    (a.min(b).to_bits(), a.max(b).to_bits())
                })
                .collect::<BTreeSet<_>>()
        };
        assert!(!ranges(CurveTraceSide::Left).is_empty());
        assert_eq!(ranges(CurveTraceSide::Left), ranges(CurveTraceSide::Right));
        for domain in &coarse.domains {
            for cycle in &domain.steps {
                for (index, step) in cycle.iter().enumerate() {
                    let next = &cycle[(index + 1) % cycle.len()];
                    assert_eq!(step.boundary.traces[1], next.boundary.traces[0]);
                    assert!(coarse.boundaries.contains(&step.boundary));
                }
            }
        }
        assert_eq!(
            topology_mesh_update_action(&coarse, &coarse),
            TopologyMeshUpdateAction::Reuse
        );

        let options = super::super::MeshingOptions {
            target_edge_length: 0.15,
            curve_tolerance: 5.0e-4,
            minimum_angle_degrees: 12.0,
            max_vertices: 40_000,
            max_triangles: 80_000,
            max_refinement_steps: 40_000,
        };
        let fine_mesh = mesh_topology_plan(&fine, 78, options).unwrap();
        let coarse_mesh = mesh_topology_plan(&coarse, 79, options).unwrap();
        for (region, area) in region_areas(&fine_mesh) {
            assert!((region_areas(&coarse_mesh)[&region] - area).abs() < 2.0e-3);
        }
        assert!(coarse_mesh.boundary_edges.len() * 2 < fine_mesh.boundary_edges.len());
        let scene = crate::Scene {
            regions: assignments
                .iter()
                .map(|assignment| crate::Region {
                    id: assignment.region.unwrap(),
                    material: crate::DEFAULT_MATERIAL,
                    frame: crate::MaterialFrame::world(),
                })
                .collect(),
            ..crate::Scene::default()
        };
        let time_step = |plan: &TopologyMeshPlan, mesh: &TriMesh| {
            crate::QuadraticWaveOperator::assemble_topology(
                mesh,
                plan,
                crate::TopologyWaveModel::from_scene(&scene),
            )
            .unwrap()
            .recommended_time_step()
        };
        let (fine_step, coarse_step) = (
            time_step(&fine, &fine_mesh),
            time_step(&coarse, &coarse_mesh),
        );
        assert!(
            coarse_step > 3.0 * fine_step,
            "time step {fine_step:.3e} fine against {coarse_step:.3e} coarse"
        );
    }

    /// Runs never cross an authored vertex, so a junction keeps every sector
    /// trace as an atom endpoint, and both faces of a transmitting divider still
    /// share one chain per merged atom.
    #[test]
    fn coarsening_keeps_junctions_and_shared_transmitting_chains() {
        let topology = t_junction_topology();
        let assignments = assign_each_face(&topology);
        let fine = TopologyMeshPlan::new(&topology, &assignments).unwrap();
        let coarse = fine
            .coarsened(
                &topology,
                AtomCoarsening {
                    tolerance: 1.0e-3,
                    maximum_chord: 0.35,
                },
            )
            .unwrap();
        // Coarsening loses no authored trace the fine plan used; sector traces
        // the fine plan never referenced stay unreferenced.
        assert_eq!(
            unused_authored_traces(&topology, &coarse),
            unused_authored_traces(&topology, &fine)
        );
        assert!(coarse.boundaries.len() <= fine.boundaries.len());
        assert_eq!(coarse.junctions, fine.junctions);
        for atom in &coarse.boundaries {
            if segments_in(&topology, atom) > 1 {
                assert!((atom.points[1] - atom.points[0]).norm() <= 0.35 + 1.0e-12);
            }
        }
        let mesh = mesh_topology_plan(&coarse, 80, mesh_options()).unwrap();
        assert!((mesh_area(&mesh) - 4.0).abs() < 1.0e-9);
        assert_eq!(
            mesh.triangles
                .iter()
                .map(|triangle| triangle.region)
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
    }

    #[test]
    fn topology_mesher_builds_the_empty_domain_with_trace_lineage() {
        let topology = compile_topology(&TopologyGeometry::default(), 14).unwrap();
        let mesh = mesh_snapshot(&topology, &assign_each_face(&topology));
        assert_eq!(mesh.geometry_revision, 14);
        assert_eq!(mesh.mesh_revision, 77);
        assert!((mesh_area(&mesh) - 4.0).abs() < 1.0e-10);
        assert!(
            mesh.vertices
                .iter()
                .filter(|vertex| vertex.trace.is_some())
                .count()
                >= 4
        );
        assert!(
            mesh.boundary_edges
                .iter()
                .all(|edge| matches!(edge.label, BoundaryLabel::Outer(_)))
        );
    }

    #[test]
    fn topology_meshing_is_deterministic_across_work_slice_sizes() {
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![square(SpanBehavior::Transmitting)],
                ..TopologyGeometry::default()
            },
            15,
        )
        .unwrap();
        let plan = TopologyMeshPlan::new(&topology, &assign_each_face(&topology)).unwrap();
        let one_unit = run_topology_job(plan.clone(), 1);
        let large_slice = run_topology_job(plan, 16_384);
        assert_eq!(one_unit, large_slice);
    }

    #[test]
    fn topology_mesher_shares_a_transmitting_chain_between_regions() {
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![square(SpanBehavior::Transmitting)],
                ..TopologyGeometry::default()
            },
            15,
        )
        .unwrap();
        let mesh = mesh_snapshot(&topology, &assign_each_face(&topology));
        assert!((mesh_area(&mesh) - 4.0).abs() < 1.0e-9);
        assert_eq!(
            mesh.triangles
                .iter()
                .map(|triangle| triangle.region)
                .collect::<BTreeSet<_>>()
                .len(),
            2
        );
        for edge in mesh.boundary_edges.iter().filter(|edge| {
            matches!(
                edge.label,
                BoundaryLabel::Curve {
                    separated: false,
                    ..
                }
            )
        }) {
            assert_eq!(
                mesh.triangles
                    .iter()
                    .filter(|triangle| {
                        triangle.vertices.contains(&edge.vertices[0])
                            && triangle.vertices.contains(&edge.vertices[1])
                    })
                    .count(),
                2
            );
        }
    }

    #[test]
    fn topology_mesher_turns_a_transmitting_hole_edge_into_a_wall() {
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![square(SpanBehavior::Transmitting)],
                ..TopologyGeometry::default()
            },
            16,
        )
        .unwrap();
        let hole = topology.face_at(Point2::new(0.0, 0.0)).unwrap();
        let assignments = topology
            .faces
            .iter()
            .map(|face| FaceRegionAssignment {
                face: face.id,
                region: (face.id != hole).then_some(RegionId(1)),
            })
            .collect::<Vec<_>>();
        let mesh = mesh_snapshot(&topology, &assignments);
        assert!((mesh_area(&mesh) - 3.0).abs() < 1.0e-9);
        assert!(mesh.boundary_edges.iter().any(|edge| matches!(
            edge.label,
            BoundaryLabel::Curve {
                separated: true,
                ..
            }
        )));
    }

    #[test]
    fn topology_mesher_cuts_a_free_baffle_into_two_traces() {
        let spline = OpenCubicSpline::polyline(vec![
            Point2::new(-0.6, 0.0),
            Point2::new(0.0, 0.0),
            Point2::new(0.6, 0.0),
        ])
        .unwrap();
        let curve = TopologyCurve::new(
            CurveId(3),
            CurveSpline::Open(spline),
            spans(30, 2, SpanBehavior::REFLECTING),
        )
        .unwrap();
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![curve],
                ..TopologyGeometry::default()
            },
            17,
        )
        .unwrap();
        let mesh = mesh_snapshot(&topology, &assign_each_face(&topology));
        assert!((mesh_area(&mesh) - 4.0).abs() < 1.0e-9);
        let sides = mesh
            .boundary_edges
            .iter()
            .filter_map(|edge| match edge.label {
                BoundaryLabel::Curve { side, .. } => Some(side),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            sides,
            BTreeSet::from([CurveTraceSide::Left, CurveTraceSide::Right])
        );
    }

    #[test]
    fn topology_mesher_handles_an_outer_to_outer_divider_and_t_junction() {
        let topology = t_junction_topology();
        assert_eq!(topology.faces.len(), 3);
        let mesh = mesh_snapshot(&topology, &assign_each_face(&topology));
        assert!((mesh_area(&mesh) - 4.0).abs() < 1.0e-9);
        assert_eq!(
            mesh.triangles
                .iter()
                .map(|triangle| triangle.region)
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
    }

    /// A transmitting divider from the left to the right outer side with a
    /// branch from the bottom side meeting it at an authored junction.
    fn t_junction_topology() -> TopologySnapshot {
        let bottom = TopologyVertexId(10);
        let left = TopologyVertexId(11);
        let right = TopologyVertexId(12);
        let center = TopologyVertexId(13);
        let mut horizontal = TopologyCurve::new(
            CurveId(10),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-1.0, 0.0),
                    Point2::new(0.0, 0.0),
                    Point2::new(1.0, 0.0),
                ])
                .unwrap(),
            ),
            spans(100, 2, SpanBehavior::Transmitting),
        )
        .unwrap();
        horizontal.nodes[0].vertex = Some(left);
        horizontal.nodes[1].vertex = Some(center);
        horizontal.nodes[2].vertex = Some(right);
        let mut branch = TopologyCurve::new(
            CurveId(11),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(0.0, -1.0), Point2::new(0.0, 0.0)])
                    .unwrap(),
            ),
            spans(102, 1, SpanBehavior::Transmitting),
        )
        .unwrap();
        branch.nodes[0].vertex = Some(bottom);
        branch.nodes[1].vertex = Some(center);
        compile_topology(
            &TopologyGeometry {
                curves: vec![horizontal, branch],
                vertices: vec![
                    TopologyVertex {
                        id: bottom,
                        location: TopologyVertexLocation::Outer {
                            side: crate::OuterSide::Bottom,
                            fraction: 0.5,
                        },
                    },
                    TopologyVertex {
                        id: left,
                        location: TopologyVertexLocation::Outer {
                            side: crate::OuterSide::Left,
                            fraction: 0.5,
                        },
                    },
                    TopologyVertex {
                        id: right,
                        location: TopologyVertexLocation::Outer {
                            side: crate::OuterSide::Right,
                            fraction: 0.5,
                        },
                    },
                    TopologyVertex {
                        id: center,
                        location: TopologyVertexLocation::Interior(Point2::new(0.0, 0.0)),
                    },
                ],
                ..TopologyGeometry::default()
            },
            18,
        )
        .unwrap()
    }

    #[test]
    fn topology_mesher_recovers_three_separated_arms_at_a_junction() {
        let center = TopologyVertexId(30);
        let mut horizontal = TopologyCurve::new(
            CurveId(30),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-0.7, 0.0),
                    Point2::new(0.0, 0.0),
                    Point2::new(0.7, 0.0),
                ])
                .unwrap(),
            ),
            spans(300, 2, SpanBehavior::REFLECTING),
        )
        .unwrap();
        horizontal.nodes[1].vertex = Some(center);
        let mut branch = TopologyCurve::new(
            CurveId(31),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(0.0, 0.0), Point2::new(0.0, 0.7)])
                    .unwrap(),
            ),
            spans(302, 1, SpanBehavior::REFLECTING),
        )
        .unwrap();
        branch.nodes[0].vertex = Some(center);
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![horizontal, branch],
                vertices: vec![TopologyVertex {
                    id: center,
                    location: TopologyVertexLocation::Interior(Point2::new(0.0, 0.0)),
                }],
                ..TopologyGeometry::default()
            },
            20,
        )
        .unwrap();
        let plan = TopologyMeshPlan::new(&topology, &assign_each_face(&topology)).unwrap();
        assert_eq!(plan.domains.len(), 1);
        assert_eq!(plan.junctions[0].faces.len(), 3);
        let expected = plan
            .vertices
            .iter()
            .filter(|vertex| vertex.point == Point2::new(0.0, 0.0))
            .map(|vertex| vertex.id)
            .collect::<BTreeSet<_>>();
        assert_eq!(expected.len(), 3);

        let mesh = mesh_topology_plan(&plan, 77, mesh_options()).unwrap();
        assert!((mesh_area(&mesh) - 4.0).abs() < 1.0e-9);
        let actual = mesh
            .vertices
            .iter()
            .filter(|vertex| vertex.point == Point2::new(0.0, 0.0))
            .filter_map(|vertex| vertex.trace)
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
        assert!(mesh.boundary_edges.iter().all(|edge| {
            !matches!(
                edge.label,
                BoundaryLabel::Curve {
                    separated: true,
                    ..
                }
            ) || mesh
                .triangles
                .iter()
                .filter(|triangle| {
                    triangle.vertices.contains(&edge.vertices[0])
                        && triangle.vertices.contains(&edge.vertices[1])
                })
                .count()
                == 1
        }));
    }

    #[test]
    fn topology_mesher_recovers_a_separated_branch_on_a_transmitting_divider() {
        let left = TopologyVertexId(40);
        let right = TopologyVertexId(41);
        let center = TopologyVertexId(42);
        let mut divider = TopologyCurve::new(
            CurveId(40),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-1.0, 0.0),
                    Point2::new(0.0, 0.0),
                    Point2::new(1.0, 0.0),
                ])
                .unwrap(),
            ),
            spans(400, 2, SpanBehavior::Transmitting),
        )
        .unwrap();
        divider.nodes[0].vertex = Some(left);
        divider.nodes[1].vertex = Some(center);
        divider.nodes[2].vertex = Some(right);
        let mut branch = TopologyCurve::new(
            CurveId(41),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(0.0, 0.0), Point2::new(0.0, 0.7)])
                    .unwrap(),
            ),
            spans(402, 1, SpanBehavior::REFLECTING),
        )
        .unwrap();
        branch.nodes[0].vertex = Some(center);
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![divider, branch],
                vertices: vec![
                    TopologyVertex {
                        id: left,
                        location: TopologyVertexLocation::Outer {
                            side: crate::OuterSide::Left,
                            fraction: 0.5,
                        },
                    },
                    TopologyVertex {
                        id: right,
                        location: TopologyVertexLocation::Outer {
                            side: crate::OuterSide::Right,
                            fraction: 0.5,
                        },
                    },
                    TopologyVertex {
                        id: center,
                        location: TopologyVertexLocation::Interior(Point2::new(0.0, 0.0)),
                    },
                ],
                ..TopologyGeometry::default()
            },
            21,
        )
        .unwrap();
        let plan = TopologyMeshPlan::new(&topology, &assign_each_face(&topology)).unwrap();
        assert_eq!(plan.domains.len(), 2);
        assert_eq!(
            plan.junctions
                .iter()
                .find(|junction| junction.vertex == center)
                .unwrap()
                .faces
                .len(),
            2
        );
        let mesh = mesh_topology_plan(&plan, 78, mesh_options()).unwrap();
        assert!((mesh_area(&mesh) - 4.0).abs() < 1.0e-9);
        assert_eq!(
            mesh.triangles
                .iter()
                .map(|triangle| triangle.region)
                .collect::<BTreeSet<_>>()
                .len(),
            2
        );
        let center_traces = mesh
            .vertices
            .iter()
            .filter(|vertex| {
                (vertex.point - Point2::new(0.0, 0.0)).norm() <= mesh_options().curve_tolerance
            })
            .filter_map(|vertex| vertex.trace)
            .collect::<BTreeSet<_>>();
        // The transmitting divider joins both sides around the branch point,
        // so the exact junction node has one conforming trace even though the
        // branch itself has two separated faces away from the endpoint.
        assert_eq!(center_traces.len(), 1);
    }

    #[test]
    fn topology_mesher_handles_a_one_ended_baffle() {
        let outer = TopologyVertexId(20);
        let mut curve = TopologyCurve::new(
            CurveId(20),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.0, 0.0),
                    Point2::new(0.5, 0.0),
                    Point2::new(1.0, 0.0),
                ])
                .unwrap(),
            ),
            spans(200, 2, SpanBehavior::REFLECTING),
        )
        .unwrap();
        curve.nodes[2].vertex = Some(outer);
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![curve],
                vertices: vec![TopologyVertex {
                    id: outer,
                    location: TopologyVertexLocation::Outer {
                        side: crate::OuterSide::Right,
                        fraction: 0.5,
                    },
                }],
                ..TopologyGeometry::default()
            },
            19,
        )
        .unwrap();
        let plan = TopologyMeshPlan::new(&topology, &assign_each_face(&topology)).unwrap();
        let expected_traces = plan
            .vertices
            .iter()
            .map(|vertex| vertex.id)
            .collect::<BTreeSet<_>>();
        let mesh = mesh_topology_plan(&plan, 77, mesh_options()).unwrap();
        assert!((mesh_area(&mesh) - 4.0).abs() < 1.0e-9);
        let actual_traces = mesh
            .vertices
            .iter()
            .filter_map(|vertex| vertex.trace)
            .collect::<BTreeSet<_>>();
        assert_eq!(actual_traces, expected_traces);
    }

    #[test]
    fn topology_update_policy_reuses_exact_geometry_and_types_rebuilds() {
        let geometry = TopologyGeometry {
            curves: vec![square(SpanBehavior::Transmitting)],
            ..TopologyGeometry::default()
        };
        let first_snapshot = compile_topology(&geometry, 1).unwrap();
        let first =
            TopologyMeshPlan::new(&first_snapshot, &assign_each_face(&first_snapshot)).unwrap();
        let identical_snapshot = compile_topology(&geometry, 2).unwrap();
        let identical =
            TopologyMeshPlan::new(&identical_snapshot, &assign_each_face(&identical_snapshot))
                .unwrap();
        assert_eq!(
            topology_mesh_update_action(&first, &identical),
            TopologyMeshUpdateAction::Reuse
        );

        let mut moved_geometry = geometry.clone();
        moved_geometry.curves[0]
            .spline
            .set_node_point(0, Point2::new(-0.6, -0.5))
            .unwrap();
        let moved_snapshot = compile_topology(&moved_geometry, 3).unwrap();
        let moved =
            TopologyMeshPlan::new(&moved_snapshot, &assign_each_face(&moved_snapshot)).unwrap();
        assert_eq!(
            topology_mesh_update_action(&first, &moved),
            TopologyMeshUpdateAction::Repair(TopologyRepairReason::CurveOrJunctionMoved)
        );

        let mut reassigned = identical.clone();
        reassigned.domains[0].region = RegionId(99);
        assert_eq!(
            topology_mesh_update_action(&first, &reassigned),
            TopologyMeshUpdateAction::Repair(TopologyRepairReason::FaceAssignmentsChanged)
        );
        let mut resized = identical.clone();
        resized.domain.max_x += 0.5;
        assert_eq!(
            topology_mesh_update_action(&first, &resized),
            TopologyMeshUpdateAction::FullRebuild(TopologyFullRebuildReason::DomainChanged)
        );

        let separated_snapshot = compile_topology(
            &TopologyGeometry {
                curves: vec![square(SpanBehavior::REFLECTING)],
                ..TopologyGeometry::default()
            },
            4,
        )
        .unwrap();
        let separated =
            TopologyMeshPlan::new(&separated_snapshot, &assign_each_face(&separated_snapshot))
                .unwrap();
        let mut changed_law = separated.clone();
        for boundary in &mut changed_law.boundaries {
            if let Some(SpanBehavior::Separated { left, .. }) = &mut boundary.behavior {
                *left = crate::FaceBoundaryCondition::Impedance { ratio: 2.0 };
            }
        }
        assert_eq!(
            topology_mesh_update_action(&separated, &changed_law),
            TopologyMeshUpdateAction::Reuse
        );
    }

    #[test]
    fn transmitting_loop_shares_trace_vertices_between_region_domains() {
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![square(SpanBehavior::Transmitting)],
                ..TopologyGeometry::default()
            },
            9,
        )
        .unwrap();
        let plan = TopologyMeshPlan::new(&topology, &assign_each_face(&topology)).unwrap();
        assert_eq!(plan.geometry_revision, 9);
        assert_eq!(plan.domains.len(), 2);
        let curve_edges = plan
            .boundaries
            .iter()
            .filter(|edge| matches!(edge.source, PlannedBoundarySource::Curve { .. }))
            .collect::<Vec<_>>();
        assert_eq!(curve_edges.len(), 8);
        let unique = curve_edges
            .iter()
            .flat_map(|edge| edge.traces)
            .collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), 4);
    }

    #[test]
    fn excluded_loop_face_becomes_a_one_sided_boundary() {
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![square(SpanBehavior::REFLECTING)],
                ..TopologyGeometry::default()
            },
            0,
        )
        .unwrap();
        let inner = topology.face_at(Point2::new(0.0, 0.0)).unwrap();
        let assignments = topology
            .faces
            .iter()
            .map(|face| FaceRegionAssignment {
                face: face.id,
                region: (face.id != inner).then_some(RegionId(1)),
            })
            .collect::<Vec<_>>();
        let plan = TopologyMeshPlan::new(&topology, &assignments).unwrap();
        assert_eq!(plan.domains.len(), 1);
        assert_eq!(
            plan.boundaries
                .iter()
                .filter(|edge| matches!(edge.source, PlannedBoundarySource::Curve { .. }))
                .count(),
            4
        );
    }

    fn walled_loop_plan(behavior: SpanBehavior) -> TopologyMeshPlan {
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![square(behavior)],
                ..TopologyGeometry::default()
            },
            0,
        )
        .unwrap();
        let inner = topology.face_at(Point2::new(0.0, 0.0)).unwrap();
        let assignments = topology
            .faces
            .iter()
            .map(|face| FaceRegionAssignment {
                face: face.id,
                region: (face.id != inner).then_some(RegionId(1)),
            })
            .collect::<Vec<_>>();
        TopologyMeshPlan::new(&topology, &assignments).unwrap()
    }

    /// A span with an excluded face on one side is walled whether it is
    /// authored transmitting or separated, so the two plans hold the same
    /// atoms in the same places and a mesh can be reused across the switch.
    /// The arrangement still renumbers the traces, though - a transmitting
    /// vertex carries one for both faces where a separated one carries a pair
    /// - and the remap is what carries the mesh over.
    #[test]
    fn a_walled_span_renumbers_its_traces_without_moving_its_atoms() {
        let separated = walled_loop_plan(SpanBehavior::REFLECTING);
        let transmitting = walled_loop_plan(SpanBehavior::Transmitting);
        assert_eq!(
            topology_mesh_update_action(&separated, &transmitting),
            TopologyMeshUpdateAction::Reuse
        );
        let remap = topology_trace_remap(&separated, &transmitting)
            .expect("the atoms line the two numberings up");
        assert!(
            remap.iter().any(|(from, to)| from != to),
            "the switch renumbered the traces: {remap:?}"
        );
        let point = |plan: &TopologyMeshPlan, id: TraceVertexId| {
            plan.vertices
                .iter()
                .find(|vertex| vertex.id == id)
                .map(|vertex| vertex.point)
        };
        for (from, to) in &remap {
            assert_eq!(point(&separated, *from), point(&transmitting, *to));
        }
        assert_eq!(
            remap.values().copied().collect::<BTreeSet<_>>(),
            transmitting
                .vertices
                .iter()
                .map(|vertex| vertex.id)
                .collect::<BTreeSet<_>>()
        );
        assert_eq!(
            topology_trace_remap(&transmitting, &separated).map(|back| back.len()),
            Some(remap.len())
        );
    }

    /// Plans that do not hold the same boundaries in the same places have no
    /// correspondence to offer, and say so rather than guessing one.
    #[test]
    fn a_moved_boundary_has_no_trace_remap() {
        let plan = walled_loop_plan(SpanBehavior::REFLECTING);
        let moved = {
            let spline = PeriodicCubicSpline::polygon(vec![
                Point2::new(-0.4, -0.5),
                Point2::new(0.5, -0.5),
                Point2::new(0.5, 0.5),
                Point2::new(-0.5, 0.5),
            ])
            .unwrap();
            let curve = TopologyCurve::new(
                CurveId(1),
                CurveSpline::Closed(spline),
                spans(1, 4, SpanBehavior::REFLECTING),
            )
            .unwrap();
            let topology = compile_topology(
                &TopologyGeometry {
                    curves: vec![curve],
                    ..TopologyGeometry::default()
                },
                0,
            )
            .unwrap();
            let inner = topology.face_at(Point2::new(0.0, 0.0)).unwrap();
            let assignments = topology
                .faces
                .iter()
                .map(|face| FaceRegionAssignment {
                    face: face.id,
                    region: (face.id != inner).then_some(RegionId(1)),
                })
                .collect::<Vec<_>>();
            TopologyMeshPlan::new(&topology, &assignments).unwrap()
        };
        assert!(topology_trace_remap(&plan, &moved).is_none());
    }

    #[test]
    fn free_baffle_keeps_separate_interior_traces_and_reconnects_tips() {
        let spline = OpenCubicSpline::polyline(vec![
            Point2::new(-0.5, 0.0),
            Point2::new(0.0, 0.0),
            Point2::new(0.5, 0.0),
        ])
        .unwrap();
        let curve = TopologyCurve::new(
            CurveId(1),
            CurveSpline::Open(spline),
            spans(1, 2, SpanBehavior::REFLECTING),
        )
        .unwrap();
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![curve],
                ..TopologyGeometry::default()
            },
            0,
        )
        .unwrap();
        let plan = TopologyMeshPlan::new(
            &topology,
            &[FaceRegionAssignment {
                face: topology.faces[0].id,
                region: Some(RegionId(1)),
            }],
        )
        .unwrap();
        let traces_at = |point: Point2| {
            plan.boundaries
                .iter()
                .filter(|boundary| matches!(boundary.source, PlannedBoundarySource::Curve { .. }))
                .flat_map(|boundary| boundary.points.into_iter().zip(boundary.traces))
                .filter_map(|(candidate, trace)| (candidate == point).then_some(trace))
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(traces_at(Point2::new(-0.5, 0.0)).len(), 1);
        assert_eq!(traces_at(Point2::new(0.5, 0.0)).len(), 1);
        assert_eq!(traces_at(Point2::new(0.0, 0.0)).len(), 2);
    }

    #[test]
    fn transmitting_face_against_excluded_face_is_effectively_reflecting() {
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![square(SpanBehavior::Transmitting)],
                ..TopologyGeometry::default()
            },
            0,
        )
        .unwrap();
        assert!(matches!(
            TopologyMeshPlan::new(
                &topology,
                &[FaceRegionAssignment {
                    face: topology.faces[0].id,
                    region: Some(RegionId(1)),
                }]
            ),
            Err(TopologyMeshPlanError::MissingFace(_))
        ));
        let assignments = topology
            .faces
            .iter()
            .enumerate()
            .map(|(index, face)| FaceRegionAssignment {
                face: face.id,
                region: (index == 0).then_some(RegionId(1)),
            })
            .collect::<Vec<_>>();
        let plan = TopologyMeshPlan::new(&topology, &assignments).unwrap();
        assert!(plan.boundaries.iter().all(|boundary| {
            !matches!(boundary.source, PlannedBoundarySource::Curve { .. })
                || boundary.behavior == Some(SpanBehavior::REFLECTING)
        }));
    }

    #[test]
    fn outer_resize_and_junction_trace_membership_survive_projection() {
        let junction = TopologyVertexId(1);
        let mut spline =
            OpenCubicSpline::polyline(vec![Point2::new(0.0, -1.0), Point2::new(0.0, 1.0)]).unwrap();
        spline
            .set_breakpoint_point(0, Point2::new(0.0, -1.0))
            .unwrap();
        let mut curve = TopologyCurve::new(
            CurveId(1),
            CurveSpline::Open(spline),
            spans(1, 1, SpanBehavior::Transmitting),
        )
        .unwrap();
        curve.nodes = vec![
            CurveNode {
                vertex: Some(junction),
            },
            CurveNode {
                vertex: Some(TopologyVertexId(2)),
            },
        ];
        let topology = compile_topology(
            &TopologyGeometry {
                domain: DomainRect::UNIT,
                curves: vec![curve],
                vertices: vec![
                    TopologyVertex {
                        id: junction,
                        location: TopologyVertexLocation::Outer {
                            side: crate::OuterSide::Bottom,
                            fraction: 0.5,
                        },
                    },
                    TopologyVertex {
                        id: TopologyVertexId(2),
                        location: TopologyVertexLocation::Outer {
                            side: crate::OuterSide::Top,
                            fraction: 0.5,
                        },
                    },
                ],
            },
            0,
        )
        .unwrap();
        let plan = TopologyMeshPlan::new(&topology, &assign_each_face(&topology)).unwrap();
        assert!(
            plan.vertices
                .iter()
                .any(|vertex| vertex.point == Point2::new(0.0, -1.0))
        );
    }

    /// The node of a closed curve that sits on `point`, for attaching a bridge
    /// to a corner of one of these test polygons.
    fn node_at(curve: &TopologyCurve, point: Point2) -> usize {
        (0..curve.spline.node_count())
            .find(|node| curve.spline.node_point(*node) == Some(point))
            .expect("a node at that point")
    }

    fn box_curve(
        id: u64,
        spans_from: u64,
        centre: Point2,
        behavior: SpanBehavior,
    ) -> TopologyCurve {
        let spline = PeriodicCubicSpline::polygon(vec![
            Point2::new(centre.x - 0.2, centre.y - 0.25),
            Point2::new(centre.x + 0.2, centre.y - 0.25),
            Point2::new(centre.x + 0.2, centre.y + 0.25),
            Point2::new(centre.x - 0.2, centre.y + 0.25),
        ])
        .unwrap();
        TopologyCurve::new(
            CurveId(id),
            CurveSpline::Closed(spline),
            spans(spans_from, 4, behavior),
        )
        .unwrap()
    }

    fn orphan_vertices(mesh: &TriMesh) -> usize {
        let used = mesh
            .triangles
            .iter()
            .flat_map(|triangle| triangle.vertices)
            .collect::<BTreeSet<_>>();
        mesh.vertices.len() - used.len()
    }

    /// A slit lying on a line of symmetry of the domain collects mesh vertices
    /// exactly on itself, and a vertex in the way of a constrained segment
    /// leaves no edge to flip: the way through is a point, not a gap. The
    /// segment is split there instead.
    #[test]
    fn topology_mesher_recovers_a_slit_along_the_domain_centre_line() {
        for points in [
            vec![
                Point2::new(-0.6, 0.0),
                Point2::new(0.0, 0.0),
                Point2::new(0.6, 0.0),
            ],
            vec![Point2::new(0.0, -0.6), Point2::new(0.0, 0.6)],
        ] {
            let spans = spans(900, points.len() - 1, SpanBehavior::REFLECTING);
            let curve = TopologyCurve::new(
                CurveId(90),
                CurveSpline::Open(OpenCubicSpline::polyline(points.clone()).unwrap()),
                spans,
            )
            .unwrap();
            let topology = compile_topology(
                &TopologyGeometry {
                    curves: vec![curve],
                    ..TopologyGeometry::default()
                },
                22,
            )
            .unwrap();
            for edge in [0.18, 0.08] {
                // The application's own settings: a coarsened plan and the
                // default minimum angle, which is what puts refinement vertices
                // on the centre line in the first place.
                let options = super::super::MeshingOptions {
                    target_edge_length: edge,
                    curve_tolerance: (edge * 0.02).min(5.0e-4),
                    ..super::super::MeshingOptions::default()
                };
                let plan = TopologyMeshPlan::new(&topology, &assign_each_face(&topology))
                    .unwrap()
                    .coarsened(&topology, crate::AtomCoarsening::from_meshing(options))
                    .unwrap();
                let mesh = mesh_topology_plan(&plan, 77, options)
                    .unwrap_or_else(|error| panic!("{points:?} at {edge}: {error}"));
                assert!(
                    (mesh_area(&mesh) - 4.0).abs() < 1.0e-9,
                    "{points:?} at {edge}"
                );
                assert_eq!(orphan_vertices(&mesh), 0, "{points:?} at {edge}");
                let sides = mesh
                    .boundary_edges
                    .iter()
                    .filter_map(|edge| match edge.label {
                        BoundaryLabel::Curve { side, .. } => Some(side),
                        _ => None,
                    })
                    .collect::<BTreeSet<_>>();
                assert_eq!(
                    sides,
                    BTreeSet::from([CurveTraceSide::Left, CurveTraceSide::Right]),
                    "{points:?} at {edge}: the slit is cut into two traces"
                );
            }
        }
    }

    /// A baffle between two loops is a slit whose removal breaks the face's
    /// boundary into two: the cycle around the background runs through both
    /// loops and crosses the bridge twice. Keeping it as one polygon leaves a
    /// zero-width channel between the loops that no refinement can ever fill.
    #[test]
    fn topology_mesher_bridges_two_loops_with_a_baffle() {
        let left_vertex = TopologyVertexId(60);
        let right_vertex = TopologyVertexId(61);
        let mut left = box_curve(60, 600, Point2::new(-0.5, 0.0), SpanBehavior::Transmitting);
        let mut right = box_curve(61, 610, Point2::new(0.5, 0.0), SpanBehavior::Transmitting);
        let start = Point2::new(-0.3, -0.25);
        let end = Point2::new(0.3, -0.25);
        let node = node_at(&left, start);
        left.nodes[node].vertex = Some(left_vertex);
        let node = node_at(&right, end);
        right.nodes[node].vertex = Some(right_vertex);
        let mut bridge = TopologyCurve::new(
            CurveId(62),
            CurveSpline::Open(OpenCubicSpline::polyline(vec![start, end]).unwrap()),
            spans(620, 1, SpanBehavior::REFLECTING),
        )
        .unwrap();
        bridge.nodes[0].vertex = Some(left_vertex);
        bridge.nodes[1].vertex = Some(right_vertex);
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![left, right, bridge],
                vertices: vec![
                    TopologyVertex {
                        id: left_vertex,
                        location: TopologyVertexLocation::Interior(start),
                    },
                    TopologyVertex {
                        id: right_vertex,
                        location: TopologyVertexLocation::Interior(end),
                    },
                ],
                ..TopologyGeometry::default()
            },
            19,
        )
        .unwrap();
        assert_eq!(topology.faces.len(), 3);
        let mesh = mesh_snapshot(&topology, &assign_each_face(&topology));
        assert!((mesh_area(&mesh) - 4.0).abs() < 1.0e-9);
        assert_eq!(orphan_vertices(&mesh), 0);
        assert_eq!(
            mesh.triangles
                .iter()
                .map(|triangle| triangle.region)
                .collect::<BTreeSet<_>>()
                .len(),
            3,
            "both loops keep their own region beside the background"
        );
        // The bridge is a slit: the background meets it from both sides.
        let sides = mesh
            .boundary_edges
            .iter()
            .filter_map(|edge| match edge.label {
                BoundaryLabel::Curve {
                    curve: CurveId(62),
                    side,
                    ..
                } => Some(side),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            sides,
            BTreeSet::from([CurveTraceSide::Left, CurveTraceSide::Right])
        );
    }

    /// The same shape against the outer boundary, where the split cycle is the
    /// face's outer one: what is left of it is the domain's own boundary plus a
    /// hole, so which run is which can no longer come from their order.
    #[test]
    fn topology_mesher_bridges_a_loop_to_the_outer_boundary() {
        let corner = TopologyVertexId(70);
        let wall = TopologyVertexId(71);
        let mut ring = box_curve(70, 700, Point2::new(0.0, 0.0), SpanBehavior::Transmitting);
        let start = Point2::new(-0.2, 0.25);
        let node = node_at(&ring, start);
        ring.nodes[node].vertex = Some(corner);
        let mut bridge = TopologyCurve::new(
            CurveId(71),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![start, Point2::new(-0.2, 1.0)]).unwrap(),
            ),
            spans(710, 1, SpanBehavior::REFLECTING),
        )
        .unwrap();
        bridge.nodes[0].vertex = Some(corner);
        bridge.nodes[1].vertex = Some(wall);
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![ring, bridge],
                vertices: vec![
                    TopologyVertex {
                        id: corner,
                        location: TopologyVertexLocation::Interior(start),
                    },
                    TopologyVertex {
                        id: wall,
                        location: TopologyVertexLocation::Outer {
                            // The top side runs right to left, so this is
                            // x = 1 - 2 * 0.6 = -0.2, over the bridge's end.
                            side: crate::OuterSide::Top,
                            fraction: 0.6,
                        },
                    },
                ],
                ..TopologyGeometry::default()
            },
            20,
        )
        .unwrap();
        let mesh = mesh_snapshot(&topology, &assign_each_face(&topology));
        assert!((mesh_area(&mesh) - 4.0).abs() < 1.0e-9);
        assert_eq!(orphan_vertices(&mesh), 0);
    }

    /// A curve that comes back to its own interior pinches its face at that
    /// point: the boundary leaves through one sector of the junction and
    /// returns through another, and both name the same place. This is the
    /// shape an editor produces by drawing from a curve's middle round to its
    /// own loose end - the loop comes first and the tail hangs off the pinch.
    #[test]
    fn topology_mesher_meshes_a_curve_that_touches_itself() {
        let pinch = TopologyVertexId(80);
        let point = Point2::new(-0.03, 0.13);
        let mut curve = TopologyCurve::new(
            CurveId(80),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![
                    point,
                    Point2::new(0.08, -0.49),
                    Point2::new(0.53, -0.71),
                    Point2::new(0.67, -0.29),
                    Point2::new(0.59, -0.11),
                    Point2::new(0.17, 0.07),
                    point,
                    Point2::new(-0.23, 0.19),
                    Point2::new(-0.61, 0.33),
                ])
                .unwrap(),
            ),
            spans(800, 8, SpanBehavior::REFLECTING),
        )
        .unwrap();
        curve.nodes[0].vertex = Some(pinch);
        curve.nodes[6].vertex = Some(pinch);
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![curve],
                vertices: vec![TopologyVertex {
                    id: pinch,
                    location: TopologyVertexLocation::Interior(point),
                }],
                ..TopologyGeometry::default()
            },
            21,
        )
        .unwrap();
        assert_eq!(topology.faces.len(), 2, "the loop encloses one face");
        for edge in [0.35, 0.12] {
            let mesh = mesh_snapshot_at(&topology, &assign_each_face(&topology), edge)
                .unwrap_or_else(|error| panic!("{edge}: {error}"));
            assert!((mesh_area(&mesh) - 4.0).abs() < 1.0e-9, "{edge}");
            assert_eq!(orphan_vertices(&mesh), 0, "{edge}");
        }
    }
}
