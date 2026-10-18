"""A line source beside PH-12's cylinders, for PH-18's far field: the Hankel
values at the source's distance that the pattern's sum needs.

Writes cylinder-source.json beside this file. PH-12's cylinder, radius a at
the origin, in a medium of speed one, e^{-iwt} in time; a line source at
distance rho from its centre, outside it. Its field is H1_0(k |r - r_s|),
which about the cylinder is sum_n J_n(k r) H1_n(k rho) e^{in(theta - phi_s)}
inside rho: order n meets the cylinder with alpha_n = H1_n(k rho) e^{-in phi_s},
and the cylinder sends out S_n alpha_n H1_n(k r) e^{in theta}. Far away
H1_n(k r) ~ sqrt(2/(pi k r)) e^{i(k r - n pi/2 - pi/4)}, so the pattern is

    F(theta) = e^{-ik rho cos(theta - phi_s)}
             + sum_n (-i)^n S_n H1_n(k rho) e^{in(theta - phi_s)},

the test's closed form, with S_n from cylinder-scattering.json and
H1_{-n} = (-1)^n H1_n. The table carries H1_n(k rho) for orders 0 to
cylinder-scattering.json's largest at each k a.

Checks before writing: every value against a second evaluation at twice the
precision, and the Wronskian J Y' - J' Y = 2/(pi x).

Run by hand when the cases change, with mpmath (not a test dependency):

    python3 -m venv venv && venv/bin/pip install mpmath
    venv/bin/python tests/physics/references/cylinder_source.py
"""

import json
import os

import mpmath as mp

RADIUS = 0.5
# The source's distance from the cylinder's centre.
SOURCE = 1.1
# The sizes k a whose patterns the test reads.
SIZES = [2.0]


def largest_order(ka):
    """cylinder_scattering.py's own rule."""
    return int(ka + 4)


def hankel(n, x, dps):
    with mp.workdps(dps):
        x = mp.mpf(x)
        return (
            mp.besselj(n, x),
            mp.bessely(n, x),
            mp.besselj(n, x, derivative=1),
            mp.bessely(n, x, derivative=1),
        )


def entry(ka):
    k = ka / RADIUS
    x = k * SOURCE
    values = []
    for n in range(largest_order(ka) + 1):
        j, y, dj, dy = hankel(n, x, 30)
        jp, yp, _, _ = hankel(n, x, 60)
        for value, precise in [(j, jp), (y, yp)]:
            assert abs(value - precise) < 1e-14 * max(abs(precise), 1e-300), (ka, n)
        wronskian = j * dy - dj * y
        assert abs(wronskian - 2 / (mp.pi * x)) < 1e-14 * (2 / (mp.pi * x)), (ka, n)
        values.append([float(j), float(y)])
    return {"ka": ka, "k_rho": float(x), "hankel": values}


def main():
    table = {
        "radius": RADIUS,
        "source": SOURCE,
        "cases": [entry(ka) for ka in SIZES],
    }
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "cylinder-source.json")
    with open(path, "w") as out:
        json.dump(table, out, indent=1)
        out.write("\n")


if __name__ == "__main__":
    main()
