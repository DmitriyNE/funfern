struct Parameters {
    time_data: vec4<f32>,
    count_data: vec4<u32>,
}

struct State {
    levels: vec4<f32>,
    auxiliary: vec4<f32>,
    integral: vec4<f32>,
}

struct ProbeStencil {
    nodes_a: vec4<u32>,
    nodes_b: vec4<u32>,
    weights_a: vec4<f32>,
    weights_b: vec4<f32>,
    gradient_x_a: vec4<f32>,
    gradient_x_b: vec4<f32>,
    gradient_y_a: vec4<f32>,
    gradient_y_b: vec4<f32>,
    material: vec4<f32>,
}

struct ProbeControl {
    // Sample stride, ring frames, active slots, EM mode.
    values: vec4<f32>,
}

struct ProbeSample {
    // Primary component, primary rate, energy density, physical time.
    primary: vec4<f32>,
    // Transverse-field magnitude, Poynting magnitude, unused, unused.
    secondary: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> parameters: Parameters;
@group(0) @binding(1) var<storage, read> states: array<State>;
@group(0) @binding(2) var<storage, read> stencils: array<ProbeStencil>;
@group(0) @binding(3) var<storage, read> control: ProbeControl;
@group(0) @binding(4) var<storage, read_write> output: array<ProbeSample>;

fn weighted(values_a: vec4<f32>, values_b: vec4<f32>, stencil: ProbeStencil) -> f32 {
    return dot(values_a, stencil.weights_a) + dot(values_b, stencil.weights_b);
}

@compute @workgroup_size(16)
fn sample_probes(@builtin(local_invocation_id) invocation: vec3<u32>) {
    let probe = invocation.x;
    let count = u32(control.values.z);
    if probe >= count || stencils[probe].material.z < 0.5 {
        return;
    }
    let stencil = stencils[probe];
    let a = stencil.nodes_a;
    let b = stencil.nodes_b;
    let displacement_a = vec4<f32>(
        states[a.x].auxiliary.w,
        states[a.y].auxiliary.w,
        states[a.z].auxiliary.w,
        states[a.w].auxiliary.w,
    );
    let displacement_b = vec4<f32>(
        states[b.x].auxiliary.w,
        states[b.y].auxiliary.w,
        states[b.z].auxiliary.w,
        0.0,
    );
    let velocity_a = vec4<f32>(
        states[a.x].auxiliary.z,
        states[a.y].auxiliary.z,
        states[a.z].auxiliary.z,
        states[a.w].auxiliary.z,
    );
    let velocity_b = vec4<f32>(
        states[b.x].auxiliary.z,
        states[b.y].auxiliary.z,
        states[b.z].auxiliary.z,
        0.0,
    );
    let integral_a = vec4<f32>(
        states[a.x].integral.y,
        states[a.y].integral.y,
        states[a.z].integral.y,
        states[a.w].integral.y,
    );
    let integral_b = vec4<f32>(
        states[b.x].integral.y,
        states[b.y].integral.y,
        states[b.z].integral.y,
        0.0,
    );
    let displacement = weighted(displacement_a, displacement_b, stencil);
    let velocity = weighted(velocity_a, velocity_b, stencil);
    let gradient_x = dot(displacement_a, stencil.gradient_x_a)
        + dot(displacement_b, stencil.gradient_x_b);
    let gradient_y = dot(displacement_a, stencil.gradient_y_a)
        + dot(displacement_b, stencil.gradient_y_b);
    let integral_gradient_x = dot(integral_a, stencil.gradient_x_a)
        + dot(integral_b, stencil.gradient_x_b);
    let integral_gradient_y = dot(integral_a, stencil.gradient_y_a)
        + dot(integral_b, stencil.gradient_y_b);
    let electromagnetic = control.values.w > 0.5;
    let primary_energy = select(
        stencil.material.x * velocity * velocity,
        stencil.material.x * displacement * displacement,
        electromagnetic,
    );
    let gradient_energy = select(
        stencil.material.y * (gradient_x * gradient_x + gradient_y * gradient_y),
        stencil.material.y * (integral_gradient_x * integral_gradient_x
            + integral_gradient_y * integral_gradient_y),
        electromagnetic,
    );
    let energy = 0.5 * (primary_energy + gradient_energy);
    let transverse = select(
        0.0,
        stencil.material.y * length(vec2<f32>(integral_gradient_x, integral_gradient_y)),
        electromagnetic,
    );
    let poynting = select(0.0, abs(displacement) * transverse, electromagnetic);
    let stride = u32(control.values.x);
    let frames = u32(control.values.y);
    let completed = u32(parameters.time_data.w);
    let frame = (completed / stride) % frames;
    let index = frame * 16u + probe;
    // The transferred solver clock is already continuous across buffer generations.
    // Auxiliary displacement and velocity describe the preceding centered level.
    let time = parameters.time_data.z - parameters.time_data.x;
    output[index].primary = vec4<f32>(displacement, velocity, energy, time);
    output[index].secondary = vec4<f32>(transverse, poynting, 0.0, 0.0);
}
