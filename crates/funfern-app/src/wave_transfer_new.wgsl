struct Parameters {
    time_data: vec4<f32>,
    count_data: vec4<u32>,
}

struct Source {
    position_width_amplitude: vec4<f32>,
    frequency_enabled: vec4<f32>,
    region: vec4<u32>,
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

@group(0) @binding(0) var<storage, read_write> parameters: Parameters;
@group(0) @binding(1) var<storage, read> source: Source;
@group(0) @binding(2) var<storage, read> row_offsets: array<u32>;
@group(0) @binding(3) var<storage, read> columns: array<u32>;
@group(0) @binding(4) var<storage, read> matrix_over_mass: array<MatrixEntry>;
@group(0) @binding(5) var<storage, read> nodes: array<NodeData>;
@group(0) @binding(6) var<storage, read_write> states: array<State>;
@group(0) @binding(7) var<storage, read> transfers: array<TransferEntry>;

fn region_match(node: vec4<u32>, region: vec4<u32>) -> f32 {
    let first = node.x == region.x && node.y == region.y;
    let second = (node.z != 0u || node.w != 0u)
        && node.z == region.x && node.w == region.y;
    return select(0.0, 1.0, first || second);
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
    let delta = nodes[i].position_damping.xy - source.position_width_amplitude.xy;
    let gaussian = exp(-0.5 * dot(delta, delta) / source.position_width_amplitude.z);
    let forcing = source.frequency_enabled.y
        * region_match(nodes[i].regions, source.region)
        * source.position_width_amplitude.w
        * gaussian
        * sin(source.frequency_enabled.x * time);
    let gamma = nodes[i].position_damping.z;
    let acceleration = forcing - ku - gamma * velocity;
    let dt = parameters.time_data.x;
    let previous = current - dt * velocity + 0.5 * dt * dt * acceleration;
    states[i].levels = vec4<f32>(previous, current, current, 0.0);
    states[i].auxiliary = vec4<f32>(
        transfers[i].mapped.z * nodes[i].position_damping.w,
        0.0,
        0.0,
        0.0,
    );
    if i == 0u {
        parameters.time_data.z = time;
        parameters.time_data.w = 0.0;
    }
}
