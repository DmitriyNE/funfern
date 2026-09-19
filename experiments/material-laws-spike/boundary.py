"""Passive nonlocal auxiliary boundary; isolated f64 correctness experiment."""
import json
import math
import sys
import time
import numpy as np
from spike import Mesh, csr_dense, check, observation, relative, RESULTS


class Boundary:
    def __init__(self, mesh):
        self.mesh = mesh
        self.ids = np.flatnonzero(mesh.bd)
        self.n = len(self.ids)
        self.mass = mesh.m[self.ids]
        self.root_mass = np.sqrt(self.mass)
        self.root_damping = np.sqrt(mesh.bd[self.ids])
        a = csr_dense(mesh.n, mesh.data['aux_rows'], mesh.data['aux_cols'], mesh.data['aux'])
        a = a[np.ix_(self.ids, self.ids)]
        l = a/self.root_damping[:, None]/self.root_damping[None, :]
        ev, self.v = np.linalg.eigh(l)
        tolerance = 1e-12*max(1., max(ev))
        if min(ev) < -tolerance:
            raise ValueError('non-positive tangential stiffness')
        ev[abs(ev) < tolerance] = 0.
        # Three-state refinement cancels the unwanted cubic angular term.
        # d=sqrt(7/4)*sqrt(lambda)=sqrt(7/8)*c|k| on a flat unit side.
        self.d = np.sqrt(7./4)*np.sqrt(np.maximum(ev, 0.))
        self.sd = np.sqrt(self.d)
        # Input w=F*(Q_gamma/sqrt(m_gamma)); transpose applies force.
        self.f = self.v.T*(self.root_damping/self.root_mass)[None, :]
        f = self.f
        beta = np.sqrt(31./7)
        ell = np.array([0., 1-beta, 2*beta-4])
        h = np.zeros((3, 3)); h[0, 0] = 6./7
        for i in (1, 2):
            for j in (1, 2): h[i, j] = 2*ell[i]*ell[j]/(i+j)
        he, hv = np.linalg.eigh(h)
        root = (hv*np.sqrt(he))@hv.T
        inverse_root = (hv/np.sqrt(he))@hv.T
        self.h = h
        bv = root@np.ones(3)
        cv = inverse_root@np.array([6./7, -8./7, 2./7])
        av = -root@np.diag([0., 1., 2.])@inverse_root
        self.av, self.bv, self.cv = av, bv, cv
        self.loss_vector = ell@inverse_root
        bf = np.vstack([v*self.sd[:, None]*f for v in bv])
        fc = np.hstack([v*f.T*self.sd[None, :] for v in cv])
        aux = np.kron(av, np.diag(self.d))
        self.g = np.block([[-f.T@f, -fc], [bf, aux]])
        self.aux_count = 3
        self.maps = {}

    def midpoint_map(self, h):
        if h not in self.maps:
            eye = np.eye((1+self.aux_count)*self.n)
            self.maps[h] = np.linalg.solve(eye-h*self.g/2, eye+h*self.g/2)
        return self.maps[h]

    def apply(self, q, p, r, h):
        combined = np.concatenate([q[self.ids]/self.root_mass, p, r])
        updated = self.midpoint_map(h)@combined
        q = q.copy()
        q[self.ids] = self.root_mass*updated[:self.n]
        return q, updated[self.n:2*self.n], updated[2*self.n:]

    def apply_reduced(self, q, p, r, h):
        """Eliminate auxiliary midpoints: only an Nb-by-Nb SPD trace solve.

        Rebuilds/factors per call here for correctness comparison. A real core
        caches geometry/mass/dt-dependent matrices, not live-field calculations.
        """
        z0 = np.r_[p, r].reshape(self.aux_count, self.n).T
        inv = np.linalg.inv(np.eye(self.aux_count)[None, :, :]-h/2*self.d[:, None, None]*self.av[None, :, :])
        ib = inv@self.bv
        iz = np.einsum('nij,nj->ni', inv, z0)
        yh = 1+h/2*self.d*(ib@self.cv)
        history = self.sd*(iz@self.cv)
        t = self.v.T*self.root_damping[None, :]
        op = t.T@(yh[:, None]*t)
        u0 = q[self.ids]/self.mass
        u1 = np.linalg.solve(np.diag(self.mass)+h/2*op,
                             self.mass*u0-h/2*op@u0-h*t.T@history)
        wmid = t@((u0+u1)/2)
        zmid = iz+h/2*(self.sd*wmid)[:, None]*ib
        z1 = (2*zmid-z0).T.ravel()
        q1 = q.copy(); q1[self.ids] = self.mass*u1
        return q1, z1[:self.n], z1[self.n:]

    def step(self, state, dt, gp=0., gc=0., kind='passive'):
        m = self.mesh
        q, b, p, r = state
        q = q*np.exp(-np.asarray(gp)*dt/2)
        b = b*np.exp(-np.asarray(gc)*dt/2)
        if kind == 'passive':
            q, p, r = self.apply(q, p, r, dt/2)
        elif kind == 'first':
            q = q*(1-dt*m.bd/(4*m.m))/(1+dt*m.bd/(4*m.m))
        q, b = m.step(q, b, dt)
        if kind == 'passive':
            q, p, r = self.apply(q, p, r, dt/2)
        elif kind == 'first':
            q = q*(1-dt*m.bd/(4*m.m))/(1+dt*m.bd/(4*m.m))
        return (q*np.exp(-np.asarray(gp)*dt/2),
                b*np.exp(-np.asarray(gc)*dt/2), p, r)

    def energy(self, state):
        q, b, p, r = state
        return self.mesh.energy(q, b)+.5*(p@p+r@r)

    def normalized_generator(self, gp=0., gc=0.):
        m = self.mesh
        c = np.sqrt(np.repeat(m.w.ravel(), 2))[:, None]*m.dense_c()/np.sqrt(m.m)[None, :]
        nv = len(c); total = m.n+nv+self.aux_count*self.n
        g = np.zeros((total, total))
        g[:m.n, m.n:m.n+nv] = -c.T
        g[m.n:m.n+nv, :m.n] = c
        qloss = np.broadcast_to(gp, (m.n,))
        bloss = np.broadcast_to(gc, m.qx.shape).ravel()
        g[np.arange(m.n), np.arange(m.n)] -= qloss
        bi = np.arange(m.n, m.n+nv)
        g[bi, bi] -= bloss
        indices = np.r_[self.ids, np.arange(m.n+nv, total)]
        g[np.ix_(indices, indices)] += self.g
        return g, c, indices


def analytic_tests():
    def y(s, a):
        d = np.sqrt(7./8)*a
        return 1+(8./7)*d*d/(s*(s+d))-(1./7)*4*d*d/(s*(s+2*d))
    worst = 0.
    for a in np.geomspace(.001, 100., 17):
        for real in np.geomspace(1e-8, 100., 13):
            s = real+1j*np.r_[0., np.geomspace(1e-7, 1e4, 151)]
            worst = max(worst, float(max(-y(s, a).real)))
    check('candidate positive-real sampled admittance', worst, 1e-12)
    check('normal incidence exact admittance', abs(y(1+.4j, 0)-1), 1e-12)
    errors = []
    for theta in (.04, .02, .01):
        exact = np.cos(theta); approx = y(1j, np.sin(theta))
        errors.append(abs((exact-approx)/(exact+approx)))
    check('small-angle reflection quartic convergence', min(errors[0]/errors[1], errors[1]/errors[2]), 14., lower=True)
    for angle in (15, 30, 45, 60, 75, 85):
        theta = np.deg2rad(angle); exact = np.cos(theta)
        first = abs((exact-1)/(exact+1))
        approx = y(1j, np.sin(theta))
        refl = abs((exact-approx)/(exact+approx))
        check(f'planar reflection versus first order {angle}deg', refl, first)
        observation(f'planar reflection {angle}deg', candidate=float(refl), first=float(first))
    observation('accuracy contracts', candidate_small_angle_order=4,
                legacy_truncation_small_angle_order=4,
                low_frequency_evanescent_relative_bias=1-(6./7)*np.sqrt(7./8),
                note='Quartic small-angle reflection, different error coefficient from old truncation. No exact curved DtN claim.')


def spectrum_and_step_tests(boundary):
    b = boundary; m = b.mesh
    rng = np.random.default_rng(720)
    y = rng.normal(size=(1+b.aux_count)*b.n)
    w = b.f@y[:b.n]; aux = y[b.n:].reshape(b.aux_count, b.n)
    expected = -np.sum((w+b.sd*(b.loss_vector@aux))**2)
    check('boundary generator energy identity', abs(y@b.g@y-expected)/max(1, abs(expected)), 1e-11)
    cases = [('lossless', 0., 0.), ('both loss', .4, .9),
             ('old counterexample', 0., 4.),
             ('spatial loss', .4*(m.x[:, 0]>0), np.broadcast_to(4.*(m.qx[..., 0]>0)[..., None], m.qx.shape))]
    for name, gp, gc in cases:
        g, c, indices = b.normalized_generator(gp, gc)
        check(f'coupled generator energy {name}', max(np.linalg.eigvalsh((g+g.T)/2)), 1e-10)
        check(f'coupled spectral growth {name}', max(np.linalg.eigvals(g).real), 1e-8)
        for factor in (.2, .7, .95):
            dt = factor*m.dt; nv = len(c); total = len(g)
            half = np.eye(total)
            half[np.ix_(indices, indices)] = b.midpoint_map(dt/2)
            rates = np.r_[np.broadcast_to(gp, (m.n,)), np.broadcast_to(gc, m.qx.shape).ravel(), np.zeros(b.aux_count*b.n)]
            half_left = np.exp(-rates*dt/2)[:, None]*half
            half_right = half*np.exp(-rates*dt/2)[None, :]
            kick = np.eye(total); drift = np.eye(total)
            kick[:m.n, m.n:m.n+nv] = -dt*c.T/2
            drift[m.n:m.n+nv, :m.n] = dt*c
            step = half_left@kick@drift@kick@half_right
            rho = float(max(abs(np.linalg.eigvals(step))))
            check(f'actual split-step spectral excess {name} CFL={factor}', rho-1, 1e-10)
    for h in (.001, .1, 1., 10.):
        update = b.midpoint_map(h)
        check(f'boundary midpoint energy excess h={h}', max(np.linalg.eigvalsh(update.T@update-np.eye(len(update)))), 1e-11)
        q = rng.normal(size=m.n); p = rng.normal(size=b.n)
        r = rng.normal(size=(b.aux_count-1)*b.n)
        full = b.apply(q, p, r, h)
        reduced = b.apply_reduced(q, p, r, h)
        check(f'reduced trace solve parity h={h}', relative(np.concatenate(reduced), np.concatenate(full)), 1e-11)
    # Fixed-state convergence compares the actual split timestep, not RK4.
    initial = (m.m*np.cos(np.pi*m.x[:, 0]/2), m.C(.03*np.sin(np.pi*m.x[:, 1])), np.zeros(b.n), np.zeros((b.aux_count-1)*b.n))
    solutions = []
    for steps in (100, 200, 400):
        state = tuple(x.copy() for x in initial)
        for _ in range(steps): state = b.step(state, .5/steps, gp=.4, gc=.9)
        solutions.append(np.r_[state[0]/np.sqrt(m.m), (np.sqrt(m.w)[..., None]*state[1]).ravel(), state[2], state[3]])
    ratio = np.linalg.norm(solutions[0]-solutions[1])/np.linalg.norm(solutions[1]-solutions[2])
    check('actual split-step temporal convergence', ratio, 3.5, lower=True)
    for name, gp, gc in cases:
        state = tuple(x.copy() for x in initial); initial_e = b.energy(state)
        steps = math.ceil(100/(.7*m.dt)); dt = 100/steps
        max_e = initial_e
        for _ in range(steps):
            state = b.step(state, dt, gp=gp, gc=gc)
            max_e = max(max_e, b.energy(state))
        check(f'long-time total energy bound {name}', max_e/initial_e, 1.01)
        observation(f'long-time {name}', time=100., steps=steps, final_energy_ratio=b.energy(state)/initial_e)


def packet_tests(b):
    m = b.mesh
    for angle in (0., 30., 60., 75., 45.):
        n = np.array([np.cos(np.deg2rad(angle)), np.sin(np.deg2rad(angle))])
        origin = np.array([-.35, -.35 if angle == 45 else (-.30 if angle else 0.)])
        along = (m.x-origin)@n; across = (m.x-origin)@np.array([-n[1], n[0]])
        k = 2*np.pi/.65; width = .23
        envelope = np.exp(-.5*(along/width)**2-.5*(across/.32)**2)
        psi = envelope*np.sin(k*along)/k
        u = envelope*(along/(width*width*k)*np.sin(k*along)-np.cos(k*along))
        initial = (m.m*u, m.C(psi), np.zeros(b.n), np.zeros((b.aux_count-1)*b.n))
        e0 = b.energy(initial); results = {}
        steps = math.ceil(2.6/(.22*m.dt)); dt = 2.6/steps
        for kind in ('reflecting', 'first', 'passive'):
            state = tuple(x.copy() for x in initial); max_e = e0
            for _ in range(steps):
                state = b.step(state, dt, kind=kind)
                max_e = max(max_e, b.energy(state))
            bulk = m.energy(*state[:2]); aux = b.energy(state)-bulk
            results[kind] = bulk
            observation(f'packet {angle}deg {kind}', bulk_fraction=bulk/e0,
                        auxiliary_fraction=aux/e0, peak_total_fraction=max_e/e0, steps=steps)
        first = np.sqrt(results['first']/results['reflecting'])
        passive = np.sqrt(results['passive']/results['reflecting'])
        check(f'packet residual amplitude {angle}deg', passive, .15 if angle == 0 else first+.02)
        observation(f'packet comparison {angle}deg', first=float(first), candidate=float(passive),
                    note='45 degrees targets a square corner. Closed tangential graph is a passive corner closure, not exact exterior corner DtN.')


def nonlinear_boundary_tests(b):
    """Exact discrete-gradient primary energy, midpoint auxiliary state.

    This is a boundary substep oracle, not a full nonlinear production solver.
    It shows what replacing the linear trace solve actually requires.
    """
    n = b.n; mass = b.mass; root = b.root_mass
    a = b.g[n:, n:]
    bu = b.g[n:, :n]*root[None, :]
    qz = root[:, None]*b.g[:n, n:]
    qu = root[:, None]*b.g[:n, :n]*root[None, :]
    rng = np.random.default_rng(192)
    u0 = rng.normal(size=n)
    z0 = .1*rng.normal(size=b.aux_count*n)
    chi = .8*(1+np.arange(n)/n)
    q0 = mass*(u0+chi*u0**3)
    energy = lambda u, z: np.sum(mass*(.5*u*u+.75*chi*u**4))+.5*z@z
    e0 = energy(u0, z0)
    for dt in (.001, .1, 1., 10.):
        inv = np.linalg.inv(np.eye(len(a))-dt*a/2)
        effective = qu+dt/2*qz@inv@bu
        offset = qz@inv@z0
        def residual(u):
            numerator = (u+u0)*(.5+.75*chi*(u*u+u0*u0))
            denominator = 1+chi*(u*u+u*u0+u0*u0)
            v = numerator/denominator
            deriv_n = .5+.75*chi*(u*u+u0*u0)+1.5*chi*u*(u+u0)
            deriv_d = chi*(2*u+u0)
            dv = (deriv_n*denominator-numerator*deriv_d)/denominator**2
            value = mass*(u+chi*u**3)-q0-dt*(effective@v+offset)
            jac = np.diag(mass*(1+3*chi*u*u))-dt*effective*dv[None, :]
            return value, jac, v
        u = u0.copy()
        for iteration in range(40):
            value, jac, v = residual(u)
            if np.linalg.norm(value) < 1e-12*max(1., np.linalg.norm(q0)): break
            delta = np.linalg.solve(jac, -value)
            scale = 1.
            while scale > 2**-20 and np.linalg.norm(residual(u+scale*delta)[0]) >= np.linalg.norm(value):
                scale /= 2
            u += scale*delta
        value, _, v = residual(u)
        zmid = inv@(z0+dt/2*bu@v)
        z1 = 2*zmid-z0
        w = b.v.T@(b.root_damping*v)
        dissipation = dt*np.sum((w+b.sd*(b.loss_vector@zmid.reshape(b.aux_count, n)))**2)
        check(f'nonlinear trace solve residual dt={dt}', np.linalg.norm(value)/max(1., np.linalg.norm(q0)), 1e-11)
        check(f'nonlinear trace energy balance dt={dt}', abs(energy(u, z1)-e0+dissipation)/e0, 1e-10)
        check(f'nonlinear trace passive energy dt={dt}', energy(u, z1)/e0, 1.+1e-11)
        observation(f'nonlinear trace solve dt={dt}', newton_iterations=iteration,
                    note='Dense boundary-only Newton with analytic discrete gradient. No blanket nonlinear-material ban is mathematically required; this cost is not yet a production decision.')


def curved_tests(b):
    m = b.mesh
    u = np.exp(-((m.x[:, 0]-.5)**2+m.x[:, 1]**2)/.02)
    initial = (m.m*u, np.zeros_like(m.qx), np.zeros(b.n), np.zeros((b.aux_count-1)*b.n))
    initial_e = b.energy(initial)
    for name, gc in (('lossless', 0.), ('complementary loss', 4.)):
        state = tuple(x.copy() for x in initial)
        steps = math.ceil(20/(.7*m.dt)); dt = 20/steps
        peak = initial_e
        for _ in range(steps):
            state = b.step(state, dt, gc=gc)
            peak = max(peak, b.energy(state))
        check(f'annular total energy bound {name}', peak/initial_e, 1.01)
        check(f'annular final total energy {name}', b.energy(state)/initial_e, .5)
        observation(f'annular fixture {name}', nodes=m.n, elements=m.ne, boundary_nodes=b.n,
                    time=20., steps=steps, final_energy_fraction=b.energy(state)/initial_e,
                    note='Two disjoint polygonal circular traces, convex outer and concave inner. Passivity/decay smoke test only; not exact curved DtN or original ignored topology reproducer.')


def main():
    started = time.perf_counter()
    with open(sys.argv[1]) as stream: data = json.load(stream)
    analytic_tests()
    small = Boundary(Mesh(data['coarse']))
    spectrum_and_step_tests(small)
    nonlinear_boundary_tests(small)
    if 'curved' in data:
        curved_tests(Boundary(Mesh(data['curved'])))
    large = Boundary(Mesh(data['packet']))
    packet_tests(large)
    observation('boundary implementation cost', boundary_nodes=large.n,
                accepted_candidate_aux_f32_bytes=8*large.aux_count*large.n,
                dense_midpoint_map_f32_bytes=4*((1+large.aux_count)*large.n)**2,
                dense_two_half_step_multiply_adds=2*((1+large.aux_count)*large.n)**2,
                reduced_trace_factor_f32_bytes=4*large.n**2,
                modal_transform_f32_bytes=4*large.n**2,
                note='Full reference precomputes dense trace map. Reduced Nb-by-Nb SPD solve is parity-tested; it and transforms still have quadratic boundary cost. Interior stays explicit; no cheap GPU claim.')
    observation('scope', nonlinear='Continuous proof plus Kerr discrete-gradient boundary substep. Full nonlinear interior/boundary composition and generic constitutive discrete gradients not implemented.',
                curved='Annular graph passivity/decay if exported; no exact curved radiation or ignored topology reproducer claim.',
                elapsed_seconds=time.perf_counter()-started)
    for result in RESULTS:
        if 'passed' in result: result['passed'] = bool(result['passed'])
    failed = [r['name'] for r in RESULTS if r.get('passed') is False]
    print(json.dumps(dict(baseline='beca47e0d05c9b24945ef632b00c5cc16f80c8a0',
                          summary=dict(passed=sum(r.get('passed') is True for r in RESULTS), failed=len(failed), failed_cases=failed),
                          results=RESULTS), indent=2, default=lambda x: x.item()))
    sys.exit(1 if failed else 0)


if __name__ == '__main__': main()
