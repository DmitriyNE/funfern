"""Single flat-boundary Bloch-strip scattering, using exported actual P2e FEM.

Complex harmonic solutions are decomposed into forward/backward discrete modes.
Both continuous-time and actual KDK/half-midpoint/half-midpoint symbols are used.
No production, transient packet, or GPU performance claims are made here.
"""
import json
import math
import sys
import time
import numpy as np
from spike import Mesh, csr_dense, check, observation, relative, RESULTS

ACTIVE_SCHEME = 'original-split' if '--original-split' in sys.argv else 'coupled-kick'


def admittance(s, a, kind):
    if kind == 'first': return np.ones_like(a, dtype=complex)
    if kind == 'legacy': return 1+a*a/(2*s*s)
    d = np.sqrt(7./8)*a
    return 1+(8./7)*d*d/(s*(s+d))-(4./7)*d*d/(s*(s+2*d))


class Cell:
    def __init__(self, source):
        self.source = source
        self.h0 = .5
        ids = sorted(set(source.ids[:2].ravel()))
        self.xy = (source.x[ids]+1)/self.h0
        self.k = np.zeros((len(ids), len(ids)))
        self.mass = np.zeros(len(ids))
        lookup = {node: i for i, node in enumerate(ids)}
        for e in range(2):
            local = [lookup[node] for node in source.ids[e]]
            grad = source.c[e]
            self.k[np.ix_(local, local)] += np.einsum('qic,qjc,q->ij', grad, grad, source.w[e])
            self.mass[local] += source.w[e].sum()*np.array([1/20]*3+[2/15]*3+[9/20])

    def reduced(self, h, ky):
        keys = [(round(x, 11), round(0 if abs(y-1)<1e-9 else y, 11)) for x,y in self.xy]
        unique = set(keys)
        left = sorted(k for k in unique if k[0] == 0)
        right = sorted(k for k in unique if k[0] == 1)
        inside = sorted(unique-set(left)-set(right))
        order = left+right+inside
        assert len(left) == len(right) == 2 and len(inside) == 4
        p = np.zeros((len(keys), 8), dtype=complex)
        for i,key in enumerate(keys):
            p[i, order.index(key)] = np.exp(1j*ky*h) if abs(self.xy[i,1]-1)<1e-9 else 1
        k = p.conj().T@self.k@p
        mass = (abs(p)**2).T@self.mass*(h/self.h0)**2
        return k, mass


def boundary_matrices(h, ky):
    phase = np.exp(1j*ky*h)
    p = np.array([[1,0],[phase,0],[0,1]], dtype=complex)
    line = np.array([[7,1,-8],[1,7,-8],[-8,-8,16]])/(6*h)
    a = p.conj().T@line@p
    d = np.array([h/3, 2*h/3])
    return d, a


class Strip:
    def __init__(self, cell, wavelength, angle, ppw, length=2.):
        self.omega = 2*np.pi/wavelength
        self.angle = angle
        self.ky = self.omega*np.sin(np.deg2rad(angle))
        self.nx = math.ceil(length*ppw/wavelength)
        self.h = length/self.nx
        self.n = 6*self.nx+2
        self.k = np.zeros((self.n,self.n), dtype=complex)
        self.mass = np.zeros(self.n)
        self.cell_k, self.cell_mass = cell.reduced(self.h, self.ky)
        for e in range(self.nx):
            ids = np.r_[6*e+np.arange(2),6*(e+1)+np.arange(2),6*e+np.arange(2,6)]
            self.k[np.ix_(ids,ids)] += self.cell_k
            self.mass[ids] += self.cell_mass
        self.bd, self.ba = boundary_matrices(self.h,self.ky)
        root = np.sqrt(self.bd)
        ev, self.v = np.linalg.eigh(self.ba/root[:,None]/root[None,:])
        ev[abs(ev)<1e-12*max(1.,max(ev))] = 0.
        self.eigen = np.maximum(ev,0.)
        self.t = self.v.conj().T*root[None,:]

    def generator(self, kind):
        mass = self.mass[-2:]; root = np.sqrt(mass)
        if kind == 'first': return -np.diag(self.bd/mass).astype(complex)
        if kind == 'legacy':
            g = np.zeros((6,6), dtype=complex)
            g[:2,:2] = -np.diag(self.bd/mass)
            g[:2,4:] = -self.ba/root[:,None]
            g[2:4,:2] = np.diag(1/root)
            g[4:,2:4] = np.eye(2)
            return g
        d = np.sqrt(7./4)*np.sqrt(self.eigen); sd = np.sqrt(d)
        beta = np.sqrt(31./7); ell = np.array([0,1-beta,2*beta-4])
        storage = np.zeros((3,3)); storage[0,0] = 6./7
        for i in (1,2):
            for j in (1,2): storage[i,j] = 2*ell[i]*ell[j]/(i+j)
        he,hv = np.linalg.eigh(storage)
        hr = (hv*np.sqrt(he))@hv.T; hi = (hv/np.sqrt(he))@hv.T
        bv = hr@np.ones(3); cv = hi@np.array([6./7,-8./7,2./7])
        av = -hr@np.diag([0.,1.,2.])@hi
        f = self.t/root[None,:]
        bf = np.vstack([x*sd[:,None]*f for x in bv])
        fc = np.hstack([x*f.conj().T*sd[None,:] for x in cv])
        return np.block([[-f.conj().T@f,-fc],[bf,np.kron(av,np.diag(d))]])

    def modes(self, effective_omega):
        dynamic = self.cell_k-np.diag(self.cell_mass*effective_omega**2)
        s = dynamic[:4,:4]-dynamic[:4,4:]@np.linalg.solve(dynamic[4:,4:],dynamic[4:,:4])
        ll,lr,rl,rr = s[:2,:2],s[:2,2:],s[2:,:2],s[2:,2:]
        transfer = np.block([[-np.linalg.solve(lr,ll+rr),-np.linalg.solve(lr,rl)],
                             [np.eye(2),np.zeros((2,2))]])
        ev,vectors = np.linalg.eig(transfer)
        propagating = [i for i,z in enumerate(ev) if abs(abs(z)-1)<1e-6]
        positive = [i for i in propagating if np.angle(ev[i])>0]
        negative = [i for i in propagating if np.angle(ev[i])<0]
        if len(positive)!=1 or len(negative)!=1:
            raise ValueError(f'propagating branch unavailable: {ev}')
        out = []
        for i in (positive[0],negative[0]):
            v = vectors[2:,i]; v = v/v[0]
            out.append((ev[i],v))
        return out

    def solve(self, kind, step_ratio=0., window=(.2,.8), scheme=None):
        scheme = ACTIVE_SCHEME if scheme is None else scheme
        dt = step_ratio*self.h
        s = -1j*self.omega
        if dt == 0:
            effective_omega = self.omega
            y = admittance(s,np.sqrt(2*self.eigen),kind)
            correction = s*self.t.conj().T@(y[:,None]*self.t)
            half = None
        else:
            delta = np.expm1(s*dt); zeta = 1+delta
            effective_omega = 2*np.sin(self.omega*dt/2)/dt
            g = self.generator(kind); eye = np.eye(len(g))
            # Two boundary half-steps join between consecutive bulk stages.
            half = np.linalg.solve(eye-dt*g/4,eye+dt*g/4)
            scale = np.r_[np.sqrt(self.mass[-2:]),np.ones(len(g)-2)]
            half = scale[:,None]*half/scale[None,:]
            joined = half@half
            effective = joined[:2,:2].copy()
            if len(g)>2:
                effective += joined[:2,2:]@np.linalg.solve(zeta*np.eye(len(g)-2)-joined[2:,2:],joined[2:,:2])
            mass_term = 2*delta/(zeta*dt*dt)*np.linalg.solve(np.eye(2)+effective,zeta*np.eye(2)-effective)@np.diag(self.mass[-2:])
            if scheme == 'coupled-kick':
                physical_g = scale[:,None]*g/scale[None,:]
                j = delta*np.eye(len(g))-dt/2*(zeta+1)*physical_g+dt*dt/16*delta*(physical_g@physical_g)
                j_eff = j[:2,:2].copy()
                if len(g)>2: j_eff -= j[:2,2:]@np.linalg.solve(j[2:,2:],j[2:,:2])
                mass_term = delta/(zeta*dt*dt)*j_eff@np.diag(self.mass[-2:])
            correction = mass_term+np.diag(effective_omega**2*self.mass[-2:])
        matrix = self.k-np.diag(effective_omega**2*self.mass)
        matrix[-2:,-2:] += correction
        rhs = np.zeros(self.n,dtype=complex)
        rhs[:2] = [1,np.exp(1j*self.ky*self.h/2)]
        matrix[:2,:] = 0; matrix[:2,:2] = np.eye(2)
        u = np.linalg.solve(matrix,rhs)
        modes = self.modes(effective_omega)
        columns = np.arange(max(4,math.ceil(window[0]*self.nx)),min(self.nx-3,math.floor(window[1]*self.nx))+1)
        # Each mode contains two node samples; stack by physical column.
        design = np.column_stack([((lam**(columns-self.nx))[:,None]*v[None,:]).ravel() for lam,v in modes])
        observed = u[(6*columns[:,None]+np.arange(2)[None,:]).ravel()]
        amplitudes,_,_,_ = np.linalg.lstsq(design,observed,rcond=None)
        reflection = amplitudes[1]/amplitudes[0]
        fit = relative(design@amplitudes,observed)
        stage_error = 0.
        if dt:
            flux = dt/delta*(self.k@u)
            qin = self.mass*u+dt/2*flux
            qout = self.mass*u-dt/2*zeta*flux
            if len(joined)>2:
                aux = np.linalg.solve(zeta*np.eye(len(joined)-2)-joined[2:,2:],joined[2:,:2]@qout[-2:])
            else: aux = np.zeros(0,dtype=complex)
            initial_boundary = np.linalg.solve(half,np.r_[qin[-2:],aux])
            q0 = qin.copy(); q0[-2:] = initial_boundary[:2]
            start = half@initial_boundary
            q = q0.copy(); q[-2:] = start[:2]
            qh = q-dt/2*flux
            flux1 = flux+dt*(self.k@(qh/self.mass))
            q1 = qh-dt/2*flux1
            final_boundary = half@np.r_[q1[-2:],start[2:]]
            q1[-2:] = final_boundary[:2]
            stage_error = max(relative(q1[2:],zeta*q0[2:]),relative(final_boundary,zeta*initial_boundary),relative(flux1,zeta*flux))
            if scheme == 'coupled-kick':
                # Independently execute the forced midpoint kicks and drift.
                resolve = np.linalg.inv(np.eye(len(g))-dt/4*physical_g)
                auxhalf = -np.linalg.solve(j[2:,2:],j[2:,:2]@(self.mass[-2:]*u[-2:])) if len(g)>2 else np.zeros(0,dtype=complex)
                statehalf = np.r_[self.mass[-2:]*u[-2:],auxhalf]
                initial_boundary = np.linalg.solve(half,statehalf+dt/2*resolve@np.r_[flux[-2:],np.zeros(len(g)-2)])
                q0 = qin.copy(); q0[-2:] = initial_boundary[:2]
                half_boundary = half@initial_boundary-dt/2*resolve@np.r_[flux[-2:],np.zeros(len(g)-2)]
                qh = q0-dt/2*flux; qh[-2:] = half_boundary[:2]
                flux1 = flux+dt*(self.k@(qh/self.mass))
                final_boundary = half@half_boundary-dt/2*resolve@np.r_[flux1[-2:],np.zeros(len(g)-2)]
                q1 = qh-dt/2*flux1; q1[-2:] = final_boundary[:2]
                stage_error = max(relative(q1[2:],zeta*q0[2:]),relative(final_boundary,zeta*initial_boundary),relative(flux1,zeta*flux))
        exact_y = admittance(s,self.ky,kind)
        cosine = np.cos(np.deg2rad(self.angle))
        exact = (cosine-exact_y)/(cosine+exact_y)
        return dict(reflection=reflection,analytic=exact,error=abs(reflection-exact),fit=fit,
                    stage_error=stage_error,condition=np.linalg.cond(design),
                    normal_phase=[float(np.angle(z)/self.h) for z,_ in modes])


def assembly_tests(mesh):
    reconstructed = np.zeros((mesh.n,mesh.n))
    for e,ids in enumerate(mesh.ids):
        reconstructed[np.ix_(ids,ids)] += np.einsum('qic,qjc,q->ij',mesh.c[e],mesh.c[e],mesh.w[e])
    original = csr_dense(mesh.n,mesh.data['rows'],mesh.data['cols'],mesh.data['k'])
    check('local FEM assembly parity',relative(reconstructed,original),1e-11)
    assembled_a = np.zeros_like(original); assembled_d = np.zeros(mesh.n)
    for axis in (0,1):
        for side in (-1.,1.):
            ids = np.flatnonzero(abs(mesh.x[:,axis]-side)<1e-10)
            ids = ids[np.argsort(mesh.x[ids,1-axis])]
            for i in range(0,len(ids)-2,2):
                local = ids[[i,i+2,i+1]]
                length = np.linalg.norm(mesh.x[local[1]]-mesh.x[local[0]])
                assembled_a[np.ix_(local,local)] += np.array([[7,1,-8],[1,7,-8],[-8,-8,16]])/(6*length)
                assembled_d[local] += length*np.array([1,1,4])/6
    original_a = csr_dense(mesh.n,mesh.data['aux_rows'],mesh.data['aux_cols'],mesh.data['aux'])
    check('boundary line assembly parity',relative(assembled_a,original_a),1e-11)
    check('boundary damping assembly parity',relative(assembled_d,mesh.bd),1e-11)


def corrected_step_stability(mesh):
    # Small full 2D state, not the Bloch strip: ensure the accuracy correction
    # does not simply reintroduce the earlier linear-loss instability.
    from boundary import Boundary
    boundary = Boundary(mesh)
    base,c,ids = boundary.normalized_generator()
    n = mesh.n; nv = len(c); total = len(base)
    cases = [('lossless',0.,0.),('both loss',.4,.9),('old loss counterexample',0.,4.),
             ('spatial loss',.4*(mesh.x[:,0]>0),np.broadcast_to(4.*(mesh.qx[...,0]>0)[...,None],mesh.qx.shape))]
    initial = np.r_[np.sqrt(mesh.m)*np.cos(np.pi*mesh.x[:,0]/2),
                    (np.sqrt(mesh.w)[...,None]*mesh.C(.03*np.sin(np.pi*mesh.x[:,1]))).ravel(),
                    np.zeros(boundary.aux_count*boundary.n)]
    def make_step(dt,gp,gc):
        local_i = np.eye(len(boundary.g))
        resolve = np.linalg.inv(local_i-dt*boundary.g/4)
        kick = np.eye(total)
        kick[np.ix_(ids,ids)] = resolve@(local_i+dt*boundary.g/4)
        kick[:n,n:n+nv] = -dt*c.T/2
        local_force = np.vstack([c.T[boundary.ids],np.zeros((boundary.aux_count*boundary.n,nv))])
        kick[np.ix_(ids,np.arange(n,n+nv))] = -dt/2*resolve@local_force
        drift = np.eye(total); drift[n:n+nv,:n] = dt*c
        rates = np.r_[np.broadcast_to(gp,(n,)),np.broadcast_to(gc,mesh.qx.shape).ravel(),np.zeros(total-n-nv)]
        decay = np.exp(-rates*dt/2)
        return decay[:,None]*(kick@drift@kick)*decay[None,:]
    for name,gp,gc in cases:
        for cfl in (.2,.7,.95):
            step=make_step(cfl*mesh.dt,gp,gc)
            check(f'corrected full-core spectral excess {name} CFL={cfl}',max(abs(np.linalg.eigvals(step)))-1,1e-10)
        steps=math.ceil(100/(.7*mesh.dt)); step=make_step(100/steps,gp,gc)
        state=initial.copy(); energy0=state@state; peak=energy0
        for _ in range(steps):
            state=step@state
            peak=max(peak,state@state)
        check(f'corrected full-core long-run peak {name}',peak/energy0,1.01)
        observation(f'corrected full-core long run {name}',time=100.,steps=steps,final_energy_fraction=(state@state)/energy0)


def main():
    start = time.perf_counter()
    with open(sys.argv[1]) as stream: data=json.load(stream)
    mesh = Mesh(data['coarse']); assembly_tests(mesh); cell = Cell(mesh)
    rows = []
    # Preserve the original split's mesh-dependent impedance bias as a negative
    # control. Do not relax its original refinement criterion to call it good.
    split_errors=[]
    for ppw in (8,16,32):
        strip=Strip(cell,1.,0,ppw)
        result=strip.solve('first',.015,scheme='original-split')
        split_errors.append(result['error'])
        rows.append(dict(wavelength=1.,angle=0,ppw=ppw,kind='first',dt_over_h=.015,series='original-split',**result))
    observation('original split negative control',errors=split_errors,
                reduction=split_errors[0]/split_errors[-1],original_required_reduction=2.,
                rejected=True,note='Boundary-only midpoint split has a nonvanishing fixed-CFL reflection floor. Full original sweep is retained separately.')
    for wavelength in (.75,1.,1.5):
        for angle in (0,30,60,75,85):
            errors = {kind:[] for kind in ('first','legacy','passive')}
            finest = {}
            for ppw in (8,16,32):
                strip = Strip(cell,wavelength,angle,ppw)
                if wavelength==1. and angle==30 and ppw==8:
                    check('Bloch FEM Hermitian residual',relative(strip.k,strip.k.conj().T),1e-11)
                for kind in errors:
                    result = strip.solve(kind,step_ratio=.015)
                    errors[kind].append(result['error'])
                    rows.append(dict(wavelength=wavelength,angle=angle,ppw=ppw,cells=strip.nx,
                                     h=strip.h,dt_over_h=.015,kind=kind,scheme=ACTIVE_SCHEME,**result))
                    if ppw==32:
                        label=f'{kind} wavelength={wavelength} angle={angle}'
                        check(f'finest complex reflection error {label}',result['error'],.01)
                        check(f'finest modal fit {label}',result['fit'],1e-5)
                        check(f'KDK stage consistency {label}',result['stage_error'],1e-9)
                        finest[kind]=abs(result['reflection'])
            for kind,values in errors.items():
                if values[0]>1e-6:
                    check(f'mesh error reduction {kind} wavelength={wavelength} angle={angle}',values[0]/max(values[-1],1e-30),2.,lower=True)
            if angle==0: check(f'normal first-order reflection wavelength={wavelength}',finest['first'],.01)
            if angle>=60: check(f'grazing improvement wavelength={wavelength} angle={angle}',finest['passive'],finest['first'])
            observation(f'finest amplitudes wavelength={wavelength} angle={angle}',**finest)
    for angle in (30,75):
        strip = Strip(cell,1.,angle,16)
        for kind in ('first','legacy','passive'):
            semi = strip.solve(kind)
            temporal=[]
            for ratio in (.12,.06,.03,.015):
                result=strip.solve(kind,ratio)
                temporal.append(abs(result['reflection']-semi['reflection']))
                rows.append(dict(wavelength=1.,angle=angle,ppw=16,kind=kind,dt_over_h=ratio,series='time',**result))
            for i in range(3):
                if temporal[i+1]>1e-8:
                    check(f'time error reduction {kind} angle={angle} pair={i}',temporal[i]/temporal[i+1],3.5,lower=True)
            observation(f'temporal error sequence {kind} angle={angle}',errors=temporal)
    for angle in (0,60,85):
        original = Strip(cell,1.,angle,32,length=2.)
        extended = Strip(cell,1.,angle,32,length=3.)
        for kind in ('first','legacy','passive'):
            a=original.solve(kind,.015); b=extended.solve(kind,.015); c=original.solve(kind,.015,window=(.3,.7))
            check(f'strip length invariance {kind} angle={angle}',abs(a['reflection']-b['reflection']),1e-4)
            check(f'fitting window invariance {kind} angle={angle}',abs(a['reflection']-c['reflection']),1e-4)
    if ACTIVE_SCHEME == 'coupled-kick': corrected_step_stability(mesh)
    for result in RESULTS:
        if 'passed' in result: result['passed']=bool(result['passed'])
    failed=[r['name'] for r in RESULTS if r.get('passed') is False]
    def encode(value):
        if isinstance(value,complex): return dict(real=value.real,imag=value.imag)
        return value.item()
    print(json.dumps(dict(baseline='beca47e0d05c9b24945ef632b00c5cc16f80c8a0',scheme=ACTIVE_SCHEME,
                         summary=dict(passed=sum(r.get('passed') is True for r in RESULTS),failed=len(failed),failed_cases=failed,seconds=time.perf_counter()-start),
                         results=RESULTS,measurements=rows),indent=2,default=encode))
    sys.exit(1 if failed else 0)


if __name__=='__main__': main()
