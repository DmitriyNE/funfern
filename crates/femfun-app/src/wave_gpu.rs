use std::{
    borrow::Cow,
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
use femfun_core::{Point2, QuadraticTransferMap, QuadraticWaveOperator, RegionId, TriMesh};

const WORKGROUP_SIZE: u32 = 128;
const STATUS_READY: u8 = 1;
const STATUS_ERROR: u8 = 2;
const STATUS_TRANSFERRING: u8 = 3;

#[derive(Clone, Copy, Debug)]
pub struct SourceSettings {
    pub enabled: bool,
    pub position: Point2,
    pub amplitude: f32,
    pub width: f32,
    pub frequency_hz: f32,
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

impl Default for SourceSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            position: Point2::new(-0.45, 0.0),
            amplitude: 18.0,
            width: 0.06,
            frequency_hz: 2.5,
            region: femfun_core::BACKGROUND_REGION,
        }
    }
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
    source: Handle<ShaderBuffer>,
    pulse: Handle<ShaderBuffer>,
    row_offsets: Handle<ShaderBuffer>,
    columns: Handle<ShaderBuffer>,
    stiffness: Handle<ShaderBuffer>,
    nodes: Handle<ShaderBuffer>,
    state: Handle<ShaderBuffer>,
}

impl WaveBufferHandles {
    fn all(&self) -> [&Handle<ShaderBuffer>; 8] {
        [
            &self.parameters,
            &self.source,
            &self.pulse,
            &self.row_offsets,
            &self.columns,
            &self.stiffness,
            &self.nodes,
            &self.state,
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
        }
    }
}

impl WaveGpuRequest {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn stats(&self) -> &Arc<WaveGpuStats> {
        &self.stats
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
        let preserve_auxiliary = source_operator.outer_boundary()
            == femfun_core::OuterBoundaryCondition::SecondOrderOutgoing
            && target_operator.outer_boundary()
                == femfun_core::OuterBoundaryCondition::SecondOrderOutgoing;
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
        source: SourceSettings,
    ) -> Result<(), String> {
        let handles = self
            .buffers
            .as_mut()
            .ok_or("The wave solver is not initialized")?;
        let replacement = assets.add(ShaderBuffer::from(gpu_source(source)));
        let old = std::mem::replace(&mut handles.source, replacement);
        assets.remove(old.id());
        self.buffer_revision = self.buffer_revision.wrapping_add(1);
        Ok(())
    }

    pub fn inject_pulse(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        position: Point2,
        amplitude: f32,
        width: f32,
        region: RegionId,
    ) -> Result<(), String> {
        let handles = self
            .buffers
            .as_mut()
            .ok_or("The wave solver is not initialized")?;
        let pulse = GpuPulse {
            position_width_amplitude: Vec4::new(
                position.x as f32,
                position.y as f32,
                width * width,
                amplitude,
            ),
            region: gpu_region_pair(region, RegionId(0)),
        };
        if !pulse.position_width_amplitude.is_finite() || width <= 0.0 {
            return Err("Pulse parameters must be finite with positive width".into());
        }
        let replacement = assets.add(ShaderBuffer::from(pulse));
        let old = std::mem::replace(&mut handles.pulse, replacement);
        assets.remove(old.id());
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
    let nodes: Vec<_> = operator
        .node_points()
        .iter()
        .zip(damping)
        .zip(operator.auxiliary_active())
        .zip(node_regions(mesh, operator)?)
        .map(|(((point, damping), auxiliary_active), regions)| GpuNode {
            position_damping: Vec4::new(
                point.x as f32,
                point.y as f32,
                damping,
                if *auxiliary_active { 1.0 } else { 0.0 },
            ),
            regions: gpu_region_pair(regions[0], regions[1]),
        })
        .collect();
    if nodes
        .iter()
        .any(|node| !node.position_damping.x.is_finite() || !node.position_damping.y.is_finite())
    {
        return Err("Mesh coordinates cannot be represented on the GPU".into());
    }
    let initial_state = GpuState {
        levels: Vec4::new(0.0, 0.0, 0.0, if awaits_transfer { -1.0 } else { 0.0 }),
        auxiliary: Vec4::ZERO,
    };
    let parameters = GpuParameters {
        time_data: Vec4::new(dt, dt * dt, 0.0, 0.0),
        count_data: UVec4::new(dof_count, 0, 0, 0),
    };
    let pulse = GpuPulse {
        position_width_amplitude: Vec4::new(0.0, 0.0, 0.06_f32.powi(2), 0.65),
        region: gpu_region_pair(femfun_core::BACKGROUND_REGION, RegionId(0)),
    };
    Ok((
        WaveBufferHandles {
            parameters: assets.add(ShaderBuffer::from(parameters)),
            source: assets.add(ShaderBuffer::from(gpu_source(source))),
            pulse: assets.add(ShaderBuffer::from(pulse)),
            row_offsets: assets.add(ShaderBuffer::from(operator.row_offsets().to_vec())),
            columns: assets.add(ShaderBuffer::from(operator.columns().to_vec())),
            stiffness: assets.add(ShaderBuffer::from(matrix)),
            nodes: assets.add(ShaderBuffer::from(nodes)),
            state: assets.add(ShaderBuffer::from(vec![
                initial_state;
                operator.degrees_of_freedom()
            ])),
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
    pub completed_steps: u64,
    pub readbacks: u64,
}

#[derive(Component)]
struct WaveReadbackTag {
    generation: u64,
    stats: Arc<WaveGpuStats>,
    expects_transfer: bool,
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
struct GpuNode {
    position_damping: Vec4,
    regions: UVec4,
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
    display.current.reserve(states.len());
    display.previous.reserve(states.len());
    display.auxiliary.reserve(states.len());
    for state in states {
        display.current.push(state.levels.y);
        display.previous.push(state.levels.x);
        display.auxiliary.push(state.auxiliary.x);
    }
    display.readbacks = display.readbacks.saturating_add(1);
}

pub struct WaveGpuPlugin;

impl Plugin for WaveGpuPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "wave.wgsl");
        embedded_asset!(app, "wave_transfer_old.wgsl");
        embedded_asset!(app, "wave_transfer_new.wgsl");
        app.init_resource::<WaveGpuRequest>()
            .init_resource::<WaveDisplay>()
            .add_observer(receive_readback)
            .add_plugins(ExtractResourcePlugin::<WaveGpuRequest>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(
                Render,
                prepare_bind_group.in_set(RenderSystems::PrepareBindGroups),
            )
            .add_systems(RenderGraph, compute_wave.before(camera_driver));
    }
}

#[derive(Resource)]
struct WavePipeline {
    layout: BindGroupLayoutDescriptor,
    transfer_old_layout: BindGroupLayoutDescriptor,
    transfer_new_layout: BindGroupLayoutDescriptor,
    step: CachedComputePipelineId,
    rotate: CachedComputePipelineId,
    inject: CachedComputePipelineId,
    transfer_velocity: CachedComputePipelineId,
    transfer_old: CachedComputePipelineId,
    transfer_new: CachedComputePipelineId,
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
                storage_buffer_read_only::<GpuSource>(false),
                storage_buffer_read_only::<GpuPulse>(false),
                storage_buffer_read_only::<Vec<u32>>(false),
                storage_buffer_read_only::<Vec<u32>>(false),
                storage_buffer_read_only::<Vec<GpuMatrixEntry>>(false),
                storage_buffer_read_only::<Vec<GpuNode>>(false),
                storage_buffer::<Vec<GpuState>>(false),
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
    let step = pipeline_cache.queue_compute_pipeline(pipeline("step"));
    let rotate = pipeline_cache.queue_compute_pipeline(pipeline("rotate"));
    let inject = pipeline_cache.queue_compute_pipeline(pipeline("inject"));
    let transfer_old_layout = BindGroupLayoutDescriptor::new(
        "wave old-state transfer buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer_read_only::<GpuParameters>(false),
                storage_buffer_read_only::<GpuSource>(false),
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
                storage_buffer_read_only::<GpuSource>(false),
                storage_buffer_read_only::<Vec<u32>>(false),
                storage_buffer_read_only::<Vec<u32>>(false),
                storage_buffer_read_only::<Vec<GpuMatrixEntry>>(false),
                storage_buffer_read_only::<Vec<GpuNode>>(false),
                storage_buffer::<Vec<GpuState>>(false),
                storage_buffer_read_only::<Vec<GpuTransferEntry>>(false),
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
    let transfer_new = pipeline_cache.queue_compute_pipeline(transfer_pipeline(
        "wave transfer new",
        "transfer",
        transfer_new_layout.clone(),
        load_embedded_asset!(asset_server.as_ref(), "wave_transfer_new.wgsl"),
    ));
    commands.insert_resource(WavePipeline {
        layout,
        transfer_old_layout,
        transfer_new_layout,
        step,
        rotate,
        inject,
        transfer_velocity,
        transfer_old,
        transfer_new,
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
    let Some(source) = gpu_buffers.get(&handles.source) else {
        return;
    };
    let Some(pulse) = gpu_buffers.get(&handles.pulse) else {
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
    let bind_group = render_device.create_bind_group(
        Some("wave gather bind group"),
        &pipeline_cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((
            parameters.buffer.as_entire_buffer_binding(),
            source.buffer.as_entire_buffer_binding(),
            pulse.buffer.as_entire_buffer_binding(),
            row_offsets.buffer.as_entire_buffer_binding(),
            columns.buffer.as_entire_buffer_binding(),
            stiffness.buffer.as_entire_buffer_binding(),
            nodes.buffer.as_entire_buffer_binding(),
            state.buffer.as_entire_buffer_binding(),
        )),
    );
    if let Some(transfer) = &request.transfer {
        let old = &transfer.old;
        let Some(old_parameters) = gpu_buffers.get(&old.parameters) else {
            return;
        };
        let Some(old_source) = gpu_buffers.get(&old.source) else {
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
                old_source.buffer.as_entire_buffer_binding(),
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
                source.buffer.as_entire_buffer_binding(),
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

fn compute_wave(
    mut render_context: RenderContext,
    request: Option<Res<WaveGpuRequest>>,
    group: Option<ResMut<WaveBindGroup>>,
    transfer_groups: Option<Res<WaveTransferBindGroups>>,
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
        pipeline.transfer_new,
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
        let (Some(transfer_velocity), Some(transfer_old), Some(transfer_new)) = (
            pipeline_cache.get_compute_pipeline(pipeline.transfer_velocity),
            pipeline_cache.get_compute_pipeline(pipeline.transfer_old),
            pipeline_cache.get_compute_pipeline(pipeline.transfer_new),
        ) else {
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
        pass.set_pipeline(transfer_new);
        pass.set_bind_group(0, &transfer_groups.new, &[]);
        pass.dispatch_workgroups(workgroups, 1, 1);
        drop(pass);
        group.initialized = true;
        request
            .stats
            .status
            .store(STATUS_TRANSFERRING, Ordering::Relaxed);
        request.stats.dispatches.fetch_add(3, Ordering::Relaxed);
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
    for _ in 0..pending {
        pass.set_pipeline(step);
        pass.dispatch_workgroups(workgroups, 1, 1);
        pass.set_pipeline(rotate);
        pass.dispatch_workgroups(workgroups, 1, 1);
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
            region: femfun_core::BACKGROUND_REGION,
        });
        assert_eq!(source.position_width_amplitude.x, 0.2);
        assert_eq!(source.position_width_amplitude.y, -0.3);
        assert!((source.position_width_amplitude.z - 0.0016).abs() < 1.0e-9);
        assert_eq!(source.position_width_amplitude.w, 7.0);
        assert!((source.frequency_enabled.x - 5.0 * std::f32::consts::PI).abs() < 1.0e-6);
        assert_eq!(source.frequency_enabled.y, 1.0);
    }
}
