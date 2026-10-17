"""Cylinder scattering for PH-12: each case's scattering ratios and the Bessel
and Hankel values the test fits with.

Writes cylinder-scattering.json beside this file. A cylinder of radius a in a
medium of nodal stiffness one and wavenumber k (the speed is one, so k a is
the size parameter), e^{-iwt} in time. Around it, order n of the field is
alpha_n J_n(k r) + c_n H1_n(k r): alpha_n whatever regular field meets it,
c_n what it sends out, and c_n = S_n alpha_n exactly. For each case:

- a reflecting hole, where the flux vanishes: S_n = -J_n'(ka) / H_n'(ka);
- a pinned cylinder, where the field does: S_n = -J_n(ka) / H_n(ka);
- a penetrable one of index n2 = k2/k and stiffness ratio s = s2/s1 for the
  nodal field, the field and its flux s d_r u continuous at r = a:

      S_n = -(k J_n'(ka) J_n(k2 a) - s k2 J_n(ka) J_n'(k2 a))
            / (k H_n'(ka) J_n(k2 a) - s k2 H_n(ka) J_n'(k2 a)).

The test maps each skin's authored medium to (n2, s) itself and finds its
case here by them. The table carries, for orders 0 to the case's largest, S_n,
its slope dS_n/d(ka) at fixed n2 and s, and J_n, H1_n at the radii the test
fits at, which are the test's own rule written here so that the test reads
them back. S_{-n} = S_n for these cylinders, and J_{-n}, H_{-n} are (-1)^n
times J_n, H_n. The slope carries S_n to the mesh's polygon: a polygon
inscribed in the circle scatters, to first order, as the circle of its area,
whose k a is smaller by the test's measured deficit.

Checks before writing: the Wronskian J Y' - J' Y = 2/(pi x) at every radius,
every value and slope against a second evaluation at twice the precision,
every lossless S_n unitary, |1 + 2 S_n| = 1, its slope along the unit circle,
Re(conj(1 + 2 S_n) S_n') = 0, and the slope carrying S_n to the ratio of a
circle smaller by the largest deficit the test meets, 2e-4, to 1e-5.

Run by hand when the cases change, with mpmath (not a test dependency):

    python3 -m venv venv && venv/bin/pip install mpmath
    venv/bin/python tests/physics/references/cylinder_scattering.py
"""

import json
import os

import mpmath as mp

RADIUS = 0.5
# The fit's radii, in units of the cylinder's radius.
FIT = [1.2, 1.4, 1.6, 1.8, 2.0]
# (name, k a, kind, n2, s): kind "hole", "pinned" or "penetrable".
CASES = [
    ("hole, ka 0.5", 0.5, "hole", None, None),
    ("hole, ka 2", 2.0, "hole", None, None),
    ("hole, ka 5", 5.0, "hole", None, None),
    ("pinned, ka 2", 2.0, "pinned", None, None),
    ("pinned, ka 5", 5.0, "pinned", None, None),
]
# Penetrable: the index and the stiffness ratio for the nodal field follow
# from the skin, and the test names the ones it uses; each skin's authored
# medium maps to one of these.
PENETRABLE = [
    (0.5, 2.0, 1.0),
    (2.0, 2.0, 1.0),
    (0.5, 2.0, 0.25),
    (2.0, 2.0, 0.25),
    (2.0, 2.0 ** 0.5, 1.0),
    (2.0, 2.0 ** 0.5, 0.5),
    (2.0, 2.0 ** 0.5, 2.0),
    (5.0, 2.0 ** 0.5, 1.0),
    (5.0, 2.0 ** 0.5, 0.5),
    (0.5, 10.0 ** 0.5, 1.0),
    (0.5, 10.0 ** 0.5, 0.1),
]


def largest_order(ka):
    """Orders the cylinder scatters measurably: past k a + 4 the ratio is
    far under the fit's resolution."""
    return int(ka + 4)


def bessel(n, x, dps):
    with mp.workdps(dps):
        x = mp.mpf(x)
        j, y = mp.besselj(n, x), mp.bessely(n, x)
        dj = mp.besselj(n, x, derivative=1)
        dy = mp.bessely(n, x, derivative=1)
        return j, y, dj, dy


def ratio(kind, n, ka, n2, s, dps):
    j, y, dj, dy = bessel(n, ka, dps)
    with mp.workdps(dps):
        h, dh = mp.mpc(j, y), mp.mpc(dj, dy)
        if kind == "hole":
            return -dj / dh
        if kind == "pinned":
            return -j / h
        k2a = ka * mp.mpf(n2)
        j2, _, dj2, _ = bessel(n, k2a, dps)
        s = mp.mpf(s)
        n2 = mp.mpf(n2)
        return -(dj * j2 - s * n2 * j * dj2) / (dh * j2 - s * n2 * h * dj2)


def slope(kind, n, ka, n2, s, dps):
    """dS_n/d(ka) by a central difference, its step a third of the digits:
    mpmath's diff steps under the precision ratio rounds its argument to."""
    with mp.workdps(dps):
        h = mp.mpf(10) ** (-dps // 3)
        ka = mp.mpf(ka)
        return (ratio(kind, n, ka + h, n2, s, dps) - ratio(kind, n, ka - h, n2, s, dps)) / (2 * h)


def check(value, precise, what, tolerance=1e-14, floor="1e-300"):
    error = abs(value - precise) / max(abs(precise), mp.mpf(floor))
    assert error < tolerance, (what, float(error))


def case_entry(name, ka, kind, n2, s):
    k = ka / RADIUS
    orders = []
    for n in range(largest_order(ka) + 1):
        value = ratio(kind, n, ka, n2, s, 30)
        check(value, ratio(kind, n, ka, n2, s, 60), (name, n))
        assert abs(abs(1 + 2 * value) - 1) < 1e-14, (name, n, "unitarity")
        gradient = slope(kind, n, ka, n2, s, 30)
        # A hole's slope vanishes at k a = n, where J'' Y' - J' Y'' does.
        check(gradient, slope(kind, n, ka, n2, s, 60), (name, n, "slope"), 1e-12, 1)
        along = (mp.conj(1 + 2 * value) * gradient).real
        assert abs(along) < 1e-12 * max(abs(gradient), 1), (name, n, "slope off the circle")
        shift = ka * mp.mpf("2e-4")
        shrunk = ratio(kind, n, ka - shift, n2, s, 30)
        assert abs(shrunk - (value - shift * gradient)) < 1e-5, (name, n, "first order")
        radii = []
        for factor in FIT:
            x = k * RADIUS * factor
            j, y, dj, dy = bessel(n, x, 30)
            jp, yp, _, _ = bessel(n, x, 60)
            check(mp.mpc(j, y), mp.mpc(jp, yp), (name, n, factor))
            wronskian = j * dy - dj * y
            assert abs(wronskian - 2 / (mp.pi * x)) < 1e-14 * (2 / (mp.pi * x)), (name, n, x)
            # H1 = J + iY: J is its real part.
            radii.append({"radius": RADIUS * factor, "h": [float(j), float(y)]})
        orders.append(
            {
                "order": n,
                "ratio": [float(value.real), float(value.imag)],
                "slope": [float(gradient.real), float(gradient.imag)],
                "radii": radii,
            }
        )
    return {
        "name": name,
        "ka": ka,
        "kind": kind,
        "index": n2,
        "stiffness_ratio": s,
        "orders": orders,
    }


def main():
    entries = [case_entry(*case) for case in CASES]
    for ka, n2, s in PENETRABLE:
        entries.append(case_entry(f"penetrable, ka {ka:g}, n {n2:.6g}, s {s:g}", ka, "penetrable", n2, s))
    table = {"radius": RADIUS, "fit": FIT, "cases": entries}
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "cylinder-scattering.json")
    with open(path, "w") as out:
        json.dump(table, out, indent=1)
        out.write("\n")
    print(f"{len(entries)} cases written to {path}")


if __name__ == "__main__":
    main()
