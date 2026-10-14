"""Hankel functions for PH-06 and PH-07, the circular walls.

Writes cylinder-hankel.json beside this file: for every case the tests run,
an outgoing circle or an absorbing hole of radius one at wavenumber k (the
speed is one, so k is kR) and angular order m, the radii the test fits at and
H1_m(k r), H2_m(k r) there, and H1_m, H2_m and their derivatives at the wall,
k r = k. The radii are the test's own rule, written here so the table carries
them and the test reads them back rather than recomputing them.

Checks before writing: at every point the Wronskian J Y' - J' Y = 2/(pi x),
and every value against a second evaluation at twice the precision.

Run by hand when the cases change, with mpmath (not a test dependency):
    python3 -m venv /tmp/hankel && /tmp/hankel/bin/pip install mpmath
    /tmp/hankel/bin/python tests/physics/references/cylinder_hankel.py
"""
import json
import os

import mpmath as mp

# The cases: each kR with the angular orders that propagate at the wall.
CASES = {0.8: [0], 2.0: [0, 1, 2], 5.0: [0, 1, 2, 4], 10.0: [0, 1, 2, 4, 8]}


def outgoing_radii(k, m):
    """Between the source ring and the wall: the ring sits where the order
    radiates, k a near m, and the fit starts 0.08 outside it."""
    inner = min(max(0.75 * m / k, 0.25), 0.6)
    first = inner + 0.08
    count = int((0.98 - first) / 0.02 + 1e-9) + 1
    return [round(first + 0.02 * i, 12) for i in range(count)]


def hole_radii(k, m):
    return [round(1.02 + 0.04 * i, 12) for i in range(23)]


def propagating(k, m, radii):
    """Only where the two Hankel functions stand apart: well below k r = m,
    where the order is evanescent, J_m vanishes beside Y_m, H1 and H2 grow
    parallel and B/A means nothing; to three quarters of m, J_m is a fair
    share of Y_m still."""
    return [r for r in radii if k * r >= max(0.75 * m, 0.5)]


def values(m, x, dps):
    with mp.workdps(dps):
        x = mp.mpf(x)
        h1, h2 = mp.hankel1(m, x), mp.hankel2(m, x)
        dh1 = (mp.hankel1(m - 1, x) - mp.hankel1(m + 1, x)) / 2 if m > 0 else -mp.hankel1(1, x)
        dh2 = (mp.hankel2(m - 1, x) - mp.hankel2(m + 1, x)) / 2 if m > 0 else -mp.hankel2(1, x)
        j, y = (h1 + h2) / 2, (h1 - h2) / (2j)
        dj, dy = (dh1 + dh2) / 2, (dh1 - dh2) / (2j)
        wronskian = (j * dy - dj * y) * mp.pi * x / 2 - 1
        assert abs(wronskian) < mp.mpf(10) ** (-(dps - 8)), (m, x, wronskian)
        return [complex(v) for v in (h1, h2, dh1, dh2)]


def checked(m, x):
    coarse, fine = values(m, x, 30), values(m, x, 60)
    for a, b in zip(coarse, fine):
        assert abs(a - b) <= 1e-15 * max(abs(b), 1e-300), (m, x, a, b)
    return fine


def pair(z):
    return [z.real, z.imag]


cases = []
for kind, rule in (("outgoing", outgoing_radii), ("hole", hole_radii)):
    for k, orders in CASES.items():
        for m in orders:
            radii = propagating(k, m, rule(k, m))
            assert len(radii) >= 3, (kind, k, m)
            at = [checked(m, k * r) for r in radii]
            wall = checked(m, k)
            cases.append({
                "kind": kind,
                "kr": k,
                "m": m,
                "radii": radii,
                "h1": [pair(v[0]) for v in at],
                "h2": [pair(v[1]) for v in at],
                "wall": {"h1": pair(wall[0]), "h2": pair(wall[1]), "dh1": pair(wall[2]), "dh2": pair(wall[3])},
            })

path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "cylinder-hankel.json")
with open(path, "w") as out:
    json.dump({"generator": "tests/physics/references/cylinder_hankel.py", "mpmath": mp.__version__,
               "radius": 1.0, "cases": cases}, out, indent=1)
    out.write("\n")
print(f"{len(cases)} cases written to {path}")
