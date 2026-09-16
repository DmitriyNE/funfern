struct Parameters {
    time_data: vec4<f32>,
    count_data: vec4<u32>,
    // Reciprocal eigenvalue ceiling and strength for the grid-scale filter in
    // `wave.wgsl`. Declared everywhere so the shared buffer has one layout.
    filter_data: vec4<f32>,
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
    envelope: vec4<f32>,
}

struct ForcingWeightWord {
    data: vec4<u32>,
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
@group(0) @binding(7) var<storage, read> forcing_weights: array<ForcingWeightWord>;

fn signal_value(signal: TimeSignal, time: f32) -> f32 {
    return signal.values.x
        + signal.values.y * sin(signal.values.z * time + signal.values.w);
}

// A sine started from rest at a phase whose cosine is not zero hands the domain
// a net impulse, and in a cavity nothing ever takes it back: a constant is in
// the null space of the stiffness operator, and a radiating wall damps velocity
// rather than position. Easing every source in together suppresses that impulse
// while leaving the phases between them alone, which is what steers a phased
// array. Mirrors `funfern_core::source_envelope`.
fn source_envelope(time: f32) -> f32 {
    let ramp = forcing.envelope.x;
    if ramp <= 0.0 {
        return 1.0;
    }
    let fraction = clamp(time / ramp, 0.0, 1.0);
    return fraction * fraction * (3.0 - 2.0 * fraction);
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
    let header = forcing_weights[i].data;
    var value = 0.0;
    for (var slot = 0u; slot < header.w; slot += 1u) {
        let packed = forcing_weights[header.z + slot / 2u].data;
        let channel = select(packed.z, packed.x, slot % 2u == 0u);
        let weight_bits = select(packed.w, packed.y, slot % 2u == 0u);
        value += bitcast<f32>(weight_bits)
            * signal_value(forcing.volume[channel - 1u], time);
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
    // Both stiffness operators annihilate constants, so they are applied to the
    // differences against this node. The plain row product does not sum to zero
    // once each entry has been divided by the lumped mass and rounded to f32;
    // that residual is a permanent force on the free constant mode of a Neumann
    // cavity and grows a uniform offset without bound. The difference form is
    // exactly zero on a constant field. The CPU solver uses the same form.
    var ku = 0.0;
    var entry = row_offsets[i];
    let end = row_offsets[i + 1u];
    let displacement = states[i].levels.y;
    let memory = states[i].auxiliary.x;
    loop {
        if entry >= end {
            break;
        }
        let column = columns[entry];
        let coefficients = matrix_over_mass[entry].coefficients;
        ku += coefficients.x * (states[column].levels.y - displacement)
            + coefficients.y * (states[column].auxiliary.x - memory);
        entry += 1u;
    }
    let dt2 = parameters.time_data.y;
    let gamma = nodes[i].position_damping.z;
    let acceleration = source_envelope(parameters.time_data.z)
        * (forcing.source.position_width_enabled.w
            * bitcast<f32>(forcing_weights[i].data.x)
            * signal_value(forcing.source.signal, parameters.time_data.z)
            + volume_acceleration(i, parameters.time_data.z))
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

// Nothing in this scheme dissipates at any wavelength, and the top of the
// element's spectrum has almost no group velocity, so whatever lands there stays
// where it landed for the rest of the run. A step in the field puts a lot there
// at once - deleting a wall that held a DC offset is the sharpest event the
// editor can produce - and it shows as a speckle that neither travels nor fades.
//
// These two passes remove it. The operator supplies its own filter shape:
// `L = (K/M) / lambda_max` has eigenvalues in [0, 1], is exactly zero on a
// constant field, and scales as the square of a mode's frequency, so applying it
// twice separates the physical band from the mesh ceiling by the fourth power of
// their frequency ratio. `parameters.filter_data.x` is `1 / lambda_max` and
// `parameters.filter_data.y` is the fraction of a mode at that ceiling one
// application removes.
//
// Only the difference between the two levels is damped, and symmetrically, so
// the midpoint never moves: a static field stays exactly where it is, including
// the standing offset a region sealed by reflecting walls is entitled to. The
// dispatcher runs this pair every `GRID_SCALE_FILTER_CADENCE` steps, after
// `rotate` has committed the levels. `auxiliary.y/z` keep the step's own
// acceleration and velocity, so a probe reading them during an application sees
// the field the filter is about to correct, which is the part being removed.
// `QuadraticWaveOperator::apply_grid_scale_filter` is the same arithmetic.
@compute @workgroup_size(128)
fn filter_stage(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= parameters.count_data.x {
        return;
    }
    let difference = states[i].levels.y - states[i].levels.x;
    var sum = 0.0;
    var entry = row_offsets[i];
    let end = row_offsets[i + 1u];
    loop {
        if entry >= end {
            break;
        }
        let column = columns[entry];
        sum += matrix_over_mass[entry].coefficients.x
            * ((states[column].levels.y - states[column].levels.x) - difference);
        entry += 1u;
    }
    // The only slot of the state the solver does not otherwise use. It is
    // written here and read by the pass below, within one application.
    states[i].reconstruction.w = sum * parameters.filter_data.x;
}

@compute @workgroup_size(128)
fn filter_apply(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= parameters.count_data.x {
        return;
    }
    // A prescribed node carries its boundary condition, not a solution, so the
    // filter has nothing to say about it.
    if nodes[i].boundary.x != 0u {
        return;
    }
    let staged = states[i].reconstruction.w;
    var sum = 0.0;
    var entry = row_offsets[i];
    let end = row_offsets[i + 1u];
    loop {
        if entry >= end {
            break;
        }
        let column = columns[entry];
        sum += matrix_over_mass[entry].coefficients.x
            * (states[column].reconstruction.w - staged);
        entry += 1u;
    }
    let half = 0.5 * parameters.filter_data.y * sum * parameters.filter_data.x;
    states[i].levels.y -= half;
    states[i].levels.x += half;
}

@compute @workgroup_size(128)
fn inject(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= parameters.count_data.x {
        return;
    }
    let addition = forcing.pulse.position_width_amplitude.w
        * bitcast<f32>(forcing_weights[i].data.y);
    states[i].levels.x += addition;
    states[i].levels.y += addition;
    states[i].auxiliary.y = 0.0;
    states[i].auxiliary.z = 0.0;
    states[i].auxiliary.w += addition;
}
