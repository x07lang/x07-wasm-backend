#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/phase10_examples"

rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}" build/phase10_examples .x07-wasm/incidents

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

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
bundle_doc = json.loads(bundle_manifest.read_text(encoding="utf-8"))

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

for key in ("profile", "capabilities", "telemetry_profile"):
    if doc.get(key) != bundle_doc.get(key):
        raise SystemExit(
            f"{manifest_path}: {key} mismatch: want={bundle_doc.get(key)!r}, got={doc.get(key)!r}"
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
    b = p.read_bytes()
    if needle in b:
        hits.append(str(p))

if hits:
    raise SystemExit(f"found unreplaced template tokens under {root}:\n" + "\n".join(hits[:50]))
print("ok: no template tokens under", root)
PY
}

check_ios_project_bundle_embed() {
  local bundle_dir="$1"
  local project_dir="$2"

  "$PYTHON" - "$bundle_dir" "$project_dir" <<'PY'
import hashlib
import pathlib
import sys

bundle_dir = pathlib.Path(sys.argv[1])
project_dir = pathlib.Path(sys.argv[2])

dst_root = project_dir / "X07DeviceApp" / "x07"
want_manifest = bundle_dir / "bundle.manifest.json"
want_wasm = bundle_dir / "ui" / "reducer.wasm"
want_capabilities = bundle_dir / "profile" / "device.capabilities.json"
want_telemetry_profile = bundle_dir / "profile" / "device.telemetry.profile.json"

got_manifest = dst_root / "bundle.manifest.json"
got_wasm = dst_root / "ui" / "reducer.wasm"
got_capabilities = dst_root / "profile" / "device.capabilities.json"
got_telemetry_profile = dst_root / "profile" / "device.telemetry.profile.json"

for p in [got_manifest, got_wasm, got_capabilities, got_telemetry_profile]:
    if not p.is_file():
        raise SystemExit(f"missing embedded file: {p}")

def sha(p: pathlib.Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()

if sha(want_manifest) != sha(got_manifest):
    raise SystemExit("embedded bundle.manifest.json sha256 mismatch")
if sha(want_wasm) != sha(got_wasm):
    raise SystemExit("embedded reducer.wasm sha256 mismatch")
if sha(want_capabilities) != sha(got_capabilities):
    raise SystemExit("embedded device.capabilities.json sha256 mismatch")
if sha(want_telemetry_profile) != sha(got_telemetry_profile):
    raise SystemExit("embedded device.telemetry.profile.json sha256 mismatch")

print("ok: iOS embedded bundle files")
PY
}

check_android_project_bundle_embed() {
  local bundle_dir="$1"
  local project_dir="$2"

  "$PYTHON" - "$bundle_dir" "$project_dir" <<'PY'
import hashlib
import pathlib
import sys

bundle_dir = pathlib.Path(sys.argv[1])
project_dir = pathlib.Path(sys.argv[2])

dst_root = project_dir / "app" / "src" / "main" / "assets" / "x07"
want_manifest = bundle_dir / "bundle.manifest.json"
want_wasm = bundle_dir / "ui" / "reducer.wasm"
want_capabilities = bundle_dir / "profile" / "device.capabilities.json"
want_telemetry_profile = bundle_dir / "profile" / "device.telemetry.profile.json"

got_manifest = dst_root / "bundle.manifest.json"
got_wasm = dst_root / "ui" / "reducer.wasm"
got_capabilities = dst_root / "profile" / "device.capabilities.json"
got_telemetry_profile = dst_root / "profile" / "device.telemetry.profile.json"

for p in [got_manifest, got_wasm, got_capabilities, got_telemetry_profile]:
    if not p.is_file():
        raise SystemExit(f"missing embedded file: {p}")

def sha(p: pathlib.Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()

if sha(want_manifest) != sha(got_manifest):
    raise SystemExit("embedded bundle.manifest.json sha256 mismatch")
if sha(want_wasm) != sha(got_wasm):
    raise SystemExit("embedded reducer.wasm sha256 mismatch")
if sha(want_capabilities) != sha(got_capabilities):
    raise SystemExit("embedded device.capabilities.json sha256 mismatch")
if sha(want_telemetry_profile) != sha(got_telemetry_profile):
    raise SystemExit("embedded device.telemetry.profile.json sha256 mismatch")

print("ok: Android embedded bundle files")
PY
}

echo "==> phase10_examples: build bundle (device_ios_dev)"
ios_bundle_dir="${OUT_DIR}/device_ios_dev_bundle"
x07-wasm device build \
  --index arch/device/index.x07device.json \
  --profile device_ios_dev \
  --out-dir "${ios_bundle_dir}" \
  --clean \
  --strict \
  --json --report-out build/phase10_examples/device.build.device_ios_dev.json --quiet-json
test -f "${ios_bundle_dir}/bundle.manifest.json"
test -f "${ios_bundle_dir}/ui/reducer.wasm"
test -f "${ios_bundle_dir}/profile/device.capabilities.json"
test -f "${ios_bundle_dir}/profile/device.telemetry.profile.json"

echo "==> phase10_examples: verify bundle (device_ios_dev)"
x07-wasm device verify \
  --dir "${ios_bundle_dir}" \
  --json --report-out build/phase10_examples/device.verify.device_ios_dev.json --quiet-json

echo "==> phase10_examples: device package (ios)"
ios_package_dir="${OUT_DIR}/device_ios_dev_package"
x07-wasm device package \
  --bundle "${ios_bundle_dir}" \
  --target ios \
  --out-dir "${ios_package_dir}" \
  --json --report-out build/phase10_examples/device.package.device_ios_dev.json --quiet-json
test -f "${ios_package_dir}/package.manifest.json"
test -d "${ios_package_dir}/ios_project"
check_device_package_manifest "${ios_package_dir}" "${ios_bundle_dir}/bundle.manifest.json" ios ios_project
check_ios_project_bundle_embed "${ios_bundle_dir}" "${ios_package_dir}/ios_project"
check_dir_has_no_x07_tokens "${ios_package_dir}/ios_project"

echo "==> phase10_examples: build bundle (device_android_dev)"
android_bundle_dir="${OUT_DIR}/device_android_dev_bundle"
x07-wasm device build \
  --index arch/device/index.x07device.json \
  --profile device_android_dev \
  --out-dir "${android_bundle_dir}" \
  --clean \
  --strict \
  --json --report-out build/phase10_examples/device.build.device_android_dev.json --quiet-json
test -f "${android_bundle_dir}/bundle.manifest.json"
test -f "${android_bundle_dir}/ui/reducer.wasm"
test -f "${android_bundle_dir}/profile/device.capabilities.json"
test -f "${android_bundle_dir}/profile/device.telemetry.profile.json"

echo "==> phase10_examples: verify bundle (device_android_dev)"
x07-wasm device verify \
  --dir "${android_bundle_dir}" \
  --json --report-out build/phase10_examples/device.verify.device_android_dev.json --quiet-json

echo "==> phase10_examples: device package (android)"
android_package_dir="${OUT_DIR}/device_android_dev_package"
x07-wasm device package \
  --bundle "${android_bundle_dir}" \
  --target android \
  --out-dir "${android_package_dir}" \
  --json --report-out build/phase10_examples/device.package.device_android_dev.json --quiet-json
test -f "${android_package_dir}/package.manifest.json"
test -d "${android_package_dir}/android_project"
check_device_package_manifest "${android_package_dir}" "${android_bundle_dir}/bundle.manifest.json" android android_project
check_android_project_bundle_embed "${android_bundle_dir}" "${android_package_dir}/android_project"
check_dir_has_no_x07_tokens "${android_package_dir}/android_project"

echo "==> phase10_examples: official mobile showcase"
bash examples/x07_field_notes/scripts/ci/check_showcase_mobile.sh

echo "phase10_examples: PASS"
