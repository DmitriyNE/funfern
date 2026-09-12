struct Parameters {
    time_data: vec4<f32>,
    count_data: vec4<u32>,
}

struct State {
    levels: vec4<f32>,
    auxiliary: vec4<f32>,
}

struct AreaContribution {
    nodes_a: vec4<u32>,
    nodes_b: vec4<u32>,
    field_a: vec4<f32>,
    field_b: vec4<f32>,
    mass: array<vec4<f32>, 7>,
    density: array<vec4<f32>, 7>,
    stiffness: array<vec4<f32>, 7>,
    // Unused, unused, clipped element area, validity.
    material_area: vec4<f32>,
}

struct AreaDescriptor {
    // Contribution offset and count.
    offset_count: vec4<u32>,
    // Covered area, target area, validity, unused.
    areas: vec4<f32>,
}

struct ProbeControl {
    // Sample stride, ring frames, active probes, total contributions.
    values: vec4<f32>,
}

struct ProbeSample {
    // Integral of u, integral of u², total energy, covered area.
    values: vec4<f32>,
}

struct AreaSample {
    // Mean u, RMS u, mean energy density, total energy.
    primary: vec4<f32>,
    // Covered area, coverage, physical time, validity.
    secondary: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> parameters: Parameters;
@group(0) @binding(1) var<storage, read> states: array<State>;
@group(0) @binding(2) var<storage, read> contributions: array<AreaContribution>;
@group(0) @binding(3) var<storage, read> descriptors: array<AreaDescriptor>;
@group(0) @binding(4) var<storage, read> control: ProbeControl;
@group(0) @binding(5) var<storage, read_write> scratch: array<ProbeSample>;
@group(0) @binding(6) var<storage, read_write> output: array<AreaSample>;

fn packed(values: array<vec4<f32>, 7>, index: u32) -> f32 {
    let block = values[index / 4u];
    return block[index % 4u];
}

@compute @workgroup_size(64)
fn sample_area_elements(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let element_index = invocation.x;
    let total = u32(control.values.w);
    if element_index >= total {
        return;
    }
    let contribution = contributions[element_index];
    if contribution.material_area.w < 0.5 {
        return;
    }
    let a = contribution.nodes_a;
    let b = contribution.nodes_b;
    var u: array<f32, 7>;
    var v: array<f32, 7>;
    u[0] = states[a.x].auxiliary.w;
    u[1] = states[a.y].auxiliary.w;
    u[2] = states[a.z].auxiliary.w;
    u[3] = states[a.w].auxiliary.w;
    u[4] = states[b.x].auxiliary.w;
    u[5] = states[b.y].auxiliary.w;
    u[6] = states[b.z].auxiliary.w;
    v[0] = states[a.x].auxiliary.z;
    v[1] = states[a.y].auxiliary.z;
    v[2] = states[a.z].auxiliary.z;
    v[3] = states[a.w].auxiliary.z;
    v[4] = states[b.x].auxiliary.z;
    v[5] = states[b.y].auxiliary.z;
    v[6] = states[b.z].auxiliary.z;

    let field = dot(vec4<f32>(u[0], u[1], u[2], u[3]), contribution.field_a)
        + dot(vec4<f32>(u[4], u[5], u[6], 0.0), contribution.field_b);
    var field_squared = 0.0;
    var kinetic = 0.0;
    var potential = 0.0;
    var entry = 0u;
    for (var row = 0u; row < 7u; row += 1u) {
        for (var column = row; column < 7u; column += 1u) {
            var symmetry = 2.0;
            if row == column {
                symmetry = 1.0;
            }
            let mass = packed(contribution.mass, entry);
            let stiffness = packed(contribution.stiffness, entry);
            field_squared += symmetry * mass * u[row] * u[column];
            kinetic += symmetry * packed(contribution.density, entry) * v[row] * v[column];
            potential += symmetry * stiffness * u[row] * u[column];
            entry += 1u;
        }
    }
    let energy = 0.5 * (kinetic + potential);
    scratch[element_index].values = vec4<f32>(
        field,
        field_squared,
        energy,
        contribution.material_area.z,
    );
}

@compute @workgroup_size(16)
fn reduce_area_probes(@builtin(local_invocation_id) invocation: vec3<u32>) {
    let probe = invocation.x;
    let count = u32(control.values.z);
    if probe >= count {
        return;
    }
    let descriptor = descriptors[probe];
    let stride = u32(control.values.x);
    let frames = u32(control.values.y);
    let completed = u32(parameters.time_data.w);
    let frame = (completed / stride) % frames;
    let output_index = frame * 16u + probe;
    let nan = bitcast<f32>(0x7fc00000u);
    if descriptor.offset_count.y == 0u || descriptor.areas.x <= 0.0 {
        output[output_index].primary = vec4<f32>(nan);
        output[output_index].secondary = vec4<f32>(nan, nan, nan, 0.0);
        return;
    }

    var accumulated = vec4<f32>(0.0);
    let end = descriptor.offset_count.x + descriptor.offset_count.y;
    for (var index = descriptor.offset_count.x; index < end; index += 1u) {
        accumulated += scratch[index].values;
    }
    let covered_area = descriptor.areas.x;
    let target_area = descriptor.areas.y;
    let mean = accumulated.x / covered_area;
    let rms = sqrt(max(accumulated.y / covered_area, 0.0));
    let total_energy = max(accumulated.z, 0.0);
    let mean_energy = total_energy / covered_area;
    let coverage = select(0.0, clamp(covered_area / target_area, 0.0, 1.0), target_area > 0.0);
    let time = parameters.time_data.z - parameters.time_data.x;
    output[output_index].primary = vec4<f32>(mean, rms, mean_energy, total_energy);
    output[output_index].secondary = vec4<f32>(covered_area, coverage, time, 1.0);
}
