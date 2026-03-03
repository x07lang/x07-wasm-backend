#!/usr/bin/env bash
set -euo pipefail

if [ $# -ne 2 ]; then
  echo "usage: $0 <bundle_dir> <report_out_json>" >&2
  exit 2
fi

BUNDLE_DIR="$1"
REPORT_OUT="$2"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TAMPER_DIR="${BUNDLE_DIR}.tampered"

rm -rf "${TAMPER_DIR}"
cp -R "${BUNDLE_DIR}" "${TAMPER_DIR}"

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

"$PYTHON" - "$TAMPER_DIR" <<'PY'
import json
import pathlib
import sys

bundle_dir = pathlib.Path(sys.argv[1])
mf = bundle_dir / "bundle.manifest.json"
doc = json.loads(mf.read_text(encoding="utf-8"))
if "host" not in doc or not isinstance(doc["host"], dict):
    raise SystemExit("bundle.manifest.json missing host object")
doc["host"]["host_abi_hash"] = "0" * 64
mf.write_text(json.dumps(doc, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

set +e
x07-wasm device verify \
  --dir "${TAMPER_DIR}" \
  --json --report-out "${REPORT_OUT}" --quiet-json
CODE="$?"
set -e

if [ "${CODE}" -ne 3 ]; then
  echo "FAIL: expected process exit code 3, got ${CODE}" >&2
  exit 1
fi

"$PYTHON" - "${REPORT_OUT}" <<'PY'
import json
import pathlib
import sys

p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))

if doc.get("ok", None) is not False:
    raise SystemExit("FAIL: report.ok must be false")
if doc.get("exit_code", None) != 3:
    raise SystemExit(f"FAIL: report.exit_code must be 3, got {doc.get('exit_code')}")

diags = doc.get("diagnostics", [])
codes = [d.get("code", "") for d in diags if isinstance(d, dict)]
want = "X07WASM_DEVICE_BUNDLE_HOST_ABI_HASH_MISMATCH"
if want not in codes:
    raise SystemExit(f"FAIL: expected diagnostic code {want} not present; got={codes}")

print("ok: tampered bundle triggers ABI mismatch diagnostic")
PY

echo "phase9_host_abi_negative: PASS"

