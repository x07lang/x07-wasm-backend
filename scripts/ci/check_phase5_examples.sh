#!/usr/bin/env bash
set -euo pipefail

# Phase-5 examples-only gate:
# - examples/solve_pure_spin (fuel budget exceeded + golden IO)
# - examples/app_min         (app build -> app pack -> app verify + negative verify tests)
#
# This script intentionally avoids any dependency on other examples, contracts,
# or toolchain checks. It assumes Phase-5 commands/flags exist:
#   - x07-wasm run --max-fuel
#   - x07-wasm app pack
#   - x07-wasm app verify
#
# Expected Phase-5 diagnostic codes:
#   - X07WASM_BUDGET_EXCEEDED_CPU_FUEL
#   - X07WASM_APP_VERIFY_DIGEST_MISMATCH
#   - X07WASM_APP_VERIFY_HEADERS_INVALID

mkdir -p build/phase5_examples dist/phase5_examples .x07-wasm/incidents

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
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
ok = bool(doc.get("ok"))
exit_code = int(doc.get("exit_code", -1))
if not ok or exit_code != 0:
    print("report not ok:", p)
    print("ok=", ok, "exit_code=", exit_code)
    print(json.dumps(doc.get("diagnostics", []), indent=2))
    sys.exit(1)
print("ok:", p)
PY
}

check_report_exit_code_and_has_code() {
  local report_path="$1"
  local expected_exit_code="$2"
  local expected_code="$3"
  "$PYTHON" - "$report_path" "$expected_exit_code" "$expected_code" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
want_exit = int(sys.argv[2])
want_code = sys.argv[3]
doc = json.loads(p.read_text(encoding="utf-8"))
exit_code = int(doc.get("exit_code", -1))
if exit_code != want_exit:
    print("unexpected exit_code:", exit_code, "expected:", want_exit)
    print("report:", p)
    print(json.dumps(doc.get("diagnostics", []), indent=2))
    sys.exit(1)
codes = [d.get("code") for d in doc.get("diagnostics", []) if isinstance(d, dict)]
if want_code not in codes:
    print("missing diagnostic code:", want_code)
    print("got codes:", codes)
    print("report:", p)
    print(json.dumps(doc.get("diagnostics", []), indent=2))
    sys.exit(1)
print("ok:", p, "exit_code=", want_exit, "has", want_code)
PY
}

get_app_build_bundle_manifest_path() {
  local report_path="$1"
  "$PYTHON" - "$report_path" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
bm = doc.get("result", {}).get("stdout_json", {}).get("bundle_manifest", {})
path = bm.get("path")
if not isinstance(path, str) or not path:
    print("missing result.stdout_json.bundle_manifest.path", file=sys.stderr)
    sys.exit(1)
print(path)
PY
}

get_app_pack_manifest_path() {
  local report_path="$1"
  "$PYTHON" - "$report_path" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
pm = doc.get("result", {}).get("stdout_json", {}).get("pack_manifest", {})
path = pm.get("path")
if not isinstance(path, str) or not path:
    print("missing result.stdout_json.pack_manifest.path", file=sys.stderr)
    sys.exit(1)
print(path)
PY
}

corrupt_first_pack_asset_file() {
  local pack_manifest_path="$1"
  "$PYTHON" - "$pack_manifest_path" <<'PY'
import json, pathlib, sys

manifest = pathlib.Path(sys.argv[1])
doc = json.loads(manifest.read_text(encoding="utf-8"))

assets = doc.get("assets", [])
if not isinstance(assets, list) or len(assets) < 1:
    print("pack manifest has no assets:", manifest, file=sys.stderr)
    sys.exit(1)

# Find first asset with file.path
asset0 = None
for a in assets:
    if isinstance(a, dict) and isinstance(a.get("file"), dict) and isinstance(a["file"].get("path"), str):
        asset0 = a
        break
if asset0 is None:
    print("pack manifest assets missing file.path", file=sys.stderr)
    sys.exit(1)

rel = asset0["file"]["path"]
p = (manifest.parent / rel).resolve()
if not p.is_file():
    print("asset file missing:", p, file=sys.stderr)
    sys.exit(1)

# Corrupt deterministically by appending one byte
p.write_bytes(p.read_bytes() + b"\x00")
print(str(p))
PY
}

remove_wasm_content_type_header_in_manifest() {
  local pack_manifest_path="$1"
  "$PYTHON" - "$pack_manifest_path" <<'PY'
import json, pathlib, sys

manifest = pathlib.Path(sys.argv[1])
doc = json.loads(manifest.read_text(encoding="utf-8"))

assets = doc.get("assets", [])
if not isinstance(assets, list):
    print("pack manifest assets not a list", file=sys.stderr)
    sys.exit(1)

wasm_asset = None
for a in assets:
    if not isinstance(a, dict):
        continue
    serve_path = a.get("serve_path")
    filep = (a.get("file") or {}).get("path") if isinstance(a.get("file"), dict) else None
    sp = serve_path if isinstance(serve_path, str) else ""
    fp = filep if isinstance(filep, str) else ""
    if sp.endswith(".wasm") or fp.endswith(".wasm"):
        wasm_asset = a
        break

if wasm_asset is None:
    print("no .wasm asset found in pack manifest:", manifest, file=sys.stderr)
    sys.exit(1)

hdrs = wasm_asset.get("headers", [])
if not isinstance(hdrs, list):
    print("wasm asset headers not a list", file=sys.stderr)
    sys.exit(1)

new_hdrs = []
removed = 0
for h in hdrs:
    if isinstance(h, dict) and isinstance(h.get("k"), str) and h.get("k").lower() == "content-type":
        removed += 1
        continue
    new_hdrs.append(h)

# Write back modified headers
wasm_asset["headers"] = new_hdrs

# Compact JSON (Phase-0 style: deterministic-ish output; ordering preserved as inserted)
manifest.write_text(json.dumps(doc, separators=(",", ":"), ensure_ascii=False) + "\n", encoding="utf-8")

if removed < 1:
    print("warning: no content-type header was removed (verify should still fail if it requires it)")
print("ok: removed content-type headers:", removed)
PY
}

echo "==> phase5_examples: build solve_pure_spin"
x07-wasm build --project examples/solve_pure_spin/x07.json --profile wasm_release \
  --out dist/phase5_examples/solve_pure_spin.wasm \
  --artifact-out dist/phase5_examples/solve_pure_spin.wasm.manifest.json \
  --json --report-out build/phase5_examples/build.solve_pure_spin.json --quiet-json
require_report_ok build/phase5_examples/build.solve_pure_spin.json
test -f dist/phase5_examples/solve_pure_spin.wasm

echo "==> phase5_examples: run solve_pure_spin (golden success)"
x07-wasm run --wasm dist/phase5_examples/solve_pure_spin.wasm \
  --input examples/solve_pure_spin/tests/fixtures/in_small.bin \
  --output-out dist/phase5_examples/solve_pure_spin.out_small.bin \
  --max-output-bytes 1024 \
  --max-fuel 1000000 \
  --json --report-out build/phase5_examples/run.solve_pure_spin.small.json --quiet-json
require_report_ok build/phase5_examples/run.solve_pure_spin.small.json
cmp dist/phase5_examples/solve_pure_spin.out_small.bin examples/solve_pure_spin/tests/fixtures/out_small.bin

echo "==> phase5_examples: run solve_pure_spin (expected fuel budget exceeded)"
set +e
x07-wasm run --wasm dist/phase5_examples/solve_pure_spin.wasm \
  --input examples/solve_pure_spin/tests/fixtures/in.bin \
  --output-out dist/phase5_examples/solve_pure_spin.out_large.bin \
  --max-output-bytes 1024 \
  --max-fuel 10000 \
  --json --report-out build/phase5_examples/run.solve_pure_spin.fuel_exceeded.json --quiet-json
code=$?
set -e
if [ "$code" -ne 4 ]; then
  echo "expected exit code 4 for fuel budget exceeded, got $code" >&2
  exit 1
fi
check_report_exit_code_and_has_code build/phase5_examples/run.solve_pure_spin.fuel_exceeded.json 4 X07WASM_BUDGET_EXCEEDED_CPU_FUEL

echo "==> phase5_examples: app_min build"
rm -rf dist/phase5_examples/app_min
x07-wasm app build --profile-file examples/app_min/app_release.json --out-dir dist/phase5_examples/app_min --clean \
  --json --report-out build/phase5_examples/app.build.app_min.json --quiet-json
require_report_ok build/phase5_examples/app.build.app_min.json

BUNDLE_MANIFEST="$(get_app_build_bundle_manifest_path build/phase5_examples/app.build.app_min.json)"
test -f "$BUNDLE_MANIFEST"

echo "==> phase5_examples: app_min pack"
rm -rf dist/phase5_examples/app_min.pack
x07-wasm app pack --bundle-manifest "$BUNDLE_MANIFEST" --out-dir dist/phase5_examples/app_min.pack --profile-id app_min_release \
  --json --report-out build/phase5_examples/app.pack.app_min.json --quiet-json
require_report_ok build/phase5_examples/app.pack.app_min.json

PACK_MANIFEST="$(get_app_pack_manifest_path build/phase5_examples/app.pack.app_min.json)"
test -f "$PACK_MANIFEST"

echo "==> phase5_examples: app_min verify (success)"
x07-wasm app verify --pack-manifest "$PACK_MANIFEST" \
  --json --report-out build/phase5_examples/app.verify.app_min.json --quiet-json
require_report_ok build/phase5_examples/app.verify.app_min.json

PACK_DIR="$(dirname "$PACK_MANIFEST")"
PACK_MANIFEST_NAME="$(basename "$PACK_MANIFEST")"

echo "==> phase5_examples: app_min verify negative (digest mismatch)"
BAD_DIGEST_DIR="dist/phase5_examples/app_min.pack.bad_digest"
rm -rf "$BAD_DIGEST_DIR"
cp -a "$PACK_DIR" "$BAD_DIGEST_DIR"
BAD_DIGEST_MANIFEST="${BAD_DIGEST_DIR}/${PACK_MANIFEST_NAME}"

corrupt_first_pack_asset_file "$BAD_DIGEST_MANIFEST" >/dev/null

set +e
x07-wasm app verify --pack-manifest "$BAD_DIGEST_MANIFEST" \
  --json --report-out build/phase5_examples/app.verify.app_min.bad_digest.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for digest mismatch, got $code" >&2
  exit 1
fi
check_report_exit_code_and_has_code build/phase5_examples/app.verify.app_min.bad_digest.json 1 X07WASM_APP_VERIFY_DIGEST_MISMATCH

echo "==> phase5_examples: app_min verify negative (missing wasm content-type header)"
BAD_HEADERS_DIR="dist/phase5_examples/app_min.pack.bad_headers"
rm -rf "$BAD_HEADERS_DIR"
cp -a "$PACK_DIR" "$BAD_HEADERS_DIR"
BAD_HEADERS_MANIFEST="${BAD_HEADERS_DIR}/${PACK_MANIFEST_NAME}"

remove_wasm_content_type_header_in_manifest "$BAD_HEADERS_MANIFEST" >/dev/null

set +e
x07-wasm app verify --pack-manifest "$BAD_HEADERS_MANIFEST" \
  --json --report-out build/phase5_examples/app.verify.app_min.bad_headers.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for headers invalid, got $code" >&2
  exit 1
fi
check_report_exit_code_and_has_code build/phase5_examples/app.verify.app_min.bad_headers.json 1 X07WASM_APP_VERIFY_HEADERS_INVALID

echo "phase5_examples: PASS"
