"""NumPy f64 validation, using Rust-exported clean-baseline P2e data.

All files this program consumes are supplied by run.sh. Results go to stdout;
progress goes to stderr. Dense matrices are used only for small spectral oracles.
"""
import json
import math
import sys
import time
import numpy as np

RESULTS = []


def check(name, value, limit, *, lower=False, note=""):
    value = float(value)
    passed = math.isfinite(value) and (value >= limit if lower else value <= limit)
    RESULTS.append(dict(name=name, value=value, limit=limit, comparison=">=" if lower else "<=", passed=passed, note=note))
    print(f"{'PASS' if passed else 'FAIL'} {name}: {value:.6g} ({'>=' if lower else '<='} {limit})", file=sys.stderr, flush=True)


def observation(name, **values):
    RESULTS.append(dict(name=name, observation=values))
    print(f"INFO {name}: {values}", file=sys.stderr, flush=True)


def relative(a, b):
    return np.linalg.norm(np.asarray(a)-np.asarray(b))/max(np.linalg.norm(b), 1e-30)


def csr_dense(n, rows, cols, values):
    a = np.zeros((n, n))
    for i in range(n):
        a[i, np.asarray(cols[rows[i]:rows[i+1]], dtype=int)] = values[rows[i]:rows[i+1]]
    return a


class Mesh:
    def __init__(self, data):
        self.data = data
        self.x = np.array(data["nodes"])
        self.ids = np.array(data["elements"], dtype=int)
        self.tri = np.array(data["triangles"])
        self.c = np.array(data["c"])
        self.w = np.array(data["w"])
        self.qx = np.array(data["qpoints"])
        self.m = np.array(data["mass"])
        self.n = len(self.m)
        self.ne = len(self.ids)
        self.dt = data["dt_max"]
        self.bd = np.array(data["boundary_d"])
        self.rows = np.repeat(np.arange(self.n), np.diff(data["rows"]))
        self.cols = np.array(data["cols"], dtype=int)
        self.k = np.array(data["k"])
        self.arows = np.repeat(np.arange(self.n), np.diff(data["aux_rows"]))
        self.acols = np.array(data["aux_cols"], dtype=int)
        self.av = np.array(data["aux"])
        # Use local offsets so constants are annihilated to arithmetic precision.
        self.c = self.c.copy()
        self.c[:, :, 0, :] = -self.c[:, :, 1:, :].sum(axis=2)

    def C(self, u):
        local = u[self.ids]
        return np.einsum("eqic,ei->eqc", self.c, local-local[:, :1], optimize=False)

    def CT(self, b):
        local = np.einsum("eqic,eqc,eq->ei", self.c, b, self.w, optimize=False)
        return np.bincount(self.ids.ravel(), weights=local.ravel(), minlength=self.n)

    def K(self, u):
        return np.bincount(self.rows, weights=self.k*u[self.cols], minlength=self.n)

    def A(self, u):
        return np.bincount(self.arows, weights=self.av*u[self.acols], minlength=self.n)

    def dense_c(self):
        c = np.zeros((self.ne, 6, 2, self.n))
        for e in range(self.ne):
            for j in range(7):
                c[e, :, :, self.ids[e, j]] = self.c[e, :, j, :]
        return c.reshape(-1, self.n)

    def energy(self, q, b):
        return 0.5*np.sum(q*q/self.m)+0.5*np.sum(self.w[..., None]*b*b)

    def step(self, q, b, dt, sign=1, gp=0., gc=0.):
        # Lie loss then second-order conservative KDK; explicitly first-order
        # for noncommuting loss. Constant rates may be arrays per sample/node.
        q = q*np.exp(-np.asarray(gp)*dt)
        b = b*np.exp(-np.asarray(gc)*dt)
        qh = q-sign*dt/2*self.CT(b)
        bn = b+sign*dt*self.C(qh/self.m)
        qn = qh-sign*dt/2*self.CT(bn)
        return qn, bn


def linear_tests(m):
    rng = np.random.default_rng(18)
    psi = rng.normal(size=m.n)
    check("baseline stiffness equals C^T W C", relative(m.CT(m.C(psi)), m.K(psi)), 1e-10)
    check("positive mass integrates area", abs(m.m.sum()-4), 1e-10)
    check("constant primary exact gradient", np.max(np.abs(m.C(np.ones(m.n)))), 1e-10)
    q0 = m.m*np.cos(np.pi*(m.x[:, 0]+1)/2)
    p0 = .03*np.sin(np.pi*m.x[:, 1])
    dt = .13*m.dt
    for sign in (1, -1):
        q, b, p, qp = q0.copy(), sign*m.C(p0), p0.copy(), q0.copy()
        for _ in range(180):
            q, b = m.step(q, b, dt, sign)
            qh = qp-dt/2*m.K(p)
            p += dt*qh/m.m
            qp = qh-dt/2*m.K(p)
        check(f"potential/direct equivalence sign {sign}", max(relative(q, qp), relative(b, sign*m.C(p))), 1e-10)
        check(f"integrated Q conservation sign {sign}", abs(q.sum()-q0.sum()), 1e-10)
    # Tensor inverse must rotate with C; compare to original G tensor assembly.
    angle=.63
    r=np.array([[0.,-1.],[1.,0.]])
    rot=np.array([[np.cos(angle),-np.sin(angle)],[np.sin(angle),np.cos(angle)]])
    a=rot@np.diag([2.4,1/2.4])@rot.T
    tinv=r@a@r.T
    g=m.C(psi)@r
    e1=np.sum(m.w*np.einsum("...i,ij,...j->...",g,a,g))
    e2=np.dot(psi,m.CT(m.C(psi)@tinv.T))
    check("rotated anisotropic energy/operator identity", abs(e1-e2)/abs(e1), 1e-10)
    c=m.dense_c()
    k=c.T@(np.repeat(m.w.ravel(),2)[:,None]*c)
    d=np.sqrt(m.m)
    eig, vec=np.linalg.eigh(k/d[:,None]/d[None,:])
    mode=vec[:,1]/d; omega=np.sqrt(eig[1]); final=.7
    errors=[]
    for steps in (80,160,320):
        q=m.m*mode; b=np.zeros_like(m.qx); e0=m.energy(q,b); excursion=0.
        for _ in range(steps):
            q,b=m.step(q,b,final/steps)
            excursion=max(excursion,abs(m.energy(q,b)/e0-1))
        errors.append(relative(q/m.m,mode*np.cos(omega*final)))
        check(f"conservative energy excursion {steps}",excursion,.005)
    check("conservative temporal error ratio",min(errors[0]/errors[1],errors[1]/errors[2]),3.5,lower=True)
    # All extra kernel vectors are energy-visible, force-invisible stationary states.
    weighted=np.sqrt(np.repeat(m.w.ravel(),2))[:,None]*c
    u,s,_=np.linalg.svd(weighted,full_matrices=True)
    rank=np.count_nonzero(s>1e-10)
    z=(u[:,rank]/np.sqrt(np.repeat(m.w.ravel(),2))).reshape(m.qx.shape)
    check("extra vector null mode force",np.linalg.norm(m.CT(z)),1e-10)
    observation("state dimensions",nodes=m.n,vector_dofs=len(z.ravel()),gradient_rank=int(rank),extra_stationary_dofs=int(len(z.ravel())-rank))
    # A constant gamma does preserve the image; spatial gamma generally does not.
    initial=m.C(np.cos(np.pi*m.x[:,0]/2)*np.cos(np.pi*m.x[:,1]/2))
    loss=initial*np.exp(-.2*(.3+.7*(m.qx[...,0]>0)))[...,None]
    wb=np.sqrt(np.repeat(m.w.ravel(),2))*loss.ravel()
    projected=u[:,:rank]@(u[:,:rank].T@wb)
    fraction=np.linalg.norm(wb-projected)/np.linalg.norm(wb)
    observation("spatial loss leaves potential image",weighted_relative_nonpotential_flux=float(fraction))
    # A projection to C psi would erase this state; this is NOT a rejection of
    # the direct scheme. Electric charge/longitudinal static response can be real.
    return eig,vec,k,c


def law_value(u, kind, chi):
    if kind=="kerr": return u*(1+chi*u*u)
    if kind=="sat": return u*(1+chi*u*u/(1+u*u))
    return u


def law_inverse(q, kind, chi):
    # Positive Kerr and the chosen saturable fixtures have tangent >= 1.
    lo=np.minimum(q,0.); hi=np.maximum(q,0.)
    for _ in range(55):
        mid=(lo+hi)/2; f=law_value(mid,kind,chi)
        lo=np.where(f<q,mid,lo); hi=np.where(f>=q,mid,hi)
    return (lo+hi)/2


def constitutive_energy(q, kind, chi):
    u=law_inverse(q,kind,chi)
    if kind=="kerr": primitive=.5*u*u+.25*chi*u**4
    elif kind=="sat": primitive=.5*u*u+.5*chi*(u*u-np.log1p(u*u))
    else: primitive=.5*u*u
    return u*q-primitive


def nonlinear_tests(m):
    for kind in ("kerr","sat"):
        chi=.8
        samples=np.linspace(-4,4,121)
        q=law_value(samples,kind,chi)
        check(f"{kind} scalar inverse",relative(law_value(law_inverse(q,kind,chi),kind,chi),q),1e-11)
        # Vector inverse uses magnitude, then orientation; zero handled directly.
        vec=np.stack([q,.4*q],axis=-1)
        norm=np.linalg.norm(vec,axis=-1)
        radius=law_inverse(norm,kind,chi)
        field=vec*np.divide(radius,norm,out=np.zeros_like(radius),where=norm>0)[:,None]
        actual=field*np.divide(law_value(radius,kind,chi),radius,out=np.ones_like(radius),where=radius>0)[:,None]
        check(f"{kind} vector inverse",relative(actual,vec),1e-11)
        new=q*np.exp(-.1/(1+samples*samples))
        check(f"{kind} passive loss energy increment",np.max(constitutive_energy(new,kind,chi)-constitutive_energy(q,kind,chi)),1e-12)
        # Primary nonlinear map + complementary inverse, two actual laws.
        q0=m.m*law_value(.25*np.cos(np.pi*m.x[:,0]/2),kind,chi)
        b0=m.C(.04*np.cos(np.pi*m.x[:,1]/2))
        def b_inv(b):
            norm=np.linalg.norm(b,axis=-1)
            r=law_inverse(norm,kind,chi)
            return b*np.divide(r,norm,out=np.zeros_like(r),where=norm>0)[...,None]
        def energy(q,b):
            return np.sum(m.m*constitutive_energy(q/m.m,kind,chi))+np.sum(m.w*constitutive_energy(np.linalg.norm(b,axis=-1),kind,chi))
        for sign in (1,-1):
            q=q0.copy(); b=sign*b0.copy(); e0=energy(q,b)
            for _ in range(60):
                dt=.1*m.dt
                qh=q-sign*.5*dt*m.CT(b_inv(b))
                b+=sign*dt*m.C(law_inverse(qh/m.m,kind,chi))
                q=qh-sign*.5*dt*m.CT(b_inv(b))
            check(f"{kind} both-side nonlinear energy sign {sign}",abs(energy(q,b)/e0-1),.005)
    # Constitutive junction P=u+0.8u^3, rates 0.2 and 1.4, gamma is not constant.
    def junction_flux(u): return u+.8*u**3
    def inv(q): return law_inverse(np.array(q),"kerr",.8)
    def decay(dt):
        q=float(junction_flux(1.2))
        for _ in range(round(1/dt)):
            u=float(inv(q)); loss=.2*u+1.4*.8*u**3
            q*=math.exp(-dt*loss/q)
        return q
    ref=decay(1/4096)
    errors=[abs(decay(dt)-ref)/ref for dt in (1/100,1/200)]
    check("mixed nonlinear frozen loss relative error",errors[0],.02)
    check("mixed nonlinear frozen loss convergence ratio",errors[0]/errors[1],1.7,lower=True)
    observation("nonlinear mixed effective rates",at_small_field=(.2*.1+1.4*.8*.1**3)/junction_flux(.1),at_large_field=(.2*1.2+1.4*.8*1.2**3)/junction_flux(1.2))
    # Physical channel mapping: swapped material data + eta sign gives duality.
    q=m.m*np.cos(np.pi*m.x[:,0]); b=m.C(.1*np.sin(np.pi*m.x[:,1]))
    a=(q.copy(),b.copy()); dual=(q.copy(),-b.copy())
    for _ in range(120):
        a=m.step(*a,.1*m.dt,sign=1,gp=.4,gc=.9)
        dual=m.step(*dual,.1*m.dt,sign=-1,gp=.4,gc=.9)
    check("electric/magnetic swapped duality",max(relative(a[0],dual[0]),relative(a[1],-dual[1])),1e-10)
    for gp,gc in ((.6,0.),(0.,.6),(.6,.9)):
        q0=q.copy(); b0=b.copy(); e0=m.energy(q0,b0)
        for _ in range(240): q0,b0=m.step(q0,b0,.1*m.dt,gp=gp,gc=gc)
        check(f"passive two-channel final energy {gp},{gc}",m.energy(q0,b0)/e0,1.)


def filter_tests(m,eig,vec,k,c):
    # Paired polynomial spectral filter on wave-coupled q and b.
    # q -> q-a K M^-1 K M^-1 q/lambda^2
    # b -> b-a C M^-1 K M^-1 C^T W b/lambda^2.
    ceiling=eig[-1]; strength=.8
    def filt(q,b):
        return (q-strength*m.CT(m.C(m.CT(m.C(q/m.m))/m.m))/ceiling**2,
                b-strength*m.C(m.CT(m.C(m.CT(b)/m.m))/m.m)/ceiling**2)
    for index,label in ((1,"resolved"),(-1,"ceiling")):
        v=vec[:,index]/np.sqrt(m.m)
        q=m.m*v; b=m.C(v)/np.sqrt(eig[index]); qn,bn=filt(q,b)
        attenuation=max(relative(qn,q),relative(bn,b))
        if index==1:
            check("filter resolved frequency fraction",np.sqrt(eig[index]/ceiling),.1)
            check("filter resolved attenuation",attenuation,.001)
        else: check("filter ceiling attenuation",attenuation,.5,lower=True)
        check(f"filter Q total {label}",abs(qn.sum()-q.sum()),1e-10)
    q=m.m*.7; b=np.zeros_like(m.qx); qn,bn=filt(q,b)
    check("filter constant primary identity",np.max(abs(qn-q)),1e-10)
    observation("filter limitation",policy="Preserves ker(C^T W) vector state. It filters coupled waves, not arbitrary charge/static modes; material edits and remap must not inject unresolved null-space artifacts.")


def boundary_spectrum(m,k):
    # Original scalar second-order outgoing: M u''+D u'+K u+A z=0,z'=u.
    # Canonical direct state needs trace history p'=u,r'=p with force -A r.
    a=csr_dense(m.n,m.data['aux_rows'],m.data['aux_cols'],m.data['aux'])
    n=m.n; z=np.zeros((n,n)); eye=np.eye(n)
    first=np.block([[z,eye],[-k/m.m[:,None],-np.diag(m.bd/m.m)]])
    second=np.block([[z,eye,z],[-k/m.m[:,None],-np.diag(m.bd/m.m),-a/m.m[:,None]],[eye,z,z]])
    ev1=np.linalg.eigvals(first); ev2=np.linalg.eigvals(second)
    check("first-order outgoing spectral growth",max(ev1.real),1e-8)
    check("original second-order outgoing spectral growth",max(ev2.real),1e-8)
    observation("second-order outgoing modes",unstable_count=int(np.count_nonzero(ev2.real>1e-8)),max_growth=float(max(ev2.real)),max_frequency=float(max(abs(ev2.imag))))
    rng=np.random.default_rng(3)
    q=rng.normal(size=n); b=rng.normal(size=m.qx.shape); p=rng.normal(size=n); r=rng.normal(size=n)
    u=q/m.m
    v=(-m.CT(b)-m.bd*u-m.A(r))/m.m
    canonical_accel=(-m.CT(m.C(u))-m.bd*v-m.A(p))/m.m
    old_accel=(-m.K(u)-m.bd*v-m.A(p))/m.m
    check("outgoing auxiliary scalar/canonical derivative",relative(canonical_accel,old_accel),1e-10)
    # Passivity counterexample of truncated admittance D + A/s^2.
    eig_a=np.linalg.eigvalsh(a)
    check("boundary tangential operator PSD",max(0.,-min(eig_a)),1e-10)
    observation("second-order admittance limitation",equation="Y(s)=D+A/s^2; Y(i omega)=D-A/omega^2 can be negative. Energy of bulk alone need not decrease; spectrum/reflection must gate use.")
    # Now test the actual direct-flux state with complementary loss and its
    # proposed independent boundary memories. The scalar cubic alone cannot
    # certify this extended system: loss breaks former state correlations.
    c=m.dense_c(); nw=c.shape[0]; w=np.repeat(m.w.ravel(),2)
    ae,av=np.linalg.eigh(a)
    active=ae>1e-10
    edge_gradient=np.sqrt(ae[active])[:,None]*av[:,active].T
    # p/r only on boundary: no unused interior auxiliary eigenvalues.
    boundary=np.flatnonzero(m.bd); nb=len(boundary)
    trace=np.eye(n)[boundary]; ab=a[:,boundary]
    for gp,gc in ((0.,0.),(.4,.9),(0.,4.)):
        total=n+nw+2*nb; system=np.zeros((total,total))
        qi=slice(0,n); bi=slice(n,n+nw); pi=slice(n+nw,n+nw+nb); ri=slice(n+nw+nb,total)
        system[qi,qi]=-np.diag(m.bd/m.m+gp)
        system[qi,bi]=-c.T*w
        system[qi,ri]=-ab
        system[bi,qi]=c/m.m[None,:]
        system[bi,bi]=-gc*np.eye(nw)
        system[pi,qi]=trace/m.m[None,:]
        system[ri,pi]=np.eye(nb)
        ev,evec=np.linalg.eig(system); winner=int(np.argmax(ev.real)); growth=float(ev[winner].real)
        check(f"direct second-order boundary growth gp={gp} gc={gc}",growth,1e-8)
        if growth>1e-8:
            lam=ev[winner]; mode=evec[:,winner]
            residual=np.linalg.norm(system@mode-lam*mode)/np.linalg.norm(system)
            observation("boundary eigenmode diagnostic",gp=gp,gc=gc,real=float(lam.real),imag=float(lam.imag),normalized_residual=float(residual),note="Near-zero roots can split numerically around redundant/Jordan memory modes; a tiny real root alone is not evidence of exponential instability.")
            if abs(lam)>1e-5:
                # Lift this very eigenmode to the redundant potential+memory
                # representation. psi'=u and z'=-gc b imply identical b'.
                potential=(mode[qi]/m.m)/lam
                memory=mode[bi]-c@potential
                check("unstable eigenmode lifts to potential plus loss memory",np.linalg.norm(lam*memory+gc*mode[bi]),1e-10)
                observation("boundary failure representation independence",gp=gp,gc=gc,growth=growth,reason="b=C psi+z, psi_dot=M^-1 Q, z_dot=-gc b yields identical b_dot. Retaining the same outgoing histories cannot remove this instability by changing representation.")
        # Passive first-order boundary alternative, with no p/r histories.
        first_direct=system[:n+nw,:n+nw]
        check(f"direct first-order boundary growth gp={gp} gc={gc}",max(np.linalg.eigvals(first_direct).real),1e-8)
        # Equivalent trace-gradient histories discard A's irrelevant constants.
        # Dense factorization is a spectral oracle; production uses local edge
        # derivative/quadrature factors, not a global eigensolve.
        nt=len(edge_gradient); reduced=np.zeros((n+nw+2*nt,n+nw+2*nt))
        reduced[:n+nw,:n+nw]=first_direct
        reduced[:n,n+nw+nt:]=-edge_gradient.T
        reduced[n+nw:n+nw+nt,:n]=edge_gradient/m.m[None,:]
        reduced[n+nw+nt:,n+nw:n+nw+nt]=np.eye(nt)
        check(f"factored second-order boundary growth gp={gp} gc={gc}",max(np.linalg.eigvals(reduced).real),1e-8)


def boundary_packet(m):
    def rhs(state,order):
        q,b,p,r=state; u=q/m.m
        return (-m.CT(b)-(m.bd*u if order else 0)-(m.A(r) if order==2 else 0),m.C(u),u,p)
    def add(a,b,s): return tuple(x+s*y for x,y in zip(a,b))
    def integrate(initial,order,tfinal):
        steps=math.ceil(tfinal/(.22*m.dt)); dt=tfinal/steps
        state=tuple(x.copy() for x in initial)
        for _ in range(steps):
            k1=rhs(state,order); k2=rhs(add(state,k1,dt/2),order)
            k3=rhs(add(state,k2,dt/2),order); k4=rhs(add(state,k3,dt),order)
            state=tuple(x+dt/6*(a+2*b+2*c+d) for x,a,b,c,d in zip(state,k1,k2,k3,k4))
        return state,steps
    for angle in (0.,30.):
        n=np.array([np.cos(np.deg2rad(angle)),np.sin(np.deg2rad(angle))])
        origin=np.array([-.35,-.30 if angle else 0.])
        along=(m.x-origin)@n; across=(m.x-origin)@np.array([-n[1],n[0]])
        k=2*np.pi/.65; width=.23
        envelope=np.exp(-.5*(along/width)**2-.5*(across/.32)**2)
        psi=envelope*np.sin(k*along)/k
        u=envelope*(along/(width**2*k)*np.sin(k*along)-np.cos(k*along))
        initial=(m.m*u,m.C(psi),psi.copy(),np.zeros(m.n))
        initial_e=m.energy(*initial[:2]); final=2.6
        energies=[]
        for order in (0,1,2):
            state,steps=integrate(initial,order,final)
            energies.append(m.energy(*state[:2]))
            observation(f"packet angle={angle} outgoing={order}",energy_ratio=float(energies[-1]/initial_e),steps=steps,finite=bool(all(np.isfinite(x).all() for x in state)))
        r1=np.sqrt(energies[1]/energies[0]); r2=np.sqrt(energies[2]/energies[0])
        if angle==0: check("first-order normal packet reflection proxy",r1,.15)
        else: check("second-order oblique packet excess reflection",r2-r1,.02)
        observation(f"packet reflections angle={angle}",first=float(r1),second=float(r2),meaning="Residual bulk-energy amplitude proxy, includes discretization/other box sides; not pure one-plane reflection.")


def thin_gap(m):
    # Pair duplicated interface P2 nodes with positive Simpson trace weights.
    left=set(m.ids[m.qx[:,:,0].mean(axis=1)<0].ravel())
    right=set(m.ids[m.qx[:,:,0].mean(axis=1)>0].ravel())
    l=sorted([i for i in left if abs(m.x[i,0])<1e-12],key=lambda i:m.x[i,1])
    r=sorted([i for i in right if abs(m.x[i,0])<1e-12],key=lambda i:m.x[i,1])
    trace=np.zeros((len(l),m.n)); weights=np.zeros(len(l)); h=2/m.data['n']
    for j,(a,b) in enumerate(zip(l,r)): trace[j,a]=1; trace[j,b]=-1
    for j in range(0,len(l)-2,2): weights[j:j+3]+=h*np.array([1,4,1])/6
    stiffness=3*weights
    rng=np.random.default_rng(6); u=rng.normal(size=m.n); z=rng.normal(size=len(l))
    force=trace.T@(stiffness*z)
    check("thin-gap paired force total",abs(force.sum()),1e-10)
    check("thin-gap energy derivative balance",abs(-u@force+np.dot(stiffness*z,trace@u)),1e-9)
    q=m.m*np.cos(np.pi*m.x[:,1]/2); b=np.zeros_like(m.qx); p=np.zeros(m.n); z=np.zeros(len(l))
    qp=q.copy(); pp=p.copy(); dt=.08*m.dt
    for _ in range(100):
        force=m.CT(b)+trace.T@(stiffness*z)
        qh=q-.5*dt*force
        b+=dt*m.C(qh/m.m); z+=dt*trace@(qh/m.m)
        q=qh-.5*dt*(m.CT(b)+trace.T@(stiffness*z))
        qph=qp-.5*dt*(m.K(pp)+trace.T@(stiffness*(trace@pp)))
        pp+=dt*qph/m.m
        qp=qph-.5*dt*(m.K(pp)+trace.T@(stiffness*(trace@pp)))
    check("thin-gap direct/reference evolution",relative(q,qp),1e-10)
    observation("thin-gap required state",trace_dofs=len(l),equations="z_dot=T u; Q_dot includes -T^T k z; energy += z^T k z/2. z is physical jump memory and transfers independently of bulk b.")


def locate(mesh, points):
    points=np.asarray(points).reshape(-1,2)
    owner=np.full(len(points),-1,dtype=int); bary=np.zeros((len(points),3))
    for e,tri in enumerate(mesh.tri):
        candidate=np.where((owner<0)&np.all(points>=tri.min(axis=0)-1e-12,axis=1)&np.all(points<=tri.max(axis=0)+1e-12,axis=1))[0]
        if not len(candidate): continue
        uv=(points[candidate]-tri[0])@np.linalg.inv(np.stack([tri[1]-tri[0],tri[2]-tri[0]],axis=1)).T
        bc=np.column_stack([1-uv.sum(axis=1),uv])
        take=np.all(bc>=-1e-12,axis=1)
        owner[candidate[take]]=e; bary[candidate[take]]=bc[take]
    if np.any(owner<0): raise ValueError("point outside source mesh")
    return owner,bary


def basis(b):
    x,y,z=b.T; bubble=27*x*y*z
    return np.stack([x*(2*x-1)+bubble/9,y*(2*y-1)+bubble/9,z*(2*z-1)+bubble/9,4*x*y-4*bubble/9,4*y*z-4*bubble/9,4*z*x-4*bubble/9,bubble],axis=1)


def poly(b):
    y,z=b[:,1],b[:,2]
    return np.stack([np.ones(len(b)),y,z,y*y,y*z,z*z],axis=1)


def transfer_scalar(source,target,density):
    owner,bary=locate(source,target.x)
    return np.sum(basis(bary)*density[source.ids[owner]],axis=1)


def transfer_vector(source,target,b):
    # Six Gauss points unisolvent for degree-two vector polynomials.
    # This is geometry-preparable local transfer, not a global projection.
    _,bc=locate(source,source.qx[0])
    inverse=np.linalg.inv(poly(bc))
    coefficients=np.einsum('ij,ejc->eic',inverse,b)
    owner,bary=locate(source,target.qx)
    return np.einsum('ni,nic->nc',poly(bary),coefficients[owner]).reshape(target.qx.shape)


def null_fraction(mesh,b):
    c=mesh.dense_c(); w=np.sqrt(np.repeat(mesh.w.ravel(),2))
    a=w[:,None]*c
    coeff=np.linalg.lstsq(a,w*b.ravel(),rcond=1e-12)[0]
    residual=w*(b.ravel()-c@coeff)
    return float(np.dot(residual,residual)/max(np.dot(w*b.ravel(),w*b.ravel()),1e-30))


def divergence_rms(mesh,b):
    # Element-interior divergence of quadratic reconstruction. Face charges
    # are distinct; this diagnostic alone is deliberately not a Gauss-law proof.
    _,bc=locate(mesh,mesh.qx[0]); coeff=np.einsum('ij,ejc->eic',np.linalg.inv(poly(bc)),b)
    div=[]
    for tri,co in zip(mesh.tri,coeff):
        inv=np.linalg.inv(np.stack([tri[1]-tri[0],tri[2]-tri[0]],axis=1))
        dy=co[1]+2*bc[:,1,None]*co[3]+bc[:,2,None]*co[4]
        dz=co[2]+bc[:,1,None]*co[4]+2*bc[:,2,None]*co[5]
        div.append(dy[:,0]*inv[0,0]+dz[:,0]*inv[1,0]+dy[:,1]*inv[0,1]+dz[:,1]*inv[1,1])
    div=np.array(div)
    return float(np.sqrt(np.sum(mesh.w*div*div)/np.sum(mesh.w)))


def transfer_tests(coarse,fine,split,packet):
    density=lambda x:.4+.3*np.cos(np.pi*x[:,0]/2)*np.cos(np.pi*x[:,1]/2)
    psi=lambda x:.1*np.cos(np.pi*x[:,0]/2)*np.cos(np.pi*x[:,1]/2)
    for src,dst,label in ((coarse,coarse,"identity"),(coarse,fine,"refine"),(fine,coarse,"coarsen"),(fine,packet,"resolved refine"),(packet,fine,"resolved coarsen")):
        q=src.m*density(src.x)
        raw=dst.m*transfer_scalar(src,dst,q/src.m)
        correction=(q.sum()-raw.sum())*dst.m/dst.m.sum()
        new=raw+correction
        check(f"transfer {label} integrated Q",abs(new.sum()-q.sum()),1e-11)
        check(f"transfer {label} primary smooth error",np.sqrt(np.sum(dst.m*(new/dst.m-density(dst.x))**2)/np.sum(dst.m*density(dst.x)**2)),.05)
        check(f"transfer {label} affine scalar",relative(transfer_scalar(src,dst,1+.1*src.x[:,0]-.2*src.x[:,1]),1+.1*dst.x[:,0]-.2*dst.x[:,1]),1e-10)
        const=np.zeros_like(src.qx); const[...,0]=.2; const[...,1]=-.3
        targetconst=np.zeros_like(dst.qx); targetconst[...,0]=.2; targetconst[...,1]=-.3
        check(f"transfer {label} constant vector",relative(transfer_vector(src,dst,const),targetconst),1e-10)
        affine=np.stack([src.qx[...,0]+2*src.qx[...,1],3*src.qx[...,0]-src.qx[...,1]],axis=-1)
        affine_target=np.stack([dst.qx[...,0]+2*dst.qx[...,1],3*dst.qx[...,0]-dst.qx[...,1]],axis=-1)
        check(f"transfer {label} affine vector",relative(transfer_vector(src,dst,affine),affine_target),1e-10)
        oldb=src.C(psi(src.x)); newb=transfer_vector(src,dst,oldb)
        exact=np.stack([.1*np.pi/2*np.cos(np.pi*dst.qx[...,0]/2)*np.sin(np.pi*dst.qx[...,1]/2),-.1*np.pi/2*np.sin(np.pi*dst.qx[...,0]/2)*np.cos(np.pi*dst.qx[...,1]/2)],axis=-1)
        rms=np.sqrt(np.sum(dst.w[...,None]*(newb-exact)**2)/np.sum(dst.w[...,None]*exact**2))
        check(f"transfer {label} smooth vector RMS",rms,.05,note="Total error versus analytic field includes source-mesh differentiation error; identity failure diagnoses input under-resolution, not failed identity transfer.")
        if dst.n>500:
            recovered_p=transfer_scalar(src,dst,psi(src.x))
            nf=float(np.sum(dst.w[...,None]*(newb-dst.C(recovered_p))**2)/np.sum(dst.w[...,None]*newb**2))
        else:
            nf=null_fraction(dst,newb)
        check(f"transfer {label} stationary energy fraction",nf,.01)
        observation(f"transfer {label} details",relative_total_correction=float(np.sum(abs(correction))/np.sum(abs(q))),interior_divergence_rms=divergence_rms(dst,newb),stationary_measure="upper bound from interpolated potential" if dst.n>500 else "weighted orthogonal projection",note="Full remesh: all support is affected. Identity is exact-copy production path; these interpolate checks expose roundoff. Interior divergence excludes face jumps. No f32 GPU transfer claimed.")
    # Geometry-only partition of integrated supports for opening/closing a seam.
    merged_q=coarse.m*density(coarse.x)
    lookup={tuple(np.round(x,12)):i for i,x in enumerate(coarse.x)}
    parent=np.array([lookup[tuple(np.round(x,12))] for x in split.x])
    split_q=merged_q[parent]*split.m/coarse.m[parent]
    recovered=np.bincount(parent,weights=split_q,minlength=coarse.n)
    check("split component shares add to old Q",abs(split_q.sum()-merged_q.sum()),1e-11)
    check("merge summed supports recovers Q",relative(recovered,merged_q),1e-10)
    oldb=coarse.C(psi(coarse.x)); sb=transfer_vector(coarse,split,oldb)
    back=transfer_vector(split,coarse,sb)
    check("split/merge vector constant-gauge independence",relative(back,oldb),1e-10)
    q0,b0=coarse.step(merged_q,oldb,.1*coarse.dt)
    q1,b1=coarse.step(recovered,back,.1*coarse.dt)
    check("split/merge no spurious first-step transient",max(relative(q1,q0),relative(b1,b0)),1e-10)
    # Bounded comparison fallback: potential plus residual memory for the same
    # spatial-loss state. Both decompositions are equivalent before transfer.
    p=psi(fine.x); b=fine.C(p)*np.exp(-.4*(fine.qx[...,0]>0))[...,None]
    memory=b-fine.C(p)
    direct=transfer_vector(fine,coarse,b)
    potential=coarse.C(transfer_scalar(fine,coarse,p))
    alternate=potential+transfer_vector(fine,coarse,memory)
    observation("fallback remap comparison",direct_stationary_fraction=null_fraction(coarse,direct),potential_plus_memory_stationary_fraction=null_fraction(coarse,alternate),relative_flux_disagreement=float(np.sqrt(np.sum(coarse.w[...,None]*(direct-alternate)**2)/np.sum(coarse.w[...,None]*direct**2))),note="Physical nonpotential loss state is retained by both; this comparison is not a zero-charge projection.")
    # Cycling without evolution quantifies persistence, rather than treating
    # a one-time stationary fraction as a transient that the wave filter removes.
    b0=fine.C(psi(fine.x)); cycling=b0.copy()
    for _ in range(12): cycling=transfer_vector(coarse,fine,transfer_vector(fine,coarse,cycling))
    observation("twelve remesh cycles",relative_vector_error=float(np.sqrt(np.sum(fine.w[...,None]*(cycling-b0)**2)/np.sum(fine.w[...,None]*b0**2))),stationary_energy_fraction=null_fraction(fine,cycling))
    # Manufactured constant Cu with balancing external source: b'=g-gc*b,
    # psi=t*u. Direct steady b is bounded while memory cancels a growing Cpsi.
    steady=.25; epoch=float(2**26); potential=np.float32(epoch)
    memory=np.float32(steady-epoch)
    observation("fallback f32 cancellation example",epoch=epoch,exact_flux=steady,reconstructed_flux=float(potential+memory),direct_stored_flux=float(np.float32(steady)),note="Algebraic long-clock counterexample, not a long-running GPU integration; potential+memory would need coordinated rebasing.")


def material_interface_and_events(m):
    side=m.qx[:,:,0].mean(axis=1)>0
    eps=np.where(side,2.,1.); mu=np.where(side,.5,1.)
    local_mass=(m.w.sum(axis=1)*eps)[:,None]*np.array([1/20]*3+[2/15]*3+[9/20])
    mass=np.bincount(m.ids.ravel(),weights=local_mass.ravel(),minlength=m.n)
    psi=np.where(m.x[:,0]>0,.5*m.x[:,0],m.x[:,0])
    b=m.C(psi); h=b/mu[:,None,None]
    force=m.CT(h)
    interior=np.max(abs(m.x),axis=1)<1-1e-12
    check("material interface weak equilibrium",np.max(abs(force[interior])),1e-10)
    rng=np.random.default_rng(51); u=rng.normal(size=m.n); b=rng.normal(size=m.qx.shape)
    qdot=-m.CT(b/mu[:,None,None]); bdot=m.C(u)
    balance=u@qdot+np.sum(m.w[...,None]*b/mu[:,None,None]*bdot)
    check("heterogeneous material energy exchange cancellation",abs(balance),1e-9)
    check("heterogeneous nodal mass integrates both materials",abs(mass.sum()-6),1e-10)
    # Homogeneous temporal-interface decomposition, incident E=H=1, D=B=1.
    for eperm,mperm,label in ((2.,2.,"equal"),(2.,1.,"electric-only")):
        e=1/eperm; h=1/mperm; impedance=np.sqrt(mperm/eperm)
        reflected=.5*(e-impedance*h)
        if label=="equal": check("equal eps/mu temporal reflection",abs(reflected),1e-10)
        else: check("eps-only temporal reflection nonzero",abs(reflected),.01,lower=True)
    # Independent region frames must be sampled before nodal assembly.
    node=np.array([.25,.5]); frame_a=node; frame_b=np.array([node[1],-node[0]])
    u=.7; chi_a=.2+.3*frame_a[0]; chi_b=.2+.3*frame_b[0]
    complete=.4*law_value(u,"kerr",chi_a)+.6*law_value(u,"kerr",chi_b)
    incorrectly_merged=law_value(u,"kerr",chi_a)
    check("region-frame contribution distinction",abs(complete-incorrectly_merged),.001,lower=True)


def runtime_tests():
    # Constitutive edit with prescribed nonzero primary value; free nodes keep Q.
    q=np.array([.3,.8]); prescribed=.7; old_mass=.4; new_mass=.9
    q[0]=old_mass*prescribed; before=q.copy()
    q[0]=new_mass*prescribed; exchange=(new_mass-old_mass)*prescribed
    check("prescribed material event primary continuity",abs(q[0]/new_mass-prescribed),1e-10)
    check("prescribed material event boundary exchange",abs(q.sum()-before.sum()-exchange),1e-10)
    check("prescribed material event free Q unchanged",abs(q[1]-before[1]),1e-10)
    def migrated(t,omega,phase,offset,amplitude):
        z=omega*t/2
        return offset*t+amplitude*t*np.sinc(z/np.pi)*np.sin(phase+z)
    for omega in (0.,1e-14,1e-5,3.7):
        t=.7; h=1e-5; phi=.3
        derivative=(migrated(t+h,omega,phi,.2,.8)-migrated(t-h,omega,phi,.2,.8))/(2*h)
        check(f"migrated source derivative omega={omega}",abs(derivative-(.2+.8*np.sin(omega*t+phi))),1e-8)
    check("migrated source exact zero limit",abs(migrated(1.2,0,.7,.2,.8)-1.2*(.2+.8*np.sin(.7))),1e-10)
    tanchor=2.**26; localtime=.0123; phase=.7; oldomega=3.; newomega=9.
    boundary_phase=phase+oldomega*localtime
    check("frequency edit preserves phase anchor",abs((boundary_phase+newomega*0)-boundary_phase),1e-10)
    newphase=boundary_phase+newomega*.01
    check("frequency edit subsequent phase",abs(newphase-(phase+oldomega*localtime+.09)),1e-10)
    check("epoch absolute-time separation",abs((tanchor+localtime-tanchor)-localtime),1e-8)
    smooth=lambda z:6*z**5-15*z**4+10*z**3
    current=1+2*smooth(.35)
    reversal=lambda z:current+(1-current)*smooth(z)
    check("ramp reversal value continuity",abs(reversal(0)-current),1e-10)
    observation("edited ramp envelope",current=current,new_target=1.2,required_upper=max(current,1.2),derivative_continuity_promised=False)
    # Nonlinear inverse near the weakest accepted saturable tangent.
    chi=-8/9+1e-5; expected=np.sqrt(3.); target=law_value(expected,"sat",chi)
    lo,hi=0.,8.
    for _ in range(80):
        mid=(lo+hi)/2
        if law_value(mid,"sat",chi)<target: lo=mid
        else: hi=mid
    check("near-threshold saturable inverse residual",abs(law_value((lo+hi)/2,"sat",chi)-target),1e-11)
    # Model the candidate/validate/commit algorithm, explicitly not a GPU race test.
    accepted=dict(q=np.array([.2,.3]),b=np.array([.1,-.2]),clock=1.,account=.5,serial=7)
    original={key:(value.copy() if isinstance(value,np.ndarray) else value) for key,value in accepted.items()}
    for stage in ("inverse","force","pulse","law_patch"):
        candidate={key:(value.copy() if isinstance(value,np.ndarray) else value) for key,value in accepted.items()}
        candidate['q'][1]=np.nan
        valid=np.isfinite(candidate['q']).all()
        if valid: accepted=candidate
        same=all(np.array_equal(accepted[key],value) for key,value in original.items())
        check(f"rejected {stage} preserves accepted state",0 if same else 1,0)
    bound=.8; chi=.8; maxflux=bound/(1+chi*bound*bound)
    check("bounded reciprocal rejects out-of-domain request",0 if .8>maxflux else 1,0)
    observation("runtime scope",coverage="CPU ownership/phase/analytic waveform fixtures only. GPU synchronization, source-buffer replacement races, and device f32 shader behavior require implementation checks.")


def resources_and_timing(meshes):
    for name in ('coarse','packet'):
        m=meshes[name]; nnz=len(m.k)
        # Rust ShaderType/WGSL: NodeData=176, State=48, forcing weight word=16.
        baseline=240*m.n+4*(m.n+1)+12*nnz
        # Candidate conservatively keeps old node metadata: q accepted/candidate,
        # u scratch + accounting =16 N; source words16 N; row ranges8 N.
        # Per element: b accepted/candidate96, connectivity32, affine geometry32,
        # quad law metadata192 (32 bytes/sample), cached local force32,
        # incidence7*8=56, seven primary contribution records16 each=112.
        # Physical boundary state budget 32 bytes/boundary node.
        boundary=int(np.count_nonzero(m.bd))
        candidate=(176+16+16+8)*m.n+(96+32+32+192+32+56+112)*m.ne+32*boundary
        check(f"resource bytes ratio {name}",candidate/baseline,2.)
        observation(f"resource accounting {name}",nodes=m.n,elements=m.ne,nonzeros=nnz,baseline_main_bytes=baseline,candidate_main_bytes=candidate,old_core_estimator=m.data['old_estimated_bytes'],transfer_stencil_bytes=96*m.n+192*m.ne,transfer_reduction_scratch_bytes=16*m.n,candidate_note="Provisional linear layout with constitutive table indices, not a compiled shader. Common source/material/recorder/display storage excluded from both. Law catalogue payloads require their own capacity budget. Transfer peak includes both generations plus stencils/scratch.")
        rng=np.random.default_rng(3); p=rng.normal(size=m.n); b=m.C(p)
        repeats=150
        t=time.perf_counter()
        for _ in range(repeats): m.K(p)
        csr=(time.perf_counter()-t)/repeats
        t=time.perf_counter()
        for _ in range(repeats): m.CT(b)
        gather=(time.perf_counter()-t)/repeats
        t=time.perf_counter()
        q=m.m*p
        for _ in range(repeats): q,b=m.step(q,b,.1*m.dt)
        step=(time.perf_counter()-t)/repeats
        observation(f"numpy CPU timing {name}",csr_force_us=csr*1e6,element_gather_us=gather*1e6,direct_step_us=step*1e6,note="Python/NumPy sparse-gather prototype; not GPU or optimized Rust throughput.")
    observation("binding/dispatch design",bindings=["control/status/clock","nodal metadata and contribution tables","element geometry/connectivity/quad laws","node incidence","accepted+candidate nodal state","accepted+candidate vector state","local-force scratch","sources/events/runtime laws"],max_storage_bindings=8,lossless_field_dispatches=3,commit_dispatches=1,loss_path_extra_element_dispatches="1-2 depending on stage fusion",force_strategy="Element-local seven-node forces + deterministic node gather, no float atomics. Direct flux does not automatically retain a one-CSR-evaluation fast path.")


def main():
    start=time.perf_counter()
    with open(sys.argv[1]) as stream: data=json.load(stream)
    meshes={name:Mesh(d) for name,d in data.items()}
    m=meshes['coarse']
    eig,vec,k,c=linear_tests(m)
    nonlinear_tests(m)
    filter_tests(m,eig,vec,k,c)
    boundary_spectrum(m,k)
    thin_gap(meshes['split'])
    material_interface_and_events(m)
    transfer_tests(m,meshes['fine'],meshes['split'],meshes['packet'])
    runtime_tests()
    resources_and_timing(meshes)
    boundary_packet(meshes['packet'])
    observation("elapsed",seconds=time.perf_counter()-start)
    failed=[r['name'] for r in RESULTS if r.get('passed') is False]
    print(json.dumps(dict(baseline="beca47e0d05c9b24945ef632b00c5cc16f80c8a0",summary=dict(passed=sum(r.get('passed') is True for r in RESULTS),failed=len(failed),failed_cases=failed),results=RESULTS),indent=2))
    sys.exit(1 if failed else 0)


if __name__=='__main__': main()
