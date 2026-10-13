#!/usr/bin/env bash
# Runs the app's end-to-end fixtures (src/ui/e2e.rs) on Mesa's software Vulkan,
# lavapipe, under a virtual X server: the shipped shaders, render graph and
# runtime on a second backend, where CI has no GPU. Linux only; expects
# mesa-vulkan-drivers, xvfb and the X client libraries winit loads, and the app
# `funfern-fixtures` binary built under the target directory (CARGO_TARGET_DIR,
# or target).
#
#   scripts/e2e-lavapipe.sh            every fixture scripts/device-suite.sh runs
#   scripts/e2e-lavapipe.sh cavity     the ones named
#
# A run must report lavapipe's adapter, or it fails: a machine where the app
# found some other device has not run what this is for. Exits 1 if any fixture
# failed.
set -u
cd "$(dirname "$0")/.."

icd=$(ls /usr/share/vulkan/icd.d/lvp_icd*.json 2>/dev/null | head -1)
if [[ -z $icd ]]; then
  echo "lavapipe's ICD is not installed (mesa-vulkan-drivers)" >&2
  exit 1
fi
if [[ $# -gt 0 ]]; then
  fixtures=("$@")
else
  # The fixtures the device suite runs, so the two lists cannot part: its run
  # lines alone, not its probe, which asks the linked binary for a fixture
  # that does not exist.
  mapfile -t fixtures < <(grep -o "'FUNFERN_E2E=[a-z-]* funfern-fixtures'" scripts/device-suite.sh \
    | sed "s/'FUNFERN_E2E=\([a-z-]*\) .*/\1/")
  if [[ ${#fixtures[@]} -eq 0 ]]; then
    echo "no fixture run lines found in scripts/device-suite.sh" >&2
    exit 1
  fi
fi
target=${CARGO_TARGET_DIR:-target}
logs=$target/e2e-lavapipe
mkdir -p "$logs"
failed=0
for fixture in "${fixtures[@]}"; do
  home=$(mktemp -d)
  log="$logs/$fixture.log"
  started=$SECONDS
  # The driver's own deadline, 120 s unless set, is for a GPU: on lavapipe
  # the rejection fixture takes 110 to 118 s on the four-core runner, and
  # took 127 on 2026-10-12. Six hundred here, inside the outer 900.
  HOME=$home VK_ICD_FILENAMES=$icd WGPU_BACKEND=vulkan FUNFERN_E2E=$fixture \
    FUNFERN_E2E_DEADLINE=${FUNFERN_E2E_DEADLINE:-600} \
    timeout 900 xvfb-run -a -s "-screen 0 1280x800x24" "$target/release/funfern-fixtures" >"$log" 2>&1
  status=$?
  rm -rf "$home"
  if ! grep -q 'AdapterInfo { name: "llvmpipe' "$log"; then
    echo "FAIL $fixture: the app did not run on lavapipe" >&2
    status=1
  fi
  if [[ $status -eq 0 ]]; then
    verdict=pass
  else
    verdict="FAIL ($status)"
    failed=$((failed + 1))
  fi
  printf '%-10s %4d s  %s\n' "$verdict" $((SECONDS - started)) "$fixture"
  # A run that held: its last two lines; one that did not: every line of
  # the driver's account, which is what there is to read.
  if [[ $status -eq 0 ]]; then
    grep "^e2e " "$log" | tail -2 | sed 's/^/           /'
  else
    grep "^e2e " "$log" | sed 's/^/           /'
  fi
done
[[ $failed -eq 0 ]]
