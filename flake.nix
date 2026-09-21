{
  description = "funfern: a browser finite-element wave playground";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;

      # The threaded browser build rebuilds `std` with Wasm atomics, so it needs
      # the nightly `scripts/trunk` selects. Keep these in step with that script,
      # rust-toolchain.toml and .github/workflows/ci.yml.
      nightlyDate = "2026-05-28";
      trunkVersion = "0.22.0-beta.5";
      wasmBindgenVersion = "0.2.128";

      # Prebuilt release archives, the same ones CI installs. Trunk uses a
      # matching tool from PATH and downloads one otherwise; what it downloads is
      # linked against an FHS loader and cannot run here.
      trunkTargets = {
        x86_64-linux = {
          triple = "x86_64-unknown-linux-gnu";
          hash = "sha256-2wxGkyOL/7fjvVpvlC1BVIoj9FFvx7W97bJ+S77X464=";
        };
        aarch64-linux = {
          triple = "aarch64-unknown-linux-gnu";
          hash = "sha256-hzzmCQQGiwIG17e/9KEcMjIYPmaU9zP17ppPTR+C+po=";
        };
      };

      wasmBindgenTargets = {
        x86_64-linux = {
          triple = "x86_64-unknown-linux-musl";
          hash = "sha256-tR8CCP3/g1FaeHvYq5rFhl7YTau2bQxwmVe7WXk8ZF8=";
        };
        aarch64-linux = {
          triple = "aarch64-unknown-linux-gnu";
          hash = "sha256-L2Eme77yecLWNXsZlklGiwcyI/695K4QjTCsPWtVQK8=";
        };
      };
    in
    {
      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };

          # Native development toolchain, read from rust-toolchain.toml so the
          # pinned channel, components and targets stay in one place.
          rustStable = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

          rustNightly = pkgs.rust-bin.nightly.${nightlyDate}.minimal.override {
            extensions = [ "rust-src" ];
            targets = [ "wasm32-unknown-unknown" ];
          };

          # `scripts/trunk` picks the nightly with RUSTUP_TOOLCHAIN, which does
          # nothing without rustup. These wrappers honour it: the nightly when the
          # variable names one, the pinned stable otherwise.
          rustToolchains = pkgs.symlinkJoin {
            name = "funfern-rust-toolchains";
            paths = map
              (tool: pkgs.writeShellScriptBin tool ''
                case "''${RUSTUP_TOOLCHAIN:-}" in
                  nightly*) root=${rustNightly} ;;
                  *) root=${rustStable} ;;
                esac
                # The nightly is a minimal profile: clippy and rustfmt come from
                # the stable toolchain either way.
                [ -x "$root/bin/${tool}" ] || root=${rustStable}
                # Point cargo at the toolchain directly. Both wrappers live at
                # one path, and cargo caches the sysroot it probes under the path
                # it ran, so a shared wrapper path hands the nightly build the
                # stable sysroot and loses rust-src.
                export RUSTC="$root/bin/rustc"
                export RUSTDOC="$root/bin/rustdoc"
                exec "$root/bin/${tool}" "$@"
              '')
              [ "cargo" "rustc" "rustdoc" "rustfmt" "cargo-fmt" "cargo-clippy" "clippy-driver" ];
          };

          trunkTarget = trunkTargets.${system};
          trunk = pkgs.stdenv.mkDerivation {
            pname = "trunk";
            version = trunkVersion;
            src = pkgs.fetchurl {
              url = "https://github.com/trunk-rs/trunk/releases/download/v${trunkVersion}/trunk-${trunkTarget.triple}.tar.gz";
              inherit (trunkTarget) hash;
            };
            sourceRoot = ".";
            nativeBuildInputs = [ pkgs.autoPatchelfHook ];
            buildInputs = [ pkgs.stdenv.cc.cc.lib ];
            installPhase = "install -Dm755 trunk $out/bin/trunk";
          };

          wasmBindgenTarget = wasmBindgenTargets.${system};
          wasmBindgenStatic = pkgs.lib.hasSuffix "musl" wasmBindgenTarget.triple;
          wasm-bindgen-cli = pkgs.stdenv.mkDerivation {
            pname = "wasm-bindgen-cli";
            version = wasmBindgenVersion;
            src = pkgs.fetchurl {
              url = "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/${wasmBindgenVersion}/wasm-bindgen-${wasmBindgenVersion}-${wasmBindgenTarget.triple}.tar.gz";
              inherit (wasmBindgenTarget) hash;
            };
            nativeBuildInputs = pkgs.lib.optional (!wasmBindgenStatic) pkgs.autoPatchelfHook;
            buildInputs = pkgs.lib.optional (!wasmBindgenStatic) pkgs.stdenv.cc.cc.lib;
            installPhase = ''
              for tool in wasm-bindgen wasm-bindgen-test-runner wasm2es6js; do
                [ -f "$tool" ] && install -Dm755 "$tool" "$out/bin/$tool"
              done
            '';
          };

          # winit, wgpu and arboard open these with dlopen, so they have to be
          # findable at run time and not only at link time.
          runtimeLibraries = with pkgs; [
            vulkan-loader
            libxkbcommon
            wayland
            libx11
            libxcursor
            libxi
            libxrandr
            libGL
          ];
        in
        {
          default = pkgs.mkShell {
            packages = [
              rustToolchains
              trunk
              wasm-bindgen-cli
              pkgs.binaryen # wasm-opt, pinned to version_132 by Trunk.toml
              pkgs.pkg-config
              pkgs.ffmpeg # native viewport recording
              pkgs.nodejs # Playwright browser smoke test
              pkgs.python3 # browser-tests/server.py
            ] ++ runtimeLibraries;

            env = {
              LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibraries;
              # rust-src lives in the nightly store path, not under ~/.rustup.
              RUST_SRC_PATH = "${rustNightly}/lib/rustlib/src/rust/library";
            };

            shellHook = ''
              echo "funfern: rustc $(rustc --version | cut -d' ' -f2), nightly ${nightlyDate} for scripts/trunk, trunk ${trunkVersion}"
            '';
          };
        });
    };
}
