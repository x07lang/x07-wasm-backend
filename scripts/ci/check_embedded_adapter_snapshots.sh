#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/adapter_snapshots.lib.sh"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "skipping embedded adapter snapshot drift check on non-Linux host"
  exit 0
fi

adapters=("$@")
if [[ "${#adapters[@]}" == "0" ]]; then
  adapters=(http cli web-ui)
fi

for adapter in "${adapters[@]}"; do
  manifest="${ROOT_DIR}/$(x07_adapter_manifest_path "${adapter}")"
  guest_out="${ROOT_DIR}/$(x07_adapter_guest_output_path "${adapter}")"
  embedded="${ROOT_DIR}/$(x07_adapter_embedded_snapshot_path "${adapter}")"

  cargo build --release --locked --target wasm32-wasip2 --manifest-path "${manifest}"

  if ! cmp -s "${guest_out}" "${embedded}"; then
    echo "ERROR: embedded $(basename "${embedded}") is out of sync with guest/${adapter}-adapter output" >&2
    echo "hint: run: bash scripts/update_adapter_snapshots.sh" >&2
    exit 1
  fi
done

