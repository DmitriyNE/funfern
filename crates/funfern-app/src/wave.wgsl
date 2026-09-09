struct Parameters {
    time_data: vec4<f32>,
    count_data: vec4<u32>,
}

struct Source {
    position_width_amplitude: vec4<f32>,
    frequency_enabled: vec4<f32>,
    region: vec4<u32>,
}

struct Pulse {
    position_width_amplitude: vec4<f32>,
    region: vec4<u32>,
}

struct Forcing {
    source: Source,
    pulse: Pulse,
}

struct NodeData {
    position_damping: vec4<f32>,
    regions: vec4<u32>,
}

struct MatrixEntry {
    coefficients: vec2<f32>,
}

struct State {
    levels: vec4<f32>,
    auxiliary: vec4<f32>,
}

@group(0) @binding(0) var<storage, read_write> parameters: Parameters;
@group(0) @binding(1) var<storage, read> forcing: Forcing;
@group(0) @binding(2) var<storage, read> row_offsets: array<u32>;
@group(0) @binding(3) var<storage, read> columns: array<u32>;
@group(0) @binding(4) var<storage, read> matrix_over_mass: array<MatrixEntry>;
@group(0) @binding(5) var<storage, read> nodes: array<NodeData>;
@group(0) @binding(6) var<storage, read_write> states: array<State>;
@group(0) @binding(7) var<storage, read> forcing_weights: array<vec2<f32>>;

fn region_match(node: vec4<u32>, region: vec4<u32>) -> f32 {
    let first = node.x == region.x && node.y == region.y;
    let second = (node.z != 0u || node.w != 0u)
        && node.z == region.x && node.w == region.y;
    return select(0.0, 1.0, first || second);
}

@compute @workgroup_size(128)
fn advance_wave(@builtin(global_invocation_id) id: vec3<u32>) {
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
        let column = columns[entry];
        let coefficients = matrix_over_mass[entry].coefficients;
        ku += coefficients.x * states[column].levels.y
            + coefficients.y * states[column].auxiliary.x;
        entry += 1u;
    }
    let dt = parameters.time_data.x;
    let dt2 = parameters.time_data.y;
    let gamma = nodes[i].position_damping.z;
    let acceleration = forcing.source.frequency_enabled.y
        * region_match(nodes[i].regions, forcing.source.region)
        * forcing.source.position_width_amplitude.w
        * forcing_weights[i].x
        * sin(forcing.source.frequency_enabled.x * parameters.time_data.z);
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
    if nodes[i].position_damping.w > 0.5 {
        states[i].auxiliary.x += 0.5 * parameters.time_data.x
            * (states[i].levels.y + states[i].levels.z);
    } else {
        states[i].auxiliary.x = 0.0;
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
    let addition = forcing.pulse.position_width_amplitude.w
        * region_match(nodes[i].regions, forcing.pulse.region)
        * forcing_weights[i].y;
    states[i].levels.x += addition;
    states[i].levels.y += addition;
}
