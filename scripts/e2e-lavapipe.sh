#!/usr/bin/env bash
# Runs the app's end-to-end fixtures (src/ui/e2e.rs) on Mesa's software Vulkan,
# lavapipe, under a virtual X server: the shipped shaders, render graph and
# runtime on a second backend, where CI has no GPU. Linux only; expects
# mesa-vulkan-drivers, xvfb and the X client libraries winit loads, and the app
# built with `--features e2e` under the target directory (CARGO_TARGET_DIR, or
# target).
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
  # The fixtures the device suite runs, so the two lists cannot part.
  mapfile -t fixtures < <(grep -o "FUNFERN_E2E=[a-z-]*" scripts/device-suite.sh | cut -d= -f2)
fi
target=${CARGO_TARGET_DIR:-target}
logs=$target/e2e-lavapipe
mkdir -p "$logs"
failed=0
for fixture in "${fixtures[@]}"; do
  home=$(mktemp -d)
  log="$logs/$fixture.log"
  started=$SECONDS
  HOME=$home VK_ICD_FILENAMES=$icd WGPU_BACKEND=vulkan FUNFERN_E2E=$fixture \
    timeout 900 xvfb-run -a -s "-screen 0 1280x800x24" "$target/release/funfern-app" >"$log" 2>&1
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
  grep "^e2e " "$log" | tail -2 | sed 's/^/           /'
done
[[ $failed -eq 0 ]]
