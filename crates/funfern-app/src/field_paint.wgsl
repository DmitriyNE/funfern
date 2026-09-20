struct FieldUniform {
    world_to_clip: vec4<f32>,
    field: vec4<f32>,
    lines: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> field: FieldUniform;

fn linear_from_gamma_rgb(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let lower = srgb / vec3<f32>(12.92);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

fn world_clip(position: vec2<f32>) -> vec2<f32> {
    return (position - field.world_to_clip.xy) * field.world_to_clip.zw;
}

struct FieldVertexInput {
    @location(0) position: vec2<f32>,
    @location(1) value: f32,
}

struct FieldVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) value: f32,
}

@vertex
fn field_vertex(input: FieldVertexInput) -> FieldVertexOutput {
    var output: FieldVertexOutput;
    output.clip_position = vec4<f32>(world_clip(input.position), 0.0, 1.0);
    output.value = input.value * field.field.x;
    return output;
}

@fragment
fn field_fragment(input: FieldVertexOutput) -> @location(0) vec4<f32> {
    let signed_amount = tanh(input.value);
    let amount = abs(signed_amount);
    let positive = vec3<f32>(244.0, 105.0, 122.0) / 255.0;
    let negative = vec3<f32>(63.0, 144.0, 239.0) / 255.0;
    let hue = select(negative, positive, signed_amount >= 0.0);
    if field.field.y > 0.5 {
        let alpha = amount * (220.0 / 255.0);
        return vec4<f32>(linear_from_gamma_rgb(hue * alpha), alpha);
    }
    let base = vec3<f32>(16.0, 23.0, 31.0) / 255.0;
    return vec4<f32>(linear_from_gamma_rgb(mix(base, hue, amount)), 1.0);
}

struct OverlayVertexInput {
    @location(0) a: vec2<f32>,
    @location(1) b: vec2<f32>,
    @location(2) c: vec2<f32>,
    @location(3) color: vec4<f32>,
    @builtin(vertex_index) vertex_index: u32,
}

struct ColorVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn overlay_vertex(input: OverlayVertexInput) -> ColorVertexOutput {
    let positions = array(input.a, input.b, input.c);
    var output: ColorVertexOutput;
    output.clip_position = vec4<f32>(world_clip(positions[input.vertex_index]), 0.0, 1.0);
    output.color = input.color;
    return output;
}

@fragment
fn overlay_fragment(input: ColorVertexOutput) -> @location(0) vec4<f32> {
    // Color32 reaches the vertex buffer as premultiplied gamma-space sRGBA,
    // matching bevy_egui's own mesh shader.
    return vec4<f32>(linear_from_gamma_rgb(input.color.rgb), input.color.a);
}

struct LineVertexInput {
    @location(0) endpoints: vec4<f32>,
    @location(1) boundary: u32,
    @builtin(vertex_index) vertex_index: u32,
}

@vertex
fn line_vertex(input: LineVertexInput) -> ColorVertexOutput {
    let a = world_clip(input.endpoints.xy);
    let b = world_clip(input.endpoints.zw);
    let clip_per_point = field.lines.xy;
    let screen_delta = (b - a) / max(clip_per_point, vec2<f32>(1.0e-12));
    let line_length = max(length(screen_delta), 1.0e-12);
    let normal = vec2<f32>(-screen_delta.y, screen_delta.x) / line_length;
    let use_boundary = input.boundary != 0u && field.lines.w > 0.5;
    let visible = use_boundary || field.lines.z > 0.5;
    let half_width = select(0.35, 0.575, use_boundary);
    let offset = normal * clip_per_point * half_width;
    let corners = array(
        a - offset,
        b - offset,
        b + offset,
        a - offset,
        b + offset,
        a + offset,
    );
    let mesh_color = vec4<f32>(160.0, 180.0, 195.0, 110.0) / 255.0;
    let boundary_color = vec4<f32>(184.0, 201.0, 211.0, 180.0) / 255.0;
    var output: ColorVertexOutput;
    output.clip_position = vec4<f32>(corners[input.vertex_index], 0.0, 1.0);
    output.color = select(vec4<f32>(0.0), select(mesh_color, boundary_color, use_boundary), visible);
    return output;
}

@fragment
fn line_fragment(input: ColorVertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(linear_from_gamma_rgb(input.color.rgb * input.color.a), input.color.a);
}
