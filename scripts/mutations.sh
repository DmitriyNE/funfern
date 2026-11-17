#!/usr/bin/env bash
# Proves the tests bite: applies each mutation under tests/mutations/, one
# deliberate defect as a patch, runs the test its manifest names, and requires
# the test to fail for the reason the manifest expects. A mutation counts as
# detected only when the tree builds with it, the named test runs to the check
# it targets and fails there with output matching the manifest's pattern. A
# compile error, a panic elsewhere, a missing device, a timeout, or the test
# not passing before the patch each count as "not detected" and fail the run.
#
#   scripts/mutations.sh              every mutation
#   scripts/mutations.sh band-fill    the ones whose name matches a pattern
#   MUTATION_SHARD=2/4 scripts/mutations.sh
#                                     the second of four shards: of the ones
#                                     matched, in their folders' order, every
#                                     fourth from the second
#
# The nightly runs four shards in parallel: one after another the 143
# mutations of 2026-11-15 take about nine hours, past a runner's six, and
# four take 2.1 to 2.5 hours each by the times measured on 2026-11-08 and 14.
#
# Each mutation is a folder holding `patch.diff`, applied with `git apply`,
# and `manifest`, with lines `test: <command>`, `expect: <extended regex the
# failing output must match>`, `needs: cpu|device` and `about: <one line>`.
# A run past MUTATION_TIMEOUT seconds (default 1800) is stopped. The tree
# must be clean to start, and is left clean. Exits 1 if any mutation is not
# detected, and 2 if no mutation matches the pattern.
set -u
cd "$(dirname "$0")/.."
pattern=${1:-}
limit=${MUTATION_TIMEOUT:-1800}
shard=${MUTATION_SHARD:-1/1}
if ! [[ $shard =~ ^([0-9]+)/([0-9]+)$ ]] || ((BASH_REMATCH[1] < 1 || BASH_REMATCH[1] > BASH_REMATCH[2])); then
  echo "MUTATION_SHARD is k/n with 1 <= k <= n, not '$shard'" >&2
  exit 2
fi
shard_index=$((BASH_REMATCH[1] - 1))
shard_count=${BASH_REMATCH[2]}
# Whether the mutation in this folder is one to run: its name matches the
# pattern, and its place among those that match falls in this shard.
selected=()
position=0
for folder in tests/mutations/*/; do
  name=$(basename "$folder")
  [[ -z $pattern || $name =~ $pattern ]] || continue
  ((position % shard_count == shard_index)) && selected+=("$folder")
  position=$((position + 1))
done
if [[ ${#selected[@]} -eq 0 ]]; then
  echo "no mutation matches '$pattern' in shard $shard" >&2
  exit 2
fi
if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "the tree is not clean; commit or stash first" >&2
  exit 2
fi
logs="target/mutations/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$logs"
results=()
undetected=0
field() { sed -n "s/^$1: //p" "$2" | head -1; }
run() {
  # The command, bounded, its output to the log.
  perl -e 'alarm shift @ARGV; exec "/bin/bash", "-c", $ARGV[0] or die "$!\n"' "$limit" "$1" >"$2" 2>&1
}
for folder in "${selected[@]}"; do
  name=$(basename "$folder")
  manifest="$folder/manifest"
  test=$(field test "$manifest")
  expect=$(field expect "$manifest")
  needs=$(field needs "$manifest")
  about=$(field about "$manifest")
  printf '%s: %s\n' "$name" "$about"
  verdict=""
  if run "$test" "$logs/$name-before.log"; then
    :
  else
    verdict="not detected: the test does not pass before the patch"
  fi
  if [[ -z $verdict ]]; then
    if ! git apply "$folder/patch.diff" >"$logs/$name-apply.log" 2>&1; then
      verdict="not detected: the patch does not apply"
    fi
  fi
  if [[ -z $verdict ]]; then
    if run "$test" "$logs/$name-after.log"; then
      verdict="not detected: the test still passes"
    elif grep -qE "error(\[E[0-9]+\])?: " "$logs/$name-after.log" && grep -q "could not compile" "$logs/$name-after.log"; then
      verdict="not detected: the mutated tree does not build"
    elif [[ $(stat -c %s "$logs/$name-after.log" 2>/dev/null || stat -f %z "$logs/$name-after.log") -eq 0 ]]; then
      verdict="not detected: no output (a timeout, or no device)"
    elif grep -qE -- "$expect" "$logs/$name-after.log"; then
      verdict="detected ($needs)"
    else
      verdict="not detected: failed for another reason (no match for '$expect')"
    fi
    git apply -R "$folder/patch.diff" >>"$logs/$name-apply.log" 2>&1 || {
      echo "could not revert $name; the tree needs attention" >&2
      exit 3
    }
  fi
  if ! git diff --quiet; then
    echo "the tree is not clean after $name" >&2
    exit 3
  fi
  [[ $verdict == detected* ]] || undetected=$((undetected + 1))
  results+=("$(printf '%-48s %s' "$name" "$verdict")")
  # Each verdict as it lands, so a run cut short still says how far it came,
  # and what an undetected one's runs last said, since the job's output may
  # be all that is kept of them.
  printf '  %s\n' "$verdict"
  if [[ $verdict != detected* ]]; then
    for log in "$logs/$name-before.log" "$logs/$name-after.log"; do
      if [[ -s $log ]]; then
        echo "  --- $(basename "$log"), its last lines:"
        tail -20 "$log" | sed 's/^/    /'
      fi
    done
  fi
done
echo
printf '%s\n' "${results[@]}"
echo
echo "$(( ${#results[@]} - undetected )) of ${#results[@]} detected (shard $shard); logs in $logs"
[[ $undetected -eq 0 ]]
