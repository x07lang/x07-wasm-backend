#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/showcase_mobile"
REPORT_DIR="${ROOT_DIR}/build/showcase_mobile"
INCIDENTS_DIR="${ROOT_DIR}/.x07-wasm/incidents"
WEB_UI_DIST_DIR="${OUT_DIR}/web_ui_debug"
TRACE_DIR="${ROOT_DIR}/tests/web_ui"

cd "${ROOT_DIR}"

WEB_UI_CASES=(
  "${TRACE_DIR}/notes_edit.trace.json"
  "${TRACE_DIR}/storage_reload.trace.json"
  "${TRACE_DIR}/sync_success.trace.json"
  "${TRACE_DIR}/sync_error.trace.json"
)

rm -rf "${OUT_DIR}" "${REPORT_DIR}"
mkdir -p "${OUT_DIR}" "${REPORT_DIR}" "${INCIDENTS_DIR}"

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

run_json() {
  local report_path="$1"
  shift
  "$@" --json --report-out "${report_path}" --quiet-json
}

resolve_stdlib_lock() {
  local x07_path
  x07_path="$(command -v x07)"
  local candidates=(
    "${ROOT_DIR}/stdlib.lock"
    "$(dirname "${x07_path}")/stdlib.lock"
    "$(cd "$(dirname "${x07_path}")/.." && pwd)/stdlib.lock"
  )
  local cand
  for cand in "${candidates[@]}"; do
    if [[ -f "${cand}" ]]; then
      printf '%s\n' "${cand}"
      return 0
    fi
  done
  echo "could not resolve stdlib.lock for x07 test" >&2
  return 1
}

check_device_package_manifest() {
  local package_dir="$1"
  local bundle_manifest_path="$2"
  local want_target="$3"
  local want_payload_path="$4"

  "$PYTHON" - "$package_dir" "$bundle_manifest_path" "$want_target" "$want_payload_path" <<'PY'
import hashlib
import json
import pathlib
import re
import sys

package_dir = pathlib.Path(sys.argv[1])
bundle_manifest = pathlib.Path(sys.argv[2])
want_target = sys.argv[3]
want_payload_path = sys.argv[4]

manifest_path = package_dir / "package.manifest.json"
doc = json.loads(manifest_path.read_text(encoding="utf-8"))

if doc.get("schema_version") != "x07.device.package.manifest@0.1.0":
    raise SystemExit(f"{manifest_path}: bad schema_version: {doc.get('schema_version')!r}")
if doc.get("kind") != "device_package":
    raise SystemExit(f"{manifest_path}: bad kind: {doc.get('kind')!r}")
if doc.get("target") != want_target:
    raise SystemExit(f"{manifest_path}: bad target: want={want_target!r}, got={doc.get('target')!r}")

want_bundle_sha = hashlib.sha256(bundle_manifest.read_bytes()).hexdigest()
got_bundle_sha = doc.get("bundle_manifest_sha256")
if got_bundle_sha != want_bundle_sha:
    raise SystemExit(
        f"{manifest_path}: bundle_manifest_sha256 mismatch: want={want_bundle_sha}, got={got_bundle_sha}"
    )

pkg = doc.get("package")
if not isinstance(pkg, dict):
    raise SystemExit(f"{manifest_path}: missing package object")

if pkg.get("kind") != "dir":
    raise SystemExit(f"{manifest_path}: expected package.kind='dir', got {pkg.get('kind')!r}")
path = pkg.get("path")
if path != want_payload_path:
    raise SystemExit(f"{manifest_path}: expected package.path={want_payload_path!r}, got {path!r}")
if not isinstance(path, str) or not path:
    raise SystemExit(f"{manifest_path}: invalid package.path: {path!r}")
if path.startswith(("/", "\\")) or re.match(r"^[A-Za-z]:", path):
    raise SystemExit(f"{manifest_path}: package.path must be relative: {path!r}")
if ".." in pathlib.PurePosixPath(path).parts:
    raise SystemExit(f"{manifest_path}: package.path must not contain '..': {path!r}")

payload = package_dir / path
if not payload.exists():
    raise SystemExit(f"{manifest_path}: package payload missing: {payload}")

print("ok:", manifest_path)
PY
}

check_dir_has_no_x07_tokens() {
  local dir_path="$1"
  "$PYTHON" - "$dir_path" <<'PY'
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
needle = b"__X07_"
hits = []

for p in sorted(root.rglob("*")):
    if not p.is_file():
        continue
    if p.name == ".DS_Store" or p.name.startswith("._"):
        continue
    if needle in p.read_bytes():
        hits.append(str(p))

if hits:
    raise SystemExit(f"found unreplaced template tokens under {root}:\n" + "\n".join(hits[:50]))

print("ok: no template tokens under", root)
PY
}

check_embedded_bundle_files() {
  local bundle_dir="$1"
  local project_assets_dir="$2"
  local platform_name="$3"

  "$PYTHON" - "$bundle_dir" "$project_assets_dir" "$platform_name" <<'PY'
import hashlib
import pathlib
import sys

bundle_dir = pathlib.Path(sys.argv[1])
assets_dir = pathlib.Path(sys.argv[2])
platform_name = sys.argv[3]

want_manifest = bundle_dir / "bundle.manifest.json"
want_wasm = bundle_dir / "ui" / "reducer.wasm"
got_manifest = assets_dir / "bundle.manifest.json"
got_wasm = assets_dir / "ui" / "reducer.wasm"

host_files = ["index.html", "bootstrap.js", "main.mjs", "app-host.mjs"]

for p in [got_manifest, got_wasm]:
    if not p.is_file():
        raise SystemExit(f"{platform_name}: missing embedded file: {p}")

for rel in host_files:
    p = assets_dir / rel
    if not p.is_file():
        raise SystemExit(f"{platform_name}: missing host asset: {p}")

def sha(p: pathlib.Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()

if sha(want_manifest) != sha(got_manifest):
    raise SystemExit(f"{platform_name}: embedded bundle.manifest.json sha256 mismatch")
if sha(want_wasm) != sha(got_wasm):
    raise SystemExit(f"{platform_name}: embedded reducer.wasm sha256 mismatch")

print(f"ok: {platform_name} embedded bundle files")
PY
}

run_web_ui_checks() {
  echo "==> validate frontend"
  x07 pkg lock --project frontend/x07.json
  x07 check --project frontend/x07.json
  x07 test --manifest frontend/tests/tests.json --stdlib-lock "$(resolve_stdlib_lock)"

  echo "==> validate web-ui/device scaffolding"
  run_json "${REPORT_DIR}/web_ui.profile.validate.json" \
    x07-wasm web-ui profile validate
  run_json "${REPORT_DIR}/device.index.validate.json" \
    x07-wasm device index validate
  run_json "${REPORT_DIR}/device.profile.validate.json" \
    x07-wasm device profile validate

  echo "==> build web-ui dist"
  run_json "${REPORT_DIR}/web_ui.build.web_ui_debug.json" \
    x07-wasm web-ui build \
      --project frontend/x07.json \
      --profile web_ui_debug \
      --out-dir "${WEB_UI_DIST_DIR}" \
      --clean \
      --strict

  echo "==> replay reducer traces"
  local case_path=""
  local base=""
  for case_path in "${WEB_UI_CASES[@]}"; do
    base="$(basename "${case_path}" .trace.json)"
    run_json "${REPORT_DIR}/web_ui.test.${base}.json" \
      x07-wasm web-ui test \
        --dist-dir "${WEB_UI_DIST_DIR}" \
        --case "${case_path}" \
        --strict \
        --incidents-dir "${INCIDENTS_DIR}"
  done
}

build_device_bundle() {
  local profile_id="$1"
  local bundle_dir="${OUT_DIR}/${profile_id}_bundle"

  echo "==> build ${profile_id} bundle"
  run_json "${REPORT_DIR}/device.build.${profile_id}.json" \
    x07-wasm device build \
      --index arch/device/index.x07device.json \
      --profile "${profile_id}" \
      --out-dir "${bundle_dir}" \
      --clean \
      --strict

  echo "==> verify ${profile_id} bundle"
  run_json "${REPORT_DIR}/device.verify.${profile_id}.json" \
    x07-wasm device verify \
      --dir "${bundle_dir}"
}

run_desktop_smoke() {
  local profile_id="device_desktop_dev"
  local bundle_dir="${OUT_DIR}/${profile_id}_bundle"

  build_device_bundle "${profile_id}"

  echo "==> run desktop headless smoke"
  run_json "${REPORT_DIR}/device.run.${profile_id}.json" \
    x07-wasm device run \
      --bundle "${bundle_dir}" \
      --target desktop \
      --headless-smoke
}

package_mobile_target() {
  local profile_id="$1"
  local target="$2"
  local payload_path="$3"
  local embedded_assets_path="$4"
  local platform_name="$5"
  local bundle_dir="${OUT_DIR}/${profile_id}_bundle"
  local package_dir="${OUT_DIR}/${profile_id}_package"

  build_device_bundle "${profile_id}"

  echo "==> package ${platform_name} project"
  run_json "${REPORT_DIR}/device.package.${profile_id}.json" \
    x07-wasm device package \
      --bundle "${bundle_dir}" \
      --target "${target}" \
      --out-dir "${package_dir}"

  check_device_package_manifest "${package_dir}" "${bundle_dir}/bundle.manifest.json" "${target}" "${payload_path}"
  check_embedded_bundle_files "${bundle_dir}" "${package_dir}/${embedded_assets_path}" "${platform_name}"
  check_dir_has_no_x07_tokens "${package_dir}/${payload_path}"
}

run_web_ui_checks
run_desktop_smoke
package_mobile_target \
  device_ios_dev \
  ios \
  ios_project \
  ios_project/X07DeviceApp/x07 \
  iOS
package_mobile_target \
  device_android_dev \
  android \
  android_project \
  android_project/app/src/main/assets/x07 \
  Android

echo "check_showcase_mobile: PASS"
