#!/usr/bin/env bash
# Runs one of the app's end-to-end fixtures (src/ui/e2e.rs), or one of its
# GPU-versus-reference examples, as the device suite runs them: built with
# `--features e2e` where the app is the binary, under a scratch HOME of its
# own, on this machine's GPU, or on lavapipe under a virtual X server on
# Linux, where the run must report lavapipe's adapter or it fails. Exits as
# the fixture or example exits: 0 when it holds, 1 when it does not. The
# mutation runner names a run through this, so one manifest serves both
# platforms.
#
#   scripts/e2e-fixture.sh handoff
#   scripts/e2e-fixture.sh --example canonical_gpu_temporal_handoff
#   scripts/e2e-fixture.sh --example canonical_gpu_handoff --second-order
set -u
cd "$(dirname "$0")/.."
if [[ ${1:-} == --example ]]; then
  shift
  example=${1:?name an example}
  shift
  cargo build -p funfern-app --release --locked --example "$example" || exit 1
  binary="target/release/examples/$example"
  environment=()
else
  fixture=${1:?name a fixture}
  cargo build -p funfern-app --release --locked --features e2e --bin funfern-app || exit 1
  binary=target/release/funfern-app
  environment=("FUNFERN_E2E=$fixture")
  set --
fi
home=$(mktemp -d)
log=$(mktemp)
trap 'rm -rf "$home" "$log"' EXIT
mkdir -p "$home/Library/Application Support/funfern"
limit=${FIXTURE_TIMEOUT:-600}
if [[ $(uname -s) == Linux ]]; then
  icd=$(ls /usr/share/vulkan/icd.d/lvp_icd*.json 2>/dev/null | head -1)
  if [[ -z $icd ]]; then
    echo "lavapipe's ICD is not installed (mesa-vulkan-drivers)" >&2
    exit 1
  fi
  env HOME="$home" VK_ICD_FILENAMES="$icd" WGPU_BACKEND=vulkan ${environment[@]+"${environment[@]}"} \
    timeout "$limit" xvfb-run -a -s "-screen 0 1280x800x24" "$binary" "$@" 2>&1 | tee "$log"
  status=${PIPESTATUS[0]}
  if ! grep -q 'AdapterInfo { name: "llvmpipe' "$log"; then
    echo "the run did not take place on lavapipe" >&2
    exit 1
  fi
  exit "$status"
fi
env HOME="$home" ${environment[@]+"${environment[@]}"} \
  perl -e 'alarm shift @ARGV; exec @ARGV or die "$!\n"' "$limit" "$binary" "$@"
