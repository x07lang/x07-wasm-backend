#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

X07_WASM_BIN="${X07_WASM_BIN:-./target/release/x07-wasm}"

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
import json
import pathlib
import sys
doc = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding='utf-8'))
if doc.get('ok') is not True:
    raise SystemExit(f'report not ok: {sys.argv[1]}')
PY
}

mkdir -p build/release_ready dist/release_ready

echo "==> release_ready: app pack"
rm -rf dist/release_ready/app_min.pack
"$X07_WASM_BIN" app pack \
  --bundle-manifest dist/phase6_examples/app_min/app.bundle.json \
  --out-dir dist/release_ready/app_min.pack \
  --profile-id app_min_release \
  --json --report-out build/release_ready/app.pack.json --quiet-json
require_report_ok build/release_ready/app.pack.json
test -f dist/release_ready/app_min.pack/app.pack.json

echo "==> release_ready: app verify"
"$X07_WASM_BIN" app verify \
  --pack-manifest dist/release_ready/app_min.pack/app.pack.json \
  --json --report-out build/release_ready/app.verify.json --quiet-json
require_report_ok build/release_ready/app.verify.json

echo "==> release_ready: deploy plan"
rm -rf dist/release_ready/deploy_plan
"$X07_WASM_BIN" deploy plan \
  --pack-manifest dist/release_ready/app_min.pack/app.pack.json \
  --ops arch/app/ops/ops_release_policy_patch.json \
  --out-dir dist/release_ready/deploy_plan \
  --json --report-out build/release_ready/deploy.plan.json --quiet-json
require_report_ok build/release_ready/deploy.plan.json
test -f dist/release_ready/deploy_plan/deploy.plan.json

echo "==> release_ready: slo eval"
"$X07_WASM_BIN" slo eval \
  --profile arch/slo/slo_min.json \
  --metrics examples/app_min/tests/metrics_canary_ok.json \
  --json --report-out build/release_ready/slo.eval.json --quiet-json
require_report_ok build/release_ready/slo.eval.json

echo "==> release_ready: app regress from-incident"
incident_dir="build/release_ready/app_incident_fixture"
rm -rf build/release_ready/regress
rm -rf "${incident_dir}"
mkdir -p "${incident_dir}"
cp examples/app_fullstack_hello/tests/trace_0001.json "${incident_dir}/trace.json"
"$X07_WASM_BIN" app regress from-incident "${incident_dir}" \
  --out-dir build/release_ready/regress \
  --name app_fullstack_hello \
  --json --report-out build/release_ready/app.regress.from_incident.json --quiet-json
require_report_ok build/release_ready/app.regress.from_incident.json
test -f build/release_ready/regress/app_fullstack_hello.trace.json
test -f build/release_ready/regress/app_fullstack_hello.final.ui.json
