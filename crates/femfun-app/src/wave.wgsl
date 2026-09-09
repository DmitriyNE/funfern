struct Parameters {
    time_data: vec4<f32>,
    count_data: vec4<u32>,
}

struct Source {
    position_width_amplitude: vec4<f32>,
    frequency_enabled: vec4<f32>,
}

struct Pulse {
    position_width_amplitude: vec4<f32>,
}

struct NodeData {
    position_damping: vec4<f32>,
}

struct State {
    levels: vec4<f32>,
}

@group(0) @binding(0) var<storage, read_write> parameters: Parameters;
@group(0) @binding(1) var<storage, read> source: Source;
@group(0) @binding(2) var<storage, read> pulse: Pulse;
@group(0) @binding(3) var<storage, read> row_offsets: array<u32>;
@group(0) @binding(4) var<storage, read> columns: array<u32>;
@group(0) @binding(5) var<storage, read> stiffness_over_mass: array<f32>;
@group(0) @binding(6) var<storage, read> nodes: array<NodeData>;
@group(0) @binding(7) var<storage, read_write> states: array<State>;

@compute @workgroup_size(128)
fn step(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= parameters.count_data.x {
        return;
    }
    var ku = 0.0;
    var entry = row_offsets[i];
    let end = row_offsets[i + 1u];
    loop {
        if entry >= end {
            break;
        }
        ku += stiffness_over_mass[entry] * states[columns[entry]].levels.y;
        entry += 1u;
    }
    let dt = parameters.time_data.x;
    let dt2 = parameters.time_data.y;
    let gamma = nodes[i].position_damping.z;
    let delta = nodes[i].position_damping.xy - source.position_width_amplitude.xy;
    let gaussian = exp(-0.5 * dot(delta, delta) / source.position_width_amplitude.z);
    let acceleration = source.frequency_enabled.y
        * source.position_width_amplitude.w
        * gaussian
        * sin(source.frequency_enabled.x * parameters.time_data.z);
    let previous = states[i].levels.x;
    let current = states[i].levels.y;
    states[i].levels.z = (
        2.0 * current
        - (1.0 - 0.5 * gamma * dt) * previous
        - dt2 * ku
        + dt2 * acceleration
    ) / (1.0 + 0.5 * gamma * dt);
    // The step dispatch reads a stable counter. The following rotate dispatch
    // commits this predicted value, allowing asynchronous readback to identify
    // exactly which time level it contains.
    states[i].levels.w = parameters.time_data.w + 1.0;
}

@compute @workgroup_size(128)
fn rotate(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= parameters.count_data.x {
        return;
    }
    states[i].levels.x = states[i].levels.y;
    states[i].levels.y = states[i].levels.z;
    if i == 0u {
        parameters.time_data.z += parameters.time_data.x;
        parameters.time_data.w += 1.0;
    }
}

@compute @workgroup_size(128)
fn inject(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= parameters.count_data.x {
        return;
    }
    let delta = nodes[i].position_damping.xy - pulse.position_width_amplitude.xy;
    let addition = pulse.position_width_amplitude.w
        * exp(-0.5 * dot(delta, delta) / pulse.position_width_amplitude.z);
    states[i].levels.x += addition;
    states[i].levels.y += addition;
}
