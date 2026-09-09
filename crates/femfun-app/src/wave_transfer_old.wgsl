struct Parameters {
    time_data: vec4<f32>,
    count_data: vec4<u32>,
}

struct Source {
    position_width_amplitude: vec4<f32>,
    frequency_enabled: vec4<f32>,
}

struct VertexData {
    position_damping: vec4<f32>,
}

struct State {
    levels: vec4<f32>,
}

struct TransferEntry {
    indices: vec4<u32>,
    weights: vec4<f32>,
    mapped: vec4<f32>,
    auxiliary: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> parameters: Parameters;
@group(0) @binding(1) var<storage, read> source: Source;
@group(0) @binding(2) var<storage, read> row_offsets: array<u32>;
@group(0) @binding(3) var<storage, read> columns: array<u32>;
@group(0) @binding(4) var<storage, read> stiffness_over_mass: array<f32>;
@group(0) @binding(5) var<storage, read> vertices: array<VertexData>;
@group(0) @binding(6) var<storage, read> states: array<State>;
@group(0) @binding(7) var<storage, read_write> transfers: array<TransferEntry>;

fn source_acceleration(i: u32) -> f32 {
    let delta = vertices[i].position_damping.xy - source.position_width_amplitude.xy;
    let gaussian = exp(-0.5 * dot(delta, delta) / source.position_width_amplitude.z);
    return source.frequency_enabled.y
        * source.position_width_amplitude.w
        * gaussian
        * sin(source.frequency_enabled.x * parameters.time_data.z);
}

fn velocity(i: u32) -> f32 {
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
    let current = states[i].levels.y;
    let previous = states[i].levels.x;
    let gamma = vertices[i].position_damping.z;
    let q = source_acceleration(i) - ku;
    return ((current - previous) / dt + 0.5 * dt * q) / (1.0 + 0.5 * gamma * dt);
}

@compute @workgroup_size(128)
fn transfer(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= arrayLength(&transfers) {
        return;
    }
    var current = 0.0;
    var mapped_velocity = 0.0;
    if transfers[i].indices.w != 0u {
        for (var corner = 0u; corner < 3u; corner += 1u) {
            let source_index = transfers[i].indices[corner];
            let weight = transfers[i].weights[corner];
            current += weight * states[source_index].levels.y;
            mapped_velocity += weight * velocity(source_index);
        }
    }
    transfers[i].mapped = vec4<f32>(mapped_velocity, current, 0.0, 0.0);
    if i == 0u {
        transfers[0].auxiliary.x = parameters.time_data.z;
    }
}
