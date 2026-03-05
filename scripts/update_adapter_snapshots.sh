#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/adapter_snapshots.lib.sh"

if ! command -v docker >/dev/null 2>&1; then
  echo "docker not found on PATH" >&2
  exit 1
fi

rust_channel="$(
  sed -nE 's/^channel[[:space:]]*=[[:space:]]*"([^"]+)".*$/\1/p' "${ROOT_DIR}/rust-toolchain.toml" | head -n1
)"
if [[ "${rust_channel}" == "" ]]; then
  echo "failed to read rust-toolchain.toml toolchain.channel" >&2
  exit 1
fi

echo "==> building guest adapters via docker (linux/amd64, rust:${rust_channel})"
docker run --rm --platform linux/amd64 \
  -u "$(id -u):$(id -g)" \
  -v "${ROOT_DIR}":/work \
  -w /work \
  "rust:${rust_channel}" \
  bash -lc '
    set -euo pipefail
    export PATH=/usr/local/cargo/bin:$PATH
    rustup target add wasm32-wasip2 >/dev/null
    cargo build --release --locked --target wasm32-wasip2 --manifest-path guest/http-adapter/Cargo.toml
    cargo build --release --locked --target wasm32-wasip2 --manifest-path guest/cli-adapter/Cargo.toml
    cargo build --release --locked --target wasm32-wasip2 --manifest-path guest/web-ui-adapter/Cargo.toml
  '

echo "==> copying embedded snapshots"
for adapter in http cli web-ui; do
  guest_out="${ROOT_DIR}/$(x07_adapter_guest_output_path "${adapter}")"
  embedded="${ROOT_DIR}/$(x07_adapter_embedded_snapshot_path "${adapter}")"

  if [[ ! -f "${guest_out}" ]]; then
    echo "missing guest build output: ${guest_out}" >&2
    exit 1
  fi
  cp "${guest_out}" "${embedded}"
done

echo "==> verifying embedded snapshots"
for adapter in http cli web-ui; do
  guest_out="${ROOT_DIR}/$(x07_adapter_guest_output_path "${adapter}")"
  embedded="${ROOT_DIR}/$(x07_adapter_embedded_snapshot_path "${adapter}")"
  if ! cmp -s "${guest_out}" "${embedded}"; then
    echo "snapshot copy verification failed for ${adapter}" >&2
    exit 1
  fi
done

echo "ok: updated embedded adapter snapshots"
