#!/usr/bin/env bash
set -euo pipefail

# Phase-5 examples-only gate:
# - examples/solve_pure_spin (fuel budget exceeded + golden IO)
#
# This script intentionally avoids any dependency on other examples, contracts,
# or toolchain checks. It assumes Phase-5 commands/flags exist:
#   - x07-wasm run --max-fuel
#
# Expected Phase-5 diagnostic codes:
#   - X07WASM_BUDGET_EXCEEDED_CPU_FUEL

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

echo "phase5_examples: PASS"
