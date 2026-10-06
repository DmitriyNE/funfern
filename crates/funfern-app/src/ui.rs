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
use funfern_app::document::{ProbeId, ProbeSamplingPreset, VectorOverlayStyle};
use funfern_app::topology_examples::ExampleGroup;
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
mod guide;
mod input;
mod inspectors;
mod law_editor;
mod line_plot;
mod materials;
mod pacing;
mod paint;
mod panels;
mod probe_hit;
mod probe_view;
mod probes;
mod readouts;
mod runtime;
mod scene_card;
mod selection;

mod session;
mod signals;
mod state;
mod streamlines;
#[cfg(test)]
mod test_support;
mod theme;
mod transfer;
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
use guide::*;
use pacing::*;
use probe_view::*;
use signals::*;
use streamlines::*;
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

/// Set by a pool worker just before it aborts: 2 when an allocation failed,
/// 1 on any other panic. The hooks `main` installs write it. A worker that
/// aborts is gone for good - the page is not told, and the job it held never
/// reports back - so without this the app would wait on that job forever.
#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
pub(crate) static BROWSER_BACKGROUND_FAILURE: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(0);

/// What to tell the user once a background worker has aborted. The pool does
/// not replace it, so edits and adaptation cannot complete again; the field
/// that is running keeps running.
fn browser_background_failure() -> Option<&'static str> {
    #[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
    match BROWSER_BACKGROUND_FAILURE.load(Ordering::Acquire) {
        0 => {}
        2 => {
            return Some(
                "A background worker ran out of memory. The simulation keeps running; \
                 reload the page to edit it or adapt the mesh",
            );
        }
        _ => {
            return Some(
                "A background worker stopped on an error. The simulation keeps running; \
                 reload the page to edit it or adapt the mesh",
            );
        }
    }
    None
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

/// What a region is called until it is given a name: Background for the
/// outer one, Region n for the rest.
fn default_region_name(region: RegionId) -> String {
    if region == BACKGROUND_REGION {
        "Background".into()
    } else {
        format!("Region {}", region.0)
    }
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

impl InspectorPanel {
    const ALL: [Self; 5] = [
        Self::Edit,
        Self::View,
        Self::Simulation,
        Self::Materials,
        Self::Probes,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Edit => "Edit",
            Self::View => "View",
            Self::Simulation => "Simulation",
            Self::Materials => "Materials",
            Self::Probes => "Probes",
        }
    }
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

impl DrawTool {
    /// Whether `points` placed are enough to finish this tool's curve: what
    /// Finish and Enter both ask. A circle and a rectangle are made by their
    /// second click, before either is asked.
    fn finishes_with(self, points: usize) -> bool {
        match self {
            Self::Circle | Self::Rectangle => points == 2,
            Self::Polygon => points >= 3,
            Self::ClosedSpline => points >= 4,
            Self::Polyline | Self::OpenSpline => points >= 2,
        }
    }

    /// Whether the points placed are vertices the curve passes through, drawn
    /// as squares, rather than controls it is drawn toward, drawn as rings.
    fn places_vertices(self) -> bool {
        matches!(self, Self::Rectangle | Self::Polygon | Self::Polyline)
    }

    /// Whether the tool draws an open curve rather than a closed one.
    fn draws_open(self) -> bool {
        matches!(self, Self::Polyline | Self::OpenSpline)
    }

    /// Whether tapping the first point again closes the loop, so that point
    /// is drawn larger.
    fn closes_on_first(self) -> bool {
        matches!(self, Self::Polygon | Self::ClosedSpline)
    }

    /// What the tool is waiting for, said in the Draw palette while it draws.
    fn prompt(self) -> &'static str {
        match self {
            Self::Circle => "Place the centre, then a point on the circle",
            Self::Rectangle => "Place two opposite corners",
            Self::Polygon => "Place the corners; the first again closes the loop",
            Self::ClosedSpline => "Place the controls; the first again closes the loop",
            Self::Polyline => "Place the vertices; an attachment ends the line",
            Self::OpenSpline => "Place the controls; an attachment ends the curve",
        }
    }
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

impl HandoffRecord {
    /// Wall seconds from the request to the generation's commit.
    fn seconds(&self) -> f64 {
        (self.prepare_ms + self.pack_ms + self.drain_ms + self.upload_ms) / 1000.0
    }
}

struct Uploading {
    token: TopologyToken,
    generation: u64,
    fresh: bool,
    degrees_of_freedom: usize,
    /// The step the candidate runs at once the device publishes it.
    time_step: f64,
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
    style: VectorOverlayStyle,
    world_spacing: f64,
    visible_bins: [i64; 4],
}

#[derive(Clone, Copy, Debug)]
struct VectorOverlayLayoutPoint {
    /// The sample's filter history key: the element for an arrow, which a
    /// pan keeps, and the world cell for a streamline lattice sample, which
    /// a pan keeps too. See [`lattice_cell_key`].
    key: u64,
    point: Point2,
    stencil: QuadraticPointStencil,
}

/// The filter key of a lattice cell. The high bit keeps the cells apart
/// from the element keys the arrows use, so a style change cannot hand a
/// cell an element's history.
fn lattice_cell_key(column: i64, row: i64) -> u64 {
    (1 << 63) | (u64::from(column as i32 as u32) << 32) | u64::from(row as i32 as u32)
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

/// One arrow's presentation filter, keyed by the mesh element it samples.
/// The AC coupler works in `input` and `output`; the low-pass runs its stages
/// through `inner` into `output`. Nothing here reaches the solver, a probe
/// or the energy.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct VectorFilterState {
    input: Point2,
    output: Point2,
    /// The low-pass's stages before its last, which is `output`.
    inner: [Point2; VECTOR_LOW_PASS_STAGES - 1],
    step: u64,
    time: f64,
    origin: Pos2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VectorOverlayFilterOwner {
    mesh_revision: u64,
    physics: PhysicsModel,
}

/// Which presentation filter the arrows are drawn through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VectorFilter {
    /// Complementary-field arrows less a slow baseline.
    AcCoupled,
    /// Energy-flow arrows averaged below the view's corner.
    LowPass,
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
    /// Frames the domain in `viewport`. The fit stays on until a gesture
    /// takes the camera, so the view is fitted to the window the scene is
    /// actually shown in rather than to the first frame's: a window tiler
    /// resizes a new window after it appears, and fitted once on that first
    /// frame the scene opened at twice the zoom, or more, by desktop.
    fn fit_view(&mut self, viewport: Rect) {
        let domain = self.editor.document.model.draft.geometry.domain;
        self.center = domain.center();
        self.scale = (viewport.width() as f64 / domain.width())
            .min(viewport.height() as f64 / domain.height())
            * 0.88;
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
            refused: None,
        });
        self.selection = TopologySelection::None;
    }
    /// Whether a gesture or a placement is under way: anything Escape would
    /// cancel.
    fn interaction_in_progress(&self) -> bool {
        self.drag.is_some()
            || self.draw.is_some()
            || self.editor.editing()
            || self.pending_merge.is_some()
            || self.pulse_mode
            || self.probe_mode.is_some()
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
        self.history_shortcuts(root.ctx());
        self.spotlights.clear();
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
            self.pulse_shape_window(root.ctx());
            self.diagnostics_window(root.ctx());
            self.formula_help_window(root.ctx());
            self.notice_window(root.ctx());
            self.scene_card(root.ctx(), viewport);
            self.examples_window(root.ctx());
            self.guide_frame(root.ctx(), viewport);
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
const FORMULA_LAWS: [(&str, &str); 14] = [
    (
        "c = c₀(x)·g·d(t)·s(t)",
        "a coefficient is its base value times the field response, the drive and the Switch; \
         Divide makes the drive and Switch divide it instead",
    ),
    (
        "field argument",
        "the density or permittivity row reads the primary field; the other row reads the \
         magnitude of the complementary field",
    ),
    (
        "Kerr  g = 1 + χ|u|²",
        "χ is relative, per unit |u|²; χ < 0 defocuses and needs an amplitude bound",
    ),
    (
        "Saturable  g = 1 + χ|u|²/(1 + |u|²/σ²)",
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
         admitted amplitude, and by a restoring law's largest curvature",
    ),
    (
        "Restoring  M₀ d²r/dt² + Kr + M₀V′(r) = 0",
        "r = ∫u dt is carried as state; the field shown is u = dr/dt, so a static kink shows \
         u = 0 and the Integrated field view shows r",
    ),
    (
        "Klein-Gordon  V = ½ω₀²r²",
        "a cutoff at ω₀: waves below it do not propagate",
    ),
    (
        "sine-Gordon  V = ω₀²(1 − cos r)",
        "kinks of one 2π step in r, slower than c and contracted as they run",
    ),
    (
        "φ⁴  V = ¼λ(r² − 1)²",
        "wells at r = ±1 and an unstable top at 0; the bound caps |r| and sets the step",
    ),
    (
        "Self-oscillating  γ = γ₀(|u|²/a² − 1)",
        "van der Pol: gain below a, loss above, counted as active gain; only beside a \
         linear response",
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

/// The displayed field and its time integral, as the view selector names them.
fn integrated_field_labels(physics: PhysicsModel) -> (&'static str, &'static str) {
    match physics {
        PhysicsModel::Mechanical => ("Displacement u", "Integrated ∫u dt"),
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        } => ("Field E_z", "Integrated −A_z"),
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        } => ("Field H_z", "Integrated ∫H_z dt"),
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

/// `fire` is when "Fire now" starts a driven face's pulse, and `preview`
/// the shape window its "Shape" opens.
fn edit_face_condition(
    ui: &mut egui::Ui,
    physics: PhysicsModel,
    condition: &mut FaceBoundaryCondition,
    fire: FireTimes,
    preview: &mut PulsePreview,
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
    sized_combo(ui, "span-condition", BoundaryKind::choices(physics).len())
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
        FaceBoundaryCondition::Neumann { signal } => {
            edit_time_signal(ui, signal, SignalUse::Flux, fire, preview);
        }
        FaceBoundaryCondition::Dirichlet { signal } => {
            edit_time_signal(ui, signal, SignalUse::Field, fire, preview);
        }
        _ => {}
    }
    *condition != before
}

/// `fire` is when "Fire now" starts a driven wall's pulse, and `preview`
/// the shape window its "Shape" opens.
/// Edits an outer boundary's law. Answers whether it changed, and where the
/// list drew its second-order outgoing entry while the list is open.
fn edit_outer_condition(
    ui: &mut egui::Ui,
    physics: PhysicsModel,
    condition: &mut OuterBoundaryCondition,
    fire: FireTimes,
    preview: &mut PulsePreview,
) -> (bool, Option<Rect>) {
    let before = *condition;
    let mut kind = outer_kind(*condition).presented(physics);
    let outgoing = sized_combo(ui, "outer-condition", BoundaryKind::choices(physics).len())
        .selected_text(kind.label_for(physics))
        .show_ui(ui, |ui| boundary_kind_choices(ui, physics, &mut kind))
        .inner
        .flatten();
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
        OuterBoundaryCondition::Neumann { signal } => {
            edit_time_signal(ui, signal, SignalUse::Flux, fire, preview);
        }
        OuterBoundaryCondition::Dirichlet { signal } => {
            edit_time_signal(ui, signal, SignalUse::Field, fire, preview);
        }
        _ => {}
    }
    (*condition != before, outgoing)
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

/// The list of laws, and where the second-order outgoing one was drawn,
/// which the guided tour lights.
fn boundary_kind_choices(
    ui: &mut egui::Ui,
    physics: PhysicsModel,
    kind: &mut BoundaryKind,
) -> Option<Rect> {
    let mut outgoing = None;
    for value in BoundaryKind::choices(physics) {
        let response = ui.selectable_value(kind, *value, value.label_for(physics));
        if *value == BoundaryKind::SecondOrder {
            outgoing = Some(response.rect);
        }
    }
    outgoing
}

/// The share of the window a combo's list may take before it scrolls.
const COMBO_LIST_SHARE: f32 = 0.6;

/// How tall a combo's list of `rows` entries is drawn: whole when it fits in
/// [`COMBO_LIST_SHARE`] of the window, or when only half an entry would be
/// left out, and otherwise cut through the middle of an entry, so the half
/// showing says there is more. A list cut at an entry's edge looked complete:
/// the boundary list showed four of its six that way.
///
/// The row is measured from the style, as a selectable entry lays itself out:
/// its text, its padding above and below, and the spacing to the next. A
/// rule counting `interact_size` rows ran 18 px against the theme's 29.
fn combo_list_height(ui: &egui::Ui, rows: usize) -> f32 {
    let spacing = ui.spacing();
    // A laid-out line, which the text layout rounds to whole pixels, rather
    // than the font's own height: 14.97 px of font is a 15 px entry.
    let text = ui
        .painter()
        .layout_no_wrap(
            "Ag".to_owned(),
            egui::TextStyle::Button.resolve(ui.style()),
            egui::Color32::WHITE,
        )
        .size()
        .y;
    let row = (text + 2.0 * spacing.button_padding.y).max(spacing.interact_size.y);
    let gap = spacing.item_spacing.y;
    let rows_tall = |count: f32| count * row + (count - 1.0).max(0.0) * gap;
    let room = COMBO_LIST_SHARE * ui.ctx().content_rect().height();
    // How many entries the room holds, fractionally.
    let capacity = (room + gap) / (row + gap);
    if rows as f32 <= capacity + 0.5 {
        return rows_tall(rows as f32);
    }
    let whole = (capacity - 0.5).floor().max(1.0);
    rows_tall(whole) + gap + 0.5 * row
}

/// A combo whose list holds `rows` entries, drawn [`combo_list_height`] tall
/// under an id that carries the count. egui keeps a popup's size from its
/// last showing and does not grow it for a longer list: the vector overlay's
/// list, once opened on its two Mechanical entries, showed two and a half of
/// its three in the EM skins from then on, and an explicit height did not
/// help. With the count in its id each length is sized on its first showing.
fn sized_combo(ui: &egui::Ui, id_salt: impl egui::AsIdSalt, rows: usize) -> egui::ComboBox {
    egui::ComboBox::from_id_salt((id_salt, rows)).height(combo_list_height(ui, rows))
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

/// A gallery tile's width, which is also its thumbnail's side.
const EXAMPLE_TILE: f32 = 108.0;
const EXAMPLE_TILE_GAP: f32 = 10.0;
/// The most tiles a gallery row holds. Most sections fit one where the
/// screen has room; a longer one folds into a second.
const GALLERY_COLUMNS: usize = 7;

/// Room a scroll bar may take beside the tiles.
const EXAMPLE_SCROLL_BAR: f32 = 16.0;

/// How many tiles fit across `width`, never fewer than one.
fn gallery_columns(width: f32) -> usize {
    (((width - EXAMPLE_SCROLL_BAR + EXAMPLE_TILE_GAP) / (EXAMPLE_TILE + EXAMPLE_TILE_GAP)).floor()
        as usize)
        .max(1)
}

/// The width `columns` tiles take, with room for the scroll bar.
fn gallery_width(columns: usize) -> f32 {
    columns as f32 * (EXAMPLE_TILE + EXAMPLE_TILE_GAP) - EXAMPLE_TILE_GAP + EXAMPLE_SCROLL_BAR
}

/// One scene in the gallery: its thumbnail over its name, the whole tile one
/// button, and what the scene shows on hover. The open scene's tile is
/// outlined in gold.
fn example_tile(
    ui: &mut egui::Ui,
    example: &funfern_app::topology_examples::TopologyExample,
    preview: Option<&ExamplePreview>,
    opened: bool,
) -> egui::Response {
    let name = ui.painter().layout(
        example.name.to_owned(),
        egui::TextStyle::Body.resolve(ui.style()),
        ui.visuals().text_color(),
        EXAMPLE_TILE,
    );
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(EXAMPLE_TILE, EXAMPLE_TILE + 4.0 + name.size().y),
        Sense::click(),
    );
    let thumbnail = Rect::from_min_size(rect.min, egui::vec2(EXAMPLE_TILE, EXAMPLE_TILE));
    let painter = ui.painter();
    paint_example_thumbnail(painter, thumbnail, example, preview);
    if opened || response.hovered() {
        let (width, color) = if opened { (2.0, GOLD) } else { (1.5, SELECT) };
        painter.rect_stroke(
            thumbnail,
            5.0,
            Stroke::new(width, color),
            egui::StrokeKind::Inside,
        );
    }
    let color = if opened {
        GOLD
    } else {
        ui.visuals().text_color()
    };
    painter.galley(
        Pos2::new(rect.left(), thumbnail.bottom() + 4.0),
        name,
        color,
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_ui(|ui| {
            ui.set_max_width(320.0);
            ui.label(example.description);
        })
}

fn paint_example_thumbnail(
    painter: &egui::Painter,
    rect: Rect,
    example: &funfern_app::topology_examples::TopologyExample,
    preview: Option<&ExamplePreview>,
) {
    let scene = &example.document.model.accepted;
    let domain = scene.geometry.domain;
    let painter = painter.with_clip_rect(rect);
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

/// Whether a refinement is the size rule's rather than the accuracy
/// target's: elements past their wavelength floor, with the error itself
/// under target or too sparse to refine for. The estimate line says so, or
/// an error well under its target beside a mesh still refining reads as a
/// contradiction.
fn size_rule_refines(report: &SolutionIndicatorReport, target_accuracy: f64) -> bool {
    adaptation_decision(report, target_accuracy) == AmrDecision::Refine
        && !(report.error_refine_candidates >= 4 && report.global_indicator > target_accuracy)
}

/// The size field a refinement applies. One the size rule asks for alone
/// applies the limits alone: handed the full field it also splits whatever
/// the estimate would like finer, which the accuracy target has just said is
/// not needed, and the parametric fiber climbed to 19,000 triangles at 2% of
/// a 12% target before coarsening back what the floor never asked for.
fn adaptation_field(
    result: &SolutionIndicatorResult,
    target_accuracy: f64,
) -> Arc<AdaptiveSizeField> {
    if size_rule_refines(&result.report, target_accuracy) {
        result.limit_field.clone()
    } else {
        result.field.clone()
    }
}

/// What each material's own drives demand of the mesh beyond the sources.
///
/// A travelling drive patterns the coefficients in space whether or not a
/// wave is present, and only where its material is; the sidebands any drive
/// mixes into the field are left to the error estimate. The scene is
/// authored, so this holds for materials whose drives are not yet
/// executable: it is the mesh rule, not the solver path.
fn scene_resolution_demand(scene: &TopologyScene) -> BTreeMap<MaterialId, f64> {
    CanonicalTemporalResolution::of_each_material(&scene.materials).unwrap_or_default()
}

/// The most `measure` gives of any signal the scene drives with: the point
/// source, the driven walls and faces, and the region sources.
fn highest_forcing(
    scene: &TopologyScene,
    source: PointSource,
    measure: impl Fn(TimeSignal) -> f64,
) -> f64 {
    let mut frequency = if source.enabled {
        measure(source.signal)
    } else {
        0.0
    };
    for condition in scene.outer_boundaries.sides {
        if let Some(signal) = condition.signal() {
            frequency = frequency.max(measure(signal));
        }
    }
    for curve in &scene.geometry.curves {
        for span in &curve.spans {
            if let SpanBehavior::Separated { left, right, .. } = span.behavior {
                for condition in [left, right] {
                    if let Some(signal) = condition.signal() {
                        frequency = frequency.max(measure(signal));
                    }
                }
            }
        }
    }
    for source in &scene.volume_sources {
        if source.enabled {
            frequency = frequency.max(measure(source.signal));
        }
    }
    frequency
}

/// A world-anchored lattice whose projected spacing is at least the
/// requested arrow spacing. The ladder is √2, as the streamlines' is, so the
/// arrows stand within 1.4 of the spacing asked for: a 1/2/5 ladder let them
/// drift to 2.5 times it with the zoom, sparse and short against their
/// gaps. Bounds include a one-cell apron so panning inside the current
/// boundary cells needs no GPU resampling; the painter clips the temporarily
/// off-screen arrows.
fn vector_overlay_lattice(
    scale: f64,
    pixel_spacing: f32,
    center: Point2,
    viewport: Rect,
) -> Option<(f64, [i64; 4])> {
    world_lattice(
        scale,
        pixel_spacing,
        center,
        viewport,
        2.0,
        &[1.0, std::f64::consts::SQRT_2],
    )
}

/// The streamlines are placed `pixel_separation` apart but integrated
/// through a finer lattice, a third of that, so a line can follow a feature
/// the arrows' spacing would step over. The ladder is √2 rather than 1/2/5
/// so the real spacing stays within 1.4 of the target. The floor keeps the
/// lattice under the sampler's cap on any viewport: at most about 12,000
/// cells cover the view, and the apron adds a few hundred.
fn streamline_lattice(
    scale: f64,
    pixel_separation: f32,
    center: Point2,
    viewport: Rect,
) -> Option<(f64, [i64; 4])> {
    if !viewport.is_positive() {
        return None;
    }
    let by_cap = (viewport.area() / 12_000.0).sqrt();
    let pixel_spacing = (pixel_separation / 3.0)
        .max(STREAMLINE_LATTICE_FLOOR_PIXELS)
        .max(by_cap);
    world_lattice(
        scale,
        pixel_spacing,
        center,
        viewport,
        2.0,
        &[1.0, std::f64::consts::SQRT_2],
    )
}

const STREAMLINE_LATTICE_FLOOR_PIXELS: f32 = 14.0;

/// A world-anchored lattice whose projected spacing is at least
/// `pixel_spacing`: the spacing is the first rung of `ladder`, scaled to the
/// decade of `base`, that reaches it. The bins are counted from the world
/// origin and widened by one cell on each side.
fn world_lattice(
    scale: f64,
    pixel_spacing: f32,
    center: Point2,
    viewport: Rect,
    base: f64,
    ladder: &[f64],
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
    let power = base.powf(raw.log(base).floor());
    let digit = ladder
        .iter()
        .copied()
        .chain(std::iter::once(base))
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
                key: u64::from(element as u32),
                point: centroid,
                stencil,
            })
        })
        .collect()
}

/// The streamline lattice: a sample at the centre of every cell the mesh
/// covers, in the element that holds the centre, so the lattice is regular
/// and the field between its samples reads bilinearly. A cell whose centre
/// no element holds, outside the domain, has no sample, which is where a
/// line ends. Elements are binned by their bounding boxes first, so each
/// centre is tested against the few elements over its cell rather than the
/// whole mesh.
fn streamline_layout(
    model: TopologyWaveModel<'_>,
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
    world_spacing: f64,
    visible_bins: [i64; 4],
) -> Vec<VectorOverlayLayoutPoint> {
    if mesh.triangles.len() != operator.element_nodes().len()
        || !world_spacing.is_finite()
        || world_spacing <= 0.0
        || visible_bins[1] < visible_bins[0]
        || visible_bins[3] < visible_bins[2]
    {
        return vec![];
    }
    let bin = |value: f64| (value / world_spacing).floor() as i64;
    let mut over_cell = BTreeMap::<(i64, i64), Vec<usize>>::new();
    for (element, triangle) in mesh.triangles.iter().enumerate() {
        let points = triangle.vertices.map(|index| mesh.vertices[index].point);
        let (mut low, mut high) = (points[0], points[0]);
        for point in &points[1..] {
            low = Point2::new(low.x.min(point.x), low.y.min(point.y));
            high = Point2::new(high.x.max(point.x), high.y.max(point.y));
        }
        let columns = bin(low.x).max(visible_bins[0])..=bin(high.x).min(visible_bins[1]);
        let rows = bin(low.y).max(visible_bins[2])..=bin(high.y).min(visible_bins[3]);
        for column in columns {
            for row in rows.clone() {
                over_cell.entry((column, row)).or_default().push(element);
            }
        }
    }
    over_cell
        .into_iter()
        .filter_map(|((column, row), elements)| {
            let center = Point2::new(
                (column as f64 + 0.5) * world_spacing,
                (row as f64 + 0.5) * world_spacing,
            );
            let (element, barycentric) = elements.into_iter().find_map(|element| {
                let points = mesh.triangles[element]
                    .vertices
                    .map(|index| mesh.vertices[index].point);
                barycentric_in(center, points).map(|barycentric| (element, barycentric))
            })?;
            let triangle = &mesh.triangles[element];
            let coefficients = model
                .directional_material_at(triangle.region, center)
                .ok()?;
            let stencil = QuadraticPointStencil {
                element: element as u32,
                barycentric,
                nodes: operator.element_nodes()[element],
                value_weights: enriched_quadratic_basis(barycentric),
                gradient_weights: [Point2::default(); 7],
                region: triangle.region,
                mass_density: coefficients.mass_density,
                stiffness: coefficients.stiffness,
            };
            Some(VectorOverlayLayoutPoint {
                key: lattice_cell_key(column, row),
                point: center,
                stencil,
            })
        })
        .collect()
}

/// The barycentric coordinates of `point` in the triangle, when it lies
/// inside or on its edge within a rounding tolerance, clamped and renormalised
/// so they are a valid interpolation even on the edge.
fn barycentric_in(point: Point2, [a, b, c]: [Point2; 3]) -> Option<[f64; 3]> {
    let area = (b - a).cross(c - a);
    if !area.is_finite() || area.abs() <= f64::MIN_POSITIVE {
        return None;
    }
    let mut weights = [
        (b - point).cross(c - point) / area,
        (c - point).cross(a - point) / area,
        (a - point).cross(b - point) / area,
    ];
    const TOLERANCE: f64 = 1.0e-9;
    if weights.iter().any(|weight| *weight < -TOLERANCE) {
        return None;
    }
    for weight in &mut weights {
        *weight = weight.max(0.0);
    }
    let total: f64 = weights.iter().sum();
    Some(weights.map(|weight| weight / total))
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

/// The maps a generation's readbacks are read through, when `departs` says
/// they are not the fixed ones: its time-driven operator, the runtime the
/// solver stepped with (the authored one until a snapshot and a clock agree),
/// and the readback's accepted time.
fn temporal_view(
    active: &PreparedTopology,
    canonical: &CanonicalGpuDisplay,
    departs: impl Fn(&funfern_core::CanonicalTemporalWaveOperator) -> bool,
) -> Option<(
    std::sync::Arc<funfern_core::CanonicalTemporalWaveOperator>,
    funfern_core::CanonicalMaterialRuntimeState,
    f64,
)> {
    let operator = active
        .canonical_temporal_operator
        .as_ref()
        .filter(|operator| departs(operator))?;
    let authored = operator.initial_runtime();
    let runtime = canonical
        .accepted_material_runtime(&authored)
        .unwrap_or(authored);
    let time = canonical.clock.map_or(0.0, |clock| clock.absolute_seconds);
    Some((operator.clone(), runtime, time))
}

/// The maps stored energy is read through, when a coefficient varies in time,
/// follows its field, or a restoring law holds energy: each stores what the
/// fixed breakdown, built on the authored coefficients, does not know.
pub(crate) fn stored_energy_view(
    active: &PreparedTopology,
    canonical: &CanonicalGpuDisplay,
) -> Option<(
    std::sync::Arc<funfern_core::CanonicalTemporalWaveOperator>,
    funfern_core::CanonicalMaterialRuntimeState,
    f64,
)> {
    temporal_view(active, canonical, |operator| {
        operator.has_temporal_laws() || operator.has_field_laws() || operator.has_restoring()
    })
}

/// The maps the painted field is read through, when a coefficient varies in
/// time or follows its field. `None` where the maps are the authored ones and
/// `Q/M` is exact.
fn primary_field_view(
    active: &PreparedTopology,
    canonical: &CanonicalGpuDisplay,
) -> Option<(
    std::sync::Arc<funfern_core::CanonicalTemporalWaveOperator>,
    funfern_core::CanonicalMaterialRuntimeState,
    f64,
)> {
    temporal_view(active, canonical, |operator| {
        operator.has_temporal_laws() || operator.has_field_laws()
    })
}

/// The maps of a generation whose coefficients follow their own field, for
/// the readouts only such a medium has.
pub(crate) fn field_law_view(
    active: &PreparedTopology,
    canonical: &CanonicalGpuDisplay,
) -> Option<(
    std::sync::Arc<funfern_core::CanonicalTemporalWaveOperator>,
    funfern_core::CanonicalMaterialRuntimeState,
    f64,
)> {
    temporal_view(active, canonical, |operator| operator.has_field_laws())
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
    if display.generation != canonical.generation {
        display.picture.clear();
        display.picture_serial = 0;
    }
    if let Some(copy) = canonical.picture()
        && copy.values.len() == operator.degrees_of_freedom()
        && copy.serial != display.picture_serial
    {
        display.picture = primary_field(active, canonical, &copy.values);
        display.picture_serial = copy.serial;
    }
    display.picture_integrated.clear();
    if let Some(copy) = canonical.integrated_picture() {
        display
            .picture_integrated
            .extend(copy.values.iter().copied());
    }
    // Gate O: `r` arrives on a stream of its own, and the first copy of a new
    // generation may come with its handoff receipt before any other readback.
    if display.generation != canonical.generation
        || display.live_integrated_readbacks != canonical.live_integrated_readbacks
    {
        display.live_integrated.clear();
        if canonical.live_integrated.len() == operator.degrees_of_freedom() {
            display
                .live_integrated
                .extend(canonical.live_integrated.iter().copied());
        }
        display.live_integrated_readbacks = canonical.live_integrated_readbacks;
    }
    if display.generation == canonical.generation && display.readbacks == canonical.readbacks {
        return;
    }
    if display.generation != canonical.generation {
        display.complementary_flux.clear();
        display.snapshot_current.clear();
        display.snapshot_previous.clear();
        display.snapshot_velocity.clear();
        display.snapshot_integrated.clear();
        display.snapshot_completed_steps = 0;
    }
    display.generation = canonical.generation;
    display.completed_steps = request.stats().completed_steps();
    display.current = primary_field(active, canonical, &canonical.primary_flux);
    if canonical.full_readback_at == canonical.readbacks {
        display.snapshot_current.clear();
        display
            .snapshot_current
            .extend(display.current.iter().copied());
        display.auxiliary.clear();
        display.auxiliary.resize(operator.degrees_of_freedom(), 0.0);
        display.snapshot_completed_steps = canonical.full_snapshot_completed_steps();
        display.snapshot_integrated.clear();
        display
            .snapshot_integrated
            .extend(canonical.integrated_field().iter().copied());
        display.complementary_flux.clear();
        display
            .complementary_flux
            .extend(canonical.complementary_flux.iter().copied());
    }
    display.readbacks = canonical.readbacks;
}

/// The primary field of a copy's flux. A field-dependent medium's field is
/// the inverse of its map, not `Q/M`: read at 30% amplitude departure, the
/// linear division would paint a different field from the one the solver
/// steps. A switched or pumped medium divides by its mass at that time, not
/// the authored one. The map is read at the latest clock, which a played-out
/// copy lags by a frame or so.
fn primary_field(
    active: &Arc<PreparedTopology>,
    canonical: &CanonicalGpuDisplay,
    flux: &[f32],
) -> Vec<f32> {
    let mapped_field =
        primary_field_view(active, canonical).and_then(|(temporal, runtime, time)| {
            let flux = flux
                .iter()
                .map(|value| f64::from(*value))
                .collect::<Vec<_>>();
            temporal.primary_field_at(&flux, time, &runtime).ok()
        });
    match mapped_field {
        Some(field) => field.into_iter().map(|value| value as f32).collect(),
        None => flux
            .iter()
            .zip(active.canonical_operator.primary_mass())
            .map(|(flux, mass)| (f64::from(*flux) / mass) as f32)
            .collect(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn frame(
    mut contexts: EguiContexts,
    mut state: ResMut<Playground>,
    time: Res<Time>,
    mut recorders: ResMut<WaveGpuRequest>,
    mut request: ResMut<CanonicalGpuRequest>,
    mut canonical_display: ResMut<CanonicalGpuDisplay>,
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
        theme::apply(ctx);
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
        state.fit_inspector_to_screen(ctx.content_rect().width());
        state.open_startup_scene(
            crate::sharing::initial_fragment(),
            crate::recovery::load,
            crate::recovery::guide_seen(),
        );
    }
    state.editor.validate_frame(12000);
    canonical_display.release_pictures();
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
                    Err(error) => FileEvent::Error("Snapshot not exported", error),
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
    state.persist_guide_seen(crate::recovery::mark_guide_seen);
    Ok(())
}
