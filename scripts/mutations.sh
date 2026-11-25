#!/usr/bin/env bash
# Proves the tests bite: applies each mutation under tests/mutations/, one
# deliberate defect as a patch, runs the test its manifest names, and requires
# the test to fail for the reason the manifest expects. A mutation counts as
# detected only when the tree builds with it, the named test runs to the check
# it targets and fails there with output matching the manifest's pattern. A
# compile error, a panic elsewhere, a missing device, a timeout, a run ended
# by a signal, or the test not passing before the patch each count as "not
# detected" and fail the run.
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
# A run past MUTATION_TIMEOUT seconds (default 1800) is stopped, with every
# process it started, and is not detected whatever it printed: it may have
# printed the failure on its way to a hang. The tree must be clean to start,
# and is left clean. Exits 1 if any mutation is not
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
# The command, its output to the log, in a process group of its own. Past the
# limit the group is stopped, TERM and ten seconds on KILL, a line in the log
# says so, and the status is 124; otherwise the status is the command's own,
# 128 and the signal's number when a signal ended it. Until 2026-11-25 an
# alarm stopped the command's own process alone, so a test binary hung under
# cargo ran on beside the next mutations, and the verdict read the output
# alone, so a run that printed its failure and then hung counted as detected.
run() {
  perl -MPOSIX=WNOHANG -e '
    my ($limit, $command) = @ARGV;
    my $pid = fork() // die "fork: $!\n";
    if ($pid == 0) {
      setpgrp(0, 0);
      exec "/bin/bash", "-c", $command or die "exec: $!\n";
    }
    setpgrp($pid, $pid);
    # Stopping the runner stops the group too.
    $SIG{$_} = sub { kill "KILL", -$pid; exit 130 } for qw(INT TERM HUP);
    my $deadline = time + $limit;
    while (waitpid($pid, WNOHANG) == 0) {
      if (time >= $deadline) {
        kill "TERM", -$pid;
        for (1 .. 100) {
          waitpid($pid, WNOHANG);
          last unless kill 0, -$pid;
          select undef, undef, undef, 0.1;
        }
        kill "KILL", -$pid;
        waitpid($pid, 0);
        print "\nmutations.sh: stopped after $limit s, with every process it started\n";
        exit 124;
      }
      select undef, undef, undef, 0.2;
    }
    exit($? & 127 ? 128 + ($? & 127) : $? >> 8);
  ' "$limit" "$1" >"$2" 2>&1
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
  run "$test" "$logs/$name-before.log"
  status=$?
  if ((status == 124)); then
    verdict="not detected: the test timed out before the patch"
  elif ((status != 0)); then
    verdict="not detected: the test does not pass before the patch"
  fi
  if [[ -z $verdict ]]; then
    if ! git apply "$folder/patch.diff" >"$logs/$name-apply.log" 2>&1; then
      verdict="not detected: the patch does not apply"
    fi
  fi
  if [[ -z $verdict ]]; then
    # How the run ended comes before what it printed. 124 is also what
    # scripts/e2e-fixture.sh returns when its own `timeout` stops a fixture;
    # the log's last line says which.
    run "$test" "$logs/$name-after.log"
    status=$?
    if ((status == 0)); then
      verdict="not detected: the test still passes"
    elif ((status == 124)); then
      verdict="not detected: timed out"
    elif ((status > 128)); then
      verdict="not detected: ended by signal $((status - 128))"
    elif grep -qE "error(\[E[0-9]+\])?: " "$logs/$name-after.log" && grep -q "could not compile" "$logs/$name-after.log"; then
      verdict="not detected: the mutated tree does not build"
    elif [[ $(stat -c %s "$logs/$name-after.log" 2>/dev/null || stat -f %z "$logs/$name-after.log") -eq 0 ]]; then
      verdict="not detected: no output (no device, perhaps)"
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
