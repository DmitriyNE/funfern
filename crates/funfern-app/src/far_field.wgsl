struct Parameters {
    time_data: vec4<f32>,
    count_data: vec4<u32>,
}

struct State {
    levels: vec4<f32>,
    auxiliary: vec4<f32>,
}

struct FarFieldStencil {
    nodes_a: vec4<u32>,
    nodes_b: vec4<u32>,
    weights_a: vec4<f32>,
    weights_b: vec4<f32>,
    gradient_x_a: vec4<f32>,
    gradient_x_b: vec4<f32>,
    gradient_y_a: vec4<f32>,
    gradient_y_b: vec4<f32>,
    position_normal: vec4<f32>,
}

struct FarFieldControl {
    // Sample stride, ring frames, contour points, directions.
    sampling: vec4<f32>,
    // Exterior speed, contour spacing, delay margin, sample interval.
    projection: vec4<f32>,
}

struct ProbeSample {
    values: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> parameters: Parameters;
@group(0) @binding(1) var<storage, read> states: array<State>;
@group(0) @binding(2) var<storage, read> stencils: array<FarFieldStencil>;
@group(0) @binding(3) var<storage, read> control: FarFieldControl;
@group(0) @binding(4) var<storage, read_write> contour_history: array<ProbeSample>;
@group(0) @binding(5) var<storage, read_write> directional_history: array<ProbeSample>;

fn weighted(values_a: vec4<f32>, values_b: vec4<f32>, stencil: FarFieldStencil) -> f32 {
    return dot(values_a, stencil.weights_a) + dot(values_b, stencil.weights_b);
}

fn ring_frame(frame: u32, age: u32, frames: u32) -> u32 {
    return (frame + frames - (age % frames)) % frames;
}

@compute @workgroup_size(64)
fn sample_contour(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let point = invocation.x;
    let point_count = u32(control.sampling.z);
    if point >= point_count {
        return;
    }
    let stride = u32(control.sampling.x);
    let completed = u32(parameters.time_data.w);
    if stride == 0u || completed % stride != 0u {
        return;
    }
    let stencil = stencils[point];
    let a = stencil.nodes_a;
    let b = stencil.nodes_b;
    let displacement_a = vec4<f32>(states[a.x].auxiliary.w, states[a.y].auxiliary.w, states[a.z].auxiliary.w, states[a.w].auxiliary.w);
    let displacement_b = vec4<f32>(states[b.x].auxiliary.w, states[b.y].auxiliary.w, states[b.z].auxiliary.w, 0.0);
    let velocity_a = vec4<f32>(states[a.x].auxiliary.z, states[a.y].auxiliary.z, states[a.z].auxiliary.z, states[a.w].auxiliary.z);
    let velocity_b = vec4<f32>(states[b.x].auxiliary.z, states[b.y].auxiliary.z, states[b.z].auxiliary.z, 0.0);
    let displacement = weighted(displacement_a, displacement_b, stencil);
    let velocity = weighted(velocity_a, velocity_b, stencil);
    let gradient_x = dot(displacement_a, stencil.gradient_x_a) + dot(displacement_b, stencil.gradient_x_b);
    let gradient_y = dot(displacement_a, stencil.gradient_y_a) + dot(displacement_b, stencil.gradient_y_b);
    let normal_gradient = gradient_x * stencil.position_normal.z + gradient_y * stencil.position_normal.w;
    let frames = u32(control.sampling.y);
    let frame = (completed / stride) % frames;
    let index = frame * point_count + point;
    let time = parameters.time_data.z - parameters.time_data.x;
    contour_history[index].values = vec4<f32>(displacement, velocity, normal_gradient, time);
}

@compute @workgroup_size(64)
fn project_directions(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let direction = invocation.x;
    let direction_count = u32(control.sampling.w);
    if direction >= direction_count {
        return;
    }
    let stride = u32(control.sampling.x);
    let completed = u32(parameters.time_data.w);
    if stride == 0u || completed % stride != 0u {
        return;
    }
    let frames = u32(control.sampling.y);
    let point_count = u32(control.sampling.z);
    let current_frame = (completed / stride) % frames;
    let angle = 6.283185307179586 * f32(direction) / f32(direction_count);
    let ray = vec2<f32>(cos(angle), sin(angle));
    let wave_speed = control.projection.x;
    let spacing = control.projection.y;
    let margin = control.projection.z;
    let sample_interval = control.projection.w;
    let far_time = parameters.time_data.z - parameters.time_data.x - margin;
    var amplitude = 0.0;
    var valid = true;
    for (var point = 0u; point < point_count; point += 1u) {
        let stencil = stencils[point];
        let position = stencil.position_normal.xy;
        let normal = stencil.position_normal.zw;
        let projection = dot(ray, position) / wave_speed;
        let age = (margin - projection) / sample_interval;
        if age < 0.0 || age >= f32(frames - 1u) {
            valid = false;
            continue;
        }
        let recent_age = u32(floor(age));
        let fraction = fract(age);
        let newer_frame = ring_frame(current_frame, recent_age, frames);
        let older_frame = ring_frame(current_frame, recent_age + 1u, frames);
        let newer = contour_history[newer_frame * point_count + point].values;
        let older = contour_history[older_frame * point_count + point].values;
        if newer.w != newer.w || older.w != older.w {
            valid = false;
            continue;
        }
        let sample = mix(newer.xyz, older.xyz, fraction);
        amplitude += spacing * (sample.z - dot(normal, ray) * sample.y / wave_speed);
    }
    let index = current_frame * direction_count + direction;
    if valid {
        directional_history[index].values = vec4<f32>(amplitude, amplitude * amplitude, far_time, 1.0);
    } else {
        let nan = bitcast<f32>(0x7fc00000u);
        directional_history[index].values = vec4<f32>(nan, nan, far_time, 0.0);
    }
}
