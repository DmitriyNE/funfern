#!/usr/bin/env bash
# PH-08: the rectangle's walls against exact outgoing multipoles. Exports the
# walls' traces and the harmonic runs from the solver into target/ph08, then
# reads them against the exact fields. Python needs numpy, scipy and mpmath;
# PH08_PYTHON names the interpreter (default python3). Run from anywhere:
#
#   PH08_PYTHON=path/to/venv/bin/python bash experiments/rectangular-radiation/run.sh
#
# Writes boundary-results.json and harmonic-results.json beside this script
# and prints both tables. About four minutes on an M1 Max, most of it the
# 168 harmonic runs.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cd "$here/../.."
python="${PH08_PYTHON:-python3}"
out="$PWD/target/ph08"
mkdir -p "$out"
FUNFERN_PH08_OUT="$out" cargo test --release -p funfern-app --lib --locked -- \
  --ignored --nocapture export_the_rectangles_wall_traces export_the_rectangles_harmonic_fields
"$python" "$here/boundary.py" "$out" "$here/boundary-results.json"
"$python" "$here/harmonic.py" "$out" "$here/harmonic-results.json"
