use std::{
    borrow::Cow,
    cmp::Ordering as CmpOrdering,
    collections::BinaryHeap,
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
};

use bevy::{
    asset::{embedded_asset, load_embedded_asset},
    core_pipeline::schedule::camera_driver,
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        gpu_readback::{Readback, ReadbackComplete},
        render_asset::RenderAssets,
        render_resource::{
            BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
            CachedComputePipelineId, CachedPipelineState, ComputePassDescriptor,
            ComputePipelineDescriptor, PipelineCache, ShaderStages, ShaderType,
            binding_types::{storage_buffer, storage_buffer_read_only},
        },
        renderer::{RenderContext, RenderDevice, RenderGraph},
        storage::{GpuShaderBuffer, ShaderBuffer},
    },
};
use funfern_core::{
    CanonicalPointStencil, CanonicalTemporalPointStencil, CanonicalTemporalWaveOperator,
    CanonicalWaveOperator, CompiledVolumeSources, GRID_SCALE_FILTER_CADENCE,
    GRID_SCALE_FILTER_STRENGTH, MAX_VOLUME_SOURCES, OuterBoundaryConditions, PhysicsModel, Point2,
    PointSource, QuadraticAreaElement, QuadraticAreaStencil, QuadraticPointStencil,
    QuadraticTransferMap, QuadraticWaveOperator, QuadraticWaveState, RegionId, TimeSignal, TriMesh,
    canonical_area_quadrature, source_ramp_seconds,
};

use crate::canonical_gpu::{
    CanonicalGpuRequest, CanonicalGpuTemporalManifest, GpuCanonicalControl, GpuCanonicalNode,
    GpuCanonicalStateWord, GpuCanonicalTableWord,
};
use crate::paced_readback::{PacedReadback, PacedReadbackPlugin};

const WORKGROUP_SIZE: u32 = 128;
pub const MAX_POINT_PROBES: usize = 16;
const PROBE_RING_FRAMES: usize = 2048;
pub const MAX_CURVE_PROBE_POINTS: usize = 512;
const CURVE_PROBE_RING_FRAMES: usize = 64;
pub const MAX_AREA_PROBE_ELEMENTS: usize = 200_000;
const AREA_PROBE_RING_FRAMES: usize = 2048;
pub const FAR_FIELD_CONTOUR_POINTS: usize = 256;
pub const FAR_FIELD_DIRECTIONS: usize = 96;
const FAR_FIELD_RING_FRAMES: usize = 512;
pub const FAR_FIELD_SAMPLE_RATE: f64 = 60.0;
pub const MAX_VECTOR_OVERLAY_SAMPLES: usize = 16_384;
const WAVE_STORAGE_BINDINGS: usize = 8;
const TRANSFER_STORAGE_BINDINGS: usize = 8;
const AREA_PROBE_STORAGE_BINDINGS: usize = 7;
const WEBGPU_PORTABLE_STORAGE_BUFFER_LIMIT: usize = 8;
const _: () = {
    assert!(WAVE_STORAGE_BINDINGS <= WEBGPU_PORTABLE_STORAGE_BUFFER_LIMIT);
    assert!(TRANSFER_STORAGE_BINDINGS <= WEBGPU_PORTABLE_STORAGE_BUFFER_LIMIT);
    assert!(AREA_PROBE_STORAGE_BINDINGS <= WEBGPU_PORTABLE_STORAGE_BUFFER_LIMIT);
};
/// Steps encoded per rendered frame. The host paces its own requests with the
/// same bound so the requested and completed counters cannot diverge without
/// limit when the solver cannot run faster than wall-clock time.
pub const MAX_STEPS_PER_FRAME: u64 = 64;
const MAX_ENCODED_STEP_LEAD: u64 = MAX_STEPS_PER_FRAME;
const STATUS_READY: u8 = 1;
const STATUS_ERROR: u8 = 2;
const STATUS_TRANSFERRING: u8 = 3;

#[derive(Clone, Copy, Debug)]
pub struct PulseSettings {
    pub position: Point2,
    pub amplitude: f32,
    pub width: f32,
    pub region: RegionId,
}

pub struct WaveTransfer<'a> {
    pub source_mesh: &'a TriMesh,
    pub source_operator: &'a QuadraticWaveOperator,
    pub target_mesh: &'a TriMesh,
    pub target_operator: &'a QuadraticWaveOperator,
    pub target_time_step: f64,
    pub source: PointSource,
    pub map: &'a QuadraticTransferMap,
}

#[derive(Default)]
pub struct WaveGpuStats {
    completed_steps: AtomicU64,
    dispatches: AtomicU64,
    status: AtomicU8,
}

impl WaveGpuStats {
    pub fn completed_steps(&self) -> u64 {
        self.completed_steps.load(Ordering::Relaxed)
    }

    pub fn dispatches(&self) -> u64 {
        self.dispatches.load(Ordering::Relaxed)
    }

    pub fn status(&self) -> &'static str {
        match self.status.load(Ordering::Relaxed) {
            STATUS_READY => "ready",
            STATUS_ERROR => "error",
            STATUS_TRANSFERRING => "transferring",
            _ => "loading",
        }
    }
}

#[derive(Clone)]
struct WaveTransferHandles {
    old: WaveBufferHandles,
    old_dof_count: u32,
    entries: Handle<ShaderBuffer>,
}

#[derive(Clone)]
struct WaveBufferHandles {
    parameters: Handle<ShaderBuffer>,
    forcing: Handle<ShaderBuffer>,
    row_offsets: Handle<ShaderBuffer>,
    columns: Handle<ShaderBuffer>,
    stiffness: Handle<ShaderBuffer>,
    nodes: Handle<ShaderBuffer>,
    state: Handle<ShaderBuffer>,
    forcing_weights: Handle<ShaderBuffer>,
    source: PointSource,
    pulse: PulseSettings,
    source_weights: Arc<[f32]>,
    pulse_weights: Arc<[f32]>,
    volume_sources: Arc<CompiledVolumeSources>,
}

#[derive(Clone)]
pub(crate) struct ProbeBufferHandles {
    stencils: Handle<ShaderBuffer>,
    control: Handle<ShaderBuffer>,
    output: Handle<ShaderBuffer>,
    ids: Arc<[u64]>,
    pub(crate) sample_stride: u64,
    physics: PhysicsModel,
    pub(crate) canonical: bool,
}

#[derive(Clone)]
pub(crate) struct CurveProbeBufferHandles {
    stencils: Handle<ShaderBuffer>,
    control: Handle<ShaderBuffer>,
    output: Handle<ShaderBuffer>,
    descriptors: Arc<[CurveProbeDescriptor]>,
    pub(crate) sample_strides: Arc<[u64]>,
    pub(crate) point_count: u32,
    physics: PhysicsModel,
    pub(crate) canonical: bool,
}

#[derive(Clone)]
pub(crate) struct AreaProbeBufferHandles {
    contributions: Handle<ShaderBuffer>,
    descriptors: Handle<ShaderBuffer>,
    control: Handle<ShaderBuffer>,
    scratch: Handle<ShaderBuffer>,
    output: Handle<ShaderBuffer>,
    ids: Arc<[u64]>,
    pub(crate) sample_stride: u64,
    pub(crate) contribution_count: u32,
    physics: PhysicsModel,
    pub(crate) canonical: bool,
}

#[derive(Clone)]
pub(crate) struct FarFieldBufferHandles {
    stencils: Handle<ShaderBuffer>,
    control: Handle<ShaderBuffer>,
    raw: Handle<ShaderBuffer>,
    output: Handle<ShaderBuffer>,
    /// What the recorded history describes. Only the stencils behind it are
    /// mesh-bound, so a new mesh over the same contour inherits the ring.
    contour: FarFieldContour,
    pub(crate) sample_stride: u64,
    pub(crate) canonical: bool,
}

#[derive(Clone)]
pub(crate) struct VectorOverlayBufferHandles {
    stencils: Handle<ShaderBuffer>,
    control: Handle<ShaderBuffer>,
    output: Handle<ShaderBuffer>,
    pub(crate) sample_count: u32,
}

/// The part of a far-field recorder that the mesh does not name: where the
/// contour is, how fast the exterior carries a wave, and the bucket the ring
/// counts in. Two recorders that agree here record the same time series.
#[derive(Clone, Debug, PartialEq)]
struct FarFieldContour {
    points: Vec<(Point2, Point2)>,
    wave_speed: f64,
    sample_spacing: f64,
    delay_margin: f64,
    period: f64,
}

/// What became of the recorded history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FarFieldHandoff {
    /// No recorder is running.
    Off,
    /// The contour is the one already being recorded, so the new mesh took over
    /// the ring and the projection keeps reading across the swap.
    Kept,
    /// The ring starts over, and reports nothing until it holds a whole delay
    /// window again.
    Restarted,
}

/// Whether a recorder replacing another inherits what it recorded. The solver
/// clock is carried across a transfer, so a ring only has to start over when
/// the clock itself does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecorderHistory {
    Keep,
    Restart,
}

/// What every probe recorder needs beyond the probes themselves: the step the
/// solver is taking, the physics its samples mean, and whether it inherits the
/// ring the last recorder was filling.
#[derive(Clone, Copy, Debug)]
pub struct RecorderContext {
    pub time_step: f64,
    pub physics: PhysicsModel,
    pub history: RecorderHistory,
}

/// What an unwritten ring frame holds. A recorded time is never negative, and
/// a comparison against a real number is one no compiler is free to fold away,
/// which `w != w` is: under the fast-math the Metal backend compiles with, a
/// NaN test silently becomes `false` and every unwritten frame reads as data.
const FAR_FIELD_UNRECORDED: f32 = -1.0e30;

/// The ring is a time series, not a step series: a frame is one bucket of the
/// wave clock, so a mesh swap that moves the time step neither shifts the write
/// cursor nor restretches what is already recorded. The period leaves 1/60 s
/// only for a step longer than that, which keeps it fixed under the small
/// changes an adaptation makes.
fn far_field_period(time_step: f64) -> f64 {
    let mut period = 1.0 / FAR_FIELD_SAMPLE_RATE;
    for _ in 0..64 {
        if period >= time_step {
            break;
        }
        period *= 2.0;
    }
    period
}

#[cfg(test)]
fn far_field_bucket(time: f64, period: f64) -> f64 {
    (time / period).floor()
}

/// How often to encode a recorder pass. The shader decides what to keep, by
/// asking the ring whether the bucket it has reached is already recorded, so
/// this only has to be dense enough to visit every bucket — it cannot skip one,
/// and it need not track the solver's own clock, which drifts from step times
/// by more than a step over a long run. The quarter-bucket spacing is what
/// keeps a recorded sample near the start of its bucket.
fn far_field_sample_stride(period: f64, time_step: f64) -> u64 {
    if !period.is_finite() || !time_step.is_finite() || period <= 0.0 || time_step <= 0.0 {
        return 1;
    }
    ((period / (4.0 * time_step)).floor() as u64).max(1)
}

impl FarFieldBufferHandles {
    fn all(&self) -> [&Handle<ShaderBuffer>; 4] {
        [&self.stencils, &self.control, &self.raw, &self.output]
    }
}

impl VectorOverlayBufferHandles {
    fn all(&self) -> [&Handle<ShaderBuffer>; 3] {
        [&self.stencils, &self.control, &self.output]
    }
}

impl AreaProbeBufferHandles {
    fn all(&self) -> [&Handle<ShaderBuffer>; 5] {
        [
            &self.contributions,
            &self.descriptors,
            &self.control,
            &self.scratch,
            &self.output,
        ]
    }
}

impl CurveProbeBufferHandles {
    fn all(&self) -> [&Handle<ShaderBuffer>; 3] {
        [&self.stencils, &self.control, &self.output]
    }
}

impl ProbeBufferHandles {
    fn all(&self) -> [&Handle<ShaderBuffer>; 3] {
        [&self.stencils, &self.control, &self.output]
    }
}

impl WaveBufferHandles {
    fn all(&self) -> [&Handle<ShaderBuffer>; WAVE_STORAGE_BINDINGS] {
        [
            &self.parameters,
            &self.forcing,
            &self.row_offsets,
            &self.columns,
            &self.stiffness,
            &self.nodes,
            &self.state,
            &self.forcing_weights,
        ]
    }
}

#[derive(Resource, Clone, ExtractResource)]
pub struct WaveGpuRequest {
    generation: u64,
    buffers: Option<WaveBufferHandles>,
    dof_count: u32,
    desired_steps: u64,
    pulse_serial: u64,
    buffer_revision: u64,
    stats: Arc<WaveGpuStats>,
    readback_entity: Option<Entity>,
    transfer: Option<WaveTransferHandles>,
    pub(crate) probes: Option<ProbeBufferHandles>,
    pub(crate) probe_revision: u64,
    probe_readback_entity: Option<Entity>,
    pub(crate) curve_probes: Option<CurveProbeBufferHandles>,
    pub(crate) curve_probe_revision: u64,
    curve_probe_readback_entity: Option<Entity>,
    pub(crate) area_probes: Option<AreaProbeBufferHandles>,
    pub(crate) area_probe_revision: u64,
    area_probe_readback_entity: Option<Entity>,
    pub(crate) far_field: Option<FarFieldBufferHandles>,
    pub(crate) far_field_revision: u64,
    far_field_readback_entity: Option<Entity>,
    pub(crate) vector_overlay: Option<VectorOverlayBufferHandles>,
    pub(crate) vector_overlay_revision: u64,
    vector_overlay_readback_entity: Option<Entity>,
    grid_scale_filter: bool,
}

impl Default for WaveGpuRequest {
    fn default() -> Self {
        Self {
            generation: 0,
            buffers: None,
            dof_count: 0,
            desired_steps: 0,
            pulse_serial: 0,
            buffer_revision: 0,
            stats: Arc::new(WaveGpuStats::default()),
            readback_entity: None,
            transfer: None,
            probes: None,
            probe_revision: 0,
            probe_readback_entity: None,
            curve_probes: None,
            curve_probe_revision: 0,
            curve_probe_readback_entity: None,
            area_probes: None,
            area_probe_revision: 0,
            area_probe_readback_entity: None,
            far_field: None,
            far_field_revision: 0,
            far_field_readback_entity: None,
            vector_overlay: None,
            vector_overlay_revision: 0,
            vector_overlay_readback_entity: None,
            grid_scale_filter: true,
        }
    }
}

impl WaveGpuRequest {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Stage-6 recorder ownership follows the accepted canonical generation.
    /// Rings deliberately survive; the configure paths decide whether their
    /// observable and contour remain compatible.
    pub fn adopt_canonical_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    /// Whether the solver removes what the mesh cannot carry. See
    /// [`QuadraticWaveOperator::apply_grid_scale_filter`]; the dispatcher runs
    /// it every [`GRID_SCALE_FILTER_CADENCE`] steps while this is set.
    pub fn grid_scale_filter(&self) -> bool {
        self.grid_scale_filter
    }

    pub fn set_grid_scale_filter(&mut self, enabled: bool) {
        self.grid_scale_filter = enabled;
    }

    pub fn buffer_revision(&self) -> u64 {
        self.buffer_revision
    }

    pub fn stats(&self) -> &Arc<WaveGpuStats> {
        &self.stats
    }

    pub fn probe_revision(&self) -> u64 {
        self.probe_revision
    }

    pub fn curve_probe_revision(&self) -> u64 {
        self.curve_probe_revision
    }

    pub fn area_probe_revision(&self) -> u64 {
        self.area_probe_revision
    }

    pub fn far_field_revision(&self) -> u64 {
        self.far_field_revision
    }

    pub fn vector_overlay_revision(&self) -> u64 {
        self.vector_overlay_revision
    }

    pub fn update_point_probes(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        probes: &[(u64, Option<QuadraticPointStencil>)],
        sample_rate: f64,
        context: RecorderContext,
    ) -> Result<(), String> {
        let RecorderContext {
            time_step,
            physics,
            history,
        } = context;
        if probes.len() > MAX_POINT_PROBES
            || !sample_rate.is_finite()
            || !(30.0..=480.0).contains(&sample_rate)
            || !time_step.is_finite()
            || time_step <= 0.0
        {
            self.clear_probe_buffers(assets, commands);
            return Err("Invalid point-probe recorder settings".into());
        }
        if probes.is_empty() {
            self.clear_probe_buffers(assets, commands);
            return Ok(());
        }
        let sample_stride = (1.0 / (sample_rate * time_step)).round().max(1.0) as u64;
        let ids = probes.iter().map(|(id, _)| *id).collect::<Arc<[u64]>>();
        let stencils = probes
            .iter()
            .map(|(_, stencil)| gpu_probe_stencil(*stencil))
            .collect::<Vec<_>>();
        let control = GpuProbeControl {
            values: Vec4::new(
                sample_stride as f32,
                PROBE_RING_FRAMES as f32,
                probes.len() as f32,
                probe_physics_flag(physics),
            ),
        };
        // The same probes read through a new mesh keep their ring. What the GPU
        // wrote since the last readback is still in it, and the host takes
        // records by time rather than by slot, so where the new stencils resume
        // writing does not matter.
        let kept = history == RecorderHistory::Keep
            && self.probes.as_ref().is_some_and(|handles| {
                handles.ids == ids && handles.physics == physics && !handles.canonical
            });
        let handles = if kept {
            let previous = self.probes.take().expect("a kept ring exists");
            assets.remove(previous.stencils.id());
            assets.remove(previous.control.id());
            ProbeBufferHandles {
                stencils: assets.add(ShaderBuffer::from(stencils)),
                control: assets.add(ShaderBuffer::from(control)),
                sample_stride,
                physics,
                canonical: false,
                ..previous
            }
        } else {
            self.clear_probe_buffers(assets, commands);
            let output = vec![
                GpuPointProbeSample {
                    primary: Vec4::splat(f32::NAN),
                    secondary: Vec4::splat(f32::NAN),
                };
                PROBE_RING_FRAMES * MAX_POINT_PROBES
            ];
            ProbeBufferHandles {
                stencils: assets.add(ShaderBuffer::from(stencils)),
                control: assets.add(ShaderBuffer::from(control)),
                output: assets.add(ShaderBuffer::from(output)),
                ids,
                sample_stride,
                physics,
                canonical: false,
            }
        };
        if let Some(entity) = self.probe_readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.probe_revision = self.probe_revision.wrapping_add(1).max(1);
        self.probe_readback_entity = Some(
            commands
                .spawn((
                    PacedReadback::continuous(Readback::buffer(handles.output.clone())),
                    ProbeReadbackTag {
                        generation: self.generation,
                        revision: self.probe_revision,
                        ids: handles.ids.clone(),
                        canonical: false,
                    },
                ))
                .id(),
        );
        self.probes = Some(handles);
        Ok(())
    }

    pub fn update_canonical_point_probes(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        operator: &CanonicalWaveOperator,
        probes: &[(u64, Option<QuadraticPointStencil>)],
        sample_rate: f64,
        context: RecorderContext,
    ) -> Result<(), String> {
        let RecorderContext {
            time_step,
            physics,
            history,
        } = context;
        if probes.len() > MAX_POINT_PROBES
            || !sample_rate.is_finite()
            || !(30.0..=480.0).contains(&sample_rate)
            || !time_step.is_finite()
            || time_step <= 0.0
        {
            self.clear_probe_buffers(assets, commands);
            return Err("Invalid point-probe recorder settings".into());
        }
        if probes.is_empty() {
            self.clear_probe_buffers(assets, commands);
            return Ok(());
        }
        let sample_stride = (1.0 / (sample_rate * time_step)).round().max(1.0) as u64;
        let stencils = probes
            .iter()
            .map(|(_, stencil)| {
                stencil
                    .map(|stencil| CanonicalPointStencil::from_quadratic(stencil, operator))
                    .transpose()
                    .map(gpu_canonical_point_stencil)
                    .map_err(|error| format!("Canonical point reconstruction failed: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.install_canonical_point_probes(
            assets,
            commands,
            probes,
            stencils,
            sample_stride,
            physics,
            history,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_temporal_canonical_point_probes(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        operator: &CanonicalTemporalWaveOperator,
        manifest: CanonicalGpuTemporalManifest,
        probes: &[(u64, Option<QuadraticPointStencil>)],
        sample_rate: f64,
        context: RecorderContext,
    ) -> Result<(), String> {
        let RecorderContext {
            time_step,
            physics,
            history,
        } = context;
        if probes.len() > MAX_POINT_PROBES
            || !sample_rate.is_finite()
            || !(30.0..=480.0).contains(&sample_rate)
            || !time_step.is_finite()
            || time_step <= 0.0
        {
            self.clear_probe_buffers(assets, commands);
            return Err("Invalid point-probe recorder settings".into());
        }
        if probes.is_empty() {
            self.clear_probe_buffers(assets, commands);
            return Ok(());
        }
        let sample_stride = (1.0 / (sample_rate * time_step)).round().max(1.0) as u64;
        let stencils = probes
            .iter()
            .map(|(_, stencil)| match stencil {
                Some(stencil) => {
                    let stencil = CanonicalTemporalPointStencil::from_quadratic(*stencil, operator)
                        .map_err(|error| {
                            format!("Temporal point reconstruction failed: {error}")
                        })?;
                    gpu_temporal_canonical_point_stencil(stencil, operator, manifest)
                }
                None => Ok(GpuCanonicalPointStencil::default()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.install_canonical_point_probes(
            assets,
            commands,
            probes,
            stencils,
            sample_stride,
            physics,
            history,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn install_canonical_point_probes(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        probes: &[(u64, Option<QuadraticPointStencil>)],
        stencils: Vec<GpuCanonicalPointStencil>,
        sample_stride: u64,
        physics: PhysicsModel,
        history: RecorderHistory,
    ) -> Result<(), String> {
        let ids = probes.iter().map(|(id, _)| *id).collect::<Arc<[u64]>>();
        let control = GpuProbeControl {
            values: Vec4::new(
                sample_stride as f32,
                PROBE_RING_FRAMES as f32,
                probes.len() as f32,
                probe_physics_flag(physics),
            ),
        };
        let kept = history == RecorderHistory::Keep
            && self.probes.as_ref().is_some_and(|handles| {
                handles.ids == ids && handles.physics == physics && handles.canonical
            });
        let handles = if kept {
            let previous = self.probes.take().expect("a kept ring exists");
            assets.remove(previous.stencils.id());
            assets.remove(previous.control.id());
            ProbeBufferHandles {
                stencils: assets.add(ShaderBuffer::from(stencils)),
                control: assets.add(ShaderBuffer::from(control)),
                sample_stride,
                physics,
                canonical: true,
                ..previous
            }
        } else {
            self.clear_probe_buffers(assets, commands);
            let output = vec![
                GpuCanonicalPointProbeSample {
                    primary: Vec4::splat(f32::NAN),
                    secondary: Vec4::splat(f32::NAN),
                    reserved: Vec4::ZERO,
                };
                PROBE_RING_FRAMES * MAX_POINT_PROBES
            ];
            ProbeBufferHandles {
                stencils: assets.add(ShaderBuffer::from(stencils)),
                control: assets.add(ShaderBuffer::from(control)),
                output: assets.add(ShaderBuffer::from(output)),
                ids,
                sample_stride,
                physics,
                canonical: true,
            }
        };
        if let Some(entity) = self.probe_readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.probe_revision = self.probe_revision.wrapping_add(1).max(1);
        self.probe_readback_entity = Some(
            commands
                .spawn((
                    PacedReadback::continuous(Readback::buffer(handles.output.clone())),
                    ProbeReadbackTag {
                        generation: self.generation,
                        revision: self.probe_revision,
                        ids: handles.ids.clone(),
                        canonical: true,
                    },
                ))
                .id(),
        );
        self.probes = Some(handles);
        Ok(())
    }

    fn clear_probe_buffers(&mut self, assets: &mut Assets<ShaderBuffer>, commands: &mut Commands) {
        if let Some(handles) = self.probes.take() {
            for handle in handles.all() {
                assets.remove(handle.id());
            }
        }
        if let Some(entity) = self.probe_readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.probe_revision = self.probe_revision.wrapping_add(1).max(1);
    }

    /// Installs the compact set of view-selected canonical stencils used by
    /// the arrow overlay. The GPU samples both vector observables once per
    /// rendered solver batch, so display-rate arrows do not require a
    /// full-state readback or a CPU traversal of the whole mesh.
    pub fn update_canonical_vector_overlay(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        operator: &CanonicalWaveOperator,
        stencils: &[QuadraticPointStencil],
    ) -> Result<(), String> {
        if stencils.len() > MAX_VECTOR_OVERLAY_SAMPLES {
            self.clear_vector_overlay(assets, commands);
            return Err(format!(
                "Vector overlay requests {} arrows; maximum is {MAX_VECTOR_OVERLAY_SAMPLES}",
                stencils.len()
            ));
        }
        if stencils.is_empty() {
            self.clear_vector_overlay(assets, commands);
            return Ok(());
        }
        let stencils = stencils
            .iter()
            .copied()
            .map(|stencil| {
                CanonicalPointStencil::from_quadratic(stencil, operator)
                    .map(|stencil| gpu_canonical_point_stencil(Some(stencil)))
                    .map_err(|error| format!("Canonical vector reconstruction failed: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.clear_vector_overlay(assets, commands);
        let control = GpuProbeControl {
            values: Vec4::new(0.0, 0.0, stencils.len() as f32, 0.0),
        };
        let output = vec![GpuVectorOverlaySample::default(); stencils.len()];
        let handles = VectorOverlayBufferHandles {
            sample_count: stencils.len() as u32,
            stencils: assets.add(ShaderBuffer::from(stencils)),
            control: assets.add(ShaderBuffer::from(control)),
            output: assets.add(ShaderBuffer::from(output)),
        };
        self.vector_overlay_revision = self.vector_overlay_revision.wrapping_add(1).max(1);
        self.vector_overlay_readback_entity = Some(
            commands
                .spawn((
                    PacedReadback::continuous(Readback::buffer(handles.output.clone())),
                    VectorOverlayReadbackTag {
                        generation: self.generation,
                        revision: self.vector_overlay_revision,
                        sample_count: handles.sample_count,
                    },
                ))
                .id(),
        );
        self.vector_overlay = Some(handles);
        Ok(())
    }

    pub fn clear_vector_overlay(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
    ) {
        let had_overlay =
            self.vector_overlay.is_some() || self.vector_overlay_readback_entity.is_some();
        if let Some(handles) = self.vector_overlay.take() {
            for handle in handles.all() {
                assets.remove(handle.id());
            }
        }
        if let Some(entity) = self.vector_overlay_readback_entity.take() {
            commands.entity(entity).try_despawn();
        }
        if had_overlay {
            self.vector_overlay_revision = self.vector_overlay_revision.wrapping_add(1).max(1);
        }
    }

    pub fn update_curve_probes(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        probes: &[CurveProbeInput],
        context: RecorderContext,
    ) -> Result<(), String> {
        let RecorderContext {
            time_step,
            physics,
            history,
        } = context;
        if !time_step.is_finite() || time_step <= 0.0 {
            self.clear_curve_probe_buffers(assets, commands);
            return Err("Invalid line-probe recorder timestep".into());
        }
        let point_count = probes
            .iter()
            .map(|probe| probe.samples.len())
            .sum::<usize>();
        if point_count > MAX_CURVE_PROBE_POINTS
            || probes.iter().any(|probe| {
                probe.samples.len() < 2
                    || !probe.sample_rate.is_finite()
                    || !(30.0..=120.0).contains(&probe.sample_rate)
                    || probe
                        .samples
                        .iter()
                        .flatten()
                        .any(|(_, normal)| !normal.finite())
            })
        {
            self.clear_curve_probe_buffers(assets, commands);
            return Err("Invalid line-probe recorder settings".into());
        }
        if probes.is_empty() {
            self.clear_curve_probe_buffers(assets, commands);
            return Ok(());
        }
        let mut stencils = Vec::with_capacity(point_count);
        let mut descriptors = Vec::with_capacity(probes.len());
        let mut sample_strides = Vec::with_capacity(probes.len());
        for probe in probes {
            let offset = stencils.len() as u32;
            let stride = (1.0 / (probe.sample_rate * time_step)).round().max(1.0) as u64;
            stencils.extend(
                probe
                    .samples
                    .iter()
                    .map(|sample| gpu_curve_probe_stencil(*sample, stride)),
            );
            descriptors.push(CurveProbeDescriptor {
                id: probe.id,
                offset,
                count: probe.samples.len() as u32,
            });
            sample_strides.push(stride);
        }
        let control = GpuProbeControl {
            values: Vec4::new(
                point_count as f32,
                CURVE_PROBE_RING_FRAMES as f32,
                MAX_CURVE_PROBE_POINTS as f32,
                probe_physics_flag(physics),
            ),
        };
        let output = vec![
            GpuCurveProbeSample {
                primary: Vec4::splat(f32::NAN),
                secondary: Vec4::splat(f32::NAN),
            };
            CURVE_PROBE_RING_FRAMES * MAX_CURVE_PROBE_POINTS
        ];
        let descriptors = Arc::<[CurveProbeDescriptor]>::from(descriptors);
        // The same probes over the same sample points keep their ring, whatever
        // mesh the stencils now read. Records are taken by time, not by slot.
        let kept = history == RecorderHistory::Keep
            && self.curve_probes.as_ref().is_some_and(|handles| {
                handles.descriptors == descriptors
                    && handles.physics == physics
                    && !handles.canonical
            });
        let handles = if kept {
            let previous = self.curve_probes.take().expect("a kept ring exists");
            assets.remove(previous.stencils.id());
            assets.remove(previous.control.id());
            CurveProbeBufferHandles {
                stencils: assets.add(ShaderBuffer::from(stencils)),
                control: assets.add(ShaderBuffer::from(control)),
                sample_strides: sample_strides.into(),
                physics,
                canonical: false,
                ..previous
            }
        } else {
            self.clear_curve_probe_buffers(assets, commands);
            CurveProbeBufferHandles {
                stencils: assets.add(ShaderBuffer::from(stencils)),
                control: assets.add(ShaderBuffer::from(control)),
                output: assets.add(ShaderBuffer::from(output)),
                descriptors,
                sample_strides: sample_strides.into(),
                point_count: point_count as u32,
                physics,
                canonical: false,
            }
        };
        if let Some(entity) = self.curve_probe_readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.curve_probe_revision = self.curve_probe_revision.wrapping_add(1).max(1);
        self.curve_probe_readback_entity = Some(
            commands
                .spawn((
                    PacedReadback::continuous(Readback::buffer(handles.output.clone())),
                    CurveProbeReadbackTag {
                        generation: self.generation,
                        revision: self.curve_probe_revision,
                        descriptors: handles.descriptors.clone(),
                    },
                ))
                .id(),
        );
        self.curve_probes = Some(handles);
        Ok(())
    }

    pub fn update_canonical_curve_probes(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        operator: &CanonicalWaveOperator,
        probes: &[CurveProbeInput],
        context: RecorderContext,
    ) -> Result<(), String> {
        let RecorderContext {
            time_step,
            physics,
            history,
        } = context;
        if !time_step.is_finite() || time_step <= 0.0 {
            self.clear_curve_probe_buffers(assets, commands);
            return Err("Invalid line-probe recorder timestep".into());
        }
        let point_count = probes
            .iter()
            .map(|probe| probe.samples.len())
            .sum::<usize>();
        if point_count > MAX_CURVE_PROBE_POINTS
            || probes.iter().any(|probe| {
                probe.samples.len() < 2
                    || !probe.sample_rate.is_finite()
                    || !(30.0..=120.0).contains(&probe.sample_rate)
                    || probe
                        .samples
                        .iter()
                        .flatten()
                        .any(|(_, normal)| !normal.finite())
            })
        {
            self.clear_curve_probe_buffers(assets, commands);
            return Err("Invalid line-probe recorder settings".into());
        }
        if probes.is_empty() {
            self.clear_curve_probe_buffers(assets, commands);
            return Ok(());
        }
        let mut stencils = Vec::with_capacity(point_count);
        let mut descriptors = Vec::with_capacity(probes.len());
        let mut sample_strides = Vec::with_capacity(probes.len());
        for probe in probes {
            let offset = stencils.len() as u32;
            let stride = (1.0 / (probe.sample_rate * time_step)).round().max(1.0) as u64;
            for sample in &probe.samples {
                let (point, normal, valid) = match sample {
                    Some((stencil, normal)) => (
                        gpu_canonical_point_stencil(Some(
                            CanonicalPointStencil::from_quadratic(*stencil, operator).map_err(
                                |error| format!("Canonical line reconstruction failed: {error}"),
                            )?,
                        )),
                        *normal,
                        1.0,
                    ),
                    None => (GpuCanonicalPointStencil::default(), Point2::default(), 0.0),
                };
                stencils.push(GpuCanonicalCurveStencil {
                    point,
                    normal_stride_valid: Vec4::new(
                        normal.x as f32,
                        normal.y as f32,
                        stride as f32,
                        valid,
                    ),
                });
            }
            descriptors.push(CurveProbeDescriptor {
                id: probe.id,
                offset,
                count: probe.samples.len() as u32,
            });
            sample_strides.push(stride);
        }
        let control = GpuProbeControl {
            values: Vec4::new(
                point_count as f32,
                CURVE_PROBE_RING_FRAMES as f32,
                MAX_CURVE_PROBE_POINTS as f32,
                probe_physics_flag(physics),
            ),
        };
        let descriptors = Arc::<[CurveProbeDescriptor]>::from(descriptors);
        let kept = history == RecorderHistory::Keep
            && self.curve_probes.as_ref().is_some_and(|handles| {
                handles.descriptors == descriptors
                    && handles.physics == physics
                    && handles.canonical
            });
        let handles = if kept {
            let previous = self.curve_probes.take().expect("a kept ring exists");
            assets.remove(previous.stencils.id());
            assets.remove(previous.control.id());
            CurveProbeBufferHandles {
                stencils: assets.add(ShaderBuffer::from(stencils)),
                control: assets.add(ShaderBuffer::from(control)),
                sample_strides: sample_strides.into(),
                physics,
                canonical: true,
                ..previous
            }
        } else {
            self.clear_curve_probe_buffers(assets, commands);
            let output = vec![
                GpuCurveProbeSample {
                    primary: Vec4::splat(f32::NAN),
                    secondary: Vec4::splat(f32::NAN),
                };
                CURVE_PROBE_RING_FRAMES * MAX_CURVE_PROBE_POINTS
            ];
            CurveProbeBufferHandles {
                stencils: assets.add(ShaderBuffer::from(stencils)),
                control: assets.add(ShaderBuffer::from(control)),
                output: assets.add(ShaderBuffer::from(output)),
                descriptors,
                sample_strides: sample_strides.into(),
                point_count: point_count as u32,
                physics,
                canonical: true,
            }
        };
        if let Some(entity) = self.curve_probe_readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.curve_probe_revision = self.curve_probe_revision.wrapping_add(1).max(1);
        self.curve_probe_readback_entity = Some(
            commands
                .spawn((
                    PacedReadback::continuous(Readback::buffer(handles.output.clone())),
                    CurveProbeReadbackTag {
                        generation: self.generation,
                        revision: self.curve_probe_revision,
                        descriptors: handles.descriptors.clone(),
                    },
                ))
                .id(),
        );
        self.curve_probes = Some(handles);
        Ok(())
    }

    fn clear_curve_probe_buffers(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
    ) {
        if let Some(handles) = self.curve_probes.take() {
            for handle in handles.all() {
                assets.remove(handle.id());
            }
        }
        if let Some(entity) = self.curve_probe_readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.curve_probe_revision = self.curve_probe_revision.wrapping_add(1).max(1);
    }

    pub fn update_area_probes(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        probes: &[AreaProbeInput],
        sample_rate: f64,
        context: RecorderContext,
    ) -> Result<(), String> {
        let RecorderContext {
            time_step,
            physics,
            history,
        } = context;
        let contribution_count = probes
            .iter()
            .filter_map(|probe| probe.stencil.as_ref())
            .map(|stencil| stencil.elements.len())
            .sum::<usize>();
        if probes.len() > MAX_POINT_PROBES {
            self.clear_area_probe_buffers(assets, commands);
            return Err(format!("Maximum {MAX_POINT_PROBES} area probes"));
        }
        if contribution_count > MAX_AREA_PROBE_ELEMENTS {
            self.clear_area_probe_buffers(assets, commands);
            return Err(format!(
                "Area probes cover {contribution_count} element pieces; maximum is {MAX_AREA_PROBE_ELEMENTS}"
            ));
        }
        if !sample_rate.is_finite()
            || !(30.0..=120.0).contains(&sample_rate)
            || !time_step.is_finite()
            || time_step <= 0.0
        {
            self.clear_area_probe_buffers(assets, commands);
            return Err("Invalid area-probe recorder settings".into());
        }
        if probes.is_empty() {
            self.clear_area_probe_buffers(assets, commands);
            return Ok(());
        }
        let sample_stride = (1.0 / (sample_rate * time_step)).round().max(1.0) as u64;
        let mut contributions = Vec::with_capacity(contribution_count.max(1));
        let mut descriptors = Vec::with_capacity(probes.len());
        for probe in probes {
            let offset = contributions.len() as u32;
            if let Some(stencil) = &probe.stencil {
                contributions.extend(
                    stencil
                        .elements
                        .iter()
                        .copied()
                        .map(|element| gpu_area_probe_contribution(element, physics)),
                );
                descriptors.push(GpuAreaProbeDescriptor {
                    offset_count: UVec4::new(offset, stencil.elements.len() as u32, 0, 0),
                    areas: Vec4::new(
                        stencil.covered_area as f32,
                        stencil.target_area as f32,
                        1.0,
                        0.0,
                    ),
                });
            } else {
                descriptors.push(GpuAreaProbeDescriptor {
                    offset_count: UVec4::new(offset, 0, 0, 0),
                    areas: Vec4::ZERO,
                });
            }
        }
        if contributions.is_empty() {
            contributions.push(GpuAreaProbeContribution::default());
        }
        if contributions.iter().any(|contribution| {
            !contribution.field_a.is_finite()
                || !contribution.field_b.is_finite()
                || contribution.mass.iter().any(|values| !values.is_finite())
                || contribution
                    .density
                    .iter()
                    .any(|values| !values.is_finite())
                || contribution
                    .stiffness
                    .iter()
                    .any(|values| !values.is_finite())
                || contribution
                    .stiffness_squared
                    .iter()
                    .any(|values| !values.is_finite())
                || !contribution.material_area.is_finite()
        }) || descriptors
            .iter()
            .any(|descriptor| !descriptor.areas.is_finite())
        {
            self.clear_area_probe_buffers(assets, commands);
            return Err("Area-probe weights cannot be represented on the GPU".into());
        }
        let control = GpuProbeControl {
            values: Vec4::new(
                sample_stride as f32,
                AREA_PROBE_RING_FRAMES as f32,
                probes.len() as f32,
                contribution_count as f32,
            ),
        };
        let scratch = vec![GpuAreaProbeContributionSample::default(); contribution_count.max(1)];
        let ids = probes.iter().map(|probe| probe.id).collect::<Arc<[u64]>>();
        // Only the output ring carries history. The scratch buffer is one
        // dispatch's working space, so it is rebuilt with the contributions it
        // sums.
        let kept = history == RecorderHistory::Keep
            && self.area_probes.as_ref().is_some_and(|handles| {
                handles.ids == ids && handles.physics == physics && !handles.canonical
            });
        let handles = if kept {
            let previous = self.area_probes.take().expect("a kept ring exists");
            for handle in [
                previous.contributions.id(),
                previous.descriptors.id(),
                previous.control.id(),
                previous.scratch.id(),
            ] {
                assets.remove(handle);
            }
            AreaProbeBufferHandles {
                contributions: assets.add(ShaderBuffer::from(contributions)),
                descriptors: assets.add(ShaderBuffer::from(descriptors)),
                control: assets.add(ShaderBuffer::from(control)),
                scratch: assets.add(ShaderBuffer::from(scratch)),
                sample_stride,
                contribution_count: contribution_count as u32,
                physics,
                canonical: false,
                ..previous
            }
        } else {
            self.clear_area_probe_buffers(assets, commands);
            let output = vec![
                GpuAreaProbeSample {
                    primary: Vec4::splat(f32::NAN),
                    secondary: Vec4::splat(f32::NAN),
                    tertiary: Vec4::splat(f32::NAN),
                };
                AREA_PROBE_RING_FRAMES * MAX_POINT_PROBES
            ];
            AreaProbeBufferHandles {
                contributions: assets.add(ShaderBuffer::from(contributions)),
                descriptors: assets.add(ShaderBuffer::from(descriptors)),
                control: assets.add(ShaderBuffer::from(control)),
                scratch: assets.add(ShaderBuffer::from(scratch)),
                output: assets.add(ShaderBuffer::from(output)),
                ids,
                sample_stride,
                contribution_count: contribution_count as u32,
                physics,
                canonical: false,
            }
        };
        if let Some(entity) = self.area_probe_readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.area_probe_revision = self.area_probe_revision.wrapping_add(1).max(1);
        self.area_probe_readback_entity = Some(
            commands
                .spawn((
                    PacedReadback::continuous(Readback::buffer(handles.output.clone())),
                    AreaProbeReadbackTag {
                        generation: self.generation,
                        revision: self.area_probe_revision,
                        ids: handles.ids.clone(),
                    },
                ))
                .id(),
        );
        self.area_probes = Some(handles);
        Ok(())
    }

    pub fn update_canonical_area_probes(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        operator: &CanonicalWaveOperator,
        probes: &[AreaProbeInput],
        sample_rate: f64,
        context: RecorderContext,
    ) -> Result<(), String> {
        let RecorderContext {
            time_step,
            physics,
            history,
        } = context;
        let contribution_count = probes
            .iter()
            .filter_map(|probe| probe.stencil.as_ref())
            .map(|stencil| stencil.elements.len())
            .sum::<usize>();
        if probes.len() > MAX_POINT_PROBES {
            self.clear_area_probe_buffers(assets, commands);
            return Err(format!("Maximum {MAX_POINT_PROBES} area probes"));
        }
        if contribution_count > MAX_AREA_PROBE_ELEMENTS {
            self.clear_area_probe_buffers(assets, commands);
            return Err(format!(
                "Area probes cover {contribution_count} element pieces; maximum is {MAX_AREA_PROBE_ELEMENTS}"
            ));
        }
        if !sample_rate.is_finite()
            || !(30.0..=120.0).contains(&sample_rate)
            || !time_step.is_finite()
            || time_step <= 0.0
        {
            self.clear_area_probe_buffers(assets, commands);
            return Err("Invalid area-probe recorder settings".into());
        }
        if probes.is_empty() {
            self.clear_area_probe_buffers(assets, commands);
            return Ok(());
        }
        let sample_stride = (1.0 / (sample_rate * time_step)).round().max(1.0) as u64;
        let mut contributions = Vec::with_capacity(contribution_count.max(1));
        let mut descriptors = Vec::with_capacity(probes.len());
        for probe in probes {
            let offset = contributions.len() as u32;
            if let Some(stencil) = &probe.stencil {
                for element in &stencil.elements {
                    contributions.push(gpu_canonical_area_contribution(*element, operator)?);
                }
                descriptors.push(GpuAreaProbeDescriptor {
                    offset_count: UVec4::new(offset, stencil.elements.len() as u32, 0, 0),
                    areas: Vec4::new(
                        stencil.covered_area as f32,
                        stencil.target_area as f32,
                        1.0,
                        0.0,
                    ),
                });
            } else {
                descriptors.push(GpuAreaProbeDescriptor {
                    offset_count: UVec4::new(offset, 0, 0, 0),
                    areas: Vec4::ZERO,
                });
            }
        }
        if contributions.is_empty() {
            contributions.push(GpuCanonicalAreaContribution::default());
        }
        let control = GpuProbeControl {
            values: Vec4::new(
                sample_stride as f32,
                AREA_PROBE_RING_FRAMES as f32,
                probes.len() as f32,
                contribution_count as f32,
            ),
        };
        let scratch = vec![GpuAreaProbeContributionSample::default(); contribution_count.max(1)];
        let ids = probes.iter().map(|probe| probe.id).collect::<Arc<[u64]>>();
        let kept = history == RecorderHistory::Keep
            && self.area_probes.as_ref().is_some_and(|handles| {
                handles.ids == ids && handles.physics == physics && handles.canonical
            });
        let handles = if kept {
            let previous = self.area_probes.take().expect("a kept ring exists");
            for handle in [
                previous.contributions.id(),
                previous.descriptors.id(),
                previous.control.id(),
                previous.scratch.id(),
            ] {
                assets.remove(handle);
            }
            AreaProbeBufferHandles {
                contributions: assets.add(ShaderBuffer::from(contributions)),
                descriptors: assets.add(ShaderBuffer::from(descriptors)),
                control: assets.add(ShaderBuffer::from(control)),
                scratch: assets.add(ShaderBuffer::from(scratch)),
                sample_stride,
                contribution_count: contribution_count as u32,
                physics,
                canonical: true,
                ..previous
            }
        } else {
            self.clear_area_probe_buffers(assets, commands);
            let output = vec![
                GpuAreaProbeSample {
                    primary: Vec4::splat(f32::NAN),
                    secondary: Vec4::splat(f32::NAN),
                    tertiary: Vec4::splat(f32::NAN),
                };
                AREA_PROBE_RING_FRAMES * MAX_POINT_PROBES
            ];
            AreaProbeBufferHandles {
                contributions: assets.add(ShaderBuffer::from(contributions)),
                descriptors: assets.add(ShaderBuffer::from(descriptors)),
                control: assets.add(ShaderBuffer::from(control)),
                scratch: assets.add(ShaderBuffer::from(scratch)),
                output: assets.add(ShaderBuffer::from(output)),
                ids,
                sample_stride,
                contribution_count: contribution_count as u32,
                physics,
                canonical: true,
            }
        };
        if let Some(entity) = self.area_probe_readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.area_probe_revision = self.area_probe_revision.wrapping_add(1).max(1);
        self.area_probe_readback_entity = Some(
            commands
                .spawn((
                    PacedReadback::continuous(Readback::buffer(handles.output.clone())),
                    AreaProbeReadbackTag {
                        generation: self.generation,
                        revision: self.area_probe_revision,
                        ids: handles.ids.clone(),
                    },
                ))
                .id(),
        );
        self.area_probes = Some(handles);
        Ok(())
    }

    fn clear_area_probe_buffers(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
    ) {
        if let Some(handles) = self.area_probes.take() {
            for handle in handles.all() {
                assets.remove(handle.id());
            }
        }
        if let Some(entity) = self.area_probe_readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.area_probe_revision = self.area_probe_revision.wrapping_add(1).max(1);
    }

    pub fn update_far_field(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        input: Option<&FarFieldInput>,
        time_step: f64,
        history: RecorderHistory,
    ) -> Result<FarFieldHandoff, String> {
        let Some(input) = input else {
            self.clear_far_field_buffers(assets, commands);
            return Ok(FarFieldHandoff::Off);
        };
        if input.samples.len() != FAR_FIELD_CONTOUR_POINTS
            || !input.wave_speed.is_finite()
            || input.wave_speed <= 0.0
            || !input.sample_spacing.is_finite()
            || input.sample_spacing <= 0.0
            || !input.delay_margin.is_finite()
            || input.delay_margin <= 0.0
            || !time_step.is_finite()
            || time_step <= 0.0
        {
            self.clear_far_field_buffers(assets, commands);
            return Err("Invalid far-field recorder settings".into());
        }
        let period = far_field_period(time_step);
        let contour = FarFieldContour {
            points: input
                .samples
                .iter()
                .map(|(_, position, normal)| (*position, *normal))
                .collect(),
            wave_speed: input.wave_speed,
            sample_spacing: input.sample_spacing,
            delay_margin: input.delay_margin,
            period,
        };
        let stencils = input
            .samples
            .iter()
            .copied()
            .map(gpu_far_field_stencil)
            .collect::<Vec<_>>();
        if stencils.iter().any(|stencil| {
            !stencil.weights_a.is_finite()
                || !stencil.weights_b.is_finite()
                || !stencil.gradient_x_a.is_finite()
                || !stencil.gradient_x_b.is_finite()
                || !stencil.gradient_y_a.is_finite()
                || !stencil.gradient_y_b.is_finite()
                || !stencil.position_normal.is_finite()
        }) {
            self.clear_far_field_buffers(assets, commands);
            return Err("Far-field stencils cannot be represented on the GPU".into());
        }
        let maximum_history = (FAR_FIELD_RING_FRAMES - 2) as f64 * period;
        if input.history_seconds() > maximum_history {
            self.clear_far_field_buffers(assets, commands);
            return Err(format!(
                "Exterior wave speed is too low for the {:.1} s far-field delay window",
                maximum_history
            ));
        }
        let control = GpuFarFieldControl {
            sampling: Vec4::new(
                period as f32,
                FAR_FIELD_RING_FRAMES as f32,
                FAR_FIELD_CONTOUR_POINTS as f32,
                FAR_FIELD_DIRECTIONS as f32,
            ),
            projection: Vec4::new(
                input.wave_speed as f32,
                input.sample_spacing as f32,
                input.delay_margin as f32,
                0.0,
            ),
        };
        // The history is a record of the exterior at fixed world points, so a
        // handoff needs nothing from the old mesh: the stencils that read the
        // new one take over the ring the old one was filling.
        let kept = history == RecorderHistory::Keep
            && self
                .far_field
                .as_ref()
                .is_some_and(|handles| handles.contour == contour && !handles.canonical);
        let handles = if kept {
            let previous = self.far_field.take().expect("a kept ring exists");
            assets.remove(previous.stencils.id());
            assets.remove(previous.control.id());
            FarFieldBufferHandles {
                stencils: assets.add(ShaderBuffer::from(stencils)),
                control: assets.add(ShaderBuffer::from(control)),
                contour,
                sample_stride: far_field_sample_stride(period, time_step),
                canonical: false,
                ..previous
            }
        } else {
            self.clear_far_field_buffers(assets, commands);
            let raw = vec![
                GpuProbeSample {
                    values: Vec4::new(0.0, 0.0, 0.0, FAR_FIELD_UNRECORDED),
                };
                FAR_FIELD_RING_FRAMES * FAR_FIELD_CONTOUR_POINTS
            ];
            let output = vec![
                GpuProbeSample {
                    values: Vec4::new(0.0, 0.0, FAR_FIELD_UNRECORDED, 0.0),
                };
                FAR_FIELD_RING_FRAMES * FAR_FIELD_DIRECTIONS
            ];
            FarFieldBufferHandles {
                stencils: assets.add(ShaderBuffer::from(stencils)),
                control: assets.add(ShaderBuffer::from(control)),
                raw: assets.add(ShaderBuffer::from(raw)),
                output: assets.add(ShaderBuffer::from(output)),
                contour,
                sample_stride: far_field_sample_stride(period, time_step),
                canonical: false,
            }
        };
        if let Some(entity) = self.far_field_readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.far_field_revision = self.far_field_revision.wrapping_add(1).max(1);
        self.far_field_readback_entity = Some(
            commands
                .spawn((
                    PacedReadback::continuous(Readback::buffer(handles.output.clone())),
                    FarFieldReadbackTag {
                        generation: self.generation,
                        revision: self.far_field_revision,
                    },
                ))
                .id(),
        );
        self.far_field = Some(handles);
        Ok(if kept {
            FarFieldHandoff::Kept
        } else {
            FarFieldHandoff::Restarted
        })
    }

    pub fn update_canonical_far_field(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        input: Option<&FarFieldInput>,
        time_step: f64,
        history: RecorderHistory,
    ) -> Result<FarFieldHandoff, String> {
        let Some(input) = input else {
            self.clear_far_field_buffers(assets, commands);
            return Ok(FarFieldHandoff::Off);
        };
        if input.samples.len() != FAR_FIELD_CONTOUR_POINTS
            || !input.wave_speed.is_finite()
            || input.wave_speed <= 0.0
            || !input.sample_spacing.is_finite()
            || input.sample_spacing <= 0.0
            || !input.delay_margin.is_finite()
            || input.delay_margin <= 0.0
            || !time_step.is_finite()
            || time_step <= 0.0
        {
            self.clear_far_field_buffers(assets, commands);
            return Err("Invalid far-field recorder settings".into());
        }
        let period = far_field_period(time_step);
        let contour = FarFieldContour {
            points: input
                .samples
                .iter()
                .map(|(_, position, normal)| (*position, *normal))
                .collect(),
            wave_speed: input.wave_speed,
            sample_spacing: input.sample_spacing,
            delay_margin: input.delay_margin,
            period,
        };
        let stencils = input
            .samples
            .iter()
            .map(|(stencil, position, normal)| {
                let point = gpu_probe_stencil(Some(*stencil));
                GpuCanonicalFarFieldStencil {
                    nodes_a: point.nodes_a,
                    nodes_b: point.nodes_b,
                    weights_a: point.weights_a,
                    weights_b: point.weights_b,
                    gradient_x_a: point.gradient_x_a,
                    gradient_x_b: point.gradient_x_b,
                    gradient_y_a: point.gradient_y_a,
                    gradient_y_b: point.gradient_y_b,
                    position_normal: Vec4::new(
                        position.x as f32,
                        position.y as f32,
                        normal.x as f32,
                        normal.y as f32,
                    ),
                }
            })
            .collect::<Vec<_>>();
        let maximum_history = (FAR_FIELD_RING_FRAMES - 2) as f64 * period;
        if input.history_seconds() > maximum_history {
            self.clear_far_field_buffers(assets, commands);
            return Err(format!(
                "Exterior wave speed is too low for the {:.1} s far-field delay window",
                maximum_history
            ));
        }
        let control = GpuFarFieldControl {
            sampling: Vec4::new(
                period as f32,
                FAR_FIELD_RING_FRAMES as f32,
                FAR_FIELD_CONTOUR_POINTS as f32,
                FAR_FIELD_DIRECTIONS as f32,
            ),
            projection: Vec4::new(
                input.wave_speed as f32,
                input.sample_spacing as f32,
                input.delay_margin as f32,
                0.0,
            ),
        };
        let kept = history == RecorderHistory::Keep
            && self
                .far_field
                .as_ref()
                .is_some_and(|handles| handles.contour == contour && handles.canonical);
        let handles = if kept {
            let previous = self.far_field.take().expect("a kept ring exists");
            assets.remove(previous.stencils.id());
            assets.remove(previous.control.id());
            FarFieldBufferHandles {
                stencils: assets.add(ShaderBuffer::from(stencils)),
                control: assets.add(ShaderBuffer::from(control)),
                contour,
                sample_stride: far_field_sample_stride(period, time_step),
                canonical: true,
                ..previous
            }
        } else {
            self.clear_far_field_buffers(assets, commands);
            let raw = vec![
                GpuProbeSample {
                    values: Vec4::new(0.0, 0.0, 0.0, FAR_FIELD_UNRECORDED),
                };
                FAR_FIELD_RING_FRAMES * FAR_FIELD_CONTOUR_POINTS
            ];
            let output = vec![
                GpuProbeSample {
                    values: Vec4::new(0.0, 0.0, FAR_FIELD_UNRECORDED, 0.0),
                };
                FAR_FIELD_RING_FRAMES * FAR_FIELD_DIRECTIONS
            ];
            FarFieldBufferHandles {
                stencils: assets.add(ShaderBuffer::from(stencils)),
                control: assets.add(ShaderBuffer::from(control)),
                raw: assets.add(ShaderBuffer::from(raw)),
                output: assets.add(ShaderBuffer::from(output)),
                contour,
                sample_stride: far_field_sample_stride(period, time_step),
                canonical: true,
            }
        };
        if let Some(entity) = self.far_field_readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.far_field_revision = self.far_field_revision.wrapping_add(1).max(1);
        self.far_field_readback_entity = Some(
            commands
                .spawn((
                    PacedReadback::continuous(Readback::buffer(handles.output.clone())),
                    FarFieldReadbackTag {
                        generation: self.generation,
                        revision: self.far_field_revision,
                    },
                ))
                .id(),
        );
        self.far_field = Some(handles);
        Ok(if kept {
            FarFieldHandoff::Kept
        } else {
            FarFieldHandoff::Restarted
        })
    }

    fn clear_far_field_buffers(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
    ) {
        if let Some(handles) = self.far_field.take() {
            for handle in handles.all() {
                assets.remove(handle.id());
            }
        }
        if let Some(entity) = self.far_field_readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.far_field_revision = self.far_field_revision.wrapping_add(1).max(1);
    }

    pub fn ready(&self) -> bool {
        self.buffers.is_some() && self.stats.status.load(Ordering::Relaxed) == STATUS_READY
    }

    pub fn failed(&self) -> bool {
        self.stats.status.load(Ordering::Relaxed) == STATUS_ERROR
    }

    /// Steps asked for so far in this generation. The gap against
    /// `WaveGpuStats::completed_steps` is the solver's outstanding backlog.
    pub fn requested_steps(&self) -> u64 {
        self.desired_steps
    }

    pub fn request_steps(&mut self, count: u64) {
        self.desired_steps = self.desired_steps.saturating_add(count);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn replace_with_volume_sources(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        time_step: f64,
        source: PointSource,
        volume_sources: &CompiledVolumeSources,
    ) -> Result<(), String> {
        let (handles, dof_count) = create_buffers(
            assets,
            mesh,
            operator,
            time_step,
            source,
            volume_sources,
            false,
        )?;
        self.install(assets, commands, handles, dof_count, None);
        Ok(())
    }

    pub fn replace_transferred_with_volume_sources(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        replacement: WaveTransfer<'_>,
        volume_sources: &CompiledVolumeSources,
    ) -> Result<(), String> {
        if self.transfer.is_some() {
            return Err("A wave-state transfer is already pending".into());
        }
        let WaveTransfer {
            source_mesh,
            source_operator,
            target_mesh,
            target_operator,
            target_time_step,
            source,
            map,
        } = replacement;
        let old = self
            .buffers
            .clone()
            .ok_or("The source wave solver is not initialized")?;
        if !map.matches(source_mesh, source_operator, target_mesh, target_operator)
            || self.dof_count as usize != source_operator.degrees_of_freedom()
        {
            return Err("The transfer map does not match the active and candidate meshes".into());
        }
        let preserve_auxiliary = source_operator.outer_boundaries()
            == target_operator.outer_boundaries()
            && source_operator.auxiliary_active() == target_operator.auxiliary_active();
        let entries = map
            .samples()
            .iter()
            .map(|sample| match sample {
                Some(sample) => GpuTransferEntry {
                    indices_a: UVec4::new(
                        sample.nodes[0],
                        sample.nodes[1],
                        sample.nodes[2],
                        sample.nodes[3],
                    ),
                    weights_a: Vec4::new(
                        sample.weights[0] as f32,
                        sample.weights[1] as f32,
                        sample.weights[2] as f32,
                        sample.weights[3] as f32,
                    ),
                    indices_b: UVec4::new(sample.nodes[4], sample.nodes[5], sample.nodes[6], 0),
                    weights_b: Vec4::new(
                        sample.weights[4] as f32,
                        sample.weights[5] as f32,
                        sample.weights[6] as f32,
                        0.0,
                    ),
                    auxiliary: Vec4::new(0.0, 0.0, if preserve_auxiliary { 1.0 } else { 0.0 }, 0.0),
                    ..default()
                },
                None => GpuTransferEntry::default(),
            })
            .collect::<Vec<_>>();
        if entries
            .iter()
            .any(|entry| !entry.weights_a.is_finite() || !entry.weights_b.is_finite())
        {
            return Err("Transfer weights cannot be represented on the GPU".into());
        }
        let (handles, dof_count) = create_buffers(
            assets,
            target_mesh,
            target_operator,
            target_time_step,
            source,
            volume_sources,
            true,
        )?;
        let transfer = WaveTransferHandles {
            old,
            old_dof_count: self.dof_count,
            entries: assets.add(ShaderBuffer::from(entries)),
        };
        self.install(assets, commands, handles, dof_count, Some(transfer));
        Ok(())
    }

    pub fn finish_transfer(&mut self, assets: &mut Assets<ShaderBuffer>) {
        if let Some(transfer) = self.transfer.take() {
            for handle in transfer.old.all() {
                assets.remove(handle.id());
            }
            assets.remove(transfer.entries.id());
        }
    }

    pub fn rollback_transfer(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
    ) -> Result<(), String> {
        self.clear_probe_buffers(assets, commands);
        self.clear_curve_probe_buffers(assets, commands);
        self.clear_area_probe_buffers(assets, commands);
        let transfer = self
            .transfer
            .take()
            .ok_or("There is no wave transfer to roll back")?;
        if let Some(current) = self.buffers.take() {
            for handle in current.all() {
                assets.remove(handle.id());
            }
        }
        assets.remove(transfer.entries.id());
        if let Some(entity) = self.readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.generation = self.generation.wrapping_add(1).max(1);
        self.dof_count = transfer.old_dof_count;
        self.desired_steps = 0;
        self.pulse_serial = 0;
        self.buffer_revision = self.buffer_revision.wrapping_add(1);
        self.stats = Arc::new(WaveGpuStats::default());
        self.readback_entity = Some(
            commands
                .spawn((
                    PacedReadback::continuous(Readback::buffer(transfer.old.state.clone())),
                    WaveReadbackTag {
                        generation: self.generation,
                        stats: self.stats.clone(),
                        expects_transfer: false,
                    },
                ))
                .id(),
        );
        self.buffers = Some(transfer.old);
        Ok(())
    }

    fn install(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        handles: WaveBufferHandles,
        dof_count: u32,
        transfer: Option<WaveTransferHandles>,
    ) {
        // Every recorder outlives the buffers it was sampled through. What they
        // hold is the field at fixed world points on a clock the transfer
        // carries across, so the readback that is already in flight still
        // lands, and `update_*_probes` decides what the next mesh inherits.
        // Nothing samples into them meanwhile, because a recorder pass is only
        // ever encoded inside the step loop and no step is encoded until the
        // handoff commits.
        let expects_transfer = transfer.is_some();
        if let Some(entity) = self.readback_entity.take() {
            commands.entity(entity).despawn();
        }
        if transfer.is_none() {
            if let Some(old) = self.buffers.take() {
                for handle in old.all() {
                    assets.remove(handle.id());
                }
            }
            if let Some(old_transfer) = self.transfer.take() {
                for handle in old_transfer.old.all() {
                    assets.remove(handle.id());
                }
                assets.remove(old_transfer.entries.id());
            }
        }
        self.generation = self.generation.wrapping_add(1).max(1);
        self.dof_count = dof_count;
        self.desired_steps = 0;
        self.pulse_serial = 0;
        self.buffer_revision = self.buffer_revision.wrapping_add(1);
        self.stats = Arc::new(WaveGpuStats::default());
        let readback_entity = commands
            .spawn((
                PacedReadback::continuous(Readback::buffer(handles.state.clone())),
                WaveReadbackTag {
                    generation: self.generation,
                    stats: self.stats.clone(),
                    expects_transfer,
                },
            ))
            .id();
        self.readback_entity = Some(readback_entity);
        self.buffers = Some(handles);
        self.transfer = transfer;
    }

    pub fn transfer_pending(&self) -> bool {
        self.transfer.is_some()
    }

    pub fn caught_up(&self) -> bool {
        self.desired_steps == self.stats.completed_steps()
    }

    /// Nodal volume acceleration for the same source represented by the GPU
    /// forcing buffer. Boundary forcing is accounted for by the estimator's
    /// interior flux terms and is deliberately absent here.
    pub fn volume_acceleration(&self, time: f64) -> Option<Vec<f64>> {
        let handles = self.buffers.as_ref()?;
        let source = handles.source;
        let value = if source.enabled {
            source.signal.value(time)
        } else {
            0.0
        };
        let volume = handles.volume_sources.acceleration(time);
        Some(
            handles
                .source_weights
                .iter()
                .zip(volume)
                .map(|(weight, volume)| value * *weight as f64 + volume)
                .collect(),
        )
    }

    pub fn reset(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        time_step: f64,
        source: PointSource,
    ) -> Result<(), String> {
        let volume_sources = self
            .buffers
            .as_ref()
            .map(|handles| handles.volume_sources.as_ref().clone())
            .unwrap_or_else(|| CompiledVolumeSources::empty(operator.degrees_of_freedom()));
        self.replace_with_volume_sources(
            assets,
            commands,
            mesh,
            operator,
            time_step,
            source,
            &volume_sources,
        )
    }

    fn validate_inputs(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        time_step: f64,
    ) -> Result<(), String> {
        if mesh.geometry_revision != operator.geometry_revision()
            || mesh.mesh_revision != operator.mesh_revision()
            || mesh.triangles.len() != operator.element_nodes().len()
        {
            return Err("Mesh and quadratic wave operator do not agree".into());
        }
        let dt = time_step as f32;
        if !dt.is_finite() || dt <= 0.0 {
            return Err("The wave time step cannot be represented on the GPU".into());
        }
        Ok(())
    }

    pub fn update_source(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        source: PointSource,
    ) -> Result<(), String> {
        let handles = self
            .buffers
            .as_mut()
            .ok_or("The wave solver is not initialized")?;
        let spatial_changed = !handles.source.spatial_eq(source);
        let region_changed = handles.source.region != source.region;
        let source_weights = if spatial_changed {
            forcing_weights(mesh, operator, source.position, source.width, source.region)?
        } else {
            handles.source_weights.to_vec()
        };
        let nodes = region_changed
            .then(|| gpu_nodes(mesh, operator, source.region))
            .transpose()?;
        let forcing = gpu_forcing(
            source,
            handles.pulse,
            operator.outer_boundaries(),
            &handles.volume_sources,
        )?;
        let weights = spatial_changed
            .then(|| {
                zip_forcing_weights(
                    &source_weights,
                    &handles.pulse_weights,
                    &handles.volume_sources,
                )
            })
            .transpose()?;
        let forcing = assets.add(ShaderBuffer::from(forcing));
        let old = std::mem::replace(&mut handles.forcing, forcing);
        assets.remove(old.id());
        if let Some(weights) = weights {
            let weights = assets.add(ShaderBuffer::from(weights));
            let old = std::mem::replace(&mut handles.forcing_weights, weights);
            assets.remove(old.id());
        }
        if let Some(nodes) = nodes {
            let nodes = assets.add(ShaderBuffer::from(nodes));
            let old = std::mem::replace(&mut handles.nodes, nodes);
            assets.remove(old.id());
        }
        handles.source = source;
        handles.source_weights = source_weights.into();
        self.buffer_revision = self.buffer_revision.wrapping_add(1);
        Ok(())
    }

    pub fn update_volume_sources(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        operator: &QuadraticWaveOperator,
        volume_sources: CompiledVolumeSources,
    ) -> Result<(), String> {
        if volume_sources.nodes.len() != operator.degrees_of_freedom()
            || volume_sources.signals.len() > MAX_VOLUME_SOURCES
        {
            return Err("Compiled volume sources do not match the wave discretization".into());
        }
        let handles = self
            .buffers
            .as_mut()
            .ok_or("The wave solver is not initialized")?;
        let forcing = assets.add(ShaderBuffer::from(gpu_forcing(
            handles.source,
            handles.pulse,
            operator.outer_boundaries(),
            &volume_sources,
        )?));
        let weights = assets.add(ShaderBuffer::from(zip_forcing_weights(
            &handles.source_weights,
            &handles.pulse_weights,
            &volume_sources,
        )?));
        let old = std::mem::replace(&mut handles.forcing, forcing);
        assets.remove(old.id());
        let old = std::mem::replace(&mut handles.forcing_weights, weights);
        assets.remove(old.id());
        handles.volume_sources = Arc::new(volume_sources);
        self.buffer_revision = self.buffer_revision.wrapping_add(1);
        Ok(())
    }

    pub fn update_volume_source_signals(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        operator: &QuadraticWaveOperator,
        signals: Vec<TimeSignal>,
    ) -> Result<(), String> {
        let handles = self
            .buffers
            .as_mut()
            .ok_or("The wave solver is not initialized")?;
        if signals.len() != handles.volume_sources.signals.len()
            || signals.len() > MAX_VOLUME_SOURCES
            || signals.iter().any(|signal| !signal.valid())
        {
            return Err("Volume-source signals do not match the compiled carriers".into());
        }
        let volume_sources = CompiledVolumeSources {
            signals,
            nodes: handles.volume_sources.nodes.clone(),
        };
        let forcing = assets.add(ShaderBuffer::from(gpu_forcing(
            handles.source,
            handles.pulse,
            operator.outer_boundaries(),
            &volume_sources,
        )?));
        let old = std::mem::replace(&mut handles.forcing, forcing);
        assets.remove(old.id());
        handles.volume_sources = Arc::new(volume_sources);
        self.buffer_revision = self.buffer_revision.wrapping_add(1);
        Ok(())
    }

    pub fn inject_pulse(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        pulse_settings: PulseSettings,
    ) -> Result<(), String> {
        let handles = self
            .buffers
            .as_mut()
            .ok_or("The wave solver is not initialized")?;
        let pulse = gpu_pulse(pulse_settings);
        if !pulse.position_width_amplitude.is_finite() || pulse_settings.width <= 0.0 {
            return Err("Pulse parameters must be finite with positive width".into());
        }
        let pulse_weights = forcing_weights(
            mesh,
            operator,
            pulse_settings.position,
            pulse_settings.width as f64,
            pulse_settings.region,
        )?;
        let forcing = assets.add(ShaderBuffer::from(gpu_forcing(
            handles.source,
            pulse_settings,
            operator.outer_boundaries(),
            &handles.volume_sources,
        )?));
        let weights = assets.add(ShaderBuffer::from(zip_forcing_weights(
            &handles.source_weights,
            &pulse_weights,
            &handles.volume_sources,
        )?));
        let old = std::mem::replace(&mut handles.forcing, forcing);
        assets.remove(old.id());
        let old = std::mem::replace(&mut handles.forcing_weights, weights);
        assets.remove(old.id());
        handles.pulse = pulse_settings;
        handles.pulse_weights = pulse_weights.into();
        self.pulse_serial = self.pulse_serial.wrapping_add(1).max(1);
        self.buffer_revision = self.buffer_revision.wrapping_add(1);
        Ok(())
    }
}

fn create_buffers(
    assets: &mut Assets<ShaderBuffer>,
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
    time_step: f64,
    source: PointSource,
    volume_sources: &CompiledVolumeSources,
    awaits_transfer: bool,
) -> Result<(WaveBufferHandles, u32), String> {
    WaveGpuRequest::validate_inputs(mesh, operator, time_step)?;
    if volume_sources.nodes.len() != operator.degrees_of_freedom()
        || volume_sources.signals.len() > MAX_VOLUME_SOURCES
    {
        return Err("Compiled volume sources do not match the wave discretization".into());
    }
    let dof_count = u32::try_from(operator.degrees_of_freedom())
        .map_err(|_| "The wave discretization is too large for the GPU")?;
    let normalized = operator
        .normalized_stiffness_f32()
        .map_err(|error| error.to_string())?;
    let normalized_auxiliary = operator
        .normalized_auxiliary_stiffness_f32()
        .map_err(|error| error.to_string())?;
    let matrix = normalized
        .into_iter()
        .zip(normalized_auxiliary)
        .map(|(stiffness, auxiliary)| GpuMatrixEntry {
            coefficients: Vec2::new(stiffness, auxiliary),
        })
        .collect::<Vec<_>>();
    let damping = operator
        .damping_ratios_f32()
        .map_err(|error| error.to_string())?;
    let dt = time_step as f32;
    let nodes = gpu_nodes_with_damping(mesh, operator, source.region, &damping)?;
    let parameters = GpuParameters {
        time_data: Vec4::new(dt, dt * dt, 0.0, 0.0),
        count_data: UVec4::new(dof_count, 0, 0, 0),
        filter_data: Vec4::new(
            (1.0 / operator.maximum_eigenvalue_bound()) as f32,
            GRID_SCALE_FILTER_STRENGTH as f32,
            0.0,
            0.0,
        ),
    };
    let pulse = PulseSettings {
        position: Point2::default(),
        amplitude: 0.65,
        width: 0.06,
        region: funfern_core::BACKGROUND_REGION,
    };
    let source_weights =
        forcing_weights(mesh, operator, source.position, source.width, source.region)?;
    let pulse_weights = forcing_weights(
        mesh,
        operator,
        Point2::default(),
        0.06_f64,
        funfern_core::BACKGROUND_REGION,
    )?;
    let initial =
        QuadraticWaveState::zero(operator, time_step).map_err(|error| error.to_string())?;
    let initial_states = initial
        .current()
        .iter()
        .zip(initial.previous())
        .zip(initial.auxiliary())
        .map(|((current, previous), auxiliary)| GpuState {
            levels: Vec4::new(
                *previous as f32,
                *current as f32,
                *current as f32,
                if awaits_transfer { -1.0 } else { 0.0 },
            ),
            auxiliary: Vec4::new(*auxiliary as f32, 0.0, 0.0, *current as f32),
            reconstruction: Vec4::ZERO,
        })
        .collect::<Vec<_>>();
    Ok((
        WaveBufferHandles {
            parameters: assets.add(ShaderBuffer::from(parameters)),
            forcing: assets.add(ShaderBuffer::from(gpu_forcing(
                source,
                pulse,
                operator.outer_boundaries(),
                volume_sources,
            )?)),
            row_offsets: assets.add(ShaderBuffer::from(operator.row_offsets().to_vec())),
            columns: assets.add(ShaderBuffer::from(operator.columns().to_vec())),
            stiffness: assets.add(ShaderBuffer::from(matrix)),
            nodes: assets.add(ShaderBuffer::from(nodes)),
            state: assets.add(ShaderBuffer::from(initial_states)),
            forcing_weights: assets.add(ShaderBuffer::from(zip_forcing_weights(
                &source_weights,
                &pulse_weights,
                volume_sources,
            )?)),
            source,
            pulse,
            source_weights: source_weights.into(),
            pulse_weights: pulse_weights.into(),
            volume_sources: Arc::new(volume_sources.clone()),
        },
        dof_count,
    ))
}

fn gpu_source(source: PointSource) -> GpuSource {
    GpuSource {
        position_width_enabled: Vec4::new(
            source.position.x as f32,
            source.position.y as f32,
            (source.width * source.width) as f32,
            if source.enabled { 1.0 } else { 0.0 },
        ),
        signal: gpu_time_signal(source.signal),
        region: gpu_region_pair(source.region, RegionId(0)),
    }
}

fn gpu_pulse(pulse: PulseSettings) -> GpuPulse {
    GpuPulse {
        position_width_amplitude: Vec4::new(
            pulse.position.x as f32,
            pulse.position.y as f32,
            pulse.width * pulse.width,
            pulse.amplitude,
        ),
        region: gpu_region_pair(pulse.region, RegionId(0)),
    }
}

fn gpu_forcing(
    source: PointSource,
    pulse: PulseSettings,
    boundaries: OuterBoundaryConditions,
    volume_sources: &CompiledVolumeSources,
) -> Result<GpuForcing, String> {
    let signals = boundaries
        .sides
        .map(|condition| gpu_boundary_signal(condition.signal().unwrap_or(TimeSignal::ZERO)));
    if signals.iter().any(|values| !values.is_finite()) {
        return Err("Boundary signal values cannot be represented on the GPU".into());
    }
    let mut volume = [GpuTimeSignal::default(); MAX_VOLUME_SOURCES];
    for (target, signal) in volume.iter_mut().zip(&volume_sources.signals) {
        *target = gpu_boundary_signal(*signal);
        if !target.is_finite() {
            return Err("Volume-source signal values cannot be represented on the GPU".into());
        }
    }
    // One envelope for the whole scene, sized on its slowest oscillating source
    // so every source is eased in over at least that many of its own periods.
    // A disabled point source is left out; a compiled volume channel is on by
    // construction.
    let lowest = std::iter::once(source.signal)
        .filter(|_| source.enabled)
        .chain(volume_sources.signals.iter().copied())
        .map(TimeSignal::frequency_ceiling_hz)
        .filter(|frequency| *frequency > 0.0)
        .fold(f64::INFINITY, f64::min);
    Ok(GpuForcing {
        source: gpu_source(source),
        pulse: gpu_pulse(pulse),
        outer: signals,
        volume,
        envelope: Vec4::new(source_ramp_seconds(lowest) as f32, 0.0, 0.0, 0.0),
    })
}

fn gpu_boundary_signal(signal: TimeSignal) -> GpuTimeSignal {
    gpu_time_signal(signal)
}

fn gpu_time_signal(signal: TimeSignal) -> GpuTimeSignal {
    let [offset, amplitude, frequency_hz, phase_radians] = signal.harmonic_parameters();
    GpuTimeSignal {
        values: Vec4::new(
            offset as f32,
            amplitude as f32,
            (std::f64::consts::TAU * frequency_hz) as f32,
            phase_radians as f32,
        ),
        extra: Vec4::ZERO,
    }
}

fn zip_forcing_weights(
    source: &[f32],
    pulse: &[f32],
    volume_sources: &CompiledVolumeSources,
) -> Result<Vec<GpuForcingWeightWord>, String> {
    if source.len() != pulse.len() || source.len() != volume_sources.nodes.len() {
        return Err("Source and pulse weights do not match the wave discretization".into());
    }
    let header_count = source.len();
    let contribution_count = volume_sources
        .nodes
        .iter()
        .map(|node| node.contributions.len().div_ceil(2))
        .sum::<usize>();
    let mut packed = vec![GpuForcingWeightWord::default(); header_count];
    packed.reserve(contribution_count);
    for (index, ((&source, &pulse), node)) in source
        .iter()
        .zip(pulse)
        .zip(&volume_sources.nodes)
        .enumerate()
    {
        let offset = u32::try_from(packed.len())
            .map_err(|_| "Volume-source weights are too large for the GPU")?;
        let count = u32::try_from(node.contributions.len())
            .map_err(|_| "Too many volume sources meet at a wave node")?;
        if !source.is_finite() || !pulse.is_finite() {
            return Err("Point-source weights cannot be represented on the GPU".into());
        }
        packed[index].data = UVec4::new(source.to_bits(), pulse.to_bits(), offset, count);
        for pair in node.contributions.chunks(2) {
            let mut data = [0_u32; 4];
            for (slot, contribution) in pair.iter().enumerate() {
                if contribution.channel as usize >= volume_sources.signals.len() {
                    return Err("Volume-source channel is out of range".into());
                }
                let weight = contribution.weight as f32;
                if !weight.is_finite() {
                    return Err("Volume-source weights cannot be represented on the GPU".into());
                }
                data[slot * 2] = contribution.channel + 1;
                data[slot * 2 + 1] = weight.to_bits();
            }
            packed.push(GpuForcingWeightWord {
                data: UVec4::from_array(data),
            });
        }
    }
    Ok(packed)
}

fn node_regions(
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
) -> Result<Vec<Vec<RegionId>>, String> {
    let mut regions = vec![vec![]; operator.degrees_of_freedom()];
    for (triangle, nodes) in mesh.triangles.iter().zip(operator.element_nodes()) {
        for node in nodes {
            let assigned = &mut regions[*node as usize];
            if !assigned.contains(&triangle.region) {
                assigned.push(triangle.region);
            }
        }
    }
    if regions.iter().any(Vec::is_empty) {
        return Err("A wave node does not belong to a material region".into());
    }
    Ok(regions)
}

fn gpu_region_pair(first: RegionId, second: RegionId) -> UVec4 {
    UVec4::new(
        first.0 as u32,
        (first.0 >> 32) as u32,
        second.0 as u32,
        (second.0 >> 32) as u32,
    )
}

fn gpu_nodes(
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
    source_region: RegionId,
) -> Result<Vec<GpuNode>, String> {
    let damping = operator
        .damping_ratios_f32()
        .map_err(|error| error.to_string())?;
    gpu_nodes_with_damping(mesh, operator, source_region, &damping)
}

fn gpu_nodes_with_damping(
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
    source_region: RegionId,
    damping: &[f32],
) -> Result<Vec<GpuNode>, String> {
    let regions = node_regions(mesh, operator)?;
    if damping.len() != operator.degrees_of_freedom() {
        return Err("Wave damping does not match the wave discretization".into());
    }
    let nodes = operator
        .node_points()
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let dirichlet = operator.dirichlet_signals()[index];
            let face_loads = operator.face_neumann_loads()[index];
            GpuNode {
                position_damping: Vec4::new(
                    point.x as f32,
                    point.y as f32,
                    damping[index],
                    if operator.auxiliary_active()[index] {
                        1.0
                    } else {
                        0.0
                    },
                ),
                source_membership: UVec4::new(
                    u32::from(regions[index].contains(&source_region)),
                    0,
                    0,
                    0,
                ),
                boundary: UVec4::new(u32::from(dirichlet.is_some()), 0, 0, 0),
                neumann_weights: Vec4::from_array(
                    operator.normalized_neumann_weights()[index].map(|value| value as f32),
                ),
                dirichlet_signal: gpu_boundary_signal(dirichlet.unwrap_or(TimeSignal::ZERO)),
                face_neumann_signal_a: gpu_boundary_signal(face_loads[0].signal),
                face_neumann_signal_b: gpu_boundary_signal(face_loads[1].signal),
                face_neumann_weights: Vec4::new(
                    face_loads[0].normalized_weight as f32,
                    face_loads[1].normalized_weight as f32,
                    0.0,
                    0.0,
                ),
            }
        })
        .collect::<Vec<_>>();
    if nodes
        .iter()
        .any(|node| !node.position_damping.x.is_finite() || !node.position_damping.y.is_finite())
    {
        return Err("Mesh coordinates cannot be represented on the GPU".into());
    }
    Ok(nodes)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DistanceNode {
    distance: f64,
    node: usize,
}

impl Eq for DistanceNode {}

impl Ord for DistanceNode {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        other
            .distance
            .total_cmp(&self.distance)
            .then_with(|| other.node.cmp(&self.node))
    }
}

impl PartialOrd for DistanceNode {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

/// Builds a compact Gaussian stencil. With an open internal boundary, graph
/// distance follows the actual cut mesh, so forcing reaches the opposite face
/// only by going around a free endpoint.
pub(crate) fn forcing_weights(
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
    position: Point2,
    width: f64,
    region: RegionId,
) -> Result<Vec<f32>, String> {
    if !position.finite() || !width.is_finite() || width <= 0.0 {
        return Err("Source parameters must be finite with positive width".into());
    }
    WaveGpuRequest::validate_inputs(mesh, operator, operator.recommended_time_step())?;
    let memberships = node_regions(mesh, operator)?;
    let eligible = memberships
        .iter()
        .map(|regions| regions.contains(&region))
        .collect::<Vec<_>>();
    if !mesh.boundary_edges.iter().any(|edge| {
        matches!(
            edge.label,
            funfern_core::BoundaryLabel::InternalBoundary { .. }
                | funfern_core::BoundaryLabel::Curve {
                    separated: true,
                    ..
                }
        )
    }) {
        return Ok(operator
            .node_points()
            .iter()
            .zip(eligible)
            .map(|(point, eligible)| {
                if eligible {
                    (-0.5 * (*point - position).dot(*point - position) / (width * width)).exp()
                        as f32
                } else {
                    0.0
                }
            })
            .collect());
    }

    let source_triangle = mesh
        .triangles
        .iter()
        .enumerate()
        .find_map(|(index, triangle)| {
            if triangle.region != region {
                return None;
            }
            let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
            ((b - a).cross(position - a) >= -1.0e-12
                && (c - b).cross(position - b) >= -1.0e-12
                && (a - c).cross(position - c) >= -1.0e-12)
                .then_some(index)
        });
    let Some(source_triangle) = source_triangle else {
        return Ok(vec![0.0; operator.degrees_of_freedom()]);
    };
    let mut adjacency = vec![Vec::<(usize, f64)>::new(); operator.degrees_of_freedom()];
    for (triangle, nodes) in mesh.triangles.iter().zip(operator.element_nodes()) {
        if triangle.region != region {
            continue;
        }
        for first in 0..nodes.len() {
            for second in first + 1..nodes.len() {
                let a = nodes[first] as usize;
                let b = nodes[second] as usize;
                let distance = (operator.node_points()[a] - operator.node_points()[b]).norm();
                adjacency[a].push((b, distance));
                adjacency[b].push((a, distance));
            }
        }
    }
    let mut distances = vec![f64::INFINITY; operator.degrees_of_freedom()];
    let mut pending = BinaryHeap::new();
    for node in operator.element_nodes()[source_triangle] {
        let node = node as usize;
        let distance = (operator.node_points()[node] - position).norm();
        distances[node] = distance;
        pending.push(DistanceNode { distance, node });
    }
    let cutoff = 6.0 * width;
    while let Some(DistanceNode { distance, node }) = pending.pop() {
        if distance != distances[node] || distance > cutoff {
            continue;
        }
        for &(neighbor, length) in &adjacency[node] {
            let candidate = distance + length;
            if candidate < distances[neighbor] && candidate <= cutoff {
                distances[neighbor] = candidate;
                pending.push(DistanceNode {
                    distance: candidate,
                    node: neighbor,
                });
            }
        }
    }
    Ok(distances
        .into_iter()
        .zip(eligible)
        .map(|(distance, eligible)| {
            if eligible && distance.is_finite() {
                (-0.5 * distance * distance / (width * width)).exp() as f32
            } else {
                0.0
            }
        })
        .collect())
}

#[derive(Resource, Default)]
pub struct WaveDisplay {
    pub generation: u64,
    pub current: Vec<f32>,
    pub previous: Vec<f32>,
    pub auxiliary: Vec<f32>,
    /// Centered fields aligned at the time level immediately before the latest
    /// committed step. They reuse spare lanes in the normal state readback.
    pub indicator_displacement: Vec<f32>,
    pub indicator_velocity: Vec<f32>,
    pub indicator_acceleration: Vec<f32>,
    /// Legacy scalar-solver reconstruction lane. The production canonical
    /// adapter leaves it empty; retained only by the optional comparison path.
    pub indicator_potential: Vec<f32>,
    /// Canonical six-sample complementary flux read directly by vector and AMR
    /// consumers. Empty on the optional scalar comparison path.
    pub complementary_flux: Vec<[f32; 2]>,
    /// Primary endpoint pair captured with `complementary_flux` by the latest
    /// aligned full canonical snapshot. Vector and AMR consumers use these
    /// rather than mixing a lower-cadence complementary field with the live
    /// primary display stream.
    pub snapshot_current: Vec<f32>,
    pub snapshot_previous: Vec<f32>,
    pub snapshot_velocity: Vec<f32>,
    pub snapshot_completed_steps: u64,
    pub completed_steps: u64,
    pub readbacks: u64,
}

#[derive(Component)]
struct WaveReadbackTag {
    generation: u64,
    stats: Arc<WaveGpuStats>,
    expects_transfer: bool,
}

#[derive(Component)]
struct ProbeReadbackTag {
    generation: u64,
    revision: u64,
    ids: Arc<[u64]>,
    canonical: bool,
}

#[derive(Clone, Debug)]
pub struct CurveProbeInput {
    pub id: u64,
    pub sample_rate: f64,
    pub samples: Vec<Option<(QuadraticPointStencil, Point2)>>,
}

#[derive(Clone, Debug)]
pub struct AreaProbeInput {
    pub id: u64,
    pub stencil: Option<QuadraticAreaStencil>,
}

#[derive(Clone, Debug)]
pub struct FarFieldInput {
    pub samples: Vec<(QuadraticPointStencil, Point2, Point2)>,
    pub wave_speed: f64,
    pub sample_spacing: f64,
    pub delay_margin: f64,
}

impl FarFieldInput {
    /// The window a projection reads, as `QuadraticFarFieldStencil` reports it
    /// to the panel: a ring shorter than this can never report anything.
    fn history_seconds(&self) -> f64 {
        let reach = self
            .samples
            .iter()
            .map(|(_, position, _)| position.norm())
            .fold(0.0, f64::max);
        self.delay_margin + reach / self.wave_speed
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CurveProbeDescriptor {
    id: u64,
    offset: u32,
    count: u32,
}

#[derive(Component)]
struct CurveProbeReadbackTag {
    generation: u64,
    revision: u64,
    descriptors: Arc<[CurveProbeDescriptor]>,
}

#[derive(Component)]
struct AreaProbeReadbackTag {
    generation: u64,
    revision: u64,
    ids: Arc<[u64]>,
}

#[derive(Component)]
struct FarFieldReadbackTag {
    generation: u64,
    revision: u64,
}

#[derive(Component)]
struct VectorOverlayReadbackTag {
    generation: u64,
    revision: u64,
    sample_count: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointProbeRecord {
    pub probe_id: u64,
    pub time: f64,
    pub displacement: f64,
    pub velocity: f64,
    pub transverse_magnitude: f64,
    pub poynting_magnitude: f64,
    pub energy_density: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CurveProbeRecord {
    pub probe_id: u64,
    pub time: f64,
    pub displacement: Vec<f32>,
    pub transverse_magnitude: Vec<f32>,
    pub energy_density: Vec<f32>,
    pub normal_flux: Vec<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AreaProbeRecord {
    pub probe_id: u64,
    pub time: f64,
    pub mean_displacement: f64,
    pub rms_displacement: f64,
    pub rms_transverse_magnitude: f64,
    pub mean_energy_density: f64,
    pub total_energy: f64,
    pub covered_area: f64,
    pub coverage: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FarFieldRecord {
    pub time: f64,
    pub amplitude: Vec<f32>,
    pub intensity: Vec<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VectorOverlaySample {
    pub complementary: Point2,
    pub energy_flow: Point2,
    /// Complementary field in the other state lane. At a resident-filter
    /// boundary this is the pre-filter value at the same physical time.
    pub pre_filter_complementary: Point2,
}

#[derive(Resource, Default)]
pub struct ProbeDisplay {
    pub generation: u64,
    pub revision: u64,
    pub records: Vec<PointProbeRecord>,
    pub readbacks: u64,
}

#[derive(Resource, Default)]
pub struct CurveProbeDisplay {
    pub generation: u64,
    pub revision: u64,
    pub records: Vec<CurveProbeRecord>,
    pub readbacks: u64,
}

#[derive(Resource, Default)]
pub struct AreaProbeDisplay {
    pub generation: u64,
    pub revision: u64,
    pub records: Vec<AreaProbeRecord>,
    pub readbacks: u64,
}

#[derive(Resource, Default)]
pub struct FarFieldDisplay {
    pub generation: u64,
    pub revision: u64,
    pub records: Vec<FarFieldRecord>,
    pub readbacks: u64,
}

#[derive(Resource, Default)]
pub struct VectorOverlayDisplay {
    pub generation: u64,
    pub revision: u64,
    pub completed_steps: u64,
    pub absolute_time: f64,
    pub samples: Vec<VectorOverlaySample>,
    pub readbacks: u64,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuParameters {
    time_data: Vec4,
    count_data: UVec4,
    /// The grid-scale filter's reciprocal eigenvalue ceiling and its strength.
    /// Both are fixed for the life of a generation; the toggle works by leaving
    /// the two filter dispatches out, not by zeroing this.
    filter_data: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuSource {
    position_width_enabled: Vec4,
    signal: GpuTimeSignal,
    region: UVec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuPulse {
    position_width_amplitude: Vec4,
    region: UVec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuTimeSignal {
    values: Vec4,
    extra: Vec4,
}

impl GpuTimeSignal {
    fn is_finite(self) -> bool {
        self.values.is_finite() && self.extra.is_finite()
    }
}

#[derive(Clone, Copy, ShaderType)]
struct GpuForcing {
    source: GpuSource,
    pulse: GpuPulse,
    outer: [GpuTimeSignal; 4],
    volume: [GpuTimeSignal; MAX_VOLUME_SOURCES],
    /// `x` is how long the shared switch-on envelope runs; the rest is padding.
    envelope: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuNode {
    position_damping: Vec4,
    source_membership: UVec4,
    boundary: UVec4,
    neumann_weights: Vec4,
    dirichlet_signal: GpuTimeSignal,
    face_neumann_signal_a: GpuTimeSignal,
    face_neumann_signal_b: GpuTimeSignal,
    face_neumann_weights: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuMatrixEntry {
    coefficients: Vec2,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuForcingWeightWord {
    data: UVec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuState {
    levels: Vec4,
    auxiliary: Vec4,
    reconstruction: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuProbeStencil {
    nodes_a: UVec4,
    nodes_b: UVec4,
    weights_a: Vec4,
    weights_b: Vec4,
    gradient_x_a: Vec4,
    gradient_x_b: Vec4,
    gradient_y_a: Vec4,
    gradient_y_b: Vec4,
    material: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuProbeControl {
    values: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuProbeSample {
    values: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuPointProbeSample {
    primary: Vec4,
    secondary: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuCanonicalPointProbeSample {
    primary: Vec4,
    secondary: Vec4,
    reserved: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuVectorOverlaySample {
    vectors: Vec4,
    pre_filter_complementary: Vec4,
    metadata: UVec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuCurveProbeSample {
    primary: Vec4,
    secondary: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuCurveProbeStencil {
    nodes_a: UVec4,
    nodes_b: UVec4,
    weights_a: Vec4,
    weights_b: Vec4,
    gradient_x_a: Vec4,
    gradient_x_b: Vec4,
    gradient_y_a: Vec4,
    gradient_y_b: Vec4,
    material: Vec4,
    normal_stride_valid: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuAreaProbeContribution {
    nodes_a: UVec4,
    nodes_b: UVec4,
    field_a: Vec4,
    field_b: Vec4,
    mass: [Vec4; 7],
    density: [Vec4; 7],
    stiffness: [Vec4; 7],
    stiffness_squared: [Vec4; 7],
    material_area: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuAreaProbeDescriptor {
    offset_count: UVec4,
    areas: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuAreaProbeSample {
    primary: Vec4,
    secondary: Vec4,
    tertiary: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuAreaProbeContributionSample {
    primary: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuFarFieldStencil {
    nodes_a: UVec4,
    nodes_b: UVec4,
    weights_a: Vec4,
    weights_b: Vec4,
    gradient_x_a: Vec4,
    gradient_x_b: Vec4,
    gradient_y_a: Vec4,
    gradient_y_b: Vec4,
    position_normal: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuFarFieldControl {
    sampling: Vec4,
    projection: Vec4,
}

/// Direct-state point reconstruction. Primary weights act on `Q / M`; the
/// complementary weights reconstruct the six independent quadrature samples
/// belonging to one element.
#[derive(Clone, Copy, Default, ShaderType)]
struct GpuCanonicalPointStencil {
    nodes_a: UVec4,
    nodes_b: UVec4,
    primary_a: Vec4,
    primary_b: Vec4,
    complementary_a: Vec4,
    complementary_b: Vec4,
    sample_valid: UVec4,
    reference_inverse: Vec4,
    orientation: Vec4,
    temporal_primary_a: UVec4,
    temporal_primary_b: UVec4,
    temporal_complementary: UVec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuCanonicalCurveStencil {
    point: GpuCanonicalPointStencil,
    normal_stride_valid: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuCanonicalAreaQuadrature {
    primary_a: Vec4,
    primary_b: Vec4,
    complementary_a: Vec4,
    complementary_b: Vec4,
    reference_inverse: Vec4,
    weight: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuCanonicalAreaContribution {
    nodes_a: UVec4,
    nodes_b: UVec4,
    sample_valid: UVec4,
    quadrature: [GpuCanonicalAreaQuadrature; 12],
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuCanonicalFarFieldStencil {
    nodes_a: UVec4,
    nodes_b: UVec4,
    weights_a: Vec4,
    weights_b: Vec4,
    gradient_x_a: Vec4,
    gradient_x_b: Vec4,
    gradient_y_a: Vec4,
    gradient_y_b: Vec4,
    position_normal: Vec4,
}

fn gpu_canonical_point_stencil(stencil: Option<CanonicalPointStencil>) -> GpuCanonicalPointStencil {
    let Some(stencil) = stencil else {
        return GpuCanonicalPointStencil::default();
    };
    let primary = stencil.primary_weights.map(|value| value as f32);
    let complementary = stencil.complementary_weights.map(|value| value as f32);
    GpuCanonicalPointStencil {
        nodes_a: UVec4::new(
            stencil.nodes[0],
            stencil.nodes[1],
            stencil.nodes[2],
            stencil.nodes[3],
        ),
        nodes_b: UVec4::new(stencil.nodes[4], stencil.nodes[5], stencil.nodes[6], 0),
        primary_a: Vec4::from_array([primary[0], primary[1], primary[2], primary[3]]),
        primary_b: Vec4::from_array([primary[4], primary[5], primary[6], 0.0]),
        complementary_a: Vec4::from_array([
            complementary[0],
            complementary[1],
            complementary[2],
            complementary[3],
        ]),
        complementary_b: Vec4::from_array([complementary[4], complementary[5], 0.0, 0.0]),
        sample_valid: UVec4::new(stencil.complementary_samples[0], 1, 0, 0),
        reference_inverse: Vec4::new(
            stencil.primary_reference as f32,
            stencil.complementary_inverse.xx as f32,
            stencil.complementary_inverse.xy as f32,
            stencil.complementary_inverse.yy as f32,
        ),
        orientation: Vec4::new(stencil.orientation as f32, 0.0, 0.0, 0.0),
        temporal_primary_a: UVec4::ZERO,
        temporal_primary_b: UVec4::ZERO,
        temporal_complementary: UVec4::ZERO,
    }
}

fn gpu_temporal_canonical_point_stencil(
    stencil: CanonicalTemporalPointStencil,
    operator: &CanonicalTemporalWaveOperator,
    manifest: CanonicalGpuTemporalManifest,
) -> Result<GpuCanonicalPointStencil, String> {
    let coefficient_words = manifest.coefficient_words;
    if coefficient_words == 0
        || manifest.primary_record_count != operator.base().primary_contributions().len()
        || manifest.complementary_record_count != operator.base().complementary_degrees_of_freedom()
    {
        return Err("Temporal point reconstruction does not match the GPU table layout".into());
    }
    let element = stencil.element();
    let contributions = operator.base().primary_contributions();
    let mut primary_words = [0_u32; 7];
    for (local_node, word) in primary_words.iter_mut().enumerate() {
        let contribution_index = contributions
            .iter()
            .position(|contribution| {
                contribution.element == element && contribution.local_node as usize == local_node
            })
            .ok_or_else(|| {
                format!(
                    "Temporal point element {element} has no primary contribution for local node {local_node}"
                )
            })?;
        let contribution = contributions[contribution_index];
        let record_index = contributions[..contribution_index]
            .iter()
            .filter(|candidate| candidate.node == contribution.node)
            .count()
            + contributions
                .iter()
                .filter(|candidate| candidate.node < contribution.node)
                .count();
        let absolute_word = manifest
            .primary_record_offset
            .checked_add(
                record_index
                    .checked_mul(coefficient_words)
                    .ok_or_else(|| "Temporal point primary table address overflowed".to_string())?,
            )
            .ok_or_else(|| "Temporal point primary table address overflowed".to_string())?;
        *word = u32::try_from(absolute_word)
            .map_err(|_| "Temporal point primary table address exceeds u32".to_string())?;
    }
    let complementary_record = usize::try_from(element)
        .ok()
        .and_then(|element| element.checked_mul(6))
        .ok_or_else(|| "Temporal point complementary table address overflowed".to_string())?;
    let complementary_word = manifest
        .complementary_record_offset
        .checked_add(
            complementary_record
                .checked_mul(coefficient_words)
                .ok_or_else(|| {
                    "Temporal point complementary table address overflowed".to_string()
                })?,
        )
        .ok_or_else(|| "Temporal point complementary table address overflowed".to_string())?;
    let mut result = gpu_canonical_point_stencil(Some(stencil.fixed()));
    result.sample_valid.z = 1;
    result.temporal_primary_a = UVec4::from_array(primary_words[..4].try_into().unwrap());
    result.temporal_primary_b =
        UVec4::from_array([primary_words[4], primary_words[5], primary_words[6], 0]);
    result.temporal_complementary = UVec4::new(
        u32::try_from(complementary_word)
            .map_err(|_| "Temporal point complementary table address exceeds u32".to_string())?,
        0,
        0,
        0,
    );
    Ok(result)
}

fn gpu_canonical_area_contribution(
    element: QuadraticAreaElement,
    operator: &CanonicalWaveOperator,
) -> Result<GpuCanonicalAreaContribution, String> {
    let quadrature = canonical_area_quadrature(element, operator)
        .map_err(|error| format!("Canonical area reconstruction failed: {error}"))?;
    Ok(GpuCanonicalAreaContribution {
        nodes_a: UVec4::new(
            element.nodes[0],
            element.nodes[1],
            element.nodes[2],
            element.nodes[3],
        ),
        nodes_b: UVec4::new(element.nodes[4], element.nodes[5], element.nodes[6], 0),
        sample_valid: UVec4::new(element.element * 6, 1, 0, 0),
        quadrature: quadrature.map(|point| {
            let primary = point.primary_weights.map(|value| value as f32);
            let complementary = point.complementary_weights.map(|value| value as f32);
            GpuCanonicalAreaQuadrature {
                primary_a: Vec4::from_array([primary[0], primary[1], primary[2], primary[3]]),
                primary_b: Vec4::from_array([primary[4], primary[5], primary[6], 0.0]),
                complementary_a: Vec4::from_array([
                    complementary[0],
                    complementary[1],
                    complementary[2],
                    complementary[3],
                ]),
                complementary_b: Vec4::from_array([complementary[4], complementary[5], 0.0, 0.0]),
                reference_inverse: Vec4::new(
                    point.primary_reference as f32,
                    point.complementary_inverse.xx as f32,
                    point.complementary_inverse.xy as f32,
                    point.complementary_inverse.yy as f32,
                ),
                weight: Vec4::new(point.physical_weight as f32, 0.0, 0.0, 0.0),
            }
        }),
    })
}

fn gpu_probe_stencil(stencil: Option<QuadraticPointStencil>) -> GpuProbeStencil {
    let Some(stencil) = stencil else {
        return GpuProbeStencil::default();
    };
    let nodes = stencil.nodes;
    let weights = stencil.value_weights.map(|value| value as f32);
    let gradient_x = stencil.gradient_weights.map(|value| value.x as f32);
    let gradient_y = stencil.gradient_weights.map(|value| value.y as f32);
    GpuProbeStencil {
        nodes_a: UVec4::new(nodes[0], nodes[1], nodes[2], nodes[3]),
        nodes_b: UVec4::new(nodes[4], nodes[5], nodes[6], 0),
        weights_a: Vec4::from_array([weights[0], weights[1], weights[2], weights[3]]),
        weights_b: Vec4::from_array([weights[4], weights[5], weights[6], 0.0]),
        gradient_x_a: Vec4::from_array([
            gradient_x[0],
            gradient_x[1],
            gradient_x[2],
            gradient_x[3],
        ]),
        gradient_x_b: Vec4::from_array([gradient_x[4], gradient_x[5], gradient_x[6], 0.0]),
        gradient_y_a: Vec4::from_array([
            gradient_y[0],
            gradient_y[1],
            gradient_y[2],
            gradient_y[3],
        ]),
        gradient_y_b: Vec4::from_array([gradient_y[4], gradient_y[5], gradient_y[6], 0.0]),
        material: Vec4::new(
            stencil.mass_density as f32,
            stencil.stiffness.xx as f32,
            stencil.stiffness.xy as f32,
            stencil.stiffness.yy as f32,
        ),
    }
}

const fn probe_physics_flag(physics: PhysicsModel) -> f32 {
    match physics {
        PhysicsModel::Mechanical => 0.0,
        PhysicsModel::Electromagnetic { .. } => 1.0,
    }
}

fn gpu_curve_probe_stencil(
    sample: Option<(QuadraticPointStencil, Point2)>,
    stride: u64,
) -> GpuCurveProbeStencil {
    let (stencil, normal) = sample
        .map(|(stencil, normal)| (Some(stencil), normal))
        .unwrap_or((None, Point2::default()));
    let point = gpu_probe_stencil(stencil);
    GpuCurveProbeStencil {
        nodes_a: point.nodes_a,
        nodes_b: point.nodes_b,
        weights_a: point.weights_a,
        weights_b: point.weights_b,
        gradient_x_a: point.gradient_x_a,
        gradient_x_b: point.gradient_x_b,
        gradient_y_a: point.gradient_y_a,
        gradient_y_b: point.gradient_y_b,
        material: point.material,
        normal_stride_valid: Vec4::new(
            normal.x as f32,
            normal.y as f32,
            stride as f32,
            f32::from(stencil.is_some()),
        ),
    }
}

fn pack_area_matrix(values: [f64; 28]) -> [Vec4; 7] {
    std::array::from_fn(|group| {
        Vec4::from_array(std::array::from_fn(|lane| values[group * 4 + lane] as f32))
    })
}

fn gpu_area_probe_contribution(
    element: QuadraticAreaElement,
    physics: PhysicsModel,
) -> GpuAreaProbeContribution {
    let matrices = element.integrated_matrices();
    GpuAreaProbeContribution {
        nodes_a: UVec4::new(
            element.nodes[0],
            element.nodes[1],
            element.nodes[2],
            element.nodes[3],
        ),
        nodes_b: UVec4::new(element.nodes[4], element.nodes[5], element.nodes[6], 0),
        field_a: Vec4::from_array([
            matrices.field[0] as f32,
            matrices.field[1] as f32,
            matrices.field[2] as f32,
            matrices.field[3] as f32,
        ]),
        field_b: Vec4::from_array([
            matrices.field[4] as f32,
            matrices.field[5] as f32,
            matrices.field[6] as f32,
            0.0,
        ]),
        mass: pack_area_matrix(matrices.mass),
        density: pack_area_matrix(matrices.density),
        stiffness: pack_area_matrix(matrices.stiffness),
        stiffness_squared: pack_area_matrix(matrices.stiffness_squared),
        material_area: Vec4::new(probe_physics_flag(physics), 0.0, element.area as f32, 1.0),
    }
}

fn gpu_far_field_stencil(
    (stencil, position, normal): (QuadraticPointStencil, Point2, Point2),
) -> GpuFarFieldStencil {
    let point = gpu_probe_stencil(Some(stencil));
    GpuFarFieldStencil {
        nodes_a: point.nodes_a,
        nodes_b: point.nodes_b,
        weights_a: point.weights_a,
        weights_b: point.weights_b,
        gradient_x_a: point.gradient_x_a,
        gradient_x_b: point.gradient_x_b,
        gradient_y_a: point.gradient_y_a,
        gradient_y_b: point.gradient_y_b,
        position_normal: Vec4::new(
            position.x as f32,
            position.y as f32,
            normal.x as f32,
            normal.y as f32,
        ),
    }
}

pub(crate) fn probe_sample_due(completed_step: u64, stride: u64) -> bool {
    stride > 0 && completed_step.is_multiple_of(stride)
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuTransferEntry {
    indices_a: UVec4,
    weights_a: Vec4,
    indices_b: UVec4,
    weights_b: Vec4,
    mapped: Vec4,
    auxiliary: Vec4,
}

fn receive_readback(
    event: On<ReadbackComplete>,
    tags: Query<&WaveReadbackTag>,
    mut display: ResMut<WaveDisplay>,
) {
    let Ok(tag) = tags.get(event.entity) else {
        return;
    };
    let states: Vec<GpuState> = event.to_shader_type();
    if states.is_empty() {
        return;
    }
    if states[0].levels.w < 0.0 {
        return;
    }
    if states.iter().any(|state| {
        !state.levels.is_finite()
            || !state.auxiliary.is_finite()
            || !state.reconstruction.is_finite()
    }) {
        tag.stats.status.store(STATUS_ERROR, Ordering::Relaxed);
        return;
    }
    if tag.expects_transfer {
        tag.stats.status.store(STATUS_READY, Ordering::Relaxed);
    }
    display.generation = tag.generation;
    display.completed_steps = states[0].levels.w.max(0.0) as u64;
    display.current.clear();
    display.previous.clear();
    display.auxiliary.clear();
    display.indicator_displacement.clear();
    display.indicator_velocity.clear();
    display.indicator_acceleration.clear();
    display.indicator_potential.clear();
    display.current.reserve(states.len());
    display.previous.reserve(states.len());
    display.auxiliary.reserve(states.len());
    display.indicator_displacement.reserve(states.len());
    display.indicator_velocity.reserve(states.len());
    display.indicator_acceleration.reserve(states.len());
    display.indicator_potential.reserve(states.len());
    for state in states {
        display.current.push(state.levels.y);
        display.previous.push(state.levels.x);
        display.auxiliary.push(state.auxiliary.x);
        display.indicator_acceleration.push(state.auxiliary.y);
        display.indicator_velocity.push(state.auxiliary.z);
        display.indicator_displacement.push(state.auxiliary.w);
        display.indicator_potential.push(state.reconstruction.y);
    }
    display.readbacks = display.readbacks.saturating_add(1);
}

fn receive_probe_readback(
    event: On<ReadbackComplete>,
    tags: Query<&ProbeReadbackTag>,
    mut display: ResMut<ProbeDisplay>,
) {
    let Ok(tag) = tags.get(event.entity) else {
        return;
    };
    let mut records = Vec::new();
    let mut append = |slot: usize, primary: Vec4, secondary: Vec4| {
        if primary.is_finite() && secondary.is_finite() {
            records.push(PointProbeRecord {
                probe_id: tag.ids[slot],
                time: primary.w as f64,
                displacement: primary.x as f64,
                velocity: primary.y as f64,
                energy_density: primary.z as f64,
                transverse_magnitude: secondary.x as f64,
                poynting_magnitude: secondary.y as f64,
            });
        }
    };
    if tag.canonical {
        let samples: Vec<GpuCanonicalPointProbeSample> = event.to_shader_type();
        if samples.len() != PROBE_RING_FRAMES * MAX_POINT_PROBES {
            return;
        }
        for frame in 0..PROBE_RING_FRAMES {
            for slot in 0..tag.ids.len() {
                let sample = samples[frame * MAX_POINT_PROBES + slot];
                append(slot, sample.primary, sample.secondary);
            }
        }
    } else {
        let samples: Vec<GpuPointProbeSample> = event.to_shader_type();
        if samples.len() != PROBE_RING_FRAMES * MAX_POINT_PROBES {
            return;
        }
        for frame in 0..PROBE_RING_FRAMES {
            for slot in 0..tag.ids.len() {
                let sample = samples[frame * MAX_POINT_PROBES + slot];
                append(slot, sample.primary, sample.secondary);
            }
        }
    }
    records.sort_by(|a, b| {
        a.time
            .total_cmp(&b.time)
            .then_with(|| a.probe_id.cmp(&b.probe_id))
    });
    display.generation = tag.generation;
    display.revision = tag.revision;
    display.records = records;
    display.readbacks = display.readbacks.saturating_add(1);
}

fn receive_vector_overlay_readback(
    event: On<ReadbackComplete>,
    tags: Query<&VectorOverlayReadbackTag>,
    request: Res<WaveGpuRequest>,
    mut display: ResMut<VectorOverlayDisplay>,
) {
    let Ok(tag) = tags.get(event.entity) else {
        return;
    };
    if tag.generation != request.generation || tag.revision != request.vector_overlay_revision {
        return;
    }
    let samples: Vec<GpuVectorOverlaySample> = event.to_shader_type();
    if samples.len() != tag.sample_count as usize || samples.is_empty() {
        return;
    }
    let completed_steps = samples[0].metadata.x as u64;
    let time_bits = samples[0].metadata.zw();
    let absolute_time = f32::from_bits(time_bits.x) as f64 + f32::from_bits(time_bits.y) as f64;
    if samples.iter().any(|sample| {
        sample.metadata.y == 0
            || sample.metadata.x as u64 != completed_steps
            || sample.metadata.zw() != time_bits
    }) || !absolute_time.is_finite()
    {
        return;
    }
    display.generation = tag.generation;
    display.revision = tag.revision;
    display.completed_steps = completed_steps;
    display.absolute_time = absolute_time;
    display.samples.clear();
    display
        .samples
        .extend(samples.into_iter().map(|sample| VectorOverlaySample {
            complementary: Point2::new(sample.vectors.x as f64, sample.vectors.y as f64),
            energy_flow: Point2::new(sample.vectors.z as f64, sample.vectors.w as f64),
            pre_filter_complementary: Point2::new(
                sample.pre_filter_complementary.x as f64,
                sample.pre_filter_complementary.y as f64,
            ),
        }));
    display.readbacks = display.readbacks.saturating_add(1);
}

fn receive_curve_probe_readback(
    event: On<ReadbackComplete>,
    tags: Query<&CurveProbeReadbackTag>,
    mut display: ResMut<CurveProbeDisplay>,
) {
    let Ok(tag) = tags.get(event.entity) else {
        return;
    };
    let samples: Vec<GpuCurveProbeSample> = event.to_shader_type();
    if samples.len() != CURVE_PROBE_RING_FRAMES * MAX_CURVE_PROBE_POINTS {
        return;
    }
    let mut records = Vec::new();
    for descriptor in tag.descriptors.iter() {
        for frame in 0..CURVE_PROBE_RING_FRAMES {
            let base = frame * MAX_CURVE_PROBE_POINTS + descriptor.offset as usize;
            let count = descriptor.count as usize;
            let Some(values) = samples.get(base..base + count) else {
                continue;
            };
            let Some(time) = values
                .iter()
                .map(|sample| sample.primary.w)
                .find(|time| time.is_finite())
            else {
                continue;
            };
            records.push(CurveProbeRecord {
                probe_id: descriptor.id,
                time: time as f64,
                displacement: values.iter().map(|sample| sample.primary.x).collect(),
                transverse_magnitude: values.iter().map(|sample| sample.secondary.x).collect(),
                energy_density: values.iter().map(|sample| sample.primary.y).collect(),
                normal_flux: values.iter().map(|sample| sample.primary.z).collect(),
            });
        }
    }
    records.sort_by(|a, b| {
        a.time
            .total_cmp(&b.time)
            .then_with(|| a.probe_id.cmp(&b.probe_id))
    });
    display.generation = tag.generation;
    display.revision = tag.revision;
    display.records = records;
    display.readbacks = display.readbacks.saturating_add(1);
}

fn receive_area_probe_readback(
    event: On<ReadbackComplete>,
    tags: Query<&AreaProbeReadbackTag>,
    mut display: ResMut<AreaProbeDisplay>,
) {
    let Ok(tag) = tags.get(event.entity) else {
        return;
    };
    let samples: Vec<GpuAreaProbeSample> = event.to_shader_type();
    if samples.len() != AREA_PROBE_RING_FRAMES * MAX_POINT_PROBES {
        return;
    }
    let mut records = Vec::new();
    for frame in 0..AREA_PROBE_RING_FRAMES {
        for (slot, id) in tag.ids.iter().copied().enumerate() {
            let sample = samples[frame * MAX_POINT_PROBES + slot];
            if sample.primary.is_finite()
                && sample.secondary.is_finite()
                && sample.tertiary.is_finite()
                && sample.tertiary.x >= 0.5
            {
                records.push(AreaProbeRecord {
                    probe_id: id,
                    time: sample.secondary.w as f64,
                    mean_displacement: sample.primary.x as f64,
                    rms_displacement: sample.primary.y as f64,
                    rms_transverse_magnitude: sample.primary.z as f64,
                    mean_energy_density: sample.primary.w as f64,
                    total_energy: sample.secondary.x as f64,
                    covered_area: sample.secondary.y as f64,
                    coverage: sample.secondary.z as f64,
                });
            }
        }
    }
    records.sort_by(|a, b| {
        a.time
            .total_cmp(&b.time)
            .then_with(|| a.probe_id.cmp(&b.probe_id))
    });
    display.generation = tag.generation;
    display.revision = tag.revision;
    display.records = records;
    display.readbacks = display.readbacks.saturating_add(1);
}

fn receive_far_field_readback(
    event: On<ReadbackComplete>,
    tags: Query<&FarFieldReadbackTag>,
    mut display: ResMut<FarFieldDisplay>,
) {
    let Ok(tag) = tags.get(event.entity) else {
        return;
    };
    let samples: Vec<GpuProbeSample> = event.to_shader_type();
    if samples.len() != FAR_FIELD_RING_FRAMES * FAR_FIELD_DIRECTIONS {
        return;
    }
    let mut records = Vec::new();
    for frame in 0..FAR_FIELD_RING_FRAMES {
        let row = &samples[frame * FAR_FIELD_DIRECTIONS..(frame + 1) * FAR_FIELD_DIRECTIONS];
        // One direction that could not read the whole contour is a frame that
        // cannot be plotted: the pattern would have a hole in it.
        if row
            .iter()
            .any(|sample| !sample.values.is_finite() || sample.values.w < 0.5)
        {
            continue;
        }
        let time = row[0].values.z;
        if time < 0.0 || !time.is_finite() {
            continue;
        }
        records.push(FarFieldRecord {
            time: time as f64,
            amplitude: row.iter().map(|sample| sample.values.x).collect(),
            intensity: row.iter().map(|sample| sample.values.y).collect(),
        });
    }
    records.sort_by(|a, b| a.time.total_cmp(&b.time));
    display.generation = tag.generation;
    display.revision = tag.revision;
    display.records = records;
    display.readbacks = display.readbacks.saturating_add(1);
}

pub struct WaveGpuPlugin;

impl Plugin for WaveGpuPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<PacedReadbackPlugin>() {
            app.add_plugins(PacedReadbackPlugin);
        }
        embedded_asset!(app, "wave.wgsl");
        embedded_asset!(app, "probe.wgsl");
        embedded_asset!(app, "curve_probe.wgsl");
        embedded_asset!(app, "area_probe.wgsl");
        embedded_asset!(app, "far_field.wgsl");
        embedded_asset!(app, "canonical_probe.wgsl");
        embedded_asset!(app, "canonical_curve_probe.wgsl");
        embedded_asset!(app, "canonical_area_probe.wgsl");
        embedded_asset!(app, "canonical_far_field.wgsl");
        embedded_asset!(app, "wave_transfer_old.wgsl");
        embedded_asset!(app, "wave_transfer_new.wgsl");
        app.init_resource::<WaveGpuRequest>()
            .init_resource::<WaveDisplay>()
            .init_resource::<ProbeDisplay>()
            .init_resource::<CurveProbeDisplay>()
            .init_resource::<AreaProbeDisplay>()
            .init_resource::<FarFieldDisplay>()
            .init_resource::<VectorOverlayDisplay>()
            .add_observer(receive_readback)
            .add_observer(receive_probe_readback)
            .add_observer(receive_vector_overlay_readback)
            .add_observer(receive_curve_probe_readback)
            .add_observer(receive_area_probe_readback)
            .add_observer(receive_far_field_readback)
            .add_plugins(ExtractResourcePlugin::<WaveGpuRequest>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(
                Render,
                (
                    prepare_bind_group,
                    prepare_probe_bind_group,
                    prepare_curve_probe_bind_group,
                    prepare_area_probe_bind_group,
                    prepare_far_field_bind_group,
                    prepare_vector_overlay_bind_group,
                )
                    .in_set(RenderSystems::PrepareBindGroups),
            )
            .add_systems(RenderGraph, compute_wave.before(camera_driver));
    }
}

#[derive(Resource)]
pub(crate) struct WavePipeline {
    layout: BindGroupLayoutDescriptor,
    probe_layout: BindGroupLayoutDescriptor,
    curve_probe_layout: BindGroupLayoutDescriptor,
    area_probe_layout: BindGroupLayoutDescriptor,
    far_field_layout: BindGroupLayoutDescriptor,
    canonical_probe_layout: BindGroupLayoutDescriptor,
    canonical_curve_probe_layout: BindGroupLayoutDescriptor,
    canonical_area_probe_layout: BindGroupLayoutDescriptor,
    canonical_far_field_layout: BindGroupLayoutDescriptor,
    canonical_vector_overlay_layout: BindGroupLayoutDescriptor,
    transfer_old_layout: BindGroupLayoutDescriptor,
    transfer_new_layout: BindGroupLayoutDescriptor,
    step: CachedComputePipelineId,
    rotate: CachedComputePipelineId,
    inject: CachedComputePipelineId,
    filter_stage: CachedComputePipelineId,
    filter_apply: CachedComputePipelineId,
    transfer_velocity: CachedComputePipelineId,
    transfer_old: CachedComputePipelineId,
    transfer_boundary: CachedComputePipelineId,
    transfer_new: CachedComputePipelineId,
    probe: CachedComputePipelineId,
    curve_probe: CachedComputePipelineId,
    area_probe_elements: CachedComputePipelineId,
    area_probe_reduce: CachedComputePipelineId,
    far_field_sample: CachedComputePipelineId,
    far_field_project: CachedComputePipelineId,
    pub(crate) canonical_probe: CachedComputePipelineId,
    pub(crate) canonical_curve_probe: CachedComputePipelineId,
    pub(crate) canonical_area_probe_elements: CachedComputePipelineId,
    pub(crate) canonical_area_probe_reduce: CachedComputePipelineId,
    pub(crate) canonical_far_field_sample: CachedComputePipelineId,
    pub(crate) canonical_far_field_project: CachedComputePipelineId,
    pub(crate) canonical_vector_overlay: CachedComputePipelineId,
}

fn init_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "wave gather buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer::<GpuParameters>(false),
                storage_buffer_read_only::<GpuForcing>(false),
                storage_buffer_read_only::<Vec<u32>>(false),
                storage_buffer_read_only::<Vec<u32>>(false),
                storage_buffer_read_only::<Vec<GpuMatrixEntry>>(false),
                storage_buffer_read_only::<Vec<GpuNode>>(false),
                storage_buffer::<Vec<GpuState>>(false),
                storage_buffer_read_only::<Vec<GpuForcingWeightWord>>(false),
            ),
        ),
    );
    let shader = load_embedded_asset!(asset_server.as_ref(), "wave.wgsl");
    let pipeline = |entry: &'static str| ComputePipelineDescriptor {
        label: Some(Cow::from(format!("wave {entry}"))),
        layout: vec![layout.clone()],
        shader: shader.clone(),
        entry_point: Some(Cow::Borrowed(entry)),
        ..default()
    };
    let step = pipeline_cache.queue_compute_pipeline(pipeline("advance_wave"));
    let rotate = pipeline_cache.queue_compute_pipeline(pipeline("rotate"));
    let inject = pipeline_cache.queue_compute_pipeline(pipeline("inject"));
    let filter_stage = pipeline_cache.queue_compute_pipeline(pipeline("filter_stage"));
    let filter_apply = pipeline_cache.queue_compute_pipeline(pipeline("filter_apply"));
    let probe_layout = BindGroupLayoutDescriptor::new(
        "wave point-probe buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer_read_only::<GpuParameters>(false),
                storage_buffer_read_only::<Vec<GpuState>>(false),
                storage_buffer_read_only::<Vec<GpuProbeStencil>>(false),
                storage_buffer_read_only::<GpuProbeControl>(false),
                storage_buffer::<Vec<GpuPointProbeSample>>(false),
            ),
        ),
    );
    let probe = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some(Cow::Borrowed("wave point probes")),
        layout: vec![probe_layout.clone()],
        shader: load_embedded_asset!(asset_server.as_ref(), "probe.wgsl"),
        entry_point: Some(Cow::Borrowed("sample_probes")),
        ..default()
    });
    let curve_probe_layout = BindGroupLayoutDescriptor::new(
        "wave line-probe buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer_read_only::<GpuParameters>(false),
                storage_buffer_read_only::<Vec<GpuState>>(false),
                storage_buffer_read_only::<Vec<GpuCurveProbeStencil>>(false),
                storage_buffer_read_only::<GpuProbeControl>(false),
                storage_buffer::<Vec<GpuCurveProbeSample>>(false),
            ),
        ),
    );
    let curve_probe = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some(Cow::Borrowed("wave line probes")),
        layout: vec![curve_probe_layout.clone()],
        shader: load_embedded_asset!(asset_server.as_ref(), "curve_probe.wgsl"),
        entry_point: Some(Cow::Borrowed("sample_curve_probes")),
        ..default()
    });
    let area_probe_layout = BindGroupLayoutDescriptor::new(
        "wave area-probe buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer_read_only::<GpuParameters>(false),
                storage_buffer_read_only::<Vec<GpuState>>(false),
                storage_buffer_read_only::<Vec<GpuAreaProbeContribution>>(false),
                storage_buffer_read_only::<Vec<GpuAreaProbeDescriptor>>(false),
                storage_buffer_read_only::<GpuProbeControl>(false),
                storage_buffer::<Vec<GpuAreaProbeContributionSample>>(false),
                storage_buffer::<Vec<GpuAreaProbeSample>>(false),
            ),
        ),
    );
    let area_shader = load_embedded_asset!(asset_server.as_ref(), "area_probe.wgsl");
    let area_pipeline = |label: &'static str, entry: &'static str| ComputePipelineDescriptor {
        label: Some(Cow::Borrowed(label)),
        layout: vec![area_probe_layout.clone()],
        shader: area_shader.clone(),
        entry_point: Some(Cow::Borrowed(entry)),
        ..default()
    };
    let area_probe_elements = pipeline_cache.queue_compute_pipeline(area_pipeline(
        "wave area-probe elements",
        "sample_area_elements",
    ));
    let area_probe_reduce = pipeline_cache.queue_compute_pipeline(area_pipeline(
        "wave area-probe reduction",
        "reduce_area_probes",
    ));
    let far_field_layout = BindGroupLayoutDescriptor::new(
        "wave far-field buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer_read_only::<GpuParameters>(false),
                storage_buffer_read_only::<Vec<GpuState>>(false),
                storage_buffer_read_only::<Vec<GpuFarFieldStencil>>(false),
                storage_buffer_read_only::<GpuFarFieldControl>(false),
                storage_buffer::<Vec<GpuProbeSample>>(false),
                storage_buffer::<Vec<GpuProbeSample>>(false),
            ),
        ),
    );
    let far_field_shader = load_embedded_asset!(asset_server.as_ref(), "far_field.wgsl");
    let far_field_pipeline = |label: &'static str, entry: &'static str| ComputePipelineDescriptor {
        label: Some(Cow::Borrowed(label)),
        layout: vec![far_field_layout.clone()],
        shader: far_field_shader.clone(),
        entry_point: Some(Cow::Borrowed(entry)),
        ..default()
    };
    let far_field_sample = pipeline_cache.queue_compute_pipeline(far_field_pipeline(
        "wave far-field contour sampler",
        "sample_contour",
    ));
    let far_field_project = pipeline_cache.queue_compute_pipeline(far_field_pipeline(
        "wave far-field projector",
        "project_directions",
    ));
    let canonical_probe_layout = BindGroupLayoutDescriptor::new(
        "canonical point-probe buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer_read_only::<GpuCanonicalControl>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalStateWord>>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalNode>>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalPointStencil>>(false),
                storage_buffer_read_only::<GpuProbeControl>(false),
                storage_buffer::<Vec<GpuCanonicalPointProbeSample>>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalTableWord>>(false),
            ),
        ),
    );
    let canonical_probe = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some(Cow::Borrowed("canonical point probes")),
        layout: vec![canonical_probe_layout.clone()],
        shader: load_embedded_asset!(asset_server.as_ref(), "canonical_probe.wgsl"),
        entry_point: Some(Cow::Borrowed("sample_probes")),
        ..default()
    });
    let canonical_vector_overlay_layout = BindGroupLayoutDescriptor::new(
        "canonical vector-overlay buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer_read_only::<GpuCanonicalControl>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalStateWord>>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalNode>>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalPointStencil>>(false),
                storage_buffer_read_only::<GpuProbeControl>(false),
                storage_buffer::<Vec<GpuVectorOverlaySample>>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalTableWord>>(false),
            ),
        ),
    );
    let canonical_vector_overlay =
        pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(Cow::Borrowed("canonical vector overlay")),
            layout: vec![canonical_vector_overlay_layout.clone()],
            shader: load_embedded_asset!(asset_server.as_ref(), "canonical_probe.wgsl"),
            entry_point: Some(Cow::Borrowed("sample_vector_overlay")),
            ..default()
        });
    let canonical_curve_probe_layout = BindGroupLayoutDescriptor::new(
        "canonical line-probe buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer_read_only::<GpuCanonicalControl>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalStateWord>>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalNode>>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalCurveStencil>>(false),
                storage_buffer_read_only::<GpuProbeControl>(false),
                storage_buffer::<Vec<GpuCurveProbeSample>>(false),
            ),
        ),
    );
    let canonical_curve_probe = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some(Cow::Borrowed("canonical line probes")),
        layout: vec![canonical_curve_probe_layout.clone()],
        shader: load_embedded_asset!(asset_server.as_ref(), "canonical_curve_probe.wgsl"),
        entry_point: Some(Cow::Borrowed("sample_curve_probes")),
        ..default()
    });
    let canonical_area_probe_layout = BindGroupLayoutDescriptor::new(
        "canonical area-probe buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer_read_only::<GpuCanonicalControl>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalStateWord>>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalNode>>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalAreaContribution>>(false),
                storage_buffer_read_only::<Vec<GpuAreaProbeDescriptor>>(false),
                storage_buffer_read_only::<GpuProbeControl>(false),
                storage_buffer::<Vec<GpuAreaProbeContributionSample>>(false),
                storage_buffer::<Vec<GpuAreaProbeSample>>(false),
            ),
        ),
    );
    let canonical_area_shader =
        load_embedded_asset!(asset_server.as_ref(), "canonical_area_probe.wgsl");
    let canonical_area_pipeline =
        |label: &'static str, entry: &'static str| ComputePipelineDescriptor {
            label: Some(Cow::Borrowed(label)),
            layout: vec![canonical_area_probe_layout.clone()],
            shader: canonical_area_shader.clone(),
            entry_point: Some(Cow::Borrowed(entry)),
            ..default()
        };
    let canonical_area_probe_elements = pipeline_cache.queue_compute_pipeline(
        canonical_area_pipeline("canonical area-probe elements", "sample_area_elements"),
    );
    let canonical_area_probe_reduce = pipeline_cache.queue_compute_pipeline(
        canonical_area_pipeline("canonical area-probe reduction", "reduce_area_probes"),
    );
    let canonical_far_field_layout = BindGroupLayoutDescriptor::new(
        "canonical far-field buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer_read_only::<GpuCanonicalControl>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalStateWord>>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalNode>>(false),
                storage_buffer_read_only::<Vec<GpuCanonicalFarFieldStencil>>(false),
                storage_buffer_read_only::<GpuFarFieldControl>(false),
                storage_buffer::<Vec<GpuProbeSample>>(false),
                storage_buffer::<Vec<GpuProbeSample>>(false),
            ),
        ),
    );
    let canonical_far_shader =
        load_embedded_asset!(asset_server.as_ref(), "canonical_far_field.wgsl");
    let canonical_far_pipeline =
        |label: &'static str, entry: &'static str| ComputePipelineDescriptor {
            label: Some(Cow::Borrowed(label)),
            layout: vec![canonical_far_field_layout.clone()],
            shader: canonical_far_shader.clone(),
            entry_point: Some(Cow::Borrowed(entry)),
            ..default()
        };
    let canonical_far_field_sample = pipeline_cache.queue_compute_pipeline(canonical_far_pipeline(
        "canonical far-field contour sampler",
        "sample_contour",
    ));
    let canonical_far_field_project = pipeline_cache.queue_compute_pipeline(
        canonical_far_pipeline("canonical far-field projector", "project_directions"),
    );
    let transfer_old_layout = BindGroupLayoutDescriptor::new(
        "wave old-state transfer buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer_read_only::<GpuParameters>(false),
                storage_buffer_read_only::<GpuForcing>(false),
                storage_buffer_read_only::<Vec<u32>>(false),
                storage_buffer_read_only::<Vec<u32>>(false),
                storage_buffer_read_only::<Vec<GpuMatrixEntry>>(false),
                storage_buffer_read_only::<Vec<GpuNode>>(false),
                storage_buffer::<Vec<GpuState>>(false),
                storage_buffer::<Vec<GpuTransferEntry>>(false),
            ),
        ),
    );
    let transfer_new_layout = BindGroupLayoutDescriptor::new(
        "wave new-state transfer buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer::<GpuParameters>(false),
                storage_buffer_read_only::<GpuForcing>(false),
                storage_buffer_read_only::<Vec<u32>>(false),
                storage_buffer_read_only::<Vec<u32>>(false),
                storage_buffer_read_only::<Vec<GpuMatrixEntry>>(false),
                storage_buffer_read_only::<Vec<GpuNode>>(false),
                storage_buffer::<Vec<GpuState>>(false),
                storage_buffer::<Vec<GpuTransferEntry>>(false),
            ),
        ),
    );
    let transfer_pipeline = |label: &'static str,
                             entry: &'static str,
                             layout: BindGroupLayoutDescriptor,
                             shader: Handle<Shader>|
     -> ComputePipelineDescriptor {
        ComputePipelineDescriptor {
            label: Some(Cow::Borrowed(label)),
            layout: vec![layout],
            shader,
            entry_point: Some(Cow::Borrowed(entry)),
            ..default()
        }
    };
    let old_transfer_shader = load_embedded_asset!(asset_server.as_ref(), "wave_transfer_old.wgsl");
    let transfer_velocity = pipeline_cache.queue_compute_pipeline(transfer_pipeline(
        "wave transfer velocity",
        "prepare_velocity",
        transfer_old_layout.clone(),
        old_transfer_shader.clone(),
    ));
    let transfer_old = pipeline_cache.queue_compute_pipeline(transfer_pipeline(
        "wave transfer old",
        "transfer",
        transfer_old_layout.clone(),
        old_transfer_shader,
    ));
    let new_transfer_shader = load_embedded_asset!(asset_server.as_ref(), "wave_transfer_new.wgsl");
    let transfer_boundary = pipeline_cache.queue_compute_pipeline(transfer_pipeline(
        "wave transfer prescribed boundary",
        "prepare_boundary",
        transfer_new_layout.clone(),
        new_transfer_shader.clone(),
    ));
    let transfer_new = pipeline_cache.queue_compute_pipeline(transfer_pipeline(
        "wave transfer new",
        "transfer",
        transfer_new_layout.clone(),
        new_transfer_shader,
    ));
    commands.insert_resource(WavePipeline {
        layout,
        probe_layout,
        curve_probe_layout,
        area_probe_layout,
        far_field_layout,
        canonical_probe_layout,
        canonical_curve_probe_layout,
        canonical_area_probe_layout,
        canonical_far_field_layout,
        canonical_vector_overlay_layout,
        transfer_old_layout,
        transfer_new_layout,
        step,
        rotate,
        inject,
        filter_stage,
        filter_apply,
        transfer_velocity,
        transfer_old,
        transfer_boundary,
        transfer_new,
        probe,
        curve_probe,
        area_probe_elements,
        area_probe_reduce,
        far_field_sample,
        far_field_project,
        canonical_probe,
        canonical_curve_probe,
        canonical_area_probe_elements,
        canonical_area_probe_reduce,
        canonical_far_field_sample,
        canonical_far_field_project,
        canonical_vector_overlay,
    });
}

#[derive(Resource)]
struct WaveBindGroup {
    generation: u64,
    buffer_revision: u64,
    completed_steps: u64,
    pulse_serial: u64,
    initialized: bool,
    bind_group: BindGroup,
}

#[derive(Resource)]
struct WaveTransferBindGroups {
    generation: u64,
    old: BindGroup,
    new: BindGroup,
}

#[derive(Resource)]
pub(crate) struct ProbeBindGroup {
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    pub(crate) bind_group: BindGroup,
}

#[derive(Resource)]
pub(crate) struct CurveProbeBindGroup {
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    pub(crate) bind_group: BindGroup,
}

#[derive(Resource)]
pub(crate) struct AreaProbeBindGroup {
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    pub(crate) bind_group: BindGroup,
}

#[derive(Resource)]
pub(crate) struct FarFieldBindGroup {
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    pub(crate) bind_group: BindGroup,
}

#[derive(Resource)]
pub(crate) struct VectorOverlayBindGroup {
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    pub(crate) sampled: bool,
    pub(crate) bind_group: BindGroup,
}

#[allow(clippy::too_many_arguments)]
fn prepare_vector_overlay_bind_group(
    mut commands: Commands,
    request: Option<Res<WaveGpuRequest>>,
    canonical_request: Option<Res<CanonicalGpuRequest>>,
    existing: Option<Res<VectorOverlayBindGroup>>,
    pipeline: Res<WavePipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    gpu_buffers: Res<RenderAssets<GpuShaderBuffer>>,
) {
    let Some(request) = request else { return };
    let Some(overlay) = &request.vector_overlay else {
        if existing.is_some() {
            commands.remove_resource::<VectorOverlayBindGroup>();
        }
        return;
    };
    if existing.as_ref().is_some_and(|group| {
        group.generation == request.generation && group.revision == request.vector_overlay_revision
    }) {
        return;
    }
    let Some(canonical) = canonical_request
        .as_ref()
        .and_then(|request| request.buffer_handles())
    else {
        return;
    };
    let (
        Some(control),
        Some(state),
        Some(nodes),
        Some(stencils),
        Some(probe_control),
        Some(output),
        Some(tables),
    ) = (
        gpu_buffers.get(&canonical.control),
        gpu_buffers.get(&canonical.state),
        gpu_buffers.get(&canonical.nodes),
        gpu_buffers.get(&overlay.stencils),
        gpu_buffers.get(&overlay.control),
        gpu_buffers.get(&overlay.output),
        gpu_buffers.get(&canonical.tables),
    )
    else {
        return;
    };
    let bind_group = render_device.create_bind_group(
        Some("canonical vector-overlay bind group"),
        &pipeline_cache.get_bind_group_layout(&pipeline.canonical_vector_overlay_layout),
        &BindGroupEntries::sequential((
            control.buffer.as_entire_buffer_binding(),
            state.buffer.as_entire_buffer_binding(),
            nodes.buffer.as_entire_buffer_binding(),
            stencils.buffer.as_entire_buffer_binding(),
            probe_control.buffer.as_entire_buffer_binding(),
            output.buffer.as_entire_buffer_binding(),
            tables.buffer.as_entire_buffer_binding(),
        )),
    );
    commands.insert_resource(VectorOverlayBindGroup {
        generation: request.generation,
        revision: request.vector_overlay_revision,
        sampled: false,
        bind_group,
    });
}

#[allow(clippy::too_many_arguments)]
fn prepare_far_field_bind_group(
    mut commands: Commands,
    request: Option<Res<WaveGpuRequest>>,
    canonical_request: Option<Res<CanonicalGpuRequest>>,
    existing: Option<Res<FarFieldBindGroup>>,
    pipeline: Res<WavePipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    gpu_buffers: Res<RenderAssets<GpuShaderBuffer>>,
) {
    let Some(request) = request else { return };
    let Some(far_field) = &request.far_field else {
        if existing.is_some() {
            commands.remove_resource::<FarFieldBindGroup>();
        }
        return;
    };
    if existing.as_ref().is_some_and(|group| {
        group.generation == request.generation && group.revision == request.far_field_revision
    }) {
        return;
    }
    let (Some(stencils), Some(control), Some(raw), Some(output)) = (
        gpu_buffers.get(&far_field.stencils),
        gpu_buffers.get(&far_field.control),
        gpu_buffers.get(&far_field.raw),
        gpu_buffers.get(&far_field.output),
    ) else {
        return;
    };
    let bind_group = if far_field.canonical {
        let Some(canonical) = canonical_request
            .as_ref()
            .and_then(|request| request.buffer_handles())
        else {
            return;
        };
        let (Some(canonical_control), Some(state), Some(nodes)) = (
            gpu_buffers.get(&canonical.control),
            gpu_buffers.get(&canonical.state),
            gpu_buffers.get(&canonical.nodes),
        ) else {
            return;
        };
        render_device.create_bind_group(
            Some("canonical far-field bind group"),
            &pipeline_cache.get_bind_group_layout(&pipeline.canonical_far_field_layout),
            &BindGroupEntries::sequential((
                canonical_control.buffer.as_entire_buffer_binding(),
                state.buffer.as_entire_buffer_binding(),
                nodes.buffer.as_entire_buffer_binding(),
                stencils.buffer.as_entire_buffer_binding(),
                control.buffer.as_entire_buffer_binding(),
                raw.buffer.as_entire_buffer_binding(),
                output.buffer.as_entire_buffer_binding(),
            )),
        )
    } else {
        let Some(wave) = &request.buffers else { return };
        let (Some(parameters), Some(state)) = (
            gpu_buffers.get(&wave.parameters),
            gpu_buffers.get(&wave.state),
        ) else {
            return;
        };
        render_device.create_bind_group(
            Some("wave far-field bind group"),
            &pipeline_cache.get_bind_group_layout(&pipeline.far_field_layout),
            &BindGroupEntries::sequential((
                parameters.buffer.as_entire_buffer_binding(),
                state.buffer.as_entire_buffer_binding(),
                stencils.buffer.as_entire_buffer_binding(),
                control.buffer.as_entire_buffer_binding(),
                raw.buffer.as_entire_buffer_binding(),
                output.buffer.as_entire_buffer_binding(),
            )),
        )
    };
    commands.insert_resource(FarFieldBindGroup {
        generation: request.generation,
        revision: request.far_field_revision,
        bind_group,
    });
}

#[allow(clippy::too_many_arguments)]
fn prepare_area_probe_bind_group(
    mut commands: Commands,
    request: Option<Res<WaveGpuRequest>>,
    canonical_request: Option<Res<CanonicalGpuRequest>>,
    existing: Option<Res<AreaProbeBindGroup>>,
    pipeline: Res<WavePipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    gpu_buffers: Res<RenderAssets<GpuShaderBuffer>>,
) {
    let Some(request) = request else { return };
    let Some(probes) = &request.area_probes else {
        if existing.is_some() {
            commands.remove_resource::<AreaProbeBindGroup>();
        }
        return;
    };
    if existing.as_ref().is_some_and(|group| {
        group.generation == request.generation && group.revision == request.area_probe_revision
    }) {
        return;
    }
    let (Some(contributions), Some(descriptors), Some(control), Some(scratch), Some(output)) = (
        gpu_buffers.get(&probes.contributions),
        gpu_buffers.get(&probes.descriptors),
        gpu_buffers.get(&probes.control),
        gpu_buffers.get(&probes.scratch),
        gpu_buffers.get(&probes.output),
    ) else {
        return;
    };
    let bind_group = if probes.canonical {
        let Some(canonical) = canonical_request
            .as_ref()
            .and_then(|request| request.buffer_handles())
        else {
            return;
        };
        let (Some(canonical_control), Some(state), Some(nodes)) = (
            gpu_buffers.get(&canonical.control),
            gpu_buffers.get(&canonical.state),
            gpu_buffers.get(&canonical.nodes),
        ) else {
            return;
        };
        render_device.create_bind_group(
            Some("canonical area-probe bind group"),
            &pipeline_cache.get_bind_group_layout(&pipeline.canonical_area_probe_layout),
            &BindGroupEntries::sequential((
                canonical_control.buffer.as_entire_buffer_binding(),
                state.buffer.as_entire_buffer_binding(),
                nodes.buffer.as_entire_buffer_binding(),
                contributions.buffer.as_entire_buffer_binding(),
                descriptors.buffer.as_entire_buffer_binding(),
                control.buffer.as_entire_buffer_binding(),
                scratch.buffer.as_entire_buffer_binding(),
                output.buffer.as_entire_buffer_binding(),
            )),
        )
    } else {
        let Some(wave) = &request.buffers else { return };
        let (Some(parameters), Some(state)) = (
            gpu_buffers.get(&wave.parameters),
            gpu_buffers.get(&wave.state),
        ) else {
            return;
        };
        render_device.create_bind_group(
            Some("wave area-probe bind group"),
            &pipeline_cache.get_bind_group_layout(&pipeline.area_probe_layout),
            &BindGroupEntries::sequential((
                parameters.buffer.as_entire_buffer_binding(),
                state.buffer.as_entire_buffer_binding(),
                contributions.buffer.as_entire_buffer_binding(),
                descriptors.buffer.as_entire_buffer_binding(),
                control.buffer.as_entire_buffer_binding(),
                scratch.buffer.as_entire_buffer_binding(),
                output.buffer.as_entire_buffer_binding(),
            )),
        )
    };
    commands.insert_resource(AreaProbeBindGroup {
        generation: request.generation,
        revision: request.area_probe_revision,
        bind_group,
    });
}

#[allow(clippy::too_many_arguments)]
fn prepare_curve_probe_bind_group(
    mut commands: Commands,
    request: Option<Res<WaveGpuRequest>>,
    canonical_request: Option<Res<CanonicalGpuRequest>>,
    existing: Option<Res<CurveProbeBindGroup>>,
    pipeline: Res<WavePipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    gpu_buffers: Res<RenderAssets<GpuShaderBuffer>>,
) {
    let Some(request) = request else { return };
    let Some(probes) = &request.curve_probes else {
        if existing.is_some() {
            commands.remove_resource::<CurveProbeBindGroup>();
        }
        return;
    };
    if existing.as_ref().is_some_and(|group| {
        group.generation == request.generation && group.revision == request.curve_probe_revision
    }) {
        return;
    }
    let (Some(stencils), Some(control), Some(output)) = (
        gpu_buffers.get(&probes.stencils),
        gpu_buffers.get(&probes.control),
        gpu_buffers.get(&probes.output),
    ) else {
        return;
    };
    let bind_group = if probes.canonical {
        let Some(canonical) = canonical_request
            .as_ref()
            .and_then(|request| request.buffer_handles())
        else {
            return;
        };
        let (Some(canonical_control), Some(state), Some(nodes)) = (
            gpu_buffers.get(&canonical.control),
            gpu_buffers.get(&canonical.state),
            gpu_buffers.get(&canonical.nodes),
        ) else {
            return;
        };
        render_device.create_bind_group(
            Some("canonical line-probe bind group"),
            &pipeline_cache.get_bind_group_layout(&pipeline.canonical_curve_probe_layout),
            &BindGroupEntries::sequential((
                canonical_control.buffer.as_entire_buffer_binding(),
                state.buffer.as_entire_buffer_binding(),
                nodes.buffer.as_entire_buffer_binding(),
                stencils.buffer.as_entire_buffer_binding(),
                control.buffer.as_entire_buffer_binding(),
                output.buffer.as_entire_buffer_binding(),
            )),
        )
    } else {
        let Some(wave) = &request.buffers else { return };
        let (Some(parameters), Some(state)) = (
            gpu_buffers.get(&wave.parameters),
            gpu_buffers.get(&wave.state),
        ) else {
            return;
        };
        render_device.create_bind_group(
            Some("wave line-probe bind group"),
            &pipeline_cache.get_bind_group_layout(&pipeline.curve_probe_layout),
            &BindGroupEntries::sequential((
                parameters.buffer.as_entire_buffer_binding(),
                state.buffer.as_entire_buffer_binding(),
                stencils.buffer.as_entire_buffer_binding(),
                control.buffer.as_entire_buffer_binding(),
                output.buffer.as_entire_buffer_binding(),
            )),
        )
    };
    commands.insert_resource(CurveProbeBindGroup {
        generation: request.generation,
        revision: request.curve_probe_revision,
        bind_group,
    });
}

#[allow(clippy::too_many_arguments)]
fn prepare_probe_bind_group(
    mut commands: Commands,
    request: Option<Res<WaveGpuRequest>>,
    canonical_request: Option<Res<CanonicalGpuRequest>>,
    existing: Option<Res<ProbeBindGroup>>,
    pipeline: Res<WavePipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    gpu_buffers: Res<RenderAssets<GpuShaderBuffer>>,
) {
    let Some(request) = request else {
        return;
    };
    let Some(probes) = &request.probes else {
        if existing.is_some() {
            commands.remove_resource::<ProbeBindGroup>();
        }
        return;
    };
    if existing.as_ref().is_some_and(|group| {
        group.generation == request.generation && group.revision == request.probe_revision
    }) {
        return;
    }
    let (Some(stencils), Some(control), Some(output)) = (
        gpu_buffers.get(&probes.stencils),
        gpu_buffers.get(&probes.control),
        gpu_buffers.get(&probes.output),
    ) else {
        return;
    };
    let bind_group = if probes.canonical {
        let Some(canonical) = canonical_request
            .as_ref()
            .and_then(|request| request.buffer_handles())
        else {
            return;
        };
        let (Some(canonical_control), Some(state), Some(nodes), Some(tables)) = (
            gpu_buffers.get(&canonical.control),
            gpu_buffers.get(&canonical.state),
            gpu_buffers.get(&canonical.nodes),
            gpu_buffers.get(&canonical.tables),
        ) else {
            return;
        };
        render_device.create_bind_group(
            Some("canonical point-probe bind group"),
            &pipeline_cache.get_bind_group_layout(&pipeline.canonical_probe_layout),
            &BindGroupEntries::sequential((
                canonical_control.buffer.as_entire_buffer_binding(),
                state.buffer.as_entire_buffer_binding(),
                nodes.buffer.as_entire_buffer_binding(),
                stencils.buffer.as_entire_buffer_binding(),
                control.buffer.as_entire_buffer_binding(),
                output.buffer.as_entire_buffer_binding(),
                tables.buffer.as_entire_buffer_binding(),
            )),
        )
    } else {
        let Some(wave) = &request.buffers else {
            return;
        };
        let (Some(parameters), Some(state)) = (
            gpu_buffers.get(&wave.parameters),
            gpu_buffers.get(&wave.state),
        ) else {
            return;
        };
        render_device.create_bind_group(
            Some("wave point-probe bind group"),
            &pipeline_cache.get_bind_group_layout(&pipeline.probe_layout),
            &BindGroupEntries::sequential((
                parameters.buffer.as_entire_buffer_binding(),
                state.buffer.as_entire_buffer_binding(),
                stencils.buffer.as_entire_buffer_binding(),
                control.buffer.as_entire_buffer_binding(),
                output.buffer.as_entire_buffer_binding(),
            )),
        )
    };
    commands.insert_resource(ProbeBindGroup {
        generation: request.generation,
        revision: request.probe_revision,
        bind_group,
    });
}

fn prepare_bind_group(
    mut commands: Commands,
    request: Option<Res<WaveGpuRequest>>,
    existing: Option<Res<WaveBindGroup>>,
    pipeline: Res<WavePipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    gpu_buffers: Res<RenderAssets<GpuShaderBuffer>>,
) {
    let Some(request) = request else {
        return;
    };
    if request.buffers.is_none()
        || existing.as_ref().is_some_and(|group| {
            group.generation == request.generation
                && group.buffer_revision == request.buffer_revision
        })
    {
        return;
    }
    let handles = request.buffers.as_ref().unwrap();
    let Some(parameters) = gpu_buffers.get(&handles.parameters) else {
        return;
    };
    let Some(forcing) = gpu_buffers.get(&handles.forcing) else {
        return;
    };
    let Some(row_offsets) = gpu_buffers.get(&handles.row_offsets) else {
        return;
    };
    let Some(columns) = gpu_buffers.get(&handles.columns) else {
        return;
    };
    let Some(stiffness) = gpu_buffers.get(&handles.stiffness) else {
        return;
    };
    let Some(nodes) = gpu_buffers.get(&handles.nodes) else {
        return;
    };
    let Some(state) = gpu_buffers.get(&handles.state) else {
        return;
    };
    let Some(forcing_weights) = gpu_buffers.get(&handles.forcing_weights) else {
        return;
    };
    let bind_group = render_device.create_bind_group(
        Some("wave gather bind group"),
        &pipeline_cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((
            parameters.buffer.as_entire_buffer_binding(),
            forcing.buffer.as_entire_buffer_binding(),
            row_offsets.buffer.as_entire_buffer_binding(),
            columns.buffer.as_entire_buffer_binding(),
            stiffness.buffer.as_entire_buffer_binding(),
            nodes.buffer.as_entire_buffer_binding(),
            state.buffer.as_entire_buffer_binding(),
            forcing_weights.buffer.as_entire_buffer_binding(),
        )),
    );
    if let Some(transfer) = &request.transfer {
        let old = &transfer.old;
        let Some(old_parameters) = gpu_buffers.get(&old.parameters) else {
            return;
        };
        let Some(old_forcing) = gpu_buffers.get(&old.forcing) else {
            return;
        };
        let Some(old_row_offsets) = gpu_buffers.get(&old.row_offsets) else {
            return;
        };
        let Some(old_columns) = gpu_buffers.get(&old.columns) else {
            return;
        };
        let Some(old_stiffness) = gpu_buffers.get(&old.stiffness) else {
            return;
        };
        let Some(old_nodes) = gpu_buffers.get(&old.nodes) else {
            return;
        };
        let Some(old_state) = gpu_buffers.get(&old.state) else {
            return;
        };
        let Some(entries) = gpu_buffers.get(&transfer.entries) else {
            return;
        };
        let old_group = render_device.create_bind_group(
            Some("wave old-state transfer bind group"),
            &pipeline_cache.get_bind_group_layout(&pipeline.transfer_old_layout),
            &BindGroupEntries::sequential((
                old_parameters.buffer.as_entire_buffer_binding(),
                old_forcing.buffer.as_entire_buffer_binding(),
                old_row_offsets.buffer.as_entire_buffer_binding(),
                old_columns.buffer.as_entire_buffer_binding(),
                old_stiffness.buffer.as_entire_buffer_binding(),
                old_nodes.buffer.as_entire_buffer_binding(),
                old_state.buffer.as_entire_buffer_binding(),
                entries.buffer.as_entire_buffer_binding(),
            )),
        );
        let new_group = render_device.create_bind_group(
            Some("wave new-state transfer bind group"),
            &pipeline_cache.get_bind_group_layout(&pipeline.transfer_new_layout),
            &BindGroupEntries::sequential((
                parameters.buffer.as_entire_buffer_binding(),
                forcing.buffer.as_entire_buffer_binding(),
                row_offsets.buffer.as_entire_buffer_binding(),
                columns.buffer.as_entire_buffer_binding(),
                stiffness.buffer.as_entire_buffer_binding(),
                nodes.buffer.as_entire_buffer_binding(),
                state.buffer.as_entire_buffer_binding(),
                entries.buffer.as_entire_buffer_binding(),
            )),
        );
        commands.insert_resource(WaveTransferBindGroups {
            generation: request.generation,
            old: old_group,
            new: new_group,
        });
    } else {
        commands.remove_resource::<WaveTransferBindGroups>();
    }
    let (completed_steps, pulse_serial, initialized) = existing
        .as_ref()
        .filter(|group| group.generation == request.generation)
        .map_or((0, 0, request.transfer.is_none()), |group| {
            (group.completed_steps, group.pulse_serial, group.initialized)
        });
    commands.insert_resource(WaveBindGroup {
        generation: request.generation,
        buffer_revision: request.buffer_revision,
        completed_steps,
        pulse_serial,
        initialized,
        bind_group,
    });
}

#[allow(clippy::too_many_arguments)]
fn compute_wave(
    mut render_context: RenderContext,
    request: Option<Res<WaveGpuRequest>>,
    group: Option<ResMut<WaveBindGroup>>,
    transfer_groups: Option<Res<WaveTransferBindGroups>>,
    probe_group: Option<Res<ProbeBindGroup>>,
    curve_probe_group: Option<Res<CurveProbeBindGroup>>,
    area_probe_group: Option<Res<AreaProbeBindGroup>>,
    far_field_group: Option<Res<FarFieldBindGroup>>,
    pipeline: Res<WavePipeline>,
    pipeline_cache: Res<PipelineCache>,
) {
    let (Some(request), Some(mut group)) = (request, group) else {
        return;
    };
    if group.generation != request.generation || group.buffer_revision != request.buffer_revision {
        return;
    }
    let pipelines = [
        pipeline.step,
        pipeline.rotate,
        pipeline.inject,
        pipeline.transfer_velocity,
        pipeline.transfer_old,
        pipeline.transfer_boundary,
        pipeline.transfer_new,
        pipeline.probe,
        pipeline.curve_probe,
        pipeline.area_probe_elements,
        pipeline.area_probe_reduce,
        pipeline.far_field_sample,
        pipeline.far_field_project,
        pipeline.filter_stage,
        pipeline.filter_apply,
    ];
    if pipelines.iter().any(|id| {
        matches!(
            pipeline_cache.get_compute_pipeline_state(*id),
            CachedPipelineState::Err(_)
        )
    }) {
        request.stats.status.store(STATUS_ERROR, Ordering::Relaxed);
        return;
    }
    let (Some(step), Some(rotate), Some(inject)) = (
        pipeline_cache.get_compute_pipeline(pipeline.step),
        pipeline_cache.get_compute_pipeline(pipeline.rotate),
        pipeline_cache.get_compute_pipeline(pipeline.inject),
    ) else {
        return;
    };
    // Both passes or neither: the second reads what the first stages, so a half
    // ready pair would apply a correction built from whatever the slot held.
    let filter = request.grid_scale_filter.then(|| {
        Some((
            pipeline_cache.get_compute_pipeline(pipeline.filter_stage)?,
            pipeline_cache.get_compute_pipeline(pipeline.filter_apply)?,
        ))
    });
    let filter = match filter {
        Some(None) => return,
        Some(Some(pair)) => Some(pair),
        None => None,
    };
    let workgroups = request.dof_count.div_ceil(WORKGROUP_SIZE);
    if workgroups == 0 {
        return;
    }
    if !group.initialized {
        let Some(transfer_groups) =
            transfer_groups.filter(|groups| groups.generation == request.generation)
        else {
            return;
        };
        let Some(transfer) = &request.transfer else {
            return;
        };
        let (
            Some(transfer_velocity),
            Some(transfer_old),
            Some(transfer_boundary),
            Some(transfer_new),
        ) = (
            pipeline_cache.get_compute_pipeline(pipeline.transfer_velocity),
            pipeline_cache.get_compute_pipeline(pipeline.transfer_old),
            pipeline_cache.get_compute_pipeline(pipeline.transfer_boundary),
            pipeline_cache.get_compute_pipeline(pipeline.transfer_new),
        )
        else {
            return;
        };
        let mut pass =
            render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor {
                    label: Some("wave state transfer"),
                    ..default()
                });
        pass.set_pipeline(transfer_velocity);
        pass.set_bind_group(0, &transfer_groups.old, &[]);
        pass.dispatch_workgroups(transfer.old_dof_count.div_ceil(WORKGROUP_SIZE), 1, 1);
        pass.set_pipeline(transfer_old);
        pass.dispatch_workgroups(workgroups, 1, 1);
        pass.set_bind_group(0, &transfer_groups.new, &[]);
        pass.set_pipeline(transfer_boundary);
        pass.dispatch_workgroups(workgroups, 1, 1);
        pass.set_pipeline(transfer_new);
        pass.dispatch_workgroups(workgroups, 1, 1);
        drop(pass);
        group.initialized = true;
        request
            .stats
            .status
            .store(STATUS_TRANSFERRING, Ordering::Relaxed);
        request.stats.dispatches.fetch_add(4, Ordering::Relaxed);
        return;
    }
    if request.transfer.is_none() {
        request.stats.status.store(STATUS_READY, Ordering::Relaxed);
    }
    let encoded_lead = group
        .completed_steps
        .saturating_sub(request.stats.completed_steps());
    let pending = request
        .desired_steps
        .saturating_sub(group.completed_steps)
        .min(MAX_STEPS_PER_FRAME)
        .min(MAX_ENCODED_STEP_LEAD.saturating_sub(encoded_lead));
    let inject_now = request.pulse_serial != group.pulse_serial;
    if pending == 0 && !inject_now {
        return;
    }
    let mut pass = render_context
        .command_encoder()
        .begin_compute_pass(&ComputePassDescriptor {
            label: Some("wave evolution"),
            ..default()
        });
    pass.set_bind_group(0, &group.bind_group, &[]);
    if inject_now {
        pass.set_pipeline(inject);
        pass.dispatch_workgroups(workgroups, 1, 1);
        group.pulse_serial = request.pulse_serial;
    }
    let probe_pipeline = probe_group
        .as_ref()
        .filter(|probe_group| {
            probe_group.generation == request.generation
                && probe_group.revision == request.probe_revision
        })
        .and_then(|_| pipeline_cache.get_compute_pipeline(pipeline.probe));
    let curve_probe_pipeline = curve_probe_group
        .as_ref()
        .filter(|curve_probe_group| {
            curve_probe_group.generation == request.generation
                && curve_probe_group.revision == request.curve_probe_revision
        })
        .and_then(|_| pipeline_cache.get_compute_pipeline(pipeline.curve_probe));
    let area_probe_pipelines = area_probe_group
        .as_ref()
        .filter(|area_probe_group| {
            area_probe_group.generation == request.generation
                && area_probe_group.revision == request.area_probe_revision
        })
        .and_then(|_| {
            Some((
                pipeline_cache.get_compute_pipeline(pipeline.area_probe_elements)?,
                pipeline_cache.get_compute_pipeline(pipeline.area_probe_reduce)?,
            ))
        });
    let far_field_pipelines = far_field_group
        .as_ref()
        .filter(|far_field_group| {
            far_field_group.generation == request.generation
                && far_field_group.revision == request.far_field_revision
        })
        .and_then(|_| {
            Some((
                pipeline_cache.get_compute_pipeline(pipeline.far_field_sample)?,
                pipeline_cache.get_compute_pipeline(pipeline.far_field_project)?,
            ))
        });
    for offset in 0..pending {
        pass.set_pipeline(step);
        pass.dispatch_workgroups(workgroups, 1, 1);
        pass.set_pipeline(rotate);
        pass.dispatch_workgroups(workgroups, 1, 1);
        let step_after = group.completed_steps + offset + 1;
        if let Some((filter_stage, filter_apply)) = filter
            && step_after.is_multiple_of(GRID_SCALE_FILTER_CADENCE)
        {
            pass.set_pipeline(filter_stage);
            pass.dispatch_workgroups(workgroups, 1, 1);
            pass.set_pipeline(filter_apply);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }
        if request
            .probes
            .as_ref()
            .is_some_and(|probes| probe_sample_due(step_after, probes.sample_stride))
            && let (Some(probe_pipeline), Some(probe_group)) =
                (probe_pipeline, probe_group.as_ref())
        {
            pass.set_pipeline(probe_pipeline);
            pass.set_bind_group(0, &probe_group.bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_bind_group(0, &group.bind_group, &[]);
        }
        if request.curve_probes.as_ref().is_some_and(|probes| {
            probes
                .sample_strides
                .iter()
                .any(|stride| probe_sample_due(step_after, *stride))
        }) && let (Some(curve_probe_pipeline), Some(curve_probe_group), Some(curve_probes)) = (
            curve_probe_pipeline,
            curve_probe_group.as_ref(),
            request.curve_probes.as_ref(),
        ) {
            pass.set_pipeline(curve_probe_pipeline);
            pass.set_bind_group(0, &curve_probe_group.bind_group, &[]);
            pass.dispatch_workgroups(curve_probes.point_count.div_ceil(64), 1, 1);
            pass.set_bind_group(0, &group.bind_group, &[]);
        }
        if request
            .area_probes
            .as_ref()
            .is_some_and(|probes| probe_sample_due(step_after, probes.sample_stride))
            && let (Some((elements, reduce)), Some(area_group), Some(area_probes)) = (
                area_probe_pipelines,
                area_probe_group.as_ref(),
                request.area_probes.as_ref(),
            )
        {
            pass.set_bind_group(0, &area_group.bind_group, &[]);
            if area_probes.contribution_count > 0 {
                pass.set_pipeline(elements);
                pass.dispatch_workgroups(area_probes.contribution_count.div_ceil(64), 1, 1);
            }
            pass.set_pipeline(reduce);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_bind_group(0, &group.bind_group, &[]);
        }
        if request
            .far_field
            .as_ref()
            .is_some_and(|far_field| probe_sample_due(step_after, far_field.sample_stride))
            && let (Some((sample, project)), Some(far_field_group)) =
                (far_field_pipelines, far_field_group.as_ref())
        {
            pass.set_bind_group(0, &far_field_group.bind_group, &[]);
            pass.set_pipeline(sample);
            pass.dispatch_workgroups((FAR_FIELD_CONTOUR_POINTS as u32).div_ceil(64), 1, 1);
            pass.set_pipeline(project);
            pass.dispatch_workgroups((FAR_FIELD_DIRECTIONS as u32).div_ceil(64), 1, 1);
            pass.set_bind_group(0, &group.bind_group, &[]);
        }
    }
    drop(pass);
    group.completed_steps += pending;
    request
        .stats
        .completed_steps
        .store(group.completed_steps, Ordering::Relaxed);
    request.stats.dispatches.fetch_add(
        pending.saturating_mul(2) + u64::from(inject_now),
        Ordering::Relaxed,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::world::CommandQueue;
    use std::collections::BTreeMap;

    use funfern_core::{
        CurveId, CurveNode, CurveSpan, CurveSpanId, CurveSpline, FaceRegionAssignment,
        MeshingOptions, OpenCubicSpline, OuterSide, Region, Scene, SpanBehavior, TopologyCurve,
        TopologyGeometry, TopologyMeshPlan, TopologyVertex, TopologyVertexId,
        TopologyVertexLocation, TopologyWaveModel, VolumeSourceContribution, VolumeSourceNode,
        compile_topology, mesh_topology_plan,
    };

    fn four_region_topology() -> (TriMesh, QuadraticWaveOperator) {
        let ids = [
            TopologyVertexId(1),
            TopologyVertexId(2),
            TopologyVertexId(3),
            TopologyVertexId(4),
        ];
        let divider = |id, points: [Point2; 2], endpoints: [TopologyVertexId; 2]| {
            let mut curve = TopologyCurve::new(
                CurveId(id),
                CurveSpline::Open(OpenCubicSpline::polyline(points.to_vec()).unwrap()),
                vec![CurveSpan {
                    id: CurveSpanId(id),
                    behavior: SpanBehavior::Transmitting,
                }],
            )
            .unwrap();
            curve.nodes = endpoints
                .map(|vertex| CurveNode {
                    vertex: Some(vertex),
                })
                .to_vec();
            curve
        };
        let geometry = TopologyGeometry {
            curves: vec![
                divider(
                    1,
                    [Point2::new(-1.0, 0.0), Point2::new(1.0, 0.0)],
                    [ids[0], ids[1]],
                ),
                divider(
                    2,
                    [Point2::new(0.0, -1.0), Point2::new(0.0, 1.0)],
                    [ids[2], ids[3]],
                ),
            ],
            vertices: [
                (ids[0], OuterSide::Left),
                (ids[1], OuterSide::Right),
                (ids[2], OuterSide::Bottom),
                (ids[3], OuterSide::Top),
            ]
            .map(|(id, side)| TopologyVertex {
                id,
                location: TopologyVertexLocation::Outer {
                    side,
                    fraction: 0.5,
                },
            })
            .to_vec(),
            ..TopologyGeometry::default()
        };
        let snapshot = compile_topology(&geometry, 7).unwrap();
        let assignments = snapshot
            .faces
            .iter()
            .enumerate()
            .map(|(index, face)| FaceRegionAssignment {
                face: face.id,
                region: Some(RegionId(index as u64 + 1)),
            })
            .collect::<Vec<_>>();
        let plan = TopologyMeshPlan::new(&snapshot, &assignments).unwrap();
        let mesh = mesh_topology_plan(
            &plan,
            9,
            MeshingOptions {
                target_edge_length: 0.35,
                minimum_angle_degrees: 8.0,
                max_vertices: 20_000,
                max_triangles: 40_000,
                max_refinement_steps: 20_000,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let scene = Scene {
            regions: assignments
                .iter()
                .map(|assignment| Region {
                    id: assignment.region.unwrap(),
                    material: funfern_core::DEFAULT_MATERIAL,
                    frame: funfern_core::MaterialFrame::world(),
                })
                .collect(),
            ..Scene::default()
        };
        let operator = QuadraticWaveOperator::assemble_topology(
            &mesh,
            &plan,
            TopologyWaveModel::from_scene(&scene),
        )
        .unwrap();
        (mesh, operator)
    }

    #[test]
    fn gpu_upload_accepts_a_node_in_four_regions_and_tracks_source_membership() {
        let (mesh, operator) = four_region_topology();
        let memberships = node_regions(&mesh, &operator).unwrap();
        let center = operator
            .node_points()
            .iter()
            .position(|point| *point == Point2::default())
            .unwrap();
        assert_eq!(memberships[center].len(), 4);
        let packed = gpu_nodes(&mesh, &operator, RegionId(4)).unwrap();
        assert_eq!(packed[center].source_membership.x, 1);
        let outside = memberships
            .iter()
            .position(|regions| !regions.contains(&RegionId(4)))
            .unwrap();
        assert_eq!(packed[outside].source_membership.x, 0);

        let sources = CompiledVolumeSources::empty(operator.degrees_of_freedom());
        let source = PointSource {
            region: RegionId(4),
            ..PointSource::default()
        };
        let mut assets = Assets::<ShaderBuffer>::default();
        let (handles, dof_count) = create_buffers(
            &mut assets,
            &mesh,
            &operator,
            operator.recommended_time_step(),
            source,
            &sources,
            false,
        )
        .unwrap();
        let original_nodes = handles.nodes.id();
        let mut request = WaveGpuRequest {
            buffers: Some(handles),
            dof_count,
            ..Default::default()
        };
        request
            .update_source(
                &mut assets,
                &mesh,
                &operator,
                PointSource {
                    region: RegionId(2),
                    ..source
                },
            )
            .unwrap();
        assert_ne!(request.buffers.as_ref().unwrap().nodes.id(), original_nodes);
    }

    #[test]
    fn wave_shaders_do_not_encode_a_fixed_region_membership_list() {
        let wave = include_str!("wave.wgsl");
        assert!(!wave.contains("region_match"));
        for transfer in [
            include_str!("wave_transfer_old.wgsl"),
            include_str!("wave_transfer_new.wgsl"),
        ] {
            assert!(transfer.contains("source_membership: vec4<u32>"));
            assert!(transfer.contains("nodes[i].source_membership.x != 0u"));
            assert!(!transfer.contains("region_match"));
        }
    }

    #[test]
    fn source_upload_uses_angular_frequency_and_squared_width() {
        let source = gpu_source(PointSource {
            enabled: true,
            position: Point2::new(0.2, -0.3),
            width: 0.04,
            region: funfern_core::BACKGROUND_REGION,
            signal: TimeSignal::harmonic(-0.2, 7.0, 2.5, 0.3),
        });
        assert_eq!(source.position_width_enabled.x, 0.2);
        assert_eq!(source.position_width_enabled.y, -0.3);
        assert!((source.position_width_enabled.z - 0.0016).abs() < 1.0e-9);
        assert_eq!(source.position_width_enabled.w, 1.0);
        assert_eq!(source.signal.values.x, -0.2);
        assert_eq!(source.signal.values.y, 7.0);
        assert!((source.signal.values.z - 5.0 * std::f32::consts::PI).abs() < 1.0e-6);
        assert_eq!(source.signal.values.w, 0.3);
        assert_eq!(source.signal.extra, Vec4::ZERO);
    }

    #[test]
    fn volume_sources_share_the_existing_forcing_weight_binding() {
        let sources = CompiledVolumeSources {
            signals: vec![TimeSignal::Harmonic {
                offset: -0.25,
                amplitude: 3.0,
                frequency_hz: 2.5,
                phase_radians: 0.4,
            }],
            nodes: vec![VolumeSourceNode {
                contributions: vec![VolumeSourceContribution {
                    channel: 0,
                    weight: 0.75,
                }],
            }],
        };
        let forcing = gpu_forcing(
            PointSource::default(),
            PulseSettings {
                position: Point2::default(),
                amplitude: 0.0,
                width: 1.0,
                region: funfern_core::BACKGROUND_REGION,
            },
            OuterBoundaryConditions::default(),
            &sources,
        )
        .unwrap();
        assert_eq!(forcing.volume[0].values.x, -0.25);
        assert_eq!(forcing.volume[0].values.y, 3.0);
        assert!((forcing.volume[0].values.z - 5.0 * std::f32::consts::PI).abs() < 1.0e-5);
        let weights = zip_forcing_weights(&[0.2], &[0.3], &sources).unwrap();
        assert_eq!(f32::from_bits(weights[0].data.x), 0.2);
        assert_eq!(f32::from_bits(weights[0].data.y), 0.3);
        assert_eq!(weights[0].data.z, 1);
        assert_eq!(weights[0].data.w, 1);
        assert_eq!(weights[1].data.x, 1);
        assert_eq!(f32::from_bits(weights[1].data.y), 0.75);
    }

    #[test]
    fn forcing_weight_buffer_packs_arbitrary_junction_channels_sparsely() {
        let sources = CompiledVolumeSources {
            signals: vec![TimeSignal::ZERO; 4],
            nodes: vec![VolumeSourceNode {
                contributions: (0..4)
                    .map(|channel| VolumeSourceContribution {
                        channel,
                        weight: channel as f64 + 0.25,
                    })
                    .collect(),
            }],
        };
        let packed = zip_forcing_weights(&[0.2], &[0.3], &sources).unwrap();
        assert_eq!(packed.len(), 3);
        assert_eq!(
            packed[0].data,
            UVec4::new(0.2_f32.to_bits(), 0.3_f32.to_bits(), 1, 4)
        );
        assert_eq!(packed[1].data.x, 1);
        assert_eq!(f32::from_bits(packed[1].data.y), 0.25);
        assert_eq!(packed[1].data.z, 2);
        assert_eq!(f32::from_bits(packed[1].data.w), 1.25);
        assert_eq!(packed[2].data.x, 3);
        assert_eq!(f32::from_bits(packed[2].data.y), 2.25);
        assert_eq!(packed[2].data.z, 4);
        assert_eq!(f32::from_bits(packed[2].data.w), 3.25);
    }

    #[test]
    fn temporal_source_updates_retain_spatial_weight_buffers() {
        use funfern_core::{MeshingOptions, Scene, mesh_scene};

        let scene = Scene::default();
        let mesh = mesh_scene(
            &scene,
            3,
            MeshingOptions {
                target_edge_length: 0.4,
                minimum_angle_degrees: 10.0,
                ..Default::default()
            },
        )
        .unwrap();
        let operator = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            funfern_core::OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let volume_sources = CompiledVolumeSources {
            signals: vec![TimeSignal::harmonic(0.0, 1.0, 2.0, 0.0)],
            nodes: vec![VolumeSourceNode::default(); operator.degrees_of_freedom()],
        };
        let mut assets = Assets::<ShaderBuffer>::default();
        let source = PointSource {
            enabled: true,
            ..Default::default()
        };
        let (handles, dof_count) = create_buffers(
            &mut assets,
            &mesh,
            &operator,
            operator.recommended_time_step(),
            source,
            &volume_sources,
            false,
        )
        .unwrap();
        let original_weights = handles.forcing_weights.id();
        let original_forcing = handles.forcing.id();
        let original_nodes = handles.nodes.id();
        let mut request = WaveGpuRequest {
            buffers: Some(handles),
            dof_count,
            ..Default::default()
        };

        request
            .update_source(
                &mut assets,
                &mesh,
                &operator,
                PointSource {
                    signal: TimeSignal::harmonic(0.25, 3.0, 4.0, 0.5),
                    ..source
                },
            )
            .unwrap();
        let handles = request.buffers.as_ref().unwrap();
        assert_eq!(handles.forcing_weights.id(), original_weights);
        assert_ne!(handles.forcing.id(), original_forcing);
        assert_eq!(handles.nodes.id(), original_nodes);

        let point_forcing = handles.forcing.id();
        request
            .update_volume_source_signals(
                &mut assets,
                &operator,
                vec![TimeSignal::harmonic(-0.5, 2.5, 5.0, -0.25)],
            )
            .unwrap();
        let handles = request.buffers.as_ref().unwrap();
        assert_eq!(handles.forcing_weights.id(), original_weights);
        assert_ne!(handles.forcing.id(), point_forcing);
        assert_eq!(
            handles.volume_sources.signals,
            vec![TimeSignal::harmonic(-0.5, 2.5, 5.0, -0.25)]
        );
    }

    #[test]
    fn open_boundary_forcing_uses_mesh_path_distance() {
        use funfern_core::*;

        let mut scene = Scene::default();
        scene.internal_boundaries.push(InternalBoundary {
            id: InternalBoundaryId(4),
            spline: OpenCubicSpline::uniform(vec![
                Point2::new(-0.72, 0.0),
                Point2::new(-0.25, 0.0),
                Point2::new(0.25, 0.0),
                Point2::new(0.72, 0.0),
            ])
            .unwrap(),
            region: BACKGROUND_REGION,
            span_laws: vec![InternalBoundaryLaw::REFLECTING],
        });
        let mesh = mesh_scene(
            &scene,
            9,
            MeshingOptions {
                target_edge_length: 0.18,
                minimum_angle_degrees: 10.0,
                ..Default::default()
            },
        )
        .unwrap();
        let operator = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let weights = forcing_weights(
            &mesh,
            &operator,
            Point2::new(0.0, 0.05),
            0.06,
            BACKGROUND_REGION,
        )
        .unwrap();
        let closest = |side| {
            mesh.vertices
                .iter()
                .enumerate()
                .filter(|(_, vertex)| {
                    matches!(
                        vertex.boundary,
                        Some(BoundaryPoint {
                            label: BoundaryLabel::InternalBoundary {
                                id: InternalBoundaryId(4),
                                side: candidate,
                            },
                            ..
                        }) if candidate == side
                    )
                })
                .min_by(|(_, a), (_, b)| a.point.x.abs().total_cmp(&b.point.x.abs()))
                .unwrap()
                .0
        };
        let left = closest(InternalBoundarySide::Left);
        let right = closest(InternalBoundarySide::Right);
        assert!(weights[left] > 0.4);
        assert_eq!(weights[right], 0.0);
        assert_eq!(mesh.vertices[left].point, mesh.vertices[right].point);

        let mut topology_mesh = mesh.clone();
        for edge in &mut topology_mesh.boundary_edges {
            let BoundaryLabel::InternalBoundary { side, .. } = edge.label else {
                continue;
            };
            edge.label = BoundaryLabel::Curve {
                curve: funfern_core::CurveId(4),
                span: funfern_core::CurveSpanId(1),
                side: match side {
                    InternalBoundarySide::Left => funfern_core::CurveTraceSide::Left,
                    InternalBoundarySide::Right => funfern_core::CurveTraceSide::Right,
                },
                separated: true,
            };
        }
        let topology_weights = forcing_weights(
            &topology_mesh,
            &operator,
            Point2::new(0.0, 0.05),
            0.06,
            BACKGROUND_REGION,
        )
        .unwrap();
        assert_eq!(topology_weights, weights);
    }

    #[test]
    fn solver_clock_sampling_is_independent_of_frame_batches() {
        let sample_steps = |batches: &[u64], stride: u64| {
            let mut completed = 0;
            let mut samples = Vec::new();
            for batch in batches {
                for offset in 0..*batch {
                    let step = completed + offset + 1;
                    if probe_sample_due(step, stride) {
                        samples.push(step);
                    }
                }
                completed += batch;
            }
            samples
        };
        assert_eq!(
            sample_steps(&[64, 36], 7),
            sample_steps(&[3, 19, 1, 40, 37], 7)
        );
        assert_eq!(
            sample_steps(&[100], 7),
            (7..=98).step_by(7).collect::<Vec<_>>()
        );
    }

    #[test]
    fn probe_shader_uses_the_continuous_transferred_solver_clock() {
        let shader = include_str!("probe.wgsl");
        assert!(shader.contains("let time = parameters.time_data.z - parameters.time_data.x;"));
        assert!(!shader.contains("control.values.w + parameters.time_data.z"));
        assert!(shader.contains("stencil.material.x * displacement * displacement"));
        assert!(shader.contains("dot(gradient, flux)"));
        assert!(shader.contains("length(potential_flux)"));
        assert!(shader.contains("let poynting = select(0.0, abs(displacement) * transverse"));
    }

    #[test]
    fn canonical_probe_shaders_consume_direct_accepted_state() {
        let point = include_str!("canonical_probe.wgsl");
        assert!(point.contains("if !temporal_enabled() { return nodes[node].mass_loss.y; }"));
        assert!(point.contains("accepted_q(a.x) * primary_inverse_mass(a.x, control.clock_f32.y)"));
        assert!(point.contains("previous_q(a.x) * primary_inverse_mass(a.x, previous_time)"));
        assert!(point.contains("return select(value.xy, value.zw"));
        assert!(point.contains("dot(flux, complement)"));
        assert!(point.contains("orientation.x * primary.x"));
        assert!(point.contains("fn sample_vector_overlay"));
        assert!(point.contains("output[sample].primary = vec4<f32>(complement, flow)"));
        assert!(point.contains("vec4<u32>(control.clock_u32.w, 1u,"));
        assert!(point.contains("bitcast<u32>(control.clock_origin.x)"));
        assert!(point.contains("bitcast<u32>(control.clock_origin.y + control.clock_f32.y)"));
        assert!(!point.contains("indicator_potential"));

        let curve = include_str!("canonical_curve_probe.wgsl");
        assert!(curve.contains("dot(flow, record.normal_stride_valid.xy)"));
        assert!(curve.contains("control.clock_u32.w % stride"));

        let area = include_str!("canonical_area_probe.wgsl");
        assert!(area.contains("fn sample_area_elements"));
        assert!(area.contains("fn reduce_area_probes"));
        assert!(area.contains("weight * dot(complement, complement)"));
        assert!(area.contains("total_energy / covered_area"));
    }

    #[test]
    fn canonical_far_field_uses_primary_field_without_inverse_derivative_state() {
        let shader = include_str!("canonical_far_field.wgsl");
        assert!(shader.contains("accepted_q(a.x) * nodes[a.x].mass_loss.y"));
        assert!(shader.contains("let rate = (primary - previous) / control.clock_f32.x;"));
        assert!(
            shader.contains("let normal_gradient = dot(gradient, stencil.position_normal.zw);")
        );
        assert!(shader.contains("sample.z - dot(normal, ray) * sample.y / wave_speed"));
        assert!(!shader.contains("potential"));
    }

    /// One envelope multiplies the whole source term, so a phased array's
    /// sources ease in together and the phases between them — which are what
    /// steer the beam — are untouched. A prescribed boundary load is left
    /// outside it.
    #[test]
    fn every_source_is_eased_in_by_one_shared_envelope() {
        let wave = include_str!("wave.wgsl");
        assert!(wave.contains("fn source_envelope(time: f32) -> f32 {"));
        assert!(wave.contains(
            "    let acceleration = source_envelope(parameters.time_data.z)\n        * (forcing.source.position_width_enabled.w"
        ));
        assert!(wave.contains("            + volume_acceleration(i, parameters.time_data.z))"));
        assert!(wave.contains("        + neumann_acceleration(i, parameters.time_data.z);"));
        // The same smoothstep `funfern_core::source_envelope` evaluates.
        assert!(wave.contains("    return fraction * fraction * (3.0 - 2.0 * fraction);"));
        assert!(wave.contains("    envelope: vec4<f32>,"));
    }

    /// The ramp reaches the shader from the scene's slowest oscillating source,
    /// and a scene with nothing oscillating asks for none.
    #[test]
    fn the_uploaded_ramp_follows_the_slowest_source() {
        let sources = CompiledVolumeSources {
            signals: vec![TimeSignal::harmonic(0.0, 3.0, 8.0, 0.0)],
            nodes: vec![],
        };
        let source = PointSource {
            enabled: true,
            signal: TimeSignal::harmonic(0.0, 5.0, 2.0, 0.0),
            ..PointSource::default()
        };
        let boundaries = OuterBoundaryConditions::default();
        let forcing = gpu_forcing(source, gpu_pulse_settings(), boundaries, &sources).unwrap();
        assert!((f64::from(forcing.envelope.x) - source_ramp_seconds(2.0)).abs() < 1.0e-6);

        // A disabled point source does not get a say in the ramp.
        let silent = PointSource {
            enabled: false,
            ..source
        };
        let forcing = gpu_forcing(silent, gpu_pulse_settings(), boundaries, &sources).unwrap();
        assert!((f64::from(forcing.envelope.x) - source_ramp_seconds(8.0)).abs() < 1.0e-6);

        let none = CompiledVolumeSources {
            signals: vec![],
            nodes: vec![],
        };
        let forcing = gpu_forcing(silent, gpu_pulse_settings(), boundaries, &none).unwrap();
        assert_eq!(forcing.envelope.x, 0.0);
    }

    fn gpu_pulse_settings() -> PulseSettings {
        PulseSettings {
            position: Point2::default(),
            amplitude: 0.65,
            width: 0.06,
            region: funfern_core::BACKGROUND_REGION,
        }
    }

    /// The shader pair and the CPU mirror have to stay the same arithmetic, and
    /// the one parameter buffer has to have one layout across every shader that
    /// binds it.
    #[test]
    fn the_grid_scale_filter_passes_match_the_solver_they_mirror() {
        let wave = include_str!("wave.wgsl");
        for shader in [
            wave,
            include_str!("probe.wgsl"),
            include_str!("curve_probe.wgsl"),
            include_str!("area_probe.wgsl"),
            include_str!("far_field.wgsl"),
            include_str!("wave_transfer_old.wgsl"),
            include_str!("wave_transfer_new.wgsl"),
        ] {
            assert!(shader.contains("filter_data: vec4<f32>"));
        }
        assert!(wave.contains("fn filter_stage("));
        assert!(wave.contains("fn filter_apply("));
        // Both gathers are differences against this node, which is what makes
        // the filter exactly zero on a constant field rather than merely small.
        assert!(wave.contains(
            "            * ((states[column].levels.y - states[column].levels.x) - difference);"
        ));
        assert!(
            wave.contains("            * (states[column].reconstruction.w - staged);"),
            "the apply pass must gather differences of the staged values"
        );
        // Symmetric about the midpoint, so a static field does not move.
        assert!(wave.contains("    states[i].levels.y -= half;"));
        assert!(wave.contains("    states[i].levels.x += half;"));
        // A prescribed node carries its boundary condition, not a solution.
        assert!(wave.contains("    if nodes[i].boundary.x != 0u {\n        return;\n    }"));
    }

    /// The handoff shaders evaluate the same stiffness product the step does,
    /// and it sets the velocity the next generation starts from, so a rounding
    /// residual there becomes a mean velocity that generation keeps.
    #[test]
    fn the_transfer_shaders_evaluate_the_stiffness_on_differences() {
        let old = include_str!("wave_transfer_old.wgsl");
        assert!(old.contains(
            "        ku += coefficients.x * (states[column].levels.y - displacement)\n            + coefficients.y * (states[column].auxiliary.x - memory);"
        ));
        let new = include_str!("wave_transfer_new.wgsl");
        assert!(new.contains(
            "        ku += coefficients.x * (transfers[column].mapped.y - current)\n            + coefficients.y * (transfers[column].mapped.z - memory);"
        ));
    }

    #[test]
    fn wave_reconstruction_rejects_dc_and_transfers_both_filter_stages() {
        let wave = include_str!("wave.wgsl");
        assert!(wave.contains("states[i].reconstruction.y = states[i].reconstruction.x"));
        assert!(wave.contains("let stage_a_next = ("));
        assert!(wave.contains("let stage_b_next = ("));
        let old_transfer = include_str!("wave_transfer_old.wgsl");
        assert!(
            old_transfer.contains(
                "mapped_reconstruction_a += weight * states[source_index].reconstruction.x"
            )
        );
        assert!(
            old_transfer.contains(
                "mapped_reconstruction_b += weight * states[source_index].reconstruction.z"
            )
        );
        let new_transfer = include_str!("wave_transfer_new.wgsl");
        assert!(new_transfer.contains("let reconstruction_b = transfers[i].auxiliary.y"));
        for shader in [
            include_str!("probe.wgsl"),
            include_str!("curve_probe.wgsl"),
            include_str!("area_probe.wgsl"),
            include_str!("far_field.wgsl"),
        ] {
            assert!(shader.contains("reconstruction: vec4<f32>"));
        }

        let decay = 0.5;
        let dt = 0.01;
        let q = 0.5 * decay * dt;
        let advance = |stage_a: f64, stage_b: f64, previous: f64, next: f64| {
            let next_a = ((1.0 - q) * stage_a + 0.5 * dt * (previous + next)) / (1.0 + q);
            let next_b = ((1.0 - q) * stage_b + 0.5 * dt * (stage_a + next_a)) / (1.0 + q);
            (next_a, next_b)
        };
        let mut stage_a = 0.0;
        let mut stage_b = 0.0;
        for _ in 0..10_000 {
            (stage_a, stage_b) = advance(stage_a, stage_b, 1.0, 1.0);
        }
        assert!((stage_a - decay * stage_b).abs() < 1.0e-12);

        let angular_frequency = std::f64::consts::TAU * 3.0;
        stage_a = 0.0;
        stage_b = 0.0;
        let mut error_squared = 0.0;
        let mut reference_squared = 0.0;
        for step in 0..2_000 {
            let time = step as f64 * dt;
            let previous = (angular_frequency * time).sin();
            let next = (angular_frequency * (time + dt)).sin();
            (stage_a, stage_b) = advance(stage_a, stage_b, previous, next);
            if step >= 1_000 {
                let reconstructed = stage_a - decay * stage_b;
                let reference = -(angular_frequency * (time + dt)).cos() / angular_frequency;
                error_squared += (reconstructed - reference).powi(2);
                reference_squared += reference.powi(2);
            }
        }
        assert!((error_squared / reference_squared).sqrt() < 0.06);
    }

    #[test]
    fn curve_probe_shader_records_profile_energy_flux_and_gaps() {
        let shader = include_str!("curve_probe.wgsl");
        assert!(shader.contains("let tensor_flux = vec2<f32>"));
        assert!(shader.contains("-velocity * dot(tensor_flux, normal)"));
        assert!(shader.contains("-displacement * dot(potential_flux, normal)"));
        assert!(shader.contains("let transverse = select("));
        assert!(shader.contains("bitcast<f32>(0x7fc00000u | (point & 1u))"));
        assert!(shader.contains("let time = parameters.time_data.z - parameters.time_data.x;"));
    }

    #[test]
    fn curve_probe_presets_have_independent_solver_step_strides() {
        let time_step = 0.001;
        let strides =
            [30.0, 60.0, 120.0].map(|rate| (1.0_f64 / (rate * time_step)).round().max(1.0) as u64);
        assert_eq!(strides, [33, 17, 8]);
        assert!(probe_sample_due(264, strides[0]));
        assert!(!probe_sample_due(264, strides[1]));
        assert!(probe_sample_due(264, strides[2]));
    }

    #[test]
    fn area_probe_upload_preserves_integrated_element_matrices() {
        let element = QuadraticAreaElement {
            element: 0,
            nodes: [0, 1, 2, 3, 4, 5, 6],
            barycentric_vertices: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            barycentric_gradients: [
                Point2::new(-1.0, -1.0),
                Point2::new(1.0, 0.0),
                Point2::new(0.0, 1.0),
            ],
            region: funfern_core::BACKGROUND_REGION,
            coefficients: [funfern_core::DirectionalWaveCoefficients {
                mass_density: 2.0,
                stiffness: funfern_core::SymmetricTensor2::isotropic(3.0),
                damping: 0.0,
            }; 12],
            area: 0.5,
        };
        let matrices = element.integrated_matrices();
        let uploaded = gpu_area_probe_contribution(element, PhysicsModel::Mechanical);
        assert_eq!(
            uploaded.field_a.to_array(),
            [
                matrices.field[0] as f32,
                matrices.field[1] as f32,
                matrices.field[2] as f32,
                matrices.field[3] as f32,
            ]
        );
        assert_eq!(
            uploaded.field_b.to_array(),
            [
                matrices.field[4] as f32,
                matrices.field[5] as f32,
                matrices.field[6] as f32,
                0.0,
            ]
        );
        let unpack = |blocks: [Vec4; 7]| {
            blocks
                .into_iter()
                .flat_map(|block| block.to_array())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            &unpack(uploaded.mass)[..28],
            &matrices.mass.map(|v| v as f32)
        );
        assert_eq!(
            &unpack(uploaded.density)[..28],
            &matrices.density.map(|v| v as f32)
        );
        assert_eq!(
            &unpack(uploaded.stiffness)[..28],
            &matrices.stiffness.map(|v| v as f32)
        );
        assert_eq!(
            &unpack(uploaded.stiffness_squared)[..28],
            &matrices.stiffness_squared.map(|v| v as f32)
        );
    }

    #[test]
    fn area_probe_shader_reduces_to_compact_physical_records() {
        let shader = include_str!("area_probe.wgsl");
        assert!(shader.contains("fn sample_area_elements"));
        assert!(shader.contains("fn reduce_area_probes"));
        assert!(shader.contains("let mean = accumulated_primary.x / covered_area"));
        assert!(shader.contains("let rms = sqrt(max(accumulated_primary.y / covered_area, 0.0))"));
        assert!(
            shader.contains(
                "let rms_transverse = sqrt(max(accumulated_primary.z / covered_area, 0.0))"
            )
        );
        assert!(shader.contains("bitcast<f32>(0x7fc00000u | (probe & 1u))"));
        assert!(shader.contains("let time = parameters.time_data.z - parameters.time_data.x;"));
    }

    #[test]
    fn far_field_shader_uses_retarded_time_and_huygens_data() {
        let shader = include_str!("far_field.wgsl");
        assert!(shader.contains("fn sample_contour"));
        assert!(shader.contains("fn project_directions"));
        assert!(shader.contains("let age = delay / period;"));
        assert!(shader.contains("sample.z - dot(normal, ray) * sample.y / wave_speed"));
        assert!(shader.contains("amplitude * amplitude"));
    }

    #[test]
    fn far_field_validity_never_rests_on_a_nan_comparison() {
        let shader = include_str!("far_field.wgsl");
        // `w != w` is a NaN test, and these shaders compile under fast math,
        // where it is folded to `false` and every unwritten frame reads as data.
        assert!(!shader.contains("!= newer.w"));
        assert!(!shader.contains("!= older.w"));
        assert!(!shader.contains("recorded == recorded"));
        assert!(shader.contains("const UNRECORDED: f32 = -1.0e30;"));
        assert!(shader.contains("if newer.w < 0.0 || older.w < 0.0 {"));
    }

    #[test]
    fn far_field_shader_keys_its_ring_to_the_solver_clock_and_recorded_times() {
        let shader = include_str!("far_field.wgsl");
        // The ring is a function of the clock, not of this generation's steps,
        // which is what lets a new mesh keep writing into an old ring. That
        // clock is the transferred one every other probe records against, and
        // adding an offset of the app's own would count it twice.
        assert!(shader.contains("return parameters.time_data.z - parameters.time_data.x;"));
        assert!(!shader.contains("control.projection.w + parameters.time_data.z"));
        assert!(!shader.contains("completed % stride"));
        assert!(!shader.contains("let frame = (completed / stride) % frames;"));
        // And the projection reads by recorded time, so buckets filled at one
        // time step still bracket correctly after another one takes over.
        assert!(shader.contains("if retarded > newer.w && index > 0u {"));
        assert!(shader.contains("fraction = clamp((newer.w - retarded) / span, 0.0, 1.0);"));
    }

    /// The recorder in the shader's terms: a dispatched step records the bucket
    /// its clock has reached, unless the ring frame for that bucket already
    /// holds a sample of it. Runs of this stand in for what the GPU does, at
    /// clocks the CPU cannot predict.
    #[derive(Default)]
    struct FarFieldRing {
        frames: usize,
        recorded: BTreeMap<usize, f64>,
    }

    impl FarFieldRing {
        fn record(&mut self, state_time: f64, period: f64) -> Option<f64> {
            let bucket = far_field_bucket(state_time, period);
            let frame = bucket.rem_euclid(self.frames as f64) as usize;
            if self
                .recorded
                .get(&frame)
                .is_some_and(|recorded| far_field_bucket(*recorded, period) == bucket)
            {
                return None;
            }
            self.recorded.insert(frame, state_time);
            Some(bucket)
        }
    }

    #[test]
    fn the_far_field_period_stays_put_unless_a_step_outgrows_it() {
        let base = 1.0 / FAR_FIELD_SAMPLE_RATE;
        // The time steps an adaptation moves between all share one period.
        for time_step in [1.0e-5, 1.0e-4, 3.7e-4, 1.0e-3, base] {
            assert_eq!(far_field_period(time_step), base);
        }
        for time_step in [0.02, 0.03, 0.05, 0.3] {
            let period = far_field_period(time_step);
            assert!(period >= time_step, "{time_step} outgrew {period}");
            assert!((period / base).log2().fract() < 1.0e-12);
            assert!(period * 0.5 < time_step.max(base));
        }
    }

    #[test]
    fn the_far_field_ring_records_every_bucket_once_across_a_handoff() {
        let period = far_field_period(1.0e-3);
        let mut ring = FarFieldRing {
            frames: FAR_FIELD_RING_FRAMES,
            ..FarFieldRing::default()
        };
        let mut buckets = Vec::new();
        // The solver's own clock accumulates a step at a time in f32 and runs
        // away from any arithmetic done over step counts, so the run below
        // drifts by more than a step and changes step mid-flight.
        let mut run = |origin: f64, time_step: f64, steps: u64, drift: f64| {
            let stride = far_field_sample_stride(period, time_step);
            for step in 1..=steps {
                if !probe_sample_due(step, stride) {
                    continue;
                }
                let state = origin + (step - 1) as f64 * time_step + drift * step as f64;
                if let Some(bucket) = ring.record(state, period) {
                    buckets.push(bucket);
                }
            }
        };
        let first_step = 1.0 / 700.0;
        let steps = 4000;
        run(0.0, first_step, steps, first_step * 0.002);
        // A finer mesh takes the ring over from where the first one stopped.
        let handoff = steps as f64 * first_step + first_step * 0.002 * steps as f64;
        run(handoff, 1.0 / 1100.0, 4000, 0.0);

        assert!(buckets.len() > 300);
        assert_eq!(buckets[0], 0.0);
        for pair in buckets.windows(2) {
            // A hole in the ring is a hole in every projection that reaches back
            // through it, and a bucket recorded twice is a sample of the wrong
            // moment standing in for the right one.
            assert_eq!(
                pair[1] - pair[0],
                1.0,
                "{:?} follows {:?}",
                pair[1],
                pair[0]
            );
        }
    }

    /// A new mesh over the same recorders keeps the rings they were filling.
    ///
    /// The samples the GPU wrote between the last readback and the handoff used
    /// to go with the buffers - two to four of them at 120 Hz, on every
    /// adaptation. They survive in the ring now, and the host takes records by
    /// time rather than by slot, so where the new stencils resume writing does
    /// not matter.
    #[test]
    fn a_new_mesh_over_the_same_probes_inherits_their_rings() {
        let mut world = World::new();
        let mut assets = Assets::<ShaderBuffer>::default();
        let mut request = WaveGpuRequest::default();
        let points = |ids: &[u64]| {
            ids.iter()
                .map(|id| (*id, None::<QuadraticPointStencil>))
                .collect::<Vec<_>>()
        };
        let curves = |id: u64, samples: usize| {
            vec![CurveProbeInput {
                id,
                sample_rate: 60.0,
                samples: vec![None; samples],
            }]
        };
        let areas = |ids: &[u64]| {
            ids.iter()
                .map(|id| AreaProbeInput {
                    id: *id,
                    stencil: None,
                })
                .collect::<Vec<_>>()
        };
        let mut update = |request: &mut WaveGpuRequest,
                          assets: &mut Assets<ShaderBuffer>,
                          point_ids: &[u64],
                          curve_samples: usize,
                          time_step: f64,
                          history: RecorderHistory| {
            let mut queue = CommandQueue::default();
            {
                let mut commands = Commands::new(&mut queue, &world);
                let context = RecorderContext {
                    time_step,
                    physics: PhysicsModel::Mechanical,
                    history,
                };
                request
                    .update_point_probes(assets, &mut commands, &points(point_ids), 120.0, context)
                    .unwrap();
                request
                    .update_curve_probes(assets, &mut commands, &curves(1, curve_samples), context)
                    .unwrap();
                request
                    .update_area_probes(assets, &mut commands, &areas(point_ids), 60.0, context)
                    .unwrap();
            }
            queue.apply(&mut world);
        };
        let rings = |request: &WaveGpuRequest| {
            (
                request.probes.as_ref().unwrap().output.id(),
                request.curve_probes.as_ref().unwrap().output.id(),
                request.area_probes.as_ref().unwrap().output.id(),
            )
        };
        let stencils = |request: &WaveGpuRequest| request.probes.as_ref().unwrap().stencils.id();

        update(
            &mut request,
            &mut assets,
            &[1, 2],
            8,
            1.0e-3,
            RecorderHistory::Keep,
        );
        let recorded = rings(&request);
        let first_stencils = stencils(&request);

        // An adaptation: the same probes read through a new mesh, at the shorter
        // step the finer elements ask for.
        update(
            &mut request,
            &mut assets,
            &[1, 2],
            8,
            7.0e-4,
            RecorderHistory::Keep,
        );
        assert_eq!(rings(&request), recorded, "the rings carried over");
        assert_ne!(stencils(&request), first_stencils, "the stencils did not");

        // A clock that restarted has nothing to hand over.
        update(
            &mut request,
            &mut assets,
            &[1, 2],
            8,
            7.0e-4,
            RecorderHistory::Restart,
        );
        assert_ne!(rings(&request), recorded);
        let recorded = rings(&request);

        // Nor does a recorder set that changed: the rings are addressed by probe
        // index and by sample offset, so what is in them no longer means what it
        // did.
        update(
            &mut request,
            &mut assets,
            &[1, 2, 3],
            8,
            7.0e-4,
            RecorderHistory::Keep,
        );
        assert_ne!(rings(&request).0, recorded.0, "a probe was added");
        assert_ne!(rings(&request).2, recorded.2, "and to the area ring too");
        let recorded = rings(&request);

        update(
            &mut request,
            &mut assets,
            &[1, 2, 3],
            9,
            7.0e-4,
            RecorderHistory::Keep,
        );
        assert_ne!(rings(&request).1, recorded.1, "the line probe was reshaped");
        assert_eq!(
            rings(&request).0,
            recorded.0,
            "which says nothing about the point ring"
        );
    }

    fn far_field_input(shift: f64) -> FarFieldInput {
        let stencil = QuadraticPointStencil {
            element: 0,
            barycentric: [1.0, 0.0, 0.0],
            nodes: [0; 7],
            value_weights: [0.0; 7],
            gradient_weights: [Point2::default(); 7],
            region: funfern_core::BACKGROUND_REGION,
            mass_density: 1.0,
            stiffness: funfern_core::SymmetricTensor2::isotropic(1.0),
        };
        let samples = (0..FAR_FIELD_CONTOUR_POINTS)
            .map(|point| {
                let angle = std::f64::consts::TAU * point as f64 / FAR_FIELD_CONTOUR_POINTS as f64;
                let normal = Point2::new(angle.cos(), angle.sin());
                (stencil, Point2::new(normal.x + shift, normal.y), normal)
            })
            .collect();
        FarFieldInput {
            samples,
            wave_speed: 1.0,
            sample_spacing: 0.02,
            delay_margin: 1.2,
        }
    }

    #[test]
    fn a_new_mesh_over_the_same_contour_inherits_the_far_field_ring() {
        let mut world = World::new();
        let mut assets = Assets::<ShaderBuffer>::default();
        let mut request = WaveGpuRequest::default();
        let keeping = RecorderHistory::Keep;
        let mut update = |request: &mut WaveGpuRequest,
                          assets: &mut Assets<ShaderBuffer>,
                          input: &FarFieldInput,
                          time_step: f64,
                          history: RecorderHistory| {
            let mut queue = CommandQueue::default();
            let handoff = request
                .update_far_field(
                    assets,
                    &mut Commands::new(&mut queue, &world),
                    Some(input),
                    time_step,
                    history,
                )
                .unwrap();
            queue.apply(&mut world);
            handoff
        };
        let ring = |request: &WaveGpuRequest| request.far_field.as_ref().unwrap().raw.id();
        let stencils = |request: &WaveGpuRequest| request.far_field.as_ref().unwrap().stencils.id();

        let input = far_field_input(0.0);
        assert_eq!(
            update(&mut request, &mut assets, &input, 1.0e-3, keeping),
            FarFieldHandoff::Restarted
        );
        let recorded = ring(&request);
        let first_stencils = stencils(&request);

        // An adaptation: the same contour read through a new mesh, at the
        // shorter step the finer elements ask for. The stencils are replaced and
        // the recording carries on.
        assert_eq!(
            update(&mut request, &mut assets, &input, 7.0e-4, keeping),
            FarFieldHandoff::Kept
        );
        assert_eq!(ring(&request), recorded);
        assert_ne!(stencils(&request), first_stencils);

        // A clock that restarted has nothing to hand over.
        assert_eq!(
            update(
                &mut request,
                &mut assets,
                &input,
                7.0e-4,
                RecorderHistory::Restart,
            ),
            FarFieldHandoff::Restarted
        );
        assert_ne!(ring(&request), recorded);
        let recorded = ring(&request);

        // And so does a contour that moved: what is recorded describes where the
        // samples were taken, not just when.
        assert_eq!(
            update(
                &mut request,
                &mut assets,
                &far_field_input(0.25),
                7.0e-4,
                keeping
            ),
            FarFieldHandoff::Restarted
        );
        assert_ne!(ring(&request), recorded);

        // Nothing is left behind by any of it.
        let mut queue = CommandQueue::default();
        request
            .update_far_field(
                &mut assets,
                &mut Commands::new(&mut queue, &world),
                None,
                7.0e-4,
                keeping,
            )
            .unwrap();
        queue.apply(&mut world);
        assert!(request.far_field.is_none());
        assert_eq!(assets.len(), 0);
    }

    #[test]
    fn the_far_field_dispatch_visits_every_bucket_it_has_to_record() {
        for time_step in [1.0e-4, 1.0 / 700.0, 0.004, 0.02, 0.4] {
            let period = far_field_period(time_step);
            let stride = far_field_sample_stride(period, time_step);
            let spacing = stride as f64 * time_step;
            assert!(
                spacing <= period,
                "{spacing} between passes leaves a bucket of {period} unvisited"
            );
            // Dense enough to land near the start of a bucket, sparse enough
            // that the projection is not re-run through every step of one.
            // Rounding a quarter bucket down to whole steps can halve it.
            assert!(spacing >= period * 0.125 || stride == 1);
        }
    }

    #[test]
    fn webgpu_probe_shaders_do_not_construct_nan_as_a_constant_expression() {
        for shader in [
            include_str!("curve_probe.wgsl"),
            include_str!("area_probe.wgsl"),
            include_str!("far_field.wgsl"),
            include_str!("canonical_curve_probe.wgsl"),
            include_str!("canonical_area_probe.wgsl"),
            include_str!("canonical_far_field.wgsl"),
        ] {
            assert!(!shader.contains("bitcast<f32>(0x7fc00000u)"));
        }
    }
}
