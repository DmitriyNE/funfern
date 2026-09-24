#![allow(clippy::collapsible_if)]

use crate::canonical_gpu::{
    CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest, CanonicalGpuTemporalManifest,
    CanonicalGpuTransferPlan,
};
use crate::files::FileEvent;
use crate::material_overlay::MaterialOverlay;
use crate::recording::{self, RecordingSpec};
use crate::wave_gpu::{
    AreaProbeDisplay, AreaProbeInput, CurveProbeDisplay, CurveProbeInput, FarFieldDisplay,
    ProbeDisplay, RecorderContext, VectorOverlayDisplay, WaveDisplay, WaveGpuRequest,
};
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy_egui::{
    EguiContexts,
    egui::{self, Color32, Pos2, Rect, Sense, Stroke},
};
use funfern_app::document::{ProbeId, ProbeSamplingPreset};
use funfern_app::topology_persistence::{self as persistence};
use funfern_app::topology_runtime::{PreparedTopology, TopologyPreparationTiming, TopologyToken};
use funfern_app::topology_viewport::{
    SampledTopologyGeometry, ScreenPoint, TopologySelection, ViewportTransform,
};
use funfern_core::*;
use std::collections::{BTreeMap, BTreeSet};
#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
use std::sync::atomic::AtomicBool;
#[cfg(any(
    not(target_arch = "wasm32"),
    all(target_arch = "wasm32", feature = "browser-threads")
))]
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, mpsc::Receiver};

mod amr;
mod chrome;
mod diagnostics;
mod domain;
mod draw_tools;
mod events;
mod exposure;
mod gesture;
mod gizmo;
mod input;
mod inspectors;
mod law_editor;
mod materials;
mod pacing;
mod paint;
mod panels;
mod probe_hit;
mod probe_view;
mod probes;
mod readouts;
mod runtime;
mod selection;

mod session;
mod state;
#[cfg(test)]
mod test_support;
mod theme;
mod viewport;
mod weld;
mod workers;

pub use state::Playground;

// The shared vocabulary every submodule reaches through `use super::*`. A
// submodule's own business stays private to it; only what more than one of them
// needs is re-exported here.
use events::*;
use exposure::*;
use gesture::*;
use pacing::*;
use probe_view::*;
#[cfg(test)]
use test_support::*;
use theme::*;
use workers::*;

const FRAME_HISTORY: usize = 120;

#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
static BROWSER_BACKGROUND_POOL_READY: AtomicBool = AtomicBool::new(false);

/// Called once the shared-memory Rayon pool is ready to run every browser
/// background lane. If bootstrap fails, target-specific cooperative fallbacks
/// remain in force.
#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
pub(crate) fn set_browser_background_pool_ready(ready: bool) {
    BROWSER_BACKGROUND_POOL_READY.store(ready, Ordering::Release);
}
/// Seconds of progress the steps-per-second readout averages over.
const STEP_RATE_WINDOW: f64 = 0.5;
/// Below one percent of the run's meaningful amplitude, automatic exposure is
/// more likely to reveal the slowly decaying numerical tail than useful wave
/// content. The saved-scene drain fixture plateaus at roughly 0.2--0.3% even
/// with resident damping, so the former 0.1% floor still normalized that tail.
const PRESENTATION_QUIET_AMPLITUDE_RATIO: f64 = 1.0e-2;
/// Energy is quadratic in amplitude. AMR's relative residual becomes dormant
/// below the energy counterpart of the presentation floor.
const DORMANT_ENERGY_RATIO: f64 =
    PRESENTATION_QUIET_AMPLITUDE_RATIO * PRESENTATION_QUIET_AMPLITUDE_RATIO;
/// Frames one line or boundary probe keeps. With the sampling presets' rates
/// this is 17, 8.5, or 4.3 seconds of path history, and it bounds how far back
/// the averaged flux row can look.
const CURVE_TRACE_FRAMES: usize = 512;
const GIZMO_PADDING: f32 = 18.0;
/// Estimated error of the whole field the adaptation aims for, in percent. A
/// factor of two apart like the mesh resolution presets, and named the same
/// way, because they answer the same question at either end of the loop: how
/// much detail is this worth.
const AMR_ACCURACY_PRESETS: [(f64, &str); 3] = [(24.0, "Coarse"), (12.0, "Medium"), (6.0, "Fine")];
/// Coarsening starts only well below the requested global error. The interval
/// between this threshold and the target is the controller's global deadband:
/// an estimate wobbling near the target cannot alternate the mesh direction.
const AMR_COARSEN_GLOBAL_RATIO: f64 = 0.65;
/// An edge must be substantially shorter than its requested size before it may
/// be collapsed. This is below half the 1.05 refinement threshold, so an edge
/// split just above that threshold is not immediately eligible for reversal.
const AMR_COARSEN_EDGE_RATIO: f64 = 0.45;
/// A vertex changed by one transaction must survive the immediately following
/// generation. Together with the size deadband this prevents split/collapse
/// ping-pong when a moving wave nudges a target across a threshold.
const AMR_TOPOLOGY_COOLDOWN_GENERATIONS: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AmrDecision {
    Hold,
    Refine,
    Coarsen,
}

impl AmrDecision {
    const fn label(self) -> &'static str {
        match self {
            Self::Hold => "settled",
            Self::Refine => "refining",
            Self::Coarsen => "coarsening",
        }
    }
}

/// What to call the accuracy the adaptation is set to. The slider reaches
/// everything between, so most values have no name.
fn amr_accuracy_preset_name(percent: f64) -> &'static str {
    AMR_ACCURACY_PRESETS
        .iter()
        .find(|(value, _)| (percent - value).abs() < 1.0e-9)
        .map_or("Custom", |(_, name)| *name)
}

/// What the Materials panel lists, and what a viewport click selects with it.
/// Faces include holes, which own no region and so cannot appear in the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubdomainListing {
    Faces,
    Regions,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum InspectorPanel {
    #[default]
    Edit,
    View,
    Simulation,
    Materials,
    Probes,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ClosedPurpose {
    #[default]
    Subdomain,
    Hole,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OpenPurpose {
    #[default]
    Separator,
    Baffle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoundaryKind {
    Reflecting,
    FirstOrder,
    SecondOrder,
    ElectricWall,
    MagneticWall,
    Neumann,
    Dirichlet,
}

impl BoundaryKind {
    /// What a condition of this kind is called, wherever it is named. The
    /// picker lists these and both condition enums answer with the same words,
    /// which `boundary_names_agree_across_every_source` holds them to.
    const fn label(self) -> &'static str {
        match self {
            Self::Reflecting => "Reflecting",
            Self::FirstOrder => "First-order outgoing",
            Self::SecondOrder => "Second-order outgoing",
            Self::ElectricWall => "Electric wall",
            Self::MagneticWall => "Magnetic wall",
            Self::Neumann => "Prescribed Neumann",
            Self::Dirichlet => "Prescribed Dirichlet",
        }
    }

    const fn choices(physics: PhysicsModel) -> &'static [Self] {
        match physics {
            PhysicsModel::Mechanical => &[
                Self::Reflecting,
                Self::FirstOrder,
                Self::SecondOrder,
                Self::Neumann,
                Self::Dirichlet,
            ],
            PhysicsModel::Electromagnetic { .. } => &[
                Self::ElectricWall,
                Self::MagneticWall,
                Self::FirstOrder,
                Self::SecondOrder,
                Self::Neumann,
                Self::Dirichlet,
            ],
        }
    }

    /// Give a persisted superset condition the physical name of its active
    /// skin. The underlying value remains untouched until the user edits it,
    /// preserving old scenes and polarization switches exactly.
    const fn presented(self, physics: PhysicsModel) -> Self {
        match (self, physics) {
            (Self::ElectricWall, PhysicsModel::Mechanical) => Self::Dirichlet,
            (Self::MagneticWall, PhysicsModel::Mechanical) => Self::Reflecting,
            (
                Self::Reflecting,
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Tm,
                },
            ) => Self::MagneticWall,
            (
                Self::Reflecting,
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Te,
                },
            ) => Self::ElectricWall,
            _ => self,
        }
    }

    const fn label_for(self, physics: PhysicsModel) -> &'static str {
        match (self, physics) {
            (Self::Reflecting, PhysicsModel::Mechanical) => "Free boundary",
            (Self::Neumann, PhysicsModel::Mechanical) => "Prescribed traction",
            (Self::Dirichlet, PhysicsModel::Mechanical) => "Fixed / prescribed displacement",
            (Self::Neumann, PhysicsModel::Electromagnetic { .. }) => "Prescribed normal flux",
            (Self::Dirichlet, PhysicsModel::Electromagnetic { .. }) => "Prescribed axial field",
            _ => self.label(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DrawTool {
    Circle,
    Rectangle,
    Polygon,
    ClosedSpline,
    Polyline,
    OpenSpline,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ProbePlacement {
    Point,
    Segment { start: Option<Point2> },
    Disk { center: Option<Point2> },
    Region,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SnapshotState {
    #[default]
    Idle,
    Requested,
    Armed,
    Capturing,
    Saving,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RecordingState {
    #[default]
    Idle,
    SelectingDestination,
    Requested,
    Preparing,
    Starting,
    Recording,
    Finalizing,
}

/// One completed geometry-to-GPU transaction, split into the three waits the
/// user can actually act on: CPU preparation, draining the solver's requested
/// steps, and the GPU upload itself.
#[derive(Clone, Debug)]
struct HandoffRecord {
    prepare_ms: f64,
    pack_ms: f64,
    drain_ms: f64,
    upload_ms: f64,
    timing: TopologyPreparationTiming,
    action: TopologyMeshUpdateAction,
    operator_reused: bool,
    adapted: bool,
    transferred: bool,
    /// Target nodes the transfer copied exactly, when the field crossed over.
    exact_nodes: usize,
    fresh: bool,
    degrees_of_freedom: usize,
    triangles: usize,
    carve: Option<CarveReport>,
    repair_fallback: Option<String>,
}

struct Uploading {
    token: TopologyToken,
    generation: u64,
    fresh: bool,
    degrees_of_freedom: usize,
}

struct PendingSourceCommit {
    token: TopologyToken,
    serial: u32,
}

struct PreparedGpuUpload {
    plan: CanonicalGpuPlan,
    transfer: Option<CanonicalGpuTransferPlan>,
}

#[derive(Clone, Debug, PartialEq)]
struct VectorOverlayLayoutKey {
    mesh_revision: u64,
    generation: u64,
    physics: PhysicsModel,
    world_spacing: f64,
    visible_bins: [i64; 4],
}

#[derive(Clone, Copy, Debug)]
struct VectorOverlayLayoutPoint {
    element: u32,
    point: Point2,
    stencil: QuadraticPointStencil,
}

struct VectorOverlayLayout {
    key: VectorOverlayLayoutKey,
    revision: u64,
    points: Vec<VectorOverlayLayoutPoint>,
    submitted_at: Instant,
}

impl VectorOverlayLayout {
    fn matches(&self, display: &VectorOverlayDisplay) -> bool {
        self.key.generation == display.generation
            && self.revision == display.revision
            && self.points.len() == display.samples.len()
    }
}

fn vector_overlay_revision_owned(
    generation: u64,
    revision: u64,
    recorder_generation: u64,
    recorder_revision: u64,
    has_buffers: bool,
) -> bool {
    generation == recorder_generation && revision == recorder_revision && has_buffers
}

/// CPU packing for a candidate generation. The immutable topology owns every
/// input, so native builds can prepare the 80+ MiB GPU layout off the UI thread
/// while the accepted generation keeps running.
struct GpuUploadPreparation {
    token: TopologyToken,
    time_step: f64,
    receiver: Mutex<Receiver<Result<PreparedGpuUpload, String>>>,
    result: Option<Result<PreparedGpuUpload, String>>,
}

/// What the probe buffers on the GPU were last built for. The topology names
/// the stencils, but the wave buffers own the probes: every path that replaces
/// them drops each probe's buffers and readback, and a reset or a rolled-back
/// transfer takes that path without a commit. Keying the upload to the GPU
/// generation as well is what brings the probes back afterwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProbeUpload {
    token: TopologyToken,
    generation: u64,
    revision: u64,
    curve_revision: u64,
    area_revision: u64,
    far_field_revision: u64,
}

fn probes_need_upload(upload: Option<ProbeUpload>, token: TopologyToken, generation: u64) -> bool {
    upload.is_none_or(|upload| upload.token != token || upload.generation != generation)
}

/// The reconstruction every recorder over a generation has to be built from.
/// A driven plan rejects a stencil that does not address its law tables, so a
/// fixed one there records nothing at all rather than a wrong value.
#[derive(Clone, Copy)]
enum RecorderSource<'a> {
    Fixed(&'a CanonicalWaveOperator),
    Temporal(
        &'a CanonicalTemporalWaveOperator,
        CanonicalGpuTemporalManifest,
    ),
}

impl<'a> RecorderSource<'a> {
    /// `None` while the active generation and the installed plan disagree
    /// about whether the medium is driven, which a handoff can leave for a
    /// frame. The caller waits rather than building against the wrong one.
    fn of(active: &'a PreparedTopology, request: &CanonicalGpuRequest) -> Option<Self> {
        let installed = request.manifest().and_then(|manifest| manifest.temporal);
        Some(
            match recorder_pairing(active.canonical_temporal_operator.as_deref(), installed)? {
                Some((operator, manifest)) => Self::Temporal(operator, manifest),
                None => Self::Fixed(&active.canonical_operator),
            },
        )
    }

    fn point_probes(
        self,
        request: &mut WaveGpuRequest,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        probes: &[(u64, Option<QuadraticPointStencil>)],
        sample_rate: f64,
        context: RecorderContext,
    ) -> Result<(), String> {
        match self {
            Self::Fixed(operator) => request.update_canonical_point_probes(
                assets,
                commands,
                operator,
                probes,
                sample_rate,
                context,
            ),
            Self::Temporal(operator, manifest) => request.update_temporal_canonical_point_probes(
                assets,
                commands,
                operator,
                manifest,
                probes,
                sample_rate,
                context,
            ),
        }
    }

    fn curve_probes(
        self,
        request: &mut WaveGpuRequest,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        probes: &[CurveProbeInput],
        context: RecorderContext,
    ) -> Result<(), String> {
        match self {
            Self::Fixed(operator) => {
                request.update_canonical_curve_probes(assets, commands, operator, probes, context)
            }
            Self::Temporal(operator, manifest) => request.update_temporal_canonical_curve_probes(
                assets, commands, operator, manifest, probes, context,
            ),
        }
    }

    fn area_probes(
        self,
        request: &mut WaveGpuRequest,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        probes: &[AreaProbeInput],
        sample_rate: f64,
        context: RecorderContext,
    ) -> Result<(), String> {
        match self {
            Self::Fixed(operator) => request.update_canonical_area_probes(
                assets,
                commands,
                operator,
                probes,
                sample_rate,
                context,
            ),
            Self::Temporal(operator, manifest) => request.update_temporal_canonical_area_probes(
                assets,
                commands,
                operator,
                manifest,
                probes,
                sample_rate,
                context,
            ),
        }
    }

    fn vector_overlay(
        self,
        request: &mut WaveGpuRequest,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        stencils: &[QuadraticPointStencil],
    ) -> Result<(), String> {
        match self {
            Self::Fixed(operator) => {
                request.update_canonical_vector_overlay(assets, commands, operator, stencils)
            }
            Self::Temporal(operator, manifest) => request.update_temporal_canonical_vector_overlay(
                assets, commands, operator, manifest, stencils,
            ),
        }
    }
}

/// A driven generation pairs with a plan that carries law tables, an inert one
/// with a plan that does not, and anything else is a handoff in between.
fn recorder_pairing<T, M>(temporal: Option<T>, installed: Option<M>) -> Option<Option<(T, M)>> {
    match (temporal, installed) {
        (Some(operator), Some(manifest)) => Some(Some((operator, manifest))),
        (None, None) => Some(None),
        _ => None,
    }
}

/// Ownership of an AMR estimate's immutable snapshot. Live GPU events replace
/// command buffers and advance the request revision, but do not change the
/// topology or invalidate a snapshot already copied to the CPU. A generation
/// handoff does both and therefore is part of the ownership key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AmrIndicatorSource {
    topology: TopologyToken,
    gpu_generation: u64,
    accepted_step: u64,
}

impl AmrIndicatorSource {
    fn is_current(self, topology: TopologyToken, gpu_generation: u64) -> bool {
        self.topology == topology && self.gpu_generation == gpu_generation
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct VectorAcState {
    input: Point2,
    output: Point2,
    step: u64,
    time: f64,
    origin: Pos2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VectorOverlayAcOwner {
    mesh_revision: u64,
    physics: PhysicsModel,
}

impl Playground {
    fn transform(&self, viewport: Rect) -> ViewportTransform {
        ViewportTransform {
            screen_center: ScreenPoint::new(viewport.center().x as f64, viewport.center().y as f64),
            world_center: self.center,
            pixels_per_world: self.scale,
        }
    }
    fn screen(&self, point: Point2, viewport: Rect) -> Pos2 {
        let p = self.transform(viewport).world_to_screen(point);
        Pos2::new(p.x as f32, p.y as f32)
    }
    fn world(&self, point: Pos2, viewport: Rect) -> Point2 {
        self.transform(viewport)
            .screen_to_world(ScreenPoint::new(point.x as f64, point.y as f64))
    }
    fn fit_view(&mut self, viewport: Rect) {
        let domain = self.editor.document.model.draft.geometry.domain;
        self.center = domain.center();
        self.scale = (viewport.width() as f64 / domain.width())
            .min(viewport.height() as f64 / domain.height())
            * 0.88;
        self.fit = false;
    }
    fn invalidate_samples(&mut self) {
        self.sampled_revision = u64::MAX;
    }
    fn refresh_samples(&mut self, viewport: Rect) {
        if self.sampled_revision == self.editor.revision && self.sampled_scale == self.scale {
            return;
        }
        match SampledTopologyGeometry::new(
            &self.editor.document.model.draft.geometry,
            self.transform(viewport),
            0.6,
        ) {
            Ok(sampled) => {
                self.sampled = Some(sampled);
                self.sampled_revision = self.editor.revision;
                self.sampled_scale = self.scale;
            }
            Err(_) => self.sampled = None,
        }
    }
    fn notify(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }
    fn begin_draw(&mut self, tool: DrawTool) {
        self.draw = Some(DrawGesture {
            tool,
            points: vec![],
            attachments: vec![],
        });
        self.selection = TopologySelection::None;
    }
    fn cancel_interaction(&mut self) {
        match &self.drag {
            Some(DragGesture::Pivot { previous, .. }) => {
                self.gizmo_pivot = previous.clone();
            }
            Some(DragGesture::Spans { gizmo_before, .. }) => {
                self.gizmo_pivot = gizmo_before.clone();
            }
            Some(DragGesture::Marquee { base, .. }) => {
                self.selection = Self::selection_from_spans(base.clone());
            }
            _ => {}
        }
        if self.editor.editing() {
            self.editor.cancel();
        }
        self.drag = None;
        self.draw = None;
        self.pending_merge = None;
        self.pulse_mode = false;
        self.probe_mode = None;
        self.invalidate_samples();
    }
    fn show(
        &mut self,
        root: &mut egui::Ui,
        display: &WaveDisplay,
        vector_display: &VectorOverlayDisplay,
    ) -> Rect {
        if self.logo_texture.is_none() {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [216, 88],
                include_bytes!("../../../assets/logo-216x88.rgba"),
            );
            self.logo_texture = Some(root.ctx().load_texture(
                "funfern-logo",
                image,
                egui::TextureOptions::LINEAR,
            ));
        }
        self.top_bar(root);
        self.status_bar(root);
        self.side_panel(root);
        let mut viewport = Rect::NOTHING;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(root, |ui| {
                viewport = self.viewport(ui, display, vector_display);
            });
        if !self.capturing() {
            self.probe_windows(root.ctx());
            self.diagnostics_window(root.ctx());
            self.formula_help_window(root.ctx());
            self.examples_window(root.ctx());
        }
        self.keyboard_focus_previous = root.ctx().egui_wants_keyboard_input();
        viewport
    }
}

fn timing_line(timing: TopologyPreparationTiming) -> String {
    let other = (timing.work_ms - timing.categorized_ms()).max(0.0);
    format!(
        "CPU work {:.1} ms · mesh {:.1} · assembly {:.1} · transfer {:.1} · sources {:.1} · probes {:.1} · other {:.1} ms · {} slices · longest {:.1} ms",
        timing.work_ms,
        timing.meshing_ms,
        timing.assembly_ms,
        timing.transfer_ms,
        timing.sources_ms,
        timing.measurements_ms,
        other,
        timing.slices,
        timing.longest_slice_ms,
    )
}

/// Which of the two states a span selection is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpanBehaviorState {
    Transmit,
    Boundary,
}

/// The state every selected span shares, or `None` when they disagree. A
/// separated span counts as a boundary whatever its two faces carry, so a
/// Dirichlet or thin-gap baffle reports the state it is actually in rather than
/// reading as neither.
fn span_behavior_state(
    geometry: &TopologyGeometry,
    spans: &BTreeSet<CurveSpanId>,
) -> Option<SpanBehaviorState> {
    let mut state = None;
    for span in geometry
        .curves
        .iter()
        .flat_map(|curve| &curve.spans)
        .filter(|span| spans.contains(&span.id))
    {
        let current = match span.behavior {
            SpanBehavior::Transmitting => SpanBehaviorState::Transmit,
            SpanBehavior::Separated { .. } => SpanBehaviorState::Boundary,
        };
        if state.is_some_and(|previous| previous != current) {
            return None;
        }
        state = Some(current);
    }
    state
}

/// A world direction as a screen one. The viewport scales uniformly and flips
/// y, so a unit vector stays a unit vector.
fn screen_direction(direction: Point2) -> egui::Vec2 {
    egui::vec2(direction.x as f32, -direction.y as f32)
}

/// Where a boundary probe reads, and which way, at the midpoint its badge sits
/// on. The path arrives in increasing curve parameter, and `Left` names the
/// face on the left of that direction, so the normal that leaves a left trace
/// is the right one. Crossing that way is what a positive flux reading means,
/// and coming that way is what the probe reads, so one vector carries both:
/// drawn from the sampled side into the badge it says where the data is from
/// and which way positive points. `along` runs the way the arclength axis does,
/// which `reversed` turns without moving the side.
fn boundary_probe_orientation(
    path: &[Point2],
    side: CurveTraceSide,
    reversed: bool,
) -> Option<(Point2, Point2, Point2)> {
    let (point, [start, end]) = Playground::polyline_anchor(path, 0.5)?;
    let tangent = end - start;
    let length = tangent.norm();
    if !length.is_finite() || length <= f64::EPSILON {
        return None;
    }
    let tangent = tangent / length;
    let left = Point2::new(-tangent.y, tangent.x);
    let outward = match side {
        CurveTraceSide::Left => left * -1.0,
        CurveTraceSide::Right => left,
    };
    let along = if reversed { tangent * -1.0 } else { tangent };
    Some((point, outward, along))
}

/// The screen-space unit normal pointing to one side of a span running along
/// `tangent`, or zero for a degenerate segment. Left of increasing parameter in
/// the world is `(t.y, -t.x)` on screen, because the viewport flips the vertical
/// axis; `screen_side` classifies a point by the same rule.
fn side_offset(tangent: egui::Vec2, side: CurveTraceSide) -> egui::Vec2 {
    let length = tangent.length();
    if !length.is_finite() || length <= f32::EPSILON {
        return egui::Vec2::ZERO;
    }
    let left = egui::vec2(tangent.y, -tangent.x) / length;
    match side {
        CurveTraceSide::Left => left,
        CurveTraceSide::Right => -left,
    }
}

/// The coordinates and constants a formula can name, as the reference lists
/// them. Names within an entry are comma separated so the test can evaluate
/// each one on its own.
const FORMULA_SYMBOLS: [(&str, &str); 4] = [
    ("x, y", "position in the region's frame, world units"),
    ("r", "distance from the frame's origin"),
    ("theta", "angle from the frame's x axis, radians"),
    ("pi, e", "3.14159..., 2.71828..."),
];

/// The functions a formula can call, each with an expression the parser has to
/// accept. The reference drifted from the parser once already - it advertised
/// `ln` and `pow`, which the language has never had - so these examples are
/// what keeps the two in step.
/// The Laws section of the formula reference: how a material's laws compose,
/// which field each reads, and where each is valid. Every law the catalogue
/// offers is here; nothing here is a law the solver does not run.
const FORMULA_LAWS: [(&str, &str); 9] = [
    (
        "c = c₀(x)·ḡ·d(t)·s(t)",
        "a coefficient is its base value times the field response, the drive and the Switch; \
         Divide makes the drive and Switch divide it instead",
    ),
    (
        "field argument",
        "the density or permittivity row reads the primary field; the other row reads the \
         magnitude of the complementary field",
    ),
    (
        "Kerr  ḡ = 1 + χ|u|²",
        "χ is relative, per unit |u|²; χ < 0 defocuses and needs an amplitude bound",
    ),
    (
        "Saturable  ḡ = 1 + χ|u|²/(1 + |u|²/σ²)",
        "Kerr that levels off at 1 + χσ²",
    ),
    (
        "Pump  d = 1 + a·cos(2πft + φ)",
        "a < 1; twice a mode's frequency amplifies it",
    ),
    (
        "Time crystal  d = 1 + a·tanh(k·cos(2πft + φ))/tanh(k)",
        "a train of temporal interfaces, sharper as k grows",
    ),
    (
        "Travelling  d = 1 + a·cos(2πft − q·x + φ)",
        "q along the direction angle, in the material frame",
    ),
    (
        "Switch  s: 1 → alternate",
        "thrown on the running medium, over the Switch ramp; zero ramp is a hard interface",
    ),
    (
        "step ceiling",
        "set by the lowest factor each row reaches over every phase, Switch state and \
         admitted amplitude",
    ),
];

const FORMULA_FUNCTIONS: [(&str, &str, &str); 11] = [
    ("sqrt(v)", "square root", "sqrt(r)"),
    ("abs(v)", "magnitude", "abs(x)"),
    ("sin(v)", "sine of an angle in radians", "sin(theta)"),
    ("cos(v)", "cosine of an angle in radians", "cos(theta)"),
    ("tan(v)", "tangent of an angle in radians", "tan(theta)"),
    ("exp(v)", "e raised to v", "exp(-r)"),
    ("log(v)", "natural logarithm", "log(1 + r)"),
    ("min(a, b)", "the smaller of the two", "min(x, y)"),
    ("max(a, b)", "the larger of the two", "max(x, y)"),
    (
        "clamp(min, max, v)",
        "v held inside [min, max]",
        "clamp(0, 1, r)",
    ),
    (
        "smoothstep(edge0, edge1, v)",
        "0 below edge0, 1 above edge1, smooth between",
        "smoothstep(0, 1, r)",
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MaterialEditorLabels {
    mass: &'static str,
    stiffness: &'static str,
    damping: &'static str,
    axis_ratio: &'static str,
}

/// The stored rows predate the skin adapters, but the editor should name the
/// physical response the user is authoring rather than its generic slot.
const fn material_editor_labels(physics: PhysicsModel) -> MaterialEditorLabels {
    match physics {
        PhysicsModel::Mechanical => MaterialEditorLabels {
            mass: "Density ρ₀",
            stiffness: "Stiffness k₀",
            damping: "Damping σ",
            axis_ratio: "Stiffness axis ratio",
        },
        PhysicsModel::Electromagnetic { .. } => MaterialEditorLabels {
            mass: "Permittivity ε",
            stiffness: "Permeability μ",
            damping: "Loss rate α",
            axis_ratio: "Constitutive axis ratio",
        },
    }
}

/// What a law on this row actually multiplies, named for the skin.
///
/// Every skin and row but one multiplies the stored coefficient directly. The
/// mechanical stiffness row is the exception: the solver divides the
/// complementary coefficient by the law's factor, while the mechanical adapter
/// stores `k₀` rather than its reciprocal, so a law there multiplies
/// `s₀ = 1/k₀`. Naming it stiffness would read backwards - a pump's depth
/// going up while the stiffness goes down.
const fn law_row_label(physics: PhysicsModel, row: LawPresetRow) -> &'static str {
    match (physics, row) {
        (PhysicsModel::Mechanical, LawPresetRow::Mass) => "Density ρ₀",
        (PhysicsModel::Mechanical, _) => "Reciprocal stiffness s₀",
        (PhysicsModel::Electromagnetic { .. }, LawPresetRow::Mass) => "Permittivity ε",
        (PhysicsModel::Electromagnetic { .. }, _) => "Permeability μ",
    }
}

/// A preset as the selector names it: the phenomenon, and the coefficient it
/// acts on when that is one row rather than the pair.
fn law_preset_label(preset: &LawPreset, physics: PhysicsModel) -> String {
    match preset.row {
        LawPresetRow::Both if preset.variables.is_empty() => preset.name.to_owned(),
        LawPresetRow::Both => format!("{} (both rows)", preset.name),
        row => format!("{} — {}", preset.name, law_row_label(physics, row)),
    }
}

#[allow(clippy::too_many_arguments)]
fn material_scalar_editor(
    ui: &mut egui::Ui,
    key: (u64, u8),
    label: &str,
    field: &mut ScalarField,
    parameters: &[MaterialParameter],
    minimum: f64,
    edits: &mut BTreeMap<(u64, u8), String>,
    errors: &mut BTreeMap<(u64, u8), String>,
) {
    let mut formula_mode = matches!(field, ScalarField::Formula(_));
    let was_formula = formula_mode;
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(("field-mode", key))
            .selected_text(if formula_mode { "Formula" } else { "Constant" })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut formula_mode, false, "Constant");
                ui.selectable_value(&mut formula_mode, true, "Formula");
            });
    });
    if formula_mode != was_formula {
        if formula_mode {
            let source = field.constant_value().unwrap_or(1.0).to_string();
            *field = ScalarField::formula(source.clone()).unwrap();
            edits.insert(key, source);
            errors.remove(&key);
        } else {
            let coordinates = MaterialCoordinates {
                x: 0.0,
                y: 0.0,
                r: 0.0,
                theta: 0.0,
            };
            match field.evaluate(coordinates, parameters) {
                Ok(value) if value >= minimum => {
                    *field = ScalarField::constant(value);
                    edits.remove(&key);
                    errors.remove(&key);
                }
                Ok(_) => {
                    errors.insert(key, format!("Value must be at least {minimum}"));
                }
                Err(error) => {
                    errors.insert(key, error.to_string());
                }
            }
        }
    }
    match field {
        ScalarField::Constant(value) => {
            ui.add(
                egui::DragValue::new(value)
                    .speed(if minimum == 0.0 { 0.005 } else { 0.01 })
                    .range(minimum..=1.0e6)
                    .update_while_editing(false),
            );
        }
        ScalarField::Formula(formula) => {
            let source = edits
                .entry(key)
                .or_insert_with(|| formula.source().to_owned());
            let response = ui.add(
                egui::TextEdit::singleline(source)
                    .desired_width(ui.available_width())
                    .hint_text("Expression"),
            );
            if response.lost_focus() {
                match ScalarField::formula(source.clone()).and_then(|candidate| {
                    let value = candidate.evaluate(
                        MaterialCoordinates {
                            x: 0.0,
                            y: 0.0,
                            r: 0.0,
                            theta: 0.0,
                        },
                        parameters,
                    )?;
                    if value < minimum {
                        return Err(MaterialError::InvalidValue);
                    }
                    Ok(candidate)
                }) {
                    Ok(candidate) => {
                        *field = candidate;
                        errors.remove(&key);
                    }
                    Err(error) => {
                        errors.insert(key, error.to_string());
                    }
                }
            }
        }
    }
    if let Some(error) = errors.get(&key) {
        ui.colored_label(RED, error);
    }
}

fn edit_face_condition(
    ui: &mut egui::Ui,
    physics: PhysicsModel,
    condition: &mut FaceBoundaryCondition,
) -> bool {
    let before = *condition;
    let mut kind = match condition {
        FaceBoundaryCondition::Reflecting => BoundaryKind::Reflecting,
        FaceBoundaryCondition::Impedance { .. } => BoundaryKind::FirstOrder,
        FaceBoundaryCondition::SecondOrderOutgoing => BoundaryKind::SecondOrder,
        FaceBoundaryCondition::ElectricWall => BoundaryKind::ElectricWall,
        FaceBoundaryCondition::MagneticWall => BoundaryKind::MagneticWall,
        FaceBoundaryCondition::Neumann { .. } => BoundaryKind::Neumann,
        FaceBoundaryCondition::Dirichlet { .. } => BoundaryKind::Dirichlet,
    }
    .presented(physics);
    let menu_height = boundary_combo_height(ui, physics);
    egui::ComboBox::from_id_salt("span-condition")
        .height(menu_height)
        .selected_text(kind.label_for(physics))
        .show_ui(ui, |ui| boundary_kind_choices(ui, physics, &mut kind));
    if kind != face_kind(before).presented(physics) {
        *condition = match kind {
            BoundaryKind::Reflecting => FaceBoundaryCondition::Reflecting,
            BoundaryKind::FirstOrder => FaceBoundaryCondition::Impedance { ratio: 1.0 },
            BoundaryKind::SecondOrder => FaceBoundaryCondition::SecondOrderOutgoing,
            BoundaryKind::ElectricWall => FaceBoundaryCondition::ElectricWall,
            BoundaryKind::MagneticWall => FaceBoundaryCondition::MagneticWall,
            BoundaryKind::Neumann => FaceBoundaryCondition::Neumann {
                signal: TimeSignal::harmonic(0.0, 1.0, 1.0, 0.0),
            },
            BoundaryKind::Dirichlet => FaceBoundaryCondition::Dirichlet {
                signal: TimeSignal::harmonic(0.0, 1.0, 1.0, 0.0),
            },
        };
    }
    match condition {
        FaceBoundaryCondition::Impedance { ratio } => {
            ui.add(
                egui::DragValue::new(ratio)
                    .speed(0.02)
                    .range(1.0e-6..=1.0e6)
                    .prefix("Ratio "),
            );
        }
        FaceBoundaryCondition::Neumann { signal } | FaceBoundaryCondition::Dirichlet { signal } => {
            edit_time_signal(ui, signal)
        }
        _ => {}
    }
    *condition != before
}

fn edit_outer_condition(
    ui: &mut egui::Ui,
    physics: PhysicsModel,
    condition: &mut OuterBoundaryCondition,
) -> bool {
    let before = *condition;
    let mut kind = outer_kind(*condition).presented(physics);
    let menu_height = boundary_combo_height(ui, physics);
    egui::ComboBox::from_id_salt("outer-condition")
        .height(menu_height)
        .selected_text(kind.label_for(physics))
        .show_ui(ui, |ui| boundary_kind_choices(ui, physics, &mut kind));
    if kind != outer_kind(before).presented(physics) {
        *condition = match kind {
            BoundaryKind::Reflecting => OuterBoundaryCondition::Reflecting,
            BoundaryKind::FirstOrder => OuterBoundaryCondition::FirstOrderOutgoing,
            BoundaryKind::SecondOrder => OuterBoundaryCondition::SecondOrderOutgoing,
            BoundaryKind::ElectricWall => OuterBoundaryCondition::ElectricWall,
            BoundaryKind::MagneticWall => OuterBoundaryCondition::MagneticWall,
            BoundaryKind::Neumann => OuterBoundaryCondition::Neumann {
                signal: TimeSignal::harmonic(0.0, 1.0, 1.0, 0.0),
            },
            BoundaryKind::Dirichlet => OuterBoundaryCondition::Dirichlet {
                signal: TimeSignal::harmonic(0.0, 1.0, 1.0, 0.0),
            },
        };
    }
    match condition {
        OuterBoundaryCondition::Neumann { signal }
        | OuterBoundaryCondition::Dirichlet { signal } => edit_time_signal(ui, signal),
        _ => {}
    }
    *condition != before
}

fn outer_kind(condition: OuterBoundaryCondition) -> BoundaryKind {
    match condition {
        OuterBoundaryCondition::Reflecting => BoundaryKind::Reflecting,
        OuterBoundaryCondition::FirstOrderOutgoing => BoundaryKind::FirstOrder,
        OuterBoundaryCondition::SecondOrderOutgoing => BoundaryKind::SecondOrder,
        OuterBoundaryCondition::ElectricWall => BoundaryKind::ElectricWall,
        OuterBoundaryCondition::MagneticWall => BoundaryKind::MagneticWall,
        OuterBoundaryCondition::Neumann { .. } => BoundaryKind::Neumann,
        OuterBoundaryCondition::Dirichlet { .. } => BoundaryKind::Dirichlet,
    }
}

fn face_kind(condition: FaceBoundaryCondition) -> BoundaryKind {
    match condition {
        FaceBoundaryCondition::Reflecting => BoundaryKind::Reflecting,
        FaceBoundaryCondition::Impedance { .. } => BoundaryKind::FirstOrder,
        FaceBoundaryCondition::SecondOrderOutgoing => BoundaryKind::SecondOrder,
        FaceBoundaryCondition::ElectricWall => BoundaryKind::ElectricWall,
        FaceBoundaryCondition::MagneticWall => BoundaryKind::MagneticWall,
        FaceBoundaryCondition::Neumann { .. } => BoundaryKind::Neumann,
        FaceBoundaryCondition::Dirichlet { .. } => BoundaryKind::Dirichlet,
    }
}

fn boundary_kind_choices(ui: &mut egui::Ui, physics: PhysicsModel, kind: &mut BoundaryKind) {
    for value in BoundaryKind::choices(physics) {
        ui.selectable_value(kind, *value, value.label_for(physics));
    }
}

fn boundary_combo_height(ui: &egui::Ui, physics: PhysicsModel) -> f32 {
    // egui's default combo height fits about five ordinary rows. Leave one
    // row of headroom so the six-entry EM picker never acquires an unobvious
    // scrollbar through rounding, font scaling, or popup padding.
    ui.spacing().interact_size.y * (BoundaryKind::choices(physics).len() as f32 + 1.0)
}

fn edit_time_signal(ui: &mut egui::Ui, signal: &mut TimeSignal) {
    let (offset, amplitude, frequency, phase) = signal.harmonic_parameters_mut();
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(offset).speed(0.02).prefix("Offset "));
        ui.add(
            egui::DragValue::new(amplitude)
                .speed(0.02)
                .prefix("Amplitude "),
        );
    });
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(frequency)
                .speed(0.05)
                .range(0.0..=1.0e6)
                .prefix("Hz "),
        );
        ui.add(egui::DragValue::new(phase).speed(0.05).prefix("Phase "));
    });
}

/// Average of the triangle centroids carrying one region, used to anchor a
/// region probe's viewport label where the region actually is.
/// Where a subdomain probe's marker sits: the area-weighted centroid of the
/// faces its region covers, taken from the compiled geometry. Averaging the
/// mesh's triangle centroids instead puts the marker wherever triangles are
/// dense, which under adaptation is wherever the wave currently is, so it
/// wandered on every remesh. For a region shaped like a ring the point lands in
/// the hole, as it does for one such face.
fn region_anchor(scene: &CompiledTopologyScene, region: RegionId) -> Option<Point2> {
    let mut moment = Point2::default();
    let mut area = 0.0;
    for assignment in &scene.assignments {
        if assignment.region != Some(region) {
            continue;
        }
        let Some(face) = scene.topology.face(assignment.face) else {
            continue;
        };
        let Some(centroid) = face.centroid() else {
            continue;
        };
        moment = moment + centroid * face.area;
        area += face.area;
    }
    (area > 0.0).then(|| moment / area)
}

fn sampling_preset_picker(
    ui: &mut egui::Ui,
    id: ProbeId,
    preset: &mut ProbeSamplingPreset,
) -> bool {
    let mut chosen = *preset;
    egui::ComboBox::from_id_salt(("probe_sampling", id.0))
        .selected_text(match chosen {
            ProbeSamplingPreset::Low => "Low · 32 pts / 30 Hz",
            ProbeSamplingPreset::Medium => "Medium · 64 pts / 60 Hz",
            ProbeSamplingPreset::High => "High · 128 pts / 120 Hz",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut chosen, ProbeSamplingPreset::Low, "Low");
            ui.selectable_value(&mut chosen, ProbeSamplingPreset::Medium, "Medium");
            ui.selectable_value(&mut chosen, ProbeSamplingPreset::High, "High");
        });
    if chosen != *preset {
        *preset = chosen;
        return true;
    }
    false
}

const fn primary_field_label(physics: PhysicsModel) -> &'static str {
    match physics {
        PhysicsModel::Mechanical => "Displacement u",
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        } => "Electric field E_z",
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        } => "Magnetic field H_z",
    }
}

const fn primary_field_rate_label(physics: PhysicsModel) -> &'static str {
    match physics {
        PhysicsModel::Mechanical => "Velocity ∂u/∂t",
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        } => "Electric-field rate ∂E_z/∂t",
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        } => "Magnetic-field rate ∂H_z/∂t",
    }
}

const fn total_energy_label(physics: PhysicsModel) -> &'static str {
    match physics {
        PhysicsModel::Mechanical => "Mechanical energy",
        PhysicsModel::Electromagnetic { .. } => "Electromagnetic energy",
    }
}

const fn transverse_field_magnitude_label(physics: PhysicsModel) -> &'static str {
    match physics {
        PhysicsModel::Mechanical => "In-plane magnitude",
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        } => "Magnetic magnitude |H|",
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        } => "Electric magnitude |E|",
    }
}

const fn energy_flow_magnitude_label(physics: PhysicsModel) -> &'static str {
    match physics {
        PhysicsModel::Mechanical => "Energy-flow magnitude |F|",
        PhysicsModel::Electromagnetic { .. } => "Poynting magnitude |S|",
    }
}

fn screen_segment_distance(point: Pos2, start: Pos2, end: Pos2) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_sq();
    if length_squared <= f32::EPSILON {
        return point.distance(start);
    }
    let fraction = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    point.distance(start + fraction * segment)
}

fn closest_curve_parameter(
    sampled: &SampledTopologyGeometry,
    transform: ViewportTransform,
    point: Pos2,
    tolerance: f32,
) -> Option<(CurveId, f64)> {
    let p = ScreenPoint::new(point.x as f64, point.y as f64);
    let mut best: Option<(f64, CurveId, f64)> = None;
    for span in &sampled.spans {
        let Some(curve) = span.curve else { continue };
        for segment in span.samples.windows(2) {
            let a = transform.world_to_screen(segment[0].point);
            let b = transform.world_to_screen(segment[1].point);
            let ab = ScreenPoint::new(b.x - a.x, b.y - a.y);
            let ap = ScreenPoint::new(p.x - a.x, p.y - a.y);
            let length_squared = ab.x * ab.x + ab.y * ab.y;
            let fraction = if length_squared > 0.0 {
                ((ap.x * ab.x + ap.y * ab.y) / length_squared).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let closest = ScreenPoint::new(a.x + ab.x * fraction, a.y + ab.y * fraction);
            let distance = ((p.x - closest.x).powi(2) + (p.y - closest.y).powi(2)).sqrt();
            if distance <= tolerance as f64
                && best
                    .as_ref()
                    .is_none_or(|(best_distance, _, _)| distance < *best_distance)
            {
                best = Some((
                    distance,
                    curve,
                    segment[0].t + (segment[1].t - segment[0].t) * fraction,
                ));
            }
        }
    }
    best.map(|(_, curve, parameter)| (curve, parameter))
}
/// The 1/2/5-per-decade spacing that keeps grid lines about 70 pixels apart.
/// The step the grid draws at, and the finer one it subdivides to.
///
/// Snapping lands on the fine step, so every position Shift can reach has a
/// line under it. The grid used to step in 1/2/5 per decade from the zoom while
/// snapping went to a fixed 0.05, so at most zooms it drew one lattice and
/// landed on another. Each decade divides into round numbers: a 2 into four
/// parts and a 1 or a 5 into five, which at the default zoom puts the fine step
/// back at the 0.05 it used to be fixed at.
fn grid_steps(scale: f64) -> (f64, f64) {
    let raw = 70.0 / scale;
    let power = 10f64.powf(raw.log10().floor());
    let (digit, divisions) = [(1.0, 5.0), (2.0, 4.0), (5.0, 5.0), (10.0, 5.0)]
        .into_iter()
        .find(|(digit, _)| digit * power >= raw)
        .unwrap_or((10.0, 5.0));
    let major = digit * power;
    (major, major / divisions)
}

/// Every multiple of `step` inside `[minimum, maximum]`, lowest first. Both
/// axes walk upwards from the lower bound; the vertical one used to start at
/// the top of the view and test against the bottom, so it never drew a line.
fn grid_lines(minimum: f64, maximum: f64, step: f64) -> Vec<f64> {
    let mut lines = vec![];
    if !minimum.is_finite() || !maximum.is_finite() || !step.is_finite() || step <= 0.0 {
        return lines;
    }
    let mut value = (minimum / step).floor() * step;
    while value <= maximum && lines.len() < 4096 {
        lines.push(value);
        value += step;
    }
    lines
}

/// Whether the next preparation starts the field at zero instead of carrying
/// the running one into it.
///
/// `reset_requested` is spent earlier in the frame by the GPU reset, so a
/// document load cannot rely on it and raises `fresh_requested` instead; this
/// has to honour that even when a topology is already active and the reset flag
/// has been cleared.
const fn starts_from_zero(active: bool, reset_requested: bool, fresh_requested: bool) -> bool {
    !active || reset_requested || fresh_requested
}

/// A fraction in `[0, 1)` without a random-number dependency: the wall clock's
/// sub-second bits natively, and the platform's own generator in the browser,
/// where `SystemTime::now` is not available.
#[cfg(not(target_arch = "wasm32"))]
fn random_fraction() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |since| f64::from(since.subsec_nanos()) / 1e9)
}

#[cfg(target_arch = "wasm32")]
fn random_fraction() -> f64 {
    js_sys::Math::random()
}

/// The catalog entry a fraction picks. A fraction of exactly one is the reason
/// for the clamp: it is outside the half-open range the callers promise, and a
/// browser's generator is only documented to stay below it.
fn random_example_index(fraction: f64, total: usize) -> usize {
    ((fraction.clamp(0.0, 1.0) * total as f64) as usize).min(total.saturating_sub(1))
}

/// One example's thumbnail, built once from its compiled scene and then only
/// painted. Faces are rasterized into scanline spans rather than triangulated:
/// a face is an arbitrary polygon with holes, and a span is one quad when the
/// face carries a flat colour and a row of small quads when the example asks
/// for a material property, which is the only way a radial profile shows up at
/// all.
#[derive(Default)]
struct ExamplePreview {
    quads: Vec<PreviewQuad>,
    /// Face outlines and the zero-area traces of baffles, stroked over the fill.
    strokes: Vec<Vec<Point2>>,
}

struct PreviewQuad {
    low: Point2,
    high: Point2,
    color: Color32,
}

/// Rows down the domain. A row is under three pixels of a thumbnail, and the
/// stroked outlines cover the stair-stepping left at a face's edge. Going finer
/// doubles the quads a shaded example costs and changes nothing that shows.
const PREVIEW_ROWS: usize = 48;

fn build_example_preview(
    example: &funfern_app::topology_examples::TopologyExample,
) -> ExamplePreview {
    let scene = &example.document.model.accepted;
    let Ok(compiled) = scene.compile(0) else {
        return ExamplePreview::default();
    };
    let domain = scene.geometry.domain;
    let property = match example.document.presentation.material_overlay {
        MaterialOverlay::Property(property) => Some(property),
        _ => None,
    };
    let mut faces = compiled.topology.faces.iter().collect::<Vec<_>>();
    faces.sort_by(|a, b| b.area.total_cmp(&a.area));

    let mut cells = Vec::new();
    let (mut minimum, mut maximum) = (f64::INFINITY, f64::NEG_INFINITY);
    for face in &faces {
        let region = compiled
            .assignments
            .iter()
            .find(|assignment| assignment.face == face.id)
            .and_then(|assignment| assignment.region);
        let flat = region
            .and_then(|region| scene.region(region))
            .and_then(|region| scene.material(region.material))
            .map(|material| {
                Color32::from_rgb(material.color[0], material.color[1], material.color[2])
            })
            // A face outside the simulation is a hole, and reads as one.
            .unwrap_or(Color32::from_rgb(16, 23, 31));
        let shaded = property.zip(region).filter(|_| face.area > 0.0);
        let height = domain.height() / PREVIEW_ROWS as f64;
        for row in 0..PREVIEW_ROWS {
            let low_y = domain.min_y + height * row as f64;
            for (start, end) in face_spans(&face.cycles, low_y + height * 0.5) {
                match shaded {
                    None => cells.push((
                        Point2::new(start, low_y),
                        Point2::new(end, low_y + height),
                        None,
                        flat,
                    )),
                    Some((property, region)) => {
                        let steps = (((end - start) / height).ceil() as usize).max(1);
                        let step = (end - start) / steps as f64;
                        for index in 0..steps {
                            let low_x = start + step * index as f64;
                            let center = Point2::new(low_x + step * 0.5, low_y + height * 0.5);
                            let value = crate::material_overlay::sample(scene, region, center)
                                .value(property)
                                .ok();
                            if let Some(value) = value {
                                minimum = minimum.min(value);
                                maximum = maximum.max(value);
                            }
                            cells.push((
                                Point2::new(low_x, low_y),
                                Point2::new(low_x + step, low_y + height),
                                value,
                                flat,
                            ));
                        }
                    }
                }
            }
        }
    }

    let span = maximum - minimum;
    let quads = cells
        .into_iter()
        .map(|(low, high, value, flat)| PreviewQuad {
            low,
            high,
            color: match (value, property) {
                (Some(value), Some(property)) => {
                    let fraction = if span > 0.0 {
                        ((value - minimum) / span).clamp(0.0, 1.0) as f32
                    } else {
                        0.5
                    };
                    overlay_property_color(property, fraction, 255)
                }
                _ => flat,
            },
        })
        .collect();
    let strokes = faces
        .iter()
        .flat_map(|face| &face.cycles)
        .filter(|cycle| cycle.len() >= 2)
        .cloned()
        .collect();
    ExamplePreview { quads, strokes }
}

/// Where one horizontal line enters and leaves a face. Every cycle is closed
/// and thrown in together, so the even-odd pairing that falls out excludes the
/// face's own holes without any extra bookkeeping.
fn face_spans(cycles: &[Vec<Point2>], y: f64) -> Vec<(f64, f64)> {
    let mut crossings = Vec::new();
    for cycle in cycles {
        for index in 0..cycle.len() {
            let a = cycle[index];
            let b = cycle[(index + 1) % cycle.len()];
            if (a.y > y) != (b.y > y) {
                crossings.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
            }
        }
    }
    crossings.sort_by(f64::total_cmp);
    crossings
        .chunks_exact(2)
        .filter(|pair| pair[1] > pair[0])
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

fn paint_example_thumbnail(
    ui: &mut egui::Ui,
    example: &funfern_app::topology_examples::TopologyExample,
    preview: Option<&ExamplePreview>,
    size: egui::Vec2,
) -> egui::Response {
    let scene = &example.document.model.accepted;
    let domain = scene.geometry.domain;
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 5.0, Color32::from_rgb(24, 32, 39));
    let inner = rect.shrink(7.0);
    let project = |point: Point2| {
        egui::pos2(
            egui::lerp(
                inner.left()..=inner.right(),
                ((point.x - domain.min_x) / domain.width()) as f32,
            ),
            egui::lerp(
                inner.bottom()..=inner.top(),
                ((point.y - domain.min_y) / domain.height()) as f32,
            ),
        )
    };
    if let Some(preview) = preview {
        let mut mesh = egui::Mesh::default();
        mesh.reserve_vertices(preview.quads.len() * 4);
        mesh.reserve_triangles(preview.quads.len() * 2);
        for quad in &preview.quads {
            let base = mesh.vertices.len() as u32;
            let low = project(quad.low);
            let high = project(quad.high);
            for corner in [
                egui::pos2(low.x, low.y),
                egui::pos2(high.x, low.y),
                egui::pos2(high.x, high.y),
                egui::pos2(low.x, high.y),
            ] {
                mesh.colored_vertex(corner, quad.color);
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
        painter.add(egui::Shape::mesh(mesh));
        for stroke in &preview.strokes {
            let mut points = stroke
                .iter()
                .map(|point| project(*point))
                .collect::<Vec<_>>();
            points.push(points[0]);
            painter.add(egui::Shape::line(points, Stroke::new(1.0, TEAL)));
        }
    }
    let corners = [
        (inner.left_bottom(), inner.right_bottom()),
        (inner.right_bottom(), inner.right_top()),
        (inner.right_top(), inner.left_top()),
        (inner.left_top(), inner.left_bottom()),
    ];
    for side in OuterSide::ALL {
        let (start, end) = corners[side.index()];
        let condition = scene.outer_boundaries.sides[side.index()];
        painter.line_segment(
            [start, end],
            Stroke::new(2.5, outer_condition_color(condition)),
        );
        if condition
            .signal()
            .is_some_and(|signal| signal.characteristic_amplitude() != 0.0)
        {
            painter.circle_filled(start.lerp(end, 0.5), 3.5, GOLD);
        }
    }
    if example.document.model.source.enabled {
        let center = project(example.document.model.source.position);
        painter.circle_stroke(center, 4.5, Stroke::new(1.6, GOLD));
        painter.line_segment(
            [center - egui::vec2(6.0, 0.0), center + egui::vec2(6.0, 0.0)],
            Stroke::new(1.0, GOLD),
        );
        painter.line_segment(
            [center - egui::vec2(0.0, 6.0), center + egui::vec2(0.0, 6.0)],
            Stroke::new(1.0, GOLD),
        );
    }
    painter.rect_stroke(
        rect,
        5.0,
        Stroke::new(1.0, Color32::from_rgb(88, 104, 116)),
        egui::StrokeKind::Inside,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Chooses one topology direction for the next adaptation transaction.
///
/// A limit is a floor rather than a judgement: too few elements across a forced
/// wavelength, or an element larger than the largest allowed, is wrong however
/// small the error reads, so those refine on their own account. What the error
/// estimate asks for answers to the accuracy target instead, and it answers to
/// it for the field as a whole rather than element by element. Without that
/// second clause anything the estimate cannot satisfy - a boundary the field
/// disagrees with, the grid-scale leftovers of a wave that has passed - shrinks
/// by the estimator's step every cycle until it reaches the smallest element
/// allowed, and no accuracy setting reaches far enough to stop it: the step is
/// clamped, so it is the same step whatever the target says.
///
/// The cost is that error concentrated in a small part of a domain the estimate
/// is otherwise happy with stops being chased once the whole field is inside
/// the target. That is the trade a single number for the whole field makes.
///
/// Refinement stops at the requested error, but coarsening does not begin until
/// the estimate is substantially below it. That deadband, plus choosing only
/// one direction per transaction, prevents a moving wave from making the same
/// patch alternate between splitting and collapsing.
fn adaptation_decision(report: &SolutionIndicatorReport, target_accuracy: f64) -> AmrDecision {
    if report.limit_refine_candidates >= 4
        || (report.error_refine_candidates >= 4 && report.global_indicator > target_accuracy)
    {
        AmrDecision::Refine
    } else if report.coarsen_candidates >= 4
        && report.global_indicator < target_accuracy * AMR_COARSEN_GLOBAL_RATIO
    {
        AmrDecision::Coarsen
    } else {
        AmrDecision::Hold
    }
}

/// A resident filter flips the accepted state lane without advancing physical
/// time. The other lane at this boundary is the pre-filter state, not the
/// previous solver endpoint; derivative and residual consumers must not treat
/// the pair as one ordinary `dt` step.
fn resident_filter_boundary(enabled: bool, completed_steps: u64) -> bool {
    enabled && completed_steps > 0 && completed_steps.is_multiple_of(GRID_SCALE_FILTER_CADENCE)
}

/// What the mesh must resolve: the sources, and whatever the medium's own
/// drives add on top of them.
///
/// A driven medium mixes with the wave, so a size rule keyed on the source
/// frequency alone under-resolves it, and a travelling drive patterns the
/// coefficients in space whether or not a wave is present. The scene is
/// authored, so this holds for materials whose drives are not yet executable:
/// it is the mesh rule, not the solver path.
fn scene_resolution_demand(
    scene: &TopologyScene,
    source: PointSource,
) -> CanonicalTemporalResolution {
    let sources = highest_forcing_frequency(scene, source);
    CanonicalTemporalResolution::of_materials(&scene.materials, sources).unwrap_or(
        CanonicalTemporalResolution {
            frequency_hz: sources,
            sideband_order: 0,
            coefficient_wavelength: f64::INFINITY,
        },
    )
}

fn highest_forcing_frequency(scene: &TopologyScene, source: PointSource) -> f64 {
    let mut frequency = if source.enabled {
        source.signal.frequency_ceiling_hz()
    } else {
        0.0
    };
    for condition in scene.outer_boundaries.sides {
        if let Some(signal) = condition.signal() {
            frequency = frequency.max(signal.frequency_ceiling_hz());
        }
    }
    for curve in &scene.geometry.curves {
        for span in &curve.spans {
            if let SpanBehavior::Separated { left, right, .. } = span.behavior {
                for condition in [left, right] {
                    if let Some(signal) = condition.signal() {
                        frequency = frequency.max(signal.frequency_ceiling_hz());
                    }
                }
            }
        }
    }
    for source in &scene.volume_sources {
        if source.enabled {
            frequency = frequency.max(source.signal.frequency_ceiling_hz());
        }
    }
    frequency
}

/// A world-anchored 1/2/5 lattice whose projected spacing is at least the
/// requested arrow spacing. Bounds include a one-cell apron so panning inside
/// the current boundary cells needs no GPU resampling; the painter clips the
/// temporarily off-screen arrows.
fn vector_overlay_lattice(
    scale: f64,
    pixel_spacing: f32,
    center: Point2,
    viewport: Rect,
) -> Option<(f64, [i64; 4])> {
    if !scale.is_finite()
        || scale <= 0.0
        || !pixel_spacing.is_finite()
        || pixel_spacing <= 0.0
        || !center.x.is_finite()
        || !center.y.is_finite()
        || !viewport.is_positive()
    {
        return None;
    }
    let raw = f64::from(pixel_spacing) / scale;
    let power = 10f64.powf(raw.log10().floor());
    let digit = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .find(|digit| digit * power >= raw)?;
    let world_spacing = digit * power;
    let half_width = f64::from(viewport.width()) * 0.5 / scale;
    let half_height = f64::from(viewport.height()) * 0.5 / scale;
    let bin = |value: f64| (value / world_spacing).floor() as i64;
    Some((
        world_spacing,
        [
            bin(center.x - half_width).saturating_sub(1),
            bin(center.x + half_width).saturating_add(1),
            bin(center.y - half_height).saturating_sub(1),
            bin(center.y + half_height).saturating_add(1),
        ],
    ))
}

fn vector_overlay_layout(
    model: TopologyWaveModel<'_>,
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
    world_spacing: f64,
    visible_bins: [i64; 4],
) -> Vec<VectorOverlayLayoutPoint> {
    if mesh.triangles.len() != operator.element_nodes().len()
        || !world_spacing.is_finite()
        || world_spacing <= 0.0
    {
        return vec![];
    }
    let mut bins = BTreeMap::<(i64, i64), (usize, Point2, f64)>::new();
    for (element, triangle) in mesh.triangles.iter().enumerate() {
        let points = triangle.vertices.map(|index| mesh.vertices[index].point);
        let centroid = (points[0] + points[1] + points[2]) / 3.0;
        let key = (
            (centroid.x / world_spacing).floor() as i64,
            (centroid.y / world_spacing).floor() as i64,
        );
        if key.0 < visible_bins[0]
            || key.0 > visible_bins[1]
            || key.1 < visible_bins[2]
            || key.1 > visible_bins[3]
        {
            continue;
        }
        let cell_center = Point2::new(
            (key.0 as f64 + 0.5) * world_spacing,
            (key.1 as f64 + 0.5) * world_spacing,
        );
        let offset = centroid - cell_center;
        let distance = offset.dot(offset);
        if bins.get(&key).is_none_or(|entry| distance < entry.2) {
            bins.insert(key, (element, centroid, distance));
        }
    }
    bins.into_iter()
        .filter_map(|(_key, (element, centroid, _))| {
            let triangle = &mesh.triangles[element];
            let coefficients = model
                .directional_material_at(triangle.region, centroid)
                .ok()?;
            let stencil = QuadraticPointStencil {
                element: element as u32,
                barycentric: [1.0 / 3.0; 3],
                nodes: operator.element_nodes()[element],
                value_weights: enriched_quadratic_basis([1.0 / 3.0; 3]),
                gradient_weights: [Point2::default(); 7],
                region: triangle.region,
                mass_density: coefficients.mass_density,
                stiffness: coefficients.stiffness,
            };
            Some(VectorOverlayLayoutPoint {
                element: element as u32,
                point: centroid,
                stencil,
            })
        })
        .collect()
}

fn aligned_indicator_auxiliary(
    display: &WaveDisplay,
    operator: &QuadraticWaveOperator,
    time_step: f64,
    completed_steps: u64,
) -> Option<Vec<f64>> {
    let count = operator.degrees_of_freedom();
    if !time_step.is_finite()
        || time_step <= 0.0
        || display.snapshot_current.len() != count
        || display.auxiliary.len() != count
    {
        return None;
    }
    Some(
        (0..count)
            .map(|node| {
                if !operator.auxiliary_active()[node]
                    || operator.dirichlet_signals()[node].is_some()
                    || completed_steps == 0
                {
                    display.auxiliary[node] as f64
                } else {
                    display.auxiliary[node] as f64
                        - 0.5
                            * time_step
                            * (display.snapshot_current[node] as f64
                                + display.snapshot_current[node] as f64)
                }
            })
            .collect(),
    )
}

/// The maps a field-dependent generation's readbacks are read through: its
/// operator, the runtime the solver stepped with (the authored one until a
/// snapshot and a clock agree), and the readback's accepted time. `None`
/// where the maps are linear and `Q/M` is exact.
pub(crate) fn field_law_view(
    active: &PreparedTopology,
    canonical: &CanonicalGpuDisplay,
) -> Option<(
    std::sync::Arc<funfern_core::CanonicalTemporalWaveOperator>,
    funfern_core::CanonicalMaterialRuntimeState,
    f64,
)> {
    let operator = active
        .canonical_temporal_operator
        .as_ref()
        .filter(|operator| operator.has_field_laws())?;
    let authored = operator.initial_runtime();
    let runtime = canonical
        .accepted_material_runtime(&authored)
        .unwrap_or(authored);
    let time = canonical.clock.map_or(0.0, |clock| clock.absolute_seconds);
    Some((operator.clone(), runtime, time))
}

fn refresh_canonical_wave_display(
    active: Option<&Arc<PreparedTopology>>,
    canonical: &CanonicalGpuDisplay,
    request: &CanonicalGpuRequest,
    display: &mut WaveDisplay,
) {
    let Some(active) = active else { return };
    let operator = &active.canonical_operator;
    if canonical.generation != request.generation()
        || canonical.primary_flux.len() != operator.degrees_of_freedom()
        || canonical.previous_primary_flux.len() != operator.degrees_of_freedom()
        || canonical.constitutive_force.len() != operator.degrees_of_freedom()
    {
        return;
    }
    if display.generation == canonical.generation && display.readbacks == canonical.readbacks {
        return;
    }
    if display.generation != canonical.generation {
        display.complementary_flux.clear();
        display.previous.clear();
        display.indicator_displacement.clear();
        display.indicator_velocity.clear();
        display.indicator_acceleration.clear();
        display.indicator_potential.clear();
        display.snapshot_current.clear();
        display.snapshot_previous.clear();
        display.snapshot_velocity.clear();
        display.snapshot_completed_steps = 0;
    }
    display.generation = canonical.generation;
    display.completed_steps = request.stats().completed_steps();
    display.current.clear();
    // A field-dependent medium's field is the inverse of its map, not `Q/M`:
    // read at 30% amplitude departure, the linear division would paint a
    // different field from the one the solver steps.
    let nonlinear_field =
        field_law_view(active, canonical).and_then(|(temporal, runtime, time)| {
            let flux = canonical
                .primary_flux
                .iter()
                .map(|value| f64::from(*value))
                .collect::<Vec<_>>();
            temporal.primary_field_at(&flux, time, &runtime).ok()
        });
    match nonlinear_field {
        Some(field) => display
            .current
            .extend(field.into_iter().map(|value| value as f32)),
        None => display.current.extend(
            canonical
                .primary_flux
                .iter()
                .zip(operator.primary_mass())
                .map(|(flux, mass)| (f64::from(*flux) / mass) as f32),
        ),
    }
    if canonical.full_readback_at == canonical.readbacks {
        display.snapshot_current.clear();
        display
            .snapshot_current
            .extend(display.current.iter().copied());
        display.auxiliary.clear();
        display.auxiliary.resize(operator.degrees_of_freedom(), 0.0);
        display.snapshot_completed_steps = canonical.full_snapshot_completed_steps();
        display.complementary_flux.clear();
        display
            .complementary_flux
            .extend(canonical.complementary_flux.iter().copied());
    }
    display.readbacks = canonical.readbacks;
}

#[allow(clippy::too_many_arguments)]
pub fn frame(
    mut contexts: EguiContexts,
    mut state: ResMut<Playground>,
    time: Res<Time>,
    mut recorders: ResMut<WaveGpuRequest>,
    mut request: ResMut<CanonicalGpuRequest>,
    canonical_display: Res<CanonicalGpuDisplay>,
    mut display: ResMut<WaveDisplay>,
    probe_display: Res<ProbeDisplay>,
    curve_display: Res<CurveProbeDisplay>,
    area_display: Res<AreaProbeDisplay>,
    far_display: Res<FarFieldDisplay>,
    vector_display: Res<VectorOverlayDisplay>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut commands: Commands,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    if !state.ready {
        ctx.set_visuals(egui::Visuals::dark());
        ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(8.0, 4.0);
            style.visuals.selection.bg_fill = Color32::from_rgb(38, 94, 135);
        });
        state.ready = true;
        #[cfg(target_arch = "wasm32")]
        if let Some(element) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("startup"))
        {
            let _ = element.set_attribute("hidden", "");
        }
    }
    state.frame_ms = state.frame_ms * 0.95 + time.delta_secs() * 1000.0 * 0.05;
    state.frame_delta = time.delta_secs();
    state.update_files();
    state.update_recording();
    state.begin_capture_frame();
    if !state.startup_done {
        state.startup_done = true;
        if let Some(bytes) = crate::sharing::initial_fragment() {
            match bytes.and_then(|bytes| persistence::parse(&bytes)) {
                Ok(candidate) => {
                    state.load = Some(candidate);
                    state.file_busy = true;
                }
                Err(error) => state.message = error,
            }
        } else {
            match crate::recovery::load()
                .ok()
                .flatten()
                .and_then(|bytes| persistence::parse(&bytes).ok())
            {
                Some(candidate) => {
                    state.load = Some(candidate);
                    state.file_busy = true;
                }
                // A first run, or an autosave that went away or stopped
                // parsing. Open a random example rather than the same one
                // every time, so the app opens on something worth looking at.
                None => state.open_random_example(),
            }
        }
    }
    state.editor.validate_frame(12000);
    state.refresh_runtime(
        &mut request,
        &canonical_display,
        &mut recorders,
        &vector_display,
        &mut assets,
        &mut commands,
        time.delta_secs_f64(),
    );
    refresh_canonical_wave_display(
        state.runtime.active(),
        &canonical_display,
        &request,
        &mut display,
    );
    state.refresh_amr(&request, &canonical_display, &display);
    state.ingest_probes(&probe_display);
    state.ingest_spatial_probes(&curve_display, &area_display, &far_display);
    state.refresh_probe_metadata();
    state.refresh_material_overlay();
    let mut root = egui::Ui::new(
        ctx.clone(),
        "root".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );
    let logical_canvas = ctx.viewport_rect();
    let logical_viewport = state.show(&mut root, &display, &vector_display);
    if state.snapshot_state == SnapshotState::Armed {
        state.snapshot_state = SnapshotState::Capturing;
        let sender = state.sender.clone();
        commands.spawn(Screenshot::primary_window()).observe(
            move |captured: On<ScreenshotCaptured>| {
                let event = match crate::capture::encode_viewport_png(
                    captured.image.clone(),
                    logical_canvas,
                    logical_viewport,
                ) {
                    Ok(bytes) => FileEvent::SnapshotCaptured(bytes),
                    Err(error) => FileEvent::Error(error),
                };
                let _ = sender.send(event);
            },
        );
    }
    if state.recording_state == RecordingState::Preparing {
        let pixels_per_point = ctx.pixels_per_point() as f64;
        let physical_width = (logical_canvas.width() as f64 * pixels_per_point).round() as u32;
        let physical_height = (logical_canvas.height() as f64 * pixels_per_point).round() as u32;
        let result = crate::capture::pixel_crop(
            logical_canvas,
            logical_viewport,
            physical_width,
            physical_height,
        )
        .and_then(crate::capture::video_dimensions)
        .and_then(|(width, height)| {
            state.video_recorder.start(
                RecordingSpec {
                    width,
                    height,
                    fps: recording::VIDEO_FPS,
                },
                logical_canvas,
                logical_viewport,
            )
        });
        match result {
            Ok(()) => state.recording_state = RecordingState::Starting,
            Err(error) => {
                state.recording_state = RecordingState::Idle;
                state.notify(error);
            }
        }
    }
    if matches!(
        state.recording_state,
        RecordingState::Starting | RecordingState::Recording
    ) {
        state
            .video_recorder
            .update_viewport(logical_canvas, logical_viewport);
    }
    #[cfg(not(target_arch = "wasm32"))]
    if state.recording_state == RecordingState::Recording
        && let Some(started) = state.recording_started
    {
        let slot = (started.elapsed().as_secs_f64() * recording::VIDEO_FPS as f64).floor() as u64;
        if state.recording_last_requested_slot != Some(slot)
            && state.recording_readback_in_flight.load(Ordering::Acquire)
                < recording::MAX_READBACKS_IN_FLIGHT
            && let Some(target) = state.video_recorder.native_frame_target()
        {
            state.recording_last_requested_slot = Some(slot);
            // The schedule is the only producer, so the load above and this
            // increment cannot race each other; the capture observers only
            // ever decrement.
            state
                .recording_readback_in_flight
                .fetch_add(1, Ordering::AcqRel);
            let in_flight = state.recording_readback_in_flight.clone();
            commands.spawn(Screenshot::primary_window()).observe(
                move |captured: On<ScreenshotCaptured>| {
                    target.submit(
                        captured.image.clone(),
                        logical_canvas,
                        logical_viewport,
                        slot,
                    );
                    in_flight.fetch_sub(1, Ordering::AcqRel);
                },
            );
        }
    }
    state.autosave();
    Ok(())
}
