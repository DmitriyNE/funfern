//! IGA feasibility spike 5: edits without a mesher, on the deformed box
//! with the fourth-order integrator. A control point of the geometry map
//! moves mid-run and the state is carried to the new patch; the solution
//! basis is refined mid-run and the state is carried to the fine patch.
//! Each is measured against a run on the final configuration from the
//! start, and spike 6's cost question is answered by the deformed box's
//! throughput against the flat one. See
//! `docs/spikes/funfern-iga-feasibility-spike.md`.
//!
//! Flags: `--side 64 --geometry-spans 16`.

use std::time::Instant;

use funfern_core::{GeometryMap, Integrator, MassTreatment, PatchStepper, Point2, SplinePatch};

const SIGMA: f64 = 0.2;

fn bulged(geometry_spans: usize) -> GeometryMap {
    let mut map = GeometryMap::flat_surface(
        Point2::new(-1.0, -1.0),
        Point2::new(2.0, 2.0),
        [geometry_spans, geometry_spans],
    );
    if let GeometryMap::Surface {
        basis_x,
        basis_y,
        controls,
    } = &mut map
    {
        let nx = basis_x.functions();
        let ny = basis_y.functions();
        for ix in 0..nx {
            let r = ix as f64 / (nx - 1) as f64;
            for iy in 0..ny {
                let s = iy as f64 / (ny - 1) as f64;
                controls[ix * ny + iy].x += 0.3 * (std::f64::consts::PI * s).sin() * r * r;
            }
        }
    }
    map
}

fn patch(side: usize, geometry_spans: usize) -> SplinePatch {
    let spans = side - 2;
    SplinePatch::curved(
        2,
        bulged(geometry_spans),
        Point2::new(2.0, 2.0),
        [spans, spans],
        2,
        true,
    )
    .unwrap()
}

/// A radial Gaussian pulse at rest, centred at `center`.
fn pulse(patch: &SplinePatch, center: Point2) -> Vec<f64> {
    patch.project(|p| (-((p - center).norm() / SIGMA).powi(2)).exp())
}

fn relative_difference(patch: &SplinePatch, a: &[f64], b: &[f64]) -> f64 {
    let difference: Vec<f64> = a.iter().zip(b).map(|(a, b)| a - b).collect();
    (patch.inner_product(&difference, &difference) / patch.inner_product(b, b)).sqrt()
}

/// The energy the state holds as the quadrature sees it: `½ uᵀ M u` for the
/// field and `½ Σ w κ |b|²` for the flux at the same instant. The same two
/// functions read on another patch differ by that patch's quadrature only.
fn physical_energy(patch: &SplinePatch, u: &[f64], b: &[Point2]) -> f64 {
    let field = 0.5 * patch.inner_product(u, u);
    let flux: f64 = b
        .iter()
        .zip(patch.sample_weights())
        .zip(patch.sample_stiffness())
        .map(|((b, w), k)| 0.5 * w * k * b.dot(*b))
        .sum();
    field + flux
}

fn step_to(stepper: &mut PatchStepper, time: f64) {
    let steps = (time / stepper.time_step()).round() as u64;
    while stepper.steps() < steps {
        stepper.step();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str, default: usize| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map_or(default, |v| v.parse().unwrap())
    };
    let side = value("--side", 64);
    let geometry_spans = value("--geometry-spans", 16);
    let treatment = MassTreatment::Lumped { sweeps: 2 };
    let integrator = Integrator::FourthOrder;
    println!(
        "Deformed box (right side bulged by 0.3, bicubic net on {geometry_spans} × {geometry_spans} spans), quadratic solution basis on {} × {} spans, Gauss 2 × 2, two sweeps, unclamped, fourth-order integrator at 0.8 of its step.\n",
        side - 2,
        side - 2
    );

    // --- Control-point edits ---
    let base = patch(side, geometry_spans);
    let determinants = base.sample_determinants();
    let (det_min, det_max) = determinants
        .iter()
        .fold((f64::INFINITY, 0.0_f64), |(lo, hi), d| {
            (lo.min(*d), hi.max(*d))
        });
    let stable_step = |p: &SplinePatch| {
        let (lambda, _) = p.largest_eigenvalue(treatment, 1500);
        0.8 * integrator.stability_factor() * 2.0 / lambda.sqrt()
    };
    let dt = stable_step(&base);
    println!(
        "det J {:.3} to {:.3}; stable step {:.3e}.",
        det_min, det_max, dt
    );
    let pulse_center = Point2::new(-0.5, -0.4);
    let edited_control = (geometry_spans + 1, (geometry_spans + 3) * 3 / 4);
    let control_point = |p: &SplinePatch| match p.geometry() {
        GeometryMap::Surface {
            basis_y, controls, ..
        } => controls[edited_control.0 * basis_y.functions() + edited_control.1],
        _ => unreachable!(),
    };
    let delta = Point2::new(0.02, 0.0);
    let (edited, report) = base
        .with_moved_control(edited_control.0, edited_control.1, delta)
        .unwrap();
    let edited_dt = stable_step(&edited);
    let moved_point = control_point(&edited);
    println!(
        "Control point {:?} at ({:.2}, {:.2}) moved by ({}, {}): {} of {} samples rebuilt ({:.1}%), {} of {} dofs touched ({:.1}%); stable step {:.3e} ({:.3} of before).",
        edited_control,
        moved_point.x,
        moved_point.y,
        delta.x,
        delta.y,
        report.affected_samples,
        report.samples,
        100.0 * report.affected_samples as f64 / report.samples as f64,
        report.affected_dofs,
        report.degrees_of_freedom,
        100.0 * report.affected_dofs as f64 / report.degrees_of_freedom as f64,
        edited_dt,
        edited_dt / dt,
    );
    // The far edit: the pulse has not reached the edited region at the edit.
    let edit_time = 0.3;
    let end_time = 1.0;
    let mut before =
        PatchStepper::with_integrator(&base, treatment, integrator, dt, pulse(&base, pulse_center));
    step_to(&mut before, edit_time);
    let centered = before.centered_flux();
    let energy_before = physical_energy(&base, before.field(), &centered);
    let (_, gradient_residual) = base.flux_potential(&centered);
    let carried = base.carry_flux(&edited, &centered);
    let energy_after = physical_energy(&edited, before.field(), &carried);
    let mut after = PatchStepper::with_state(
        &edited,
        treatment,
        integrator,
        edited_dt,
        before.field().to_vec(),
        Some(carried),
    );
    step_to(&mut after, end_time - edit_time);
    let mut reference = PatchStepper::with_integrator(
        &edited,
        treatment,
        integrator,
        edited_dt,
        pulse(&edited, pulse_center),
    );
    step_to(&mut reference, end_time);
    println!(
        "  Edit at t = {edit_time} with the pulse {:.2} from the moved point (flux is a gradient to {:.1e}): energy change {:+.2e}; at t = {end_time} the field differs from a run on the edited box from the start by {:.2e} (relative L2).",
        (moved_point - pulse_center).norm(),
        gradient_residual,
        energy_after / energy_before - 1.0,
        relative_difference(&edited, after.field(), reference.field()),
    );
    // The edit under the pulse: the energy changes with the stretch.
    let under = Point2::new(moved_point.x - 0.2, moved_point.y);
    for scale in [1.0, 2.0, 4.0] {
        let delta = Point2::new(0.004 * scale, 0.0);
        let (edited, _) = base
            .with_moved_control(edited_control.0, edited_control.1, delta)
            .unwrap();
        let mut run =
            PatchStepper::with_integrator(&base, treatment, integrator, dt, pulse(&base, under));
        step_to(&mut run, 0.05);
        let centered = run.centered_flux();
        let before = physical_energy(&base, run.field(), &centered);
        let carried = base.carry_flux(&edited, &centered);
        let after = physical_energy(&edited, run.field(), &carried);
        println!(
            "  Edit by {:.3} under the pulse: energy change {:+.3e}.",
            delta.x,
            after / before - 1.0
        );
    }
    println!();

    // --- Refinement ---
    let coarse = base.clone();
    let fine = coarse.refined().unwrap();
    let coarse_dt = dt;
    // Half the coarse step keeps every run on the same instants; it must
    // sit under the fine patch's own limit.
    let fine_dt = 0.5 * coarse_dt;
    assert!(fine_dt < stable_step(&fine) / 0.8);
    let coarse_steps_to_refine = (0.3 / coarse_dt).round() as u64;
    let coarse_steps_to_end = (end_time / coarse_dt).round() as u64;
    let refine_time = coarse_steps_to_refine as f64 * coarse_dt;
    let mut on_coarse = PatchStepper::with_integrator(
        &coarse,
        treatment,
        integrator,
        coarse_dt,
        pulse(&coarse, pulse_center),
    );
    while on_coarse.steps() < coarse_steps_to_refine {
        on_coarse.step();
    }
    let centered = on_coarse.centered_flux();
    let coarse_energy = physical_energy(&coarse, on_coarse.field(), &centered);
    let (_, gradient_residual) = coarse.flux_potential(&centered);
    let u_fine = coarse.refine_field(&fine, on_coarse.field());
    let b_fine = coarse.refine_flux(&fine, &centered);
    let fine_energy = physical_energy(&fine, &u_fine, &b_fine);
    let mut refined_run =
        PatchStepper::with_state(&fine, treatment, integrator, fine_dt, u_fine, Some(b_fine));
    while refined_run.steps() < 2 * (coarse_steps_to_end - coarse_steps_to_refine) {
        refined_run.step();
    }
    let mut fine_reference = PatchStepper::with_integrator(
        &fine,
        treatment,
        integrator,
        fine_dt,
        pulse(&fine, pulse_center),
    );
    while fine_reference.steps() < 2 * coarse_steps_to_end {
        fine_reference.step();
    }
    let mut coarse_reference = PatchStepper::with_integrator(
        &coarse,
        treatment,
        integrator,
        coarse_dt,
        pulse(&coarse, pulse_center),
    );
    while coarse_reference.steps() < coarse_steps_to_end {
        coarse_reference.step();
    }
    let coarse_on_fine = coarse.refine_field(&fine, coarse_reference.field());
    println!(
        "Refinement {} × {} → {} × {} spans at t = {refine_time:.4} (flux is a gradient to {:.1e}): energy change {:+.2e}; at t = {:.4} the refined run differs from the fine run from the start by {:.2e}, where the coarse run differs by {:.2e}.",
        coarse.basis_x().spans(),
        coarse.basis_y().spans(),
        fine.basis_x().spans(),
        fine.basis_y().spans(),
        gradient_residual,
        fine_energy / coarse_energy - 1.0,
        coarse_steps_to_end as f64 * coarse_dt,
        relative_difference(&fine, refined_run.field(), fine_reference.field()),
        relative_difference(&fine, &coarse_on_fine, fine_reference.field()),
    );
    println!();

    // --- Cost on the deformed box ---
    let flat = SplinePatch::unclamped(
        2,
        Point2::new(-1.0, -1.0),
        Point2::new(2.0, 2.0),
        [side - 2, side - 2],
        2,
    )
    .unwrap();
    let flat_dt = stable_step(&flat);
    for (name, p, dt) in [("flat box", &flat, flat_dt), ("deformed box", &base, dt)] {
        let mut run =
            PatchStepper::with_integrator(p, treatment, integrator, dt, pulse(p, pulse_center));
        let start = Instant::now();
        step_to(&mut run, 2.0);
        let wall = start.elapsed().as_secs_f64();
        println!(
            "{name}: {} DOFs, dt {:.3e}, {} steps to t = 2, {:.1} simulated s/wall s",
            p.degrees_of_freedom(),
            dt,
            run.steps(),
            run.time() / wall
        );
    }
}
