#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

X07_WASM_BIN="${X07_WASM_BIN:-./target/release/x07-wasm}"

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

require_report_ok() {
  local report_path="$1"
  "$PYTHON" - "$report_path" <<'PY'
import json
import pathlib
import sys
doc = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding='utf-8'))
if doc.get('ok') is not True:
    raise SystemExit(f'report not ok: {sys.argv[1]}')
PY
}

mkdir -p build/release_ready dist/release_ready

echo "==> release_ready: slo eval"
"$X07_WASM_BIN" slo eval \
  --profile arch/slo/slo_min.json \
  --metrics arch/slo/metrics_canary_ok.json \
  --json --report-out build/release_ready/slo.eval.json --quiet-json
require_report_ok build/release_ready/slo.eval.json
