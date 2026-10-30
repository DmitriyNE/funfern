use std::{borrow::Cow, sync::Arc};

use bevy::{
    asset::{embedded_asset, load_embedded_asset},
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_resource::{
            BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
            CachedComputePipelineId, ComputePipelineDescriptor, PipelineCache, ShaderStages,
            ShaderType,
            binding_types::{storage_buffer, storage_buffer_read_only},
        },
        renderer::RenderDevice,
        storage::{GpuShaderBuffer, ShaderBuffer},
    },
};
use funfern_core::{
    CanonicalPointStencil, CanonicalTemporalPointStencil, CanonicalTemporalWaveOperator,
    CanonicalWaveOperator, GRID_SCALE_FILTER_CADENCE, PhysicsModel, Point2, QuadraticAreaElement,
    QuadraticAreaStencil, QuadraticPointStencil, canonical_area_contribution,
};

use crate::canonical_gpu::{
    CanonicalGpuLimits, CanonicalGpuRequest, CanonicalGpuTemporalManifest, GpuCanonicalControl,
    GpuCanonicalNode, GpuCanonicalStateWord, GpuCanonicalTableWord,
};
use crate::paced_readback::{PacedReadback, PacedReadbackComplete, PacedReadbackPlugin};

/// Independent complementary flux samples per enriched-quadratic element.
const COMPLEMENTARY_SAMPLES: usize = 6;
/// Nodes per enriched-quadratic element.
const LOCAL_NODES: usize = 7;
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
/// The canonical area recorder declares nine buffers across its two passes,
/// which is one more than a portable stage may bind. Neither entry point uses
/// all nine, so each takes its own layout over the same numbering; these are
/// the counts those layouts must respect.
const CANONICAL_AREA_ELEMENT_STORAGE_BINDINGS: usize = 7;
const CANONICAL_AREA_REDUCE_STORAGE_BINDINGS: usize = 5;
const WEBGPU_PORTABLE_STORAGE_BUFFER_LIMIT: usize = 8;
const _: () = {
    assert!(CANONICAL_AREA_ELEMENT_STORAGE_BINDINGS <= WEBGPU_PORTABLE_STORAGE_BUFFER_LIMIT);
    assert!(CANONICAL_AREA_REDUCE_STORAGE_BINDINGS <= WEBGPU_PORTABLE_STORAGE_BUFFER_LIMIT);
};

#[derive(Clone)]
pub(crate) struct ProbeBufferHandles {
    stencils: Handle<ShaderBuffer>,
    control: Handle<ShaderBuffer>,
    output: Handle<ShaderBuffer>,
    ids: Arc<[u64]>,
    pub(crate) sample_stride: u64,
    physics: PhysicsModel,
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

/// How often to encode a recorder pass. A pass records a frame once its time
/// stands a period past the ring's newest, so this only has to be dense enough
/// that frames stand little more than a period apart, and it need not track
/// the solver's own clock, which drifts from step times by more than a step
/// over a long run. A quarter period between passes keeps a frame within a
/// quarter period of being due.
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

#[derive(Resource, Clone, Default, ExtractResource)]
pub struct WaveGpuRequest {
    generation: u64,
    /// The device's buffer limits, copied in once it exists; none where no
    /// device is, as in host tests, which admits every recorder.
    limits: Option<CanonicalGpuLimits>,
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
}

impl WaveGpuRequest {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The strides the recorders run at, which their rings' frames follow.
    pub(crate) fn recorder_strides(&self) -> RecorderStrides {
        RecorderStrides {
            point: self.probes.as_ref().map(|handles| handles.sample_stride),
            curves: self
                .curve_probes
                .as_ref()
                .map_or_else(Vec::new, |handles| handles.sample_strides.to_vec()),
            area: self
                .area_probes
                .as_ref()
                .map(|handles| handles.sample_stride),
        }
    }

    /// Where the rings kept here hold what a source generation, at `strides`,
    /// recorded for the steps after `snapshot` through `last`: each a buffer,
    /// an offset and a length in bytes. A kept line ring has the source's
    /// descriptors, its keeping asks for that.
    pub(crate) fn stale_ring_ranges(
        &self,
        snapshot: u64,
        last: u64,
        strides: &RecorderStrides,
    ) -> Vec<(Handle<ShaderBuffer>, u64, u64)> {
        let mut ranges = Vec::new();
        if let (Some(handles), Some(stride)) = (&self.probes, strides.point) {
            let block = (MAX_POINT_PROBES * size_of::<GpuCanonicalPointProbeSample>()) as u64;
            for frame in stale_frames(snapshot, last, stride, PROBE_RING_FRAMES) {
                ranges.push((handles.output.clone(), frame as u64 * block, block));
            }
        }
        if let Some(handles) = &self.curve_probes {
            let size = size_of::<GpuCurveProbeSample>() as u64;
            for (descriptor, stride) in handles.descriptors.iter().zip(&strides.curves) {
                for frame in stale_frames(snapshot, last, *stride, CURVE_PROBE_RING_FRAMES) {
                    let first =
                        (frame * MAX_CURVE_PROBE_POINTS) as u64 + u64::from(descriptor.offset);
                    ranges.push((
                        handles.output.clone(),
                        first * size,
                        u64::from(descriptor.count) * size,
                    ));
                }
            }
        }
        if let (Some(handles), Some(stride)) = (&self.area_probes, strides.area) {
            let block = (MAX_POINT_PROBES * size_of::<GpuAreaProbeSample>()) as u64;
            for frame in stale_frames(snapshot, last, stride, AREA_PROBE_RING_FRAMES) {
                ranges.push((handles.output.clone(), frame as u64 * block, block));
            }
        }
        ranges
    }

    /// Stage-6 recorder ownership follows the accepted canonical generation.
    /// Rings deliberately survive; the configure paths decide whether their
    /// observable and contour remain compatible.
    pub fn adopt_canonical_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    pub fn set_limits(&mut self, limits: Option<CanonicalGpuLimits>) {
        self.limits = limits;
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

    pub fn update_canonical_point_probes(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        operator: &CanonicalWaveOperator,
        probes: &[(u64, Option<QuadraticPointStencil>)],
        sample_rate: f64,
        context: RecorderContext,
    ) -> Result<(), String> {
        let result = self.update_point_probes_from(
            assets,
            commands,
            CanonicalStencilSource::Fixed(operator),
            probes,
            sample_rate,
            context,
        );
        self.cleared_on_error(result, Self::clear_probe_buffers, assets, commands)
    }

    /// A rejection clears the recorder, whatever rejected it. The validation
    /// rejections always did, but a stencil that could not be built returned
    /// with the previous mesh's stencils still bound, and they sampled
    /// unrelated nodes of the new generation for as long as it ran (PR-4 of
    /// the 2026-10-20 review).
    fn cleared_on_error(
        &mut self,
        result: Result<(), String>,
        clear: fn(&mut Self, &mut Assets<ShaderBuffer>, &mut Commands),
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
    ) -> Result<(), String> {
        if result.is_err() {
            clear(self, assets, commands);
        }
        result
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
        let result = TemporalTableIndex::build(operator, manifest).and_then(|index| {
            self.update_point_probes_from(
                assets,
                commands,
                CanonicalStencilSource::Temporal {
                    operator,
                    index: &index,
                },
                probes,
                sample_rate,
                context,
            )
        });
        self.cleared_on_error(result, Self::clear_probe_buffers, assets, commands)
    }

    fn update_point_probes_from(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        source: CanonicalStencilSource<'_>,
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
                Some(stencil) => source.build(*stencil, "point"),
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
            && self
                .probes
                .as_ref()
                .is_some_and(|handles| handles.ids == ids && handles.physics == physics);
        let handles = if kept {
            let previous = self.probes.take().expect("a kept ring exists");
            assets.remove(previous.stencils.id());
            assets.remove(previous.control.id());
            ProbeBufferHandles {
                stencils: assets.add(ShaderBuffer::from(stencils)),
                control: assets.add(ShaderBuffer::from(control)),
                sample_stride,
                physics,
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
            }
        };
        if let Some(entity) = self.probe_readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.probe_revision = self.probe_revision.wrapping_add(1).max(1);
        self.probe_readback_entity = Some(
            commands
                .spawn((
                    PacedReadback::buffer(handles.output.clone()),
                    ProbeReadbackTag {
                        generation: self.generation,
                        revision: self.probe_revision,
                        ids: handles.ids.clone(),
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
        self.update_vector_overlay_from(
            assets,
            commands,
            CanonicalStencilSource::Fixed(operator),
            stencils,
        )
    }

    /// Arrows over a time-driven generation. The lattice runs through the same
    /// shared reconstruction the point recorder uses, so a modulated element
    /// shows the field its own samples carry.
    pub fn update_temporal_canonical_vector_overlay(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        operator: &CanonicalTemporalWaveOperator,
        manifest: CanonicalGpuTemporalManifest,
        stencils: &[QuadraticPointStencil],
    ) -> Result<(), String> {
        let index = TemporalTableIndex::build(operator, manifest)?;
        self.update_vector_overlay_from(
            assets,
            commands,
            CanonicalStencilSource::Temporal {
                operator,
                index: &index,
            },
            stencils,
        )
    }

    fn update_vector_overlay_from(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        source: CanonicalStencilSource<'_>,
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
            .map(|stencil| source.build(stencil, "vector"))
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
                    PacedReadback::buffer(handles.output.clone()),
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

    /// Releases every recorder and the arrow overlay, as at launch. A
    /// replaced scene's generation is dropped with the canonical request's
    /// `clear`, and nothing may go on reading what it left: without this the
    /// probe bind groups of that generation outlive its buffers.
    pub fn clear_recorders(&mut self, assets: &mut Assets<ShaderBuffer>, commands: &mut Commands) {
        self.clear_probe_buffers(assets, commands);
        self.clear_curve_probe_buffers(assets, commands);
        self.clear_area_probe_buffers(assets, commands);
        self.clear_far_field_buffers(assets, commands);
        self.clear_vector_overlay(assets, commands);
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

    pub fn update_canonical_curve_probes(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        operator: &CanonicalWaveOperator,
        probes: &[CurveProbeInput],
        context: RecorderContext,
    ) -> Result<(), String> {
        let result = self.update_curve_probes_from(
            assets,
            commands,
            CanonicalStencilSource::Fixed(operator),
            probes,
            context,
        );
        self.cleared_on_error(result, Self::clear_curve_probe_buffers, assets, commands)
    }

    /// Line probes over a time-driven generation. Each sample reconstructs
    /// through the same shared shader block the point recorder uses, so a
    /// travelling drive is resolved at the element's own samples here too.
    pub fn update_temporal_canonical_curve_probes(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        operator: &CanonicalTemporalWaveOperator,
        manifest: CanonicalGpuTemporalManifest,
        probes: &[CurveProbeInput],
        context: RecorderContext,
    ) -> Result<(), String> {
        let result = TemporalTableIndex::build(operator, manifest).and_then(|index| {
            self.update_curve_probes_from(
                assets,
                commands,
                CanonicalStencilSource::Temporal {
                    operator,
                    index: &index,
                },
                probes,
                context,
            )
        });
        self.cleared_on_error(result, Self::clear_curve_probe_buffers, assets, commands)
    }

    fn update_curve_probes_from(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        source: CanonicalStencilSource<'_>,
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
                    Some((stencil, normal)) => (source.build(*stencil, "line")?, *normal, 1.0),
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
                handles.descriptors == descriptors && handles.physics == physics
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
            }
        };
        if let Some(entity) = self.curve_probe_readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.curve_probe_revision = self.curve_probe_revision.wrapping_add(1).max(1);
        self.curve_probe_readback_entity = Some(
            commands
                .spawn((
                    PacedReadback::buffer(handles.output.clone()),
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

    pub fn update_canonical_area_probes(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        operator: &CanonicalWaveOperator,
        probes: &[AreaProbeInput],
        sample_rate: f64,
        context: RecorderContext,
    ) -> Result<(), String> {
        let result = self.update_area_probes_from(
            assets,
            commands,
            operator,
            None,
            probes,
            sample_rate,
            context,
        );
        self.cleared_on_error(result, Self::clear_area_probe_buffers, assets, commands)
    }

    /// Area probes over a time-driven generation. Each contribution addresses
    /// its own law records, so the assembled nodal map and the samples'
    /// inverses are evaluated at the sampled instant.
    #[allow(clippy::too_many_arguments)]
    pub fn update_temporal_canonical_area_probes(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        operator: &CanonicalTemporalWaveOperator,
        manifest: CanonicalGpuTemporalManifest,
        probes: &[AreaProbeInput],
        sample_rate: f64,
        context: RecorderContext,
    ) -> Result<(), String> {
        let result = TemporalTableIndex::build(operator, manifest).and_then(|index| {
            self.update_area_probes_from(
                assets,
                commands,
                operator.base(),
                Some((operator, &index)),
                probes,
                sample_rate,
                context,
            )
        });
        self.cleared_on_error(result, Self::clear_area_probe_buffers, assets, commands)
    }

    #[allow(clippy::too_many_arguments)]
    fn update_area_probes_from(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        operator: &CanonicalWaveOperator,
        temporal: Option<(&CanonicalTemporalWaveOperator, &TemporalTableIndex)>,
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
        // A record per element piece, bound whole: past what the device binds,
        // the recorder's bind group failed validation and rendering stopped
        // for good, at about 115k pieces under WebGPU's default 128 MiB where
        // the cap allows 200k. Refused, the run goes on (the solver's buffers
        // are admitted alike, GPU-7 of the 2026-10-20 review).
        if let Some(limits) = self.limits {
            let pieces = contribution_count.max(1);
            let admitted = limits
                .admit(
                    "area probes' contribution",
                    pieces * size_of::<GpuCanonicalAreaContribution>(),
                )
                .and_then(|()| {
                    limits.admit(
                        "area probes' scratch",
                        pieces * size_of::<GpuAreaProbeContributionSample>(),
                    )
                });
            if let Err(refusal) = admitted {
                self.clear_area_probe_buffers(assets, commands);
                return Err(refusal);
            }
        }
        let sample_stride = (1.0 / (sample_rate * time_step)).round().max(1.0) as u64;
        let mut contributions = Vec::with_capacity(contribution_count.max(1));
        let mut descriptors = Vec::with_capacity(probes.len());
        for probe in probes {
            let offset = contributions.len() as u32;
            if let Some(stencil) = &probe.stencil {
                for element in &stencil.elements {
                    contributions.push(gpu_canonical_area_contribution(
                        *element, operator, temporal,
                    )?);
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
            && self
                .area_probes
                .as_ref()
                .is_some_and(|handles| handles.ids == ids && handles.physics == physics);
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
            }
        };
        if let Some(entity) = self.area_probe_readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.area_probe_revision = self.area_probe_revision.wrapping_add(1).max(1);
        self.area_probe_readback_entity = Some(
            commands
                .spawn((
                    PacedReadback::buffer(handles.output.clone()),
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
                .is_some_and(|handles| handles.contour == contour);
        let handles = if kept {
            let previous = self.far_field.take().expect("a kept ring exists");
            assets.remove(previous.stencils.id());
            assets.remove(previous.control.id());
            FarFieldBufferHandles {
                stencils: assets.add(ShaderBuffer::from(stencils)),
                control: assets.add(ShaderBuffer::from(control)),
                contour,
                sample_stride: far_field_sample_stride(period, time_step),
                ..previous
            }
        } else {
            self.clear_far_field_buffers(assets, commands);
            // Each slot's samples, one header per slot, and the ring's own
            // state - the next slot, how many frames it holds, whether a
            // pass records - last, all zero: an empty ring.
            let raw = vec![
                GpuProbeSample::default();
                FAR_FIELD_RING_FRAMES * (FAR_FIELD_CONTOUR_POINTS + 1) + 1
            ];
            let output =
                vec![GpuProbeSample::default(); FAR_FIELD_RING_FRAMES * FAR_FIELD_DIRECTIONS];
            FarFieldBufferHandles {
                stencils: assets.add(ShaderBuffer::from(stencils)),
                control: assets.add(ShaderBuffer::from(control)),
                raw: assets.add(ShaderBuffer::from(raw)),
                output: assets.add(ShaderBuffer::from(output)),
                contour,
                sample_stride: far_field_sample_stride(period, time_step),
            }
        };
        if let Some(entity) = self.far_field_readback_entity.take() {
            commands.entity(entity).despawn();
        }
        self.far_field_revision = self.far_field_revision.wrapping_add(1).max(1);
        self.far_field_readback_entity = Some(
            commands
                .spawn((
                    PacedReadback::buffer(handles.output.clone()),
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
}

#[derive(Resource, Default)]
pub struct WaveDisplay {
    pub generation: u64,
    pub current: Vec<f32>,
    /// Canonical six-sample complementary flux read directly by vector and AMR
    /// consumers.
    pub complementary_flux: Vec<[f32; 2]>,
    /// The primary field captured with `complementary_flux` by the latest
    /// aligned full canonical snapshot. Vector and AMR consumers use it
    /// rather than mixing a lower-cadence complementary field with the live
    /// primary display stream.
    pub snapshot_current: Vec<f32>,
    pub snapshot_completed_steps: u64,
    /// Gate O: the integrated field `r` from the latest full canonical
    /// snapshot, empty without a restoring law.
    pub snapshot_integrated: Vec<f32>,
    /// Gate O: `r` from its own continuous readback or the latest handoff
    /// receipt, for the view that paints it, and that stream's serial.
    pub live_integrated: Vec<f32>,
    pub live_integrated_readbacks: u64,
    /// The field the painter shows: the played-out copy
    /// (`picture_playout`), a frame or so behind `current`, and the serial it
    /// was made from. The integrated field's played-out copy beside it.
    pub picture: Vec<f32>,
    pub picture_serial: u64,
    pub picture_integrated: Vec<f32>,
    pub completed_steps: u64,
    pub readbacks: u64,
}

#[derive(Component)]
struct ProbeReadbackTag {
    generation: u64,
    revision: u64,
    ids: Arc<[u64]>,
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
    /// The arclength the frame was sampled over: the probe's in the
    /// generation that recorded it, which ingestion sets. Integrated over one
    /// length, the document's as it stood, a segment dragged from length 1
    /// to 2 doubled every frame it had recorded (PR-13 of the 2026-10-20
    /// review).
    pub length: f64,
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
struct GpuFarFieldControl {
    sampling: Vec4,
    projection: Vec4,
}

/// Direct-state point reconstruction. Primary weights act on `Q / M`; the
/// complementary weights interpolate the physical field recovered at the six
/// independent quadrature samples belonging to one element.
///
/// `reference_inverse.yzw` is the *forward* coefficient at the probe point,
/// used only for the energy density. The inverses live per sample, because
/// that is where the solver owns one. These base tensors are immutable for
/// the life of a generation, unlike the law records the temporal words
/// address, so holding them here cannot go stale.
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
    /// `xyz` is the constitutive inverse at sample `i`; `w` is unused here and
    /// carries the integration weight in the area contribution.
    sample_inverse: [Vec4; COMPLEMENTARY_SAMPLES],
    temporal_primary_a: UVec4,
    temporal_primary_b: UVec4,
    temporal_complementary: UVec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuCanonicalCurveStencil {
    point: GpuCanonicalPointStencil,
    normal_stride_valid: Vec4,
}

/// No constitutive coefficient: field statistics are moments of the
/// interpolated fields, and the energy comes from the parent element's
/// canonical terms below.
#[derive(Clone, Copy, Default, ShaderType)]
struct GpuCanonicalAreaQuadrature {
    primary_a: Vec4,
    primary_b: Vec4,
    complementary_a: Vec4,
    complementary_b: Vec4,
    weight: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuCanonicalAreaContribution {
    nodes_a: UVec4,
    nodes_b: UVec4,
    sample_valid: UVec4,
    /// `xyz` is the constitutive inverse at sample `i` of the parent element,
    /// `w` its integration weight. Shared by all twelve quadrature points.
    sample_inverse: [Vec4; COMPLEMENTARY_SAMPLES],
    /// This element's contribution to each local node's lumped mass, locals
    /// 0-3 then 4-6, with the piece's covered fraction of its parent in the
    /// last lane. The shader applies the node's inverse mass itself, so these
    /// stay correct when a driven material changes that mass.
    node_shares_a: Vec4,
    node_shares_b: Vec4,
    /// Law-table word for each local node's own primary contribution, and in
    /// the last lane of `b` the element's first complementary record. Zero on
    /// a static generation, where `sample_valid.z` is also zero.
    temporal_primary_a: UVec4,
    temporal_primary_b: UVec4,
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

/// Where a consumer's point reconstruction comes from. The fixed operator and
/// the temporal one produce the same GPU record; only the temporal case also
/// addresses the law tables, so every recorder takes this rather than growing
/// a second copy of its installer.
#[derive(Clone, Copy)]
enum CanonicalStencilSource<'a> {
    Fixed(&'a CanonicalWaveOperator),
    Temporal {
        operator: &'a CanonicalTemporalWaveOperator,
        index: &'a TemporalTableIndex,
    },
}

impl CanonicalStencilSource<'_> {
    fn build(
        &self,
        stencil: QuadraticPointStencil,
        consumer: &str,
    ) -> Result<GpuCanonicalPointStencil, String> {
        match self {
            Self::Fixed(operator) => CanonicalPointStencil::from_quadratic(stencil, operator)
                .map(|stencil| gpu_canonical_point_stencil(Some(stencil)))
                .map_err(|error| format!("Canonical {consumer} reconstruction failed: {error}")),
            Self::Temporal { operator, index } => {
                let stencil = CanonicalTemporalPointStencil::from_quadratic(stencil, operator)
                    .map_err(|error| {
                        format!("Temporal {consumer} reconstruction failed: {error}")
                    })?;
                gpu_temporal_canonical_point_stencil(stencil, operator, index)
            }
        }
    }
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
            stencil.complementary_reference.xx as f32,
            stencil.complementary_reference.xy as f32,
            stencil.complementary_reference.yy as f32,
        ),
        orientation: Vec4::new(stencil.orientation as f32, 0.0, 0.0, 0.0),
        sample_inverse: std::array::from_fn(|local| {
            let tensor = stencil.sample_inverses[local];
            Vec4::new(tensor.xx as f32, tensor.xy as f32, tensor.yy as f32, 0.0)
        }),
        temporal_primary_a: UVec4::ZERO,
        temporal_primary_b: UVec4::ZERO,
        temporal_complementary: UVec4::ZERO,
    }
}

/// Where each element's law records sit in the GPU table.
///
/// Primary records are packed node-major, so an element's contribution to a
/// node sits in that node's run, offset by how many earlier elements share
/// it. Resolving that by scanning the contribution list is tolerable for
/// sixteen point probes and quadratic for an area probe covering a mesh, so
/// the runs and ranks are computed once per recorder upload instead.
struct TemporalTableIndex {
    node_starts: Vec<u32>,
    contribution_ranks: Vec<u32>,
    complementary_offset: usize,
    coefficient_words: usize,
}

impl TemporalTableIndex {
    fn build(
        operator: &CanonicalTemporalWaveOperator,
        manifest: CanonicalGpuTemporalManifest,
    ) -> Result<Self, String> {
        let contributions = operator.base().primary_contributions();
        if manifest.coefficient_words == 0
            || manifest.primary_record_count != contributions.len()
            || manifest.complementary_record_count
                != operator.base().complementary_degrees_of_freedom()
        {
            return Err("Temporal reconstruction does not match the GPU table layout".into());
        }
        let node_count = operator.base().degrees_of_freedom();
        let mut counts = vec![0_u32; node_count + 1];
        for contribution in contributions {
            let node = contribution.node as usize;
            if node >= node_count {
                return Err("A temporal primary contribution names an unknown node".into());
            }
            counts[node + 1] += 1;
        }
        for index in 1..counts.len() {
            counts[index] += counts[index - 1];
        }
        let node_starts = counts[..node_count]
            .iter()
            .map(|records| {
                usize::try_from(*records)
                    .ok()
                    .and_then(|records| records.checked_mul(manifest.coefficient_words))
                    .and_then(|offset| manifest.primary_record_offset.checked_add(offset))
                    .and_then(|word| u32::try_from(word).ok())
                    .ok_or_else(|| "A temporal primary table address exceeds u32".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut seen = vec![0_u32; node_count];
        let mut contribution_ranks = Vec::with_capacity(contributions.len());
        for contribution in contributions {
            let node = contribution.node as usize;
            contribution_ranks.push(seen[node]);
            seen[node] += 1;
        }
        Ok(Self {
            node_starts,
            contribution_ranks,
            complementary_offset: manifest.complementary_record_offset,
            coefficient_words: manifest.coefficient_words,
        })
    }

    /// The seven primary record words of one element, in local-node order.
    fn primary_words(
        &self,
        operator: &CanonicalTemporalWaveOperator,
        element: u32,
    ) -> Result<[u32; LOCAL_NODES], String> {
        let contributions = operator.base().primary_contributions();
        let first = (element as usize)
            .checked_mul(LOCAL_NODES)
            .ok_or_else(|| "A temporal primary contribution range overflowed".to_string())?;
        let owned = contributions
            .get(first..first + LOCAL_NODES)
            .filter(|owned| {
                owned.iter().enumerate().all(|(local, contribution)| {
                    contribution.element == element && contribution.local_node as usize == local
                })
            })
            .ok_or_else(|| format!("Element {element} has no contiguous primary contributions"))?;
        let mut words = [0_u32; LOCAL_NODES];
        for (local, word) in words.iter_mut().enumerate() {
            let contribution = owned[local];
            let start = self
                .node_starts
                .get(contribution.node as usize)
                .copied()
                .ok_or_else(|| {
                    "A temporal primary contribution names an unknown node".to_string()
                })?;
            let rank = self.contribution_ranks[first + local];
            *word = u32::try_from(self.coefficient_words)
                .ok()
                .and_then(|words| rank.checked_mul(words))
                .and_then(|offset| start.checked_add(offset))
                .ok_or_else(|| "A temporal primary table address exceeds u32".to_string())?;
        }
        Ok(words)
    }

    /// The first of an element's six complementary records.
    fn complementary_word(&self, element: u32) -> Result<u32, String> {
        (element as usize)
            .checked_mul(COMPLEMENTARY_SAMPLES)
            .and_then(|record| record.checked_mul(self.coefficient_words))
            .and_then(|offset| self.complementary_offset.checked_add(offset))
            .and_then(|word| u32::try_from(word).ok())
            .ok_or_else(|| "A temporal complementary table address exceeds u32".to_string())
    }
}

fn gpu_temporal_canonical_point_stencil(
    stencil: CanonicalTemporalPointStencil,
    operator: &CanonicalTemporalWaveOperator,
    index: &TemporalTableIndex,
) -> Result<GpuCanonicalPointStencil, String> {
    let element = stencil.element();
    let primary_words = index.primary_words(operator, element)?;
    let mut result = gpu_canonical_point_stencil(Some(stencil.fixed()));
    result.sample_valid.z = 1;
    result.temporal_primary_a = UVec4::from_array(primary_words[..4].try_into().unwrap());
    result.temporal_primary_b =
        UVec4::from_array([primary_words[4], primary_words[5], primary_words[6], 0]);
    result.temporal_complementary = UVec4::new(index.complementary_word(element)?, 0, 0, 0);
    Ok(result)
}

fn gpu_canonical_area_contribution(
    element: QuadraticAreaElement,
    operator: &CanonicalWaveOperator,
    temporal: Option<(&CanonicalTemporalWaveOperator, &TemporalTableIndex)>,
) -> Result<GpuCanonicalAreaContribution, String> {
    let contribution = canonical_area_contribution(element, operator)
        .map_err(|error| format!("Canonical area reconstruction failed: {error}"))?;
    let shares = contribution.node_references.map(|value| value as f32);
    let (valid, primary_words, complementary_word) = match temporal {
        Some((temporal, index)) => (
            1,
            index.primary_words(temporal, element.element)?,
            index.complementary_word(element.element)?,
        ),
        None => (0, [0_u32; LOCAL_NODES], 0),
    };
    Ok(GpuCanonicalAreaContribution {
        nodes_a: UVec4::new(
            element.nodes[0],
            element.nodes[1],
            element.nodes[2],
            element.nodes[3],
        ),
        nodes_b: UVec4::new(element.nodes[4], element.nodes[5], element.nodes[6], 0),
        sample_valid: UVec4::new(element.element * COMPLEMENTARY_SAMPLES as u32, 1, valid, 0),
        sample_inverse: std::array::from_fn(|local| {
            let tensor = contribution.sample_inverses[local];
            Vec4::new(
                tensor.xx as f32,
                tensor.xy as f32,
                tensor.yy as f32,
                contribution.sample_weights[local] as f32,
            )
        }),
        node_shares_a: Vec4::from_array([shares[0], shares[1], shares[2], shares[3]]),
        node_shares_b: Vec4::from_array([
            shares[4],
            shares[5],
            shares[6],
            contribution.covered_fraction as f32,
        ]),
        temporal_primary_a: UVec4::from_array(primary_words[..4].try_into().unwrap()),
        temporal_primary_b: UVec4::from_array([
            primary_words[4],
            primary_words[5],
            primary_words[6],
            complementary_word,
        ]),
        quadrature: contribution.quadrature.map(|point| {
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

/// The recorders' strides as a source generation had them when its handoff
/// was admitted: what its kept rings' frames were written at.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RecorderStrides {
    point: Option<u64>,
    /// One per line probe, in its descriptors' order.
    curves: Vec<u64>,
    area: Option<u64>,
}

/// The ring frames a recorder of `stride` wrote for the steps after
/// `snapshot` through `last`: the frame `(w / stride) % frames` of each step
/// `w` it was scheduled at, on the device's count, and every frame once the
/// steps go round the ring.
pub(crate) fn stale_frames(snapshot: u64, last: u64, stride: u64, frames: usize) -> Vec<usize> {
    if stride == 0 || last <= snapshot {
        return Vec::new();
    }
    if last - snapshot >= stride.saturating_mul(frames as u64) {
        return (0..frames).collect();
    }
    let mut stale = (snapshot + 1..=last)
        .map(device_step)
        .filter(|step| probe_sample_due(*step, stride))
        .map(|step| ((step / stride) % frames as u64) as usize)
        .collect::<Vec<_>>();
    stale.sort_unstable();
    stale.dedup();
    stale
}

/// The step the recorders are scheduled at: the device's own count, a u32
/// the host's u64 one wraps onto. The line recorder tests each probe's stride
/// on it again, and on the host's count, past 2^32 steps, no step chosen
/// passed that test unless the stride divides 2^32 (PR-10 of the 2026-10-20
/// review). The ring slots are the device's arithmetic on it too.
pub(crate) fn device_step(step: u64) -> u64 {
    u64::from(step as u32)
}

pub(crate) fn probe_sample_due(completed_step: u64, stride: u64) -> bool {
    stride > 0 && completed_step.is_multiple_of(stride)
}

/// Whether `completed_step` is a resident grid-filter commit.
///
/// The filter flips the accepted state lane without advancing physical time,
/// so at this boundary the other lane holds the pre-filter value at the same
/// instant rather than the previous endpoint.
pub(crate) fn resident_filter_commit(filtering: bool, completed_step: u64) -> bool {
    filtering && completed_step > 0 && completed_step.is_multiple_of(GRID_SCALE_FILTER_CADENCE)
}

/// When a recorder that differences the two state lanes takes its sample.
///
/// A sample falling exactly on a filter commit is taken one step later
/// instead, because differencing the lanes there yields a filter correction
/// divided by `dt` rather than a rate. Measured before this existed: the
/// reported rate collapsed to between 0.03% and 18% of the truth. The
/// coincidence is not rare, because below about `0.3x` the paced step is the
/// speed over 120, so the stride follows the speed control and shares a
/// factor with the sixteen-step cadence; at `0.0625x` every point sample
/// landed on a commit.
///
/// The deferred sample keeps its ring slot, since `step / stride` is
/// unchanged by the extra step when `step` is a multiple of `stride`. With a
/// stride of one there is no later step to move to and the boundary sample is
/// dropped. Consumers that read only the accepted lane are unaffected and do
/// not use this; the adaptation controller already waits the same way.
pub(crate) fn differencing_sample_due(completed_step: u64, stride: u64, filtering: bool) -> bool {
    if resident_filter_commit(filtering, completed_step) {
        return false;
    }
    probe_sample_due(completed_step, stride)
        || (completed_step > 0
            && resident_filter_commit(filtering, completed_step - 1)
            && probe_sample_due(completed_step - 1, stride))
}

/// A record's time from the two parts the recorders write, as the vector
/// overlay writes its own: the epoch origin's coarse part, and its fine part
/// with the epoch-local time. Added here in f64, a stamp keeps the fine part's
/// precision, 3e-5 s, however long the run.
pub(crate) fn stamp_seconds(coarse: f32, fine: f32) -> f64 {
    coarse as f64 + fine as f64
}

fn receive_probe_readback(
    event: On<PacedReadbackComplete>,
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
                time: stamp_seconds(primary.w, secondary.z),
                displacement: primary.x as f64,
                velocity: primary.y as f64,
                energy_density: primary.z as f64,
                transverse_magnitude: secondary.x as f64,
                poynting_magnitude: secondary.y as f64,
            });
        }
    };
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
    event: On<PacedReadbackComplete>,
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
    let absolute_time = stamp_seconds(f32::from_bits(time_bits.x), f32::from_bits(time_bits.y));
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
    event: On<PacedReadbackComplete>,
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
                .map(|sample| stamp_seconds(sample.primary.w, sample.secondary.y))
                .find(|time| time.is_finite())
            else {
                continue;
            };
            records.push(CurveProbeRecord {
                probe_id: descriptor.id,
                time,
                length: f64::NAN,
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
    event: On<PacedReadbackComplete>,
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
                    time: stamp_seconds(sample.secondary.w, sample.tertiary.y),
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
    event: On<PacedReadbackComplete>,
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
        // The far time is a stamp, as a probe record's is.
        let time = stamp_seconds(row[0].values.y, row[0].values.z);
        if time < 0.0 || !time.is_finite() {
            continue;
        }
        records.push(FarFieldRecord {
            time,
            amplitude: row.iter().map(|sample| sample.values.x).collect(),
            intensity: row
                .iter()
                .map(|sample| sample.values.x * sample.values.x)
                .collect(),
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
        embedded_asset!(app, "canonical_probe.wgsl");
        embedded_asset!(app, "canonical_curve_probe.wgsl");
        embedded_asset!(app, "canonical_area_probe.wgsl");
        embedded_asset!(app, "canonical_far_field.wgsl");
        app.init_resource::<WaveGpuRequest>()
            .init_resource::<WaveDisplay>()
            .init_resource::<ProbeDisplay>()
            .init_resource::<CurveProbeDisplay>()
            .init_resource::<AreaProbeDisplay>()
            .init_resource::<FarFieldDisplay>()
            .init_resource::<VectorOverlayDisplay>()
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
                    prepare_probe_bind_group,
                    prepare_curve_probe_bind_group,
                    prepare_area_probe_bind_group,
                    prepare_far_field_bind_group,
                    prepare_vector_overlay_bind_group,
                )
                    .in_set(RenderSystems::PrepareBindGroups),
            );
    }

    fn finish(&self, app: &mut App) {
        let limits = CanonicalGpuLimits::of_device(app.world());
        app.world_mut()
            .resource_mut::<WaveGpuRequest>()
            .set_limits(limits);
    }
}

#[derive(Resource)]
pub(crate) struct WavePipeline {
    canonical_probe_layout: BindGroupLayoutDescriptor,
    canonical_curve_probe_layout: BindGroupLayoutDescriptor,
    canonical_area_probe_layout: BindGroupLayoutDescriptor,
    canonical_area_reduce_layout: BindGroupLayoutDescriptor,
    canonical_far_field_layout: BindGroupLayoutDescriptor,
    canonical_vector_overlay_layout: BindGroupLayoutDescriptor,
    pub(crate) canonical_probe: CachedComputePipelineId,
    pub(crate) canonical_curve_probe: CachedComputePipelineId,
    pub(crate) canonical_area_probe_elements: CachedComputePipelineId,
    pub(crate) canonical_area_probe_reduce: CachedComputePipelineId,
    pub(crate) canonical_far_field_decide: CachedComputePipelineId,
    pub(crate) canonical_far_field_sample: CachedComputePipelineId,
    pub(crate) canonical_far_field_project: CachedComputePipelineId,
    pub(crate) canonical_far_field_advance: CachedComputePipelineId,
    pub(crate) canonical_far_field_rewind: CachedComputePipelineId,
    pub(crate) canonical_vector_overlay: CachedComputePipelineId,
}

fn init_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
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
                storage_buffer_read_only::<Vec<GpuCanonicalTableWord>>(false),
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
    // Sparse indices, because the two passes share one set of shader binding
    // numbers but each declares only the buffers its own entry point reads.
    let canonical_area_probe_layout = BindGroupLayoutDescriptor::new(
        "canonical area-probe element buffers",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::COMPUTE,
            (
                (0, storage_buffer_read_only::<GpuCanonicalControl>(false)),
                (
                    1,
                    storage_buffer_read_only::<Vec<GpuCanonicalStateWord>>(false),
                ),
                (2, storage_buffer_read_only::<Vec<GpuCanonicalNode>>(false)),
                (
                    3,
                    storage_buffer_read_only::<Vec<GpuCanonicalAreaContribution>>(false),
                ),
                (5, storage_buffer_read_only::<GpuProbeControl>(false)),
                (
                    6,
                    storage_buffer::<Vec<GpuAreaProbeContributionSample>>(false),
                ),
                (
                    8,
                    storage_buffer_read_only::<Vec<GpuCanonicalTableWord>>(false),
                ),
            ),
        ),
    );
    let canonical_area_reduce_layout = BindGroupLayoutDescriptor::new(
        "canonical area-probe reduction buffers",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::COMPUTE,
            (
                (0, storage_buffer_read_only::<GpuCanonicalControl>(false)),
                (
                    4,
                    storage_buffer_read_only::<Vec<GpuAreaProbeDescriptor>>(false),
                ),
                (5, storage_buffer_read_only::<GpuProbeControl>(false)),
                (
                    6,
                    storage_buffer::<Vec<GpuAreaProbeContributionSample>>(false),
                ),
                (7, storage_buffer::<Vec<GpuAreaProbeSample>>(false)),
            ),
        ),
    );
    let canonical_area_shader =
        load_embedded_asset!(asset_server.as_ref(), "canonical_area_probe.wgsl");
    let canonical_area_pipeline =
        |label: &'static str, entry: &'static str, layout: &BindGroupLayoutDescriptor| {
            ComputePipelineDescriptor {
                label: Some(Cow::Borrowed(label)),
                layout: vec![layout.clone()],
                shader: canonical_area_shader.clone(),
                entry_point: Some(Cow::Borrowed(entry)),
                ..default()
            }
        };
    let canonical_area_probe_elements =
        pipeline_cache.queue_compute_pipeline(canonical_area_pipeline(
            "canonical area-probe elements",
            "sample_area_elements",
            &canonical_area_probe_layout,
        ));
    let canonical_area_probe_reduce =
        pipeline_cache.queue_compute_pipeline(canonical_area_pipeline(
            "canonical area-probe reduction",
            "reduce_area_probes",
            &canonical_area_reduce_layout,
        ));
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
    let canonical_far_field_decide = pipeline_cache.queue_compute_pipeline(canonical_far_pipeline(
        "canonical far-field frame decision",
        "decide_frame",
    ));
    let canonical_far_field_sample = pipeline_cache.queue_compute_pipeline(canonical_far_pipeline(
        "canonical far-field contour sampler",
        "sample_contour",
    ));
    let canonical_far_field_project = pipeline_cache.queue_compute_pipeline(
        canonical_far_pipeline("canonical far-field projector", "project_directions"),
    );
    let canonical_far_field_rewind = pipeline_cache.queue_compute_pipeline(canonical_far_pipeline(
        "canonical far-field ring rewind",
        "rewind_frames",
    ));
    let canonical_far_field_advance = pipeline_cache.queue_compute_pipeline(
        canonical_far_pipeline("canonical far-field ring advance", "advance_frame"),
    );
    commands.insert_resource(WavePipeline {
        canonical_probe_layout,
        canonical_curve_probe_layout,
        canonical_area_probe_layout,
        canonical_area_reduce_layout,
        canonical_far_field_layout,
        canonical_vector_overlay_layout,
        canonical_probe,
        canonical_curve_probe,
        canonical_area_probe_elements,
        canonical_area_probe_reduce,
        canonical_far_field_decide,
        canonical_far_field_sample,
        canonical_far_field_project,
        canonical_far_field_advance,
        canonical_far_field_rewind,
        canonical_vector_overlay,
    });
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

/// The area recorder's two passes take separate bind groups. The element pass
/// needs state, nodes and the law tables; the reduction needs the descriptors
/// and the output ring. Splitting them keeps each pass inside the portable
/// eight-storage-buffer limit, which one combined layout of nine exceeded.
#[derive(Resource)]
pub(crate) struct AreaProbeBindGroup {
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    pub(crate) bind_group: BindGroup,
    pub(crate) reduce: BindGroup,
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
    let bind_group = render_device.create_bind_group(
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
    );
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
    let reduce = render_device.create_bind_group(
        Some("canonical area-probe reduction bind group"),
        &pipeline_cache.get_bind_group_layout(&pipeline.canonical_area_reduce_layout),
        &BindGroupEntries::with_indices((
            (0, canonical_control.buffer.as_entire_buffer_binding()),
            (4, descriptors.buffer.as_entire_buffer_binding()),
            (5, control.buffer.as_entire_buffer_binding()),
            (6, scratch.buffer.as_entire_buffer_binding()),
            (7, output.buffer.as_entire_buffer_binding()),
        )),
    );
    let bind_group = render_device.create_bind_group(
        Some("canonical area-probe element bind group"),
        &pipeline_cache.get_bind_group_layout(&pipeline.canonical_area_probe_layout),
        &BindGroupEntries::with_indices((
            (0, canonical_control.buffer.as_entire_buffer_binding()),
            (1, state.buffer.as_entire_buffer_binding()),
            (2, nodes.buffer.as_entire_buffer_binding()),
            (3, contributions.buffer.as_entire_buffer_binding()),
            (5, control.buffer.as_entire_buffer_binding()),
            (6, scratch.buffer.as_entire_buffer_binding()),
            (8, tables.buffer.as_entire_buffer_binding()),
        )),
    );
    commands.insert_resource(AreaProbeBindGroup {
        generation: request.generation,
        revision: request.area_probe_revision,
        bind_group,
        reduce,
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
    let bind_group = render_device.create_bind_group(
        Some("canonical line-probe bind group"),
        &pipeline_cache.get_bind_group_layout(&pipeline.canonical_curve_probe_layout),
        &BindGroupEntries::sequential((
            canonical_control.buffer.as_entire_buffer_binding(),
            state.buffer.as_entire_buffer_binding(),
            nodes.buffer.as_entire_buffer_binding(),
            stencils.buffer.as_entire_buffer_binding(),
            control.buffer.as_entire_buffer_binding(),
            output.buffer.as_entire_buffer_binding(),
            tables.buffer.as_entire_buffer_binding(),
        )),
    );
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
    let bind_group = render_device.create_bind_group(
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
    );
    commands.insert_resource(ProbeBindGroup {
        generation: request.generation,
        revision: request.probe_revision,
        bind_group,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::world::CommandQueue;
    use funfern_core::TopologyWaveModel;
    use funfern_core::test_scenes::*;

    use funfern_core::{MeshingOptions, OuterBoundaryCondition};

    /// The operator a recorder reads its stencils through. The rings below are
    /// kept or replaced on what the recorders are, whatever the mesh.
    fn canonical_operator() -> CanonicalWaveOperator {
        let scene = initial();
        let mesh = meshed(
            &scene,
            1,
            MeshingOptions {
                target_edge_length: 0.24,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let scalar = assembled(&mesh, &scene, OuterBoundaryCondition::Reflecting).unwrap();
        CanonicalWaveOperator::compile(
            &mesh,
            &scalar,
            TopologyWaveModel::from_topology_scene(&scene),
            1,
        )
        .unwrap()
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

    /// The collision is not rare. Below about `0.3x` the paced step is the
    /// speed over 120, so the recorder stride follows the speed control; at
    /// `0.0625x` a 120 Hz point probe strides 16 against a 16-step cadence
    /// and every single sample would land on a commit.
    #[test]
    fn samples_step_past_a_filter_commit_and_keep_their_slot() {
        let cadence = GRID_SCALE_FILTER_CADENCE;

        // With the filter off nothing moves.
        for step in 1..64 {
            assert_eq!(
                differencing_sample_due(step, 4, false),
                probe_sample_due(step, 4)
            );
        }

        // Stride sharing every factor with the cadence: each sample would
        // collide, and each is taken one step later instead.
        let stride = cadence;
        for multiple in 1..6 {
            let due = multiple * stride;
            assert!(!differencing_sample_due(due, stride, true));
            assert!(differencing_sample_due(due + 1, stride, true));
            // The ring slot the shader computes is unchanged by the step.
            assert_eq!(due / stride, (due + 1) / stride);
        }

        // A stride coprime with the cadence collides once every sixteen.
        let stride = 3;
        let collisions = (1..=16 * stride)
            .filter(|step| {
                probe_sample_due(*step, stride) && !differencing_sample_due(*step, stride, true)
            })
            .count();
        assert_eq!(collisions, 1);
        // The window runs one step past the last due sample, because that is
        // where the colliding one now lands.
        let taken = (1..=16 * stride + 1)
            .filter(|step| differencing_sample_due(*step, stride, true))
            .count();
        assert_eq!(taken, 16, "every sample is still taken, one of them later");

        // Stride one has no later step to move to, so the boundary sample is
        // dropped rather than duplicated.
        assert!(!differencing_sample_due(cadence, 1, true));
        assert!(differencing_sample_due(cadence + 1, 1, true));
        let taken = (1..=cadence)
            .filter(|step| differencing_sample_due(*step, 1, true))
            .count();
        assert_eq!(taken as u64, cadence - 1);
    }

    #[test]
    fn shader_binding_counts_match_their_declared_budgets() {
        let count = |source: &str| source.matches("@group(0) @binding").count();
        // The area shader declares more than a stage may bind, which is the
        // whole reason its two passes take separate layouts.
        assert_eq!(
            count(include_str!("canonical_area_probe.wgsl")),
            CANONICAL_AREA_ELEMENT_STORAGE_BINDINGS + CANONICAL_AREA_REDUCE_STORAGE_BINDINGS - 3,
            "control, probe_control and scratch are shared by both passes"
        );
        const {
            assert!(
                CANONICAL_AREA_ELEMENT_STORAGE_BINDINGS + CANONICAL_AREA_REDUCE_STORAGE_BINDINGS
                    - 3
                    > WEBGPU_PORTABLE_STORAGE_BUFFER_LIMIT
            )
        };
        assert_eq!(
            count(include_str!("canonical_probe.wgsl")),
            count(include_str!("canonical_curve_probe.wgsl")),
            "the point-family shaders share one bind-group shape"
        );
    }

    #[test]
    fn canonical_probe_shaders_consume_direct_accepted_state() {
        let point = include_str!("canonical_probe.wgsl");
        assert!(point.contains("if !temporal_enabled() { return nodes[node].mass_loss.y; }"));
        // The nodal field goes through the solver's own assembled map, which
        // is the linear division unless a record follows its field.
        assert!(point.contains("probe_primary_field(a.x, accepted_q(a.x), local_time)"));
        assert!(point.contains("probe_primary_field(a.x, previous_q(a.x), previous_time)"));
        assert!(point.contains("if !nonlinear { return flux / mass; }"));
        assert!(point.contains("return select(value.xy, value.zw"));
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

    fn shared_reconstruction_block(source: &str) -> &str {
        let start = source
            .find("// ---- shared canonical point reconstruction ----")
            .expect("the shared block opens");
        let end_marker = "// ---- end shared canonical point reconstruction ----";
        let end = source[start..]
            .find(end_marker)
            .expect("the shared block closes")
            + start
            + end_marker.len();
        &source[start..end]
    }

    /// A record's stamp keeps the epoch-local time's precision however long
    /// the run, where one f32 summed on the device kept f32's spacing at the
    /// run's absolute time. The device's arithmetic is reproduced here: the
    /// origin split as the plan packs it, whole seconds and the fraction
    /// (`split_f64`; split instead into a nearest f32 and its residual, the
    /// test held another split than the device's, VER-45 of the 2026-10-20
    /// review), the local time `steps × dt` in f32, the fine part their f32
    /// sum. Up to 36 hours in, at the observables fixture's step, every stamp
    /// is within the 3e-5 s of the clock that `stamp_seconds` states (2.3e-5,
    /// the local time's f32 near 256 s); the single f32 was 6.3e-3 s off at 36
    /// hours, three quarters of a 120 Hz record's spacing.
    #[test]
    fn probe_stamps_keep_their_precision_however_long_the_run() {
        let dt = 6.6931e-3_f64;
        let mut single_worst = 0.0_f64;
        for hours in [0.0, 1.0, 10.0, 18.2, 36.0] {
            let start = hours * 3600.0;
            // The epoch the device rebased onto last: its origin is a step
            // count's time, within 256 s of the clock.
            let origin = (start / dt).floor() * dt;
            let [hi, lo] = crate::canonical_gpu::split_f64(origin).unwrap();
            for step in (0..(256.0 / dt) as u32).step_by(97) {
                let local = step as f32 * dt as f32;
                let exact = origin + f64::from(step) * dt;
                let read = stamp_seconds(hi, lo + local);
                assert!(
                    (read - exact).abs() < 3.0e-5,
                    "{hours} h, step {step}: {read} against {exact}"
                );
                let single = f64::from(hi + lo + local);
                single_worst = single_worst.max((single - exact).abs());
            }
        }
        assert!(
            single_worst > 3.0e-3,
            "a single f32 read {single_worst:.2e} off"
        );
    }

    /// The clock's origin moves on by exact sums: whole seconds added to whole
    /// seconds, and a fraction to a fraction, `floor` taking the one from the
    /// other. The solver's and the transfer's copies are one text, and no
    /// compensated sum is left for a compiler to fold, as Metal folded one at
    /// every rebase and Chrome folded a guarded one in the transfer. The
    /// transfer's delay and a live pulse's start are differences taken part by
    /// part, every term small.
    #[test]
    fn the_clock_origin_moves_on_by_exact_sums() {
        let function = |source: &'static str| {
            let start = source.find("fn advance_origin(").expect("advance_origin");
            let end = start + source[start..].find("\n}\n").expect("its end");
            source[start..end].to_owned()
        };
        let wave = include_str!("canonical_wave.wgsl");
        let transfer = include_str!("canonical_transfer_runtime.wgsl");
        assert_eq!(function(wave), function(transfer));
        let body = function(wave);
        assert!(body.contains("let whole = floor(increment);"));
        assert!(body.contains("let fraction = low + (increment - whole);"));
        assert!(body.contains("let carry = floor(fraction);"));
        assert!(body.contains("return vec2<f32>(high + (whole + carry), fraction - carry);"));
        for source in [wave, transfer] {
            assert!(!source.contains("add_compensated"));
            assert!(source.contains(
                "advance_origin(\n        control.clock_origin.x, control.clock_origin.y, control.clock_f32.y);"
            ) || source.contains(
                "advance_origin(\n        old_control.clock_origin.x, old_control.clock_origin.y, old_control.clock_f32.y);"
            ));
        }
        assert!(transfer.contains(
            "let preparation_delta = ((old_control.clock_origin.x - new_control.clock_origin.x)"
        ));
        assert!(wave.contains("let start = (window.x - control.clock_origin.x) * opaque_one()"));
    }

    /// Every probe recorder writes a record's time as the pair `stamp()`
    /// forms, into the slots its decoder reads, and none sums it into one f32.
    #[test]
    fn probe_recorders_write_their_stamps_as_two_parts() {
        let pair = "    return vec2<f32>(control.clock_origin.x, control.clock_origin.y + control.clock_f32.y);";
        for (name, shader, writes) in [
            (
                "point",
                include_str!("canonical_probe.wgsl"),
                &[
                    "output[index].primary = vec4<f32>(primary, energy, time_pair.x);",
                    "vec4<f32>(length(complement), length(flow), time_pair.y, 0.0);",
                ][..],
            ),
            (
                "curve",
                include_str!("canonical_curve_probe.wgsl"),
                &[
                    "output[slot].primary = vec4<f32>(nan, nan, nan, time_pair.x);",
                    "output[slot].secondary = vec4<f32>(nan, time_pair.y, nan, nan);",
                    "primary, energy, dot(flow, record.normal_stride_valid.xy), time_pair.x);",
                    "vec4<f32>(length(complement), time_pair.y, 0.0, 0.0);",
                ][..],
            ),
            (
                "area",
                include_str!("canonical_area_probe.wgsl"),
                &[
                    "        time_pair.x);",
                    "output[output_index].tertiary = vec4<f32>(1.0, time_pair.y, 0.0, 0.0);",
                ][..],
            ),
        ] {
            assert!(shader.contains(pair), "{name}");
            assert!(!shader.contains("absolute_time"), "{name}");
            for write in writes {
                assert!(shader.contains(write), "{name}: {write}");
            }
        }
    }

    /// Point and line consumers must reconstruct fields with exactly the same
    /// text. A consumer that drifts from its neighbours does not fail; it
    /// reports a plausible wrong number, which is far more expensive to find.
    #[test]
    fn point_and_curve_consumers_share_one_reconstruction_block() {
        let point = shared_reconstruction_block(include_str!("canonical_probe.wgsl"));
        let curve = shared_reconstruction_block(include_str!("canonical_curve_probe.wgsl"));
        assert_eq!(point, curve);
        assert!(point.contains("fn physical_complement"));
        assert!(point.contains("fn sample_temporal_factor"));
        assert!(point.len() > 4_000, "the block lost its contents");
    }

    /// Every consumer recovers the physical complementary field at the
    /// element's own samples and interpolates that, rather than inverting once
    /// at an interpolated flux. The area shader cannot share the text above,
    /// so its arithmetic is pinned here instead.
    #[test]
    fn consumers_invert_at_solver_samples_before_interpolating() {
        let point = include_str!("canonical_probe.wgsl");
        assert!(point.contains(
            "field += sample_field(stencil, local, flux[local], local_time) * weights[local];"
        ));
        assert!(
            point.contains("let linear = apply_symmetric(stencil.sample_inverse[local].xyz, flux)")
        );
        assert!(point.contains("/ sample_temporal_factor(stencil, local, local_time)"));
        assert!(
            point.contains("dot(field, apply_symmetric(stencil.reference_inverse.yzw, field))")
        );

        let area = include_str!("canonical_area_probe.wgsl");
        assert!(area.contains("area_sample(contribution, local, accepted_b(start + local), time)"));
        assert!(area.contains(
            "apply_symmetric(inverse.xyz, flux) / sample_factor(contribution, local, local_time)"
        ));
        // Canonical energy: each node's store split over its materials plus
        // the samples' own energies, scaled by the piece's covered fraction.
        assert!(area.contains("accumulated.w = contribution.node_shares_b.w * energy;"));
        assert!(area.contains("let linear_energy = 0.5 * inverse.w * dot(flux, linear);"));
        // The area recorder inverts the nodal map and evaluates the field
        // laws with exactly the text the point and line probes use.
        let piece = |source: &'static str, from: &str, to: &str| {
            let start = source.find(from).expect(from);
            let end = start + source[start..].find(to).expect(to);
            source[start..end].to_owned()
        };
        for (from, to) in [
            ("fn field_kind(word: u32)", "fn record_factor(word: u32"),
            (
                "fn probe_primary_field(",
                "// `v(b)` at one of the element's samples",
            ),
        ] {
            let shared = piece(point, from, to);
            assert!(
                area.contains(&shared),
                "the area shader drifted from {from}"
            );
        }
        // The area quadrature carries no material law at all now.
        assert!(!area.contains("reference_inverse"));
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

    /// Each curve probe samples on its own solver-step stride, read back from
    /// the recorder that stores it: the test once checked its own copy of the
    /// formula (VER-20 of the 2026-10-20 review).
    #[test]
    fn curve_probe_presets_have_independent_solver_step_strides() {
        let operator = canonical_operator();
        let world = World::new();
        let mut assets = Assets::<ShaderBuffer>::default();
        let mut request = WaveGpuRequest::default();
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        let probes = [30.0, 60.0, 120.0]
            .into_iter()
            .zip(1..)
            .map(|(sample_rate, id)| CurveProbeInput {
                id,
                sample_rate,
                samples: vec![None; 3],
            })
            .collect::<Vec<_>>();
        request
            .update_canonical_curve_probes(
                &mut assets,
                &mut commands,
                &operator,
                &probes,
                RecorderContext {
                    time_step: 0.001,
                    physics: PhysicsModel::Mechanical,
                    history: RecorderHistory::Keep,
                },
            )
            .unwrap();
        let strides = request.recorder_strides().curves;
        assert_eq!(strides, vec![33, 17, 8]);
        assert!(probe_sample_due(264, strides[0]));
        assert!(!probe_sample_due(264, strides[1]));
        assert!(probe_sample_due(264, strides[2]));
    }

    #[test]
    fn area_probe_shader_reduces_to_compact_physical_records() {
        let shader = include_str!("canonical_area_probe.wgsl");
        assert!(shader.contains("fn sample_area_elements"));
        assert!(shader.contains("fn reduce_area_probes"));
        assert!(shader.contains("        accumulated.x / covered_area,"));
        assert!(shader.contains("        sqrt(max(accumulated.y / covered_area, 0.0)),"));
        assert!(shader.contains("        sqrt(max(accumulated.z / covered_area, 0.0)),"));
        assert!(shader.contains("bitcast<f32>(0x7fc00000u | (probe & 1u))"));
    }

    /// A frame stands half a step back, the field and its normal gradient
    /// the mean of the step's two lanes beside their difference, and the
    /// projection reads from that frame's own stamp: a rate half a step older
    /// than the gradient beside it moved a 3 Hz source's pattern by 10%.
    #[test]
    fn far_field_frames_are_centred_on_their_step() {
        let shader = include_str!("canonical_far_field.wgsl");
        assert!(shader.contains(
            "control.clock_origin.y + (control.clock_f32.y - 0.5 * control.clock_f32.x));"
        ));
        assert!(shader.contains("let gradient = 0.5 * (current_gradient + previous_gradient);"));
        assert!(
            shader.contains("vec4<f32>(0.5 * (primary + previous), rate, normal_gradient, 0.0);")
        );
        assert!(shader.contains(
            "contour_history[header_index(slot)].values = vec4<f32>(centred_stamp(), 0.0, 0.0);"
        ));
        assert!(shader.contains("let now = contour_history[header_index(newest)].values.xy;"));
    }

    #[test]
    fn far_field_shader_uses_retarded_time_and_huygens_data() {
        let shader = include_str!("canonical_far_field.wgsl");
        assert!(shader.contains("fn sample_contour"));
        assert!(shader.contains("fn project_directions"));
        assert!(shader.contains("let retarded = dot(ray, position) / wave_speed - margin;"));
        assert!(shader.contains("sample.z - dot(normal, ray) * sample.y / wave_speed"));
        assert!(shader.contains("fraction = clamp((newer_time - retarded) / span, 0.0, 1.0);"));
    }

    /// The ring knows which frames it holds by its own count, never by a NaN
    /// test, which fast math folds to `false`, nor by a sentinel time.
    #[test]
    fn far_field_validity_never_rests_on_a_nan_comparison() {
        let shader = include_str!("canonical_far_field.wgsl");
        assert!(!shader.contains("!= newer"));
        assert!(!shader.contains("!= older"));
        assert!(!shader.contains("UNRECORDED"));
        assert!(shader.contains("let held = min(frames_held() + 1u, frames);"));
        assert!(shader.contains("if stamp_difference(oldest, now) > retarded {"));
    }

    /// Every time the far field reads is a two-part stamp, and every
    /// difference of two is taken part by part: summed into one f32, the
    /// frames' times kept f32's spacing at the run's absolute time, 2e-3 s
    /// after ten hours. The ring is keyed to its own count, so nothing reads
    /// the absolute time, which past 36 hours could not tell one 1/60 s
    /// bucket from the next.
    #[test]
    fn far_field_shader_keys_its_ring_to_its_count_and_reads_stamps() {
        let shader = include_str!("canonical_far_field.wgsl");
        assert!(!shader.contains("state_time"));
        assert!(!shader.contains("floor(time"));
        assert!(!shader.contains("clock_origin.x + control.clock_origin.y"));
        assert!(shader.contains(
            "    let coarse = select(a.x - b.x, 0.0, bitcast<u32>(a.x) == bitcast<u32>(b.x));\n    return coarse * one + (a.y - b.y);"
        ));
        assert!(shader.contains("let slot = next_slot();"));
        assert!(shader.contains("f32((next_slot() + 1u) % frames),"));
        assert!(
            shader.contains(
                "due = stamp_difference(centred_stamp(), last) >= far_control.sampling.x;"
            )
        );
        assert!(shader.contains(
            "directional_history[slot].values = vec4<f32>(amplitude, far, select(0.0, 1.0, valid));"
        ));
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

    /// The ring in the shader's terms: a pass records a frame once its time
    /// stands a period past the newest frame's. Across a handoff that changes
    /// the step, with a clock drifting from the step count by more than a
    /// step, its frames still stand in time order slot by slot, a period and
    /// at most a pass more apart, none missing at the handoff.
    #[test]
    fn the_far_field_ring_holds_its_frames_in_order_across_a_handoff() {
        let period = far_field_period(1.0e-3);
        let mut frames: Vec<(f64, f64)> = Vec::new();
        let mut run = |origin: f64, time_step: f64, steps: u64, drift: f64| {
            let stride = far_field_sample_stride(period, time_step);
            for step in 1..=steps {
                let time = origin + step as f64 * (time_step + drift);
                if probe_sample_due(step, stride)
                    && frames.last().is_none_or(|(last, _)| time - last >= period)
                {
                    frames.push((time, stride as f64 * time_step));
                }
            }
            origin + steps as f64 * (time_step + drift)
        };
        let handoff = run(0.0, 1.0 / 700.0, 4000, 0.002 / 700.0);
        run(handoff, 1.0 / 1100.0, 4000, 0.0);
        assert!(frames.len() > 300);
        for pair in frames.windows(2) {
            let ((before, _), (after, pass)) = (pair[0], pair[1]);
            // A hole in the ring is a hole in every projection that reaches
            // back through it, and frames out of order break its bisection.
            assert!(
                after - before >= period && after - before <= period + pass + 1.0e-9,
                "{after} follows {before}"
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
    /// PR-4 of the 2026-10-20 review: a recorder whose stencils could not be
    /// built for a new generation kept the old mesh's, and sampled unrelated
    /// nodes of the new one for the whole generation. The validation
    /// rejections cleared it; the reconstructions returned through `?`. Every
    /// rejection clears the recorder.
    #[test]
    fn a_stencil_that_cannot_be_built_clears_its_recorder() {
        let operator = canonical_operator();
        let mut world = World::new();
        let mut assets = Assets::<ShaderBuffer>::default();
        let mut request = WaveGpuRequest::default();
        let context = RecorderContext {
            time_step: 1.0e-3,
            physics: PhysicsModel::Mechanical,
            history: RecorderHistory::Keep,
        };
        // An element the operator does not have.
        let foreign = QuadraticPointStencil {
            element: u32::MAX,
            barycentric: [1.0 / 3.0; 3],
            nodes: [0; 7],
            value_weights: [0.0; 7],
            gradient_weights: [Point2::default(); 7],
            region: funfern_core::BACKGROUND_REGION,
            mass_density: 1.0,
            stiffness: funfern_core::SymmetricTensor2::isotropic(1.0),
        };
        let mut queue = CommandQueue::default();
        {
            let mut commands = Commands::new(&mut queue, &world);
            for stencil in [None, Some(foreign)] {
                let points = request.update_canonical_point_probes(
                    &mut assets,
                    &mut commands,
                    &operator,
                    &[(1, stencil)],
                    120.0,
                    context,
                );
                let curves = request.update_canonical_curve_probes(
                    &mut assets,
                    &mut commands,
                    &operator,
                    &[CurveProbeInput {
                        id: 1,
                        sample_rate: 60.0,
                        samples: vec![stencil.map(|stencil| (stencil, Point2::new(1.0, 0.0))); 2],
                    }],
                    context,
                );
                assert_eq!(points.is_ok(), stencil.is_none());
                assert_eq!(curves.is_ok(), stencil.is_none());
            }
        }
        queue.apply(&mut world);
        assert!(request.probes.is_none(), "the point recorder is cleared");
        assert!(
            request.curve_probes.is_none(),
            "the line recorder is cleared"
        );
    }

    /// PR-10 of the 2026-10-20 review: the line recorder tests each probe's
    /// stride again on the device's u32 count. The host chose its steps on
    /// its own u64 count, and past 2^32 steps, where 2^32 mod 36 = 4, no step
    /// it chose for a stride of 36 passed the shader's test: the probe
    /// stopped recording, with nothing said. Every step the host chooses is
    /// one the shader records.
    /// PR-5 of the 2026-10-20 review: the frames of a kept ring the source
    /// wrote past a handoff's snapshot, which the target writes again only
    /// as it gets there.
    #[test]
    fn a_handoff_marks_the_ring_frames_past_its_snapshot() {
        // Steps 104, 112, 120 and 128 of a stride of 8: not the snapshot's.
        assert_eq!(stale_frames(100, 130, 8, 2048), vec![13, 14, 15, 16]);
        assert_eq!(stale_frames(104, 104, 8, 2048), Vec::<usize>::new());
        // Round the ring: every frame.
        assert_eq!(stale_frames(0, 8 * 64, 8, 64).len(), 64);
        // Across the device's wrap, as it schedules there: 2^32 - 4 before
        // it, in frame 7, and 0 and 36 after.
        let wrap = 1_u64 << 32;
        assert_eq!(stale_frames(wrap - 40, wrap + 40, 36, 64), vec![0, 1, 7]);

        let operator = canonical_operator();
        let world = World::new();
        let mut assets = Assets::<ShaderBuffer>::default();
        let mut request = WaveGpuRequest::default();
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        let context = RecorderContext {
            time_step: 1.0 / 960.0,
            physics: PhysicsModel::Mechanical,
            history: RecorderHistory::Keep,
        };
        request
            .update_canonical_point_probes(
                &mut assets,
                &mut commands,
                &operator,
                &[(1, None)],
                120.0,
                context,
            )
            .unwrap();
        request
            .update_canonical_curve_probes(
                &mut assets,
                &mut commands,
                &operator,
                &[CurveProbeInput {
                    id: 1,
                    sample_rate: 60.0,
                    samples: vec![None; 3],
                }],
                context,
            )
            .unwrap();
        let strides = request.recorder_strides();
        assert_eq!(strides.point, Some(8));
        assert_eq!(strides.curves, vec![16]);
        let ranges = request
            .stale_ring_ranges(100, 130, &strides)
            .into_iter()
            .map(|(_, offset, length)| (offset, length))
            .collect::<Vec<_>>();
        let point = 16 * 48;
        assert_eq!(
            ranges,
            vec![
                (13 * point, point),
                (14 * point, point),
                (15 * point, point),
                (16 * point, point),
                // Steps 112 and 128 of the line probe's 16: its three points.
                (7 * 512 * 32, 3 * 32),
                (8 * 512 * 32, 3 * 32),
            ]
        );
    }

    #[test]
    fn the_host_chooses_recorder_steps_on_the_devices_count() {
        let wrap = 1_u64 << 32;
        for stride in [36_u64, 16, 1] {
            let shader_records = |step: u64| (step as u32).is_multiple_of(stride as u32);
            let chosen = (wrap - 100..wrap + 1000)
                .filter(|step| probe_sample_due(device_step(*step), stride))
                .collect::<Vec<_>>();
            assert!(
                chosen.iter().all(|step| shader_records(*step)),
                "stride {stride}: the shader refuses steps the host chose"
            );
            assert!(chosen.len() as u64 >= 1100 / stride - 1, "stride {stride}");
        }
    }

    /// The area probes' buffers are admitted against what the device binds,
    /// as the solver's are (GPU-7 of the 2026-10-20 review). Past it, the
    /// recorder's bind group failed validation and rendering stopped for good:
    /// a 1,168-byte record per element piece passes WebGPU's default 128 MiB
    /// at about 115k pieces, where the cap allows 200k. Refused, the run goes
    /// on and the refusal is the recorder's error.
    #[test]
    fn area_probes_past_the_device_limits_are_refused() {
        let operator = canonical_operator();
        let world = World::new();
        let mut assets = Assets::<ShaderBuffer>::default();
        let areas = vec![AreaProbeInput {
            id: 1,
            stencil: None,
        }];
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        let context = RecorderContext {
            time_step: 0.01,
            physics: PhysicsModel::Mechanical,
            history: RecorderHistory::Restart,
        };
        for (binding, admitted) in [(1 << 30, true), (1024, false)] {
            let mut request = WaveGpuRequest::default();
            request.set_limits(Some(CanonicalGpuLimits {
                max_storage_buffer_binding_size: binding,
                max_buffer_size: 1 << 30,
            }));
            let result = request.update_canonical_area_probes(
                &mut assets,
                &mut commands,
                &operator,
                &areas,
                60.0,
                context,
            );
            assert_eq!(result.is_ok(), admitted, "binding {binding}: {result:?}");
            if let Err(refusal) = result {
                assert!(refusal.contains("this device binds"), "{refusal}");
                assert!(request.area_probes.is_none(), "nothing is left bound");
            }
        }
    }

    #[test]
    fn a_new_mesh_over_the_same_probes_inherits_their_rings() {
        let operator = canonical_operator();
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
                    .update_canonical_point_probes(
                        assets,
                        &mut commands,
                        &operator,
                        &points(point_ids),
                        120.0,
                        context,
                    )
                    .unwrap();
                request
                    .update_canonical_curve_probes(
                        assets,
                        &mut commands,
                        &operator,
                        &curves(1, curve_samples),
                        context,
                    )
                    .unwrap();
                request
                    .update_canonical_area_probes(
                        assets,
                        &mut commands,
                        &operator,
                        &areas(point_ids),
                        60.0,
                        context,
                    )
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
                .update_canonical_far_field(
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
            .update_canonical_far_field(
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
    fn the_far_field_passes_come_often_enough_to_keep_frames_a_period_apart() {
        for time_step in [1.0e-4, 1.0 / 700.0, 0.004, 0.02, 0.4] {
            let period = far_field_period(time_step);
            let stride = far_field_sample_stride(period, time_step);
            let spacing = stride as f64 * time_step;
            // A frame comes at most a pass after it is due.
            assert!(
                spacing <= period,
                "{spacing} between passes for a period of {period}"
            );
            // Dense enough to keep frames near a period apart, sparse enough
            // that the decision is not taken at every step of one. Rounding a
            // quarter period down to whole steps can halve it.
            assert!(spacing >= period * 0.125 || stride == 1);
        }
    }

    #[test]
    fn webgpu_probe_shaders_do_not_construct_nan_as_a_constant_expression() {
        for shader in [
            include_str!("canonical_curve_probe.wgsl"),
            include_str!("canonical_area_probe.wgsl"),
            include_str!("canonical_far_field.wgsl"),
        ] {
            assert!(!shader.contains("bitcast<f32>(0x7fc00000u)"));
        }
    }
}
