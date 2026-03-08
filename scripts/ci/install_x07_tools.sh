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

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

wasm_version="$(
  sed -nE 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*$/\1/p' "${ROOT_DIR}/crates/x07-wasm/Cargo.toml" | head -n1
)"
if [[ -z "${wasm_version}" ]]; then
  echo "failed to read crates/x07-wasm/Cargo.toml package.version" >&2
  exit 1
fi

device_host_tag="$(
  "${PYTHON}" - "${ROOT_DIR}" "${wasm_version}" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
version = sys.argv[2]
compat_path = root / "releases" / "compat" / f"{version}.json"
doc = json.loads(compat_path.read_text(encoding="utf-8"))
device_host = doc.get("device_host")
if not isinstance(device_host, str) or not device_host:
    raise SystemExit(f"{compat_path}: missing device_host")
print(f"v{device_host}")
PY
)"

tools=("$@")
if [[ ${#tools[@]} -eq 0 ]]; then
  tools=(x07 x07-host-runner x07-os-runner x07-device-host-desktop)
fi

for tool in "${tools[@]}"; do
  if [[ "${tool}" == "x07-device-host-desktop" ]]; then
    cargo install --locked --git https://github.com/x07lang/x07-device-host.git --tag "${device_host_tag}" "${tool}"
  else
    cargo install --locked --git https://github.com/x07lang/x07.git --tag "${x07_tag}" "${tool}"
  fi
  if ! "${tool}" --version >/dev/null 2>&1; then
    "${tool}" --help >/dev/null
  fi
done

if command -v x07 >/dev/null 2>&1; then
  x07_bin_dir="$(dirname "$(command -v x07)")"
  base_url="https://raw.githubusercontent.com/x07lang/x07/${x07_tag}"
  for lock_name in stdlib.lock stdlib.os.lock; do
    curl -fsSL --retry 10 --retry-all-errors --retry-delay 2 \
      "${base_url}/${lock_name}" \
      -o "${x07_bin_dir}/${lock_name}"
  done
fi
