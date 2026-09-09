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

struct TransferEntry {
    indices_a: vec4<u32>,
    weights_a: vec4<f32>,
    indices_b: vec4<u32>,
    weights_b: vec4<f32>,
    mapped: vec4<f32>,
    auxiliary: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> parameters: Parameters;
@group(0) @binding(1) var<storage, read> forcing: Forcing;
@group(0) @binding(2) var<storage, read> row_offsets: array<u32>;
@group(0) @binding(3) var<storage, read> columns: array<u32>;
@group(0) @binding(4) var<storage, read> matrix_over_mass: array<MatrixEntry>;
@group(0) @binding(5) var<storage, read> nodes: array<NodeData>;
@group(0) @binding(6) var<storage, read_write> states: array<State>;
@group(0) @binding(7) var<storage, read_write> transfers: array<TransferEntry>;

fn region_match(node: vec4<u32>, region: vec4<u32>) -> f32 {
    let first = node.x == region.x && node.y == region.y;
    let second = (node.z != 0u || node.w != 0u)
        && node.z == region.x && node.w == region.y;
    return select(0.0, 1.0, first || second);
}

fn source_acceleration(i: u32) -> f32 {
    let delta = nodes[i].position_damping.xy - forcing.source.position_width_amplitude.xy;
    let gaussian = exp(-0.5 * dot(delta, delta) / forcing.source.position_width_amplitude.z);
    return forcing.source.frequency_enabled.y
        * region_match(nodes[i].regions, forcing.source.region)
        * forcing.source.position_width_amplitude.w
        * gaussian
        * sin(forcing.source.frequency_enabled.x * parameters.time_data.z);
}

fn compute_velocity(i: u32) -> f32 {
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
    let current = states[i].levels.y;
    let previous = states[i].levels.x;
    let gamma = nodes[i].position_damping.z;
    let q = source_acceleration(i) - ku;
    return ((current - previous) / dt + 0.5 * dt * q) / (1.0 + 0.5 * gamma * dt);
}

@compute @workgroup_size(128)
fn prepare_velocity(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= parameters.count_data.x {
        return;
    }
    states[i].levels.z = compute_velocity(i);
}

@compute @workgroup_size(128)
fn transfer(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= arrayLength(&transfers) {
        return;
    }
    var current = 0.0;
    var mapped_velocity = 0.0;
    var mapped_auxiliary = 0.0;
    for (var local = 0u; local < 4u; local += 1u) {
        let source_index = transfers[i].indices_a[local];
        let weight = transfers[i].weights_a[local];
        current += weight * states[source_index].levels.y;
        mapped_velocity += weight * states[source_index].levels.z;
        mapped_auxiliary += weight * states[source_index].auxiliary.x;
    }
    for (var local = 0u; local < 4u; local += 1u) {
        let source_index = transfers[i].indices_b[local];
        let weight = transfers[i].weights_b[local];
        current += weight * states[source_index].levels.y;
        mapped_velocity += weight * states[source_index].levels.z;
        mapped_auxiliary += weight * states[source_index].auxiliary.x;
    }
    mapped_auxiliary *= transfers[i].auxiliary.z;
    transfers[i].mapped = vec4<f32>(mapped_velocity, current, mapped_auxiliary, 0.0);
    if i == 0u {
        transfers[0].auxiliary.x = parameters.time_data.z;
    }
}
