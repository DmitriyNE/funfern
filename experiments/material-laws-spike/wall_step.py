"""The outgoing wall under the production step, by its exact discrete harmonic response.

The Bloch strip of scattering.py, stepped by an explicit one-step map built
from its stages, so any kick, drift or wall update can be tried without
deriving its harmonic equation by hand. State: the free nodes' Q, the wall's
auxiliary Z, the free nodes' force F, and the drive. A harmonic solution of
the map is fitted to the strip's discrete propagating modes as scattering.py
fits its closed form, and the 2D coarse mesh of boundary.py checks stability,
energy and a pulse in the time domain.

See docs/spikes/funfern-outgoing-wall-step-spike.md for the derivation and
the reading of the results.
"""
import json
import math
import sys
import time
import numpy as np
from spike import Mesh, check, observation, RESULTS
from scattering import Cell, Strip, admittance
from boundary import Boundary

# The fixture's mesh steps at 1.2 of its leapfrog bound 4.517e-3 s (edge 0.05).
FIXTURE_BOUND = 4.517e-3
# The structured strip at this cell is about as stiff as that mesh: its own
# bound is 5.46e-3 s.
STRIP_CELL = .025


def expm(a):
    norm = np.abs(a).sum(axis=1).max()
    squarings = max(0, int(math.ceil(math.log2(max(norm, 1e-300)/.25))))
    a = a/2**squarings
    result = np.eye(len(a), dtype=complex)
    term = np.eye(len(a), dtype=complex)
    for k in range(1, 20):
        term = term@a/k
        result = result+term
    for _ in range(squarings):
        result = result@result
    return result


class Scheme:
    """One way to step the wall.

    fourth: the drift-form fourth-order step, u~ = u - dt^2/12 M^-1 K u.
    wall: 'halves' kicks the wall in each half-kick by `kick` ('cayley',
      'pade22', 'exact', 'sqrt-cayley', 'sqrt-pade'); 'after' kicks the trace
      as the bulk and then takes one Cayley over (Q_b, Z) for the whole step;
      'trapezoid' splits that Cayley into its halves, (I + dt/2 G) with the
      first kick and (I - dt/2 G)^-1 after the second.
    force_quadratic, generator_cubic, drift_wall: the Taylor-derived coupling
      (kick force (I + c dt^2 G^2) E F, generator G + c dt^2 G^3, u~_b +=
      c dt^2 M^-1 (G^2 X)_Q).
    wall_acceleration: u~_b += c dt^2 M^-1 (G (G X - E F_mid))_Q, the wall's
      share of dt^2/12 u'', with F_mid = F + dt/2 K u.
    kick_form: the fourth-order kick form, its linear limit: each kick takes
      F~ = F - dt^2/12 K M^-1 F and the drift the plain field.
    wall_rate: under the kick form, the trace's rate in F~ includes the
      wall's, F - (G X)_Q, with X as the kick finds it.
    """
    def __init__(self, name, fourth=True, wall='halves', kick='cayley', force_quadratic=0.,
                 generator_cubic=0., drift_wall=0., wall_acceleration=0., kick_form=False,
                 wall_rate=False):
        self.name = name
        self.fourth = fourth
        self.wall = wall
        self.kick = kick
        self.force_quadratic = force_quadratic
        self.generator_cubic = generator_cubic
        self.drift_wall = drift_wall
        self.wall_acceleration = wall_acceleration
        self.kick_form = kick_form
        self.wall_rate = wall_rate


PRODUCTION = Scheme('production')
LEAPFROG = Scheme('leapfrog', fourth=False)
PADE = Scheme('pade22 halves', kick='pade22')
EXACT = Scheme('exact halves', kick='exact')
SQRT_PADE = Scheme('sqrt of full-step pade', kick='sqrt-pade')
SQRT_CAYLEY = Scheme('sqrt of full-step cayley', kick='sqrt-cayley')
ACCELERATION = Scheme('wall acceleration only', wall_acceleration=1/12)
TAYLOR = Scheme('taylor-derived pair, pade halves', kick='pade22', force_quadratic=-1/24, drift_wall=1/24)
TAYLOR_CAYLEY = Scheme('taylor-derived pair, cayley halves', generator_cubic=-1/48, force_quadratic=-1/16, drift_wall=1/24)
TARGET = Scheme('full-step cayley + wall acceleration', wall='after', wall_acceleration=1/12)
TARGET_SQRT = Scheme('same, as square-root halves', kick='sqrt-cayley', wall_acceleration=1/12)
TRAPEZOID = Scheme('same, as the trapezoid split', wall='trapezoid', wall_acceleration=1/12)
KICK_PRODUCTION = Scheme('kick form, production wall', kick_form=True)
KICK_PLAIN = Scheme('kick form, trapezoid split, no wall rate', wall='trapezoid', kick_form=True)
KICK_TRAPEZOID = Scheme('kick form, trapezoid split, wall rate', wall='trapezoid', kick_form=True, wall_rate=True)
LEAPFROG_TARGET = Scheme('leapfrog, full-step cayley', fourth=False, wall='after')


class Model:
    def __init__(self, strip, kind):
        self.strip = strip
        self.kind = kind
        n = strip.n
        self.n = n
        self.mass = strip.mass.astype(float)
        self.k = strip.k
        g = strip.generator(kind)
        scale = np.r_[np.sqrt(self.mass[-2:]), np.ones(len(g)-2)]
        self.g = scale[:, None]*g/scale[None, :]
        self.m = len(g)-2
        self.nf = n-2
        nf, m = self.nf, self.m
        self.N = 2*nf+m+1
        self.q = np.arange(nf)
        self.z = nf+np.arange(m)
        self.f = nf+m+np.arange(nf)
        self.drive = self.N-1
        self.x = np.r_[self.q[-2:], self.z]
        self.fb = self.f[-2:]
        self.pin = np.array([1, np.exp(1j*strip.ky*strip.h/2)])
        self.e = np.zeros((len(g), 2)); self.e[:2, :2] = np.eye(2)

    def field(self):
        u = np.zeros((self.n, self.N), dtype=complex)
        u[2+np.arange(self.nf), self.q] = 1/self.mass[2:]
        u[:2, self.drive] = self.pin
        return u

    def plain_kick(self, tau):
        kick = np.eye(self.N, dtype=complex)
        kick[self.q, self.f] = -tau
        return kick

    def wall_kick(self, tau, dt, scheme):
        """A half-kick that steps the wall by X' = A X + B F_b."""
        g = self.g+scheme.generator_cubic*dt*dt*(self.g@self.g@self.g)
        eye = np.eye(len(g))
        e = self.e
        if scheme.kick == 'exact':
            big = np.zeros((len(g)+2, len(g)+2), dtype=complex)
            big[:len(g), :len(g)] = tau*g
            big[:len(g), len(g):] = -tau*e
            flow = expm(big)
            a, b = flow[:len(g), :len(g)], flow[:len(g), len(g):]
        elif scheme.kick in ('sqrt-cayley', 'sqrt-pade'):
            # The half step is the square root of a full-step propagator, and
            # the force weight keeps the wall row's linear term exact:
            # B = -(A - I) G^-1 E.
            values, vectors = np.linalg.eig(dt*g)
            if scheme.kick == 'sqrt-pade':
                full = (1+values/2+values*values/12)/(1-values/2+values*values/12)
            else:
                full = (1+values/2)/(1-values/2)
            half = np.sqrt(full)
            safe = np.where(abs(values) > 1e-12, values, 1)
            weight = np.where(abs(values) > 1e-12, (half-1)/safe, .5)
            inverse = np.linalg.inv(vectors)
            a = vectors@np.diag(half)@inverse
            b = -dt*(vectors@np.diag(weight)@inverse)@e
        else:
            if scheme.kick == 'cayley':
                p, q = eye-tau/2*g, eye+tau/2*g
            else:
                gg = g@g
                p, q = eye-tau/2*g+tau*tau/12*gg, eye+tau/2*g+tau*tau/12*gg
            r = eye+scheme.force_quadratic*dt*dt*(self.g@self.g)
            a = np.linalg.solve(p, q)
            b = -tau*np.linalg.solve(p, r@e)
        kick = self.plain_kick(tau)
        kick[self.x, :] = 0
        kick[np.ix_(self.x, self.x)] = a
        kick[np.ix_(self.x, self.fb)] = b
        return kick

    def corrected_force(self, dt, scheme):
        """The kick form's F~ at every free node, a map of the augmented state."""
        v = np.zeros((self.n, self.N), dtype=complex)
        v[2+np.arange(self.nf), self.f] = 1
        if scheme.wall_rate:
            x = np.zeros((2+self.m, self.N), dtype=complex)
            x[np.arange(2+self.m), self.x] = 1
            v[-2:] -= (self.g@x)[:2]
        corrected = np.zeros((self.nf, self.N), dtype=complex)
        corrected[np.arange(self.nf), self.f] = 1
        corrected -= dt*dt/12*(self.k@(v/self.mass[:, None]))[2:]
        return corrected

    def with_force(self, kick, dt, scheme):
        """The kick reading F~ where it read F, under the kick form."""
        if not scheme.kick_form:
            return kick
        out = kick.copy()
        out[:, self.f] = 0
        out += kick[:, self.f]@self.corrected_force(dt, scheme)
        out[self.f, :] = np.eye(self.N, dtype=complex)[self.f, :]
        return out

    def full_cayley(self, duration):
        eye = np.eye(len(self.g))
        wall = np.eye(self.N, dtype=complex)
        wall[np.ix_(self.x, self.x)] = np.linalg.solve(eye-duration/2*self.g, eye+duration/2*self.g)
        return wall

    def drift(self, dt, scheme):
        u = self.field()
        ku = self.k@u
        ut = u.copy()
        if scheme.fourth and not scheme.kick_form:
            ut[2:] -= dt*dt/12*ku[2:]/self.mass[2:, None]
        x = np.zeros((2+self.m, self.N), dtype=complex)
        x[np.arange(2+self.m), self.x] = 1
        if scheme.drift_wall:
            ut[-2:] += scheme.drift_wall*dt*dt*(self.g@self.g@x)[:2]/self.mass[-2:, None]
        if scheme.wall_acceleration:
            force = np.zeros((2+self.m, self.N), dtype=complex)
            force[:2, self.fb] = np.eye(2)
            force[:2] += dt/2*ku[-2:]
            rate = self.g@x-force
            ut[-2:] += scheme.wall_acceleration*dt*dt*(self.g@rate)[:2]/self.mass[-2:, None]
        drift = np.eye(self.N, dtype=complex)
        drift[self.f, :] += dt*(self.k@ut)[2:]
        return drift

    def step(self, dt, scheme):
        """The step map and the first half-kick, which gives the drift's field."""
        if scheme.wall == 'after':
            kick = self.plain_kick(dt/2)
            return self.full_cayley(dt)@kick@self.drift(dt, scheme)@kick, kick
        if scheme.wall == 'trapezoid':
            eye = np.eye(len(self.g))
            first = self.plain_kick(dt/2)
            first[np.ix_(self.x, self.x)] = eye+dt/2*self.g
            implicit = np.eye(self.N, dtype=complex)
            implicit[np.ix_(self.x, self.x)] = np.linalg.inv(eye-dt/2*self.g)
            first = self.with_force(first, dt, scheme)
            second = self.with_force(self.plain_kick(dt/2), dt, scheme)
            return implicit@second@self.drift(dt, scheme)@first, first
        kick = self.with_force(self.wall_kick(dt/2, dt, scheme), dt, scheme)
        return kick@self.drift(dt, scheme)@kick, kick

    def reflection(self, dt, scheme, window=(.2, .8)):
        strip = self.strip
        omega = strip.omega
        zeta = np.exp(-1j*omega*dt)
        step, kick = self.step(dt, scheme)
        y = slice(0, self.N-1)
        state = np.linalg.solve(zeta*np.eye(self.N-1)-step[y, y], step[y, self.drive])
        qh = (kick@np.r_[state, 1])[self.q]
        u = np.r_[self.pin, qh/self.mass[2:]]
        effective = 2*math.sin(omega*dt/2)/dt
        # The interior propagates at lambda_1, the smaller root of
        # lambda - dt^2 lambda^2/12 = effective^2 under the fourth-order step.
        lam = 6/(dt*dt)*(1-math.sqrt(1-dt*dt*effective**2/3)) if scheme.fourth else effective**2
        modes = strip.modes(math.sqrt(lam))
        nx = strip.nx
        columns = np.arange(max(4, math.ceil(window[0]*nx)), min(nx-3, math.floor(window[1]*nx))+1)
        design = np.column_stack([((z**(columns-nx))[:, None]*v[None, :]).ravel() for z, v in modes])
        observed = u[(6*columns[:, None]+np.arange(2)[None, :]).ravel()]
        amplitudes, *_ = np.linalg.lstsq(design, observed, rcond=None)
        return amplitudes[1]/amplitudes[0]

    def law(self):
        strip = self.strip
        y = admittance(-1j*strip.omega, strip.ky, self.kind)
        cosine = math.cos(math.radians(strip.angle))
        return (cosine-y)/(cosine+y)


def strip_bound(cell, h, samples=33, cells=16):
    """2/omega_max of the structured mesh at cell size h, over every Bloch ky."""
    largest = 0.
    for ky in np.linspace(0, math.pi/h, samples):
        k_cell, m_cell = cell.reduced(h, ky)
        n = 6*cells+2
        k = np.zeros((n, n), dtype=complex)
        mass = np.zeros(n)
        for e in range(cells):
            ids = np.r_[6*e+np.arange(2), 6*(e+1)+np.arange(2), 6*e+np.arange(2, 6)]
            k[np.ix_(ids, ids)] += k_cell
            mass[ids] += m_cell
        root = 1/np.sqrt(mass)
        largest = max(largest, np.linalg.eigvalsh(root[:, None]*k*root[None, :]).max())
    return 2/math.sqrt(largest)


def strip_at(cell, wavelength, angle, h=STRIP_CELL):
    return Strip(cell, wavelength, angle, wavelength/h)


def band(cell, kind, scheme, dt, h=STRIP_CELL):
    """|R|/|R_law| over the fixture's band: its channel's second mode, 1.6 to 2.2 Hz."""
    ratios = []
    for f in (1.6, 1.9, 2.2):
        model = Model(strip_at(cell, 1/f, math.degrees(math.asin(1/f)), h), kind)
        ratios.append(abs(model.reflection(dt, scheme))/abs(model.law()))
    return ratios


def solver_parity(cell):
    worst = 0.
    for kind in ('first', 'passive'):
        for angle in (0, 30, 60):
            strip = Strip(cell, 1., angle, 16)
            model = Model(strip, kind)
            for ratio in (.12, .015):
                closed = strip.solve(kind, ratio, scheme='coupled-kick')['reflection']
                worst = max(worst, abs(model.reflection(ratio*strip.h, LEAPFROG)-closed))
    check('step map against the closed-form leapfrog response', worst, 1e-11)


def reproduction(cell):
    bound = strip_bound(cell, STRIP_CELL)
    observation('strip bound at the fixture-like cell', cell=STRIP_CELL, bound=bound,
                fixture_step_over_it=1.2*FIXTURE_BOUND/bound)
    production = band(cell, 'passive', PRODUCTION, 1.2*FIXTURE_BOUND)
    leapfrog = band(cell, 'passive', LEAPFROG, .9*FIXTURE_BOUND)
    first = band(cell, 'first', PRODUCTION, 1.2*FIXTURE_BOUND)
    observation('fixture band reproduced', production_1_2=production, production_mean=float(np.mean(production)),
                fixture_production=1.098, leapfrog_0_9=leapfrog, leapfrog_mean=float(np.mean(leapfrog)),
                fixture_leapfrog=.911, first_order_production=first)
    check('production reproduces the second-order excess', np.mean(production), 1.05, lower=True)
    check('leapfrog reproduces the second-order deficit', np.mean(leapfrog), .95)
    return bound


def orders(cell):
    factors = (1.2, .6, .3, .15)
    schemes = (PRODUCTION, PADE, EXACT, SQRT_PADE, SQRT_CAYLEY, ACCELERATION, TAYLOR, TAYLOR_CAYLEY, TARGET, LEAPFROG, LEAPFROG_TARGET)
    expected = {PRODUCTION: 2, PADE: 4, EXACT: 4, SQRT_PADE: 4, SQRT_CAYLEY: 2, ACCELERATION: 2,
                TAYLOR: 4, TAYLOR_CAYLEY: 4, TARGET: 2, LEAPFROG: 2, LEAPFROG_TARGET: 2}
    for kind in ('first', 'passive'):
        for f in (1.6, 2.2):
            strip = strip_at(cell, 1/f, math.degrees(math.asin(1/f)))
            model = Model(strip, kind)
            semi = strip.solve(kind)['reflection']
            for scheme in schemes:
                errors = [abs(model.reflection(factor*FIXTURE_BOUND, scheme)-semi) for factor in factors]
                order = math.log2(errors[-2]/errors[-1])
                observation(f'temporal error {kind} {f} Hz {scheme.name}', steps=[factor*FIXTURE_BOUND for factor in factors],
                            errors=errors, orders=[math.log2(errors[i]/errors[i+1]) for i in range(3)])
                check(f'order {kind} {f} Hz {scheme.name}', abs(order-expected[scheme]), .15)
    # At normal incidence the law reflects nothing, so the step's error is the
    # whole reflection: the full-step Cayley's admittance is off by z^2/12
    # relative (sinh z against z(1 + z^2/12)), half that in R.
    model = Model(strip_at(cell, .45, 0), 'first')
    semi = model.strip.solve('first')['reflection']
    dt = .15*FIXTURE_BOUND
    z = model.strip.omega*dt
    check('normal incidence: full-step cayley + acceleration at z^2/24',
          abs(abs(model.reflection(dt, TARGET)-semi)/(z*z/24)-1), .05)
    check('normal incidence: leapfrog with full-step cayley at z^2/16',
          abs(abs(model.reflection(dt, LEAPFROG_TARGET)-semi)/(z*z/16)-1), .05)


def split_equivalence(cell):
    worst = trapezoid = 0.
    for kind in ('first', 'passive'):
        for wavelength, angle in ((.45, 0), (.625, 38.7), (1., 75)):
            model = Model(strip_at(cell, wavelength, angle), kind)
            for factor in (1.2, .3):
                a = model.reflection(factor*FIXTURE_BOUND, TARGET)
                b = model.reflection(factor*FIXTURE_BOUND, TARGET_SQRT)
                c = model.reflection(factor*FIXTURE_BOUND, TRAPEZOID)
                worst = max(worst, abs(a-b)/abs(a))
                trapezoid = max(trapezoid, abs(a-c)/abs(a))
    check('the wall after the kicks responds as the square-root halves', worst, 1e-9)
    check('the trapezoid split responds as the wall after the kicks', trapezoid, 1e-9)


def refinement(cell):
    f = 2.2
    for kind in ('first', 'passive'):
        errors = {scheme.name: [] for scheme in (PRODUCTION, PADE, TARGET)}
        for h in (.05, .025, .0125, .00625):
            strip = strip_at(cell, 1/f, math.degrees(math.asin(1/f)), h)
            model = Model(strip, kind)
            dt = 1.2*strip_bound(cell, strip.h, samples=9, cells=8)
            semi = strip.solve(kind)['reflection']
            for scheme in (PRODUCTION, PADE, TARGET):
                errors[scheme.name].append(abs(model.reflection(dt, scheme)-semi))
        observation(f'error at 1.2 of each mesh bound, {kind}, h = .05 to .00625', **errors)
        target = errors[TARGET.name]
        check(f'fixed-CFL refinement {kind}: full-step cayley + acceleration falls as h^2',
              min(target[i]/target[i+1] for i in range(3)), 3.5, lower=True)
        production = errors[PRODUCTION.name]
        check(f'fixed-CFL refinement {kind}: production falls as h',
              max(production[i]/production[i+1] for i in range(3)), 2.2)


def sweep(cell, bound):
    for label, dt in (('fixture step', 1.2*FIXTURE_BOUND), ('1.2 of the strip bound', 1.2*bound)):
        for kind in ('first', 'passive'):
            ratios, pade = [], []
            for wavelength in (.45, .625, 1.):
                for angle in (0, 15, 30, 45, 60, 75, 85):
                    model = Model(strip_at(cell, wavelength, angle), kind)
                    semi = model.strip.solve(kind)['reflection']
                    production = abs(model.reflection(dt, PRODUCTION)-semi)
                    ratios.append(abs(model.reflection(dt, TARGET)-semi)/production)
                    pade.append(abs(model.reflection(dt, PADE)-semi)/production)
            observation(f'sweep {label} {kind}: error over production', target_worst=max(ratios),
                        target_median=float(np.median(ratios)), pade_worst=max(pade), pade_best=min(pade))
            check(f'sweep {label} {kind}: full-step cayley + acceleration at most 0.4 of production', max(ratios), .4)
    target = band(cell, 'passive', TARGET, 1.2*FIXTURE_BOUND)
    pade = band(cell, 'passive', PADE, 1.2*FIXTURE_BOUND)
    observation('fixture band, second order', target=target, target_mean=float(np.mean(target)),
                pade=pade, pade_mean=float(np.mean(pade)))
    check('fixture band, second order, within 1% of its law', abs(np.mean(target)-1), .01)


class Box:
    """The 2D coarse mesh of boundary.py behind second-order walls, normalized."""
    def __init__(self, mesh):
        self.mesh = mesh
        self.boundary = Boundary(mesh)
        _, self.c, _ = self.boundary.normalized_generator()
        self.n = mesh.n
        self.nv = len(self.c)
        self.g = self.boundary.g
        self.nb = self.boundary.n
        self.na = len(self.g)-self.nb
        self.total = self.n+self.nv+self.na
        self.trace = self.boundary.ids
        self.qi = np.arange(self.n)
        self.bi = self.n+np.arange(self.nv)
        self.zi = self.n+self.nv+np.arange(self.na)
        self.xi = np.r_[self.trace, self.zi]
        self.ktk = self.c.T@self.c

    def step(self, dt, scheme, gp=0., gc=0.):
        n, nv, g, total = self.n, self.nv, self.g, self.total
        qi, bi, xi, trace = self.qi, self.bi, self.xi, self.trace
        tau = dt/2
        eye = np.eye(len(g))
        ut = np.zeros((n, total)); ut[qi, qi] = 1
        if scheme.fourth:
            ut[:, qi] -= dt*dt/12*self.ktk
        if scheme.wall_acceleration:
            x = np.zeros((len(g), total)); x[np.arange(len(g)), xi] = 1
            force = np.zeros((self.nb, total)); force[:, bi] = self.c.T[trace]
            force[:, qi] += dt/2*self.ktk[trace]
            rate = g@x; rate[:self.nb] -= force
            ut[trace] += scheme.wall_acceleration*dt*dt*(g@rate)[:self.nb]
        drift = np.eye(total)
        drift[np.ix_(bi, np.arange(total))] += dt*self.c@ut
        kick = np.eye(total)
        kick[np.ix_(qi, bi)] = -tau*self.c.T
        if scheme.wall == 'trapezoid':
            first = kick.copy()
            first[np.ix_(xi, xi)] = eye+dt*g/2
            implicit = np.eye(total)
            implicit[np.ix_(xi, xi)] = np.linalg.inv(eye-dt*g/2)
            step = implicit@kick@drift@first
        elif scheme.wall == 'after':
            wall = np.eye(total)
            wall[np.ix_(xi, xi)] = np.linalg.solve(eye-dt*g/2, eye+dt*g/2)
            step = wall@kick@drift@kick
        else:
            resolve = np.linalg.inv(eye-tau*g/2)
            kick[np.ix_(xi, xi)] = resolve@(eye+tau*g/2)
            force = np.vstack([self.c.T[trace], np.zeros((self.na, nv))])
            kick[np.ix_(xi, bi)] = -tau*resolve@force
            step = kick@drift@kick
        rates = np.r_[np.broadcast_to(gp, (n,)), np.broadcast_to(gc, self.mesh.qx.shape).ravel(), np.zeros(self.na)]
        decay = np.exp(-rates*dt/2)
        return decay[:, None]*step*decay[None, :]

    def run(self, scheme, cfl, start, times):
        dt = cfl*self.mesh.dt
        steps = math.ceil(times[-1]/dt); dt = times[-1]/steps
        step = self.step(dt, scheme)
        state = start.copy(); e0 = state@state
        out, done = [], 0
        for t in times:
            while done < round(t/dt):
                state = step@state; done += 1
            out.append(state@state/e0)
        return out


def box_tests(mesh):
    box = Box(mesh)
    cases = [('lossless', 0., 0.), ('both loss', .4, .9), ('old loss counterexample', 0., 4.),
             ('spatial loss', .4*(mesh.x[:, 0] > 0), np.broadcast_to(4.*(mesh.qx[..., 0] > 0)[..., None], mesh.qx.shape))]
    initial = np.r_[np.sqrt(mesh.m)*np.cos(np.pi*mesh.x[:, 0]/2),
                    (np.sqrt(mesh.w)[..., None]*mesh.C(.03*np.sin(np.pi*mesh.x[:, 1]))).ravel(),
                    np.zeros(box.na)]
    for scheme in (PRODUCTION, TARGET, TRAPEZOID):
        for name, gp, gc in cases:
            excess = max(max(abs(np.linalg.eigvals(box.step(cfl*mesh.dt, scheme, gp, gc))))-1
                         for cfl in (.2, .5, .9, 1., 1.1, 1.2, 1.22))
            check(f'2D spectral excess {scheme.name} {name}, 0.2 to 1.22 of the bound', excess, 1e-10)
            steps = math.ceil(100/(1.2*mesh.dt)); step = box.step(100/steps, scheme, gp, gc)
            state = initial.copy(); energy0 = state@state; peak = energy0
            for _ in range(steps):
                state = step@state
                peak = max(peak, state@state)
            check(f'2D long-run peak {scheme.name} {name}', peak/energy0, 1.01)
    # A pulse from the interior, nothing at the walls: the energy still in the
    # box as its wave leaves, against a step of 0.075 of the bound.
    r2 = (mesh.x[:, 0]-.2)**2+(mesh.x[:, 1]+.1)**2
    pulse = np.r_[np.sqrt(mesh.m)*np.exp(-r2/.08), np.zeros(box.nv+box.na)]
    times = (10., 30.)
    reference = box.run(TARGET, .075, pulse, times)
    reference_production = box.run(PRODUCTION, .075, pulse, times)
    reference_trapezoid = box.run(TRAPEZOID, .075, pulse, times)
    observation('2D pulse: energy in the box at the small step', target=reference, production=reference_production,
                trapezoid=reference_trapezoid)
    for scheme, ref in ((PRODUCTION, reference_production), (TARGET, reference), (TRAPEZOID, reference_trapezoid)):
        at_step = box.run(scheme, 1.2, pulse, times)
        errors = [a/b-1 for a, b in zip(at_step, ref)]
        observation(f'2D pulse at 1.2 of the bound, {scheme.name}', energy=at_step, relative_to_small_step=errors)
        if scheme is not PRODUCTION:
            check(f'2D pulse: {scheme.name} within 10% of its small step', max(map(abs, errors)), .1)
    # The caveat: data that starts with a normal field at the wall. The raw
    # state means something else at the trace under each step, and the second
    # order wall's static memory keeps a different share of it.
    residue = {}
    for scheme in (PRODUCTION, TARGET, TRAPEZOID):
        residue[scheme.name] = {str(cfl): box.run(scheme, cfl, initial, (100.,))[0] for cfl in (1.2, .6, .3, .075)}
    observation('2D data with a field at the wall: energy at t = 100', **residue)
    # The trapezoid split stores the average of its two drift instants, as
    # the interior does, and keeps less of a field started at the wall than
    # the wall-after split; the rest is the stiff modes' start, which the
    # full-step bilinear map rings on.
    check('2D data at the wall: the trapezoid split keeps less than the wall-after split at 1.2',
          residue[TRAPEZOID.name]['1.2']/residue[TARGET.name]['1.2'], .7)


def kick_form(cell):
    """The kick form's linear limit: F~ in the kicks, the plain field in the drift."""
    factors = (1.2, .6, .3, .15)
    for kind in ('first', 'passive'):
        for wavelength, angle in ((.45, 0), (1/2.2, math.degrees(math.asin(1/2.2))), (1., 75)):
            model = Model(strip_at(cell, wavelength, angle), kind)
            semi = model.strip.solve(kind)['reflection']
            errors = {}
            for scheme in (PRODUCTION, KICK_PRODUCTION, KICK_PLAIN, KICK_TRAPEZOID, TRAPEZOID):
                errors[scheme.name] = [abs(model.reflection(f*FIXTURE_BOUND, scheme)-semi) for f in factors]
            observation(f'kick form {kind} wavelength={wavelength:.3f} angle={angle:.1f}', **errors)
            same = max(abs(a-b)/b for a, b in zip(errors[KICK_PRODUCTION.name], errors[PRODUCTION.name]))
            check(f'kick form {kind} {wavelength:.3f} {angle:.0f}: production wall reflects as the drift form', same, 1e-6)
            check(f'kick form {kind} {wavelength:.3f} {angle:.0f}: without the wall rate worse than production',
                  errors[KICK_PLAIN.name][0]/errors[PRODUCTION.name][0], 1., lower=True)
            check(f'kick form {kind} {wavelength:.3f} {angle:.0f}: with the wall rate at most 0.6 of production',
                  errors[KICK_TRAPEZOID.name][0]/errors[PRODUCTION.name][0], .6)
    model = Model(strip_at(cell, .45, 0), 'first')
    semi = model.strip.solve('first')['reflection']
    dt = .15*FIXTURE_BOUND
    z = model.strip.omega*dt
    check('normal incidence: kick form with the wall rate at z^2/24',
          abs(abs(model.reflection(dt, KICK_TRAPEZOID)-semi)/(z*z/24)-1), .1)


def main():
    start = time.perf_counter()
    with open(sys.argv[1]) as stream:
        data = json.load(stream)
    mesh = Mesh(data['coarse'])
    cell = Cell(mesh)
    solver_parity(cell)
    bound = reproduction(cell)
    orders(cell)
    split_equivalence(cell)
    refinement(cell)
    sweep(cell, bound)
    kick_form(cell)
    box_tests(mesh)
    for result in RESULTS:
        if 'passed' in result:
            result['passed'] = bool(result['passed'])
    failed = [r['name'] for r in RESULTS if r.get('passed') is False]

    def encode(value):
        if isinstance(value, complex):
            return dict(real=value.real, imag=value.imag)
        return value.item()
    print(json.dumps(dict(baseline='beca47e0d05c9b24945ef632b00c5cc16f80c8a0',
                          summary=dict(passed=sum(r.get('passed') is True for r in RESULTS), failed=len(failed),
                                       failed_cases=failed, seconds=time.perf_counter()-start),
                          results=RESULTS), indent=2, default=encode))
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    main()
