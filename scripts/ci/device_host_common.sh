#!/usr/bin/env bash

resolve_x07_device_host_desktop() {
  local repo_root="$1"
  local snapshot_path="${repo_root}/vendor/x07-device-host/host_abi.snapshot.json"
  local python_bin=""

  if command -v python3 >/dev/null 2>&1; then
    python_bin="python3"
  elif command -v python >/dev/null 2>&1; then
    python_bin="python"
  else
    echo "python not found on PATH" >&2
    return 1
  fi

  local expected_hash=""
  if [ -f "${snapshot_path}" ]; then
    expected_hash="$("${python_bin}" - "${snapshot_path}" <<'PY'
import json
import pathlib
import sys

snapshot = pathlib.Path(sys.argv[1])
doc = json.loads(snapshot.read_text(encoding="utf-8"))
value = doc.get("host_abi_hash")
if isinstance(value, str):
    print(value)
PY
)"
  fi

  local candidates=()
  local candidate=""
  local abi_hash=""

  if [ -n "${X07_DEVICE_HOST_DESKTOP:-}" ]; then
    candidates+=("${X07_DEVICE_HOST_DESKTOP}")
  fi

  for candidate in \
    "${repo_root}/../x07-device-host/target/debug/x07-device-host-desktop" \
    "${repo_root}/../x07-device-host/target/release/x07-device-host-desktop"
  do
    if [ -x "${candidate}" ]; then
      candidates+=("${candidate}")
    fi
  done

  if command -v x07-device-host-desktop >/dev/null 2>&1; then
    candidates+=("$(command -v x07-device-host-desktop)")
  fi

  for candidate in "${candidates[@]}"; do
    if ! abi_hash="$("${candidate}" --host-abi-hash 2>/dev/null)"; then
      continue
    fi
    case "${abi_hash}" in
      [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]*)
        if [ "${#abi_hash}" -ne 64 ]; then
          continue
        fi
        if [ -n "${expected_hash}" ] && [ "${abi_hash}" != "${expected_hash}" ]; then
          continue
        fi
        printf '%s\n' "${candidate}"
        return 0
        ;;
    esac
  done

  return 1
}
