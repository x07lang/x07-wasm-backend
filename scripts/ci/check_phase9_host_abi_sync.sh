#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

python3 "${ROOT_DIR}/scripts/vendor_x07_device_host_abi.py" check

