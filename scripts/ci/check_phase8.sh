#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Re-run Phase0..7 gates first.
bash "${ROOT_DIR}/scripts/ci/check_phase7.sh"

# Phase8: ensure device-host ABI hash stays in sync with the pinned web-ui host snapshot.
bash "${ROOT_DIR}/scripts/ci/check_phase8_device_host_abi_sync.sh"

# Schema index must be complete and in sync with spec/schemas/*.schema.json.
bash "${ROOT_DIR}/scripts/ci/check_schema_index.sh"

# Phase8 examples: device contracts + bundle build/verify.
bash "${ROOT_DIR}/scripts/ci/check_phase8_examples.sh"

# Phase8 diagcode allowlist (Phase5..8).
bash "${ROOT_DIR}/scripts/ci/check_phase8_diagcodes.sh"
