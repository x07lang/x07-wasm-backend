#!/usr/bin/env bash
set -euo pipefail

# Phase-5 top-level gate:
#  1) Runs the full Phase-5 "contracts + environment" gates
#  2) Then runs the lightweight examples-only gate:
#       scripts/ci/check_phase5_examples.sh
#
# This script assumes the Phase-5 commands exist:
#  - x07-wasm toolchain validate
#  - x07-wasm profile validate
#  - x07-wasm web-ui contracts validate
#  - x07-wasm http contracts validate
#  - x07-wasm http serve/test
#  - x07-wasm build / run
#  - x07-wasm app build / pack / verify

mkdir -p build/phase5 dist/phase5 .x07-wasm/incidents

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

echo "==> phase5: gate toolchain validate (pinned tool versions)"
x07-wasm toolchain validate --profile arch/wasm/toolchain/profiles/toolchain_ci.json \
  --json --report-out build/phase5/toolchain.validate.json --quiet-json
require_report_ok build/phase5/toolchain.validate.json

echo "==> phase5: gate wasm profile validate (includes runtime.limits)"
x07-wasm profile validate \
  --json --report-out build/phase5/profile.validate.json --quiet-json
require_report_ok build/phase5/profile.validate.json

echo "==> phase5: gate http contracts validate (schemas + fixtures, zero external validators)"
x07-wasm http contracts validate --strict \
  --json --report-out build/phase5/http.contracts.validate.json --quiet-json
require_report_ok build/phase5/http.contracts.validate.json

echo "==> phase5: gate web-ui contracts validate (strict; typed effects)"
x07-wasm web-ui contracts validate --strict \
  --json --report-out build/phase5/web-ui.contracts.validate.json --quiet-json
require_report_ok build/phase5/web-ui.contracts.validate.json

echo "==> phase5: gate vendored x07-web-ui snapshot integrity"
${PYTHON} scripts/vendor_x07_web_ui.py check

echo "==> phase5: gate web-ui build emits dist/wasm.profile.json and test can load it"
(
  web_ui_root="$(cd "vendor/x07-web-ui" && pwd)"
  export X07_WORKSPACE_ROOT="${web_ui_root}"

  counter_project="${web_ui_root}/examples/web_ui_counter/x07.json"
  counter_trace="${web_ui_root}/examples/web_ui_counter/tests/counter.trace.json"

  counter_core_dir="dist/phase5/web_ui_counter_core"
  rm -rf "${counter_core_dir}"
  x07-wasm web-ui build --project "${counter_project}" --profile web_ui_debug --out-dir "${counter_core_dir}" --clean \
    --json --report-out build/phase5/web-ui.build.counter.core.json --quiet-json
  require_report_ok build/phase5/web-ui.build.counter.core.json
  test -f "${counter_core_dir}/wasm.profile.json"

  x07-wasm web-ui test --dist-dir "${counter_core_dir}" --case "${counter_trace}" \
    --json --report-out build/phase5/web-ui.test.counter.core.json --quiet-json
  require_report_ok build/phase5/web-ui.test.counter.core.json
)

echo "==> phase5: gate http reducer runner loop (build + serve + trace replay)"
x07-wasm build --project examples/http_reducer_echo/x07.json --profile wasm_release \
  --out dist/phase5/http_reducer_echo.wasm \
  --json --report-out build/phase5/build.http_reducer_echo.json --quiet-json
require_report_ok build/phase5/build.http_reducer_echo.json

x07-wasm build --project examples/http_reducer_effect_kv/x07.json --profile wasm_release \
  --out dist/phase5/http_reducer_effect_kv.wasm \
  --json --report-out build/phase5/build.http_reducer_effect_kv.json --quiet-json
require_report_ok build/phase5/build.http_reducer_effect_kv.json

x07-wasm build --project examples/http_reducer_effect_http/x07.json --profile wasm_release \
  --out dist/phase5/http_reducer_effect_http.wasm \
  --json --report-out build/phase5/build.http_reducer_effect_http.json --quiet-json
require_report_ok build/phase5/build.http_reducer_effect_http.json

echo "==> phase5: gate http serve (canary) - kv effect loop smoke"
x07-wasm http serve --component dist/phase5/http_reducer_effect_kv.wasm --mode canary \
  --json --report-out build/phase5/http.serve.kv.canary.json --quiet-json
require_report_ok build/phase5/http.serve.kv.canary.json

echo "==> phase5: gate http serve (canary) - expected effect loop budget exceeded"
set +e
x07-wasm http serve --component dist/phase5/http_reducer_effect_kv.wasm --mode canary --max-effect-steps 1 \
  --json --report-out build/phase5/http.serve.kv.budget_loops.json --quiet-json
code=$?
set -e
if [ "$code" -ne 4 ]; then
  echo "expected exit code 4 for http effects loop budget exceeded, got $code" >&2
  exit 1
fi
check_report_exit_code_and_has_code build/phase5/http.serve.kv.budget_loops.json 4 X07WASM_BUDGET_EXCEEDED_HTTP_EFFECTS_LOOPS

echo "==> phase5: gate http test (trace replay)"
x07-wasm http test --component dist/phase5/http_reducer_echo.wasm --trace spec/fixtures/http/trace.min.json \
  --json --report-out build/phase5/http.test.echo.json --quiet-json
require_report_ok build/phase5/http.test.echo.json

x07-wasm http test --component dist/phase5/http_reducer_effect_kv.wasm --trace spec/fixtures/http/trace.effect_kv.min.json \
  --json --report-out build/phase5/http.test.effect_kv.json --quiet-json
require_report_ok build/phase5/http.test.effect_kv.json

x07-wasm http test --component dist/phase5/http_reducer_effect_http.wasm --trace spec/fixtures/http/trace.effect_http.min.json \
  --json --report-out build/phase5/http.test.effect_http.json --quiet-json
require_report_ok build/phase5/http.test.effect_http.json

echo "==> phase5: gate runtime fuel budget (x07-wasm run) - minimal smoke"
x07-wasm build --project examples/solve_pure_spin/x07.json --profile wasm_release \
  --out dist/phase5/solve_pure_spin.wasm \
  --json --report-out build/phase5/build.solve_pure_spin.json --quiet-json
require_report_ok build/phase5/build.solve_pure_spin.json
test -f dist/phase5/solve_pure_spin.wasm

set +e
x07-wasm run --wasm dist/phase5/solve_pure_spin.wasm \
  --input examples/solve_pure_spin/tests/fixtures/in.bin \
  --output-out dist/phase5/solve_pure_spin.out_large.bin \
  --max-output-bytes 1024 \
  --max-fuel 10000 \
  --json --report-out build/phase5/run.solve_pure_spin.fuel_exceeded.json --quiet-json
code=$?
set -e
if [ "$code" -ne 4 ]; then
  echo "expected exit code 4 for fuel budget exceeded, got $code" >&2
  exit 1
fi
check_report_exit_code_and_has_code build/phase5/run.solve_pure_spin.fuel_exceeded.json 4 X07WASM_BUDGET_EXCEEDED_CPU_FUEL

echo "==> phase5: gate app_min build -> pack -> verify (positive)"
rm -rf dist/phase5/app_min dist/phase5/app_min.pack
x07-wasm app build --profile-file examples/app_min/app_release.json \
  --out-dir dist/phase5/app_min --clean \
  --json --report-out build/phase5/app.build.app_min.json --quiet-json
require_report_ok build/phase5/app.build.app_min.json

BUNDLE_MANIFEST="$(get_app_build_bundle_manifest_path build/phase5/app.build.app_min.json)"
test -f "$BUNDLE_MANIFEST"

x07-wasm app pack --bundle-manifest "$BUNDLE_MANIFEST" \
  --out-dir dist/phase5/app_min.pack \
  --profile-id app_min_release \
  --json --report-out build/phase5/app.pack.app_min.json --quiet-json
require_report_ok build/phase5/app.pack.app_min.json

PACK_MANIFEST="$(get_app_pack_manifest_path build/phase5/app.pack.app_min.json)"
test -f "$PACK_MANIFEST"

x07-wasm app verify --pack-manifest "$PACK_MANIFEST" \
  --json --report-out build/phase5/app.verify.app_min.json --quiet-json
require_report_ok build/phase5/app.verify.app_min.json

echo "==> phase5: lightweight examples-only gate (includes negative verify tests)"
bash scripts/ci/check_phase5_examples.sh

echo "phase5: PASS"
