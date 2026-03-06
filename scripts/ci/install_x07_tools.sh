#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

x07_tag="$(
  sed -nE 's/^channel[[:space:]]*=[[:space:]]*"([^"]+)".*$/\1/p' "${ROOT_DIR}/x07-toolchain.toml" | head -n1
)"
if [[ -z "${x07_tag}" ]]; then
  echo "failed to read x07-toolchain.toml toolchain.channel" >&2
  exit 1
fi

tools=("$@")
if [[ ${#tools[@]} -eq 0 ]]; then
  tools=(x07 x07-host-runner)
fi

for tool in "${tools[@]}"; do
  cargo install --locked --git https://github.com/x07lang/x07.git --tag "${x07_tag}" "${tool}"
  if ! "${tool}" --version >/dev/null 2>&1; then
    "${tool}" --help >/dev/null
  fi
done
