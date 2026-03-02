#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Re-run Phase0..6 gates first.
bash "${ROOT_DIR}/scripts/ci/check_phase6.sh"

# Phase7 gates: prove no clang/wasm-ld invocation is required.
bash "${ROOT_DIR}/scripts/ci/check_phase7_examples.sh"

# Phase7 diagcode allowlist (Phase5..7).
bash "${ROOT_DIR}/scripts/ci/check_phase7_diagcodes.sh"

