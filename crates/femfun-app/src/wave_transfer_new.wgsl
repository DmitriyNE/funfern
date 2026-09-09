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

@group(0) @binding(0) var<storage, read_write> parameters: Parameters;
@group(0) @binding(1) var<storage, read> source: Source;
@group(0) @binding(2) var<storage, read> row_offsets: array<u32>;
@group(0) @binding(3) var<storage, read> columns: array<u32>;
@group(0) @binding(4) var<storage, read> stiffness_over_mass: array<f32>;
@group(0) @binding(5) var<storage, read> vertices: array<VertexData>;
@group(0) @binding(6) var<storage, read_write> states: array<State>;
@group(0) @binding(7) var<storage, read> transfers: array<TransferEntry>;

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
        ku += stiffness_over_mass[entry] * transfers[columns[entry]].mapped.y;
        entry += 1u;
    }
    let time = transfers[0].auxiliary.x;
    let delta = vertices[i].position_damping.xy - source.position_width_amplitude.xy;
    let gaussian = exp(-0.5 * dot(delta, delta) / source.position_width_amplitude.z);
    let forcing = source.frequency_enabled.y
        * source.position_width_amplitude.w
        * gaussian
        * sin(source.frequency_enabled.x * time);
    let gamma = vertices[i].position_damping.z;
    let acceleration = forcing - ku - gamma * velocity;
    let dt = parameters.time_data.x;
    let previous = current - dt * velocity + 0.5 * dt * dt * acceleration;
    states[i].levels = vec4<f32>(previous, current, current, 0.0);
    if i == 0u {
        parameters.time_data.z = time;
        parameters.time_data.w = 0.0;
    }
}
