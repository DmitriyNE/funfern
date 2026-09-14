//! Unified user-authored curves and a derived planar face arrangement.
//!
//! This module is intentionally independent from the legacy `Scene` object
//! categories. It is the migration target for closed loops, material dividers,
//! and baffles; application wiring is performed in later slices.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    DomainRect, FaceBoundaryCondition, InternalBoundaryCoupling, OpenCubicSpline, OpenSampler,
    OuterSide, PeriodicCubicSpline, Point2, PredicateSign, Sampler, SamplingOptions, orient2d,
    point_segment_distance,
};

pub const MAX_TOPOLOGY_CURVES: usize = 64;
pub const MAX_TOPOLOGY_VERTICES: usize = 256;
pub const MAX_TOPOLOGY_SEGMENTS: usize = 16_384;
pub const MAX_TOPOLOGY_FACES: usize = 256;
pub const MAX_TOPOLOGY_WORK: usize = 50_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CurveId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CurveSpanId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TopologyVertexId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaceId(pub u64);

pub const EXTERIOR_FACE: FaceId = FaceId(0);

/// Snapshot-local finite-element trace identity at an arrangement vertex.
/// Several trace vertices may occupy the same point when separated spans meet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TraceVertexId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TopologyVertexLocation {
    /// A deliberately free curve endpoint.
    Free(Point2),
    /// An unconstrained junction shared by two or more curve breakpoints.
    Interior(Point2),
    /// A junction constrained to one side of the rectangular outer domain.
    Outer { side: OuterSide, fraction: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TopologyVertex {
    pub id: TopologyVertexId,
    pub location: TopologyVertexLocation,
}

impl TopologyVertex {
    pub fn point(self, domain: DomainRect) -> Option<Point2> {
        match self.location {
            TopologyVertexLocation::Free(point) | TopologyVertexLocation::Interior(point) => {
                point.finite().then_some(point)
            }
            TopologyVertexLocation::Outer { side, fraction }
                if fraction.is_finite() && (0.0..=1.0).contains(&fraction) =>
            {
                Some(outer_point(domain, side, fraction))
            }
            TopologyVertexLocation::Outer { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CurveNode {
    pub vertex: Option<TopologyVertexId>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpanBehavior {
    /// One conforming finite-element trace with material transmission.
    Transmitting,
    /// Two coincident traces. Each active side receives its own law; an optional
    /// coupling acts between the traces.
    Separated {
        left: FaceBoundaryCondition,
        right: FaceBoundaryCondition,
        coupling: InternalBoundaryCoupling,
    },
}

impl SpanBehavior {
    pub const REFLECTING: Self = Self::Separated {
        left: FaceBoundaryCondition::Reflecting,
        right: FaceBoundaryCondition::Reflecting,
        coupling: InternalBoundaryCoupling::Independent,
    };

    pub fn valid(self) -> bool {
        match self {
            Self::Transmitting => true,
            Self::Separated {
                left,
                right,
                coupling,
            } => {
                left.valid()
                    && right.valid()
                    && coupling.valid()
                    && (!matches!(coupling, InternalBoundaryCoupling::ThinGap { .. })
                        || left == FaceBoundaryCondition::Reflecting
                            && right == FaceBoundaryCondition::Reflecting)
            }
        }
    }

    fn transmitting(self) -> bool {
        matches!(self, Self::Transmitting)
    }

    /// The same behaviour seen from the opposite curve direction: the two
    /// separated laws swap sides so each face keeps the law it had.
    pub fn mirrored(self) -> Self {
        match self {
            Self::Transmitting => Self::Transmitting,
            Self::Separated {
                left,
                right,
                coupling,
            } => Self::Separated {
                left: right,
                right: left,
                coupling,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveSpan {
    pub id: CurveSpanId,
    pub behavior: SpanBehavior,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CurveSpline {
    Closed(PeriodicCubicSpline),
    Open(OpenCubicSpline),
}

impl CurveSpline {
    pub fn span_count(&self) -> usize {
        match self {
            Self::Closed(spline) => spline.intervals().len(),
            Self::Open(spline) => spline.intervals().len(),
        }
    }

    pub fn node_count(&self) -> usize {
        match self {
            Self::Closed(spline) => spline.intervals().len(),
            Self::Open(spline) => spline.intervals().len() + 1,
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open(_))
    }

    pub fn node_parameter(&self, index: usize) -> Option<f64> {
        match self {
            Self::Closed(spline) => spline.knots().get(index).copied(),
            Self::Open(spline) => spline.breakpoint(index),
        }
    }

    pub fn node_point(&self, index: usize) -> Option<Point2> {
        let parameter = self.node_parameter(index)?;
        Some(match self {
            Self::Closed(spline) => spline.evaluate(parameter),
            Self::Open(spline) => spline.evaluate(parameter),
        })
    }

    pub fn span_bounds(&self, index: usize) -> Option<[f64; 2]> {
        match self {
            Self::Closed(spline) => spline.span_bounds(index),
            Self::Open(spline) => spline.span_bounds(index),
        }
    }

    pub fn set_node_point(
        &mut self,
        index: usize,
        point: Point2,
    ) -> Result<(), crate::SplineError> {
        match self {
            Self::Closed(spline) => spline.set_breakpoint_point(index, point),
            Self::Open(spline) => spline.set_breakpoint_point(index, point),
        }
    }

    fn sample(&self, options: SamplingOptions) -> CurveSampler {
        match self {
            Self::Closed(spline) => CurveSampler::Closed(Sampler::new(spline, options)),
            Self::Open(spline) => CurveSampler::Open(OpenSampler::new(spline, options)),
        }
    }

    fn span_index(&self, parameter: f64) -> usize {
        match self {
            Self::Closed(spline) => spline.span_index(parameter).unwrap(),
            Self::Open(spline) => spline.span_index(parameter).unwrap(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyCurve {
    pub id: CurveId,
    pub spline: CurveSpline,
    pub nodes: Vec<CurveNode>,
    pub spans: Vec<CurveSpan>,
}

impl TopologyCurve {
    pub fn new(
        id: CurveId,
        spline: CurveSpline,
        spans: Vec<CurveSpan>,
    ) -> Result<Self, TopologyIssue> {
        let nodes = vec![CurveNode::default(); spline.node_count()];
        let curve = Self {
            id,
            spline,
            nodes,
            spans,
        };
        curve.structure_valid()?;
        Ok(curve)
    }

    fn structure_valid(&self) -> Result<(), TopologyIssue> {
        if self.id.0 == 0
            || self.nodes.len() != self.spline.node_count()
            || self.spans.len() != self.spline.span_count()
            || self
                .spans
                .iter()
                .any(|span| span.id.0 == 0 || !span.behavior.valid())
        {
            return Err(TopologyIssue::Structure);
        }
        Ok(())
    }

    /// The same open curve traversed the other way. Spans and nodes reverse
    /// with the spline and every separated span swaps its laws, so each face
    /// keeps the law it had. Parameter `p` becomes `period - p`.
    pub fn reversed(&self) -> Result<Self, TopologyIssue> {
        let CurveSpline::Open(spline) = &self.spline else {
            return Err(TopologyIssue::Structure);
        };
        let curve = Self {
            id: self.id,
            spline: CurveSpline::Open(spline.reversed()),
            nodes: self.nodes.iter().rev().copied().collect(),
            spans: self
                .spans
                .iter()
                .rev()
                .map(|span| CurveSpan {
                    id: span.id,
                    behavior: span.behavior.mirrored(),
                })
                .collect(),
        };
        curve.structure_valid()?;
        Ok(curve)
    }

    /// Inserts or reuses a breakpoint, raises it to C0 without changing the
    /// curve, and associates it with a topology vertex. The original span ID is
    /// retained on the first piece and `new_span` is assigned to the second.
    pub fn insert_topology_vertex(
        &mut self,
        parameter: f64,
        vertex: TopologyVertexId,
        new_span: CurveSpanId,
    ) -> Result<TopologyInsertion, crate::SplineError> {
        if vertex.0 == 0 || new_span.0 == 0 || self.spans.iter().any(|span| span.id == new_span) {
            return Err(crate::SplineError::Index);
        }
        let old_span_count = self.spans.len();
        let old_span = self.spline.span_index(parameter);
        match &mut self.spline {
            CurveSpline::Open(spline) => {
                spline.insert(parameter)?;
                let node = closest_open_breakpoint(spline, parameter);
                let inserted = spline.intervals().len() != old_span_count;
                if inserted {
                    let behavior = self.spans[old_span].behavior;
                    self.spans.insert(
                        node,
                        CurveSpan {
                            id: new_span,
                            behavior,
                        },
                    );
                    self.nodes.insert(node, CurveNode::default());
                }
                while spline
                    .continuity(node)
                    .is_some_and(|continuity| continuity > 0)
                {
                    spline.increase_multiplicity(node)?;
                }
                self.nodes[node].vertex = Some(vertex);
                Ok(TopologyInsertion {
                    node,
                    inserted_span: inserted,
                })
            }
            CurveSpline::Closed(spline) => {
                spline.insert(parameter)?;
                let node = closest_periodic_breakpoint(spline, parameter);
                let inserted = spline.intervals().len() != old_span_count;
                if inserted {
                    let behavior = self.spans[old_span].behavior;
                    self.spans.insert(
                        node,
                        CurveSpan {
                            id: new_span,
                            behavior,
                        },
                    );
                    self.nodes.insert(node, CurveNode::default());
                }
                while spline
                    .continuity(node)
                    .is_some_and(|continuity| continuity > 0)
                {
                    spline.increase_multiplicity(node)?;
                }
                self.nodes[node].vertex = Some(vertex);
                Ok(TopologyInsertion {
                    node,
                    inserted_span: inserted,
                })
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TopologyInsertion {
    pub node: usize,
    pub inserted_span: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TopologyGeometry {
    pub domain: DomainRect,
    pub curves: Vec<TopologyCurve>,
    pub vertices: Vec<TopologyVertex>,
}

impl TopologyGeometry {
    pub fn curve(&self, id: CurveId) -> Option<&TopologyCurve> {
        self.curves.iter().find(|curve| curve.id == id)
    }

    /// Applies authoritative topology-vertex positions to every incident C0
    /// breakpoint. Call this after moving a junction or resizing the domain.
    pub fn synchronize_vertices(&mut self) -> Result<(), TopologyIssue> {
        let points = self
            .vertices
            .iter()
            .map(|vertex| {
                vertex
                    .point(self.domain)
                    .map(|point| (vertex.id, point))
                    .ok_or(TopologyIssue::Structure)
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        for curve in &mut self.curves {
            for (node_index, node) in curve.nodes.iter().enumerate() {
                if let Some(vertex) = node.vertex {
                    let point = *points.get(&vertex).ok_or(TopologyIssue::MissingVertex {
                        curve: curve.id,
                        vertex,
                    })?;
                    curve
                        .spline
                        .set_node_point(node_index, point)
                        .map_err(|_| TopologyIssue::NonC0Attachment {
                            curve: curve.id,
                            node: node_index,
                        })?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompiledEdgeSource {
    Outer(OuterSide),
    Curve(CurveSpanId),
}

/// Names an arrangement edge the way the status bar should say it out loud.
impl std::fmt::Display for CompiledEdgeSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Outer(side) => write!(
                formatter,
                "the {} boundary",
                match side {
                    OuterSide::Bottom => "bottom",
                    OuterSide::Right => "right",
                    OuterSide::Top => "top",
                    OuterSide::Left => "left",
                }
            ),
            Self::Curve(span) => write!(formatter, "span {}", span.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledEdge {
    pub source: CompiledEdgeSource,
    pub curve: Option<CurveId>,
    pub behavior: Option<SpanBehavior>,
    pub points: [Point2; 2],
    pub parameter: [f64; 2],
    pub left: FaceId,
    pub right: FaceId,
    pub traces: [CompiledEdgeTraces; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledEdgeTraces {
    pub left: TraceVertexId,
    pub right: TraceVertexId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledTraceVertex {
    pub id: TraceVertexId,
    pub face: FaceId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledFace {
    pub id: FaceId,
    /// The first cycle is the counter-clockwise outer boundary. Remaining
    /// clockwise or zero-area cycles are holes or slit traces in that face.
    pub cycles: Vec<Vec<Point2>>,
    /// The same cycles as directed references to `TopologySnapshot::edges`.
    /// Every step is oriented with this face on its left.
    pub boundaries: Vec<Vec<CompiledBoundaryStep>>,
    pub area: f64,
}

impl CompiledFace {
    /// Area-weighted centroid of the face with its holes removed. It is a
    /// representative interior point for a simply connected face, but not in
    /// general: an annulus puts it in the hole. That is the right answer for a
    /// radial material profile in a ring, which is what it is for. Falls back to
    /// the mean of the outer cycle when the cycles carry no usable area.
    pub fn centroid(&self) -> Option<Point2> {
        let mut area = 0.0;
        let mut moment = Point2::default();
        for cycle in &self.cycles {
            if cycle.len() < 3 {
                continue;
            }
            for (a, b) in cycle.iter().zip(cycle.iter().cycle().skip(1)) {
                let cross = a.x * b.y - b.x * a.y;
                area += cross;
                moment = moment + (*a + *b) * cross;
            }
        }
        let area = area * 0.5;
        if area.abs() > 1.0e-12 {
            let centroid = moment / (6.0 * area);
            if centroid.finite() {
                return Some(centroid);
            }
        }
        let outer = self.cycles.first()?;
        if outer.is_empty() {
            return None;
        }
        let mean = outer
            .iter()
            .fold(Point2::default(), |total, point| total + *point)
            / outer.len() as f64;
        mean.finite().then_some(mean)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledBoundaryStep {
    pub edge: usize,
    pub reversed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledVertex {
    pub point: Point2,
    pub authored: Option<TopologyVertexId>,
    pub traces: Vec<CompiledTraceVertex>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologySnapshot {
    pub revision: u64,
    pub domain: DomainRect,
    pub vertices: Vec<CompiledVertex>,
    pub faces: Vec<CompiledFace>,
    pub edges: Vec<CompiledEdge>,
}

impl TopologySnapshot {
    pub fn face(&self, id: FaceId) -> Option<&CompiledFace> {
        self.faces.iter().find(|face| face.id == id)
    }

    pub fn face_at(&self, point: Point2) -> Option<FaceId> {
        self.faces
            .iter()
            .filter(|face| point_in_polygon(point, &face.cycles[0]))
            .min_by(|left, right| left.area.total_cmp(&right.area))
            .map(|face| face.id)
    }

    pub fn span_edges(&self, span: CurveSpanId) -> impl Iterator<Item = &CompiledEdge> {
        self.edges
            .iter()
            .filter(move |edge| edge.source == CompiledEdgeSource::Curve(span))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyResult {
    pub revision: u64,
    pub result: Result<TopologySnapshot, TopologyIssue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyIssue {
    Structure,
    WorkLimit,
    Sampling(CurveId),
    MissingVertex {
        curve: CurveId,
        vertex: TopologyVertexId,
    },
    NonC0Attachment {
        curve: CurveId,
        node: usize,
    },
    VertexMismatch {
        curve: CurveId,
        vertex: TopologyVertexId,
    },
    Outside(CurveId),
    Degenerate(CurveId),
    NearContact {
        first: CompiledEdgeSource,
        second: CompiledEdgeSource,
    },
    Overlap {
        first: CompiledEdgeSource,
        second: CompiledEdgeSource,
    },
    IncompatibleCrossing {
        first: CompiledEdgeSource,
        second: CompiledEdgeSource,
    },
    FreeTransmittingEnd(CurveId),
    TooManySegments,
    TooManyFaces,
}

impl std::fmt::Display for TopologyIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Structure => formatter.write_str("a curve's spans and nodes do not line up"),
            Self::WorkLimit => formatter.write_str("tracing this geometry ran out of budget"),
            Self::Sampling(curve) => write!(formatter, "curve {} could not be sampled", curve.0),
            Self::MissingVertex { curve, .. } => {
                write!(
                    formatter,
                    "curve {} points at a junction that is gone",
                    curve.0
                )
            }
            Self::NonC0Attachment { curve, .. } => write!(
                formatter,
                "a junction on curve {} sits on a knot that is not a corner",
                curve.0
            ),
            Self::VertexMismatch { curve, .. } => {
                write!(formatter, "curve {} has drifted off its junction", curve.0)
            }
            Self::Outside(curve) => write!(formatter, "curve {} leaves the domain", curve.0),
            Self::Degenerate(curve) => {
                write!(formatter, "curve {} has a span with no length", curve.0)
            }
            Self::NearContact { first, second } => write!(
                formatter,
                "{first} and {second} touch with no junction between them"
            ),
            Self::Overlap { first, second } => {
                write!(formatter, "{first} and {second} lie on top of each other")
            }
            Self::IncompatibleCrossing { first, second } => write!(
                formatter,
                "{first} and {second} cross, which needs both of them to transmit"
            ),
            Self::FreeTransmittingEnd(curve) => write!(
                formatter,
                "curve {} transmits but ends in open space",
                curve.0
            ),
            Self::TooManySegments => {
                formatter.write_str("this geometry has too many segments to trace")
            }
            Self::TooManyFaces => {
                formatter.write_str("this geometry has too many subdomains to trace")
            }
        }
    }
}

impl std::error::Error for TopologyIssue {}

enum CurveSampler {
    Closed(Sampler),
    Open(OpenSampler),
}

impl CurveSampler {
    fn step(&mut self) -> bool {
        match self {
            Self::Closed(sampler) => sampler.step(),
            Self::Open(sampler) => sampler.step(),
        }
    }

    fn finish(self) -> Result<Vec<crate::Sample>, crate::SamplingError> {
        match self {
            Self::Closed(sampler) => sampler.finish(),
            Self::Open(sampler) => sampler.finish(),
        }
    }
}

#[derive(Clone)]
struct RawSegment {
    source: CompiledEdgeSource,
    behavior: Option<SpanBehavior>,
    points: [Point2; 2],
    parameter: [f64; 2],
    endpoint_vertices: [Option<TopologyVertexId>; 2],
    curve: Option<usize>,
    curve_id: Option<CurveId>,
    sequence: usize,
    sequence_count: usize,
    closed: bool,
}

struct SampledCurve {
    samples: Vec<crate::Sample>,
}

struct ArrangementWork {
    raw: Vec<RawSegment>,
    splits: Vec<Vec<f64>>,
    first: usize,
    second: usize,
}

impl ArrangementWork {
    fn new(raw: Vec<RawSegment>) -> Self {
        let splits = vec![vec![0.0, 1.0]; raw.len()];
        Self {
            raw,
            splits,
            first: 0,
            second: 1,
        }
    }

    fn complete(&self) -> bool {
        self.raw.len() < 2 || self.first + 1 >= self.raw.len()
    }

    fn advance_pair(&mut self) {
        self.second += 1;
        if self.second >= self.raw.len() {
            self.first += 1;
            self.second = self.first + 1;
        }
    }
}

pub struct TopologyJob {
    revision: u64,
    geometry: TopologyGeometry,
    options: SamplingOptions,
    sampler: Option<CurveSampler>,
    sample_index: usize,
    sampled: Vec<SampledCurve>,
    arrangement: Option<ArrangementWork>,
    work: usize,
    result: Option<TopologyResult>,
}

impl TopologyJob {
    pub fn new(geometry: TopologyGeometry, revision: u64) -> Self {
        let tolerance = geometry.domain.width().max(geometry.domain.height()) * 1.0e-4;
        Self::with_options(
            geometry,
            revision,
            SamplingOptions {
                tolerance: tolerance * 0.25,
                ..SamplingOptions::default()
            },
        )
    }

    pub fn with_options(
        geometry: TopologyGeometry,
        revision: u64,
        options: SamplingOptions,
    ) -> Self {
        let issue = validate_structure(&geometry, options);
        Self {
            revision,
            geometry,
            options,
            sampler: None,
            sample_index: 0,
            sampled: vec![],
            arrangement: None,
            work: 0,
            result: issue.map(|issue| TopologyResult {
                revision,
                result: Err(issue),
            }),
        }
    }

    pub fn advance(&mut self, budget: usize) -> Option<TopologyResult> {
        for _ in 0..budget {
            if self.result.is_some() {
                break;
            }
            self.work += 1;
            if self.work > MAX_TOPOLOGY_WORK {
                self.finish(Err(TopologyIssue::WorkLimit));
                break;
            }
            if self.sample_index < self.geometry.curves.len() {
                if self.sampler.is_none() {
                    self.sampler = Some(
                        self.geometry.curves[self.sample_index]
                            .spline
                            .sample(self.options),
                    );
                }
                if self.sampler.as_mut().unwrap().step() {
                    let samples = self.sampler.take().unwrap().finish().map_err(|_| {
                        TopologyIssue::Sampling(self.geometry.curves[self.sample_index].id)
                    });
                    match samples {
                        Ok(samples) => self.sampled.push(SampledCurve { samples }),
                        Err(issue) => {
                            self.finish(Err(issue));
                            break;
                        }
                    }
                    self.sample_index += 1;
                }
                continue;
            }

            let tolerance = self.options.tolerance * 4.0;
            if self.arrangement.is_none() {
                match build_raw_segments(&self.geometry, &self.sampled, tolerance) {
                    Ok(raw) => self.arrangement = Some(ArrangementWork::new(raw)),
                    Err(issue) => self.finish(Err(issue)),
                }
                continue;
            }
            let arrangement = self.arrangement.as_mut().unwrap();
            if !arrangement.complete() {
                if let Err(issue) = inspect_pair(
                    &self.geometry,
                    &arrangement.raw,
                    arrangement.first,
                    arrangement.second,
                    tolerance,
                    &mut arrangement.splits,
                ) {
                    self.finish(Err(issue));
                    continue;
                }
                arrangement.advance_pair();
                continue;
            }
            let arrangement = self.arrangement.take().unwrap();
            let result = finish_arrangement(
                &self.geometry,
                arrangement.raw,
                arrangement.splits,
                self.revision,
                tolerance,
            );
            self.finish(result);
        }
        self.result.clone()
    }

    fn finish(&mut self, result: Result<TopologySnapshot, TopologyIssue>) {
        self.result = Some(TopologyResult {
            revision: self.revision,
            result,
        });
    }
}

pub fn compile_topology(
    geometry: &TopologyGeometry,
    revision: u64,
) -> Result<TopologySnapshot, TopologyIssue> {
    let mut job = TopologyJob::new(geometry.clone(), revision);
    loop {
        if let Some(result) = job.advance(4096) {
            return result.result;
        }
    }
}

fn validate_structure(
    geometry: &TopologyGeometry,
    options: SamplingOptions,
) -> Option<TopologyIssue> {
    if !geometry.domain.valid()
        || !options.tolerance.is_finite()
        || options.tolerance <= 0.0
        || options.max_points < 2
        || geometry.curves.len() > MAX_TOPOLOGY_CURVES
        || geometry.vertices.len() > MAX_TOPOLOGY_VERTICES
    {
        return Some(TopologyIssue::Structure);
    }
    let mut curve_ids = BTreeSet::new();
    let mut span_ids = BTreeSet::new();
    for curve in &geometry.curves {
        if curve.structure_valid().is_err()
            || !curve_ids.insert(curve.id)
            || curve.spans.iter().any(|span| !span_ids.insert(span.id))
        {
            return Some(TopologyIssue::Structure);
        }
    }
    let mut vertex_ids = BTreeSet::new();
    let mut points = BTreeMap::new();
    for vertex in &geometry.vertices {
        let Some(point) = vertex.point(geometry.domain) else {
            return Some(TopologyIssue::Structure);
        };
        if vertex.id.0 == 0 || !vertex_ids.insert(vertex.id) {
            return Some(TopologyIssue::Structure);
        }
        points.insert(vertex.id, point);
    }
    let tolerance = options.tolerance * 4.0;
    for curve in &geometry.curves {
        for (node_index, node) in curve.nodes.iter().enumerate() {
            let Some(vertex) = node.vertex else {
                continue;
            };
            let Some(target) = points.get(&vertex) else {
                return Some(TopologyIssue::MissingVertex {
                    curve: curve.id,
                    vertex,
                });
            };
            let Some(point) = curve.spline.node_point(node_index) else {
                return Some(TopologyIssue::Structure);
            };
            if (point - *target).norm() > tolerance {
                return Some(TopologyIssue::VertexMismatch {
                    curve: curve.id,
                    vertex,
                });
            }
        }
    }
    None
}

fn build_raw_segments(
    geometry: &TopologyGeometry,
    sampled: &[SampledCurve],
    tolerance: f64,
) -> Result<Vec<RawSegment>, TopologyIssue> {
    let mut raw = domain_segments(geometry.domain);
    for (curve_index, sampled) in sampled.iter().enumerate() {
        let curve = &geometry.curves[curve_index];
        if sampled.samples.len() < 2 {
            return Err(TopologyIssue::Degenerate(curve.id));
        }
        let sequence_count = sampled.samples.len() - 1;
        let mut total_length = 0.0;
        for (sequence, pair) in sampled.samples.windows(2).enumerate() {
            if !inside_domain(geometry.domain, pair[0].point, tolerance)
                || !inside_domain(geometry.domain, pair[1].point, tolerance)
            {
                return Err(TopologyIssue::Outside(curve.id));
            }
            let length = (pair[1].point - pair[0].point).norm();
            if length <= tolerance * 1.0e-4 {
                return Err(TopologyIssue::Degenerate(curve.id));
            }
            total_length += length;
            let middle = (pair[0].t + pair[1].t) * 0.5;
            let span_index = curve.spline.span_index(middle);
            raw.push(RawSegment {
                source: CompiledEdgeSource::Curve(curve.spans[span_index].id),
                behavior: Some(curve.spans[span_index].behavior),
                points: [pair[0].point, pair[1].point],
                parameter: [pair[0].t, pair[1].t],
                endpoint_vertices: [
                    node_vertex_at(curve, pair[0].t),
                    node_vertex_at(curve, pair[1].t),
                ],
                curve: Some(curve_index),
                curve_id: Some(curve.id),
                sequence,
                sequence_count,
                closed: !curve.spline.is_open(),
            });
        }
        if total_length <= tolerance {
            return Err(TopologyIssue::Degenerate(curve.id));
        }
    }
    if raw.len() > MAX_TOPOLOGY_SEGMENTS {
        return Err(TopologyIssue::TooManySegments);
    }
    Ok(raw)
}

fn finish_arrangement(
    geometry: &TopologyGeometry,
    raw: Vec<RawSegment>,
    splits: Vec<Vec<f64>>,
    revision: u64,
    tolerance: f64,
) -> Result<TopologySnapshot, TopologyIssue> {
    let mut builder = GraphBuilder::new(tolerance);
    for (segment_index, segment) in raw.iter().enumerate() {
        let mut values = splits[segment_index].clone();
        values.sort_by(f64::total_cmp);
        values.dedup_by(|left, right| (*left - *right).abs() <= 1.0e-12);
        for pair in values.windows(2) {
            if pair[1] - pair[0] <= 1.0e-12 {
                continue;
            }
            let a = segment.points[0].lerp(segment.points[1], pair[0]);
            let b = segment.points[0].lerp(segment.points[1], pair[1]);
            let endpoint_vertex = |value: f64| {
                if value <= 1.0e-12 {
                    segment.endpoint_vertices[0]
                } else if value >= 1.0 - 1.0e-12 {
                    segment.endpoint_vertices[1]
                } else {
                    None
                }
            };
            builder.add_edge(
                [a, b],
                [endpoint_vertex(pair[0]), endpoint_vertex(pair[1])],
                EdgeDescriptor {
                    source: segment.source,
                    curve: segment.curve_id,
                    behavior: segment.behavior,
                    parameter: [
                        lerp_scalar(segment.parameter[0], segment.parameter[1], pair[0]),
                        lerp_scalar(segment.parameter[0], segment.parameter[1], pair[1]),
                    ],
                },
            )?;
        }
    }
    if builder.edges.len() > MAX_TOPOLOGY_SEGMENTS * 2 {
        return Err(TopologyIssue::TooManySegments);
    }
    builder.finish(geometry, revision, tolerance)
}

fn inspect_pair(
    geometry: &TopologyGeometry,
    raw: &[RawSegment],
    first: usize,
    second: usize,
    tolerance: f64,
    splits: &mut [Vec<f64>],
) -> Result<(), TopologyIssue> {
    let a = &raw[first];
    let b = &raw[second];
    let [a0, a1] = a.points;
    let [b0, b1] = b.points;
    let signs = [
        orient2d(a0, a1, b0),
        orient2d(a0, a1, b1),
        orient2d(b0, b1, a0),
        orient2d(b0, b1, a1),
    ];
    let proper = opposite(signs[0], signs[1]) && opposite(signs[2], signs[3]);
    if proper {
        if !a.behavior.is_some_and(SpanBehavior::transmitting)
            || !b.behavior.is_some_and(SpanBehavior::transmitting)
        {
            return Err(TopologyIssue::IncompatibleCrossing {
                first: a.source,
                second: b.source,
            });
        }
        let (ta, tb) = line_intersection_parameters(a0, a1, b0, b1).unwrap();
        splits[first].push(ta);
        splits[second].push(tb);
        return Ok(());
    }

    let collinear = signs.iter().all(|sign| *sign == PredicateSign::Zero);
    if collinear {
        let overlap = collinear_overlap(a0, a1, b0, b1);
        if overlap > tolerance {
            return Err(TopologyIssue::Overlap {
                first: a.source,
                second: b.source,
            });
        }
    }

    let contacts = endpoint_contacts(a0, a1, b0, b1, signs);
    if !contacts.is_empty() {
        for (ta, tb, point) in contacts {
            if !authorized_contact(geometry, a, b, ta, tb, point, tolerance) {
                return Err(TopologyIssue::NearContact {
                    first: a.source,
                    second: b.source,
                });
            }
            splits[first].push(ta);
            splits[second].push(tb);
        }
        return Ok(());
    }

    let distance = point_segment_distance(a0, b0, b1)
        .min(point_segment_distance(a1, b0, b1))
        .min(point_segment_distance(b0, a0, a1))
        .min(point_segment_distance(b1, a0, a1));
    if distance <= tolerance {
        return Err(TopologyIssue::NearContact {
            first: a.source,
            second: b.source,
        });
    }
    Ok(())
}

fn authorized_contact(
    geometry: &TopologyGeometry,
    a: &RawSegment,
    b: &RawSegment,
    ta: f64,
    tb: f64,
    point: Point2,
    tolerance: f64,
) -> bool {
    if naturally_adjacent(a, b) {
        return true;
    }
    let va = endpoint_id(a, ta);
    let vb = endpoint_id(b, tb);
    if va.is_some() && va == vb {
        return true;
    }
    let outer_matches = |vertex: Option<TopologyVertexId>, source: CompiledEdgeSource| {
        let CompiledEdgeSource::Outer(side) = source else {
            return false;
        };
        vertex.is_some_and(|id| {
            geometry.vertices.iter().any(|vertex| {
                vertex.id == id
                    && matches!(
                        vertex.location,
                        TopologyVertexLocation::Outer {
                            side: vertex_side,
                            ..
                        } if vertex_side == side
                    )
                    && vertex
                        .point(geometry.domain)
                        .is_some_and(|target| (target - point).norm() <= tolerance)
            })
        })
    };
    outer_matches(va, b.source) || outer_matches(vb, a.source)
}

fn endpoint_id(segment: &RawSegment, parameter: f64) -> Option<TopologyVertexId> {
    if parameter <= 1.0e-10 {
        segment.endpoint_vertices[0]
    } else if parameter >= 1.0 - 1.0e-10 {
        segment.endpoint_vertices[1]
    } else {
        None
    }
}

fn naturally_adjacent(a: &RawSegment, b: &RawSegment) -> bool {
    match (a.curve, b.curve) {
        (None, None) => {
            (a.sequence + 1 == b.sequence)
                || (b.sequence + 1 == a.sequence)
                || (a.sequence == 0 && b.sequence + 1 == b.sequence_count)
                || (b.sequence == 0 && a.sequence + 1 == a.sequence_count)
        }
        (Some(first), Some(second)) if first == second => {
            a.sequence + 1 == b.sequence
                || b.sequence + 1 == a.sequence
                || (a.closed
                    && ((a.sequence == 0 && b.sequence + 1 == b.sequence_count)
                        || (b.sequence == 0 && a.sequence + 1 == a.sequence_count)))
        }
        _ => false,
    }
}

fn opposite(left: PredicateSign, right: PredicateSign) -> bool {
    matches!(
        (left, right),
        (PredicateSign::Negative, PredicateSign::Positive)
            | (PredicateSign::Positive, PredicateSign::Negative)
    )
}

fn endpoint_contacts(
    a0: Point2,
    a1: Point2,
    b0: Point2,
    b1: Point2,
    signs: [PredicateSign; 4],
) -> Vec<(f64, f64, Point2)> {
    let mut contacts = vec![];
    let candidates = [
        (0.0, segment_parameter(a0, b0, b1), a0, signs[2]),
        (1.0, segment_parameter(a1, b0, b1), a1, signs[3]),
        (segment_parameter(b0, a0, a1), 0.0, b0, signs[0]),
        (segment_parameter(b1, a0, a1), 1.0, b1, signs[1]),
    ];
    for (ta, tb, point, sign) in candidates {
        if sign == PredicateSign::Zero
            && (-1.0e-12..=1.0 + 1.0e-12).contains(&ta)
            && (-1.0e-12..=1.0 + 1.0e-12).contains(&tb)
            && !contacts
                .iter()
                .any(|(_, _, existing): &(f64, f64, Point2)| (*existing - point).norm() == 0.0)
        {
            contacts.push((ta.clamp(0.0, 1.0), tb.clamp(0.0, 1.0), point));
        }
    }
    contacts
}

fn line_intersection_parameters(
    a0: Point2,
    a1: Point2,
    b0: Point2,
    b1: Point2,
) -> Option<(f64, f64)> {
    let a = a1 - a0;
    let b = b1 - b0;
    let denominator = a.cross(b);
    (denominator != 0.0).then(|| {
        let delta = b0 - a0;
        (delta.cross(b) / denominator, delta.cross(a) / denominator)
    })
}

fn segment_parameter(point: Point2, start: Point2, end: Point2) -> f64 {
    let delta = end - start;
    if delta.x.abs() >= delta.y.abs() && delta.x != 0.0 {
        (point.x - start.x) / delta.x
    } else if delta.y != 0.0 {
        (point.y - start.y) / delta.y
    } else {
        0.0
    }
}

fn collinear_overlap(a0: Point2, a1: Point2, b0: Point2, b1: Point2) -> f64 {
    let direction = a1 - a0;
    let length = direction.norm();
    if length == 0.0 {
        return 0.0;
    }
    let unit = direction / length;
    let mut values = [(b0 - a0).dot(unit), (b1 - a0).dot(unit)];
    values.sort_by(f64::total_cmp);
    (values[1].min(length) - values[0].max(0.0)).max(0.0)
}

#[derive(Clone)]
struct AtomicEdge {
    vertices: [usize; 2],
    source: CompiledEdgeSource,
    curve: Option<CurveId>,
    behavior: Option<SpanBehavior>,
    parameter: [f64; 2],
}

struct EdgeDescriptor {
    source: CompiledEdgeSource,
    curve: Option<CurveId>,
    behavior: Option<SpanBehavior>,
    parameter: [f64; 2],
}

struct GraphBuilder {
    tolerance: f64,
    vertices: Vec<CompiledVertex>,
    buckets: BTreeMap<(i64, i64), Vec<usize>>,
    edges: Vec<AtomicEdge>,
}

impl GraphBuilder {
    fn new(tolerance: f64) -> Self {
        Self {
            tolerance,
            vertices: vec![],
            buckets: BTreeMap::new(),
            edges: vec![],
        }
    }

    fn add_edge(
        &mut self,
        points: [Point2; 2],
        authored: [Option<TopologyVertexId>; 2],
        descriptor: EdgeDescriptor,
    ) -> Result<(), TopologyIssue> {
        let a = self.vertex(points[0], authored[0])?;
        let b = self.vertex(points[1], authored[1])?;
        if a == b {
            return Err(match descriptor.source {
                CompiledEdgeSource::Curve(span) => TopologyIssue::NearContact {
                    first: CompiledEdgeSource::Curve(span),
                    second: CompiledEdgeSource::Curve(span),
                },
                CompiledEdgeSource::Outer(side) => TopologyIssue::NearContact {
                    first: CompiledEdgeSource::Outer(side),
                    second: CompiledEdgeSource::Outer(side),
                },
            });
        }
        self.edges.push(AtomicEdge {
            vertices: [a, b],
            source: descriptor.source,
            curve: descriptor.curve,
            behavior: descriptor.behavior,
            parameter: descriptor.parameter,
        });
        Ok(())
    }

    fn vertex(
        &mut self,
        point: Point2,
        authored: Option<TopologyVertexId>,
    ) -> Result<usize, TopologyIssue> {
        let key = quantized(point, self.tolerance);
        for x in key.0 - 1..=key.0 + 1 {
            for y in key.1 - 1..=key.1 + 1 {
                if let Some(candidates) = self.buckets.get(&(x, y)) {
                    for index in candidates {
                        if (self.vertices[*index].point - point).norm() <= self.tolerance * 0.25 {
                            let existing = &mut self.vertices[*index].authored;
                            if existing.is_some() && authored.is_some() && *existing != authored {
                                return Err(TopologyIssue::Structure);
                            }
                            if existing.is_none() {
                                *existing = authored;
                            }
                            return Ok(*index);
                        }
                    }
                }
            }
        }
        if self.vertices.len() >= MAX_TOPOLOGY_SEGMENTS * 2 {
            return Err(TopologyIssue::TooManySegments);
        }
        let index = self.vertices.len();
        self.vertices.push(CompiledVertex {
            point,
            authored,
            traces: vec![],
        });
        self.buckets.entry(key).or_default().push(index);
        Ok(index)
    }

    fn finish(
        self,
        geometry: &TopologyGeometry,
        revision: u64,
        tolerance: f64,
    ) -> Result<TopologySnapshot, TopologyIssue> {
        let mut outgoing = vec![vec![]; self.vertices.len()];
        for (edge, atomic) in self.edges.iter().enumerate() {
            outgoing[atomic.vertices[0]].push(edge * 2);
            outgoing[atomic.vertices[1]].push(edge * 2 + 1);
        }
        for (vertex, half_edges) in outgoing.iter_mut().enumerate() {
            half_edges.sort_by(|left, right| {
                let left_target = half_edge_target(&self.edges, *left);
                let right_target = half_edge_target(&self.edges, *right);
                let origin = self.vertices[vertex].point;
                let left_delta = self.vertices[left_target].point - origin;
                let right_delta = self.vertices[right_target].point - origin;
                left_delta
                    .y
                    .atan2(left_delta.x)
                    .total_cmp(&right_delta.y.atan2(right_delta.x))
            });
        }

        let half_edge_count = self.edges.len() * 2;
        let mut next = vec![0; half_edge_count];
        for (half_edge, next_half_edge) in next.iter_mut().enumerate() {
            let target = half_edge_target(&self.edges, half_edge);
            let twin = half_edge ^ 1;
            let around = &outgoing[target];
            let position = around
                .iter()
                .position(|candidate| *candidate == twin)
                .unwrap();
            *next_half_edge = around[(position + around.len() - 1) % around.len()];
        }

        let mut half_edge_cycle = vec![usize::MAX; half_edge_count];
        let mut cycles = vec![];
        for start in 0..half_edge_count {
            if half_edge_cycle[start] != usize::MAX {
                continue;
            }
            let cycle_index = cycles.len();
            let mut half_edges = vec![];
            let mut current = start;
            loop {
                if half_edge_cycle[current] != usize::MAX {
                    if current != start {
                        return Err(TopologyIssue::Structure);
                    }
                    break;
                }
                half_edge_cycle[current] = cycle_index;
                half_edges.push(current);
                current = next[current];
            }
            let points = half_edges
                .iter()
                .map(|half_edge| self.vertices[half_edge_origin(&self.edges, *half_edge)].point)
                .collect::<Vec<_>>();
            cycles.push(Cycle {
                area: signed_area(&points),
                points,
                half_edges,
            });
        }

        let positive = cycles
            .iter()
            .enumerate()
            .filter(|(_, cycle)| cycle.area > tolerance * tolerance)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if positive.len() > MAX_TOPOLOGY_FACES {
            return Err(TopologyIssue::TooManyFaces);
        }
        let mut cycle_faces = vec![EXTERIOR_FACE; cycles.len()];
        let mut faces = Vec::with_capacity(positive.len());
        for (ordinal, cycle_index) in positive.iter().copied().enumerate() {
            let id = FaceId(ordinal as u64 + 1);
            cycle_faces[cycle_index] = id;
            faces.push(CompiledFace {
                id,
                cycles: vec![cycles[cycle_index].points.clone()],
                boundaries: vec![cycle_steps(&cycles[cycle_index])],
                area: cycles[cycle_index].area,
            });
        }
        for (cycle_index, cycle) in cycles.iter().enumerate() {
            if cycle_faces[cycle_index] != EXTERIOR_FACE {
                continue;
            }
            let Some(probe) = left_probe(cycle, tolerance) else {
                continue;
            };
            let containing = positive
                .iter()
                .copied()
                .filter(|candidate| point_in_polygon(probe, &cycles[*candidate].points))
                .min_by(|left, right| cycles[*left].area.total_cmp(&cycles[*right].area));
            if let Some(container) = containing {
                let face = cycle_faces[container];
                cycle_faces[cycle_index] = face;
                let compiled = faces
                    .iter_mut()
                    .find(|candidate| candidate.id == face)
                    .unwrap();
                compiled.area += cycle.area;
                compiled.cycles.push(cycle.points.clone());
                compiled.boundaries.push(cycle_steps(cycle));
            }
        }

        let mut vertices = self.vertices;
        let edge_traces = compile_trace_vertices(
            &mut vertices,
            &self.edges,
            &outgoing,
            &half_edge_cycle,
            &cycle_faces,
        );
        let mut compiled_edges = Vec::with_capacity(self.edges.len());
        for (edge_index, edge) in self.edges.iter().enumerate() {
            compiled_edges.push(CompiledEdge {
                source: edge.source,
                curve: edge.curve,
                behavior: edge.behavior,
                points: [
                    vertices[edge.vertices[0]].point,
                    vertices[edge.vertices[1]].point,
                ],
                parameter: edge.parameter,
                left: cycle_faces[half_edge_cycle[edge_index * 2]],
                right: cycle_faces[half_edge_cycle[edge_index * 2 + 1]],
                traces: edge_traces[edge_index],
            });
        }

        for curve in &geometry.curves {
            if !curve.spline.is_open() {
                continue;
            }
            for endpoint in [0, curve.nodes.len() - 1] {
                let span = if endpoint == 0 {
                    curve.spans[0]
                } else {
                    *curve.spans.last().unwrap()
                };
                if !span.behavior.transmitting() {
                    continue;
                }
                let point = curve.spline.node_point(endpoint).unwrap();
                let degree = vertices
                    .iter()
                    .position(|vertex| (vertex.point - point).norm() <= tolerance * 0.25)
                    .map(|vertex| outgoing[vertex].len())
                    .unwrap_or(0);
                if degree <= 1 {
                    return Err(TopologyIssue::FreeTransmittingEnd(curve.id));
                }
            }
        }

        Ok(TopologySnapshot {
            revision,
            domain: geometry.domain,
            vertices,
            faces,
            edges: compiled_edges,
        })
    }
}

fn compile_trace_vertices(
    vertices: &mut [CompiledVertex],
    edges: &[AtomicEdge],
    outgoing: &[Vec<usize>],
    half_edge_cycle: &[usize],
    cycle_faces: &[FaceId],
) -> Vec<[CompiledEdgeTraces; 2]> {
    let mut next_trace = 1u64;
    let mut sector_traces = vec![vec![]; vertices.len()];
    for (vertex, around) in outgoing.iter().enumerate() {
        let count = around.len();
        let mut parents = (0..count).collect::<Vec<_>>();
        for (position, half_edge) in around.iter().copied().enumerate() {
            if edges[half_edge / 2]
                .behavior
                .is_some_and(SpanBehavior::transmitting)
            {
                union(&mut parents, position, (position + count - 1) % count);
            }
        }
        let mut roots = BTreeMap::<usize, TraceVertexId>::new();
        for (sector, half_edge) in around.iter().copied().enumerate() {
            let root = find_root(&mut parents, sector);
            let trace = *roots.entry(root).or_insert_with(|| {
                let id = TraceVertexId(next_trace);
                next_trace += 1;
                id
            });
            sector_traces[vertex].push(trace);
            let face = cycle_faces[half_edge_cycle[half_edge]];
            let trace_vertex = CompiledTraceVertex { id: trace, face };
            if !vertices[vertex].traces.contains(&trace_vertex) {
                vertices[vertex].traces.push(trace_vertex);
            }
        }
    }

    edges
        .iter()
        .enumerate()
        .map(|(edge_index, edge)| {
            let start_half_edge = edge_index * 2;
            let end_half_edge = start_half_edge + 1;
            let start_around = &outgoing[edge.vertices[0]];
            let start = start_around
                .iter()
                .position(|half_edge| *half_edge == start_half_edge)
                .unwrap();
            let end_around = &outgoing[edge.vertices[1]];
            let end = end_around
                .iter()
                .position(|half_edge| *half_edge == end_half_edge)
                .unwrap();
            [
                CompiledEdgeTraces {
                    left: sector_traces[edge.vertices[0]][start],
                    right: sector_traces[edge.vertices[0]]
                        [(start + start_around.len() - 1) % start_around.len()],
                },
                CompiledEdgeTraces {
                    left: sector_traces[edge.vertices[1]]
                        [(end + end_around.len() - 1) % end_around.len()],
                    right: sector_traces[edge.vertices[1]][end],
                },
            ]
        })
        .collect()
}

fn find_root(parents: &mut [usize], mut value: usize) -> usize {
    while parents[value] != value {
        parents[value] = parents[parents[value]];
        value = parents[value];
    }
    value
}

fn union(parents: &mut [usize], left: usize, right: usize) {
    let left = find_root(parents, left);
    let right = find_root(parents, right);
    if left != right {
        parents[right] = left;
    }
}

struct Cycle {
    points: Vec<Point2>,
    half_edges: Vec<usize>,
    area: f64,
}

fn cycle_steps(cycle: &Cycle) -> Vec<CompiledBoundaryStep> {
    cycle
        .half_edges
        .iter()
        .map(|half_edge| CompiledBoundaryStep {
            edge: half_edge / 2,
            reversed: half_edge % 2 == 1,
        })
        .collect()
}

fn left_probe(cycle: &Cycle, tolerance: f64) -> Option<Point2> {
    for pair in cycle
        .points
        .iter()
        .zip(cycle.points.iter().cycle().skip(1).take(cycle.points.len()))
    {
        let delta = *pair.1 - *pair.0;
        let length = delta.norm();
        if length > tolerance * 1.0e-4 {
            return Some(
                pair.0.lerp(*pair.1, 0.5)
                    + Point2::new(-delta.y, delta.x) * (tolerance * 0.25 / length),
            );
        }
    }
    None
}

fn half_edge_origin(edges: &[AtomicEdge], half_edge: usize) -> usize {
    edges[half_edge / 2].vertices[half_edge % 2]
}

fn half_edge_target(edges: &[AtomicEdge], half_edge: usize) -> usize {
    edges[half_edge / 2].vertices[1 - half_edge % 2]
}

fn signed_area(points: &[Point2]) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a.cross(*b))
        .sum::<f64>()
        * 0.5
}

fn point_in_polygon(point: Point2, polygon: &[Point2]) -> bool {
    let mut inside = false;
    for (a, b) in polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
    {
        if (a.y > point.y) != (b.y > point.y) {
            let x = (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x;
            if point.x < x {
                inside = !inside;
            }
        }
    }
    inside
}

fn domain_segments(domain: DomainRect) -> Vec<RawSegment> {
    let corners = domain.corners();
    OuterSide::ALL
        .into_iter()
        .enumerate()
        .map(|(index, side)| RawSegment {
            source: CompiledEdgeSource::Outer(side),
            behavior: None,
            points: [corners[index], corners[(index + 1) % 4]],
            parameter: [0.0, 1.0],
            endpoint_vertices: [None, None],
            curve: None,
            curve_id: None,
            sequence: index,
            sequence_count: 4,
            closed: true,
        })
        .collect()
}

fn node_vertex_at(curve: &TopologyCurve, parameter: f64) -> Option<TopologyVertexId> {
    let (scale, closed) = match &curve.spline {
        CurveSpline::Closed(spline) => (spline.period(), true),
        CurveSpline::Open(spline) => (spline.period(), false),
    };
    let tolerance = scale * 1.0e-10;
    curve
        .nodes
        .iter()
        .enumerate()
        .find(|(index, _)| {
            curve.spline.node_parameter(*index).is_some_and(|node| {
                // A closed curve's seam is node 0, which the sampler reaches
                // again at the period. Without the wrap the span that ends
                // there loses its junction and the contact reads as accidental.
                let distance = if closed {
                    periodic_distance(node, parameter, scale)
                } else {
                    (node - parameter).abs()
                };
                distance <= tolerance
            })
        })
        .and_then(|(_, node)| node.vertex)
}

fn closest_open_breakpoint(spline: &OpenCubicSpline, parameter: f64) -> usize {
    (0..=spline.intervals().len())
        .min_by(|left, right| {
            (spline.breakpoint(*left).unwrap() - parameter)
                .abs()
                .total_cmp(&(spline.breakpoint(*right).unwrap() - parameter).abs())
        })
        .unwrap()
}

fn closest_periodic_breakpoint(spline: &PeriodicCubicSpline, parameter: f64) -> usize {
    let wrapped = parameter.rem_euclid(spline.period());
    (0..spline.intervals().len())
        .min_by(|left, right| {
            periodic_distance(spline.knots()[*left], wrapped, spline.period()).total_cmp(
                &periodic_distance(spline.knots()[*right], wrapped, spline.period()),
            )
        })
        .unwrap()
}

fn periodic_distance(left: f64, right: f64, period: f64) -> f64 {
    let distance = (left - right).abs();
    distance.min(period - distance)
}

fn outer_point(domain: DomainRect, side: OuterSide, fraction: f64) -> Point2 {
    match side {
        OuterSide::Bottom => Point2::new(domain.min_x + domain.width() * fraction, domain.min_y),
        OuterSide::Right => Point2::new(domain.max_x, domain.min_y + domain.height() * fraction),
        OuterSide::Top => Point2::new(domain.max_x - domain.width() * fraction, domain.max_y),
        OuterSide::Left => Point2::new(domain.min_x, domain.max_y - domain.height() * fraction),
    }
}

fn inside_domain(domain: DomainRect, point: Point2, tolerance: f64) -> bool {
    point.x >= domain.min_x - tolerance
        && point.x <= domain.max_x + tolerance
        && point.y >= domain.min_y - tolerance
        && point.y <= domain.max_y + tolerance
}

fn quantized(point: Point2, tolerance: f64) -> (i64, i64) {
    let scale = 1.0 / (tolerance * 0.25);
    (
        (point.x * scale).round() as i64,
        (point.y * scale).round() as i64,
    )
}

fn lerp_scalar(a: f64, b: f64, amount: f64) -> f64 {
    a * (1.0 - amount) + b * amount
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(start: u64, count: usize, behavior: SpanBehavior) -> Vec<CurveSpan> {
        (0..count)
            .map(|index| CurveSpan {
                id: CurveSpanId(start + index as u64),
                behavior,
            })
            .collect()
    }

    fn open(
        id: u64,
        span_start: u64,
        points: &[[f64; 2]],
        behavior: SpanBehavior,
    ) -> TopologyCurve {
        let spline = OpenCubicSpline::polyline(
            points
                .iter()
                .map(|point| Point2::new(point[0], point[1]))
                .collect(),
        )
        .unwrap();
        TopologyCurve::new(
            CurveId(id),
            CurveSpline::Open(spline),
            spans(span_start, points.len() - 1, behavior),
        )
        .unwrap()
    }

    fn closed(
        id: u64,
        span_start: u64,
        points: &[[f64; 2]],
        behavior: SpanBehavior,
    ) -> TopologyCurve {
        let spline = PeriodicCubicSpline::polygon(
            points
                .iter()
                .map(|point| Point2::new(point[0], point[1]))
                .collect(),
        )
        .unwrap();
        TopologyCurve::new(
            CurveId(id),
            CurveSpline::Closed(spline),
            spans(span_start, points.len(), behavior),
        )
        .unwrap()
    }

    fn attach(curve: &mut TopologyCurve, node: usize, vertex: TopologyVertexId) {
        curve.nodes[node].vertex = Some(vertex);
    }

    #[test]
    fn empty_rectangle_has_one_bounded_face() {
        let snapshot = compile_topology(&TopologyGeometry::default(), 7).unwrap();
        assert_eq!(snapshot.revision, 7);
        assert_eq!(snapshot.faces.len(), 1);
        assert!((snapshot.faces[0].area - 4.0).abs() < 1.0e-12);
        assert_eq!(snapshot.face_at(Point2::new(0.0, 0.0)), Some(FaceId(1)));
        assert_eq!(snapshot.face_at(Point2::new(2.0, 0.0)), None);
    }

    #[test]
    fn closed_loop_derives_nested_faces_and_side_adjacency() {
        let curve = closed(
            1,
            1,
            &[[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]],
            SpanBehavior::Transmitting,
        );
        let snapshot = compile_topology(
            &TopologyGeometry {
                curves: vec![curve],
                ..TopologyGeometry::default()
            },
            0,
        )
        .unwrap();
        assert_eq!(snapshot.faces.len(), 2);
        let inside = snapshot.face_at(Point2::new(0.0, 0.0)).unwrap();
        let outside = snapshot.face_at(Point2::new(0.8, 0.0)).unwrap();
        assert_ne!(inside, outside);
        for span in 1..=4 {
            let edges = snapshot.span_edges(CurveSpanId(span)).collect::<Vec<_>>();
            assert!(!edges.is_empty());
            assert!(edges.iter().all(|edge| edge.left != edge.right));
        }
    }

    /// A closed curve's seam is node zero, and the span that ends there arrives
    /// at the period rather than at zero. A baffle attached to the seam is a
    /// genuine junction, not an accidental touch, in both directions.
    #[test]
    fn a_junction_on_a_closed_curve_seam_is_authorised_from_both_spans() {
        let corner = TopologyVertexId(1);
        for seam in [0usize, 1] {
            let mut ring = closed(
                1,
                1,
                &[[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]],
                SpanBehavior::REFLECTING,
            );
            attach(&mut ring, seam, corner);
            let seam_point = ring.spline.node_point(seam).unwrap();
            let mut arm = open(
                2,
                10,
                &[
                    [seam_point.x, seam_point.y],
                    [seam_point.x * 1.6, seam_point.y * 1.6],
                ],
                SpanBehavior::REFLECTING,
            );
            attach(&mut arm, 0, corner);
            let geometry = TopologyGeometry {
                domain: DomainRect::UNIT,
                curves: vec![ring, arm],
                vertices: vec![TopologyVertex {
                    id: corner,
                    location: TopologyVertexLocation::Interior(seam_point),
                }],
            };
            let snapshot = compile_topology(&geometry, 0)
                .unwrap_or_else(|issue| panic!("seam node {seam} rejected: {issue}"));
            assert!(
                snapshot.span_edges(CurveSpanId(10)).next().is_some(),
                "the attached arm survives at seam node {seam}"
            );
        }
    }

    #[test]
    fn outer_attached_divider_splits_the_domain() {
        let mut divider = open(1, 1, &[[0.0, -1.0], [0.0, 1.0]], SpanBehavior::Transmitting);
        let bottom = TopologyVertexId(1);
        let top = TopologyVertexId(2);
        attach(&mut divider, 0, bottom);
        attach(&mut divider, 1, top);
        let geometry = TopologyGeometry {
            domain: DomainRect::UNIT,
            curves: vec![divider],
            vertices: vec![
                TopologyVertex {
                    id: bottom,
                    location: TopologyVertexLocation::Outer {
                        side: OuterSide::Bottom,
                        fraction: 0.5,
                    },
                },
                TopologyVertex {
                    id: top,
                    location: TopologyVertexLocation::Outer {
                        side: OuterSide::Top,
                        fraction: 0.5,
                    },
                },
            ],
        };
        let snapshot = compile_topology(&geometry, 0).unwrap();
        assert_eq!(snapshot.faces.len(), 2);
        assert_ne!(
            snapshot.face_at(Point2::new(-0.5, 0.0)),
            snapshot.face_at(Point2::new(0.5, 0.0))
        );
        assert!(
            snapshot
                .span_edges(CurveSpanId(1))
                .all(|edge| edge.traces.iter().all(|trace| trace.left == trace.right))
        );
    }

    #[test]
    fn free_baffle_keeps_one_face_and_two_sides() {
        let baffle = open(1, 1, &[[-0.5, 0.0], [0.5, 0.0]], SpanBehavior::REFLECTING);
        let snapshot = compile_topology(
            &TopologyGeometry {
                curves: vec![baffle],
                ..TopologyGeometry::default()
            },
            0,
        )
        .unwrap();
        assert_eq!(snapshot.faces.len(), 1);
        assert!(
            snapshot
                .span_edges(CurveSpanId(1))
                .all(|edge| edge.left == edge.right)
        );
        assert!(
            snapshot
                .span_edges(CurveSpanId(1))
                .all(|edge| edge.traces.iter().all(|trace| trace.left == trace.right))
        );
    }

    #[test]
    fn closed_separated_curve_has_distinct_trace_nodes() {
        let curve = closed(
            1,
            1,
            &[[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]],
            SpanBehavior::REFLECTING,
        );
        let snapshot = compile_topology(
            &TopologyGeometry {
                curves: vec![curve],
                ..TopologyGeometry::default()
            },
            0,
        )
        .unwrap();
        assert!(
            snapshot
                .edges
                .iter()
                .filter(|edge| matches!(edge.source, CompiledEdgeSource::Curve(_)))
                .all(|edge| edge.traces.iter().all(|trace| trace.left != trace.right))
        );
    }

    #[test]
    fn transmitting_free_end_is_an_invalid_draft() {
        let divider = open(1, 1, &[[0.0, 0.0], [0.5, 0.0]], SpanBehavior::Transmitting);
        let issue = compile_topology(
            &TopologyGeometry {
                curves: vec![divider],
                ..TopologyGeometry::default()
            },
            0,
        )
        .unwrap_err();
        assert_eq!(issue, TopologyIssue::FreeTransmittingEnd(CurveId(1)));
    }

    #[test]
    fn branch_attaches_to_a_closed_curve() {
        let junction = TopologyVertexId(1);
        let mut loop_curve = closed(
            1,
            1,
            &[[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]],
            SpanBehavior::Transmitting,
        );
        attach(&mut loop_curve, 1, junction);
        let mut branch = open(
            2,
            10,
            &[[0.5, -0.5], [1.0, -0.5]],
            SpanBehavior::Transmitting,
        );
        attach(&mut branch, 0, junction);
        let outer = TopologyVertexId(2);
        attach(&mut branch, 1, outer);
        let snapshot = compile_topology(
            &TopologyGeometry {
                domain: DomainRect::UNIT,
                curves: vec![loop_curve, branch],
                vertices: vec![
                    TopologyVertex {
                        id: junction,
                        location: TopologyVertexLocation::Interior(Point2::new(0.5, -0.5)),
                    },
                    TopologyVertex {
                        id: outer,
                        location: TopologyVertexLocation::Outer {
                            side: OuterSide::Right,
                            fraction: 0.25,
                        },
                    },
                ],
            },
            0,
        )
        .unwrap();
        // The branch opens the annular face along a cut; it does not introduce
        // another material face.
        assert_eq!(snapshot.faces.len(), 2);
    }

    #[test]
    fn outer_resize_moves_an_attached_endpoint() {
        let vertex = TopologyVertexId(1);
        let mut curve = open(1, 1, &[[0.0, 0.0], [1.0, 0.0]], SpanBehavior::REFLECTING);
        attach(&mut curve, 1, vertex);
        let mut geometry = TopologyGeometry {
            domain: DomainRect::UNIT,
            curves: vec![curve],
            vertices: vec![TopologyVertex {
                id: vertex,
                location: TopologyVertexLocation::Outer {
                    side: OuterSide::Right,
                    fraction: 0.5,
                },
            }],
        };
        geometry.domain.max_x = 2.0;
        geometry.synchronize_vertices().unwrap();
        assert_eq!(
            geometry.curves[0].spline.node_point(1),
            Some(Point2::new(2.0, 0.0))
        );
    }

    #[test]
    fn periodic_c0_attachment_moves_all_incident_geometry() {
        let vertex = TopologyVertexId(1);
        let mut curve = closed(
            1,
            1,
            &[[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]],
            SpanBehavior::Transmitting,
        );
        attach(&mut curve, 1, vertex);
        let mut geometry = TopologyGeometry {
            domain: DomainRect::UNIT,
            curves: vec![curve],
            vertices: vec![TopologyVertex {
                id: vertex,
                location: TopologyVertexLocation::Interior(Point2::new(0.6, -0.4)),
            }],
        };
        geometry.synchronize_vertices().unwrap();
        assert_eq!(
            geometry.curves[0].spline.node_point(1),
            Some(Point2::new(0.6, -0.4))
        );
    }

    #[test]
    fn insertion_is_shape_preserving_and_splits_span_identity() {
        let mut curve = closed(
            1,
            1,
            &[[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]],
            SpanBehavior::Transmitting,
        );
        let before = (0..=128)
            .map(|index| {
                let parameter = index as f64 * 4.0 / 128.0;
                match &curve.spline {
                    CurveSpline::Closed(spline) => spline.evaluate(parameter),
                    CurveSpline::Open(_) => unreachable!(),
                }
            })
            .collect::<Vec<_>>();
        let insertion = curve
            .insert_topology_vertex(0.5, TopologyVertexId(1), CurveSpanId(9))
            .unwrap();
        assert!(insertion.inserted_span);
        assert_eq!(curve.spans.len(), 5);
        assert_eq!(curve.spans[insertion.node].id, CurveSpanId(9));
        let CurveSpline::Closed(spline) = &curve.spline else {
            unreachable!()
        };
        for (index, expected) in before.into_iter().enumerate() {
            let parameter = index as f64 * 4.0 / 128.0;
            assert!((spline.evaluate(parameter) - expected).norm() < 1.0e-10);
        }
    }

    #[test]
    fn separated_crossing_is_rejected_but_transmitting_crossing_is_compiled() {
        let horizontal = open(1, 1, &[[-0.8, 0.0], [0.8, 0.0]], SpanBehavior::Transmitting);
        let vertical = open(2, 2, &[[0.0, -0.8], [0.0, 0.8]], SpanBehavior::Transmitting);
        let geometry = TopologyGeometry {
            curves: vec![horizontal.clone(), vertical],
            ..TopologyGeometry::default()
        };
        // All four free ends still make the transmitting draft incomplete, but
        // the proper crossing itself is accepted and atomized.
        assert!(matches!(
            compile_topology(&geometry, 0),
            Err(TopologyIssue::FreeTransmittingEnd(_))
        ));

        let separated = open(3, 3, &[[0.0, -0.8], [0.0, 0.8]], SpanBehavior::REFLECTING);
        let issue = compile_topology(
            &TopologyGeometry {
                curves: vec![horizontal, separated],
                ..TopologyGeometry::default()
            },
            0,
        )
        .unwrap_err();
        assert!(matches!(issue, TopologyIssue::IncompatibleCrossing { .. }));
    }

    #[test]
    fn transmitting_crossing_and_t_junction_derive_four_and_three_faces() {
        let outer = [
            TopologyVertex {
                id: TopologyVertexId(1),
                location: TopologyVertexLocation::Outer {
                    side: OuterSide::Left,
                    fraction: 0.5,
                },
            },
            TopologyVertex {
                id: TopologyVertexId(2),
                location: TopologyVertexLocation::Outer {
                    side: OuterSide::Right,
                    fraction: 0.5,
                },
            },
            TopologyVertex {
                id: TopologyVertexId(3),
                location: TopologyVertexLocation::Outer {
                    side: OuterSide::Bottom,
                    fraction: 0.5,
                },
            },
            TopologyVertex {
                id: TopologyVertexId(4),
                location: TopologyVertexLocation::Outer {
                    side: OuterSide::Top,
                    fraction: 0.5,
                },
            },
        ];
        let mut horizontal = open(1, 1, &[[-1.0, 0.0], [1.0, 0.0]], SpanBehavior::Transmitting);
        attach(&mut horizontal, 0, TopologyVertexId(1));
        attach(&mut horizontal, 1, TopologyVertexId(2));
        let mut vertical = open(2, 2, &[[0.0, -1.0], [0.0, 1.0]], SpanBehavior::Transmitting);
        attach(&mut vertical, 0, TopologyVertexId(3));
        attach(&mut vertical, 1, TopologyVertexId(4));
        let crossing = compile_topology(
            &TopologyGeometry {
                domain: DomainRect::UNIT,
                curves: vec![horizontal, vertical],
                vertices: outer.to_vec(),
            },
            0,
        )
        .unwrap();
        assert_eq!(crossing.faces.len(), 4);
        let crossing_vertex = crossing
            .vertices
            .iter()
            .find(|vertex| vertex.authored.is_none() && vertex.point == Point2::new(0.0, 0.0))
            .unwrap();
        assert_eq!(
            crossing_vertex
                .traces
                .iter()
                .map(|trace| trace.id)
                .collect::<BTreeSet<_>>()
                .len(),
            1
        );

        let junction = TopologyVertexId(5);
        let mut horizontal = open(
            3,
            10,
            &[[-1.0, 0.0], [0.0, 0.0], [1.0, 0.0]],
            SpanBehavior::Transmitting,
        );
        attach(&mut horizontal, 0, TopologyVertexId(1));
        attach(&mut horizontal, 1, junction);
        attach(&mut horizontal, 2, TopologyVertexId(2));
        let mut branch = open(
            4,
            20,
            &[[0.0, -1.0], [0.0, 0.0]],
            SpanBehavior::Transmitting,
        );
        attach(&mut branch, 0, TopologyVertexId(3));
        attach(&mut branch, 1, junction);
        let mut vertices = outer[..3].to_vec();
        vertices.push(TopologyVertex {
            id: junction,
            location: TopologyVertexLocation::Interior(Point2::new(0.0, 0.0)),
        });
        let tee = compile_topology(
            &TopologyGeometry {
                domain: DomainRect::UNIT,
                curves: vec![horizontal, branch],
                vertices,
            },
            0,
        )
        .unwrap();
        assert_eq!(tee.faces.len(), 3);
    }

    #[test]
    fn one_ended_baffle_is_accepted() {
        let outer = TopologyVertexId(1);
        let mut baffle = open(1, 1, &[[0.0, 0.0], [1.0, 0.0]], SpanBehavior::REFLECTING);
        attach(&mut baffle, 1, outer);
        let snapshot = compile_topology(
            &TopologyGeometry {
                domain: DomainRect::UNIT,
                curves: vec![baffle],
                vertices: vec![TopologyVertex {
                    id: outer,
                    location: TopologyVertexLocation::Outer {
                        side: OuterSide::Right,
                        fraction: 0.5,
                    },
                }],
            },
            0,
        )
        .unwrap();
        assert_eq!(snapshot.faces.len(), 1);
    }

    #[test]
    fn overlap_and_ambiguous_near_contact_are_rejected() {
        let first = open(1, 1, &[[-0.8, 0.0], [0.2, 0.0]], SpanBehavior::REFLECTING);
        let overlapping = open(2, 2, &[[-0.2, 0.0], [0.8, 0.0]], SpanBehavior::REFLECTING);
        let issue = compile_topology(
            &TopologyGeometry {
                curves: vec![first.clone(), overlapping],
                ..TopologyGeometry::default()
            },
            0,
        )
        .unwrap_err();
        assert!(matches!(issue, TopologyIssue::Overlap { .. }));

        let nearby = open(
            3,
            3,
            &[[-0.8, 0.0001], [0.2, 0.0001]],
            SpanBehavior::REFLECTING,
        );
        let issue = compile_topology(
            &TopologyGeometry {
                curves: vec![first, nearby],
                ..TopologyGeometry::default()
            },
            0,
        )
        .unwrap_err();
        assert!(matches!(issue, TopologyIssue::NearContact { .. }));
    }

    #[test]
    fn subdivision_exhaustion_is_unaccepted() {
        let spline = PeriodicCubicSpline::rounded(Point2::new(0.0, 0.0), 0.5);
        let curve = TopologyCurve::new(
            CurveId(1),
            CurveSpline::Closed(spline),
            spans(1, 8, SpanBehavior::Transmitting),
        )
        .unwrap();
        let mut job = TopologyJob::with_options(
            TopologyGeometry {
                curves: vec![curve],
                ..TopologyGeometry::default()
            },
            0,
            SamplingOptions {
                tolerance: 1.0e-12,
                max_depth: 0,
                max_points: 32,
            },
        );
        let result = loop {
            if let Some(result) = job.advance(1) {
                break result;
            }
        };
        assert_eq!(result.result, Err(TopologyIssue::Sampling(CurveId(1))));
    }

    #[test]
    fn result_carries_revision_for_stale_job_rejection() {
        let mut job = TopologyJob::new(TopologyGeometry::default(), 41);
        let result = loop {
            if let Some(result) = job.advance(1) {
                break result;
            }
        };
        assert_eq!(result.revision, 41);
        assert_eq!(result.result.unwrap().revision, 41);
    }
}
