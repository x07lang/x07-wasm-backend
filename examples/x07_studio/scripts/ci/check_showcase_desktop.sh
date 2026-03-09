#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUILD_DIR="${ROOT_DIR}/build/showcase_desktop"
DIST_DIR="${ROOT_DIR}/dist/showcase_desktop"

cd "${ROOT_DIR}"

rm -rf "${BUILD_DIR}" "${DIST_DIR}"
mkdir -p "${BUILD_DIR}" "${DIST_DIR}"

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

check_ok_report() {
  local report_path="$1"
  "$PYTHON" - "$report_path" <<'PY'
import json
import pathlib
import sys

p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
if doc.get("ok") is not True:
    raise SystemExit(f"{p}: ok != true")
if int(doc.get("exit_code", 1)) != 0:
    raise SystemExit(f"{p}: exit_code != 0")
print("ok:", p)
PY
}

check_run_report() {
  local report_path="$1"
  "$PYTHON" - "$report_path" <<'PY'
import json
import pathlib
import sys

p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
if doc.get("schema_version") != "x07.wasm.device.run.report@0.1.0":
    raise SystemExit(f"{p}: unexpected schema_version={doc.get('schema_version')!r}")
if doc.get("ok") is not True:
    raise SystemExit(f"{p}: ok != true")
if int(doc.get("exit_code", 1)) != 0:
    raise SystemExit(f"{p}: exit_code != 0")
result = doc.get("result", {})
if result.get("ui_ready") is not True:
    raise SystemExit(f"{p}: result.ui_ready != true")
print("ok:", p)
PY
}

host_tool="${X07_DEVICE_HOST_DESKTOP:-}"
if [ -z "${host_tool}" ]; then
  if command -v x07-device-host-desktop >/dev/null 2>&1; then
    host_tool="$(command -v x07-device-host-desktop)"
  fi
fi
if [ -z "${host_tool}" ] || [ ! -x "${host_tool}" ]; then
  echo "x07-device-host-desktop is required for the desktop readiness smoke" >&2
  exit 1
fi

export X07_DEVICE_HOST_DESKTOP="${host_tool}"

dev_bundle="${DIST_DIR}/device_dev_bundle"
release_bundle="${DIST_DIR}/device_release_bundle"
dev_package="${DIST_DIR}/device_dev_package"
release_package="${DIST_DIR}/device_release_package"

x07 pkg lock --project frontend/x07.json
x07 check --project frontend/x07.json
x07 test --manifest frontend/tests/tests.json

x07-wasm device build \
  --index arch/device/index.x07device.json \
  --profile device_dev \
  --out-dir "${dev_bundle}" \
  --clean \
  --strict \
  --json --report-out "${BUILD_DIR}/device.build.device_dev.json" --quiet-json
check_ok_report "${BUILD_DIR}/device.build.device_dev.json"
test -f "${dev_bundle}/bundle.manifest.json"
test -f "${dev_bundle}/ui/reducer.wasm"
test -f "${dev_bundle}/profile/device.profile.json"
test -f "${dev_bundle}/profile/device.capabilities.json"
test -f "${dev_bundle}/profile/device.telemetry.profile.json"

x07-wasm device verify \
  --dir "${dev_bundle}" \
  --json --report-out "${BUILD_DIR}/device.verify.device_dev.json" --quiet-json
check_ok_report "${BUILD_DIR}/device.verify.device_dev.json"

x07-wasm device provenance attest \
  --bundle-dir "${dev_bundle}" \
  --signing-key arch/provenance/dev.ed25519.signing_key.b64 \
  --out "${dev_bundle}/provenance.dsse.json" \
  --json --report-out "${BUILD_DIR}/device.provenance.attest.device_dev.json" --quiet-json
check_ok_report "${BUILD_DIR}/device.provenance.attest.device_dev.json"
test -f "${dev_bundle}/provenance.dsse.json"

x07-wasm device provenance verify \
  --attestation "${dev_bundle}/provenance.dsse.json" \
  --bundle-dir "${dev_bundle}" \
  --trusted-public-key arch/provenance/dev.ed25519.public_key.b64 \
  --json --report-out "${BUILD_DIR}/device.provenance.verify.device_dev.json" --quiet-json
check_ok_report "${BUILD_DIR}/device.provenance.verify.device_dev.json"

x07-wasm device package \
  --bundle "${dev_bundle}" \
  --target desktop \
  --out-dir "${dev_package}" \
  --json --report-out "${BUILD_DIR}/device.package.device_dev.json" --quiet-json
check_ok_report "${BUILD_DIR}/device.package.device_dev.json"
test -f "${dev_package}/package.manifest.json"

x07-wasm device run \
  --bundle "${dev_bundle}" \
  --target desktop \
  --headless-smoke \
  --json --report-out "${BUILD_DIR}/device.run.device_dev.json" --quiet-json
check_run_report "${BUILD_DIR}/device.run.device_dev.json"

x07-wasm device build \
  --index arch/device/index.x07device.json \
  --profile device_release \
  --out-dir "${release_bundle}" \
  --clean \
  --strict \
  --json --report-out "${BUILD_DIR}/device.build.device_release.json" --quiet-json
check_ok_report "${BUILD_DIR}/device.build.device_release.json"
test -f "${release_bundle}/bundle.manifest.json"
test -f "${release_bundle}/ui/reducer.wasm"
test -f "${release_bundle}/profile/device.profile.json"
test -f "${release_bundle}/profile/device.capabilities.json"
test -f "${release_bundle}/profile/device.telemetry.profile.json"

x07-wasm device verify \
  --dir "${release_bundle}" \
  --json --report-out "${BUILD_DIR}/device.verify.device_release.json" --quiet-json
check_ok_report "${BUILD_DIR}/device.verify.device_release.json"

x07-wasm device package \
  --bundle "${release_bundle}" \
  --target desktop \
  --out-dir "${release_package}" \
  --json --report-out "${BUILD_DIR}/device.package.device_release.json" --quiet-json
check_ok_report "${BUILD_DIR}/device.package.device_release.json"
test -f "${release_package}/package.manifest.json"

echo "check_showcase_desktop: PASS"
