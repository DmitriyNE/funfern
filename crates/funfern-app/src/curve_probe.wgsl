struct Parameters {
    time_data: vec4<f32>,
    count_data: vec4<u32>,
}

struct State {
    levels: vec4<f32>,
    auxiliary: vec4<f32>,
}

struct CurveProbeStencil {
    nodes_a: vec4<u32>,
    nodes_b: vec4<u32>,
    weights_a: vec4<f32>,
    weights_b: vec4<f32>,
    gradient_x_a: vec4<f32>,
    gradient_x_b: vec4<f32>,
    gradient_y_a: vec4<f32>,
    gradient_y_b: vec4<f32>,
    material: vec4<f32>,
    // Positive normal x/y, sample stride, validity.
    normal_stride_valid: vec4<f32>,
}

struct ProbeControl {
    // Active points, ring frames, output row stride, unused.
    values: vec4<f32>,
}

struct ProbeSample {
    // Displacement, energy density, signed normal flux, physical time.
    values: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> parameters: Parameters;
@group(0) @binding(1) var<storage, read> states: array<State>;
@group(0) @binding(2) var<storage, read> stencils: array<CurveProbeStencil>;
@group(0) @binding(3) var<storage, read> control: ProbeControl;
@group(0) @binding(4) var<storage, read_write> output: array<ProbeSample>;

fn weighted(values_a: vec4<f32>, values_b: vec4<f32>, stencil: CurveProbeStencil) -> f32 {
    return dot(values_a, stencil.weights_a) + dot(values_b, stencil.weights_b);
}

@compute @workgroup_size(64)
fn sample_curve_probes(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let point = invocation.x;
    let point_count = u32(control.values.x);
    if point >= point_count {
        return;
    }
    let stencil = stencils[point];
    let stride = u32(stencil.normal_stride_valid.z);
    let completed = u32(parameters.time_data.w);
    if stride == 0u || completed % stride != 0u {
        return;
    }
    let frames = u32(control.values.y);
    let row_stride = u32(control.values.z);
    let frame = (completed / stride) % frames;
    let index = frame * row_stride + point;
    let time = parameters.time_data.z - parameters.time_data.x;
    if stencil.normal_stride_valid.w < 0.5 {
        output[index].values = vec4<f32>(bitcast<f32>(0x7fc00000u), bitcast<f32>(0x7fc00000u), bitcast<f32>(0x7fc00000u), time);
        return;
    }
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
    let energy = 0.5 * (stencil.material.x * velocity * velocity + stencil.material.y * (gradient_x * gradient_x + gradient_y * gradient_y));
    let normal_gradient = gradient_x * stencil.normal_stride_valid.x + gradient_y * stencil.normal_stride_valid.y;
    let flux = -stencil.material.y * velocity * normal_gradient;
    output[index].values = vec4<f32>(displacement, energy, flux, time);
}
