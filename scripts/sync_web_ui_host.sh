#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 /path/to/x07-web-ui" >&2
  exit 2
fi

SRC_ROOT="$(cd "$1" && pwd)"

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

"${PYTHON}" scripts/vendor_x07_web_ui.py update --src "${SRC_ROOT}"
"${PYTHON}" scripts/vendor_x07_web_ui.py check --src "${SRC_ROOT}"

HOST_DIR="vendor/x07-web-ui/host"
IOS_HOST_DIR="crates/x07-wasm/src/support/mobile/ios/template/X07DeviceApp/x07"
ANDROID_HOST_DIR="crates/x07-wasm/src/support/mobile/android/template/app/src/main/assets/x07"

for host_file in index.html bootstrap.js main.mjs app-host.mjs; do
  cp "${HOST_DIR}/${host_file}" "${IOS_HOST_DIR}/${host_file}"
  cp "${HOST_DIR}/${host_file}" "${ANDROID_HOST_DIR}/${host_file}"
done

echo "ok: synced vendored x07-web-ui snapshot"
