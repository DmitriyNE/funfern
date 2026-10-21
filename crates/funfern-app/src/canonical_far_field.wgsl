struct Control {
    counts_a: vec4<u32>, counts_b: vec4<u32>, counts_c: vec4<u32>,
    table_offsets: vec4<u32>, boundary_offsets: vec4<u32>, clock_u32: vec4<u32>,
    clock_f32: vec4<f32>, clock_origin: vec4<f32>, event: vec4<u32>,
    event_result: vec4<u32>, runtime_serials: vec4<u32>, runtime_slots: vec4<u32>,
    accepted_accounting_a: vec4<f32>, accepted_accounting_b: vec4<f32>,
    candidate_accounting_a: vec4<f32>, candidate_accounting_b: vec4<f32>,
    evolution: vec4<f32>,
    // Gate O: accepted and candidate active gain (x); the rest is reserved.
    accepted_accounting_c: vec4<f32>,
    candidate_accounting_c: vec4<f32>,
}
struct StateWord { values: vec4<f32> }
struct Node {
    mass_loss: vec4<f32>, ranges: vec4<u32>, boundary: vec4<u32>,
    prescribed: vec4<f32>, damping_support: vec4<f32>, stiffness: vec4<u32>,
}
struct FarFieldStencil {
    nodes_a: vec4<u32>, nodes_b: vec4<u32>,
    weights_a: vec4<f32>, weights_b: vec4<f32>,
    gradient_x_a: vec4<f32>, gradient_x_b: vec4<f32>,
    gradient_y_a: vec4<f32>, gradient_y_b: vec4<f32>,
    position_normal: vec4<f32>,
}
struct FarFieldControl { sampling: vec4<f32>, projection: vec4<f32> }
struct ProbeSample { values: vec4<f32> }

@group(0) @binding(0) var<storage, read> control: Control;
@group(0) @binding(1) var<storage, read> state: array<StateWord>;
@group(0) @binding(2) var<storage, read> nodes: array<Node>;
@group(0) @binding(3) var<storage, read> stencils: array<FarFieldStencil>;
@group(0) @binding(4) var<storage, read> far_control: FarFieldControl;
@group(0) @binding(5) var<storage, read_write> contour_history: array<ProbeSample>;
@group(0) @binding(6) var<storage, read_write> directional_history: array<ProbeSample>;

fn accepted_q(node: u32) -> f32 {
    return select(state[node].values.x, state[node].values.y, (control.event.z & 1u) != 0u);
}
fn previous_q(node: u32) -> f32 {
    return select(state[node].values.y, state[node].values.x, (control.event.z & 1u) != 0u);
}
// The ring's layout in `contour_history`: a frame's samples, one per contour
// point, in its slot's block; after every block, one header per slot holding
// the frame's time as a stamp; after the headers, the ring's own state: the
// slot the next frame goes to, how many frames it holds, and whether this
// pass records one. The counts stay under 512, which f32 holds exactly, so no
// bits are reinterpreted that a device flushing subnormals could lose.
fn header_index(slot: u32) -> u32 {
    return u32(far_control.sampling.y) * u32(far_control.sampling.z) + slot;
}
fn ring_state_index() -> u32 {
    return u32(far_control.sampling.y) * (u32(far_control.sampling.z) + 1u);
}
fn next_slot() -> u32 { return u32(contour_history[ring_state_index()].values.x); }
fn frames_held() -> u32 { return u32(contour_history[ring_state_index()].values.y); }
fn recording() -> bool { return contour_history[ring_state_index()].values.z > 0.5; }

// The time a pass stands at, as the probe stamps carry it: the epoch origin's
// coarse part, and its fine part with the epoch-local time, centred on the
// step. A frame is centred on the step it is taken over: the field and its
// normal gradient the mean of the step's two lanes, the rate their difference,
// all three half a step back, where the backward difference is second order. A
// frame stamped at the step's end paired the gradient with a rate half a step
// older, and the contour's far side stopped cancelling an outgoing wave by
// about ωΔt/2 of it: a 3 Hz source's pattern moved by 10% at the default step.
fn centred_stamp() -> vec2<f32> {
    return vec2<f32>(
        control.clock_origin.x,
        control.clock_origin.y + (control.clock_f32.y - 0.5 * control.clock_f32.x));
}

// `a − b` for two stamps, part by part. Two coarse parts are one epoch's
// origin, whose difference is zero, or two origins minutes apart, whose
// difference f32 holds exactly; the fine parts stay within an epoch's 256 s.
// Summed into one f32 first, as frames were stamped until 2026-10-18, the
// difference kept f32's spacing at the run's absolute time: 2e-3 s after ten
// hours, and a 1/60 s bucket's worth after 36.
// The coarse difference passes through a product with a 1 the compiler cannot
// see, the square of the solver's orientation, so fast math cannot sum the
// parts first.
fn stamp_difference(a: vec2<f32>, b: vec2<f32>) -> f32 {
    let one = control.evolution.y * control.evolution.y;
    let coarse = select(a.x - b.x, 0.0, bitcast<u32>(a.x) == bitcast<u32>(b.x));
    return coarse * one + (a.y - b.y);
}

// A pass, every quarter period or so (`far_field_sample_stride`), records a
// frame once its centred stamp stands a period past the newest frame's, into
// the slot after it: the ring is keyed to its own count, and the gap to its
// own last stamp, so a new mesh or a new step keeps writing after the frames
// it inherits, a period on, and nothing reads the absolute time. One thread
// decides, before any thread of the passes below reads the decision.
@compute @workgroup_size(1)
fn decide_frame() {
    let index = ring_state_index();
    let frames = u32(far_control.sampling.y);
    var due = frames_held() == 0u;
    if !due {
        let last = contour_history[header_index((next_slot() + frames - 1u) % frames)].values.xy;
        due = stamp_difference(centred_stamp(), last) >= far_control.sampling.x;
    }
    var ring = contour_history[index].values;
    ring.z = select(0.0, 1.0, due);
    contour_history[index].values = ring;
}

@compute @workgroup_size(64)
fn sample_contour(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let point = invocation.x;
    let point_count = u32(far_control.sampling.z);
    if point >= point_count || !recording() { return; }
    let slot = next_slot();
    let index = slot * point_count + point;
    let stencil = stencils[point];
    let a = stencil.nodes_a;
    let b = stencil.nodes_b;
    let current_a = vec4<f32>(
        accepted_q(a.x) * nodes[a.x].mass_loss.y,
        accepted_q(a.y) * nodes[a.y].mass_loss.y,
        accepted_q(a.z) * nodes[a.z].mass_loss.y,
        accepted_q(a.w) * nodes[a.w].mass_loss.y);
    let current_b = vec4<f32>(
        accepted_q(b.x) * nodes[b.x].mass_loss.y,
        accepted_q(b.y) * nodes[b.y].mass_loss.y,
        accepted_q(b.z) * nodes[b.z].mass_loss.y, 0.0);
    let previous_a = vec4<f32>(
        previous_q(a.x) * nodes[a.x].mass_loss.y,
        previous_q(a.y) * nodes[a.y].mass_loss.y,
        previous_q(a.z) * nodes[a.z].mass_loss.y,
        previous_q(a.w) * nodes[a.w].mass_loss.y);
    let previous_b = vec4<f32>(
        previous_q(b.x) * nodes[b.x].mass_loss.y,
        previous_q(b.y) * nodes[b.y].mass_loss.y,
        previous_q(b.z) * nodes[b.z].mass_loss.y, 0.0);
    let primary = dot(current_a, stencil.weights_a) + dot(current_b, stencil.weights_b);
    let previous = dot(previous_a, stencil.weights_a) + dot(previous_b, stencil.weights_b);
    let rate = (primary - previous) / control.clock_f32.x;
    let current_gradient = vec2<f32>(
        dot(current_a, stencil.gradient_x_a) + dot(current_b, stencil.gradient_x_b),
        dot(current_a, stencil.gradient_y_a) + dot(current_b, stencil.gradient_y_b));
    let previous_gradient = vec2<f32>(
        dot(previous_a, stencil.gradient_x_a) + dot(previous_b, stencil.gradient_x_b),
        dot(previous_a, stencil.gradient_y_a) + dot(previous_b, stencil.gradient_y_b));
    let gradient = 0.5 * (current_gradient + previous_gradient);
    let normal_gradient = dot(gradient, stencil.position_normal.zw);
    contour_history[index].values =
        vec4<f32>(0.5 * (primary + previous), rate, normal_gradient, 0.0);
    if point == 0u {
        contour_history[header_index(slot)].values = vec4<f32>(centred_stamp(), 0.0, 0.0);
    }
}

// Portable trigonometry: this block is the same in every shader that needs
// it, which `every_shader_carries_the_same_portable_trigonometry` holds.
// WGSL promises its own sin and cos only to 2^-11 absolute, and SwiftShader's
// are 1.9e-4 off everywhere, which carried a harmonic drive's field 2e-4 off
// the reference. These are 9e-8 off from multiply-adds alone: the argument
// loses its multiple of pi/2 in three parts, the first two short enough that
// their products with it are exact, through fma - Metal's fast math folded
// the plain differences back into one rounded pi/2 - and Cephes'
// single-precision polynomials take the rest on [-pi/4, pi/4].
//
// They give up the GPU's special-function units on purpose. A builtin is one
// or two of their instructions and this about twenty multiply-adds, but the
// step is bound by memory, not arithmetic: on the M1 Max the driven step costs
// 0.3% more for it and the sine-Gordon step, two calls a node a stage, 1.9%.
// Calls that repeat across nodes - a pump's carrier is its material's - are
// still made in place rather than once a stage, since handing the values on
// through memory would add to the traffic the step is bound by. The same
// arithmetic everywhere is the point: Metal and SwiftShader read the same
// errors at the same arguments.
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

// A frame's stamp by its age, 0 the one this pass wrote.
fn aged_slot(newest: u32, age: u32, frames: u32) -> u32 {
    return (newest + frames - age) % frames;
}

@compute @workgroup_size(64)
fn project_directions(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let direction = invocation.x;
    let direction_count = u32(far_control.sampling.w);
    if direction >= direction_count || !recording() { return; }
    let frames = u32(far_control.sampling.y);
    let point_count = u32(far_control.sampling.z);
    let newest = next_slot();
    let held = min(frames_held() + 1u, frames);
    let now = contour_history[header_index(newest)].values.xy;
    let wave_speed = far_control.projection.x;
    let spacing = far_control.projection.y;
    let margin = far_control.projection.z;
    let angle = 6.283185307179586 * f32(direction) / f32(direction_count);
    let ray = vec2<f32>(portable_cos(angle), portable_sin(angle));
    var amplitude = 0.0;
    var valid = true;
    for (var point = 0u; point < point_count; point += 1u) {
        let stencil = stencils[point];
        let position = stencil.position_normal.xy;
        let normal = stencil.position_normal.zw;
        // The retarded time, `delay` before the pass's own, which every frame's
        // time is read against.
        let retarded = dot(ray, position) / wave_speed - margin;
        let oldest_age = held - 1u;
        let oldest = contour_history[header_index(aged_slot(newest, oldest_age, frames))].values.xy;
        if stamp_difference(oldest, now) > retarded {
            valid = false;
            continue;
        }
        // The youngest frame at or before the retarded time, by bisection: a
        // frame's time falls with its age.
        var low = 0u;
        var high = oldest_age;
        loop {
            if low >= high { break; }
            let middle = (low + high) / 2u;
            let stamp = contour_history[header_index(aged_slot(newest, middle, frames))].values.xy;
            if stamp_difference(stamp, now) <= retarded {
                high = middle;
            } else {
                low = middle + 1u;
            }
        }
        let older = aged_slot(newest, low, frames);
        let newer = aged_slot(newest, max(low, 1u) - 1u, frames);
        let older_time = stamp_difference(contour_history[header_index(older)].values.xy, now);
        let newer_time = stamp_difference(contour_history[header_index(newer)].values.xy, now);
        let span = newer_time - older_time;
        var fraction = 0.0;
        if span > 0.0 { fraction = clamp((newer_time - retarded) / span, 0.0, 1.0); }
        let sample = mix(
            contour_history[newer * point_count + point].values.xyz,
            contour_history[older * point_count + point].values.xyz,
            fraction);
        amplitude += spacing * (sample.z - dot(normal, ray) * sample.y / wave_speed);
    }
    // The far time is the pass's own less the margin, carried as a stamp.
    let far = vec2<f32>(now.x, now.y - margin);
    let slot = newest * direction_count + direction;
    directional_history[slot].values = vec4<f32>(amplitude, far, select(0.0, 1.0, valid));
}

// A handed-over generation starts at its snapshot's step, behind the frames
// the source recorded while the handoff waited for admission, which are of a
// trajectory the target discards and stand newest in the ring: the target
// recorded nothing until it passed them, and its projections read across
// them. Before its first pass the ring gives up every frame stamped after
// the target's clock, newest first, and their directions read invalid (PR-5
// of the 2026-10-20 review). Dispatched only at the snapshot's step, where
// the clock is the snapshot's; again there, it finds nothing to give up.
@compute @workgroup_size(1)
fn rewind_frames() {
    let frames = u32(far_control.sampling.y);
    let directions = u32(far_control.sampling.w);
    let now = vec2<f32>(control.clock_origin.x, control.clock_origin.y + control.clock_f32.y);
    var next = next_slot();
    var held = frames_held();
    loop {
        if held == 0u { break; }
        let newest = (next + frames - 1u) % frames;
        if stamp_difference(contour_history[header_index(newest)].values.xy, now) <= 0.0 {
            break;
        }
        for (var direction = 0u; direction < directions; direction += 1u) {
            directional_history[newest * directions + direction].values.w = 0.0;
        }
        next = newest;
        held -= 1u;
    }
    contour_history[ring_state_index()].values = vec4<f32>(f32(next), f32(held), 0.0, 0.0);
}

// After the projection: a recorded frame moves the ring on a slot.
@compute @workgroup_size(1)
fn advance_frame() {
    if !recording() { return; }
    let index = ring_state_index();
    let frames = u32(far_control.sampling.y);
    contour_history[index].values = vec4<f32>(
        f32((next_slot() + 1u) % frames),
        f32(min(frames_held() + 1u, frames)),
        0.0,
        0.0);
}
