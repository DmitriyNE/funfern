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
use femfun_core::{Point2, TriMesh, WaveOperator};

const WORKGROUP_SIZE: u32 = 128;
const STATUS_READY: u8 = 1;
const STATUS_ERROR: u8 = 2;

#[derive(Clone, Copy, Debug)]
pub struct SourceSettings {
    pub enabled: bool,
    pub position: Point2,
    pub amplitude: f32,
    pub width: f32,
    pub frequency_hz: f32,
}

impl Default for SourceSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            position: Point2::new(-0.45, 0.0),
            amplitude: 18.0,
            width: 0.06,
            frequency_hz: 2.5,
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
            STATUS_ERROR => "shader error",
            _ => "loading",
        }
    }
}

#[derive(Clone)]
struct WaveBufferHandles {
    parameters: Handle<ShaderBuffer>,
    source: Handle<ShaderBuffer>,
    pulse: Handle<ShaderBuffer>,
    row_offsets: Handle<ShaderBuffer>,
    columns: Handle<ShaderBuffer>,
    stiffness: Handle<ShaderBuffer>,
    vertices: Handle<ShaderBuffer>,
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
            &self.vertices,
            &self.state,
        ]
    }
}

#[derive(Resource, Clone, ExtractResource)]
pub struct WaveGpuRequest {
    generation: u64,
    buffers: Option<WaveBufferHandles>,
    vertex_count: u32,
    desired_steps: u64,
    pulse_serial: u64,
    stats: Arc<WaveGpuStats>,
    readback_entity: Option<Entity>,
}

impl Default for WaveGpuRequest {
    fn default() -> Self {
        Self {
            generation: 0,
            buffers: None,
            vertex_count: 0,
            desired_steps: 0,
            pulse_serial: 0,
            stats: Arc::new(WaveGpuStats::default()),
            readback_entity: None,
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

    pub fn request_steps(&mut self, count: u64) {
        self.desired_steps = self.desired_steps.saturating_add(count);
    }

    pub fn replace(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        mesh: &TriMesh,
        operator: &WaveOperator,
        time_step: f64,
        source: SourceSettings,
    ) -> Result<(), String> {
        if mesh.vertices.len() != operator.degrees_of_freedom() {
            return Err("Mesh and wave operator sizes do not agree".into());
        }
        let vertex_count = u32::try_from(mesh.vertices.len())
            .map_err(|_| "The wave mesh is too large for the GPU")?;
        let normalized = operator
            .normalized_stiffness_f32()
            .map_err(|error| error.to_string())?;
        let damping = operator
            .damping_ratios_f32()
            .map_err(|error| error.to_string())?;
        let dt = time_step as f32;
        if !dt.is_finite() || dt <= 0.0 {
            return Err("The wave time step cannot be represented on the GPU".into());
        }
        let vertices: Vec<_> = mesh
            .vertices
            .iter()
            .zip(damping)
            .map(|(vertex, damping)| GpuVertex {
                position_damping: Vec4::new(
                    vertex.point.x as f32,
                    vertex.point.y as f32,
                    damping,
                    0.0,
                ),
            })
            .collect();
        if vertices.iter().any(|vertex| {
            !vertex.position_damping.x.is_finite() || !vertex.position_damping.y.is_finite()
        }) {
            return Err("Mesh coordinates cannot be represented on the GPU".into());
        }
        let states = vec![GpuState::default(); mesh.vertices.len()];
        let parameters = GpuParameters {
            time_data: Vec4::new(dt, dt * dt, 0.0, 0.0),
            count_data: UVec4::new(vertex_count, 0, 0, 0),
        };
        let pulse = GpuPulse {
            position_width_amplitude: Vec4::new(0.0, 0.0, 0.06_f32.powi(2), 0.65),
        };
        let handles = WaveBufferHandles {
            parameters: assets.add(ShaderBuffer::from(parameters)),
            source: assets.add(ShaderBuffer::from(gpu_source(source))),
            pulse: assets.add(ShaderBuffer::from(pulse)),
            row_offsets: assets.add(ShaderBuffer::from(operator.row_offsets().to_vec())),
            columns: assets.add(ShaderBuffer::from(operator.columns().to_vec())),
            stiffness: assets.add(ShaderBuffer::from(normalized)),
            vertices: assets.add(ShaderBuffer::from(vertices)),
            state: assets.add(ShaderBuffer::from(states)),
        };

        if let Some(entity) = self.readback_entity.take() {
            commands.entity(entity).despawn();
        }
        if let Some(old) = self.buffers.take() {
            for handle in old.all() {
                assets.remove(handle.id());
            }
        }
        self.generation = self.generation.wrapping_add(1).max(1);
        self.vertex_count = vertex_count;
        self.desired_steps = 0;
        self.pulse_serial = 0;
        self.stats = Arc::new(WaveGpuStats::default());
        let readback_entity = commands
            .spawn((
                Readback::buffer(handles.state.clone()),
                WaveReadbackTag {
                    generation: self.generation,
                },
            ))
            .id();
        self.readback_entity = Some(readback_entity);
        self.buffers = Some(handles);
        Ok(())
    }

    pub fn update_source(
        &self,
        assets: &mut Assets<ShaderBuffer>,
        source: SourceSettings,
    ) -> Result<(), String> {
        let handles = self
            .buffers
            .as_ref()
            .ok_or("The wave solver is not initialized")?;
        let mut buffer = assets
            .get_mut(&handles.source)
            .ok_or("The GPU source buffer is unavailable")?;
        buffer.set_data(gpu_source(source));
        Ok(())
    }

    pub fn inject_pulse(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        position: Point2,
        amplitude: f32,
        width: f32,
    ) -> Result<(), String> {
        let handles = self
            .buffers
            .as_ref()
            .ok_or("The wave solver is not initialized")?;
        let pulse = GpuPulse {
            position_width_amplitude: Vec4::new(
                position.x as f32,
                position.y as f32,
                width * width,
                amplitude,
            ),
        };
        if !pulse.position_width_amplitude.is_finite() || width <= 0.0 {
            return Err("Pulse parameters must be finite with positive width".into());
        }
        assets
            .get_mut(&handles.pulse)
            .ok_or("The GPU pulse buffer is unavailable")?
            .set_data(pulse);
        self.pulse_serial = self.pulse_serial.wrapping_add(1).max(1);
        Ok(())
    }
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
    }
}

#[derive(Resource, Default)]
pub struct WaveDisplay {
    pub generation: u64,
    pub current: Vec<f32>,
    pub previous: Vec<f32>,
    pub completed_steps: u64,
    pub readbacks: u64,
}

#[derive(Component)]
struct WaveReadbackTag {
    generation: u64,
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
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuPulse {
    position_width_amplitude: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuVertex {
    position_damping: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct GpuState {
    levels: Vec4,
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
    display.generation = tag.generation;
    display.completed_steps = states[0].levels.w.max(0.0) as u64;
    display.current.clear();
    display.previous.clear();
    display.current.reserve(states.len());
    display.previous.reserve(states.len());
    for state in states {
        display.current.push(state.levels.y);
        display.previous.push(state.levels.x);
    }
    display.readbacks = display.readbacks.saturating_add(1);
}

pub struct WaveGpuPlugin;

impl Plugin for WaveGpuPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "wave.wgsl");
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
    step: CachedComputePipelineId,
    rotate: CachedComputePipelineId,
    inject: CachedComputePipelineId,
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
                storage_buffer_read_only::<Vec<f32>>(false),
                storage_buffer_read_only::<Vec<GpuVertex>>(false),
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
    commands.insert_resource(WavePipeline {
        layout,
        step,
        rotate,
        inject,
    });
}

#[derive(Resource)]
struct WaveBindGroup {
    generation: u64,
    completed_steps: u64,
    pulse_serial: u64,
    bind_group: BindGroup,
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
        || existing
            .as_ref()
            .is_some_and(|group| group.generation == request.generation)
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
    let Some(vertices) = gpu_buffers.get(&handles.vertices) else {
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
            vertices.buffer.as_entire_buffer_binding(),
            state.buffer.as_entire_buffer_binding(),
        )),
    );
    commands.insert_resource(WaveBindGroup {
        generation: request.generation,
        completed_steps: 0,
        pulse_serial: 0,
        bind_group,
    });
}

fn compute_wave(
    mut render_context: RenderContext,
    request: Option<Res<WaveGpuRequest>>,
    group: Option<ResMut<WaveBindGroup>>,
    pipeline: Res<WavePipeline>,
    pipeline_cache: Res<PipelineCache>,
) {
    let (Some(request), Some(mut group)) = (request, group) else {
        return;
    };
    if group.generation != request.generation {
        return;
    }
    let pipelines = [pipeline.step, pipeline.rotate, pipeline.inject];
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
    request.stats.status.store(STATUS_READY, Ordering::Relaxed);
    let workgroups = request.vertex_count.div_ceil(WORKGROUP_SIZE);
    if workgroups == 0 {
        return;
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
        });
        assert_eq!(source.position_width_amplitude.x, 0.2);
        assert_eq!(source.position_width_amplitude.y, -0.3);
        assert!((source.position_width_amplitude.z - 0.0016).abs() < 1.0e-9);
        assert_eq!(source.position_width_amplitude.w, 7.0);
        assert!((source.frequency_enabled.x - 5.0 * std::f32::consts::PI).abs() < 1.0e-6);
        assert_eq!(source.frequency_enabled.y, 1.0);
    }
}
