#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/phase8_examples"

rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}" build/phase8_examples .x07-wasm/incidents

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

echo "==> phase8_examples: device index validate"
x07-wasm device index validate \
  --index arch/device/index.x07device.json \
  --json --report-out build/phase8_examples/device.index.validate.json --quiet-json

echo "==> phase8_examples: device profile validate (index)"
x07-wasm device profile validate \
  --index arch/device/index.x07device.json \
  --json --report-out build/phase8_examples/device.profile.validate.index.json --quiet-json

echo "==> phase8_examples: device profile validate (profile_file)"
x07-wasm device profile validate \
  --profile-file arch/device/profiles/device_dev.json \
  --json --report-out build/phase8_examples/device.profile.validate.device_dev.json --quiet-json
x07-wasm device profile validate \
  --profile-file arch/device/profiles/device_release.json \
  --json --report-out build/phase8_examples/device.profile.validate.device_release.json --quiet-json

echo "==> phase8_examples: device build + verify"
bundle_dir="${OUT_DIR}/device_dev_bundle"
x07-wasm device build \
  --index arch/device/index.x07device.json \
  --profile device_dev \
  --out-dir "${bundle_dir}" \
  --clean \
  --strict \
  --json --report-out build/phase8_examples/device.build.device_dev.json --quiet-json
test -f "${bundle_dir}/bundle.manifest.json"
test -f "${bundle_dir}/ui/reducer.wasm"
test -f "${bundle_dir}/profile/device.profile.json"

x07-wasm device verify \
  --dir "${bundle_dir}" \
  --json --report-out build/phase8_examples/device.verify.device_dev.json --quiet-json

echo "==> phase8_examples: device provenance attest + verify (ok)"
x07-wasm device provenance attest \
  --bundle-dir "${bundle_dir}" \
  --signing-key arch/provenance/dev.ed25519.signing_key.b64 \
  --out "${bundle_dir}/provenance.dsse.json" \
  --json --report-out build/phase8_examples/device.provenance.attest.json --quiet-json
test -f "${bundle_dir}/provenance.dsse.json"

x07-wasm device provenance verify \
  --attestation "${bundle_dir}/provenance.dsse.json" \
  --bundle-dir "${bundle_dir}" \
  --trusted-public-key arch/provenance/dev.ed25519.public_key.b64 \
  --json --report-out build/phase8_examples/device.provenance.verify.ok.json --quiet-json

echo "==> phase8_examples: device verify (oversize reducer.wasm => failure + expected diag code)"
too_large_dir="${bundle_dir}.too_large"
rm -rf "${too_large_dir}"
cp -a "${bundle_dir}" "${too_large_dir}"

MAX_BUNDLE_FILE_BYTES=$((256 * 1024 * 1024))
"$PYTHON" - "${too_large_dir}" "$((MAX_BUNDLE_FILE_BYTES + 1))" <<'PY'
import os
import pathlib
import sys

bundle = pathlib.Path(sys.argv[1])
size = int(sys.argv[2])
p = bundle / "ui" / "reducer.wasm"
os.truncate(p, size)
print("oversized:", p, "size=", size)
PY

set +e
x07-wasm device verify \
  --dir "${too_large_dir}" \
  --json --report-out build/phase8_examples/device.verify.too_large.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for oversize bundle verify, got $code" >&2
  exit 1
fi

"$PYTHON" - build/phase8_examples/device.verify.too_large.json <<'PY'
import json
import pathlib
import sys

p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
codes = []
for d in doc.get("diagnostics", []):
    if isinstance(d, dict) and isinstance(d.get("code"), str):
        codes.append(d["code"])
if "X07WASM_DEVICE_BUNDLE_FILE_TOO_LARGE" not in codes:
    print("expected diagnostic X07WASM_DEVICE_BUNDLE_FILE_TOO_LARGE; got:", codes, file=sys.stderr)
    print(p.read_text(encoding="utf-8")[:2000], file=sys.stderr)
    sys.exit(1)
print("ok: expected diag code present:", p)
PY

echo "==> phase8_examples: device verify (corrupt reducer.wasm => failure + expected diag code)"
"$PYTHON" - "${bundle_dir}" <<'PY'
import pathlib
import sys

bundle = pathlib.Path(sys.argv[1])
p = bundle / "ui" / "reducer.wasm"
b = bytearray(p.read_bytes())
if not b:
    raise SystemExit(f"empty reducer.wasm: {p}")
b[0] ^= 0xFF
p.write_bytes(bytes(b))
print("corrupted:", p)
PY

set +e
x07-wasm device verify \
  --dir "${bundle_dir}" \
  --json --report-out build/phase8_examples/device.verify.corrupt.json --quiet-json
code=$?
set -e
if [ "$code" -eq 0 ]; then
  echo "expected nonzero exit code for corrupt bundle verify" >&2
  exit 1
fi

"$PYTHON" - build/phase8_examples/device.verify.corrupt.json <<'PY'
import json
import pathlib
import sys

p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
codes = []
for d in doc.get("diagnostics", []):
    if isinstance(d, dict) and isinstance(d.get("code"), str):
        codes.append(d["code"])
if "X07WASM_DEVICE_BUNDLE_SHA256_MISMATCH" not in codes:
    print("expected diagnostic X07WASM_DEVICE_BUNDLE_SHA256_MISMATCH; got:", codes, file=sys.stderr)
    print(p.read_text(encoding="utf-8")[:2000], file=sys.stderr)
    sys.exit(1)
print("ok: expected diag code present:", p)
PY

echo "==> phase8_examples: device provenance verify (corrupt reducer.wasm => failure + expected diag code)"
set +e
x07-wasm device provenance verify \
  --attestation "${bundle_dir}/provenance.dsse.json" \
  --bundle-dir "${bundle_dir}" \
  --trusted-public-key arch/provenance/dev.ed25519.public_key.b64 \
  --json --report-out build/phase8_examples/device.provenance.verify.corrupt.json --quiet-json
code=$?
set -e
if [ "$code" -eq 0 ]; then
  echo "expected nonzero exit code for corrupt device provenance verify" >&2
  exit 1
fi

"$PYTHON" - build/phase8_examples/device.provenance.verify.corrupt.json <<'PY'
import json
import pathlib
import sys

p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
codes = []
for d in doc.get("diagnostics", []):
    if isinstance(d, dict) and isinstance(d.get("code"), str):
        codes.append(d["code"])
if "X07WASM_PROVENANCE_DIGEST_MISMATCH" not in codes:
    print("expected diagnostic X07WASM_PROVENANCE_DIGEST_MISMATCH; got:", codes, file=sys.stderr)
    print(p.read_text(encoding="utf-8")[:2000], file=sys.stderr)
    sys.exit(1)
print("ok: expected diag code present:", p)
PY

echo "phase8_examples: PASS"
