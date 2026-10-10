// Runtime/clock half of canonical generation handoff.
const TRANSFER_LAYOUT_VERSION: u32 = 4u;
const NO_INDEX: u32 = 0xffffffffu;
const DRIVE_TARGET_PARAMETERS: u32 = 0x80000000u;
const DRIVE_INDEX_MASK: u32 = 0x7fffffffu;
// Drive records and pulse windows as `canonical_wave.wgsl` lays them out.
const DRIVE_WORDS: u32 = 6u;
const TEMPORAL_RUNTIME_WORDS_PER_SLOT: u32 = 3u;
const TEMPORAL_RUNTIME_GATE_WORD: u32 = 6u;
const TEMPORAL_RUNTIME_WORDS_PER_MATERIAL: u32 = 10u;
const DRIVE_SLOT_WORDS: u32 = 3u;
const DRIVE_INTEGRATED: u32 = 1u;
const DRIVE_PULSE: u32 = 2u;
const PRESCRIBED_RECORD_SHIFT: u32 = 3u;
const STATUS_LAYOUT: u32 = 1u;
// The exponent bits of an f32 that is infinite or NaN. Finiteness is read
// from them rather than against a float bound: naga writes an f32 constant
// out in full digits, which Safari cannot read past 2^63 and Chrome rejects at
// f32::MAX, and a float comparison is one fast math may assume never meets a
// NaN.
const NON_FINITE_EXPONENT: u32 = 0x7f800000u;

fn finite_vec2(value: vec2<f32>) -> bool {
    return all((bitcast<vec2<u32>>(value) & vec2<u32>(NON_FINITE_EXPONENT))
        != vec2<u32>(NON_FINITE_EXPONENT));
}

struct Control {
    counts_a: vec4<u32>, counts_b: vec4<u32>, counts_c: vec4<u32>,
    table_offsets: vec4<u32>, boundary_offsets: vec4<u32>, clock_u32: vec4<u32>,
    clock_f32: vec4<f32>, clock_origin: vec4<f32>, event: vec4<u32>,
    event_result: vec4<u32>, runtime_serials: vec4<u32>,
    runtime_slots: vec4<u32>,
    accepted_accounting_a: vec4<f32>, accepted_accounting_b: vec4<f32>,
    candidate_accounting_a: vec4<f32>, candidate_accounting_b: vec4<f32>,
    evolution: vec4<f32>,
    // Gate O: accepted and candidate active gain (x); the rest is reserved.
    accepted_accounting_c: vec4<f32>,
    candidate_accounting_c: vec4<f32>,
}
struct Status {
    candidate: atomic<u32>, latch: atomic<u32>,
    injection: atomic<u32>, injection_index: atomic<u32>,
    handoff: atomic<u32>, transaction_1: atomic<u32>,
    transaction_2: atomic<u32>, transaction_3: atomic<u32>,
}
struct Node {
    mass_loss: vec4<f32>, ranges: vec4<u32>, boundary: vec4<u32>,
    prescribed: vec4<f32>, damping_support: vec4<f32>, stiffness: vec4<u32>,
}
struct TableWord { data: vec4<u32> }
struct TransferWord { data: vec4<u32> }

@group(0) @binding(0) var<storage, read_write> old_control: Control;
@group(0) @binding(1) var<storage, read_write> old_nodes: array<Node>;
@group(0) @binding(2) var<storage, read_write> old_tables: array<TableWord>;
@group(0) @binding(3) var<storage, read_write> new_control: Control;
@group(0) @binding(4) var<storage, read_write> new_status: Status;
@group(0) @binding(5) var<storage, read_write> new_nodes: array<Node>;
@group(0) @binding(6) var<storage, read_write> new_tables: array<TableWord>;
@group(0) @binding(7) var<storage, read_write> transfer: array<TransferWord>;

fn reject(reason: u32) { atomicMax(&new_status.candidate, reason); }
// Portable trigonometry: this block is the same in every shader that needs
// it, which `every_shader_carries_the_same_portable_trigonometry` holds.
// WGSL promises its own sin and cos only to 2^-11 absolute, and SwiftShader's
// are 1.9e-4 off everywhere, which carried a harmonic drive's field 2e-4 off
// the reference. These are 9e-8 off from multiply-adds alone: the argument
// loses its multiple of pi/2 in three parts, the first two short enough that
// their products with it are exact, through fma - Metal's fast math folded
// the plain differences back into one rounded pi/2 - and Cephes'
// single-precision polynomials take the rest on [-pi/4, pi/4].
const TRIG_TWO_OVER_PI: f32 = 0.63661975;
const TRIG_HALF_PI_1: f32 = 1.5703125;
const TRIG_HALF_PI_2: f32 = 4.837512969970703e-4;
const TRIG_HALF_PI_3: f32 = 7.549790126404332e-8;

fn portable_sin_cos(x: f32) -> vec2<f32> {
    let k = round(x * TRIG_TWO_OVER_PI);
    let r = fma(-k, TRIG_HALF_PI_3, fma(-k, TRIG_HALF_PI_2, fma(-k, TRIG_HALF_PI_1, x)));
    let z = r * r;
    let s = r + r * z * (-1.6666654611e-1 + z * (8.3321608736e-3 + z * -1.9515295891e-4));
    let c = 1.0 - 0.5 * z
        + z * z * (4.166664568298827e-2 + z * (-1.388731625493765e-3 + z * 2.443315711809948e-5));
    let quadrant = i32(k) & 3;
    let swap = (quadrant & 1) != 0;
    let sine = select(s, c, swap);
    let cosine = select(c, s, swap);
    return vec2<f32>(
        select(sine, -sine, (quadrant & 2) != 0),
        select(cosine, -cosine, ((quadrant + 1) & 2) != 0),
    );
}

fn portable_sin(x: f32) -> f32 { return portable_sin_cos(x).x; }

fn portable_cos(x: f32) -> f32 { return portable_sin_cos(x).y; }

// The angle `value` names, in [-pi, pi]: its multiple of 2 pi taken out as
// above.
fn reduced_phase(value: f32) -> f32 {
    let k = round(value * (0.25 * TRIG_TWO_OVER_PI));
    return fma(-k, 4.0 * TRIG_HALF_PI_3,
        fma(-k, 4.0 * TRIG_HALF_PI_2, fma(-k, 4.0 * TRIG_HALF_PI_1, value)));
}
// End of the portable trigonometry.
fn add_compensated(high: f32, low: f32, increment: f32) -> vec2<f32> {
    let sum = high + increment;
    let residual = (high - sum) + increment + low;
    let next_high = sum + residual;
    return vec2<f32>(next_high, (sum - next_high) + residual);
}
fn sinc(value: f32) -> f32 {
    if abs(value) < 0.0005 {
        let square = value * value;
        return 1.0 - square / 6.0 + square * square / 120.0;
    }
    return portable_sin(value) / value;
}
fn old_float(word: u32, lane: u32) -> f32 {
    return bitcast<f32>(old_tables[word].data[lane]);
}
fn new_float(word: u32, lane: u32) -> f32 {
    return bitcast<f32>(new_tables[word].data[lane]);
}
fn mapped_index(offset: u32, index: u32) -> u32 {
    return transfer[offset + index / 4u].data[index % 4u];
}
// The step an edit moved a carrier's authored phase by, from the section at
// `offset`; none at all when the transfer packed no section there.
fn phase_step(offset: u32, index: u32) -> f32 {
    if offset == 0u { return 0.0; }
    return bitcast<f32>(transfer[offset + index / 4u].data[index % 4u]);
}
fn write_material_runtime(
    root: u32,
    phase: vec4<f32>,
    switch_state: vec4<f32>,
    frequency: vec4<f32>,
) {
    let phase_bits = bitcast<vec4<u32>>(phase);
    let switch_bits = bitcast<vec4<u32>>(switch_state);
    let frequency_bits = bitcast<vec4<u32>>(frequency);
    for (var slot = 0u; slot < 2u; slot += 1u) {
        let base = root + TEMPORAL_RUNTIME_WORDS_PER_SLOT * slot;
        new_tables[base].data = phase_bits;
        new_tables[base + 1u].data = switch_bits;
        new_tables[base + 2u].data = frequency_bits;
    }
}
// A gate is a function of absolute time, as a pulse is: the target's own
// windows, packed against its preparation origin, move on to the handoff's.
fn start_material_gates(root: u32, absolute_elapsed: f32) {
    for (var lane = 0u; lane < 4u; lane += 1u) {
        let word = root + TEMPORAL_RUNTIME_GATE_WORD + lane;
        var window = bitcast<vec4<f32>>(new_tables[word].data);
        if window.z > 0.0 {
            window.x = pulse_start_after(window, absolute_elapsed);
            new_tables[word].data = bitcast<vec4<u32>>(window);
        }
    }
}
fn old_drive(base: u32, elapsed: f32) -> f32 {
    let parameters = vec4<f32>(
        old_float(base, 0u), old_float(base, 1u),
        old_float(base, 2u), old_float(base, 3u));
    let runtime = old_tables[base + 1u].data;
    if runtime.x != DRIVE_INTEGRATED {
        return parameters.x + parameters.y * portable_sin(parameters.w + parameters.z * elapsed);
    }
    let half_phase = 0.5 * parameters.z * elapsed;
    return bitcast<f32>(runtime.y) + parameters.x * elapsed
        + parameters.y * elapsed * sinc(half_phase)
            * portable_sin(parameters.w + half_phase);
}
// `canonical_wave.wgsl`'s `pulse_start_after`.
fn pulse_start_after(window: vec4<f32>, elapsed: f32) -> f32 {
    var start = window.x - elapsed;
    if window.y > 0.0 {
        if start < 0.0 { start -= window.y * ceil(start / window.y); }
    } else {
        start = max(start, -(2.0 * window.z + 1.0));
    }
    return start;
}
fn write_drive(base: u32, parameters: vec4<f32>, runtime: vec4<u32>, window: vec4<u32>) {
    let parameter_bits = bitcast<vec4<u32>>(parameters);
    for (var slot = 0u; slot < 2u; slot += 1u) {
        let slot_base = base + DRIVE_SLOT_WORDS * slot;
        new_tables[slot_base].data = parameter_bits;
        new_tables[slot_base + 1u].data = runtime;
        new_tables[slot_base + 2u].data = window;
    }
}
fn retain_drive(old_base: u32, new_base: u32, elapsed: f32) {
    var parameters = vec4<f32>(
        old_float(old_base, 0u), old_float(old_base, 1u),
        old_float(old_base, 2u), old_float(old_base, 3u));
    var runtime = old_tables[old_base + 1u].data;
    if runtime.x == DRIVE_INTEGRATED {
        let half_phase = 0.5 * parameters.z * elapsed;
        let next_anchor = bitcast<f32>(runtime.y) + parameters.x * elapsed
            + parameters.y * elapsed * sinc(half_phase)
                * portable_sin(parameters.w + half_phase);
        runtime.y = bitcast<u32>(next_anchor);
    }
    parameters.w = reduced_phase(parameters.w + parameters.z * elapsed);
    write_drive(new_base, parameters, runtime, old_tables[old_base + 2u].data);
}
fn replace_drive(old_base: u32, new_base: u32, old_elapsed: f32, step: f32) {
    var parameters = vec4<f32>(
        new_float(new_base, 0u), new_float(new_base, 1u),
        new_float(new_base, 2u), new_float(new_base, 3u));
    var runtime = new_tables[new_base + 1u].data;
    let old_parameters = vec4<f32>(
        old_float(old_base, 0u), old_float(old_base, 1u),
        old_float(old_base, 2u), old_float(old_base, 3u));
    // The old carrier's phase where it has got to, then the edit's step.
    parameters.w = reduced_phase(old_parameters.w + old_parameters.z * old_elapsed + step);
    if runtime.x == DRIVE_INTEGRATED {
        runtime.y = bitcast<u32>(old_drive(old_base, old_elapsed));
    }
    write_drive(new_base, parameters, runtime, new_tables[new_base + 2u].data);
}
fn start_drive(base: u32, absolute_elapsed: f32) {
    var parameters = vec4<f32>(
        new_float(base, 0u), new_float(base, 1u),
        new_float(base, 2u), new_float(base, 3u));
    let runtime = new_tables[base + 1u].data;
    // A genuinely new legacy drive starts with zero integrated rate at the
    // handoff instant; only its acceleration phase advances to that instant.
    parameters.w = reduced_phase(parameters.w + parameters.z * absolute_elapsed);
    write_drive(base, parameters, runtime, new_tables[base + 2u].data);
}
// A pulse is a function of absolute time alone, so whatever it follows, the
// new generation's own record holds, its start moved by how far the actual
// origin is from the one the host prepared against. Its carrier counts from
// the pulse's centre and does not move.
fn start_pulse(base: u32, absolute_elapsed: f32) {
    var window = bitcast<vec4<f32>>(new_tables[base + 2u].data);
    window.x = pulse_start_after(window, absolute_elapsed);
    write_drive(
        base,
        vec4<f32>(new_float(base, 0u), new_float(base, 1u), new_float(base, 2u), new_float(base, 3u)),
        new_tables[base + 1u].data,
        bitcast<vec4<u32>>(window));
}

@compute @workgroup_size(128)
fn transfer_runtime(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if transfer[0].data.x != TRANSFER_LAYOUT_VERSION { reject(STATUS_LAYOUT); return; }
    let target_nodes = transfer[1].data.y;
    let source_drives = transfer[5].data.z;
    let target_drives = transfer[5].data.w;
    let current_origin = add_compensated(
        old_control.clock_origin.x, old_control.clock_origin.y, old_control.clock_f32.y);
    let preparation_delta = (current_origin.x - new_control.clock_origin.x)
        + (current_origin.y - new_control.clock_origin.y);
    if i < target_nodes && new_nodes[i].boundary.z != 0u {
        let mapping = mapped_index(transfer[4].data.z, i);
        let record = new_nodes[i].boundary.z >> PRESCRIBED_RECORD_SHIFT;
        // A harmonic carries its carrier over from the node it maps from,
        // unless that node was pulsed, whose carrier counts from its pulse.
        var carried = mapping != NO_INDEX;
        if carried {
            carried = (old_nodes[mapping & DRIVE_INDEX_MASK].boundary.z
                >> PRESCRIBED_RECORD_SHIFT) == 0u;
        }
        if record != 0u {
            let word = new_control.table_offsets.y + DRIVE_WORDS * new_control.counts_c.z
                + record - 1u;
            var window = bitcast<vec4<f32>>(new_tables[word].data);
            window.x = pulse_start_after(window, preparation_delta);
            new_tables[word].data = bitcast<vec4<u32>>(window);
        } else if carried {
            let source = mapping & DRIVE_INDEX_MASK;
            let old_signal = old_nodes[source].prescribed;
            if (mapping & DRIVE_TARGET_PARAMETERS) != 0u {
                new_nodes[i].prescribed.w = reduced_phase(
                    old_signal.w + old_signal.z * old_control.clock_f32.y
                        + phase_step(transfer[10].data.y, i));
            } else {
                var signal = old_signal;
                signal.w = reduced_phase(signal.w + signal.z * old_control.clock_f32.y);
                new_nodes[i].prescribed = signal;
            }
        } else {
            new_nodes[i].prescribed.w = reduced_phase(
                new_nodes[i].prescribed.w + new_nodes[i].prescribed.z * preparation_delta);
        }
    }
    if i < target_drives {
        let mapping = mapped_index(transfer[4].data.w, i);
        let new_base = new_control.table_offsets.y + DRIVE_WORDS * i;
        if new_tables[new_base + 1u].data.x == DRIVE_PULSE {
            start_pulse(new_base, preparation_delta);
        } else if mapping == NO_INDEX {
            start_drive(new_base, preparation_delta);
        } else {
            let source = mapping & DRIVE_INDEX_MASK;
            let old_base = old_control.table_offsets.y + DRIVE_WORDS * source
                + DRIVE_SLOT_WORDS * (old_control.runtime_slots.x & 1u);
            if old_tables[old_base + 1u].data.x == DRIVE_PULSE {
                // A harmonic taking over from a pulse starts as a new one would.
                start_drive(new_base, preparation_delta);
            } else if (mapping & DRIVE_TARGET_PARAMETERS) != 0u {
                replace_drive(
                    old_base, new_base, old_control.clock_f32.y, phase_step(transfer[10].data.x, i));
            } else {
                retain_drive(old_base, new_base, old_control.clock_f32.y);
            }
        }
    }
    let material_header = transfer[8].data;
    let target_materials = material_header.z;
    if material_header.w != 0u && i < target_materials {
        let mapping = transfer[material_header.x + i].data;
        let new_header = new_tables[new_control.runtime_slots.z].data;
        let new_root = new_header.w + TEMPORAL_RUNTIME_WORDS_PER_MATERIAL * i;
        let target_phase = bitcast<vec4<f32>>(new_tables[new_root].data);
        var phase = vec4<f32>(0.0);
        var switch_state = bitcast<vec4<f32>>(new_tables[new_root + 1u].data);
        let frequency = bitcast<vec4<f32>>(new_tables[new_root + 2u].data);
        if mapping.x != NO_INDEX {
            let old_header = old_tables[old_control.runtime_slots.z].data;
            let old_slot = old_control.runtime_slots.y & 1u;
            let old_root = old_header.w + TEMPORAL_RUNTIME_WORDS_PER_MATERIAL * mapping.x
                + TEMPORAL_RUNTIME_WORDS_PER_SLOT * old_slot;
            let old_phase = bitcast<vec4<f32>>(old_tables[old_root].data);
            let old_switch = bitcast<vec4<f32>>(old_tables[old_root + 1u].data);
            let old_frequency = bitcast<vec4<f32>>(old_tables[old_root + 2u].data);
            for (var lane = 0u; lane < 4u; lane += 1u) {
                // The lane a drive occupies can move - a physics skin change
                // swaps the mass and stiffness rows - so the carrier is taken
                // from the lane that holds the same drive, which the host
                // matched by signature. Without a move this is `lane` itself.
                let source_lane = (mapping.z >> (2u * lane)) & 3u;
                phase[lane] = select(
                    reduced_phase(target_phase[lane] + frequency[lane] * preparation_delta),
                    reduced_phase(
                        old_phase[source_lane]
                            + old_frequency[source_lane] * old_control.clock_f32.y),
                    (mapping.y & (1u << lane)) != 0u);
            }
            switch_state = vec4<f32>(
                old_switch.x,
                old_switch.y,
                old_switch.z - old_control.clock_f32.y,
                old_switch.w);
        } else {
            for (var lane = 0u; lane < 4u; lane += 1u) {
                phase[lane] = reduced_phase(
                    target_phase[lane] + frequency[lane] * preparation_delta);
            }
            switch_state.z -= preparation_delta;
        }
        write_material_runtime(new_root, phase, switch_state, frequency);
        start_material_gates(new_root, preparation_delta);
    }
    if i != 0u { return; }
    let epoch_low = old_control.clock_u32.x + 1u;
    let epoch_high = old_control.clock_u32.y + select(0u, 1u, epoch_low == 0u);
    if epoch_low == 0u && epoch_high == 0u { reject(STATUS_LAYOUT); return; }
    new_control.clock_u32 = vec4<u32>(
        epoch_low, epoch_high, 0u, old_control.clock_u32.w);
    new_control.clock_f32.y = 0.0;
    new_control.clock_f32.z = 0.0;
    new_control.clock_origin = vec4<f32>(current_origin, current_origin);
    new_control.event.x = old_control.event.x;
    new_control.event.y = old_control.event.x;
    new_control.event.z = 0u;
    new_control.event_result = old_control.event_result;
    let requested_serials = transfer[6].data;
    new_control.runtime_serials = vec4<u32>(
        select(old_control.runtime_serials.x, requested_serials.x, requested_serials.x != 0u),
        select(old_control.runtime_serials.y, requested_serials.y, requested_serials.y != 0u),
        select(old_control.runtime_serials.z, requested_serials.z, requested_serials.z != 0u),
        select(old_control.runtime_serials.w, requested_serials.w, requested_serials.w != 0u));
    new_control.accepted_accounting_a = old_control.accepted_accounting_a;
    new_control.accepted_accounting_b = old_control.accepted_accounting_b;
    new_control.candidate_accounting_a = old_control.accepted_accounting_a;
    new_control.candidate_accounting_b = old_control.accepted_accounting_b;
    new_control.accepted_accounting_c = old_control.accepted_accounting_c;
    new_control.candidate_accounting_c = old_control.accepted_accounting_c;
    // A side carries no runtime bank when its medium does not move, and a
    // medium can start or stop moving across a handoff. So zero records on
    // either side is legitimate: with none at the source every target record is
    // written from its own authored anchors, and with none at the target there
    // is nothing to write. What still has to hold is that a side claiming
    // records is a temporal generation that has them.
    if source_drives > old_control.counts_c.z
        || (material_header.w != 0u
            && ((material_header.y != 0u && old_control.runtime_slots.w == 0u)
                || (material_header.z != 0u && new_control.runtime_slots.w == 0u)))
        || !finite_vec2(current_origin) {
        reject(STATUS_LAYOUT);
    }
}
