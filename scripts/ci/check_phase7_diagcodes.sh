#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-build}"

export X07WASM_DIAG_INCLUDE_PHASE7=1
bash "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check_phase6_diagcodes.sh" "$ROOT"

