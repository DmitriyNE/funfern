#!/bin/bash
# The gate: every check a commit must pass, in one command. It stops at the
# first that fails, names it and prints the end of its output; each step's
# output is kept under target/gate/. The device suite (scripts/device-suite.sh)
# joins it by hand when shaders, the device path or the fixtures change.
#
# Until 2026-11-17 the gate lived in a session's scratch script and was lost
# with it more than once (the testing campaign's closeout, section 10).
set -u
cd "$(dirname "$0")/.." || exit 1
logs=target/gate
mkdir -p "$logs"

step() {
  local name=$1
  shift
  local started=$SECONDS
  printf '%-14s' "$name"
  if "$@" >"$logs/$name.log" 2>&1; then
    echo "ok      $((SECONDS - started)) s"
  else
    echo "FAILED  $((SECONDS - started)) s, $logs/$name.log:"
    tail -30 "$logs/$name.log"
    exit 1
  fi
}

step fmt cargo fmt --all -- --check
step clippy cargo clippy --workspace --all-targets --locked -- -D warnings
# The browser's build: raw wgpu handles in a resource fail only there, and in
# the threaded build below.
step clippy-wasm cargo clippy -p funfern-app --bins --target wasm32-unknown-unknown --locked -- -D warnings
step test cargo test --workspace --release --locked
# The default scene in the debug profile, whose overflow checks the release
# tests do not make.
step test-debug cargo test -p funfern-core --test default_scene --locked
step build cargo build --release --locked
step wasm cargo check -p funfern-app --target wasm32-unknown-unknown --locked
step wasm-threads env RUSTUP_TOOLCHAIN=nightly-2026-05-28 CARGO_UNSTABLE_BUILD_STD=panic_abort,std \
  CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals' \
  cargo check -p funfern-app --target wasm32-unknown-unknown --features browser-threads --locked
# Every shader compiled by Chrome's own WebGPU, which needs a hardware adapter.
step shaders env PLAYWRIGHT_CHANNEL="${PLAYWRIGHT_CHANNEL:-chrome}" npm run test:shaders
step kani cargo kani -p funfern-protocol
echo "the gate passed"
