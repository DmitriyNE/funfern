#!/usr/bin/env bash
# Runs the app's end-to-end fixtures (src/ui/e2e.rs) in the browser on every
# way the bundle is hosted, and reports each run's verdict: the threaded
# bundle under the isolation headers a server sends; the same with its worker
# pool refused, as on iOS Safari, the work on the main thread; the bundle as
# GitHub Pages serves it, under /funfern/ without the headers, through the
# isolation service worker's one reload, cold and warm; and the
# single-threaded bundle served that way. Every run first holds the page to
# the build this checkout is. Chrome unless PLAYWRIGHT_CHANNEL says otherwise.
#
#   scripts/browser-suite.sh             build the three bundles, run everything
#   scripts/browser-suite.sh --no-build  run against the bundles as they are
#   scripts/browser-suite.sh pages       the runs whose name matches a pattern
#
# The bundles take about five minutes each to build; a run a minute or two.
# Logs go to target/browser-suite/<time>/. Exits 1 if any run failed.
set -u
cd "$(dirname "$0")/.."

build=1
pattern=
for argument in "$@"; do
  case $argument in
    --no-build) build=0 ;;
    *) pattern=$argument ;;
  esac
done
export PLAYWRIGHT_CHANNEL=${PLAYWRIGHT_CHANNEL:-chrome}

# Name, npm script, the bundle it serves.
RUNS=(
  'isolated test:e2e dist-e2e'
  'nopool test:e2e:nopool dist-e2e'
  'pages test:e2e:pages dist-e2e-pages'
  'plain test:e2e:plain dist-e2e-plain'
)
selected=()
for run in "${RUNS[@]}"; do
  read -r name _ _ <<<"$run"
  if [[ -z $pattern || $name =~ $pattern ]]; then
    selected+=("$run")
  fi
done
if [[ ${#selected[@]} -eq 0 ]]; then
  echo "no run matches '$pattern'" >&2
  exit 2
fi

if [[ $build -eq 1 ]]; then
  TRUNK_BUILD_FEATURES=browser-threads,e2e scripts/trunk build --release --dist dist-e2e || exit 1
  TRUNK_BUILD_FEATURES=browser-threads,e2e scripts/trunk build --release --public-url /funfern/ --dist dist-e2e-pages || exit 1
  TRUNK_BUILD_FEATURES=e2e trunk build --release --public-url /funfern/ --dist dist-e2e-plain || exit 1
fi

logs="target/browser-suite/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$logs"
results=()
failed=0
for run in "${selected[@]}"; do
  read -r name script dist <<<"$run"
  if [[ ! -d $dist ]]; then
    echo "no $dist to serve: build it, or run without --no-build" >&2
    exit 2
  fi
  printf '%s (%s on %s)\n' "$name" "$script" "$dist"
  started=$SECONDS
  npm run --silent "$script" >"$logs/$name.log" 2>&1
  status=$?
  elapsed=$((SECONDS - started))
  if [[ $status -eq 0 ]]; then
    verdict=pass
  else
    verdict="FAIL ($status)"
    failed=$((failed + 1))
  fi
  results+=("$(printf '%-10s %5d s  %s' "$verdict" "$elapsed" "$name")")
  # Each fixture's verdict line, as the spec prints it.
  grep -E '^[a-z-]+ \((cold|warm)\): ' "$logs/$name.log" | sed 's/^/           /'
done

echo
printf '%s\n' "${results[@]}"
echo "$((${#selected[@]} - failed)) of ${#selected[@]} passed; logs in $logs"
[[ $failed -eq 0 ]]
