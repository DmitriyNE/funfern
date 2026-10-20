#!/usr/bin/env bash
# Runs the GPU-versus-reference examples, and the app's own end-to-end
# fixtures, on this machine's GPU and reports each run's verdict. Every run
# steps the device and compares what it reads back with the f64 reference on
# the same discretization; a run fails on a device fault, a timeout, or an
# error outside its bound, a NaN included.
#
#   scripts/device-suite.sh             every run
#   scripts/device-suite.sh handoff     the runs whose line matches a pattern
#
# The examples open a window and read the autosave, so each runs with HOME
# pointed at a scratch directory of its own. They are built first with the
# real HOME, or the toolchain would be fetched again into the scratch one.
# Every binary the suite runs is hard-linked into its log directory first and
# run from there: a build in the meantime, a plain `cargo run` beside the
# suite, renames a new file into target/release/ and the link keeps the one
# the suite built, where on 12 October a plain build replaced the e2e app
# under the fixtures and they sat in the first-run tour until the timeout.
# Before the fixtures the linked app is asked for a fixture that does not
# exist, and must refuse it as the driver does, so an app without the driver
# fails the suite rather than opening. Logs go to target/device-suite/<time>/.
# A run past DEVICE_SUITE_TIMEOUT seconds (default 900) is stopped and fails.
# Exits 1 if any run failed.
set -u
cd "$(dirname "$0")/.."

RUNS=(
  # docs/checks.md's runs.
  'canonical_gpu_filter_boundary'
  'canonical_gpu_temporal_work'
  'canonical_gpu_temporal_amr'
  'TEMPORAL_AMR_LOSSY_WALLS=1 canonical_gpu_temporal_amr'
  'canonical_gpu_temporal_forced'
  'canonical_gpu_temporal_timing'
  'canonical_gpu_temporal_timing --fixed'
  'canonical_gpu_temporal_timing --outgoing'
  'canonical_gpu_temporal_timing --oscillator'
  'canonical_gpu_driven_document'
  'DRIVEN_WALLS=outgoing canonical_gpu_driven_document'
  'DRIVEN_LAW=van-der-pol canonical_gpu_driven_document'
  'OSCILLATOR_MEDIUM=kink OSCILLATOR_FILTER=1 canonical_gpu_oscillator'
  'OSCILLATOR_MEDIUM=van-der-pol OSCILLATOR_COMPOSE=junction OSCILLATOR_SHORT_WAVE=0.3 canonical_gpu_oscillator'
  'NONLINEAR_FORCED=1 NONLINEAR_SHORT_WAVE=0.3 canonical_gpu_nonlinear'
  'OSCILLATOR_MEDIUM=van-der-pol OSCILLATOR_FIELD_LAW=kerr OSCILLATOR_COMPOSE=junction OSCILLATOR_SHORT_WAVE=0.3 canonical_gpu_oscillator'
  'LONG_RUN_SCENE="Self-sustained emitter" LONG_RUN_SHORT_WAVE=0.25 LONG_RUN_STEPS=1000 canonical_gpu_long_run'
  'OSCILLATOR_HANDOFF=remesh canonical_gpu_oscillator_handoff'
  'FAILURE_LAW=phi4 canonical_gpu_nonlinear_failure'
  # The walls under the default trapezoid wall step, and beside it the half
  # kicks it replaced, which stay for comparison.
  'canonical_gpu_timing --second-order'
  'canonical_gpu_timing --second-order --half-kicks'
  'canonical_gpu_timing --second-order --loss --forcing'
  'canonical_gpu_timing --second-order --loss --forcing --half-kicks'
  'canonical_gpu_timing --first-order --loss --forcing'
  'canonical_gpu_timing --second-order --leapfrog'
  'canonical_gpu_timing --second-order --law-patch'
  'canonical_gpu_handoff --second-order --half-kicks'
  'canonical_gpu_handoff --second-order --pinned-sides --half-kicks'
  'LONG_RUN_SCENE="Luneburg lens" LONG_RUN_WALLS=second LONG_RUN_STEPS=1000 canonical_gpu_long_run'
  'FORCED_WALLS=second canonical_gpu_temporal_forced'
  'FORCED_WALLS=second FORCED_WALL_STEP=half-kicks canonical_gpu_temporal_forced'
  'FORCED_PINNED_LOSS=1 FORCED_WALLS=second canonical_gpu_temporal_forced'
  'NONLINEAR_WALL=1 canonical_gpu_nonlinear'
  'NONLINEAR_WALL=2 canonical_gpu_nonlinear'
  'NONLINEAR_WALL=2 NONLINEAR_WALL_STEP=half-kicks canonical_gpu_nonlinear'
  'NONLINEAR_WALL=2 NONLINEAR_FORCED=1 canonical_gpu_nonlinear'
  'NONLINEAR_WALL=2 NONLINEAR_FORCED=1 NONLINEAR_LINEAR=1 canonical_gpu_nonlinear'
  'OSCILLATOR_MEDIUM=kerr OSCILLATOR_COMPOSE=wall1 canonical_gpu_oscillator'
  'OSCILLATOR_MEDIUM=kerr OSCILLATOR_COMPOSE=wall2-pins canonical_gpu_oscillator'
  'OSCILLATOR_MEDIUM=van-der-pol OSCILLATOR_FIELD_LAW=kerr OSCILLATOR_COMPOSE=wall2-pins canonical_gpu_oscillator'
  'DRIVEN_WALLS=outgoing DRIVEN_WALL_STEP=half-kicks canonical_gpu_driven_document'
  'TEMPORAL_AMR_LOSSY_WALLS=1 TEMPORAL_AMR_WALL_STEP=half-kicks canonical_gpu_temporal_amr'
  'LONG_RUN_WALLS=second LONG_RUN_STEPS=1000 canonical_gpu_long_run'
  'LONG_RUN_WALLS=second LONG_RUN_WALL_STEP=half-kicks LONG_RUN_STEPS=1000 canonical_gpu_long_run'
  # Each other example as it stands, and the rejections and rollbacks the
  # device performs for real.
  'canonical_gpu_complementary_mode'
  'canonical_gpu_complementary_mode --baffle'
  'canonical_gpu_complementary_mode --remesh'
  'canonical_gpu_complementary_mode --baffle --remesh'
  'canonical_gpu_cavity_modes --physics mechanical'
  'canonical_gpu_cavity_modes --physics tm'
  'canonical_gpu_cavity_modes --physics te'
  'canonical_gpu_handoff'
  'canonical_gpu_handoff --source --prescribed --edit'
  'canonical_gpu_handoff --failure'
  'canonical_gpu_handoff --second-order'
  'canonical_gpu_handoff --second-order --pinned-sides'
  'canonical_gpu_handoff --pulse'
  'canonical_gpu_live_events'
  'canonical_gpu_long_run'
  'canonical_gpu_nonlinear'
  'canonical_gpu_nonlinear_failure'
  'canonical_gpu_oscillator'
  'canonical_gpu_oscillator_handoff'
  'canonical_gpu_pulse'
  'canonical_gpu_pulse --long'
  'canonical_gpu_temporal'
  'canonical_gpu_temporal --resident-filter'
  'canonical_gpu_temporal_consumer'
  'canonical_gpu_temporal_gate'
  'canonical_gpu_temporal_handoff'
  'canonical_gpu_temporal_live_source'
  'canonical_gpu_temporal_pulse'
  'canonical_gpu_temporal_rollback'
  'canonical_gpu_timing'
  'canonical_gpu_timing --failure'
  'canonical_gpu_timing --loss --law-patch'
  # The app itself, driven through its own handlers (src/ui/e2e.rs).
  'FUNFERN_E2E=cavity funfern-fixtures'
  'FUNFERN_E2E=cavity-batches funfern-fixtures'
  'FUNFERN_E2E=switch funfern-fixtures'
  'FUNFERN_E2E=handoff funfern-fixtures'
  'FUNFERN_E2E=handoff-driven funfern-fixtures'
  'FUNFERN_E2E=remesh funfern-fixtures'
  'FUNFERN_E2E=handoff-memory-rebuild funfern-fixtures'
  'FUNFERN_E2E=handoff-memory-remesh funfern-fixtures'
  'FUNFERN_E2E=handoff-memory-edit funfern-fixtures'
  'FUNFERN_E2E=handoff-memory-timestep funfern-fixtures'
  'FUNFERN_E2E=handoff-memory-flux funfern-fixtures'
  'FUNFERN_E2E=handoff-memory-running funfern-fixtures'
  'FUNFERN_E2E=rejection funfern-fixtures'
  'FUNFERN_E2E=device-loss funfern-fixtures'
  'FUNFERN_E2E=observables funfern-fixtures'
  'FUNFERN_E2E=far-field funfern-fixtures'
  'FUNFERN_E2E=far-field-late funfern-fixtures'
  'FUNFERN_E2E=handoff-memory-running-late funfern-fixtures'
)

pattern=${1:-}
limit=${DEVICE_SUITE_TIMEOUT:-900}
selected=()
for run in "${RUNS[@]}"; do
  if [[ -z $pattern || $run =~ $pattern ]]; then
    selected+=("$run")
  fi
done
if [[ ${#selected[@]} -eq 0 ]]; then
  echo "no run matches '$pattern'" >&2
  exit 2
fi

cargo build -p funfern-app --release --locked --examples || exit 1
cargo build -p funfern-app --release --locked --bin funfern-fixtures || exit 1

logs="target/device-suite/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$logs/bin"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
# The app the fixtures run, linked and asked for a fixture that does not
# exist: the driver refuses by name and exits 2, where the plain app would
# open and wait.
ln -f target/release/funfern-fixtures "$logs/bin/funfern-fixtures"
probe="$scratch/probe"
mkdir -p "$probe/Library/Application Support/funfern"
env HOME="$probe" FUNFERN_E2E=no-such-fixture \
  perl -e 'alarm shift @ARGV; exec @ARGV or die "$!\n"' 60 \
  "$logs/bin/funfern-fixtures" >"$logs/00-driver-check.log" 2>&1
if [[ $? -ne 2 ]] || ! grep -q "no fixture named" "$logs/00-driver-check.log"; then
  echo "the app the suite linked has no fixture driver; see $logs/00-driver-check.log" >&2
  exit 1
fi
results=()
failed=0
index=0
for run in "${selected[@]}"; do
  index=$((index + 1))
  # The run's words, quotes respected: assignments, the example, its flags.
  eval "set -- $run"
  assignments=()
  while [[ $# -gt 0 && $1 == *=* ]]; do
    assignments+=("$1")
    shift
  done
  example=$1
  shift
  binary="target/release/examples/$example"
  [[ $example == funfern-fixtures ]] && binary=target/release/funfern-fixtures
  # The binary as built, kept by a link whatever a later build renames in.
  ln -f "$binary" "$logs/bin/$example"
  binary="$logs/bin/$example"
  log="$logs/$(printf %02d "$index")-$example.log"
  home="$scratch/$index"
  mkdir -p "$home/Library/Application Support/funfern"
  printf '[%d/%d] %s\n' "$index" "${#selected[@]}" "$run"
  started=$SECONDS
  # Perl's alarm outlives the exec and ends the run with SIGALRM at the limit,
  # where macOS has no `timeout`.
  env HOME="$home" ${assignments[@]+"${assignments[@]}"} \
    perl -e 'alarm shift @ARGV; exec @ARGV or die "$!\n"' "$limit" \
    "$binary" "$@" >"$log" 2>&1
  status=$?
  if [[ $status -eq 142 ]]; then
    echo "stopped after ${limit} s" >>"$log"
  fi
  elapsed=$((SECONDS - started))
  if [[ $status -eq 0 ]]; then
    verdict=pass
  else
    verdict="FAIL ($status)"
    failed=$((failed + 1))
  fi
  results+=("$(printf '%-10s %5d s  %s' "$verdict" "$elapsed" "$run")")
done

echo
printf '%s\n' "${results[@]}"
echo
echo "$((${#selected[@]} - failed)) of ${#selected[@]} passed; logs in $logs"
[[ $failed -eq 0 ]]
