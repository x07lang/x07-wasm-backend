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

check_device_package_manifest() {
  local package_dir="$1"
  local bundle_manifest_path="$2"
  local want_kind="$3"

  "$PYTHON" - "$package_dir" "$bundle_manifest_path" "$want_kind" <<'PY'
import hashlib
import json
import pathlib
import re
import sys

package_dir = pathlib.Path(sys.argv[1])
bundle_manifest = pathlib.Path(sys.argv[2])
want_kind = sys.argv[3]

manifest_path = package_dir / "package.manifest.json"
doc = json.loads(manifest_path.read_text(encoding="utf-8"))

if doc.get("schema_version") != "x07.device.package.manifest@0.1.0":
    raise SystemExit(f"{manifest_path}: bad schema_version: {doc.get('schema_version')!r}")
if doc.get("kind") != "device_package":
    raise SystemExit(f"{manifest_path}: bad kind: {doc.get('kind')!r}")
if doc.get("target") != "desktop":
    raise SystemExit(f"{manifest_path}: bad target: {doc.get('target')!r}")

want_bundle_sha = hashlib.sha256(bundle_manifest.read_bytes()).hexdigest()
got_bundle_sha = doc.get("bundle_manifest_sha256")
if got_bundle_sha != want_bundle_sha:
    raise SystemExit(
        f"{manifest_path}: bundle_manifest_sha256 mismatch: want={want_bundle_sha}, got={got_bundle_sha}"
    )

pkg = doc.get("package")
if not isinstance(pkg, dict):
    raise SystemExit(f"{manifest_path}: missing package object")

kind = pkg.get("kind")
path = pkg.get("path")
if kind != want_kind:
    raise SystemExit(f"{manifest_path}: package.kind mismatch: want={want_kind!r}, got={kind!r}")
if not isinstance(path, str) or not path:
    raise SystemExit(f"{manifest_path}: invalid package.path: {path!r}")
if path.startswith(("/", "\\")) or re.match(r"^[A-Za-z]:", path):
    raise SystemExit(f"{manifest_path}: package.path must be relative: {path!r}")
if ".." in pathlib.PurePosixPath(path).parts:
    raise SystemExit(f"{manifest_path}: package.path must not contain '..': {path!r}")

payload = package_dir / path
if not payload.exists():
    raise SystemExit(f"{manifest_path}: package payload missing: {payload}")

if kind == "archive":
    sha = pkg.get("sha256")
    if not isinstance(sha, str) or not re.fullmatch(r"[0-9a-f]{64}", sha):
        raise SystemExit(f"{manifest_path}: invalid package.sha256: {sha!r}")
    got_sha = hashlib.sha256(payload.read_bytes()).hexdigest()
    if got_sha != sha:
        raise SystemExit(f"{manifest_path}: zip sha256 mismatch: want={sha}, got={got_sha}")

print("ok:", manifest_path)
PY
}

check_device_run_smoke_report() {
  local report_path="$1"

  "$PYTHON" - "$report_path" <<'PY'
import json
import pathlib
import sys

report = pathlib.Path(sys.argv[1])
doc = json.loads(report.read_text(encoding="utf-8"))

if doc.get("schema_version") != "x07.wasm.device.run.report@0.1.0":
    raise SystemExit(f"{report}: bad schema_version: {doc.get('schema_version')!r}")
if doc.get("ok") is not True:
    raise SystemExit(f"{report}: ok != true: {doc.get('ok')!r}")
if int(doc.get("exit_code", 1)) != 0:
    raise SystemExit(f"{report}: exit_code != 0: {doc.get('exit_code')!r}")

result = doc.get("result", {})
if not isinstance(result, dict):
    raise SystemExit(f"{report}: result is not an object")
if result.get("ui_ready") is not True:
    raise SystemExit(f"{report}: result.ui_ready != true: {result.get('ui_ready')!r}")

print("ok:", report)
PY
}

echo "==> phase9_examples: build bundle (device_dev)"
dev_bundle_dir="${OUT_DIR}/device_dev_bundle"
x07-wasm device build \
  --index arch/device/index.x07device.json \
  --profile device_dev \
  --out-dir "${dev_bundle_dir}" \
  --clean \
  --strict \
  --json --report-out build/phase9_examples/device.build.device_dev.json --quiet-json
test -f "${dev_bundle_dir}/bundle.manifest.json"
test -f "${dev_bundle_dir}/ui/reducer.wasm"

echo "==> phase9_examples: verify bundle (device_dev)"
x07-wasm device verify \
  --dir "${dev_bundle_dir}" \
  --json --report-out build/phase9_examples/device.verify.device_dev.json --quiet-json

echo "==> phase9_examples: verify bundle (negative - tampered host ABI hash)"
bash "${ROOT_DIR}/scripts/ci/check_phase9_host_abi_negative.sh" \
  "${dev_bundle_dir}" \
  "build/phase9_examples/device.verify.tampered_host_abi.json"

echo "==> phase9_examples: build bundle (device_release)"
release_bundle_dir="${OUT_DIR}/device_release_bundle"
x07-wasm device build \
  --index arch/device/index.x07device.json \
  --profile device_release \
  --out-dir "${release_bundle_dir}" \
  --clean \
  --strict \
  --json --report-out build/phase9_examples/device.build.device_release.json --quiet-json
test -f "${release_bundle_dir}/bundle.manifest.json"
test -f "${release_bundle_dir}/ui/reducer.wasm"

echo "==> phase9_examples: verify bundle (device_release)"
x07-wasm device verify \
  --dir "${release_bundle_dir}" \
  --json --report-out build/phase9_examples/device.verify.device_release.json --quiet-json

echo "==> phase9_examples: device run (unsupported target => expected diag code)"
set +e
x07-wasm device run \
  --bundle "${dev_bundle_dir}" \
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
  --bundle "${dev_bundle_dir}" \
  --target bogus \
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
dev_package_dir="${OUT_DIR}/device_dev_package"
x07-wasm device package \
  --bundle "${dev_bundle_dir}" \
  --target desktop \
  --out-dir "${dev_package_dir}" \
  --json --report-out build/phase9_examples/device.package.device_dev.json --quiet-json
test -f "${dev_package_dir}/package.manifest.json"
check_device_package_manifest "${dev_package_dir}" "${dev_bundle_dir}/bundle.manifest.json" dir

echo "==> phase9_examples: device package (desktop, archive)"
release_package_dir="${OUT_DIR}/device_release_package"
x07-wasm device package \
  --bundle "${release_bundle_dir}" \
  --target desktop \
  --out-dir "${release_package_dir}" \
  --json --report-out build/phase9_examples/device.package.device_release.json --quiet-json
test -f "${release_package_dir}/package.manifest.json"
check_device_package_manifest "${release_package_dir}" "${release_bundle_dir}/bundle.manifest.json" archive

echo "==> phase9_examples: device run smoke (optional)"
if [ "${X07WASM_DEVICE_RUN_SMOKE:-1}" = "0" ]; then
  echo "device run smoke skipped (X07WASM_DEVICE_RUN_SMOKE=0)"
else
  x07-wasm device run \
    --bundle "${dev_bundle_dir}" \
    --target desktop \
    --headless-smoke \
    --json --report-out build/phase9_examples/device.run.smoke.json --quiet-json
  check_device_run_smoke_report build/phase9_examples/device.run.smoke.json
fi

echo "phase9_examples: PASS"
