#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Re-run Phase0..8 gates first.
bash "${ROOT_DIR}/scripts/ci/check_phase8.sh"

# Guardrail: host ABI snapshot + Rust pin must remain in sync.
bash "${ROOT_DIR}/scripts/ci/check_phase9_host_abi_sync.sh"

# Phase9 examples: device run + package.
bash "${ROOT_DIR}/scripts/ci/check_phase9_examples.sh"

# Phase9 diagcode allowlist (Phase5..9).
bash "${ROOT_DIR}/scripts/ci/check_phase9_diagcodes.sh"
