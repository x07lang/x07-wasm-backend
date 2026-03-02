#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/phase9_examples"

rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}" build/phase9_examples .x07-wasm/incidents

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

check_report_exit_code_and_has_code() {
  local report_path="$1"
  local want_exit_code="$2"
  local want_code="$3"
  "$PYTHON" - "$report_path" "$want_exit_code" "$want_code" <<'PY'
import json
import pathlib
import sys

report = pathlib.Path(sys.argv[1])
want_exit_code = int(sys.argv[2])
want_code = sys.argv[3]

doc = json.loads(report.read_text(encoding="utf-8"))
exit_code = int(doc.get("exit_code", 999))
if exit_code != want_exit_code:
    print(f"{report}: expected exit_code={want_exit_code}, got {exit_code}", file=sys.stderr)
    print(report.read_text(encoding="utf-8")[:2000], file=sys.stderr)
    sys.exit(1)

codes = []
for d in doc.get("diagnostics", []):
    if isinstance(d, dict) and isinstance(d.get("code"), str):
        codes.append(d["code"])
if want_code not in codes:
    print(f"{report}: expected diagnostic {want_code}; got {codes}", file=sys.stderr)
    print(report.read_text(encoding="utf-8")[:2000], file=sys.stderr)
    sys.exit(1)
print("ok:", report)
PY
}

echo "==> phase9_examples: build bundle (device_dev)"
bundle_dir="${OUT_DIR}/device_dev_bundle"
x07-wasm device build \
  --index arch/device/index.x07device.json \
  --profile device_dev \
  --out-dir "${bundle_dir}" \
  --clean \
  --strict \
  --json --report-out build/phase9_examples/device.build.device_dev.json --quiet-json
test -f "${bundle_dir}/bundle.manifest.json"
test -f "${bundle_dir}/ui/reducer.wasm"

echo "==> phase9_examples: verify bundle (device_dev)"
x07-wasm device verify \
  --dir "${bundle_dir}" \
  --json --report-out build/phase9_examples/device.verify.device_dev.json --quiet-json

echo "==> phase9_examples: device run (unsupported target => expected diag code)"
set +e
x07-wasm device run \
  --bundle "${bundle_dir}" \
  --target ios \
  --json --report-out build/phase9_examples/device.run.unsupported_target.json --quiet-json
code=$?
set -e
if [ "$code" -eq 0 ]; then
  echo "expected nonzero exit for unsupported target" >&2
  exit 1
fi
check_report_exit_code_and_has_code build/phase9_examples/device.run.unsupported_target.json 3 X07WASM_DEVICE_RUN_TARGET_UNSUPPORTED

echo "==> phase9_examples: device package (unsupported target => expected diag code)"
set +e
x07-wasm device package \
  --bundle "${bundle_dir}" \
  --target ios \
  --out-dir "${OUT_DIR}/package_unsupported_target" \
  --json --report-out build/phase9_examples/device.package.unsupported_target.json --quiet-json
code=$?
set -e
if [ "$code" -eq 0 ]; then
  echo "expected nonzero exit for unsupported target" >&2
  exit 1
fi
check_report_exit_code_and_has_code build/phase9_examples/device.package.unsupported_target.json 3 X07WASM_DEVICE_PACKAGE_FAILED

host_tool="${X07_DEVICE_HOST_DESKTOP:-}"
if [ -z "${host_tool}" ]; then
  if command -v x07-device-host-desktop >/dev/null 2>&1; then
    host_tool="$(command -v x07-device-host-desktop)"
  fi
fi

if [ -z "${host_tool}" ]; then
  echo "==> phase9_examples: host tool missing; skipping device package/run smoke"
  echo "phase9_examples: PASS (host smoke skipped)"
  exit 0
fi

echo "==> phase9_examples: device package (desktop)"
export X07_DEVICE_HOST_DESKTOP="${host_tool}"
package_dir="${OUT_DIR}/device_dev_package"
x07-wasm device package \
  --bundle "${bundle_dir}" \
  --target desktop \
  --out-dir "${package_dir}" \
  --json --report-out build/phase9_examples/device.package.device_dev.json --quiet-json
test -f "${package_dir}/package.manifest.json"

echo "==> phase9_examples: device run smoke (optional)"
if [ "${X07WASM_DEVICE_RUN_SMOKE:-0}" = "1" ]; then
  x07-wasm device run \
    --bundle "${bundle_dir}" \
    --target desktop \
    --headless-smoke \
    --json --report-out build/phase9_examples/device.run.smoke.json --quiet-json
else
  echo "device run smoke skipped (set X07WASM_DEVICE_RUN_SMOKE=1 to enable)"
fi

echo "phase9_examples: PASS"
