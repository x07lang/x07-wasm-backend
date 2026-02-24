#!/usr/bin/env bash
set -euo pipefail

mkdir -p build/wasm dist .x07-wasm/incidents

export X07_WASM_WASMTIME_VERSION="$(wasmtime --version | head -n 1)"

decode_solve_output_b64() {
  local report_path="$1"
  local out_path="$2"
  python3 - "$report_path" "$out_path" <<'PY'
import base64
import json
import pathlib
import sys

report = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
doc = json.loads(report.read_text(encoding="utf-8"))
def find_solve_output_b64(doc: dict) -> str:
    if isinstance(doc.get("solve_output_b64"), str):
        return doc["solve_output_b64"]
    solve = doc.get("solve")
    if isinstance(solve, dict) and isinstance(solve.get("solve_output_b64"), str):
        return solve["solve_output_b64"]
    report = doc.get("report")
    if isinstance(report, dict):
        solve = report.get("solve")
        if isinstance(solve, dict) and isinstance(solve.get("solve_output_b64"), str):
            return solve["solve_output_b64"]
    return ""

b64 = find_solve_output_b64(doc)
data = base64.b64decode(b64)
out.parent.mkdir(parents=True, exist_ok=True)
out.write_bytes(data)
PY
}

x07-wasm doctor --json --report-out build/wasm/doctor.json --quiet-json
x07-wasm profile validate --json --report-out build/wasm/profile.validate.json --quiet-json
x07-wasm cli specrows check --json --report-out build/wasm/cli.specrows.check.json --quiet-json

x07-wasm build --project examples/solve_pure_echo/x07.json --profile wasm_release \
  --out dist/echo.wasm --artifact-out dist/echo.wasm.manifest.json \
  --json --report-out build/wasm/build.echo.json --quiet-json

for n in empty hello binary; do
  x07 run --project examples/solve_pure_echo/x07.json --input "examples/solve_pure_echo/tests/fixtures/in_${n}.bin" \
    --report-out "build/wasm/native.run.echo.${n}.json" --quiet-json
  decode_solve_output_b64 "build/wasm/native.run.echo.${n}.json" "dist/native.echo.${n}.out.bin"

  x07-wasm run --wasm dist/echo.wasm --input "examples/solve_pure_echo/tests/fixtures/in_${n}.bin" \
    --output-out "dist/echo.${n}.out.bin" \
    --json --report-out "build/wasm/run.echo.${n}.json" --quiet-json
  cmp "dist/echo.${n}.out.bin" "dist/native.echo.${n}.out.bin"
  cmp "dist/echo.${n}.out.bin" "examples/solve_pure_echo/tests/fixtures/out_${n}.bin"
done

x07-wasm build --project examples/json_patch/x07.json --profile wasm_release \
  --out dist/json_patch.wasm --artifact-out dist/json_patch.wasm.manifest.json \
  --json --report-out build/wasm/build.json_patch.json --quiet-json

x07 run --project examples/json_patch/x07.json --input-b64 "" \
  --report-out build/wasm/native.run.json_patch.json --quiet-json
decode_solve_output_b64 build/wasm/native.run.json_patch.json dist/native.json_patch.out.bin

x07-wasm run --wasm dist/json_patch.wasm --input-hex "" \
  --output-out dist/json_patch.out.bin \
  --json --report-out build/wasm/run.json_patch.json --quiet-json
cmp dist/json_patch.out.bin dist/native.json_patch.out.bin
cmp dist/json_patch.out.bin examples/json_patch/tests/expected.bin

x07-wasm build --project examples/task_sched/x07.json --profile wasm_release \
  --out dist/task_sched.wasm --artifact-out dist/task_sched.wasm.manifest.json \
  --json --report-out build/wasm/build.task_sched.json --quiet-json

x07 run --project examples/task_sched/x07.json --input-b64 "" \
  --report-out build/wasm/native.run.task_sched.json --quiet-json
decode_solve_output_b64 build/wasm/native.run.task_sched.json dist/native.task_sched.out.bin

x07-wasm run --wasm dist/task_sched.wasm --input-hex "" \
  --output-out dist/task_sched.out.bin \
  --json --report-out build/wasm/run.task_sched.json --quiet-json
cmp dist/task_sched.out.bin dist/native.task_sched.out.bin
cmp dist/task_sched.out.bin examples/task_sched/tests/expected.bin

echo "phase0: PASS"
