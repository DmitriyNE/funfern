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

const RECONSTRUCTION_DECAY_RATE: f32 = 0.5;

struct TransferEntry {
    indices_a: vec4<u32>,
    weights_a: vec4<f32>,
    indices_b: vec4<u32>,
    weights_b: vec4<f32>,
    mapped: vec4<f32>,
    auxiliary: vec4<f32>,
}

@group(0) @binding(0) var<storage, read_write> parameters: Parameters;
@group(0) @binding(1) var<storage, read> forcing: Forcing;
@group(0) @binding(2) var<storage, read> row_offsets: array<u32>;
@group(0) @binding(3) var<storage, read> columns: array<u32>;
@group(0) @binding(4) var<storage, read> matrix_over_mass: array<MatrixEntry>;
@group(0) @binding(5) var<storage, read> nodes: array<NodeData>;
@group(0) @binding(6) var<storage, read_write> states: array<State>;
@group(0) @binding(7) var<storage, read_write> transfers: array<TransferEntry>;

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

@compute @workgroup_size(128)
fn prepare_boundary(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= parameters.count_data.x {
        return;
    }
    let dirichlet = nodes[i].boundary.x;
    if dirichlet != 0u {
        let time = transfers[0].auxiliary.x;
        let dt = parameters.time_data.x;
        transfers[i].mapped.x = (signal_value(nodes[i].dirichlet_signal, time + dt)
            - signal_value(nodes[i].dirichlet_signal, time - dt)) / (2.0 * dt);
        transfers[i].mapped.y = signal_value(nodes[i].dirichlet_signal, time);
        transfers[i].mapped.z = 0.0;
    }
}

@compute @workgroup_size(128)
fn transfer(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= parameters.count_data.x {
        return;
    }
    let current = transfers[i].mapped.y;
    let velocity = transfers[i].mapped.x;
    var ku = 0.0;
    var entry = row_offsets[i];
    let end = row_offsets[i + 1u];
    loop {
        if entry >= end {
            break;
        }
        let column = columns[entry];
        let coefficients = matrix_over_mass[entry].coefficients;
        ku += coefficients.x * transfers[column].mapped.y
            + coefficients.y * transfers[column].mapped.z;
        entry += 1u;
    }
    let time = transfers[0].auxiliary.x;
    let delta = nodes[i].position_damping.xy - forcing.source.position_width_enabled.xy;
    let gaussian = exp(-0.5 * dot(delta, delta) / forcing.source.position_width_enabled.z);
    let source_forcing = forcing.source.position_width_enabled.w
        * select(0.0, 1.0, nodes[i].source_membership.x != 0u)
        * gaussian
        * signal_value(forcing.source.signal, time);
    let gamma = nodes[i].position_damping.z;
    let acceleration = source_forcing + neumann_acceleration(i, time) - ku - gamma * velocity;
    let dt = parameters.time_data.x;
    let dirichlet = nodes[i].boundary.x;
    var aligned_acceleration = acceleration;
    var aligned_velocity = velocity;
    var aligned_current = current;
    if dirichlet != 0u {
        let prescribed = signal_value(nodes[i].dirichlet_signal, time);
        let previous = signal_value(nodes[i].dirichlet_signal, time - dt);
        let next = signal_value(nodes[i].dirichlet_signal, time + dt);
        states[i].levels = vec4<f32>(previous, prescribed, prescribed, 0.0);
        aligned_acceleration = (next - 2.0 * prescribed + previous) / (dt * dt);
        aligned_velocity = (next - previous) / (2.0 * dt);
        aligned_current = prescribed;
    } else {
        let previous = current - dt * velocity + 0.5 * dt * dt * acceleration;
        states[i].levels = vec4<f32>(previous, current, current, 0.0);
    }
    states[i].auxiliary = vec4<f32>(
        transfers[i].mapped.z * nodes[i].position_damping.w,
        aligned_acceleration,
        aligned_velocity,
        aligned_current,
    );
    let reconstruction_a = transfers[i].mapped.w;
    let reconstruction_b = transfers[i].auxiliary.y;
    states[i].reconstruction = vec4<f32>(
        reconstruction_a,
        reconstruction_a - RECONSTRUCTION_DECAY_RATE * reconstruction_b,
        reconstruction_b,
        0.0,
    );
    if i == 0u {
        parameters.time_data.z = time;
        parameters.time_data.w = 0.0;
    }
}
