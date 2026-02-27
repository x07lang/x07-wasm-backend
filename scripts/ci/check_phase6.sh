#!/usr/bin/env bash
set -euo pipefail

# Phase-6 top-level gate:
#  1) Runs the full Phase-5 gate (Phase-6 is layered on Phase-5)
#  2) Runs Phase-6 examples-only gate
#  3) Enforces Phase-5 + Phase-6 diagnostic allowlists on non-ok reports

mkdir -p build/phase6 dist/phase6 .x07-wasm/incidents

echo "==> phase6: Phase-5 hardening loop"
bash scripts/ci/check_phase5.sh

echo "==> phase6: Phase-6 examples gate"
bash scripts/ci/check_phase6_examples.sh

echo "==> phase6: diagnostic allowlists"
bash scripts/ci/check_phase6_diagcodes.sh build

echo "phase6: PASS"

