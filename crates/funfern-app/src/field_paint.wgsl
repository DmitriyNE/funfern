struct FieldUniform {
    world_to_clip: vec4<f32>,
    color: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> field: FieldUniform;

fn linear_from_gamma_rgb(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let lower = srgb / vec3<f32>(12.92);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) value: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) value: f32,
}

@vertex
fn vertex(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = vec4<f32>(
        (input.position - field.world_to_clip.xy) * field.world_to_clip.zw,
        0.0,
        1.0,
    );
    output.value = input.value * field.color.x;
    return output;
}

@fragment
fn fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    let signed_amount = tanh(input.value);
    let amount = abs(signed_amount);
    let positive = vec3<f32>(244.0, 105.0, 122.0) / 255.0;
    let negative = vec3<f32>(63.0, 144.0, 239.0) / 255.0;
    let hue = select(negative, positive, signed_amount >= 0.0);
    if field.color.y > 0.5 {
        let alpha = amount * (220.0 / 255.0);
        return vec4<f32>(linear_from_gamma_rgb(hue * alpha), alpha);
    }
    let base = vec3<f32>(16.0, 23.0, 31.0) / 255.0;
    return vec4<f32>(linear_from_gamma_rgb(mix(base, hue, amount)), 1.0);
}
