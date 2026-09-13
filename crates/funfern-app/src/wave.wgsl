struct Parameters {
    time_data: vec4<f32>,
    count_data: vec4<u32>,
}

struct TimeSignal {
    values: vec4<f32>,
    extra: vec4<f32>,
}

struct Source {
    position_width_enabled: vec4<f32>,
    signal: TimeSignal,
    region: vec4<u32>,
}

struct Pulse {
    position_width_amplitude: vec4<f32>,
    region: vec4<u32>,
}

struct Forcing {
    source: Source,
    pulse: Pulse,
    outer: array<TimeSignal, 4>,
    volume: array<TimeSignal, 33>,
}

struct ForcingWeights {
    point_pulse: vec2<f32>,
    channels: vec2<u32>,
    volume: vec2<f32>,
}

struct NodeData {
    position_damping: vec4<f32>,
    source_membership: vec4<u32>,
    boundary: vec4<u32>,
    neumann_weights: vec4<f32>,
    dirichlet_signal: TimeSignal,
    face_neumann_signal_a: TimeSignal,
    face_neumann_signal_b: TimeSignal,
    face_neumann_weights: vec4<f32>,
}

struct MatrixEntry {
    coefficients: vec2<f32>,
}

struct State {
    levels: vec4<f32>,
    auxiliary: vec4<f32>,
    reconstruction: vec4<f32>,
}

// The scalar wave equation does not constrain the DC integration constant of
// the complementary EM field. This critically damped inverse derivative rejects
// that null mode below roughly 0.08 Hz while remaining close to 1/(i omega) over
// the frequencies used by the playground's sources.
const RECONSTRUCTION_DECAY_RATE: f32 = 0.5;

@group(0) @binding(0) var<storage, read_write> parameters: Parameters;
@group(0) @binding(1) var<storage, read> forcing: Forcing;
@group(0) @binding(2) var<storage, read> row_offsets: array<u32>;
@group(0) @binding(3) var<storage, read> columns: array<u32>;
@group(0) @binding(4) var<storage, read> matrix_over_mass: array<MatrixEntry>;
@group(0) @binding(5) var<storage, read> nodes: array<NodeData>;
@group(0) @binding(6) var<storage, read_write> states: array<State>;
@group(0) @binding(7) var<storage, read> forcing_weights: array<ForcingWeights>;

fn signal_value(signal: TimeSignal, time: f32) -> f32 {
    return signal.values.x
        + signal.values.y * sin(signal.values.z * time + signal.values.w);
}

fn boundary_value(side: u32, time: f32) -> f32 {
    return signal_value(forcing.outer[side], time);
}

fn neumann_acceleration(i: u32, time: f32) -> f32 {
    var value = 0.0;
    for (var side = 0u; side < 4u; side += 1u) {
        value += nodes[i].neumann_weights[side] * boundary_value(side, time);
    }
    value += nodes[i].face_neumann_weights.x
        * signal_value(nodes[i].face_neumann_signal_a, time);
    value += nodes[i].face_neumann_weights.y
        * signal_value(nodes[i].face_neumann_signal_b, time);
    return value;
}

fn volume_acceleration(i: u32, time: f32) -> f32 {
    let weights = forcing_weights[i];
    var value = 0.0;
    if weights.channels.x != 0u {
        value += weights.volume.x
            * signal_value(forcing.volume[weights.channels.x - 1u], time);
    }
    if weights.channels.y != 0u {
        value += weights.volume.y
            * signal_value(forcing.volume[weights.channels.y - 1u], time);
    }
    return value;
}

@compute @workgroup_size(128)
fn advance_wave(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= parameters.count_data.x {
        return;
    }
    let dt = parameters.time_data.x;
    // Keep the reconstructed potential aligned with auxiliary.w/auxiliary.z at u^n.
    states[i].reconstruction.y = states[i].reconstruction.x
        - RECONSTRUCTION_DECAY_RATE * states[i].reconstruction.z;
    let dirichlet = nodes[i].boundary.x;
    if dirichlet != 0u {
        let previous = signal_value(nodes[i].dirichlet_signal, parameters.time_data.z - dt);
        let current = signal_value(nodes[i].dirichlet_signal, parameters.time_data.z);
        let next = signal_value(nodes[i].dirichlet_signal, parameters.time_data.z + dt);
        states[i].levels.z = next;
        states[i].auxiliary.y = (next - 2.0 * current + previous) / (dt * dt);
        states[i].auxiliary.z = (next - previous) / (2.0 * dt);
        states[i].auxiliary.w = current;
        states[i].levels.w = parameters.time_data.w + 1.0;
        return;
    }
    var ku = 0.0;
    var entry = row_offsets[i];
    let end = row_offsets[i + 1u];
    loop {
        if entry >= end {
            break;
        }
        let column = columns[entry];
        let coefficients = matrix_over_mass[entry].coefficients;
        ku += coefficients.x * states[column].levels.y
            + coefficients.y * states[column].auxiliary.x;
        entry += 1u;
    }
    let dt2 = parameters.time_data.y;
    let gamma = nodes[i].position_damping.z;
    let acceleration = forcing.source.position_width_enabled.w
        * forcing_weights[i].point_pulse.x
        * signal_value(forcing.source.signal, parameters.time_data.z)
        + volume_acceleration(i, parameters.time_data.z)
        + neumann_acceleration(i, parameters.time_data.z);
    let previous = states[i].levels.x;
    let current = states[i].levels.y;
    let next = (
        2.0 * current
        - (1.0 - 0.5 * gamma * dt) * previous
        - dt2 * ku
        + dt2 * acceleration
    ) / (1.0 + 0.5 * gamma * dt);
    states[i].levels.z = next;
    states[i].auxiliary.y = (next - 2.0 * current + previous) / dt2;
    states[i].auxiliary.z = (next - previous) / (2.0 * dt);
    states[i].auxiliary.w = current;
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
    if nodes[i].position_damping.w > 0.5 && nodes[i].boundary.x == 0u {
        states[i].auxiliary.x += 0.5 * parameters.time_data.x
            * (states[i].levels.y + states[i].levels.z);
    } else {
        states[i].auxiliary.x = 0.0;
    }
    let half_decay_step = 0.5 * RECONSTRUCTION_DECAY_RATE * parameters.time_data.x;
    let denominator = 1.0 + half_decay_step;
    let stage_a_previous = states[i].reconstruction.x;
    let stage_a_next = (
        (1.0 - half_decay_step) * stage_a_previous
        + 0.5 * parameters.time_data.x * (states[i].levels.y + states[i].levels.z)
    ) / denominator;
    let stage_b_previous = states[i].reconstruction.z;
    let stage_b_next = (
        (1.0 - half_decay_step) * stage_b_previous
        + 0.5 * parameters.time_data.x * (stage_a_previous + stage_a_next)
    ) / denominator;
    states[i].reconstruction.x = stage_a_next;
    states[i].reconstruction.z = stage_b_next;
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
    let addition = forcing.pulse.position_width_amplitude.w
        * forcing_weights[i].point_pulse.y;
    states[i].levels.x += addition;
    states[i].levels.y += addition;
    states[i].auxiliary.y = 0.0;
    states[i].auxiliary.z = 0.0;
    states[i].auxiliary.w += addition;
}
