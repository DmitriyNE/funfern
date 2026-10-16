#!/usr/bin/env python3
"""PH-08, the boundary operator: production's walls on the rectangle against
the exact outgoing multipoles' Dirichlet-to-Neumann pairs.

`export_the_rectangles_wall_traces` writes each wall's trace as production
assembles it on the box `[-1, 1]²` of a unit medium (`c = Z = 1`): the trace
nodes, their first-order damping `D`, and the second-order wall's modes, each
its decay `d` and its row of `T` (`Tᵀ T = D`). For a source at `x₀` of order
`m` the field `u = H_m(k|x − x₀|) e^{imθ}` is exact outside it, and so is its
normal derivative on Γ. With the field the primary one, `e^{−iωt}`, `ω = k`,
the boundary's weak flux - what a step adds to `Q` - is exactly

    F_exact,i = (i/ω) ∫_Γ φ_i ∂_n u,

and the wall's, on the exact field's nodal values `U`, is

    F_law = −Tᵀ diag(Y(−iω, d)) T U,   Y = 1 + d (6/7 / s − 8/7 / (s + d) + 2/7 / (s + 2d)),

or `−D U` for the first-order wall. The difference is the law's error at the
boundary, the corners' closed-ring tangential operator included, on that mesh.
Reported per case: the flux error in the `D⁻¹` norm relative to the exact
flux, the share of its square within 0.25 of a corner (a quarter of Γ), and
the radiated power `−½ Re Uᴴ F` against the exact pair's, itself checked
against the analytic `2/ω` per unit multipole.

    boundary.py <directory of trace-*.json> <results.json>
"""

import json
import pathlib
import sys

import mpmath
import numpy as np
from scipy.special import hankel1

RESIDUES = (6.0 / 7.0, -8.0 / 7.0, 2.0 / 7.0)
GAUSS = np.polynomial.legendre.leggauss(6)
GAUSS_T = 0.5 * (GAUSS[0] + 1.0)
GAUSS_W = 0.5 * GAUSS[1]
CORNERS = np.array([[1.0, 1.0], [-1.0, 1.0], [-1.0, -1.0], [1.0, -1.0]])

POSITIONS = [
    ("centre", (0.0, 0.0)),
    ("off-centre", (0.4, 0.2)),
    ("0.25 from an edge", (0.75, 0.0)),
    ("0.05 from an edge", (0.95, 0.0)),
    ("0.3 from a corner", (0.7, 0.7)),
    ("0.05 from a corner", (0.95, 0.95)),
]
ORDERS = [0, 1, 2, 4]
WAVENUMBERS = [2.0, 5.0, 10.0]
# Two multipoles at once: (order, position, amplitude).
COMBINED = [(0, (0.4, 0.2), 1.0), (2, (-0.3, -0.5), 0.7)]


def load(path):
    record = json.loads(pathlib.Path(path).read_text())
    nodes = np.array(record["nodes"], dtype=float)
    first = np.array(record["first_order_damping"], dtype=float)
    if record["modes"]:
        trace = np.array([mode["trace"] for mode in record["modes"]], dtype=float)
        decay = np.array([mode["decay"] for mode in record["modes"]], dtype=float)
    else:
        trace, decay = None, None
    return record["wall"], record["edge"], nodes, first, trace, decay


def quadratic_edges(nodes):
    """The trace's quadratic edges, side by side: (first, middle, last, outward normal)."""
    tolerance = 1.0e-9
    sides = [
        (0, 1.0, 1),
        (0, -1.0, 1),
        (1, 1.0, 0),
        (1, -1.0, 0),
    ]
    edges = []
    for axis, value, along in sides:
        on = [i for i, point in enumerate(nodes) if abs(point[axis] - value) < tolerance]
        on.sort(key=lambda i: nodes[i][along])
        assert len(on) % 2 == 1 and len(on) >= 3, (axis, value, len(on))
        normal = np.zeros(2)
        normal[axis] = value
        for edge in range(len(on) // 2):
            a, m, b = on[2 * edge], on[2 * edge + 1], on[2 * edge + 2]
            length = np.linalg.norm(nodes[b] - nodes[a])
            middle = 0.5 * (nodes[a] + nodes[b])
            assert np.linalg.norm(nodes[m] - middle) < 1.0e-9 * max(length, 1.0), (a, m, b)
            edges.append((a, m, b, normal))
    covered = {index for edge in edges for index in edge[:3]}
    assert covered == set(range(len(nodes))), "every trace node lies on a side"
    return edges


def multipole(order, centre, k, points):
    """`H_m(k r) e^{imθ}` and its gradient at `points`."""
    offset = points - np.asarray(centre)
    r = np.hypot(offset[:, 0], offset[:, 1])
    theta = np.arctan2(offset[:, 1], offset[:, 0])
    z = k * r
    h = hankel1(order, z)
    dh = -hankel1(1, z) if order == 0 else hankel1(order - 1, z) - order / z * h
    phase = np.exp(1j * order * theta)
    radial = np.stack([np.cos(theta), np.sin(theta)], axis=1)
    angular = np.stack([-np.sin(theta), np.cos(theta)], axis=1)
    value = h * phase
    gradient = (k * dh * phase)[:, None] * radial + (1j * order / r * h * phase)[:, None] * angular
    return value, gradient


def field(sources, k, points):
    value = np.zeros(len(points), dtype=complex)
    gradient = np.zeros((len(points), 2), dtype=complex)
    for order, centre, amplitude in sources:
        v, g = multipole(order, centre, k, points)
        value += amplitude * v
        gradient += amplitude * g
    return value, gradient


def exact_flux(nodes, edges, sources, k):
    """`(i/ω) ∫ φ_i ∂_n u` by six Gauss points on each quadratic edge."""
    flux = np.zeros(len(nodes), dtype=complex)
    shapes = np.stack(
        [
            (1 - GAUSS_T) * (1 - 2 * GAUSS_T),
            4 * GAUSS_T * (1 - GAUSS_T),
            GAUSS_T * (2 * GAUSS_T - 1),
        ]
    )
    for a, m, b, normal in edges:
        start, end = nodes[a], nodes[b]
        points = start[None, :] + GAUSS_T[:, None] * (end - start)[None, :]
        length = np.linalg.norm(end - start)
        _, gradient = field(sources, k, points)
        normal_derivative = gradient @ normal
        for local, node in enumerate((a, m, b)):
            flux[node] += length * np.sum(GAUSS_W * shapes[local] * normal_derivative)
    return 1j / k * flux


def law_flux(wall, first, trace, decay, values, k):
    if wall == "first":
        return -first * values
    s = -1j * k
    admittance = np.ones(len(decay), dtype=complex)
    active = decay > 0.0
    d = decay[active]
    admittance[active] += d * (
        RESIDUES[0] / s + RESIDUES[1] / (s + d) + RESIDUES[2] / (s + 2 * d)
    )
    return -trace.T @ (admittance * (trace @ values))


def lumped_damping(wall, first, trace):
    return first if wall == "first" else np.sum(trace * trace, axis=0)


def read_case(wall, nodes, edges, first, trace, decay, sources, k):
    values, _ = field(sources, k, nodes)
    exact = exact_flux(nodes, edges, sources, k)
    law = law_flux(wall, first, trace, decay, values, k)
    weight = 1.0 / lumped_damping(wall, first, trace)
    error = law - exact
    norm = lambda flux: np.sqrt(np.sum(weight * np.abs(flux) ** 2))
    near = np.min(np.linalg.norm(nodes[:, None, :] - CORNERS[None, :, :], axis=2), axis=1) <= 0.25
    error_squared = weight * np.abs(error) ** 2
    power = lambda flux: -0.5 * np.real(np.vdot(values, flux))
    return {
        "flux_error": float(norm(error) / norm(exact)),
        "corner_share": float(np.sum(error_squared[near]) / np.sum(error_squared)),
        "power_exact": float(power(exact)),
        "power_law": float(power(law)),
    }


def check_hankel():
    """scipy's Hankel functions against mpmath's over the arguments the cases reach."""
    worst = 0.0
    for order in range(0, 6):
        for z in (0.05, 0.1, 0.5, 1.0, 3.7, 9.2, 14.0, 28.5):
            reference = complex(mpmath.hankel1(order, z))
            value = complex(hankel1(order, z))
            worst = max(worst, abs(value - reference) / abs(reference))
    return worst


def main():
    directory, output = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    hankel = check_hankel()
    print(f"scipy's H1_m against mpmath's, orders 0 to 5, z from 0.05 to 28.5: {hankel:.1e}")
    cases = [
        (f"m = {order}, {name}", [(order, centre, 1.0)])
        for name, centre in POSITIONS
        for order in ORDERS
    ]
    cases.append(("m = 0 off-centre with m = 2 at (-0.3, -0.5)", COMBINED))
    results = {"hankel_check": hankel, "readings": []}
    for wall in ("first", "second"):
        for edge in (0.1, 0.07, 0.05):
            name, _, nodes, first, trace, decay = load(directory / f"trace-{wall}-{edge}.json")
            assert name == wall
            edges = quadratic_edges(nodes)
            # Either wall damps by `Z ℓ` lumped by Simpson's weights; the
            # second-order wall's damping is all in its modes, `Tᵀ T = D`.
            if wall == "second":
                assert np.max(np.abs(first)) == 0.0
            simpson = np.zeros(len(nodes))
            for a, m, b, _ in edges:
                length = np.linalg.norm(nodes[b] - nodes[a])
                simpson[[a, m, b]] += length * np.array([1 / 6, 2 / 3, 1 / 6])
            lumps = lumped_damping(wall, first, trace)
            assert np.max(np.abs(lumps - simpson)) < 1.0e-10, np.max(np.abs(lumps - simpson))
            for k in WAVENUMBERS:
                for label, sources in cases:
                    reading = read_case(wall, nodes, edges, first, trace, decay, sources, k)
                    reading.update(
                        {"wall": wall, "edge": edge, "k": k, "case": label, "nodes": len(nodes)}
                    )
                    results["readings"].append(reading)
    output.write_text(json.dumps(results, indent=1))
    analytic = lambda k: 2.0 / k
    print("flux error (D⁻¹ norm, of the exact), corner share of its square, P_law/P_exact; "
          "P_exact against 2/ω on the finest mesh")
    by_key = {}
    for reading in results["readings"]:
        by_key[(reading["wall"], reading["k"], reading["case"], reading["edge"])] = reading
    for k in WAVENUMBERS:
        print(f"\nk = {k:g}")
        for label, sources in cases:
            line = f"  {label:46s}"
            # Where the exact pair's own power misses `2/ω`, the mesh does not
            # resolve the boundary data and no power is read.
            exact_power = by_key[("second", k, label, 0.05)]["power_exact"] / analytic(k)
            resolved = len(sources) > 1 or abs(exact_power - 1.0) < 1.0e-3
            for wall in ("first", "second"):
                errors = [by_key[(wall, k, label, edge)]["flux_error"] for edge in (0.1, 0.07, 0.05)]
                fine = by_key[(wall, k, label, 0.05)]
                power = (
                    f"{fine['power_law'] / fine['power_exact']:.4f}" if resolved else "unresolved"
                )
                line += (
                    f" | {wall[0]}: {errors[0]:.3e} {errors[1]:.3e} {errors[2]:.3e}"
                    f" corners {fine['corner_share']:.2f} P {power}"
                )
            if len(sources) == 1:
                line += f" | P_exact {exact_power:.5f}"
            print(line)


if __name__ == "__main__":
    main()
