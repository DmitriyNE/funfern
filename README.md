<p align="center">
  <img src="assets/logo.svg" alt="funfern" width="540">
</p>

<p align="center">
  <strong>An interactive 2D finite-element playground for mechanical and electromagnetic waves.</strong>
</p>

<p align="center">
  Draw geometry, assign materials and boundaries, launch waves, and reshape the scene while the simulation keeps running.
</p>

<p align="center">
  <a href="https://dmitriyne.github.io/funfern/"><strong>Try funfern in the browser →</strong></a>
</p>

<p align="center">
  <a href="https://github.com/DmitriyNE/funfern/actions/workflows/ci.yml">
    <img src="https://github.com/DmitriyNE/funfern/actions/workflows/ci.yml/badge.svg" alt="CI and Pages">
  </a>
</p>

> [!WARNING]
> funfern is an experimental simulation **toy** intended for exploration, visualization and numerical experimentation. It is **not a validated engineering or scientific analysis tool** and should not be used where a quantitatively correct solution is required for design, safety, certification, publication or other consequential decisions.

> [!CAUTION]
> This project is **heavily vibecoded**. A substantial fraction of the code has been written with LLM assistance. It is reviewed and tested, but consistency, elegance and complete absence of creative machine nonsense should not be assumed. Read the code and trust the tests, not the vibes.

## Highlights

* **NURBS-based 2D boundary-representation geometry and topology model** with explicit regions, boundaries and junctions, curve splitting and merging, arbitrary-valence junctions, C0/C1/C2 continuity editing, topology-aware transforms, holes, interfaces and open two-sided boundaries.

* **Live finite-element reassembly and solution handoff across edits and remeshing**, committed transactionally while the current simulation keeps running.

* **Incremental meshing and mesh repair** for boundary motion and topology changes, replacing only the affected part of the constrained mesh.

* **Time-domain field solver over one canonical formulation** covering scalar/mechanical waves and both scalar electromagnetic polarizations, TM and TE.

* **Rich boundary-condition model**: reflecting and prescribed boundaries, first- and higher-order absorbing boundaries, impedance conditions, internal two-sided boundaries and conservative thin-gap coupling.

* **Spatially varying and anisotropic constitutive laws** with formula-defined material properties in local coordinate frames.

* **Error-estimate-driven adaptive mesh refinement and coarsening** under wavelength, grading and mesh-size constraints, preserving the running solution.

* **Extensive solution diagnostics**: scalar and vector field overlays, point, line, boundary and area probes, energy and flux measurements, and time-domain far-field estimation.

* **Browser-native and GPU accelerated** using WebGPU, with the same application also available as a native build.

## What is funfern?

funfern is an interactive 2D finite-element wave-simulation **toy** and geometry
editor for scalar mechanical waves and TE/TM electromagnetic fields. It combines
a NURBS-based 2D boundary representation, meshing and adaptive discretization,
GPU-accelerated time-domain simulation, extensive diagnostics, and live
solution-preserving remeshing in a single browser-native application.

The geometry model is effectively a 2D B-rep: curves define boundaries, but
regions, sidedness, junctions and adjacency are explicit parts of the model
rather than being rediscovered from an unstructured collection of curves.
Material interfaces, holes, transmitting separators and two-sided baffles
therefore participate in the same editable topology, and curves can be split,
merged and reshaped while preserving their topological role.

The time-domain solver uses a shared canonical state consisting of a scalar
potential and an integrated nodal quantity. Mechanical, electromagnetic TM and
electromagnetic TE modes expose different constitutive parameters and physical
observables over this common formulation rather than being three unrelated
solvers. The interactive solver runs on the GPU in single precision, with a
double-precision CPU implementation maintained as a numerical reference.

Editing does not require a stop, rebuild and restart cycle. Geometry and
material changes prepare a replacement discretization while the accepted
simulation continues to run, using incremental mesh repair where possible rather
than rebuilding the entire domain. Once preparation is complete, the running
field is transferred and the replacement discretization and operator are
committed as one transaction. Invalid drafts or failed replacements leave the
last accepted simulation intact. Automatic mesh adaptation refines and coarsens
through the same machinery, driven by solution-based error estimates under
wavelength, grading and element-size constraints.

Scenes support undo/redo, autosave, versioned JSON files and shareable links.
Geometry, snapshots and viewport recordings can be exported. The built-in
example gallery ships obstacle arrays, GRIN rods and Luneburg profiles, ready to
run with their drivers, probes and view presets; `examples/` currently holds the
obstacle scene as a standalone file as well.

Nonlinear and time-dependent constitutive laws are work in progress.

## Implementation

funfern is written in Rust. The application is built with
[Bevy](https://bevyengine.org/) and [egui](https://github.com/emilk/egui)
through [bevy_egui](https://github.com/vladbat00/bevy_egui). GPU compute and
rendering share Bevy's [wgpu](https://wgpu.rs/) device, using WebGPU in the
browser and native graphics backends in the desktop build.

The numerical core lives in `funfern-core`: geometry, topology, meshing,
finite-element assembly and the f64 reference solver are independent of the
application framework. The crate has no external dependencies, including
numerical libraries.

The current discretization is seven-node enriched quadratic, mass-lumped
triangles, advanced by a symplectic-style explicit integrator, on both the f64
CPU reference and an f32 WebGPU gather kernel. An isogeometric discretization is
planned, so the element family is not a fixed property of the project.

## Build and run

### Get the source

Install Rust through [rustup](https://rustup.rs/), then clone the repository:

```sh
git clone https://github.com/DmitriyNE/funfern.git
cd funfern
```

Run the following commands from the repository root.

`rust-toolchain.toml` pins Rust **1.96.1** and declares the
`wasm32-unknown-unknown` target.

### Nix and NixOS

`nix develop` supplies everything the sections below install by hand: both Rust
toolchains, Trunk, the matching `wasm-bindgen` and `wasm-opt` that Trunk would
otherwise download as binaries that do not run on NixOS, and the graphics and
input libraries the native build opens with `dlopen`. `scripts/trunk` and every
`cargo` command work unchanged inside the shell.

The Playwright smoke test still needs a browser Playwright can launch:
`npx playwright install` downloads builds that will not run on NixOS without
`nix-ld`, so run it with `PLAYWRIGHT_CHANNEL=chrome` and an installed Google
Chrome.

### Browser development

The threaded browser build additionally uses **nightly-2026-05-28** to rebuild
the Rust standard library with Wasm atomics, together with
**Trunk 0.22.0-beta.5**.

Install them once:

```sh
rustup toolchain install nightly-2026-05-28 \
  --profile minimal \
  --component rust-src \
  --target wasm32-unknown-unknown

cargo install trunk --version 0.22.0-beta.5 --locked
```

Start the development server:

```sh
scripts/trunk serve --locked
```

Open:

http://127.0.0.1:8080/

A WebGPU-capable browser with hardware acceleration enabled is required. Trunk
watches the source tree and reloads after rebuilds. It binds the HTTP socket
only after the first build succeeds, so wait for the explicit
`server listening at` line before opening the URL.

The wrapper enables Wasm shared memory and builds a worker used for CPU-side
preparation work. `Trunk.toml` supplies the COOP/COEP headers required for
cross-origin isolation.

### Native application

```sh
cargo run -p funfern-app --locked
```

For an optimized build:

```sh
cargo run --release -p funfern-app --locked
```

Native viewport recording additionally requires `ffmpeg` on `PATH`. Browser
recording uses the browser's `MediaRecorder` implementation and requires no
external encoder.

On Ubuntu, the native build dependencies used by CI are:

```sh
sudo apt-get update
sudo apt-get install --yes \
  g++ \
  pkg-config \
  libx11-dev \
  libxkbcommon-x11-0 \
  libwayland-dev \
  libxkbcommon-dev
```

### Release browser build

Build the threaded browser bundle:

```sh
scripts/trunk build --release --locked
```

The output is written to `dist/`.

To serve the optimized build locally:

```sh
scripts/trunk serve --release --locked
```

A deployed threaded build must be served over HTTPS with:

```http
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

Localhost may use HTTP.

### Docker

Docker builds the threaded browser version and serves it through nginx with the
required headers:

```sh
docker compose up --build
```

Open:

http://localhost:8080/

To select another host port:

```sh
env FUNFERN_PORT=9000 docker compose up --build
```

Stop it with:

```sh
docker compose down
```

Public deployment still requires HTTPS, normally through a reverse proxy.

### Static hosting and GitHub Pages

Static hosts such as GitHub Pages cannot send the cross-origin isolation headers
that the threaded shared-memory build requires. `index.html` works around that
with [coi-serviceworker](https://github.com/gzuidhof/coi-serviceworker), vendored
at the repository root: it installs a service worker that adds COOP/COEP to every
response from inside the browser, so the published build at
<https://dmitriyne.github.io/funfern/> is the threaded one.

The first visit costs one reload while the worker installs. The page stops
parsing before Trunk's loader and its wasm preload are reached, so that reload
does not discard a partly downloaded module. Where no worker can be registered —
an insecure context, or a browser with service workers unavailable — the page
says so rather than failing as a WebGPU error.

The worker stays dormant whenever the page is already cross-origin isolated,
which is the case under `trunk serve`, under the Docker nginx configuration, and
on any host that attaches the headers itself.

Plain Trunk still builds the cooperative, non-threaded bundle, which needs no
isolation at all:

```sh
trunk build --release --locked
```

There CPU preparation work is scheduled cooperatively rather than in a
shared-memory worker.

For hosting below a URL prefix, specify it explicitly:

```sh
trunk build --release --locked --public-url /funfern/
```

Pushes to `main` are checked and automatically deployed through GitHub Actions.

## Development

### Checks

The normal checks are:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release -p funfern-app --locked
scripts/trunk build --release --locked
```

The Rust test suite also parses and validates the WGSL shaders using the same
`naga` version linked by `wgpu`, so a reserved keyword or a type error in a
kernel fails the suite rather than the device.

Browser smoke tests additionally require Node.js/npm and a working WebGPU
adapter:

```sh
npm ci
npx playwright install chromium
npm run test:browser
```

Set `PLAYWRIGHT_CHANNEL=chrome` to use an installed Google Chrome instead of
Playwright's pinned Chromium. GitHub-hosted runners expose no usable WebGPU
adapter, so this check runs locally rather than in CI.

Convergence tests, boundary-reflection tests, mesh benchmarks, GPU/reference
comparisons and handoff checks are in [docs/checks.md](docs/checks.md).

### Pinned tooling

These pins are deliberate:

* **Trunk 0.22.0-beta.5** accepts conventional nonempty `NO_COLOR` values such
  as `1`; 0.21.14 exits before serving when that value is present. The beta
  requires Rust 1.96.1 or newer.
* **`wasm_opt = "version_132"`** in `Trunk.toml`, because Trunk's older default
  cannot read Rust 1.96.1's WASM metadata.
* **Bevy 0.19.1 with bevy_egui 0.41.1**, the release line that retains the
  `Send + Sync` browser input representation required when Bevy is compiled with
  Wasm atomics.

`Cargo.lock` is committed and the commands above pass `--locked`.

### Repository layout

```text
crates/funfern-core/       Geometry, topology, meshing, FEM and f64 reference solver
crates/funfern-app/        Bevy/egui application, GPU solver and visualization
examples/                  Standalone scene files; the gallery scenes are built in
assets/                    Logo and application assets
docs/                      Architecture, development plans and verification notes
```

Bevy owns the wgpu device, and the viewport draws through `bevy_egui` on that
same device; there is no second renderer. Audio, 3D render pipelines and the
WebGL fallback are disabled.

Useful documentation:

* [Architecture](docs/architecture.md) — representations and numerical machinery.
* [Checks](docs/checks.md) — specialized verification runs and what they assert.
* [Development plan](docs/plan.md) — milestones and planned work.
* [Engineering log](docs/engineering-log.md) — verification results and implementation history.
* [Browser checks](docs/browser-checks.md) — browser verification procedures.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
