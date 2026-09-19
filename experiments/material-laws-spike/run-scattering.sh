#!/usr/bin/env bash
set -euo pipefail
spike_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(git -C "$spike_dir" rev-parse --show-toplevel)"
spike_tmp="$(mktemp -d /tmp/funfern-scattering-spike.XXXXXX)"
printf 'Isolated baseline/build: %s\n' "$spike_tmp" >&2
git -C "$repo_dir" archive beca47e0d05c9b24945ef632b00c5cc16f80c8a0 crates/funfern-core/src | tar -x -C "$spike_tmp"
rustc --edition=2024 --crate-name funfern_core --crate-type rlib -O \
  "$spike_tmp/crates/funfern-core/src/lib.rs" -o "$spike_tmp/libfunfern_core.rlib"
rustc --edition=2024 -O "$spike_dir/export.rs" \
  --extern "funfern_core=$spike_tmp/libfunfern_core.rlib" -o "$spike_tmp/export"
"$spike_tmp/export" > "$spike_tmp/fixtures.json"
OPENBLAS_NUM_THREADS=1 VECLIB_MAXIMUM_THREADS=1 PYTHONDONTWRITEBYTECODE=1 \
  python3 "$spike_dir/scattering.py" "$spike_tmp/fixtures.json" "$@"
