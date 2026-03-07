#!/usr/bin/env bash
set -euo pipefail

# Phase-6 top-level gate:
#  1) Runs the full Phase-5 gate (Phase-6 is layered on Phase-5)
#  2) Runs Phase-6 examples-only gate
#  3) Enforces Phase-5 + Phase-6 diagnostic allowlists on non-ok reports

mkdir -p build/phase6 dist/phase6 .x07-wasm/incidents

phase6_on_exit() {
  local code=$?
  set +e

  echo "==> phase6: diagnostic allowlists"
  bash scripts/ci/check_phase6_diagcodes.sh build
  local diagcodes=$?

  if [ "$code" -eq 0 ] && [ "$diagcodes" -eq 0 ]; then
    echo "phase6: PASS"
    exit 0
  fi

  if [ "$code" -eq 0 ] && [ "$diagcodes" -ne 0 ]; then
    exit "$diagcodes"
  fi

  exit "$code"
}

trap phase6_on_exit EXIT

echo "==> phase6: Phase-5 hardening loop"
bash scripts/ci/check_phase5.sh

echo "==> phase6: Phase-6 examples gate"
bash scripts/ci/check_phase6_examples.sh

echo "==> phase6: wasm profile defaults"
bash scripts/ci/check_phase6_profile_defaults.sh

echo "==> phase6: web-ui host safety"
bash scripts/ci/check_phase6_web_ui_host_safety.sh

echo "==> phase6: web-ui focus retention"
bash scripts/ci/check_phase6_web_ui_focus.sh
