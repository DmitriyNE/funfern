//! Persistent GPU paint path for the dense scalar field surface.
//!
//! Egui meshes are flattened into one transient vertex/index stream every
//! frame. At adaptive-mesh scale that copied several megabytes of unchanged
//! topology on the main thread. This callback keeps positions and indices on
//! the render device and uploads only one f32 field value per node plus a tiny
//! view/exposure uniform.

use std::{borrow::Cow, sync::Arc};

use bevy::{
    asset::{embedded_asset, load_embedded_asset},
    mesh::VertexBufferLayout,
    prelude::*,
    render::{
        RenderApp,
        render_phase::TrackedRenderPass,
        render_resource::{
            BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
            BlendState, Buffer, BufferDescriptor, BufferUsages, CachedRenderPipelineId,
            ColorTargetState, ColorWrites, FragmentState, IndexFormat, MultisampleState,
            PipelineCache, PolygonMode, PrimitiveState, RenderPipelineDescriptor, ShaderStages,
            ShaderType, SpecializedRenderPipeline, SpecializedRenderPipelines, VertexAttribute,
            VertexFormat, VertexState, VertexStepMode, binding_types::uniform_buffer,
        },
        renderer::{RenderDevice, RenderQueue},
        sync_world::RenderEntity,
    },
};
use bevy_egui::{
    egui,
    render::{EguiBevyPaintCallback, EguiBevyPaintCallbackImpl, EguiPipelineKey},
};
use bytemuck::{Pod, Zeroable, bytes_of, cast_slice};

#[derive(Clone, Debug)]
pub struct FieldPaintTopology {
    pub mesh_revision: u64,
    pub positions: Arc<[[f32; 2]]>,
    pub indices: Arc<[u32]>,
}

#[derive(Clone)]
pub struct FieldPaintCallback {
    pub topology: Arc<FieldPaintTopology>,
    pub values: Arc<[f32]>,
    pub world_center: [f32; 2],
    pub world_to_clip: [f32; 2],
    pub color_scale: f32,
    pub over_overlay: bool,
}

impl FieldPaintCallback {
    pub fn shape(self, rect: egui::Rect) -> egui::epaint::PaintCallback {
        EguiBevyPaintCallback::new_paint_callback(rect, self)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, ShaderType)]
struct FieldPaintUniform {
    world_to_clip: [f32; 4],
    color: [f32; 4],
}

#[derive(Component)]
struct FieldPaintBuffers {
    mesh_revision: u64,
    vertex_count: usize,
    index_count: u32,
    positions: Buffer,
    values: Buffer,
    indices: Buffer,
    uniform: Buffer,
    bind_group: BindGroup,
    pipeline: CachedRenderPipelineId,
}

#[derive(Resource)]
struct FieldPaintPipeline {
    shader: Handle<Shader>,
    layout: BindGroupLayoutDescriptor,
}

impl FromWorld for FieldPaintPipeline {
    fn from_world(world: &mut World) -> Self {
        let asset_server = world.resource::<AssetServer>();
        Self {
            shader: load_embedded_asset!(asset_server, "field_paint.wgsl"),
            layout: BindGroupLayoutDescriptor::new(
                "field paint layout",
                &BindGroupLayoutEntries::single(
                    ShaderStages::VERTEX_FRAGMENT,
                    uniform_buffer::<FieldPaintUniform>(false),
                ),
            ),
        }
    }
}

impl SpecializedRenderPipeline for FieldPaintPipeline {
    type Key = EguiPipelineKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some(Cow::Borrowed("field paint pipeline")),
            layout: vec![self.layout.clone()],
            immediate_size: 0,
            vertex: VertexState {
                shader: self.shader.clone(),
                shader_defs: vec![],
                entry_point: Some(Cow::Borrowed("vertex")),
                buffers: vec![
                    VertexBufferLayout {
                        array_stride: 8,
                        step_mode: VertexStepMode::Vertex,
                        attributes: vec![VertexAttribute {
                            format: VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        }],
                    },
                    VertexBufferLayout {
                        array_stride: 4,
                        step_mode: VertexStepMode::Vertex,
                        attributes: vec![VertexAttribute {
                            format: VertexFormat::Float32,
                            offset: 0,
                            shader_location: 1,
                        }],
                    },
                ],
            },
            primitive: PrimitiveState {
                topology: bevy::mesh::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: bevy::render::render_resource::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs: vec![],
                entry_point: Some(Cow::Borrowed("fragment")),
                targets: vec![Some(ColorTargetState {
                    format: key.target_format,
                    blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            zero_initialize_workgroup_memory: false,
        }
    }
}

impl EguiBevyPaintCallbackImpl for FieldPaintCallback {
    fn update(
        &self,
        _info: egui::PaintCallbackInfo,
        render_entity: RenderEntity,
        key: EguiPipelineKey,
        world: &mut World,
    ) {
        if self.values.len() != self.topology.positions.len() || self.topology.indices.is_empty() {
            return;
        }
        let pipeline_id = world.resource_scope(
            |world, mut specialized: Mut<SpecializedRenderPipelines<FieldPaintPipeline>>| {
                let pipeline = world.resource::<FieldPaintPipeline>();
                let cache = world.resource::<PipelineCache>();
                specialized.specialize(cache, pipeline, key)
            },
        );
        world
            .resource_mut::<PipelineCache>()
            .block_on_render_pipeline(pipeline_id);

        let replace = world
            .get_entity(render_entity.id())
            .ok()
            .and_then(|entity| entity.get::<FieldPaintBuffers>())
            .is_none_or(|buffers| {
                buffers.mesh_revision != self.topology.mesh_revision
                    || buffers.vertex_count != self.topology.positions.len()
                    || buffers.index_count as usize != self.topology.indices.len()
            });
        if replace {
            let device = world.resource::<RenderDevice>();
            let cache = world.resource::<PipelineCache>();
            let pipeline = world.resource::<FieldPaintPipeline>();
            let positions = device.create_buffer(&BufferDescriptor {
                label: Some("field paint positions"),
                size: (self.topology.positions.len() * 8) as u64,
                usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let values = device.create_buffer(&BufferDescriptor {
                label: Some("field paint values"),
                size: (self.values.len() * 4) as u64,
                usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let indices = device.create_buffer(&BufferDescriptor {
                label: Some("field paint indices"),
                size: (self.topology.indices.len() * 4) as u64,
                usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let uniform = device.create_buffer(&BufferDescriptor {
                label: Some("field paint uniform"),
                size: std::mem::size_of::<FieldPaintUniform>() as u64,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(
                Some("field paint bind group"),
                &cache.get_bind_group_layout(&pipeline.layout),
                &BindGroupEntries::single(uniform.as_entire_buffer_binding()),
            );
            let queue = world.resource::<RenderQueue>();
            queue.write_buffer(&positions, 0, cast_slice(self.topology.positions.as_ref()));
            queue.write_buffer(&indices, 0, cast_slice(self.topology.indices.as_ref()));
            world
                .entity_mut(render_entity.id())
                .insert(FieldPaintBuffers {
                    mesh_revision: self.topology.mesh_revision,
                    vertex_count: self.topology.positions.len(),
                    index_count: self.topology.indices.len() as u32,
                    positions,
                    values,
                    indices,
                    uniform,
                    bind_group,
                    pipeline: pipeline_id,
                });
        }

        let entity = world.entity(render_entity.id());
        let Some(buffers) = entity.get::<FieldPaintBuffers>() else {
            return;
        };
        let uniform = FieldPaintUniform {
            world_to_clip: [
                self.world_center[0],
                self.world_center[1],
                self.world_to_clip[0],
                self.world_to_clip[1],
            ],
            color: [self.color_scale, self.over_overlay as u8 as f32, 0.0, 0.0],
        };
        let queue = world.resource::<RenderQueue>();
        queue.write_buffer(&buffers.values, 0, cast_slice(self.values.as_ref()));
        queue.write_buffer(&buffers.uniform, 0, bytes_of(&uniform));
    }

    fn render<'pass>(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut TrackedRenderPass<'pass>,
        render_entity: RenderEntity,
        _key: EguiPipelineKey,
        world: &'pass World,
    ) {
        let Some(buffers) = world
            .get_entity(render_entity.id())
            .ok()
            .and_then(|entity| entity.get::<FieldPaintBuffers>())
        else {
            return;
        };
        let Some(pipeline) = world
            .resource::<PipelineCache>()
            .get_render_pipeline(buffers.pipeline)
        else {
            return;
        };
        render_pass.set_render_pipeline(pipeline);
        render_pass.set_bind_group(0, &buffers.bind_group, &[]);
        render_pass.set_vertex_buffer(0, buffers.positions.slice(..));
        render_pass.set_vertex_buffer(1, buffers.values.slice(..));
        render_pass.set_index_buffer(buffers.indices.slice(..), IndexFormat::Uint32);
        render_pass.draw_indexed(0..buffers.index_count, 0, 0..1);
    }
}

pub struct FieldPaintPlugin;

impl Plugin for FieldPaintPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "field_paint.wgsl");
        app.get_sub_app_mut(RenderApp)
            .expect("render app")
            .insert_resource(SpecializedRenderPipelines::<FieldPaintPipeline>::default())
            .init_resource::<FieldPaintPipeline>();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shader_parses() {
        naga::front::wgsl::parse_str(include_str!("field_paint.wgsl")).unwrap();
    }
}
