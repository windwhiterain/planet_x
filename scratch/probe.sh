#!/usr/bin/env bash
# Sweep a config variant against the long-horizon health probes.
# Usage: bash scratch/probe.sh scratch/v1.ron
set -euo pipefail
CFG="$1"
export PLANET_X_CONFIG="$CFG"
cd "$(dirname "$0")/.."
echo "=== probing $CFG ==="
cargo test --test longhorizon probe_sanction -- --ignored --nocapture 2>&1 \
  | grep -E "^seed |result:"
echo "--- peak ---"
cargo test --test longhorizon probe_peak -- --ignored --nocapture 2>&1 \
  | grep -E "^seed |result:"
