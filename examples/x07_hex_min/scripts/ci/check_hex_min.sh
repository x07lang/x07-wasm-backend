#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WASM_ROOT="$(cd "${ROOT_DIR}/../.." && pwd)"
PYTHON="${PYTHON:-python3}"
OUT_DIR="${ROOT_DIR}/dist/hex_min"
REPORT_DIR="${ROOT_DIR}/build/hex_min"
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
  "${TRACE_DIR}/turn_flow.trace.json"
  "${TRACE_DIR}/clipboard_success.trace.json"
  "${TRACE_DIR}/share_export_success.trace.json"
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

check_device_verify_report() {
  local report_path="$1"
  local want_target="$2"

  "${PYTHON}" - "$report_path" "$want_target" <<'PY'
import json
import pathlib
import sys

report_path = pathlib.Path(sys.argv[1])
want_target = sys.argv[2]
doc = json.loads(report_path.read_text(encoding="utf-8"))

if doc.get("schema_version") != "x07.wasm.device.verify.report@0.2.0":
    raise SystemExit(f"{report_path}: bad schema_version {doc.get('schema_version')!r}")

result = doc.get("result", {})
summary = result.get("native_summary")
if not isinstance(summary, dict):
    raise SystemExit(f"{report_path}: missing native_summary")
if summary.get("target_kind") != want_target:
    raise SystemExit(f"{report_path}: bad target_kind {summary.get('target_kind')!r}")
if summary.get("permission_declarations") != ["haptics_present"]:
    raise SystemExit(
        f"{report_path}: bad permission_declarations {summary.get('permission_declarations')!r}"
    )

caps = summary.get("capabilities")
if not isinstance(caps, dict):
    raise SystemExit(f"{report_path}: missing capabilities summary")
for key in ("audio_playback", "files_save", "clipboard_write_text", "haptics_present", "share_present"):
    if caps.get(key) is not True:
        raise SystemExit(f"{report_path}: expected {key}=true, got {caps.get(key)!r}")
for key in ("files_pick", "files_pick_multiple", "files_drop", "clipboard_read_text"):
    if caps.get(key) is not False:
        raise SystemExit(f"{report_path}: expected {key}=false, got {caps.get(key)!r}")

readiness = result.get("release_readiness")
if readiness != {"status": "ok", "warnings": [], "errors": []}:
    raise SystemExit(f"{report_path}: unexpected release_readiness {readiness!r}")

print("ok:", report_path)
PY
}

check_device_package_report() {
  local report_path="$1"
  local want_target="$2"

  "${PYTHON}" - "$report_path" "$want_target" <<'PY'
import json
import pathlib
import sys

report_path = pathlib.Path(sys.argv[1])
want_target = sys.argv[2]
doc = json.loads(report_path.read_text(encoding="utf-8"))

if doc.get("schema_version") != "x07.wasm.device.package.report@0.2.0":
    raise SystemExit(f"{report_path}: bad schema_version {doc.get('schema_version')!r}")

result = doc.get("result", {})
summary = result.get("native_summary")
if not isinstance(summary, dict):
    raise SystemExit(f"{report_path}: missing native_summary")
if summary.get("target_kind") != want_target:
    raise SystemExit(f"{report_path}: bad target_kind {summary.get('target_kind')!r}")
if summary.get("permission_declarations") != ["haptics_present"]:
    raise SystemExit(
        f"{report_path}: bad permission_declarations {summary.get('permission_declarations')!r}"
    )

package_manifest = result.get("package_manifest", {})
package_sha = package_manifest.get("sha256")
if summary.get("package_manifest_sha256") != package_sha:
    raise SystemExit(
        f"{report_path}: package manifest sha mismatch: want={package_sha!r}, got={summary.get('package_manifest_sha256')!r}"
    )

readiness = result.get("release_readiness")
if readiness != {"status": "ok", "warnings": [], "errors": []}:
    raise SystemExit(f"{report_path}: unexpected release_readiness {readiness!r}")

print("ok:", report_path)
PY
}

check_android_native_projection() {
  local project_dir="$1"

  "${PYTHON}" - "$project_dir" <<'PY'
import pathlib
import sys

manifest_path = pathlib.Path(sys.argv[1]) / "app" / "src" / "main" / "AndroidManifest.xml"
src = manifest_path.read_text(encoding="utf-8")

if "android.permission.VIBRATE" not in src:
    raise SystemExit(f"{manifest_path}: missing generated runtime permission android.permission.VIBRATE")
if "android.permission.CAMERA" in src:
    raise SystemExit(f"{manifest_path}: unexpected android.permission.CAMERA")

print("ok: Android native capability projection")
PY
}

echo "==> build and verify desktop bundle"
build_and_verify device_desktop_dev
check_device_verify_report "${REPORT_DIR}/device.verify.device_desktop_dev.json" desktop

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
check_device_verify_report "${REPORT_DIR}/device.verify.device_ios_dev.json" ios
run_json "${REPORT_DIR}/device.package.device_ios_dev.json" \
  x07-wasm device package \
    --bundle "${OUT_DIR}/device_ios_dev_bundle" \
    --target ios \
    --out-dir "${OUT_DIR}/device_ios_dev_package"
check_device_package_report "${REPORT_DIR}/device.package.device_ios_dev.json" ios

echo "==> build, verify, and package Android bundle"
build_and_verify device_android_dev
check_device_verify_report "${REPORT_DIR}/device.verify.device_android_dev.json" android
run_json "${REPORT_DIR}/device.package.device_android_dev.json" \
  x07-wasm device package \
    --bundle "${OUT_DIR}/device_android_dev_bundle" \
    --target android \
    --out-dir "${OUT_DIR}/device_android_dev_package"
check_device_package_report "${REPORT_DIR}/device.package.device_android_dev.json" android
check_android_native_projection "${OUT_DIR}/device_android_dev_package/android_project"

echo "check_hex_min: PASS"
