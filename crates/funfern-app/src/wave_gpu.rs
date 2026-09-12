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
use funfern_app::editor::SourceSettings;
use funfern_core::{
    BoundarySignal, OuterBoundaryConditions, Point2, QuadraticAreaElement, QuadraticAreaStencil,
    QuadraticPointStencil, QuadraticTransferMap, QuadraticWaveOperator, QuadraticWaveState,
    RegionId, TriMesh,
};

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
const WAVE_STORAGE_BINDINGS: usize = 8;
const TRANSFER_STORAGE_BINDINGS: usize = 8;
const AREA_PROBE_STORAGE_BINDINGS: usize = 7;
const WEBGPU_PORTABLE_STORAGE_BUFFER_LIMIT: usize = 8;
const _: () = {
    assert!(WAVE_STORAGE_BINDINGS <= WEBGPU_PORTABLE_STORAGE_BUFFER_LIMIT);
    assert!(TRANSFER_STORAGE_BINDINGS <= WEBGPU_PORTABLE_STORAGE_BUFFER_LIMIT);
    assert!(AREA_PROBE_STORAGE_BINDINGS <= WEBGPU_PORTABLE_STORAGE_BUFFER_LIMIT);
};
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
    pub source: SourceSettings,
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
    source: SourceSettings,
    pulse: PulseSettings,
    source_weights: Arc<[f32]>,
    pulse_weights: Arc<[f32]>,
}

#[derive(Clone)]
struct ProbeBufferHandles {
    stencils: Handle<ShaderBuffer>,
    control: Handle<ShaderBuffer>,
    output: Handle<ShaderBuffer>,
    ids: Arc<[u64]>,
    sample_stride: u64,
}

#[derive(Clone)]
struct CurveProbeBufferHandles {
    stencils: Handle<ShaderBuffer>,
    control: Handle<ShaderBuffer>,
    output: Handle<ShaderBuffer>,
    descriptors: Arc<[CurveProbeDescriptor]>,
    sample_strides: Arc<[u64]>,
    point_count: u32,
}

#[derive(Clone)]
struct AreaProbeBufferHandles {
    contributions: Handle<ShaderBuffer>,
    descriptors: Handle<ShaderBuffer>,
    control: Handle<ShaderBuffer>,
    scratch: Handle<ShaderBuffer>,
    output: Handle<ShaderBuffer>,
    ids: Arc<[u64]>,
    sample_stride: u64,
    contribution_count: u32,
}

#[derive(Clone)]
struct FarFieldBufferHandles {
    stencils: Handle<ShaderBuffer>,
    control: Handle<ShaderBuffer>,
    raw: Handle<ShaderBuffer>,
    output: Handle<ShaderBuffer>,
    sample_stride: u64,
}

impl FarFieldBufferHandles {
    fn all(&self) -> [&Handle<ShaderBuffer>; 4] {
        [&self.stencils, &self.control, &self.raw, &self.output]
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
    probes: Option<ProbeBufferHandles>,
    probe_revision: u64,
    probe_readback_entity: Option<Entity>,
    curve_probes: Option<CurveProbeBufferHandles>,
    curve_probe_revision: u64,
    curve_probe_readback_entity: Option<Entity>,
    area_probes: Option<AreaProbeBufferHandles>,
    area_probe_revision: u64,
    area_probe_readback_entity: Option<Entity>,
    far_field: Option<FarFieldBufferHandles>,
    far_field_revision: u64,
    far_field_readback_entity: Option<Entity>,
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
        }
    }
}

impl WaveGpuRequest {
    pub fn generation(&self) -> u64 {
        self.generation
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

    pub fn update_point_probes(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        probes: &[(u64, Option<QuadraticPointStencil>)],
        sample_rate: f64,
        time_step: f64,
    ) -> Result<(), String> {
        if probes.len() > MAX_POINT_PROBES
            || !sample_rate.is_finite()
            || !(30.0..=480.0).contains(&sample_rate)
            || !time_step.is_finite()
            || time_step <= 0.0
        {
            return Err("Invalid point-probe recorder settings".into());
        }
        self.clear_probe_buffers(assets, commands);
        if probes.is_empty() {
            return Ok(());
        }
        let sample_stride = (1.0 / (sample_rate * time_step)).round().max(1.0) as u64;
        let stencils = probes
            .iter()
            .map(|(_, stencil)| gpu_probe_stencil(*stencil))
            .collect::<Vec<_>>();
        let control = GpuProbeControl {
            values: Vec4::new(
                sample_stride as f32,
                PROBE_RING_FRAMES as f32,
                probes.len() as f32,
                0.0,
            ),
        };
        let output = vec![
            GpuProbeSample {
                values: Vec4::splat(f32::NAN),
            };
            PROBE_RING_FRAMES * MAX_POINT_PROBES
        ];
        let handles = ProbeBufferHandles {
            stencils: assets.add(ShaderBuffer::from(stencils)),
            control: assets.add(ShaderBuffer::from(control)),
            output: assets.add(ShaderBuffer::from(output)),
            ids: probes.iter().map(|(id, _)| *id).collect(),
            sample_stride,
        };
        self.probe_revision = self.probe_revision.wrapping_add(1).max(1);
        self.probe_readback_entity = Some(
            commands
                .spawn((
                    Readback::buffer(handles.output.clone()),
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

    pub fn update_curve_probes(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        probes: &[CurveProbeInput],
        time_step: f64,
    ) -> Result<(), String> {
        if !time_step.is_finite() || time_step <= 0.0 {
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
            return Err("Invalid line-probe recorder settings".into());
        }
        self.clear_curve_probe_buffers(assets, commands);
        if probes.is_empty() {
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
                0.0,
            ),
        };
        let output = vec![
            GpuProbeSample {
                values: Vec4::splat(f32::NAN),
            };
            CURVE_PROBE_RING_FRAMES * MAX_CURVE_PROBE_POINTS
        ];
        let handles = CurveProbeBufferHandles {
            stencils: assets.add(ShaderBuffer::from(stencils)),
            control: assets.add(ShaderBuffer::from(control)),
            output: assets.add(ShaderBuffer::from(output)),
            descriptors: descriptors.into(),
            sample_strides: sample_strides.into(),
            point_count: point_count as u32,
        };
        self.curve_probe_revision = self.curve_probe_revision.wrapping_add(1).max(1);
        self.curve_probe_readback_entity = Some(
            commands
                .spawn((
                    Readback::buffer(handles.output.clone()),
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
        time_step: f64,
    ) -> Result<(), String> {
        let contribution_count = probes
            .iter()
            .filter_map(|probe| probe.stencil.as_ref())
            .map(|stencil| stencil.elements.len())
            .sum::<usize>();
        self.clear_area_probe_buffers(assets, commands);
        if probes.len() > MAX_POINT_PROBES {
            return Err(format!("Maximum {MAX_POINT_PROBES} area probes"));
        }
        if contribution_count > MAX_AREA_PROBE_ELEMENTS {
            return Err(format!(
                "Area probes cover {contribution_count} element pieces; maximum is {MAX_AREA_PROBE_ELEMENTS}"
            ));
        }
        if !sample_rate.is_finite()
            || !(30.0..=120.0).contains(&sample_rate)
            || !time_step.is_finite()
            || time_step <= 0.0
        {
            return Err("Invalid area-probe recorder settings".into());
        }
        if probes.is_empty() {
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
                        .map(gpu_area_probe_contribution),
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
                || !contribution.material_area.is_finite()
        }) || descriptors
            .iter()
            .any(|descriptor| !descriptor.areas.is_finite())
        {
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
        let scratch = vec![GpuProbeSample::default(); contribution_count.max(1)];
        let output = vec![
            GpuAreaProbeSample {
                primary: Vec4::splat(f32::NAN),
                secondary: Vec4::splat(f32::NAN),
            };
            AREA_PROBE_RING_FRAMES * MAX_POINT_PROBES
        ];
        let handles = AreaProbeBufferHandles {
            contributions: assets.add(ShaderBuffer::from(contributions)),
            descriptors: assets.add(ShaderBuffer::from(descriptors)),
            control: assets.add(ShaderBuffer::from(control)),
            scratch: assets.add(ShaderBuffer::from(scratch)),
            output: assets.add(ShaderBuffer::from(output)),
            ids: probes.iter().map(|probe| probe.id).collect(),
            sample_stride,
            contribution_count: contribution_count as u32,
        };
        self.area_probe_revision = self.area_probe_revision.wrapping_add(1).max(1);
        self.area_probe_readback_entity = Some(
            commands
                .spawn((
                    Readback::buffer(handles.output.clone()),
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
    ) -> Result<(), String> {
        self.clear_far_field_buffers(assets, commands);
        let Some(input) = input else {
            return Ok(());
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
            return Err("Invalid far-field recorder settings".into());
        }
        let sample_stride = (1.0 / (FAR_FIELD_SAMPLE_RATE * time_step)).round().max(1.0) as u64;
        let sample_time = sample_stride as f64 * time_step;
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
            return Err("Far-field stencils cannot be represented on the GPU".into());
        }
        let maximum_history = (FAR_FIELD_RING_FRAMES - 2) as f64 * sample_time;
        if 2.0 * input.delay_margin > maximum_history {
            return Err(format!(
                "Exterior wave speed is too low for the {:.1} s far-field delay window",
                maximum_history
            ));
        }
        let control = GpuFarFieldControl {
            sampling: Vec4::new(
                sample_stride as f32,
                FAR_FIELD_RING_FRAMES as f32,
                FAR_FIELD_CONTOUR_POINTS as f32,
                FAR_FIELD_DIRECTIONS as f32,
            ),
            projection: Vec4::new(
                input.wave_speed as f32,
                input.sample_spacing as f32,
                input.delay_margin as f32,
                sample_time as f32,
            ),
        };
        let raw = vec![
            GpuProbeSample { values: Vec4::NAN };
            FAR_FIELD_RING_FRAMES * FAR_FIELD_CONTOUR_POINTS
        ];
        let output = vec![
            GpuProbeSample { values: Vec4::NAN };
            FAR_FIELD_RING_FRAMES * FAR_FIELD_DIRECTIONS
        ];
        let handles = FarFieldBufferHandles {
            stencils: assets.add(ShaderBuffer::from(stencils)),
            control: assets.add(ShaderBuffer::from(control)),
            raw: assets.add(ShaderBuffer::from(raw)),
            output: assets.add(ShaderBuffer::from(output)),
            sample_stride,
        };
        self.far_field_revision = self.far_field_revision.wrapping_add(1).max(1);
        self.far_field_readback_entity = Some(
            commands
                .spawn((
                    Readback::buffer(handles.output.clone()),
                    FarFieldReadbackTag {
                        generation: self.generation,
                        revision: self.far_field_revision,
                    },
                ))
                .id(),
        );
        self.far_field = Some(handles);
        Ok(())
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

    pub fn request_steps(&mut self, count: u64) {
        self.desired_steps = self.desired_steps.saturating_add(count);
    }

    pub fn replace(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        time_step: f64,
        source: SourceSettings,
    ) -> Result<(), String> {
        let (handles, dof_count) =
            create_buffers(assets, mesh, operator, time_step, source, false)?;
        self.install(assets, commands, handles, dof_count, None);
        Ok(())
    }

    pub fn replace_transferred(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        replacement: WaveTransfer<'_>,
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
        self.clear_far_field_buffers(assets, commands);
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
                    Readback::buffer(transfer.old.state.clone()),
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
        self.clear_probe_buffers(assets, commands);
        self.clear_curve_probe_buffers(assets, commands);
        self.clear_area_probe_buffers(assets, commands);
        self.clear_far_field_buffers(assets, commands);
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
                Readback::buffer(handles.state.clone()),
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
            source.amplitude as f64
                * (std::f64::consts::TAU * source.frequency_hz as f64 * time).sin()
        } else {
            0.0
        };
        Some(
            handles
                .source_weights
                .iter()
                .map(|weight| value * *weight as f64)
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
        source: SourceSettings,
    ) -> Result<(), String> {
        self.replace(assets, commands, mesh, operator, time_step, source)
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
        source: SourceSettings,
    ) -> Result<(), String> {
        let handles = self
            .buffers
            .as_mut()
            .ok_or("The wave solver is not initialized")?;
        let source_weights =
            forcing_weights(mesh, operator, source.position, source.width, source.region)?;
        let forcing = assets.add(ShaderBuffer::from(gpu_forcing(
            source,
            handles.pulse,
            operator.outer_boundaries(),
        )?));
        let weights = assets.add(ShaderBuffer::from(zip_forcing_weights(
            &source_weights,
            &handles.pulse_weights,
        )?));
        let old = std::mem::replace(&mut handles.forcing, forcing);
        assets.remove(old.id());
        let old = std::mem::replace(&mut handles.forcing_weights, weights);
        assets.remove(old.id());
        handles.source = source;
        handles.source_weights = source_weights.into();
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
            pulse_settings.width,
            pulse_settings.region,
        )?;
        let forcing = assets.add(ShaderBuffer::from(gpu_forcing(
            handles.source,
            pulse_settings,
            operator.outer_boundaries(),
        )?));
        let weights = assets.add(ShaderBuffer::from(zip_forcing_weights(
            &handles.source_weights,
            &pulse_weights,
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
    source: SourceSettings,
    awaits_transfer: bool,
) -> Result<(WaveBufferHandles, u32), String> {
    WaveGpuRequest::validate_inputs(mesh, operator, time_step)?;
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
    let regions = node_regions(mesh, operator)?;
    let nodes: Vec<_> = operator
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
                regions: gpu_region_pair(regions[index][0], regions[index][1]),
                boundary: UVec4::new(u32::from(dirichlet.is_some()), 0, 0, 0),
                neumann_weights: Vec4::from_array(
                    operator.normalized_neumann_weights()[index].map(|value| value as f32),
                ),
                dirichlet_signal: gpu_boundary_signal(dirichlet.unwrap_or(BoundarySignal::ZERO)),
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
        .collect();
    if nodes
        .iter()
        .any(|node| !node.position_damping.x.is_finite() || !node.position_damping.y.is_finite())
    {
        return Err("Mesh coordinates cannot be represented on the GPU".into());
    }
    let parameters = GpuParameters {
        time_data: Vec4::new(dt, dt * dt, 0.0, 0.0),
        count_data: UVec4::new(dof_count, 0, 0, 0),
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
        0.06,
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
        })
        .collect::<Vec<_>>();
    Ok((
        WaveBufferHandles {
            parameters: assets.add(ShaderBuffer::from(parameters)),
            forcing: assets.add(ShaderBuffer::from(gpu_forcing(
                source,
                pulse,
                operator.outer_boundaries(),
            )?)),
            row_offsets: assets.add(ShaderBuffer::from(operator.row_offsets().to_vec())),
            columns: assets.add(ShaderBuffer::from(operator.columns().to_vec())),
            stiffness: assets.add(ShaderBuffer::from(matrix)),
            nodes: assets.add(ShaderBuffer::from(nodes)),
            state: assets.add(ShaderBuffer::from(initial_states)),
            forcing_weights: assets.add(ShaderBuffer::from(zip_forcing_weights(
                &source_weights,
                &pulse_weights,
            )?)),
            source,
            pulse,
            source_weights: source_weights.into(),
            pulse_weights: pulse_weights.into(),
        },
        dof_count,
    ))
}

fn gpu_source(source: SourceSettings) -> GpuSource {
    GpuSource {
        position_width_amplitude: Vec4::new(
            source.position.x as f32,
            source.position.y as f32,
            source.width * source.width,
            source.amplitude,
        ),
        frequency_enabled: Vec4::new(
            std::f32::consts::TAU * source.frequency_hz,
            if source.enabled { 1.0 } else { 0.0 },
            0.0,
            0.0,
        ),
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
    source: SourceSettings,
    pulse: PulseSettings,
    boundaries: OuterBoundaryConditions,
) -> Result<GpuForcing, String> {
    let signals = boundaries
        .sides
        .map(|condition| gpu_boundary_signal(condition.signal().unwrap_or(BoundarySignal::ZERO)));
    if signals.iter().any(|values| !values.is_finite()) {
        return Err("Boundary signal values cannot be represented on the GPU".into());
    }
    Ok(GpuForcing {
        source: gpu_source(source),
        pulse: gpu_pulse(pulse),
        outer: signals.map(|values| GpuBoundarySignal { values }),
    })
}

fn gpu_boundary_signal(signal: BoundarySignal) -> Vec4 {
    Vec4::new(
        signal.offset as f32,
        signal.amplitude as f32,
        (std::f64::consts::TAU * signal.frequency_hz) as f32,
        signal.phase_radians as f32,
    )
}

fn zip_forcing_weights(source: &[f32], pulse: &[f32]) -> Result<Vec<Vec2>, String> {
    if source.len() != pulse.len() {
        return Err("Source and pulse weights do not match the wave discretization".into());
    }
    Ok(source
        .iter()
        .zip(pulse)
        .map(|(&source, &pulse)| Vec2::new(source, pulse))
        .collect())
}

fn node_regions(
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
) -> Result<Vec<[RegionId; 2]>, String> {
    let mut regions = vec![[RegionId(0); 2]; operator.degrees_of_freedom()];
    for (triangle, nodes) in mesh.triangles.iter().zip(operator.element_nodes()) {
        for node in nodes {
            let assigned = &mut regions[*node as usize];
            if assigned.contains(&triangle.region) {
                continue;
            }
            if assigned[0].0 == 0 {
                assigned[0] = triangle.region;
            } else if assigned[1].0 == 0 {
                assigned[1] = triangle.region;
            } else {
                return Err("A wave node belongs to more than two material regions".into());
            }
        }
    }
    if regions.iter().any(|regions| regions[0].0 == 0) {
        return Err("A wave node does not belong to a material region".into());
    }
    Ok(regions)
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
    width: f32,
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
    let width = width as f64;
    if !mesh.boundary_edges.iter().any(|edge| {
        matches!(
            edge.label,
            funfern_core::BoundaryLabel::InternalBoundary { .. }
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

fn gpu_region_pair(first: RegionId, second: RegionId) -> UVec4 {
    UVec4::new(
        first.0 as u32,
        (first.0 >> 32) as u32,
        second.0 as u32,
        (second.0 >> 32) as u32,
    )
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

#[derive(Clone, Copy, Debug)]
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointProbeRecord {
    pub probe_id: u64,
    pub time: f64,
    pub displacement: f64,
    pub velocity: f64,
    pub energy_density: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CurveProbeRecord {
    pub probe_id: u64,
    pub time: f64,
    pub displacement: Vec<f32>,
    pub energy_density: Vec<f32>,
    pub normal_flux: Vec<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaProbeRecord {
    pub probe_id: u64,
    pub time: f64,
    pub mean_displacement: f64,
    pub rms_displacement: f64,
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

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuParameters {
    time_data: Vec4,
    count_data: UVec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuSource {
    position_width_amplitude: Vec4,
    frequency_enabled: Vec4,
    region: UVec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuPulse {
    position_width_amplitude: Vec4,
    region: UVec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuBoundarySignal {
    values: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuForcing {
    source: GpuSource,
    pulse: GpuPulse,
    outer: [GpuBoundarySignal; 4],
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuNode {
    position_damping: Vec4,
    regions: UVec4,
    boundary: UVec4,
    neumann_weights: Vec4,
    dirichlet_signal: Vec4,
    face_neumann_signal_a: Vec4,
    face_neumann_signal_b: Vec4,
    face_neumann_weights: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuMatrixEntry {
    coefficients: Vec2,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuState {
    levels: Vec4,
    auxiliary: Vec4,
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
            stencil.stiffness as f32,
            1.0,
            0.0,
        ),
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

fn gpu_area_probe_contribution(element: QuadraticAreaElement) -> GpuAreaProbeContribution {
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
        material_area: Vec4::new(0.0, 0.0, element.area as f32, 1.0),
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

fn probe_sample_due(completed_step: u64, stride: u64) -> bool {
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
    if states
        .iter()
        .any(|state| !state.levels.is_finite() || !state.auxiliary.is_finite())
    {
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
    display.current.reserve(states.len());
    display.previous.reserve(states.len());
    display.auxiliary.reserve(states.len());
    display.indicator_displacement.reserve(states.len());
    display.indicator_velocity.reserve(states.len());
    display.indicator_acceleration.reserve(states.len());
    for state in states {
        display.current.push(state.levels.y);
        display.previous.push(state.levels.x);
        display.auxiliary.push(state.auxiliary.x);
        display.indicator_acceleration.push(state.auxiliary.y);
        display.indicator_velocity.push(state.auxiliary.z);
        display.indicator_displacement.push(state.auxiliary.w);
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
    let samples: Vec<GpuProbeSample> = event.to_shader_type();
    if samples.len() != PROBE_RING_FRAMES * MAX_POINT_PROBES {
        return;
    }
    let mut records = Vec::new();
    for frame in 0..PROBE_RING_FRAMES {
        for (slot, id) in tag.ids.iter().copied().enumerate() {
            let value = samples[frame * MAX_POINT_PROBES + slot].values;
            if value.is_finite() {
                records.push(PointProbeRecord {
                    probe_id: id,
                    time: value.w as f64,
                    displacement: value.x as f64,
                    velocity: value.y as f64,
                    energy_density: value.z as f64,
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

fn receive_curve_probe_readback(
    event: On<ReadbackComplete>,
    tags: Query<&CurveProbeReadbackTag>,
    mut display: ResMut<CurveProbeDisplay>,
) {
    let Ok(tag) = tags.get(event.entity) else {
        return;
    };
    let samples: Vec<GpuProbeSample> = event.to_shader_type();
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
                .map(|sample| sample.values.w)
                .find(|time| time.is_finite())
            else {
                continue;
            };
            records.push(CurveProbeRecord {
                probe_id: descriptor.id,
                time: time as f64,
                displacement: values.iter().map(|sample| sample.values.x).collect(),
                energy_density: values.iter().map(|sample| sample.values.y).collect(),
                normal_flux: values.iter().map(|sample| sample.values.z).collect(),
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
                && sample.secondary.w >= 0.5
            {
                records.push(AreaProbeRecord {
                    probe_id: id,
                    time: sample.secondary.z as f64,
                    mean_displacement: sample.primary.x as f64,
                    rms_displacement: sample.primary.y as f64,
                    mean_energy_density: sample.primary.z as f64,
                    total_energy: sample.primary.w as f64,
                    covered_area: sample.secondary.x as f64,
                    coverage: sample.secondary.y as f64,
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
        let Some(time) = row
            .iter()
            .map(|sample| sample.values.z)
            .find(|time| time.is_finite())
        else {
            continue;
        };
        if row
            .iter()
            .any(|sample| !sample.values.is_finite() || sample.values.w < 0.5)
        {
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
        embedded_asset!(app, "wave.wgsl");
        embedded_asset!(app, "probe.wgsl");
        embedded_asset!(app, "curve_probe.wgsl");
        embedded_asset!(app, "area_probe.wgsl");
        embedded_asset!(app, "far_field.wgsl");
        embedded_asset!(app, "wave_transfer_old.wgsl");
        embedded_asset!(app, "wave_transfer_new.wgsl");
        app.init_resource::<WaveGpuRequest>()
            .init_resource::<WaveDisplay>()
            .init_resource::<ProbeDisplay>()
            .init_resource::<CurveProbeDisplay>()
            .init_resource::<AreaProbeDisplay>()
            .init_resource::<FarFieldDisplay>()
            .add_observer(receive_readback)
            .add_observer(receive_probe_readback)
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
                )
                    .in_set(RenderSystems::PrepareBindGroups),
            )
            .add_systems(RenderGraph, compute_wave.before(camera_driver));
    }
}

#[derive(Resource)]
struct WavePipeline {
    layout: BindGroupLayoutDescriptor,
    probe_layout: BindGroupLayoutDescriptor,
    curve_probe_layout: BindGroupLayoutDescriptor,
    area_probe_layout: BindGroupLayoutDescriptor,
    far_field_layout: BindGroupLayoutDescriptor,
    transfer_old_layout: BindGroupLayoutDescriptor,
    transfer_new_layout: BindGroupLayoutDescriptor,
    step: CachedComputePipelineId,
    rotate: CachedComputePipelineId,
    inject: CachedComputePipelineId,
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
                storage_buffer_read_only::<Vec<Vec2>>(false),
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
    let probe_layout = BindGroupLayoutDescriptor::new(
        "wave point-probe buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer_read_only::<GpuParameters>(false),
                storage_buffer_read_only::<Vec<GpuState>>(false),
                storage_buffer_read_only::<Vec<GpuProbeStencil>>(false),
                storage_buffer_read_only::<GpuProbeControl>(false),
                storage_buffer::<Vec<GpuProbeSample>>(false),
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
                storage_buffer::<Vec<GpuProbeSample>>(false),
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
                storage_buffer::<Vec<GpuProbeSample>>(false),
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
        transfer_old_layout,
        transfer_new_layout,
        step,
        rotate,
        inject,
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
struct ProbeBindGroup {
    generation: u64,
    revision: u64,
    bind_group: BindGroup,
}

#[derive(Resource)]
struct CurveProbeBindGroup {
    generation: u64,
    revision: u64,
    bind_group: BindGroup,
}

#[derive(Resource)]
struct AreaProbeBindGroup {
    generation: u64,
    revision: u64,
    bind_group: BindGroup,
}

#[derive(Resource)]
struct FarFieldBindGroup {
    generation: u64,
    revision: u64,
    bind_group: BindGroup,
}

fn prepare_far_field_bind_group(
    mut commands: Commands,
    request: Option<Res<WaveGpuRequest>>,
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
    let Some(wave) = &request.buffers else { return };
    let (Some(parameters), Some(state), Some(stencils), Some(control), Some(raw), Some(output)) = (
        gpu_buffers.get(&wave.parameters),
        gpu_buffers.get(&wave.state),
        gpu_buffers.get(&far_field.stencils),
        gpu_buffers.get(&far_field.control),
        gpu_buffers.get(&far_field.raw),
        gpu_buffers.get(&far_field.output),
    ) else {
        return;
    };
    let bind_group = render_device.create_bind_group(
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
    );
    commands.insert_resource(FarFieldBindGroup {
        generation: request.generation,
        revision: request.far_field_revision,
        bind_group,
    });
}

fn prepare_area_probe_bind_group(
    mut commands: Commands,
    request: Option<Res<WaveGpuRequest>>,
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
    let Some(wave) = &request.buffers else { return };
    let (
        Some(parameters),
        Some(state),
        Some(contributions),
        Some(descriptors),
        Some(control),
        Some(scratch),
        Some(output),
    ) = (
        gpu_buffers.get(&wave.parameters),
        gpu_buffers.get(&wave.state),
        gpu_buffers.get(&probes.contributions),
        gpu_buffers.get(&probes.descriptors),
        gpu_buffers.get(&probes.control),
        gpu_buffers.get(&probes.scratch),
        gpu_buffers.get(&probes.output),
    )
    else {
        return;
    };
    let bind_group = render_device.create_bind_group(
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
    );
    commands.insert_resource(AreaProbeBindGroup {
        generation: request.generation,
        revision: request.area_probe_revision,
        bind_group,
    });
}

fn prepare_curve_probe_bind_group(
    mut commands: Commands,
    request: Option<Res<WaveGpuRequest>>,
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
    let Some(wave) = &request.buffers else { return };
    let (Some(parameters), Some(state), Some(stencils), Some(control), Some(output)) = (
        gpu_buffers.get(&wave.parameters),
        gpu_buffers.get(&wave.state),
        gpu_buffers.get(&probes.stencils),
        gpu_buffers.get(&probes.control),
        gpu_buffers.get(&probes.output),
    ) else {
        return;
    };
    let bind_group = render_device.create_bind_group(
        Some("wave line-probe bind group"),
        &pipeline_cache.get_bind_group_layout(&pipeline.curve_probe_layout),
        &BindGroupEntries::sequential((
            parameters.buffer.as_entire_buffer_binding(),
            state.buffer.as_entire_buffer_binding(),
            stencils.buffer.as_entire_buffer_binding(),
            control.buffer.as_entire_buffer_binding(),
            output.buffer.as_entire_buffer_binding(),
        )),
    );
    commands.insert_resource(CurveProbeBindGroup {
        generation: request.generation,
        revision: request.curve_probe_revision,
        bind_group,
    });
}

fn prepare_probe_bind_group(
    mut commands: Commands,
    request: Option<Res<WaveGpuRequest>>,
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
    let Some(wave) = &request.buffers else {
        return;
    };
    let (Some(parameters), Some(state), Some(stencils), Some(control), Some(output)) = (
        gpu_buffers.get(&wave.parameters),
        gpu_buffers.get(&wave.state),
        gpu_buffers.get(&probes.stencils),
        gpu_buffers.get(&probes.control),
        gpu_buffers.get(&probes.output),
    ) else {
        return;
    };
    let bind_group = render_device.create_bind_group(
        Some("wave point-probe bind group"),
        &pipeline_cache.get_bind_group_layout(&pipeline.probe_layout),
        &BindGroupEntries::sequential((
            parameters.buffer.as_entire_buffer_binding(),
            state.buffer.as_entire_buffer_binding(),
            stencils.buffer.as_entire_buffer_binding(),
            control.buffer.as_entire_buffer_binding(),
            output.buffer.as_entire_buffer_binding(),
        )),
    );
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
    let pending = request
        .desired_steps
        .saturating_sub(group.completed_steps)
        .min(64);
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

    #[test]
    fn source_upload_uses_angular_frequency_and_squared_width() {
        let source = gpu_source(SourceSettings {
            enabled: true,
            position: Point2::new(0.2, -0.3),
            amplitude: 7.0,
            width: 0.04,
            frequency_hz: 2.5,
            region: funfern_core::BACKGROUND_REGION,
        });
        assert_eq!(source.position_width_amplitude.x, 0.2);
        assert_eq!(source.position_width_amplitude.y, -0.3);
        assert!((source.position_width_amplitude.z - 0.0016).abs() < 1.0e-9);
        assert_eq!(source.position_width_amplitude.w, 7.0);
        assert!((source.frequency_enabled.x - 5.0 * std::f32::consts::PI).abs() < 1.0e-6);
        assert_eq!(source.frequency_enabled.y, 1.0);
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
    }

    #[test]
    fn curve_probe_shader_records_profile_energy_flux_and_gaps() {
        let shader = include_str!("curve_probe.wgsl");
        assert!(shader.contains("let normal_gradient"));
        assert!(shader.contains("let flux = -stencil.material.y * velocity * normal_gradient"));
        assert!(shader.contains("bitcast<f32>(0x7fc00000u)"));
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
            nodes: [0, 1, 2, 3, 4, 5, 6],
            barycentric_vertices: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            barycentric_gradients: [
                Point2::new(-1.0, -1.0),
                Point2::new(1.0, 0.0),
                Point2::new(0.0, 1.0),
            ],
            region: funfern_core::BACKGROUND_REGION,
            coefficients: [funfern_core::EvaluatedMaterial {
                mass_density: 2.0,
                stiffness: 3.0,
                damping: 0.0,
            }; 12],
            area: 0.5,
        };
        let matrices = element.integrated_matrices();
        let uploaded = gpu_area_probe_contribution(element);
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
    }

    #[test]
    fn area_probe_shader_reduces_to_compact_physical_records() {
        let shader = include_str!("area_probe.wgsl");
        assert!(shader.contains("fn sample_area_elements"));
        assert!(shader.contains("fn reduce_area_probes"));
        assert!(shader.contains("let mean = accumulated.x / covered_area"));
        assert!(shader.contains("let rms = sqrt(max(accumulated.y / covered_area, 0.0))"));
        assert!(shader.contains("let time = parameters.time_data.z - parameters.time_data.x;"));
    }

    #[test]
    fn far_field_shader_uses_retarded_time_and_huygens_data() {
        let shader = include_str!("far_field.wgsl");
        assert!(shader.contains("fn sample_contour"));
        assert!(shader.contains("fn project_directions"));
        assert!(shader.contains("let age = (margin - projection) / sample_interval;"));
        assert!(shader.contains("sample.z - dot(normal, ray) * sample.y / wave_speed"));
        assert!(shader.contains("amplitude * amplitude"));
    }
}
