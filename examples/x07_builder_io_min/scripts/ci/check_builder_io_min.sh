#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WASM_ROOT="$(cd "${ROOT_DIR}/../.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/builder_io_min"
REPORT_DIR="${ROOT_DIR}/build/builder_io_min"
WEB_UI_DIST_DIR="${OUT_DIR}/web_ui_debug"
TRACE_DIR="${ROOT_DIR}/tests/web_ui"

cd "${ROOT_DIR}"

source "${WASM_ROOT}/scripts/ci/device_host_common.sh"

run_json() {
  local report_path="$1"
  shift
  "$@" --json --report-out "${report_path}" --quiet-json
}

rm -rf "${OUT_DIR}" "${REPORT_DIR}"
mkdir -p "${OUT_DIR}" "${REPORT_DIR}"

WEB_UI_CASES=(
  "${TRACE_DIR}/import_export.trace.json"
  "${TRACE_DIR}/clipboard_roundtrip.trace.json"
  "${TRACE_DIR}/share_success.trace.json"
  "${TRACE_DIR}/negative.trace.json"
)

echo "==> validate frontend"
x07 check --project frontend/x07.json

echo "==> validate scaffolding"
run_json "${REPORT_DIR}/web_ui.profile.validate.json" x07-wasm web-ui profile validate
run_json "${REPORT_DIR}/device.index.validate.json" x07-wasm device index validate
run_json "${REPORT_DIR}/device.profile.validate.json" x07-wasm device profile validate

echo "==> build web-ui dist"
run_json "${REPORT_DIR}/web_ui.build.web_ui_debug.json" \
  x07-wasm web-ui build \
    --project frontend/x07.json \
    --profile web_ui_debug \
    --out-dir "${WEB_UI_DIST_DIR}" \
    --clean \
    --strict

echo "==> replay reducer traces"
for case_path in "${WEB_UI_CASES[@]}"; do
  base="$(basename "${case_path}" .trace.json)"
  run_json "${REPORT_DIR}/web_ui.test.${base}.json" \
    x07-wasm web-ui test \
      --dist-dir "${WEB_UI_DIST_DIR}" \
      --case "${case_path}" \
      --strict
done

build_and_verify() {
  local profile_id="$1"
  local bundle_dir="${OUT_DIR}/${profile_id}_bundle"
  run_json "${REPORT_DIR}/device.build.${profile_id}.json" \
    x07-wasm device build \
      --index arch/device/index.x07device.json \
      --profile "${profile_id}" \
      --out-dir "${bundle_dir}" \
      --clean \
      --strict
  run_json "${REPORT_DIR}/device.verify.${profile_id}.json" \
    x07-wasm device verify \
      --dir "${bundle_dir}"
}

echo "==> build and verify desktop bundle"
build_and_verify device_desktop_dev

if [ "${X07WASM_DEVICE_RUN_SMOKE:-1}" = "0" ]; then
  echo "desktop smoke skipped (X07WASM_DEVICE_RUN_SMOKE=0)"
else
  host_tool="$(resolve_x07_device_host_desktop "${WASM_ROOT}")" || {
    echo "compatible x07-device-host-desktop not found; build ../x07-device-host or set X07_DEVICE_HOST_DESKTOP" >&2
    exit 1
  }
  export X07_DEVICE_HOST_DESKTOP="${host_tool}"
  run_json "${REPORT_DIR}/device.run.device_desktop_dev.json" \
    x07-wasm device run \
      --bundle "${OUT_DIR}/device_desktop_dev_bundle" \
      --target desktop \
      --headless-smoke
fi

echo "==> build, verify, and package iOS bundle"
build_and_verify device_ios_dev
run_json "${REPORT_DIR}/device.package.device_ios_dev.json" \
  x07-wasm device package \
    --bundle "${OUT_DIR}/device_ios_dev_bundle" \
    --target ios \
    --out-dir "${OUT_DIR}/device_ios_dev_package"

echo "==> build, verify, and package Android bundle"
build_and_verify device_android_dev
run_json "${REPORT_DIR}/device.package.device_android_dev.json" \
  x07-wasm device package \
    --bundle "${OUT_DIR}/device_android_dev_bundle" \
    --target android \
    --out-dir "${OUT_DIR}/device_android_dev_package"

echo "check_builder_io_min: PASS"
