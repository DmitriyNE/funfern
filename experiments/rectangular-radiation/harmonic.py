#!/usr/bin/env python3
"""PH-08, the full solver: production's harmonic steady state on the walled box
against the source's exact free-space field.

`export_the_rectangles_harmonic_fields` drives the box `[-1, 1]²` of a unit
medium, every side the first- or the second-order wall, by a volume source
`(r/σ)ᵐ e^{−r²/σ²} cos mθ` about `x₀`, carrier `sin ω(t − t_c)`, and writes every
node's `e^{−iωt}` phasor `U` of the primary field after it has settled, with
the wall's and the source's time-averaged power. In free space the same
source gives, with `ψ̇ = u`, `(Δ + k²)Ψ = −S f`, `S = i e^{iωt_c}`,

    U_free = −iω S (iπ/2) cos mθ [H_m(kr) ∫₀ʳ J_m g r′ dr′ + J_m(kr) ∫ᵣ^∞ H_m g r′ dr′],

`g = (r/σ)ᵐ e^{−r²/σ²}`, exactly. What the box returns is `U − U_free`. Reported
per run: that in the lumped-mass norm relative to `U_free` - the walls'
reflection with the discretization's own error, which the meshes separate -
the amplitude a least-squares fit of `U_free` to `U` finds (one, if the
source is read as the solver applies it), and the wall's power against the
power the source radiates in free space.

    harmonic.py <directory of field-*.json> <results.json>
"""

import json
import pathlib
import sys

import numpy as np
from scipy.integrate import cumulative_trapezoid
from scipy.special import hankel1, jv


def radial_profile(order, k, sigma, radii):
    """`H_m(kr) ∫₀ʳ J_m g r′ + J_m(kr) ∫ᵣ^∞ H_m g r′` at `radii`, and `∫₀^∞ J_m g r′`."""
    grid = np.linspace(0.0, 12.0 * sigma, 240_001)
    g = (grid / sigma) ** order * np.exp(-((grid / sigma) ** 2))
    inner = cumulative_trapezoid(jv(order, k * grid) * g * grid, grid, initial=0.0)
    with np.errstate(invalid="ignore"):
        outer_integrand = np.where(grid > 0.0, hankel1(order, k * grid) * g * grid, 0.0)
    outer_from_zero = cumulative_trapezoid(outer_integrand, grid, initial=0.0)
    outer = outer_from_zero[-1] - outer_from_zero
    total = inner[-1]
    inside = np.interp(np.minimum(radii, grid[-1]), grid, inner)
    beyond = np.interp(np.minimum(radii, grid[-1]), grid, outer.real) + 1j * np.interp(
        np.minimum(radii, grid[-1]), grid, outer.imag
    )
    profile = jv(order, k * radii) * beyond
    away = radii > 0.0
    profile[away] += hankel1(order, k * radii[away]) * inside[away]
    return profile, total


# The source integral `∫₀^∞ J_m g r dr` against its closed form
# `(σ²/2)(kσ/2)ᵐ e^{−(kσ)²/4}`, run by run.
INTEGRAL_CHECK = []


def free_field(record, nodes):
    k, sigma = record["k"], record["sigma"]
    source_phasor = 1j * np.exp(1j * k * record["carrier_centre"])
    field = np.zeros(len(nodes), dtype=complex)
    power = 0.0
    for order, x0, y0, amplitude in record["sources"]:
        order = int(order)
        offset = nodes - np.array([x0, y0])
        r = np.hypot(offset[:, 0], offset[:, 1])
        theta = np.arctan2(offset[:, 1], offset[:, 0])
        profile, total = radial_profile(order, k, sigma, r)
        closed = sigma**2 / 2.0 * (k * sigma / 2.0) ** order * np.exp(-((k * sigma) ** 2) / 4.0)
        INTEGRAL_CHECK.append(abs(total - closed) / closed)
        scale = amplitude * (-1j * k) * source_phasor * (1j * np.pi / 2.0)
        field += scale * np.cos(order * theta) * profile
        # `a cos mθ H_m` radiates `|a|²/ω`, and `a H_0` twice that.
        far = abs(scale * total) ** 2 / k
        power += 2.0 * far if order == 0 else far
    return field, power


def read_run(path):
    record = json.loads(pathlib.Path(path).read_text())
    nodes = np.array(record["nodes"], dtype=float)
    mass = np.array(record["mass"], dtype=float)
    measured = np.array(record["phasor_re"]) + 1j * np.array(record["phasor_im"])
    free, power = free_field(record, nodes)
    norm = lambda field: np.sqrt(np.sum(mass * np.abs(field) ** 2))
    fit = np.sum(mass * np.conj(free) * measured) / np.sum(mass * np.abs(free) ** 2)
    return {
        "wall": record["wall"],
        "edge": record["edge"],
        "k": record["k"],
        "case": pathlib.Path(path).stem.split(f"-k{record['k']:g}-")[-1],
        "returned": float(norm(measured - free) / norm(free)),
        "fit": [float(fit.real), float(fit.imag)],
        "wall_power": record["wall_power"],
        "source_power": record["source_power"],
        "free_power": float(power),
        "single": len(record["sources"]) == 1,
    }


def main():
    directory, output = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    readings = [read_run(path) for path in sorted(directory.glob("field-*.json"))]
    output.write_text(json.dumps({"integral_check": max(INTEGRAL_CHECK), "readings": readings}, indent=1))
    print(f"the source integral against its closed form, every run: {max(INTEGRAL_CHECK):.1e}")
    table = {(r["wall"], r["k"], r["case"], r["edge"]): r for r in readings}
    cases = sorted({r["case"] for r in readings})
    print("returned field ‖U − U_free‖/‖U_free‖ on edges 0.1, 0.07 (0.05); "
          "the fitted amplitude; the wall's power against the free-space radiation")
    for k in sorted({r["k"] for r in readings}):
        print(f"\nk = {k:g}")
        for case in cases:
            line = f"  {case:16s}"
            for wall in ("first", "second"):
                levels = [table.get((wall, k, case, edge)) for edge in (0.1, 0.07, 0.05)]
                levels = [level for level in levels if level is not None]
                if not levels:
                    continue
                fine = levels[-1]
                returned = " ".join(f"{level['returned']:.3e}" for level in levels)
                fit = complex(*fine["fit"])
                line += f" | {wall[0]}: {returned}"
                line += f" fit {abs(fit):.4f}∠{np.angle(fit):+.3f}"
                line += f" P {fine['wall_power'] / fine['free_power']:.4f}"
                line += f" (source {fine['source_power'] / fine['wall_power'] - 1:+.1e})"
            print(line)


if __name__ == "__main__":
    main()
