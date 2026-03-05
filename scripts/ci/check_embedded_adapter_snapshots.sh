#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/adapter_snapshots.lib.sh"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "skipping embedded adapter snapshot drift check on non-Linux host"
  exit 0
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "docker not found on PATH (required for adapter snapshot drift check)" >&2
  exit 1
fi

adapters=("$@")
if [[ "${#adapters[@]}" == "0" ]]; then
  adapters=(http cli web-ui)
fi

rust_channel="$(
  sed -nE 's/^channel[[:space:]]*=[[:space:]]*"([^"]+)".*$/\1/p' "${ROOT_DIR}/rust-toolchain.toml" | head -n1
)"
if [[ "${rust_channel}" == "" ]]; then
  echo "failed to read rust-toolchain.toml toolchain.channel" >&2
  exit 1
fi

docker run --rm --platform linux/amd64 \
  -u "$(id -u):$(id -g)" \
  -v "${ROOT_DIR}":/work \
  -w /work \
  "rust:${rust_channel}" \
  bash -lc '
    set -euo pipefail
    export PATH=/usr/local/cargo/bin:$PATH
    # shellcheck source=/dev/null
    source scripts/adapter_snapshots.lib.sh
    rustup target add wasm32-wasip2 >/dev/null
    export CARGO_TARGET_DIR=/tmp/x07-adapter-snapshots

    adapters=("$@")
    if [[ "${#adapters[@]}" == "0" ]]; then
      adapters=(http cli web-ui)
    fi

    for adapter in "${adapters[@]}"; do
      cargo build --release --locked --target wasm32-wasip2 --manifest-path "$(x07_adapter_manifest_path "${adapter}")"
    done

    for adapter in "${adapters[@]}"; do
      out="/tmp/x07-adapter-snapshots/wasm32-wasip2/release/$(basename "$(x07_adapter_guest_output_path "${adapter}")")"
      embedded="$(x07_adapter_embedded_snapshot_path "${adapter}")"
      if ! cmp -s "${out}" "${embedded}"; then
        echo "ERROR: embedded $(basename "${embedded}") is out of sync with guest/${adapter}-adapter output" >&2
        echo "sha256 (built, embedded):" >&2
        sha256sum "${out}" "${embedded}" >&2 || true
        echo "hint: run: bash scripts/update_adapter_snapshots.sh" >&2
        exit 1
      fi
    done
  ' -- "${adapters[@]}"
