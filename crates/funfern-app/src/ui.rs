#![allow(clippy::collapsible_if)]

use crate::canonical_gpu::{
    CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest, CanonicalGpuTransferPlan,
};
use crate::files::FileEvent;
use crate::material_overlay::MaterialOverlay;
use crate::recording::{self, RecordingSpec};
use crate::wave_gpu::{
    AreaProbeDisplay, CurveProbeDisplay, FarFieldDisplay, ProbeDisplay, VectorOverlayDisplay,
    WaveDisplay, WaveGpuRequest,
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
    scene: &TopologyScene,
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
            let region = scene.region(triangle.region)?;
            let material = scene.material(region.material)?;
            let raw = material.evaluate(region.frame, centroid).ok()?;
            let coefficients = scene
                .physics
                .directional_wave_coefficients(raw, region.frame);
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
    display.current.extend(
        canonical
            .primary_flux
            .iter()
            .zip(operator.primary_mass())
            .map(|(flux, mass)| (f64::from(*flux) / mass) as f32),
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material_overlay::MaterialProperty;
    use crate::wave_gpu::MAX_STEPS_PER_FRAME;
    use funfern_app::topology_editor::{ClosedCurvePurpose, TopologyAcceptance, TopologyEditor};
    use funfern_app::topology_viewport::TopologyHandle;
    use funfern_app::topology_viewport::TopologyHit;
    use funfern_app::topology_viewport::TopologySpanTarget;
    use funfern_app::topology_viewport::screen_side;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_worker_returns_a_candidate_to_the_runtime_transaction() {
        let mut state = Playground::default();
        assert!(state.background_preparation.is_some());
        for _ in 0..100_000 {
            state.editor.validate_frame(64);
            if state.editor.acceptance != TopologyAcceptance::Pending {
                break;
            }
        }
        assert_eq!(state.editor.acceptance, TopologyAcceptance::Valid);
        state.request_runtime();
        assert!(state.runtime.phase().is_some());

        state.advance_runtime_preparation();
        assert!(state.runtime.phase().is_none());
        assert!(state.preparation_in_progress());

        let started = std::time::Instant::now();
        while state.runtime.ready().is_none()
            && state.runtime.last_error().is_none()
            && started.elapsed() < std::time::Duration::from_secs(5)
        {
            std::thread::yield_now();
            state.advance_runtime_preparation();
        }
        assert!(state.runtime.last_error().is_none());
        assert!(
            state.runtime.ready().is_some(),
            "native preparation timed out"
        );
        assert!(state.handoff_ready.is_some());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_amr_worker_finishes_a_mesh_transaction() {
        let scene = Scene::default();
        let mesh = Arc::new(
            mesh_scene(
                &scene,
                1,
                MeshingOptions {
                    target_edge_length: 0.2,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let job = MeshAdaptationJob::new(
            mesh.clone(),
            scene,
            MeshAdaptationState::from_mesh(&mesh),
            2,
            Arc::new(|_, _| 0.2),
            MeshAdaptationOptions {
                minimum_target_edge_length: 0.01,
                maximum_target_edge_length: 0.3,
                max_refinement_changes: 0,
                max_coarsening_changes: 0,
                ..Default::default()
            },
        );
        let mut worker = BackgroundAmrWorker::spawn().expect("native AMR worker");
        worker
            .submit(BackgroundAmrJob::Adaptation(Box::new(job)))
            .map_err(|_| ())
            .unwrap();

        let started = std::time::Instant::now();
        loop {
            for event in worker.drain() {
                if let BackgroundAmrEvent::Finished { result, .. } = event {
                    let BackgroundAmrResult::Adaptation(result) = *result else {
                        panic!("wrong AMR result kind");
                    };
                    let result = result.unwrap();
                    assert_eq!(result.report.topology_changes, 0);
                    return;
                }
            }
            assert!(
                started.elapsed() < std::time::Duration::from_secs(5),
                "native AMR worker timed out"
            );
            std::thread::yield_now();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_amr_worker_prepares_the_canonical_supplement() {
        let mut scene = Scene::initial();
        scene.outer_boundaries =
            OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
        let mesh = Arc::new(
            mesh_scene(
                &scene,
                1,
                MeshingOptions {
                    target_edge_length: 0.2,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let operator = Arc::new(
            QuadraticWaveOperator::assemble_scene(
                &mesh,
                &scene,
                OuterBoundaryCondition::Reflecting,
            )
            .unwrap(),
        );
        let canonical =
            Arc::new(CanonicalWaveOperator::compile_scene(&mesh, &operator, &scene, 1).unwrap());
        let forcing = Arc::new(CanonicalForcing::none(&canonical));
        let dofs = operator.degrees_of_freedom();
        let snapshot = QuadraticSolutionSnapshot {
            mesh_revision: mesh.mesh_revision,
            displacement: vec![0.0; dofs],
            velocity: vec![0.0; dofs],
            acceleration: vec![0.0; dofs],
            auxiliary: vec![0.0; dofs],
            volume_acceleration: vec![0.0; dofs],
            time: 0.01,
            time_step: 0.01,
        };
        let canonical_snapshot = CanonicalIndicatorSnapshot {
            mesh_revision: mesh.mesh_revision,
            primary_flux: vec![0.0; canonical.degrees_of_freedom()],
            previous_primary_flux: vec![0.0; canonical.degrees_of_freedom()],
            complementary_flux: vec![
                Point2::default();
                canonical.complementary_degrees_of_freedom()
            ],
            previous_complementary_flux: vec![
                Point2::default();
                canonical.complementary_degrees_of_freedom()
            ],
            auxiliary: Vec::new(),
            previous_auxiliary: Vec::new(),
            time: 0.01,
            time_step: 0.01,
        };
        let job = AmrIndicatorJob::with_canonical(
            SolutionIndicatorJob::new(
                mesh.clone(),
                operator,
                scene,
                snapshot,
                SolutionIndicatorOptions::default(),
            ),
            mesh,
            canonical,
            forcing,
            canonical_snapshot,
        );
        assert_eq!(job.phase(), "Preparing canonical AMR estimate");

        let mut worker = BackgroundAmrWorker::spawn().expect("native AMR worker");
        worker
            .submit(BackgroundAmrJob::Indicator(Box::new(job)))
            .map_err(|_| ())
            .unwrap();

        let started = std::time::Instant::now();
        loop {
            for event in worker.drain() {
                if let BackgroundAmrEvent::Finished { result, .. } = event {
                    let BackgroundAmrResult::Indicator(result) = *result else {
                        panic!("wrong AMR result kind");
                    };
                    let result = result.unwrap();
                    assert_eq!(result.report.canonical_drift_contribution, 0.0);
                    assert_eq!(result.report.complementary_recovery_contribution, 0.0);
                    return;
                }
            }
            assert!(
                started.elapsed() < std::time::Duration::from_secs(5),
                "native canonical AMR worker timed out"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn handoff_withholds_steps_only_when_the_source_cannot_advance() {
        assert!(!canonical_steps_withheld(false, false));
        // A packed candidate drains the requests already published before the
        // UI calls begin_handoff.
        assert!(canonical_steps_withheld(true, false));
        // A fresh install has no accepted source generation to advance.
        assert!(canonical_steps_withheld(false, true));
        // Ordinary target upload and validation are not solver pauses. Later
        // requests become a target catch-up backlog after admission.
        assert!(!canonical_steps_withheld(false, false));
    }

    /// The point of the whole mechanism: the catalog's quietest example and its
    /// loudest sit a hundredfold apart, and both must paint the same picture.
    #[test]
    fn an_exposure_paints_the_same_picture_at_any_field_scale() {
        let scale = 118.0;
        let mut levels = vec![0.0, 1.0e-3, 6.0e-3, 4.0e-3, 6.2e-3, 5.9e-3, 2.0e-2];
        // Then all the way down past the exposure's own floor, so a scale that
        // is relative to the field is told apart from one pinned to a constant.
        let mut decaying = 2.0e-2;
        for _ in 0..40 {
            decaying *= 0.5;
            levels.push(decaying);
        }
        let mut quiet = AutoExposure::default();
        let mut loud = AutoExposure::default();
        let mut floored = false;
        for level in levels {
            let (Some(a), Some(b)) = (quiet.update(level, 0.1), loud.update(level * scale, 0.1))
            else {
                assert_eq!(level, 0.0, "a measured level produced no reference");
                continue;
            };
            floored |= a > level * 2.0;
            let probe = level * 0.7;
            assert!(
                (probe / a - probe * scale / b).abs() < 1.0e-9 * (probe / a).max(1.0e-9),
                "{level}: {a} against {b}"
            );
            assert!(
                (quiet.visibility(level) - loud.visibility(level * scale)).abs() < 1.0e-12,
                "the quiet-tail fade changed with absolute field scale"
            );
        }
        assert!(floored, "the run never reached the quiet floor");
    }

    /// The failure the monotone run peak had: one placed pulse set the scale for
    /// the rest of the run.
    #[test]
    fn an_exposure_recovers_after_a_transient_spike() {
        let mut exposure = AutoExposure::default();
        for _ in 0..120 {
            exposure.update(1.0, 1.0 / 60.0);
        }
        assert_eq!(exposure.update(20.0, 1.0 / 60.0), Some(20.0), "clipped");
        // The release gives up a factor of 1.15 a second, so a twentyfold spike
        // takes some twenty-one seconds to walk off. Bounded is the property
        // that matters — the run peak it replaced never gave it up at all.
        for _ in 0..1_500 {
            exposure.update(1.0, 1.0 / 60.0);
        }
        assert_eq!(exposure.reference(), Some(1.0));
    }

    /// The failure this release rate was chosen for. A scale that falls faster
    /// than the field does simply follows it down, so a domain that has emptied
    /// still paints at full brightness and the wave looks like it never left.
    /// The shape here is the recorded one: full amplitude, then three decades
    /// over eight seconds once the sources stop.
    #[test]
    fn a_field_that_drains_away_stops_being_painted() {
        let mut exposure = AutoExposure::default();
        for _ in 0..600 {
            exposure.update(3.0e-2, 1.0 / 60.0);
        }
        let mut level = 3.0e-2;
        for _ in 0..480 {
            level *= 0.985_7;
            exposure.update(level, 1.0 / 60.0);
        }
        let reference = exposure.reference().unwrap();
        let painted = level / reference;
        assert!(
            level < 3.0e-5,
            "the fixture did not actually drain: {level:e}"
        );
        assert!(
            painted < 0.05,
            "a drained domain still paints at {:.1}%",
            painted * 100.0
        );
    }

    /// And the failure the other way: once a field has decayed into rounding
    /// noise, renormalizing it would fill the view with structure that is not
    /// there.
    #[test]
    fn an_exposure_refuses_to_magnify_decayed_noise() {
        let mut exposure = AutoExposure::default();
        exposure.update(1.0, 1.0 / 60.0);
        // Two decades down to the floor at 1.15 a second is about thirty-three.
        for _ in 0..3_600 {
            exposure.update(1.0e-9, 1.0 / 60.0);
        }
        let floor = exposure.reference().unwrap();
        assert!(
            (floor - AutoExposure::QUIET_FLOOR).abs() < 1.0e-12,
            "{floor}"
        );
        assert!(1.0e-9 / floor < 1.0e-5, "noise would still be drawn");
        assert!(
            exposure.visibility(1.0e-9) < 1.0e-10,
            "late residue was not faded out"
        );
    }

    #[test]
    fn exposure_fades_smoothly_below_its_run_relative_floor() {
        let mut exposure = AutoExposure::default();
        exposure.update(1.0, 0.0);
        assert_eq!(exposure.visibility(AutoExposure::QUIET_FLOOR), 1.0);
        let tenth = exposure.visibility(AutoExposure::QUIET_FLOOR * 0.1);
        assert!((0.0..1.0e-3).contains(&tenth), "weak fade {tenth}");
        assert_eq!(exposure.visibility(0.0), 0.0);
        assert_eq!(exposure.visibility(f64::NAN), 0.0);
    }

    #[test]
    fn measured_damped_tail_is_not_renormalized_into_a_field() {
        let mut exposure = AutoExposure::default();
        exposure.update(1.0, 0.0);
        // The saved source-free TM drain fixture settles near 0.2--0.3% of
        // its propagated peak. Let the release reach its run-relative floor,
        // then verify that tail still paints below two percent of full scale.
        let tail = 3.0e-3;
        for _ in 0..6_000 {
            exposure.update(tail, 1.0 / 60.0);
        }
        let painted = tail / exposure.reference().unwrap() * exposure.visibility(tail);
        assert!(painted < 0.02, "tail still paints at {painted:.3}");
        assert_eq!(
            DORMANT_ENERGY_RATIO,
            AutoExposure::QUIET_FLOOR * AutoExposure::QUIET_FLOOR
        );
    }

    #[test]
    fn arrow_ac_coupling_rejects_static_state_in_simulation_time() {
        let mut state = Playground::default();
        let sample = |value| {
            let value = Point2::new(value, 0.0);
            vec![(0, Pos2::ZERO, value, value)]
        };

        let mut first = sample(1.0);
        state.ac_couple_vector_samples(&mut first, 0, 0.0, false);
        assert_eq!(first[0].2.x, 0.0);

        let mut after_one_second = sample(1.0);
        state.ac_couple_vector_samples(&mut after_one_second, 100, 1.0, false);
        assert_eq!(after_one_second[0].2.x, 0.0);

        let mut after_two_seconds = sample(1.0);
        state.ac_couple_vector_samples(&mut after_two_seconds, 200, 2.0, false);
        assert_eq!(after_two_seconds[0].2.x, 0.0);
    }

    #[test]
    fn amr_generation_change_orphans_an_in_flight_arrow_lattice() {
        assert!(vector_overlay_revision_owned(7, 12, 7, 12, true));
        assert!(
            !vector_overlay_revision_owned(7, 12, 8, 12, true),
            "AMR generation incorrectly kept the stale zoom readback alive"
        );
        assert!(!vector_overlay_revision_owned(7, 12, 7, 13, true));
        assert!(!vector_overlay_revision_owned(7, 12, 7, 12, false));
    }

    #[test]
    fn resident_filter_boundaries_are_not_ordinary_time_endpoints() {
        assert!(!resident_filter_boundary(true, 0));
        assert!(!resident_filter_boundary(true, 15));
        assert!(resident_filter_boundary(true, 16));
        assert!(resident_filter_boundary(true, 32));
        assert!(!resident_filter_boundary(false, 16));
    }

    #[test]
    fn arrow_ac_coupling_does_not_turn_filter_maintenance_into_a_wave() {
        let mut state = Playground::default();
        let mut first = vec![(0, Pos2::ZERO, Point2::default(), Point2::default())];
        state.ac_couple_vector_samples(&mut first, 14, 0.14, false);

        let mut ordinary = vec![(0, Pos2::ZERO, Point2::new(0.2, 0.0), Point2::new(0.2, 0.0))];
        state.ac_couple_vector_samples(&mut ordinary, 15, 0.15, false);
        assert!((0.19..0.21).contains(&ordinary[0].2.x));

        // Deliberately exaggerated maintenance correction. Feeding the raw
        // input jump to the high-pass would produce an arrow near 9.0.
        let mut filtered = vec![(0, Pos2::ZERO, Point2::new(9.0, 0.0), Point2::new(0.2, 0.0))];
        state.ac_couple_vector_samples(&mut filtered, 16, 0.16, true);
        assert!((0.19..0.21).contains(&filtered[0].2.x));

        let mut after = vec![(0, Pos2::ZERO, Point2::new(9.1, 0.0), Point2::new(9.1, 0.0))];
        state.ac_couple_vector_samples(&mut after, 17, 0.17, false);
        assert!((0.28..0.31).contains(&after[0].2.x));
    }

    #[test]
    fn arrow_ac_coupling_keeps_evolution_before_filter_maintenance() {
        let mut state = Playground::default();
        let mut first = vec![(0, Pos2::ZERO, Point2::new(0.2, 0.0), Point2::new(0.2, 0.0))];
        state.ac_couple_vector_samples(&mut first, 15, 0.15, false);

        // The ordinary endpoint advanced to 0.5 before an intentionally huge
        // same-time maintenance correction moved the accepted state to 9.0.
        // The wave increment must survive while the correction itself does not.
        let mut filtered = vec![(0, Pos2::ZERO, Point2::new(9.0, 0.0), Point2::new(0.5, 0.0))];
        state.ac_couple_vector_samples(&mut filtered, 16, 0.16, true);
        assert!((0.29..0.31).contains(&filtered[0].2.x));
    }

    #[test]
    fn arrow_ac_coupling_tracks_physical_samples_and_cold_starts_new_ones() {
        let mut state = Playground::default();
        let mut first = vec![(7, Pos2::ZERO, Point2::new(1.0, 0.0), Point2::new(1.0, 0.0))];
        state.ac_couple_vector_samples(&mut first, 0, 0.0, false);
        assert_eq!(first[0].2, Point2::default());

        // The same mesh element carries temporal history even if its screen
        // cell changes. A genuinely new element does not inherit that history
        // or flash its unknown baseline into the AC view.
        let mut moved = vec![
            (
                7,
                Pos2::new(80.0, 40.0),
                Point2::new(1.5, 0.0),
                Point2::new(1.5, 0.0),
            ),
            (11, Pos2::ZERO, Point2::new(9.0, 0.0), Point2::new(9.0, 0.0)),
        ];
        state.ac_couple_vector_samples(&mut moved, 1, 0.01, false);
        assert!(moved[0].2.x > 0.49, "lost physical-sample history");
        assert_eq!(moved[1].2, Point2::default(), "new sample flashed DC");

        // A lazily retained element uses its own last accepted step when it
        // returns to view; time spent off-screen still decays its baseline.
        let mut elsewhere = vec![(11, Pos2::ZERO, Point2::new(9.0, 0.0), Point2::new(9.0, 0.0))];
        state.ac_couple_vector_samples(&mut elsewhere, 100, 1.0, false);
        let mut returned = vec![(7, Pos2::ZERO, Point2::new(1.5, 0.0), Point2::new(1.5, 0.0))];
        state.ac_couple_vector_samples(&mut returned, 101, 1.01, false);
        assert!(
            (0.29..0.31).contains(&returned[0].2.x),
            "off-screen time was lost: {}",
            returned[0].2.x
        );
    }

    #[test]
    fn arrow_ac_history_survives_a_compatible_generation_handoff() {
        let mut state = Playground::default();
        let owner = VectorOverlayAcOwner {
            mesh_revision: 17,
            physics: PhysicsModel::Mechanical,
        };
        state.retain_vector_overlay_ac_owner(owner, &[], 100, 2.0, 80.0);
        let mut first = vec![(3, Pos2::ZERO, Point2::new(1.0, 0.0), Point2::new(1.0, 0.0))];
        state.ac_couple_vector_samples(&mut first, 100, 2.0, false);
        let mut changing = vec![(3, Pos2::ZERO, Point2::new(1.4, 0.0), Point2::new(1.4, 0.0))];
        state.ac_couple_vector_samples(&mut changing, 110, 2.1, false);
        assert!(changing[0].2.x > 0.39);

        // A GPU generation is deliberately absent from the owner. Rebinding
        // material-dependent stencils on the same mesh therefore retains the
        // temporal baseline and continues at the transferred absolute time.
        state.retain_vector_overlay_ac_owner(owner, &[], 120, 2.2, 80.0);
        let mut after_handoff = vec![(3, Pos2::ZERO, Point2::new(1.5, 0.0), Point2::new(1.5, 0.0))];
        state.ac_couple_vector_samples(&mut after_handoff, 120, 2.2, false);
        assert!(after_handoff[0].2.x > 0.45, "handoff cold-started arrows");

        state.retain_vector_overlay_ac_owner(
            VectorOverlayAcOwner {
                mesh_revision: 18,
                ..owner
            },
            &[],
            120,
            2.2,
            80.0,
        );
        let mut after_remesh = vec![(3, Pos2::ZERO, Point2::new(1.5, 0.0), Point2::new(1.5, 0.0))];
        state.ac_couple_vector_samples(&mut after_remesh, 120, 2.2, false);
        assert_eq!(after_remesh[0].2, Point2::default());
    }

    #[test]
    fn arrow_ac_history_is_spatially_rebased_across_a_remesh() {
        let mut state = Playground::default();
        let old_owner = VectorOverlayAcOwner {
            mesh_revision: 17,
            physics: PhysicsModel::Mechanical,
        };
        state.retain_vector_overlay_ac_owner(old_owner, &[], 100, 2.0, 80.0);
        let mut first = vec![(
            3,
            Pos2::new(40.0, 50.0),
            Point2::new(1.0, 0.0),
            Point2::new(1.0, 0.0),
        )];
        state.ac_couple_vector_samples(&mut first, 100, 2.0, false);
        let mut changing = vec![(
            3,
            Pos2::new(40.0, 50.0),
            Point2::new(1.4, 0.0),
            Point2::new(1.4, 0.0),
        )];
        state.ac_couple_vector_samples(&mut changing, 110, 2.1, false);
        let before = changing[0].2;

        let new_samples = vec![(
            91,
            Pos2::new(43.0, 48.0),
            Point2::new(1.45, 0.0),
            Point2::new(1.45, 0.0),
        )];
        state.retain_vector_overlay_ac_owner(
            VectorOverlayAcOwner {
                mesh_revision: 18,
                ..old_owner
            },
            &new_samples,
            110,
            2.1,
            80.0,
        );
        let mut accepted = new_samples;
        state.ac_couple_vector_samples(&mut accepted, 110, 2.1, false);
        assert_eq!(accepted[0].2, before, "remesh blinked the AC arrows");
        assert_eq!(state.vector_overlay_ac_state[&91].input.x, 1.45);
    }

    #[test]
    fn sparse_arrow_outliers_cannot_defeat_the_global_quiet_fade() {
        let maximum = 55.2;
        let visibility = VECTOR_OVERLAY_VISIBILITY_CUTOFF;
        let ordinary = vector_arrow_length(1.0, 1.0, 1.0, maximum, visibility);
        let outlier = vector_arrow_length(1.0e12, 1.0, 5.0, maximum, visibility);
        assert!((ordinary - outlier).abs() < f32::EPSILON);
        assert!(outlier < 0.11, "quiet outlier still spans {outlier} px");
    }

    #[test]
    fn arrow_ac_coupling_preserves_an_ordinary_source_frequency() {
        let mut state = Playground {
            uploaded_time_step: 1.0 / 600.0,
            ..Playground::default()
        };
        let frequency = 3.0;
        let mut input_square = 0.0;
        let mut output_square = 0.0;
        // The solver advances ten small steps between display-rate samples.
        for frame in 0..600_u64 {
            let step = frame * 10;
            let time = step as f64 * state.uploaded_time_step;
            let scalar = (std::f64::consts::TAU * frequency * time).sin();
            let value = Point2::new(scalar, 0.0);
            let mut samples = vec![(0, Pos2::ZERO, value, value)];
            state.ac_couple_vector_samples(&mut samples, step, time, false);
            if frame >= 300 {
                input_square += scalar * scalar;
                output_square += samples[0].2.x * samples[0].2.x;
            }
        }
        let retained = (output_square / input_square).sqrt();
        assert!(retained > 0.999, "3 Hz amplitude retention {retained}");
    }

    #[test]
    fn arrow_ac_coupling_does_not_leave_a_mean_on_a_dc_offset_sine() {
        let mut state = Playground::default();
        let sample_rate = 60.0;
        let frequency = 2.3;
        let mut mean = 0.0;
        let mut count = 0_u64;
        for frame in 0..1_800_u64 {
            let time = frame as f64 / sample_rate;
            let scalar = 4.0 + (std::f64::consts::TAU * frequency * time).sin();
            let value = Point2::new(scalar, 0.0);
            let mut samples = vec![(0, Pos2::ZERO, value, value)];
            state.ac_couple_vector_samples(&mut samples, frame, time, false);
            if time >= 20.0 {
                mean += samples[0].2.x;
                count += 1;
            }
        }
        mean /= count as f64;
        assert!(mean.abs() < 1.0e-4, "high-pass mean was {mean}");
    }

    /// Release is a rate in seconds, so the same second of wall clock has to
    /// land in the same place whether it took two frames or two hundred.
    #[test]
    fn an_exposure_releases_by_wall_clock_not_by_frame_count() {
        let mut coarse = AutoExposure::default();
        let mut fine = AutoExposure::default();
        coarse.update(10.0, 0.016);
        fine.update(10.0, 0.016);
        coarse.update(1.0, 0.5);
        coarse.update(1.0, 0.5);
        for _ in 0..100 {
            fine.update(1.0, 0.01);
        }
        // A relative tolerance: the two differ only in how the same decay was
        // recomposed in floating point.
        let (a, b) = (coarse.reference().unwrap(), fine.reference().unwrap());
        assert!((a - b).abs() < a * 1.0e-6, "{a} against {b}");
    }

    #[test]
    fn a_sampled_quantile_matches_the_sorted_one() {
        let mut scratch = Vec::new();
        let values = (0..50_000)
            .map(|index| index as f32 / 50_000.0)
            .collect::<Vec<_>>();
        let level = exposure_level(&values, 0.98, &mut scratch);
        assert!((level - 0.98).abs() < 0.01, "{level}");
        assert_eq!(exposure_level(&[0.0; 32], 0.98, &mut scratch), 0.0);
        assert_eq!(exposure_level(&[], 0.98, &mut scratch), 0.0);
        let broken = [f32::NAN, f32::INFINITY, -3.0, 1.0];
        assert_eq!(exposure_level(&broken, 0.5, &mut scratch), 3.0);
    }

    /// The default gain has to land the reference level somewhere legible, and
    /// leave the nodes above it room to read brighter still.
    #[test]
    fn the_default_gain_paints_the_reference_level_in_the_readable_band() {
        let default = funfern_app::document::PresentationSettings::default().field_gain;
        let scale = default * FIELD_EXPOSURE_GAIN;
        let base = field_color(0.0, Color32::TRANSPARENT);
        let at_reference = field_color(scale, Color32::TRANSPARENT);
        let above = field_color(2.0 * scale, Color32::TRANSPARENT);
        let reach = |color: Color32| f32::from(color.r() - base.r()) / f32::from(244 - base.r());
        assert!(
            (0.7..0.85).contains(&reach(at_reference)),
            "{}",
            reach(at_reference)
        );
        assert!(reach(above) > reach(at_reference) + 0.1, "no room above");
        assert_eq!(
            field_color_over_overlay(scale).a(),
            (0.761_594_f32 * 220.0).round() as u8
        );
    }

    /// Below a ceiling of `recommended / PACING_FRAME_SECONDS`, pacing by step
    /// count alone leaves whole frames without one. The step shrinks there
    /// instead — always downward, since the mesh's figure is a stability limit.
    #[test]
    fn a_low_ceiling_shrinks_the_step_rather_than_skipping_frames() {
        // The GRIN rod's step at the default mesh, whose threshold is most of
        // the slider.
        let recommended = 6.6e-3;
        let threshold = recommended / PACING_FRAME_SECONDS;
        assert!(
            (0.7..0.85).contains(&threshold),
            "threshold moved: {threshold}"
        );

        // At and above it the mesh keeps its own step and the count does the work.
        assert_eq!(paced_time_step(recommended, 1.0), recommended);
        assert_eq!(paced_time_step(recommended, 2.0), recommended);
        assert!(
            paced_time_step(recommended, 1.0e6) <= recommended,
            "went above the limit"
        );

        // Below it the step follows the ceiling down.
        assert_eq!(
            paced_time_step(recommended, 0.1),
            PACING_FRAME_SECONDS * 0.1
        );
        assert_eq!(
            paced_time_step(recommended, 0.02),
            PACING_FRAME_SECONDS * 0.02
        );

        // Nonsense leaves the mesh's own step alone.
        assert_eq!(paced_time_step(recommended, 0.0), recommended);
        assert_eq!(paced_time_step(recommended, f64::NAN), recommended);
        assert_eq!(paced_time_step(recommended, -1.0), recommended);
    }

    /// What the cap is for: a frame's budget buys at least one step at every
    /// ceiling, on coarse meshes and fine. Without it a coarse mesh leaves half
    /// the frames unadvanced below 0.8x and the picture judders.
    #[test]
    fn a_frame_advances_at_every_ceiling() {
        for recommended in [6.6e-3, 8.9e-4, 6.2e-4] {
            for speed in [2.0, 1.0, 0.5, 0.2, 0.05, 0.02] {
                let step = paced_time_step(recommended, speed);
                assert!(step <= recommended, "{recommended:e} {speed}");
                let mut accumulator = 0.0;
                let idle = (0..600)
                    .filter(|_| {
                        steps_for_frame(&mut accumulator, PACING_FRAME_SECONDS, speed, step) == 0
                    })
                    .count();
                assert_eq!(
                    idle, 0,
                    "recommended {recommended:e} at {speed}x left {idle} frames unadvanced"
                );
            }
        }
    }

    /// The ceiling is on simulated seconds per wall second, so half the speed
    /// asks for half the steps out of the same frame.
    #[test]
    fn speed_scales_the_steps_a_frame_asks_for() {
        let step = 1.0e-3;
        let frame = 16.0e-3;
        let mut full = 0.0;
        let mut half = 0.0;
        let mut quiet = 0.0;
        let (mut full_total, mut half_total, mut quiet_total) = (0, 0, 0);
        for _ in 0..60 {
            full_total += steps_for_frame(&mut full, frame, 1.0, step);
            half_total += steps_for_frame(&mut half, frame, 0.5, step);
            quiet_total += steps_for_frame(&mut quiet, frame, 0.02, step);
        }
        // A second of frames at one millisecond a step.
        assert_eq!(full_total, 960);
        assert_eq!(half_total, 480);
        assert_eq!(quiet_total, 19);
    }

    /// A late display frame drops missed wall time instead of asking the GPU to
    /// catch up and making the next frame later still. Very high requested
    /// speeds retain the independent solver-batch ceiling.
    #[test]
    fn late_frames_preserve_the_display_budget_and_cap_the_leftover() {
        let step = 1.0e-3;
        let mut on_time = 0.0;
        let mut late = 0.0;
        let expected = steps_for_frame(&mut on_time, 1.0 / 60.0, 1.0, step);
        assert_eq!(steps_for_frame(&mut late, 1.0, 1.0, step), expected);
        assert!((late - on_time).abs() < 1.0e-12);

        let mut accumulator = 0.0;
        let steps = steps_for_frame(&mut accumulator, 1.0, 8.0, step);
        assert_eq!(steps, MAX_STEPS_PER_FRAME);
        assert!(
            accumulator <= MAX_STEPS_PER_FRAME as f64 * step + 1.0e-12,
            "the leftover built a backlog: {accumulator}"
        );
        // And it stays capped however long the solver is behind.
        for _ in 0..100 {
            steps_for_frame(&mut accumulator, 1.0, 8.0, step);
        }
        assert!(accumulator <= MAX_STEPS_PER_FRAME as f64 * step + 1.0e-12);

        // Nonsense asks for nothing rather than panicking or racing.
        let mut idle = 0.0;
        assert_eq!(steps_for_frame(&mut idle, 0.016, 1.0, 0.0), 0);
        assert_eq!(steps_for_frame(&mut idle, 0.016, 0.0, 1.0e-3), 0);
        assert_eq!(steps_for_frame(&mut idle, -1.0, 1.0, 1.0e-3), 0);
    }

    #[test]
    fn gpu_backpressure_drops_requests_beyond_the_completed_lead() {
        assert_eq!(steps_with_gpu_backpressure(100, 100, 12), 12);
        assert_eq!(
            steps_with_gpu_backpressure(100, 150, 20),
            MAX_STEPS_PER_FRAME - 50
        );
        assert_eq!(steps_with_gpu_backpressure(100, 164, 20), 0);
        assert_eq!(steps_with_gpu_backpressure(100, 200, 20), 0);
    }

    /// The windowed rate dips whenever a handoff withholds stepping inside its
    /// window, at every speed and including ones the solver reaches easily, so
    /// the note reads a held best rather than the raw measurement.
    #[test]
    fn a_held_rate_rides_over_a_dip_but_not_a_slowdown() {
        let frame = 1.0 / 60.0;
        let mut held = 0.0;
        for _ in 0..120 {
            held = hold_rate(held, 1.0, frame);
        }
        assert_eq!(held, 1.0);

        // A second of the worst dip measured still reads as keeping up.
        let mut dipped = held;
        for _ in 0..60 {
            dipped = hold_rate(dipped, 0.74, frame);
        }
        assert!(
            speed_shortfall(dipped, 1.0, true).is_none(),
            "a dip was reported as a shortfall: {dipped}"
        );

        // A real slowdown gets through inside two seconds.
        let mut slow = held;
        for _ in 0..120 {
            slow = hold_rate(slow, 0.5, frame);
        }
        assert_eq!(speed_shortfall(slow, 1.0, true), Some(slow));

        // A better reading is taken at once, and nonsense is ignored.
        assert_eq!(hold_rate(0.5, 2.0, frame), 2.0);
        assert_eq!(hold_rate(0.5, f64::NAN, frame), 0.5);
        assert_eq!(hold_rate(0.5, -1.0, frame), 0.5);
    }

    /// A rate below the one asked for is worth saying, but only when the solver
    /// is actually trying to reach it.
    #[test]
    fn a_shortfall_is_only_reported_while_the_solver_is_trying() {
        assert_eq!(speed_shortfall(0.34, 1.0, true), Some(0.34));
        assert_eq!(
            speed_shortfall(0.98, 1.0, true),
            None,
            "jitter is not a shortfall"
        );
        assert_eq!(speed_shortfall(0.19, 0.2, true), None);
        assert_eq!(speed_shortfall(0.09, 0.2, true), Some(0.09));
        // Paused, mid-handoff, or before anything has been measured.
        assert_eq!(speed_shortfall(0.34, 1.0, false), None);
        assert_eq!(speed_shortfall(0.0, 1.0, true), None);
        assert_eq!(speed_shortfall(f64::NAN, 1.0, true), None);
    }

    /// Adaptation hands the field to a new mesh every second or two. It is the
    /// same field, so its scale has to carry across: restarting it there dropped
    /// the reference onto the instantaneous level, and a decaying field fell in
    /// visible steps instead of easing down at the release rate.
    #[test]
    fn a_mesh_handoff_leaves_the_scale_alone() {
        let mut state = Playground::default();
        state.field_exposure.update(0.71, 0.016);
        state.vector_overlay_exposure.update(0.71, 0.016);
        state.vector_overlay_ac_owner = Some(VectorOverlayAcOwner {
            mesh_revision: 3,
            physics: PhysicsModel::Mechanical,
        });
        state.vector_overlay_ac_state.insert(
            4,
            VectorAcState {
                input: Point2::new(1.0, 0.0),
                output: Point2::new(0.2, 0.0),
                step: 10,
                time: 0.1,
                origin: Pos2::new(20.0, 30.0),
            },
        );
        state.restart_exposures_after_handoff(false);
        assert_eq!(state.field_exposure.reference(), Some(0.71));
        assert_eq!(state.vector_overlay_exposure.reference(), Some(0.71));
        assert_eq!(state.vector_overlay_ac_state.len(), 1);

        // A field replaced with zeros starts the scale again.
        state.restart_exposures_after_handoff(true);
        assert_eq!(state.field_exposure.reference(), None);
        assert_eq!(state.vector_overlay_exposure.reference(), None);
        assert!(state.vector_overlay_ac_state.is_empty());
        assert_eq!(state.vector_overlay_ac_owner, None);
    }

    /// Loading a document and pressing Reset both leave the scale alone. Each
    /// zeroes the field on the GPU, but the outgoing scene stays on display
    /// until the replacement arrives — a reset for a few frames, a load for as
    /// long as the new mesh takes. A scale cleared at the request measures that
    /// residue and paints it at full brightness: measured at 0.09 % before, 100 %
    /// after, for a tenth of a second on a reset and four tenths on a load.
    #[test]
    fn asking_for_a_new_field_does_not_magnify_the_outgoing_one() {
        let mut state = Playground::default();
        state.field_exposure.update(3.0e-2, 0.016);
        let residue = 3.8e-6;
        let held = state.field_exposure.update(residue, 0.016).unwrap();
        assert!(residue / held < 0.01, "the residue was not already dark");

        state.reset_requested = true;
        let document = state.editor.document.clone();
        state.set_document(document, false, true).unwrap();
        let after = state.field_exposure.update(residue, 0.016).unwrap();
        assert!(
            residue / after < 0.01,
            "the outgoing field was magnified to {:.0}%",
            100.0 * residue / after
        );
    }

    /// The first frames of a replaced field are numerical dust, and an instant
    /// attack onto a scale with nothing behind it paints that dust at full
    /// colour. Measured at 3.5e-10 arriving one frame before the real field.
    #[test]
    fn a_restarted_scale_does_not_latch_onto_the_first_dust() {
        let mut exposure = AutoExposure::default();
        exposure.update(3.0e-2, 0.016);
        exposure.restart();
        let dust = 3.5e-10;
        let reference = exposure.update(dust, 0.016).unwrap();
        assert!(
            dust / reference < 1.0e-4,
            "dust painted at {:.0}%",
            100.0 * dust / reference
        );
        // The real field, when it arrives, takes the scale straight over.
        assert_eq!(exposure.update(2.0e-2, 0.016), Some(2.0e-2));
    }

    /// The symptom the handoff bug showed as: a scale that falls faster than the
    /// release allows. Nothing `update` does may outrun that rate.
    #[test]
    fn the_scale_never_falls_faster_than_the_release_rate() {
        let mut exposure = AutoExposure::default();
        let mut level = 1.0_f64;
        let step = 1.0_f32 / 60.0;
        let mut previous = exposure.update(level, step).unwrap();
        for _ in 0..1_200 {
            level *= 0.98;
            let reference = exposure.update(level, step).unwrap();
            // The same widening `update` does, so the two agree to the bit.
            let allowed = previous * AutoExposure::RELEASE_PER_SECOND.powf(-f64::from(step));
            assert!(
                reference >= allowed * (1.0 - 1.0e-12),
                "the scale fell to {reference:e} when {allowed:e} was the floor"
            );
            previous = reference;
        }
    }

    /// Turning the automatic scale off has to put the field back exactly where it
    /// was before there was one: the slider as the whole scale.
    #[test]
    fn turning_auto_exposure_off_restores_the_plain_gain() {
        let default = funfern_app::document::PresentationSettings::default();
        assert!(default.field_auto_exposure, "it should start on");
        assert_eq!(field_scale(2.0, Some(0.02), false), 2.0);
        assert_eq!(field_scale(0.25, None, false), 0.25);
        // Automatic, the same field paints the same whatever its size.
        let quiet = field_scale(2.0, Some(6.0e-3), true) * 6.0e-3;
        let loud = field_scale(2.0, Some(7.1e-1), true) * 7.1e-1;
        assert!((quiet - loud).abs() < 1.0e-12);
        assert_eq!(field_scale(2.0, None, true), 0.0);
    }

    /// Loading a document used to raise `reset_requested`, which the GPU reset
    /// spends earlier in the frame and against the topology still active — the
    /// scene on its way out. That scene was zeroed and then ran on for the
    /// seconds its replacement took to prepare, so by the time the new mesh was
    /// ready the transfer carried a full-amplitude field into it. Only the
    /// oscillation radiated away; the constant it left behind is in the
    /// stiffness operator's null space and no outgoing wall can remove it, so
    /// opening the Luneburg lens and then anything else washed the new scene
    /// flat.
    #[test]
    fn loading_a_document_asks_for_a_field_that_starts_at_zero() {
        let mut state = Playground::default();
        let example = &funfern_app::topology_examples::catalog()[0];
        state
            .set_document(example.document.clone(), false, true)
            .unwrap();
        assert!(
            state.fresh_requested,
            "the load did not ask to start at zero"
        );
        assert!(
            !state.reset_requested,
            "the load armed the flag the GPU reset spends against the outgoing scene"
        );
        // The shape of the bug: a topology is already active and the GPU reset
        // has taken its flag, and the load must still start the field at zero.
        assert!(starts_from_zero(true, false, true));
        assert!(starts_from_zero(true, true, false));
        assert!(starts_from_zero(false, false, false));
        assert!(!starts_from_zero(true, false, false));
    }

    /// A thumbnail is rasterized, not traced, so a face covers area rather than
    /// only an outline, and every quad lands inside the scene it came from.
    #[test]
    fn a_thumbnail_fills_the_faces_of_the_scene_it_previews() {
        let example = &funfern_app::topology_examples::catalog()[0];
        let preview = build_example_preview(example);
        let domain = example.document.model.accepted.geometry.domain;
        assert!(preview.quads.len() > PREVIEW_ROWS, "a face was not filled");
        assert!(!preview.strokes.is_empty(), "nothing was outlined");
        for quad in &preview.quads {
            assert!(quad.low.x < quad.high.x && quad.low.y < quad.high.y);
            assert!(
                quad.low.x >= domain.min_x - 1e-9
                    && quad.high.x <= domain.max_x + 1e-9
                    && quad.low.y >= domain.min_y - 1e-9
                    && quad.high.y <= domain.max_y + 1e-9,
                "a quad left the domain"
            );
        }
        let colors = preview
            .quads
            .iter()
            .map(|quad| quad.color.to_array())
            .collect::<BTreeSet<_>>();
        assert!(
            colors.len() > 1,
            "the obstacle is the same colour as the background it sits in"
        );
    }

    /// The reason a thumbnail samples cell centres rather than polygon corners:
    /// every corner of a Luneburg lens sits on the same circle, so a profile
    /// read at the corners alone is one flat colour.
    #[test]
    fn a_radial_material_profile_reaches_the_thumbnail() {
        let example = funfern_app::topology_examples::catalog()
            .iter()
            .find(|example| example.name == "Luneburg lens")
            .expect("the catalog still carries the Luneburg lens");
        assert!(matches!(
            example.document.presentation.material_overlay,
            MaterialOverlay::Property(MaterialProperty::WaveSpeed)
        ));
        let preview = build_example_preview(example);
        let shades = preview
            .quads
            .iter()
            .map(|quad| quad.color.to_array())
            .collect::<BTreeSet<_>>();
        assert!(
            shades.len() > 8,
            "the lens reads as {} colour(s), not a profile",
            shades.len()
        );
    }

    /// Even-odd pairing across every cycle at once is what keeps a face out of
    /// its own holes, and a slit traced out and back contributes nothing.
    #[test]
    fn scanline_spans_skip_the_holes_in_a_face() {
        let outer = vec![
            Point2::new(-1.0, -1.0),
            Point2::new(1.0, -1.0),
            Point2::new(1.0, 1.0),
            Point2::new(-1.0, 1.0),
        ];
        let hole = vec![
            Point2::new(-0.5, -0.5),
            Point2::new(-0.5, 0.5),
            Point2::new(0.5, 0.5),
            Point2::new(0.5, -0.5),
        ];
        assert_eq!(
            face_spans(std::slice::from_ref(&outer), 0.0),
            vec![(-1.0, 1.0)]
        );
        assert_eq!(
            face_spans(&[outer.clone(), hole], 0.0),
            vec![(-1.0, -0.5), (0.5, 1.0)]
        );
        // A line above the face crosses nothing.
        assert!(face_spans(&[outer], 2.0).is_empty());
    }

    /// New is a scene that can be run, not just an empty one.
    #[test]
    fn a_new_scene_is_empty_outgoing_and_driven() {
        let mut state = Playground::default();
        state.new_scene();
        let scene = &state.editor.document.model.accepted;
        assert!(scene.geometry.curves.is_empty());
        assert_eq!(scene.regions.len(), 1);
        for side in OuterSide::ALL {
            assert_eq!(
                scene.outer_boundaries.sides[side.index()],
                OuterBoundaryCondition::SecondOrderOutgoing
            );
        }
        assert!(state.editor.document.model.source.enabled);
        assert_eq!(state.example_opened, None, "a new scene is not an example");
        assert!(state.editor.undo(), "New is one undoable action");
    }

    /// The gallery survives a pick, and the pick is what changes the document.
    #[test]
    fn opening_an_example_leaves_the_gallery_open_and_marks_the_row() {
        let mut state = Playground {
            examples_open: true,
            ..Playground::default()
        };
        let index = funfern_app::topology_examples::catalog()
            .iter()
            .position(|example| example.name == "Double slit")
            .unwrap();
        state.open_example(index);
        assert!(state.examples_open, "the gallery closed on a pick");
        assert_eq!(state.example_opened, Some(index));
        assert_eq!(
            state.editor.document.model,
            funfern_app::topology_examples::catalog()[index]
                .document
                .model
        );
        state.new_scene();
        assert_eq!(state.example_opened, None, "the marker outlived its scene");
    }

    /// Every catalog entry has to reach the gallery, and building them all is
    /// spread over frames because the largest one costs a frame by itself.
    #[test]
    fn every_example_previews_and_the_cache_fills_one_per_frame() {
        let mut state = Playground::default();
        let total = funfern_app::topology_examples::catalog().len();
        assert_eq!(state.example_previews.len(), total);
        assert!(state.example_previews.iter().all(Option::is_none));
        for filled in 1..=total {
            let index = state
                .example_previews
                .iter()
                .position(Option::is_none)
                .expect("an unbuilt preview");
            state.example_previews[index] = Some(build_example_preview(
                &funfern_app::topology_examples::catalog()[index],
            ));
            assert_eq!(
                state
                    .example_previews
                    .iter()
                    .filter(|p| p.is_some())
                    .count(),
                filled
            );
        }
        for (index, preview) in state.example_previews.iter().enumerate() {
            let preview = preview.as_ref().unwrap();
            assert!(
                !preview.quads.is_empty(),
                "{} previews as nothing",
                funfern_app::topology_examples::catalog()[index].name
            );
        }
    }

    /// The random start has to land on every example and never off the end.
    #[test]
    fn the_random_start_stays_inside_the_catalog() {
        let total = funfern_app::topology_examples::catalog().len();
        let mut seen = BTreeSet::new();
        for step in 0..=1000 {
            let index = random_example_index(f64::from(step) / 1000.0, total);
            assert!(index < total, "{step} lands past the catalog");
            seen.insert(index);
        }
        assert_eq!(seen.len(), total, "some example can never open at startup");
        assert!((0.0..1.0).contains(&random_fraction()));
    }

    #[test]
    fn a_span_selection_reports_one_state_only_when_every_span_agrees() {
        let mut editor = TopologyEditor::default();
        editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(0.0, 0.0), 0.3),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        let spans = editor.document.model.draft.geometry.curves[0]
            .spans
            .iter()
            .map(|span| span.id)
            .collect::<BTreeSet<_>>();
        assert!(spans.len() > 1);
        let first = BTreeSet::from([*spans.first().unwrap()]);
        let state = |editor: &TopologyEditor, spans: &BTreeSet<CurveSpanId>| {
            span_behavior_state(&editor.document.model.draft.geometry, spans)
        };

        // A closed subdomain's spans start out transmitting.
        assert_eq!(state(&editor, &spans), Some(SpanBehaviorState::Transmit));

        editor
            .set_span_behavior(&first, SpanBehavior::REFLECTING)
            .unwrap();
        assert_eq!(state(&editor, &spans), None);
        assert_eq!(state(&editor, &first), Some(SpanBehaviorState::Boundary));

        // A boundary stays a boundary whatever its faces carry, which is what
        // the old pair of buttons could not say.
        editor
            .set_span_face_condition(
                &first,
                CurveTraceSide::Left,
                FaceBoundaryCondition::Impedance { ratio: 2.0 },
            )
            .unwrap();
        assert_eq!(state(&editor, &first), Some(SpanBehaviorState::Boundary));

        editor
            .set_span_behavior(&spans, SpanBehavior::REFLECTING)
            .unwrap();
        assert_eq!(state(&editor, &spans), Some(SpanBehaviorState::Boundary));
    }

    #[test]
    fn the_side_band_falls_where_a_click_reads_the_same_side() {
        // A span drawn left to right on screen runs along +x in the world, so
        // its left side is up the screen.
        assert_eq!(
            side_offset(egui::vec2(2.0, 0.0), CurveTraceSide::Left),
            egui::vec2(0.0, -1.0)
        );
        for (x, y) in [
            (1.0, 0.0),
            (0.0, 1.0),
            (-1.0, 0.0),
            (0.0, -1.0),
            (3.0, -2.0),
            (-1.5, -4.0),
        ] {
            let a = ScreenPoint::new(0.0, 0.0);
            let b = ScreenPoint::new(f64::from(x), f64::from(y));
            for side in [CurveTraceSide::Left, CurveTraceSide::Right] {
                let offset = side_offset(egui::vec2(x, y), side) * 3.0;
                let point = ScreenPoint::new(f64::from(offset.x), f64::from(offset.y));
                assert_eq!(
                    screen_side(a, b, point),
                    side,
                    "a band on the {side:?} of ({x}, {y}) reads back as the other side"
                );
            }
        }
        assert_eq!(
            side_offset(egui::Vec2::ZERO, CurveTraceSide::Left),
            egui::Vec2::ZERO
        );
    }

    #[test]
    fn the_formula_reference_names_what_the_parser_accepts() {
        let origin = MaterialCoordinates {
            x: 0.0,
            y: 0.0,
            r: 0.0,
            theta: 0.0,
        };
        for (names, _) in FORMULA_SYMBOLS {
            for name in names.split(", ") {
                // An unknown name parses as a material parameter and only fails
                // once it is evaluated without one, so the evaluation is what
                // proves the reference still names something built in.
                let field = ScalarField::formula(name).expect("a listed symbol parses");
                assert!(
                    field.evaluate(origin, &[]).is_ok(),
                    "the reference lists `{name}`, which the parser does not know"
                );
            }
        }
        for (signature, _, example) in FORMULA_FUNCTIONS {
            let field = ScalarField::formula(example)
                .unwrap_or_else(|error| panic!("{signature}: `{example}` is rejected: {error}"));
            assert!(
                field.evaluate(origin, &[]).is_ok(),
                "{signature}: `{example}` does not evaluate"
            );
        }
        for retired in ["ln(1 + r)", "pow(r, 2)"] {
            assert!(
                ScalarField::formula(retired).is_err(),
                "`{retired}` parses, so the reference should be listing it"
            );
        }
    }

    /// Every channel the log watches overwrites itself, so a change is the only
    /// moment its value can be caught. Holding a value logs it once, clearing
    /// and returning without anything in between counts a repeat rather than
    /// filling the ring, and an error keeps the status marker lit until the
    /// window is opened on it.
    #[test]
    fn the_log_keeps_one_entry_per_change_of_a_transient_channel() {
        let mut state = Playground::default();
        state.record_events(1.0);
        assert!(state.events.is_empty());
        assert!(!state.diagnostics_warning());

        state.message = "Pulse queued in region 1".into();
        state.record_events(2.0);
        state.record_events(3.0);
        assert_eq!(state.events.len(), 1, "a held value is logged once");
        assert_eq!(state.events[0].source, EventSource::Status);
        assert_eq!(state.events[0].repeats, 1);
        assert!(
            !state.diagnostics_warning(),
            "a status line is not an error"
        );

        state.amr_error = Some("Invalid adaptation source".into());
        state.record_events(4.0);
        assert_eq!(state.events.len(), 2);
        assert_eq!(state.events[1].source, EventSource::Adaptation);
        assert!(state.unseen_error);

        // The adaptation clears itself and fails again with nothing logged in
        // between, which is the shape that would otherwise flood the ring.
        for time in [5.0, 6.0, 7.0, 8.0] {
            state.amr_error = (time as u64)
                .is_multiple_of(2)
                .then(|| "Invalid adaptation source".to_owned());
            state.record_events(time);
        }
        assert_eq!(state.events.len(), 2, "{:?}", state.events);
        assert_eq!(state.events[1].repeats, 3);

        // A different message is its own entry, and the marker survives the
        // error clearing.
        state.amr_error = None;
        state.message = "Simulation topology committed".into();
        state.record_events(9.0);
        assert_eq!(state.events.len(), 3);
        assert_eq!(state.events[2].source, EventSource::Status);
        assert!(
            state.diagnostics_warning(),
            "the marker stays lit for an error the window has not been opened on"
        );

        state.unseen_error = false;
        assert!(!state.diagnostics_warning());
        assert_eq!(
            event_line(&state.events[1]),
            "0:08.0 · adaptation · Invalid adaptation source ×3"
        );

        // A repair fallback is queued by the transaction that reported it and
        // stamped on the next frame, so two transactions that fall back the
        // same way are counted rather than reading as one.
        state.pending_repairs = vec!["mesh repair failed".into(), "mesh repair failed".into()];
        state.record_events(10.0);
        assert!(state.pending_repairs.is_empty());
        assert_eq!(state.events.len(), 4);
        assert_eq!(state.events[3].source, EventSource::Repair);
        assert_eq!(state.events[3].repeats, 2);
        assert!(
            !state.diagnostics_warning(),
            "a rebuild that carried the edit through is not an error"
        );
    }

    /// The picker lists one set of names and the two condition enums answer
    /// with their own, so a condition used to rename itself the moment it was
    /// chosen. They are held to the same words here, in both directions: every
    /// kind is reachable and every condition reads back as the kind that lists
    /// it.
    #[test]
    fn boundary_names_agree_across_every_source() {
        let all = [
            BoundaryKind::Reflecting,
            BoundaryKind::FirstOrder,
            BoundaryKind::SecondOrder,
            BoundaryKind::ElectricWall,
            BoundaryKind::MagneticWall,
            BoundaryKind::Neumann,
            BoundaryKind::Dirichlet,
        ];
        let signal = TimeSignal::harmonic(0.0, 1.0, 1.0, 0.0);
        let faces = [
            FaceBoundaryCondition::Reflecting,
            FaceBoundaryCondition::Impedance { ratio: 1.0 },
            FaceBoundaryCondition::SecondOrderOutgoing,
            FaceBoundaryCondition::ElectricWall,
            FaceBoundaryCondition::MagneticWall,
            FaceBoundaryCondition::Neumann { signal },
            FaceBoundaryCondition::Dirichlet { signal },
        ];
        for condition in faces {
            assert_eq!(
                face_kind(condition).label(),
                condition.label(),
                "{condition:?} is listed under another name"
            );
        }
        let outers = [
            OuterBoundaryCondition::Reflecting,
            OuterBoundaryCondition::FirstOrderOutgoing,
            OuterBoundaryCondition::SecondOrderOutgoing,
            OuterBoundaryCondition::ElectricWall,
            OuterBoundaryCondition::MagneticWall,
            OuterBoundaryCondition::Neumann { signal },
            OuterBoundaryCondition::Dirichlet { signal },
        ];
        for condition in outers {
            assert_eq!(
                outer_kind(condition).label(),
                condition.label(),
                "{condition:?} is listed under another name"
            );
        }
        // Each list covers every kind the picker offers, so nothing is
        // unreachable and no two kinds share a name.
        for kinds in [
            faces.map(face_kind).to_vec(),
            outers.map(outer_kind).to_vec(),
        ] {
            assert_eq!(kinds, all.to_vec());
        }
        assert_eq!(
            all.iter()
                .map(|kind| kind.label())
                .collect::<BTreeSet<_>>()
                .len(),
            all.len()
        );
    }

    #[test]
    fn boundary_picker_uses_the_active_skins_physical_vocabulary() {
        let mechanical = BoundaryKind::choices(PhysicsModel::Mechanical);
        assert!(mechanical.contains(&BoundaryKind::Reflecting));
        assert!(!mechanical.contains(&BoundaryKind::ElectricWall));
        assert!(!mechanical.contains(&BoundaryKind::MagneticWall));
        assert_eq!(
            BoundaryKind::Reflecting.label_for(PhysicsModel::Mechanical),
            "Free boundary"
        );

        let tm = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        };
        let te = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        };
        for physics in [tm, te] {
            let choices = BoundaryKind::choices(physics);
            assert!(!choices.contains(&BoundaryKind::Reflecting));
            assert!(choices.contains(&BoundaryKind::ElectricWall));
            assert!(choices.contains(&BoundaryKind::MagneticWall));
        }
        assert_eq!(
            BoundaryKind::Reflecting.presented(tm),
            BoundaryKind::MagneticWall,
            "the natural TM scalar boundary is a magnetic wall"
        );
        assert_eq!(
            BoundaryKind::Reflecting.presented(te),
            BoundaryKind::ElectricWall,
            "the natural TE scalar boundary is an electric wall"
        );
        assert_eq!(
            BoundaryKind::ElectricWall.presented(PhysicsModel::Mechanical),
            BoundaryKind::Dirichlet
        );
        assert_eq!(
            BoundaryKind::MagneticWall.presented(PhysicsModel::Mechanical),
            BoundaryKind::Reflecting
        );
    }

    #[test]
    fn material_editor_names_the_active_physical_coefficients() {
        assert_eq!(
            material_editor_labels(PhysicsModel::Mechanical),
            MaterialEditorLabels {
                mass: "Density ρ₀",
                stiffness: "Stiffness k₀",
                damping: "Damping σ",
                axis_ratio: "Stiffness axis ratio",
            }
        );
        for polarization in [
            ElectromagneticPolarization::Tm,
            ElectromagneticPolarization::Te,
        ] {
            assert_eq!(
                material_editor_labels(PhysicsModel::Electromagnetic { polarization }),
                MaterialEditorLabels {
                    mass: "Permittivity ε",
                    stiffness: "Permeability μ",
                    damping: "Loss rate α",
                    axis_ratio: "Constitutive axis ratio",
                }
            );
        }
    }

    /// Both axes step upwards from the lower bound. The vertical one used to
    /// start at the top of the view and test against the bottom, so the grid
    /// had only ever been columns.
    #[test]
    fn the_grid_covers_both_axes_of_the_view() {
        // An 800x600 view at 300 pixels per world unit, centred on the origin.
        let state = Playground {
            scale: 300.0,
            center: Point2::default(),
            ..Playground::default()
        };
        let view = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let (step, fine) = grid_steps(state.scale);
        assert_eq!(step, 0.5, "70 px at this zoom lands on the half-unit");
        assert_eq!(fine, 0.1, "which divides into five");
        let (columns, rows) = state.grid_axes(view, step);
        assert_eq!(columns, vec![-1.5, -1.0, -0.5, 0.0, 0.5, 1.0]);
        assert_eq!(rows, vec![-1.0, -0.5, 0.0, 0.5, 1.0]);
        assert!(
            rows.iter().any(|y| y.abs() < step * 0.1),
            "the horizontal axis is among them, and is the emphasised line"
        );
        // The fine lattice nests inside the drawn one, so every line that is
        // drawn is one Shift can land on.
        assert!(
            (step / fine - 5.0).abs() < 1.0e-9,
            "a half-unit divides in five"
        );
        let (fine_columns, fine_rows) = state.grid_axes(view, fine);
        assert!(fine_columns.len() >= columns.len() * 4);
        assert!(fine_rows.len() >= rows.len() * 4);

        // A bound the wrong way round draws nothing rather than looping.
        assert!(grid_lines(1.0, -1.0, step).is_empty());
        assert!(grid_lines(-1.0, 1.0, 0.0).is_empty());
        assert!(grid_lines(f64::NAN, 1.0, step).is_empty());
        // A degenerate zoom cannot hang the painter.
        assert!(grid_lines(-1.0, 1.0, grid_steps(0.0).0).is_empty());
        assert!(grid_lines(-1.0e9, 1.0e9, 1.0e-9).len() <= 4096);

        // The spacing holds its decade: about 70 pixels apart at any zoom, and
        // the step Shift lands on is always a round division of it.
        for scale in [12.0, 37.0, 300.0, 1_500.0, 9_000.0] {
            let (step, fine) = grid_steps(scale);
            let pixels = step * scale;
            assert!(
                (35.0..=180.0).contains(&pixels),
                "{scale} pixels per unit put lines {pixels} apart"
            );
            let divisions = step / fine;
            assert!(
                (divisions - divisions.round()).abs() < 1.0e-9 && (4.0..=5.0).contains(&divisions),
                "{scale} divides {step} into {divisions}"
            );
        }
    }

    /// Shift places a point on the same 0.05 grid every drag snaps to. A
    /// default editor has compiled nothing, so no attachment can outrank it and
    /// this is the grid path.
    #[test]
    fn shift_places_a_drawn_point_on_the_grid() {
        let viewport = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        // The grid belongs to the zoom, so the zoom is named: 400 pixels per
        // unit divides fifths of a unit in four, landing on 0.05.
        let mut state = Playground {
            scale: 400.0,
            ..Playground::default()
        };
        state.begin_draw(DrawTool::Polyline);
        state.draw_click(
            Point2::new(0.117, -0.233),
            ScreenPoint::new(0.0, 0.0),
            viewport,
            true,
        );
        state.draw_click(
            Point2::new(-0.481, 0.062),
            ScreenPoint::new(0.0, 0.0),
            viewport,
            false,
        );
        let points = &state.draw.as_ref().unwrap().points;
        assert_eq!(points[0], Point2::new(0.10, -0.25));
        assert_eq!(
            points[1],
            Point2::new(-0.481, 0.062),
            "without shift the point stays where it was put"
        );

        // Every tool goes through the same place, the two-click rectangle
        // included. Clear of the default scene's loop, so it compiles.
        let before = state.editor.document.model.draft.geometry.curves.len();
        state.begin_draw(DrawTool::Rectangle);
        for point in [Point2::new(0.537, 0.562), Point2::new(0.873, 0.818)] {
            state.draw_click(point, ScreenPoint::new(0.0, 0.0), viewport, true);
        }
        assert_eq!(
            state.editor.document.model.draft.geometry.curves.len(),
            before + 1,
            "the rectangle was refused: {}",
            state.message
        );
        let rectangle = state
            .editor
            .document
            .model
            .draft
            .geometry
            .curves
            .last()
            .expect("the rectangle was created")
            .spline
            .clone();
        assert_eq!(rectangle.node_count(), 4);
        for index in 0..rectangle.node_count() {
            let corner = rectangle.node_point(index).unwrap();
            for value in [corner.x, corner.y] {
                let steps = value / 0.05;
                assert!(
                    (steps - steps.round()).abs() < 1.0e-9,
                    "corner off the grid at {value}"
                );
            }
        }
    }

    /// Picking a tool starts the gesture and leaves the palette up, so one
    /// primitive can follow another without a trip back to the toolbar.
    #[test]
    fn the_draw_palette_outlives_the_gesture_it_starts() {
        let mut state = Playground {
            draw_open: true,
            ..Playground::default()
        };
        state.begin_draw(DrawTool::Circle);
        assert!(state.draw.is_some());
        assert!(state.draw_open, "the palette closes only when it is closed");
        state.begin_draw(DrawTool::Polyline);
        assert!(state.draw_open);
    }

    /// Handover generations preserve the accepted-step total. Their first
    /// observation establishes a baseline instead of re-crediting the run;
    /// genuinely new steps on either side remain in the same rate window.
    #[test]
    fn the_step_rate_survives_a_handover() {
        let mut state = Playground::default();
        // Four frames of a settled generation, then the window closes.
        state.accumulate_step_rate(7, 0, 0.0);
        for (frame, completed) in [(1, 300_u64), (2, 600), (3, 900), (4, 1200)] {
            state.accumulate_step_rate(7, completed, if frame == 4 { 0.5 } else { 0.1 });
        }
        assert_eq!(state.steps_per_second, 2400.0);
        assert_eq!(state.rate_window_steps, 0, "a closed window starts empty");

        // A handover mid-window preserves its total, and the steps already
        // banked stay without the preserved total being counted a second time.
        state.accumulate_step_rate(7, 1500, 0.1);
        assert_eq!(state.rate_window_steps, 300);
        state.accumulate_step_rate(8, 1500, 0.1);
        assert_eq!(
            state.rate_window_steps, 300,
            "the preserved total is a baseline and the old progress is kept"
        );
        state.accumulate_step_rate(8, 1700, 0.5);
        assert_eq!(state.steps_per_second, 1000.0);

        // A window holding only the frames either side of another handover.
        state.accumulate_step_rate(8, 5000, 0.1);
        state.accumulate_step_rate(9, 5000, 0.5);
        assert_eq!(state.steps_per_second, 3300.0 / 0.5);
        assert!(
            state.steps_per_second > 0.0,
            "a handover is not a stall in the solver"
        );

        // A fresh install may reset the counter; its first observation is also
        // only a baseline, and subsequent progress is measured normally.
        state.accumulate_step_rate(10, 0, 0.1);
        state.accumulate_step_rate(10, 250, 0.5);
        assert_eq!(state.steps_per_second, 500.0);
    }

    /// The ring drops its oldest rather than growing without bound.
    #[test]
    fn the_log_is_bounded() {
        let mut state = Playground::default();
        for index in 0..EVENT_LOG_ENTRIES + 20 {
            state.message = format!("message {index}");
            state.record_events(index as f64);
        }
        assert_eq!(state.events.len(), EVENT_LOG_ENTRIES);
        assert_eq!(state.events[0].text, format!("message {}", 20));
        assert_eq!(
            state.events[EVENT_LOG_ENTRIES - 1].text,
            format!("message {}", EVENT_LOG_ENTRIES + 19)
        );
    }

    #[test]
    fn marquee_operation_and_direction_are_live_conventions() {
        assert_eq!(
            Playground::marquee_operation(egui::Modifiers::NONE),
            MarqueeOperation::Replace
        );
        assert_eq!(
            Playground::marquee_operation(egui::Modifiers::SHIFT),
            MarqueeOperation::Add
        );
        assert_eq!(
            Playground::marquee_operation(egui::Modifiers::ALT),
            MarqueeOperation::Subtract
        );
        assert_eq!(
            MarqueeContainment::from_drag(Pos2::ZERO, Pos2::new(10.0, 4.0)),
            MarqueeContainment::Enclosed
        );
        assert_eq!(
            MarqueeContainment::from_drag(Pos2::ZERO, Pos2::new(-10.0, 4.0)),
            MarqueeContainment::Crossing
        );
    }

    #[test]
    fn marquee_add_and_subtract_apply_against_the_drag_baseline() {
        let a = TopologySpanTarget::Curve(CurveSpanId(1));
        let b = TopologySpanTarget::Curve(CurveSpanId(2));
        let base = BTreeSet::from([a]);
        let hits = BTreeSet::from([b]);
        assert_eq!(
            Playground::marquee_result(&base, hits.clone(), MarqueeOperation::Replace),
            BTreeSet::from([b])
        );
        assert_eq!(
            Playground::marquee_result(&base, hits.clone(), MarqueeOperation::Add),
            BTreeSet::from([a, b])
        );
        assert_eq!(
            Playground::marquee_result(&BTreeSet::from([a, b]), hits, MarqueeOperation::Subtract,),
            BTreeSet::from([a])
        );
    }

    #[test]
    fn dragging_a_selected_span_preserves_the_complete_selection() {
        let a = TopologySpanTarget::Curve(CurveSpanId(1));
        let b = TopologySpanTarget::Curve(CurveSpanId(2));
        let selection = TopologySelection::Spans(BTreeSet::from([a, b]));

        assert!(Playground::drag_starts_inside_span_selection(
            &selection,
            TopologyHit::Span {
                target: a,
                distance: 0.0,
            },
        ));
        assert!(!Playground::drag_starts_inside_span_selection(
            &selection,
            TopologyHit::Span {
                target: TopologySpanTarget::Curve(CurveSpanId(3)),
                distance: 0.0,
            },
        ));
        assert!(!Playground::drag_starts_inside_span_selection(
            &selection,
            TopologyHit::Handle {
                handle: TopologyHandle::Control {
                    curve: CurveId(1),
                    control: 0,
                },
                distance: 0.0,
            },
        ));
    }
}

#[cfg(test)]
mod probe_interaction_tests {
    use super::*;
    use funfern_app::document::VectorOverlay;
    use funfern_app::topology_editor::TopologyProbeTarget;
    use funfern_app::topology_editor::{
        ClosedCurvePurpose, TopologyAcceptance, TopologyBoundaryProbeTarget, TopologyEditor,
    };
    use funfern_app::topology_viewport::TopologySpanTarget;

    #[test]
    fn vector_overlay_layout_is_world_anchored_and_pan_stable() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        let active = activate_at(&mut state, 0.08);
        let viewport = viewport();
        let spacing = 28.0;
        let (world_spacing, visible_bins) =
            vector_overlay_lattice(state.scale, spacing, state.center, viewport).unwrap();
        let points = vector_overlay_layout(
            &active.bundle.authored,
            &active.mesh,
            &active.operator,
            world_spacing,
            visible_bins,
        );
        assert!(!points.is_empty());
        let keys = points
            .iter()
            .map(|point| {
                (
                    (point.point.x / world_spacing).floor() as i64,
                    (point.point.y / world_spacing).floor() as i64,
                )
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(keys.len(), points.len());
        let maximum_bins = (visible_bins[1] - visible_bins[0] + 1) as usize
            * (visible_bins[3] - visible_bins[2] + 1) as usize;
        assert!(points.len() <= maximum_bins);
        assert!(points.iter().all(|point| {
            point.point.x.is_finite()
                && point.point.y.is_finite()
                && point.stencil.element < active.mesh.triangles.len() as u32
        }));

        let shifted_center = state.center + Point2::new(world_spacing * 0.35, 0.0);
        let (shifted_spacing, shifted_bins) =
            vector_overlay_lattice(state.scale, spacing, shifted_center, viewport).unwrap();
        assert_eq!(shifted_spacing, world_spacing);
        let shifted = vector_overlay_layout(
            &active.bundle.authored,
            &active.mesh,
            &active.operator,
            shifted_spacing,
            shifted_bins,
        );
        let common = [
            visible_bins[0].max(shifted_bins[0]) + 1,
            visible_bins[1].min(shifted_bins[1]) - 1,
            visible_bins[2].max(shifted_bins[2]) + 1,
            visible_bins[3].min(shifted_bins[3]) - 1,
        ];
        let interior = |points: &[VectorOverlayLayoutPoint]| {
            points
                .iter()
                .filter_map(|point| {
                    let key = (
                        (point.point.x / world_spacing).floor() as i64,
                        (point.point.y / world_spacing).floor() as i64,
                    );
                    (key.0 >= common[0]
                        && key.0 <= common[1]
                        && key.1 >= common[2]
                        && key.1 <= common[3])
                        .then_some((key, point.element))
                })
                .collect::<BTreeMap<_, _>>()
        };
        let before = interior(&points);
        assert!(!before.is_empty());
        assert_eq!(before, interior(&shifted));
    }

    #[test]
    fn polyline_midpoint_splits_the_path_by_arclength() {
        let path = [
            Point2::new(0.0, 0.0),
            Point2::new(2.0, 0.0),
            Point2::new(2.0, 2.0),
        ];
        let midpoint = Playground::polyline_midpoint(&path).unwrap();
        assert!((midpoint - Point2::new(2.0, 0.0)).norm() < 1.0e-12);
        assert_eq!(
            Playground::polyline_midpoint(&[Point2::new(1.0, 3.0)]),
            Some(Point2::new(1.0, 3.0))
        );
        assert_eq!(Playground::polyline_midpoint(&[]), None);
    }

    #[test]
    fn polyline_anchor_carries_the_segment_it_landed_on() {
        let path = [
            Point2::new(0.0, 0.0),
            Point2::new(2.0, 0.0),
            Point2::new(2.0, 2.0),
        ];
        let (point, segment) = Playground::polyline_anchor(&path, 0.25).unwrap();
        assert!((point - Point2::new(1.0, 0.0)).norm() < 1.0e-12);
        assert_eq!(segment, [path[0], path[1]]);
        let (point, segment) = Playground::polyline_anchor(&path, 0.75).unwrap();
        assert!((point - Point2::new(2.0, 1.0)).norm() < 1.0e-12);
        assert_eq!(segment, [path[1], path[2]]);
        assert_eq!(
            Playground::polyline_anchor(&[Point2::new(1.0, 3.0)], 0.5),
            None
        );
    }

    /// The arrow is drawn from the trace the probe reads into its badge, so it
    /// has to run out of that trace. `Left` names the face on the left of
    /// increasing parameter, so on a path running east the left trace is the
    /// northern one and an arrow arriving from it points south.
    #[test]
    fn a_boundary_probes_arrow_leaves_the_trace_it_reads() {
        let path = [Point2::new(-1.0, 0.0), Point2::new(1.0, 0.0)];
        let outward = |side, reversed| boundary_probe_orientation(&path, side, reversed).unwrap().1;
        assert!((outward(CurveTraceSide::Left, false) - Point2::new(0.0, -1.0)).norm() < 1.0e-12);
        assert!((outward(CurveTraceSide::Right, false) - Point2::new(0.0, 1.0)).norm() < 1.0e-12);
        // Reversing turns the arclength axis, never the side that is read.
        assert!((outward(CurveTraceSide::Left, true) - Point2::new(0.0, -1.0)).norm() < 1.0e-12);
        assert!((outward(CurveTraceSide::Right, true) - Point2::new(0.0, 1.0)).norm() < 1.0e-12);
    }

    /// The two arrows share a corner, so reversing has to turn one of them and
    /// leave the other where it is: the arclength axis runs the other way, the
    /// side that is read does not change, and the corner stays put.
    #[test]
    fn reversing_a_boundary_probe_turns_its_arclength_arrow_alone() {
        let path = [
            Point2::new(-1.0, 0.0),
            Point2::new(0.0, 0.0),
            Point2::new(0.0, 2.0),
        ];
        let orientation =
            |reversed| boundary_probe_orientation(&path, CurveTraceSide::Left, reversed).unwrap();
        let (forward_point, forward_outward, forward_along) = orientation(false);
        let (back_point, back_outward, back_along) = orientation(true);
        assert!((forward_point - Point2::new(0.0, 0.5)).norm() < 1.0e-12);
        assert!((back_point - forward_point).norm() < 1.0e-12);
        assert!((forward_along - Point2::new(0.0, 1.0)).norm() < 1.0e-12);
        assert!((back_along - Point2::new(0.0, -1.0)).norm() < 1.0e-12);
        assert!((forward_outward - Point2::new(1.0, 0.0)).norm() < 1.0e-12);
        assert!((back_outward - forward_outward).norm() < 1.0e-12);
    }

    #[test]
    fn a_boundary_path_too_short_to_orient_draws_no_marks() {
        for path in [vec![], vec![Point2::new(0.0, 0.0)]] {
            assert!(boundary_probe_orientation(&path, CurveTraceSide::Left, false).is_none());
        }
        let stationary = [Point2::new(1.0, 1.0), Point2::new(1.0, 1.0)];
        assert!(boundary_probe_orientation(&stationary, CurveTraceSide::Left, false).is_none());
    }

    /// Every probe kind must resolve a badge point, otherwise it draws no label.
    #[test]
    fn every_probe_kind_resolves_a_label_anchor() {
        let mut state = Playground::default();
        let curve = state
            .editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(0.0, 0.0), 0.4),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        for _ in 0..100_000 {
            state.editor.validate_frame(256);
            if state.editor.acceptance != TopologyAcceptance::Pending {
                break;
            }
        }
        assert_eq!(state.editor.acceptance, TopologyAcceptance::Valid);
        let spans = state
            .editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|candidate| candidate.id == curve)
            .unwrap()
            .spans
            .iter()
            .map(|span| span.id)
            .collect::<Vec<_>>();
        state
            .editor
            .create_probe(
                "Edge".into(),
                [248, 196, 112],
                TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
                    curve,
                    spans,
                    side: CurveTraceSide::Left,
                    reversed: false,
                    preset: ProbeSamplingPreset::Medium,
                }),
            )
            .unwrap();
        state
            .editor
            .create_probe(
                "Line".into(),
                [91, 220, 194],
                TopologyProbeTarget::Segment {
                    start: Point2::new(-0.8, -0.2),
                    end: Point2::new(0.8, 0.2),
                    preset: ProbeSamplingPreset::Medium,
                },
            )
            .unwrap();
        state
            .editor
            .create_probe(
                "Spot".into(),
                [91, 220, 194],
                TopologyProbeTarget::Point(Point2::new(0.6, 0.6)),
            )
            .unwrap();
        state.refresh_samples(viewport());
        assert!(state.sampled.is_some());

        for probe in state.editor.document.model.probes.clone() {
            let badge = state
                .probe_badge(&probe)
                .unwrap_or_else(|| panic!("{} has no label anchor", probe.name));
            assert!(
                badge.finite(),
                "{} anchored at a non-finite point",
                probe.name
            );
            if matches!(probe.target, TopologyProbeTarget::Boundary(_)) {
                assert!(
                    (badge.norm() - 0.4).abs() < 0.05,
                    "boundary badge left its curve: {badge:?}"
                );
            }
        }
    }

    /// A click on a probe must select it without starting a drag, and the badge
    /// of a boundary probe must be grabbable at its drawn position.
    #[test]
    fn probe_hit_testing_covers_every_kind() {
        let mut state = Playground::default();
        state
            .editor
            .create_probe(
                "Spot".into(),
                [91, 220, 194],
                TopologyProbeTarget::Point(Point2::new(0.2, 0.1)),
            )
            .unwrap();
        state
            .editor
            .create_probe(
                "Line".into(),
                [91, 220, 194],
                TopologyProbeTarget::Segment {
                    start: Point2::new(-0.6, -0.3),
                    end: Point2::new(0.6, -0.3),
                    preset: ProbeSamplingPreset::Medium,
                },
            )
            .unwrap();
        state
            .editor
            .create_probe(
                "Disk".into(),
                [91, 220, 194],
                TopologyProbeTarget::AreaDisk {
                    center: Point2::new(-0.5, 0.5),
                    radius: 0.2,
                },
            )
            .unwrap();
        state.refresh_samples(viewport());
        let id_of = |state: &Playground, name: &str| {
            state
                .editor
                .document
                .model
                .probes
                .iter()
                .find(|probe| probe.name == name)
                .unwrap()
                .id
        };
        let point = id_of(&state, "Spot");
        let line = id_of(&state, "Line");
        let disk = id_of(&state, "Disk");

        let at = |state: &Playground, world: Point2| state.screen(world, viewport());
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(0.2, 0.1)), viewport()),
            Some(ProbeHit::Point(point))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(-0.6, -0.3)), viewport()),
            Some(ProbeHit::SegmentEndpoint(line, true))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(0.6, -0.3)), viewport()),
            Some(ProbeHit::SegmentEndpoint(line, false))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(0.0, -0.3)), viewport()),
            Some(ProbeHit::SegmentBody(line))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(-0.3, 0.5)), viewport()),
            Some(ProbeHit::AreaDiskRadius(disk))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(-0.5, 0.5)), viewport()),
            Some(ProbeHit::AreaDiskBody(disk))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(0.9, 0.9)), viewport()),
            None
        );
    }

    /// The frame gizmo must be reachable wherever the numeric placement controls
    /// are, otherwise a region-local profile can only be aligned by typing.
    /// Deleting a selection is one gesture, so it is one undo step however many
    /// curves it covers.
    #[test]
    fn deleting_several_curves_is_one_history_entry() {
        // The default playground opens an example, which this test is not about.
        let mut state = Playground {
            editor: funfern_app::topology_editor::TopologyEditor::default(),
            ..Playground::default()
        };
        let settle = |state: &mut Playground| {
            for _ in 0..100_000 {
                state.editor.validate_frame(4096);
                if state.editor.acceptance != TopologyAcceptance::Pending {
                    return;
                }
            }
            panic!("validation did not terminate")
        };
        settle(&mut state);
        let mut curves = vec![];
        for (index, y) in [0.3f64, 0.0, -0.3].into_iter().enumerate() {
            let x = -0.4 + 0.1 * index as f64;
            curves.push(
                state
                    .editor
                    .create_boundary_baffle(
                        OpenCubicSpline::polyline(vec![
                            Point2::new(x, y),
                            Point2::new(x + 0.2, y + 0.08),
                            Point2::new(x + 0.4, y),
                        ])
                        .unwrap(),
                    )
                    .unwrap(),
            );
            settle(&mut state);
        }
        assert_eq!(state.editor.acceptance, TopologyAcceptance::Valid);
        let before = state.editor.history_len().0;
        state.selection = TopologySelection::Spans(
            state
                .editor
                .document
                .model
                .draft
                .geometry
                .curves
                .iter()
                .flat_map(|curve| curve.spans.iter())
                .map(|span| TopologySpanTarget::Curve(span.id))
                .collect(),
        );

        state.delete_selection();
        settle(&mut state);
        assert_eq!(state.editor.acceptance, TopologyAcceptance::Valid);
        assert!(
            state.editor.document.model.draft.geometry.curves.is_empty(),
            "every selected curve went"
        );
        assert_eq!(
            state.editor.history_len().0,
            before + 1,
            "three curves, one entry"
        );

        assert!(state.editor.undo());
        settle(&mut state);
        assert_eq!(
            state.editor.document.model.draft.geometry.curves.len(),
            curves.len(),
            "one undo brings all three back"
        );
    }

    /// A capture crops to the viewport, so what has to be suppressed is the
    /// chrome inside the crop, not the panels outside it.
    #[test]
    fn a_capture_hides_the_chrome_inside_its_crop() {
        let mut state = Playground::default();
        assert!(!state.capturing(), "idle is not a capture");
        for snapshot in [SnapshotState::Armed, SnapshotState::Capturing] {
            state.snapshot_state = snapshot;
            assert!(state.capturing(), "{snapshot:?}");
        }
        state.snapshot_state = SnapshotState::Saving;
        assert!(
            !state.capturing(),
            "saving happens after the pixels are taken"
        );
        state.snapshot_state = SnapshotState::Idle;
        for recording in [
            RecordingState::Preparing,
            RecordingState::Starting,
            RecordingState::Recording,
        ] {
            state.recording_state = recording;
            assert!(state.capturing(), "{recording:?}");
        }
        state.recording_state = RecordingState::SelectingDestination;
        assert!(
            !state.capturing(),
            "choosing a destination is not yet a capture"
        );

        // Selection emphasis is the one thing that has to read through the
        // predicate rather than be gated at a call site.
        state.recording_state = RecordingState::Recording;
        state.selection = TopologySelection::Spans(
            [TopologySpanTarget::Outer(OuterSide::Bottom)]
                .into_iter()
                .collect(),
        );
        assert!(
            !state.span_selected(TopologySpanTarget::Outer(OuterSide::Bottom)),
            "a selected span must not read as selected in a capture"
        );
        state.recording_state = RecordingState::Idle;
        assert!(state.span_selected(TopologySpanTarget::Outer(OuterSide::Bottom)));
    }

    #[test]
    fn material_frame_gizmo_appears_with_its_numeric_controls() {
        let mut state = Playground::default();
        assert_eq!(state.selected_material_frame(), None, "hidden by default");

        let material = state.editor.add_material().unwrap();
        let mut updated = state
            .editor
            .document
            .model
            .draft
            .material(material)
            .unwrap()
            .clone();
        updated.stiffness = ScalarField::formula("1 + 0.2 * x").unwrap();
        state.editor.update_material(updated).unwrap();
        state
            .editor
            .set_region_material(BACKGROUND_REGION, material)
            .unwrap();

        state.inspector = Some(InspectorPanel::Edit);
        assert_eq!(
            state.selected_material_frame(),
            None,
            "the gizmo belongs to the Materials panel"
        );
        state.inspector = Some(InspectorPanel::Materials);
        state.region_selection = BACKGROUND_REGION;
        let (region, frame) = state
            .selected_material_frame()
            .expect("a frame-using material must expose its gizmo");
        assert_eq!(region, BACKGROUND_REGION);

        let centre = state.screen(frame.origin, viewport());
        assert_eq!(
            state.hit_material_frame_gizmo(centre, viewport()),
            Some(MaterialFrameGizmoHit::Origin)
        );
        assert_eq!(
            state.hit_material_frame_gizmo(
                centre + egui::vec2(MATERIAL_FRAME_RADIUS, 0.0),
                viewport()
            ),
            Some(MaterialFrameGizmoHit::Rotate)
        );
        assert_eq!(
            state.hit_material_frame_gizmo(centre + egui::vec2(25.0, 0.0), viewport()),
            None
        );
    }

    /// The combo label and the drawn colour must name the same thing, and two
    /// subdomains sharing a material must still be told apart.
    /// Shift snaps what the drag moves, not the pointer, so where inside a probe
    /// it was picked up cannot leave it off the grid.
    /// The combo label and the drawn colour must name the same thing, and two
    /// subdomains sharing a material must still be told apart.
    #[test]
    fn subdomain_overlay_is_categorical_and_named_consistently() {
        assert_eq!(MaterialOverlay::Subdomains.label(), "Subdomains");
        assert_eq!(
            MaterialOverlay::Subdomains.label_for(PhysicsModel::Mechanical),
            MaterialOverlay::Subdomains.label()
        );
        assert_eq!(MaterialOverlay::Regions.label(), "Materials");

        let mut scene = TopologyScene::default();
        scene.regions.push(Region {
            id: RegionId(7),
            material: DEFAULT_MATERIAL,
            frame: MaterialFrame::world(),
        });
        scene.regions.push(Region {
            id: RegionId(8),
            material: DEFAULT_MATERIAL,
            frame: MaterialFrame::world(),
        });
        let background = subdomain_color(&scene, BACKGROUND_REGION, 1.0);
        let first = subdomain_color(&scene, RegionId(7), 1.0);
        let second = subdomain_color(&scene, RegionId(8), 1.0);
        assert_ne!(first, second, "equal materials must still read apart");
        assert_ne!(background, first);
        assert_eq!(
            subdomain_color(&scene, RegionId(99), 1.0),
            Color32::TRANSPARENT
        );
    }

    /// Placing a probe reads the same modifier dragging one already does, with
    /// the same conventions: a position lands on the grid, and a disk's radius
    /// is itself a multiple of it rather than the distance to a snapped rim.
    ///
    /// The grid is the zoom's, so the zoom is named. At 400 pixels per unit it
    /// draws fifths of a unit and divides them in four, which is the 0.05 this
    /// used to be fixed at.
    #[test]
    fn shift_places_a_probe_on_the_grid() {
        let step = grid_steps(400.0).1;
        assert!((step - 0.05).abs() < 1.0e-9, "the zoom moved: {step}");
        let on_grid = |value: f64| {
            let steps = value / step;
            (steps - steps.round()).abs() < 1.0e-9
        };
        let at = |point: Point2, x: f64, y: f64| {
            assert!(
                on_grid(point.x) && on_grid(point.y),
                "{point:?} is off the grid"
            );
            assert!(
                (point.x - x).abs() < 1.0e-9 && (point.y - y).abs() < 1.0e-9,
                "{point:?} is not the nearest node to ({x}, {y})"
            );
        };
        let mut state = Playground {
            probe_mode: Some(ProbePlacement::Point),
            scale: 400.0,
            ..Playground::default()
        };
        state.probe_placement_click(Point2::new(0.117, -0.233), true);
        let TopologyProbeTarget::Point(placed) =
            state.editor.document.model.probes.last().unwrap().target
        else {
            panic!("a point probe")
        };
        at(placed, 0.10, -0.25);

        state.probe_mode = Some(ProbePlacement::Segment { start: None });
        state.probe_placement_click(Point2::new(-0.312, 0.081), true);
        state.probe_placement_click(Point2::new(0.446, -0.377), true);
        let TopologyProbeTarget::Segment { start, end, .. } = state
            .editor
            .document
            .model
            .probes
            .last()
            .unwrap()
            .target
            .clone()
        else {
            panic!("a line probe")
        };
        at(start, -0.30, 0.10);
        at(end, 0.45, -0.40);

        // The rim click lands at a distance of 0.3111…, which is no multiple of
        // the grid; snapping the point rather than the radius would keep it.
        state.probe_mode = Some(ProbePlacement::Disk { center: None });
        state.probe_placement_click(Point2::new(0.019, -0.022), true);
        state.probe_placement_click(Point2::new(0.244, 0.193), true);
        let TopologyProbeTarget::AreaDisk { center, radius } =
            state.editor.document.model.probes.last().unwrap().target
        else {
            panic!("a disk probe")
        };
        at(center, 0.0, 0.0);
        assert!(on_grid(radius), "radius {radius} is off the grid");
        assert!(radius >= step);

        // Without the modifier nothing moves.
        state.probe_mode = Some(ProbePlacement::Point);
        state.probe_placement_click(Point2::new(0.117, -0.233), false);
        let TopologyProbeTarget::Point(loose) =
            state.editor.document.model.probes.last().unwrap().target
        else {
            panic!("a point probe")
        };
        assert_eq!(loose, Point2::new(0.117, -0.233));
    }

    #[test]
    fn shift_snaps_the_probe_rather_than_the_cursor() {
        let mut state = Playground::default();
        state
            .editor
            .create_probe(
                "Spot".into(),
                [91, 220, 194],
                TopologyProbeTarget::Point(Point2::new(0.0, 0.0)),
            )
            .unwrap();
        let probe = state.editor.document.model.probes.last().unwrap().clone();
        // Grabbed well off centre, then dragged to an arbitrary place.
        for grab_offset in [0.0, 0.017, -0.023] {
            let original = TopologyProbeTarget::Point(Point2::new(0.0, 0.0));
            let pointer = Point2::new(0.31 + grab_offset, -0.22 + grab_offset);
            let grab = Point2::new(grab_offset, grab_offset);
            state.drag_probe(ProbeHit::Point(probe.id), &original, pointer - grab, true);
            let TopologyProbeTarget::Point(position) =
                state.editor.document.model.probes.last().unwrap().target
            else {
                panic!("a point probe")
            };
            for value in [position.x, position.y] {
                let steps = value / 0.05;
                assert!(
                    (steps - steps.round()).abs() < 1.0e-9,
                    "grabbed at {grab_offset}, landed off the grid at {value}"
                );
            }
        }
        // A disk radius snaps too, rather than jumping by the grab offset.
        state
            .editor
            .create_probe(
                "Disk".into(),
                [91, 220, 194],
                TopologyProbeTarget::AreaDisk {
                    center: Point2::new(0.0, 0.0),
                    radius: 0.2,
                },
            )
            .unwrap();
        let disk = state.editor.document.model.probes.last().unwrap().clone();
        state.drag_probe(
            ProbeHit::AreaDiskRadius(disk.id),
            &disk.target,
            Point2::new(0.113, 0.0),
            true,
        );
        let TopologyProbeTarget::AreaDisk { radius, .. } =
            state.editor.document.model.probes.last().unwrap().target
        else {
            panic!("a disk probe")
        };
        let steps = radius / 0.05;
        assert!((steps - steps.round()).abs() < 1.0e-9, "radius {radius}");
    }

    /// A cursor appears for an affordance the drawing does not announce, or one
    /// whose direction matters, and stays away from drawn handles that move
    /// themselves.
    #[test]
    fn cursors_mark_the_affordances_the_drawing_does_not() {
        let mut state = Playground::default();
        let r = viewport();
        state.refresh_samples(r);
        let domain = state.editor.document.model.draft.geometry.domain;

        // Direction matters: the outer rectangle's corners and sides.
        let corner = state.screen(domain.corners()[0], r);
        assert_eq!(
            state.hover_cursor(corner, r),
            Some(egui::CursorIcon::ResizeNeSw)
        );
        let left_middle = state.screen(
            Point2::new(domain.min_x, (domain.min_y + domain.max_y) * 0.5),
            r,
        );
        assert_eq!(
            state.hover_cursor(left_middle, r),
            Some(egui::CursorIcon::ResizeHorizontal),
            "a bare outer side announces nothing on its own"
        );
        let bottom_middle = state.screen(
            Point2::new((domain.min_x + domain.max_x) * 0.5, domain.min_y),
            r,
        );
        assert_eq!(
            state.hover_cursor(bottom_middle, r),
            Some(egui::CursorIcon::ResizeVertical)
        );

        // Open space keeps the plain arrow.
        assert_eq!(
            state.hover_cursor(state.screen(Point2::default(), r), r),
            None
        );

        // A drawn handle that moves itself says enough by itself.
        let settle = |state: &mut Playground| {
            for _ in 0..100000 {
                state.editor.validate_frame(1000);
                if state.editor.acceptance != TopologyAcceptance::Pending {
                    return;
                }
            }
            panic!("validation did not terminate")
        };
        settle(&mut state);
        let curve = state
            .editor
            .create_boundary_baffle(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-0.3, 0.25),
                    Point2::new(0.0, 0.45),
                    Point2::new(0.3, 0.25),
                ])
                .unwrap(),
            )
            .unwrap();
        settle(&mut state);
        state.invalidate_samples();
        state.refresh_samples(r);
        let control = match &state
            .editor
            .document
            .model
            .draft
            .geometry
            .curve(curve)
            .unwrap()
            .spline
        {
            CurveSpline::Open(spline) => spline.controls()[0],
            CurveSpline::Closed(spline) => spline.controls()[0],
        };
        assert_eq!(
            state.hover_cursor(state.screen(control, r), r),
            None,
            "a control point is its own announcement"
        );

        // A selected span moves the whole selection, which nothing draws.
        let spans = state
            .editor
            .document
            .model
            .draft
            .geometry
            .curve(curve)
            .unwrap()
            .spans
            .iter()
            .map(|span| TopologySpanTarget::Curve(span.id))
            .collect::<BTreeSet<_>>();
        let midpoint = Point2::new(0.0, 0.3833);
        assert_eq!(
            state.hover_cursor(state.screen(midpoint, r), r),
            None,
            "an unselected span offers no move"
        );
        state.selection = TopologySelection::Spans(spans.clone());
        assert_eq!(
            state.hover_cursor(state.screen(midpoint, r), r),
            Some(egui::CursorIcon::Move)
        );
        // Widen the selection to include an outer side, which cannot move.
        let mut blocked = spans;
        blocked.insert(TopologySpanTarget::Outer(OuterSide::Bottom));
        state.selection = TopologySelection::Spans(blocked);
        assert_eq!(
            state.hover_cursor(state.screen(midpoint, r), r),
            None,
            "the cursor is absent exactly when the drag would do nothing"
        );
    }

    /// A mode that owns the viewport says so, since no handle can.
    #[test]
    fn modal_cursors_say_what_a_click_would_do() {
        let mut state = Playground::default();
        let r = viewport();
        let centre = state.screen(Point2::default(), r);
        assert_eq!(state.modal_cursor(centre, r), None);
        state.pulse_mode = true;
        assert_eq!(
            state.modal_cursor(centre, r),
            Some(egui::CursorIcon::Crosshair)
        );
        state.pulse_mode = false;
        state.pending_merge = Some(PendingMerge {
            action: MergeAction::Delete(BTreeSet::from([CurveSpanId(99)])),
            choices: vec![],
        });
        assert_eq!(
            state.modal_cursor(centre, r),
            None,
            "nowhere is clickable while no candidate is offered"
        );
    }

    /// Whatever appeared under the pointer stays for the gesture it started.
    #[test]
    fn a_gesture_keeps_the_cursor_it_began_with() {
        let mut state = Playground::default();
        assert_eq!(state.drag_cursor(), None);
        state.drag = Some(DragGesture::Domain {
            drag: DomainDrag::Side {
                side: OuterSide::Left,
                start: state.editor.document.model.draft.geometry.domain,
            },
        });
        assert_eq!(
            state.drag_cursor(),
            Some(egui::CursorIcon::ResizeHorizontal)
        );
        state.drag = Some(DragGesture::Marquee {
            start: Pos2::ZERO,
            current: Pos2::ZERO,
            base: BTreeSet::new(),
            operation: MarqueeOperation::Replace,
        });
        assert_eq!(
            state.drag_cursor(),
            None,
            "a marquee needs no cursor of its own"
        );
    }

    #[test]
    fn vector_overlay_keeps_the_mechanical_skin_on_physical_observables() {
        assert_eq!(
            VectorOverlay::choices(PhysicsModel::Mechanical),
            &[VectorOverlay::Off, VectorOverlay::RelativeEnergyFlow]
        );
        assert_eq!(
            VectorOverlay::choices(PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            })
            .len(),
            3
        );
        assert_eq!(
            VectorOverlay::ComplementaryField.resolved(PhysicsModel::Mechanical),
            VectorOverlay::Off
        );
        assert_eq!(
            VectorOverlay::ComplementaryField.resolved(PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            }),
            VectorOverlay::ComplementaryField
        );
        assert_eq!(
            VectorOverlay::ComplementaryField.label(PhysicsModel::Mechanical),
            "Off"
        );
    }
}
