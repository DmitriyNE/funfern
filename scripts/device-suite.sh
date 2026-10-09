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
# Logs go to target/device-suite/<time>/. A run past DEVICE_SUITE_TIMEOUT
# seconds (default 900) is stopped and fails. Exits 1 if any run failed.
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
  # Each other example as it stands, and the rejections and rollbacks the
  # device performs for real.
  'canonical_gpu_handoff'
  'canonical_gpu_handoff --source --prescribed --edit'
  'canonical_gpu_handoff --failure'
  'canonical_gpu_handoff --second-order'
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
  # The app itself, driven through its own handlers (src/ui/e2e.rs).
  'FUNFERN_E2E=cavity funfern-app'
  'FUNFERN_E2E=cavity-batches funfern-app'
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
cargo build -p funfern-app --release --locked --features e2e --bin funfern-app || exit 1

logs="target/device-suite/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$logs"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
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
  [[ $example == funfern-app ]] && binary=target/release/funfern-app
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
