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

echo "ok: synced vendored x07-web-ui snapshot"
