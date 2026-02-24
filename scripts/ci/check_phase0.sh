#!/usr/bin/env bash
set -euo pipefail

mkdir -p build/wasm dist .x07-wasm/incidents

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

decode_solve_output_b64() {
  local report_path="$1"
  local out_path="$2"
  "$PYTHON" - "$report_path" "$out_path" <<'PY'
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

check_build_report_invariants() {
  local report_path="$1"
  "$PYTHON" - "$report_path" <<'PY'
import json
import pathlib
import sys

p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
if doc.get("ok") is not True:
    print("build report not ok:", p)
    print(json.dumps(doc.get("diagnostics", []), indent=2))
    sys.exit(1)

res = doc.get("result", {})
exports = res.get("exports", {})
missing = exports.get("missing", [])
if missing:
    print("missing required exports:", missing)
    sys.exit(1)

found = exports.get("found", [])
if "_start" in found:
    print("unexpected export present: _start")
    sys.exit(1)

mem = res.get("memory", {})
if mem.get("growable_memory") is True:
    print("growable memory is not allowed")
    sys.exit(1)
if mem.get("initial_memory_bytes") != mem.get("max_memory_bytes"):
    print("expected fixed memory: initial != max")
    sys.exit(1)

flags = res.get("flags", {}).get("wasm_ld", [])
if "--no-entry" not in flags:
    print("missing wasm-ld flag: --no-entry")
    sys.exit(1)
if "--no-growable-memory" not in flags:
    print("missing wasm-ld flag: --no-growable-memory")
    sys.exit(1)

wasm = res.get("wasm", {})
if int(wasm.get("bytes_len", 0)) <= 0:
    print("wasm bytes_len is not positive")
    sys.exit(1)

print("ok: build invariants:", p)
PY
}

check_run_report_budget_exceeded() {
  local report_path="$1"
  "$PYTHON" - "$report_path" <<'PY'
import json
import pathlib
import sys

p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
exit_code = int(doc.get("exit_code", 0))
if exit_code != 4:
    print("expected exit_code=4 for budget exceeded, got:", exit_code)
    sys.exit(1)
codes = [d.get("code") for d in doc.get("diagnostics", []) if isinstance(d, dict)]
if "X07WASM_BUDGET_EXCEEDED_OUTPUT" not in codes:
    print("expected diagnostic code X07WASM_BUDGET_EXCEEDED_OUTPUT, got:", codes)
    sys.exit(1)
print("ok: budget exceeded report:", p)
PY
}

check_incident_manifest_present() {
  local run_report_path="$1"
  local input_path="$2"
  "$PYTHON" - "$run_report_path" "$input_path" <<'PY'
import datetime
import hashlib
import json
import pathlib
import sys

run_report = pathlib.Path(sys.argv[1])
input_path = pathlib.Path(sys.argv[2])
doc = json.loads(run_report.read_text(encoding="utf-8"))
wasm_sha = doc.get("result", {}).get("wasm", {}).get("sha256")
if not isinstance(wasm_sha, str) or len(wasm_sha) != 64:
    print("missing wasm sha256 in run report")
    sys.exit(1)
input_sha = hashlib.sha256(input_path.read_bytes()).hexdigest()
run_id = hashlib.sha256(f"{wasm_sha}:{input_sha}".encode()).hexdigest()[:32]
date = datetime.datetime.now(datetime.timezone.utc).date().isoformat()
inc_dir = pathlib.Path(".x07-wasm") / "incidents" / date / run_id
manifest = inc_dir / "wasm.manifest.json"
if not manifest.is_file():
    print("missing incident manifest:", manifest)
    sys.exit(1)
first_line = manifest.read_text(encoding="utf-8").splitlines()[0]
if "\"schema_version\":\"x07.wasm.incident.manifest@0.1.0\"" not in first_line:
    print("expected synthesized incident manifest, got first line:", first_line[:200])
    sys.exit(1)
print("ok: incident manifest present:", manifest)
PY
}

echo "==> gate: toolchain"
x07-wasm doctor --json --report-out build/wasm/doctor.json --quiet-json
x07-wasm profile validate --json --report-out build/wasm/profile.validate.json --quiet-json
x07-wasm cli specrows check --json --report-out build/wasm/cli.specrows.check.json --quiet-json

echo "==> gate: build + invariants (echo)"
x07-wasm build --project examples/solve_pure_echo/x07.json --profile wasm_release \
  --out dist/echo.wasm --artifact-out dist/echo.wasm.manifest.json \
  --json --report-out build/wasm/build.echo.json --quiet-json
check_build_report_invariants build/wasm/build.echo.json

cp -f dist/echo.wasm dist/echo.bare.wasm
rm -f dist/echo.bare.wasm.manifest.json

echo "==> gate: semantic equivalence (echo)"
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

echo "==> gate: budgets + incidents (echo)"
set +e
x07-wasm run --wasm dist/echo.bare.wasm --input examples/solve_pure_echo/tests/fixtures/in_hello.bin \
  --max-output-bytes 1 \
  --json --report-out build/wasm/run.echo.budget_exceeded.json --quiet-json
code=$?
set -e
if [ "$code" -ne 4 ]; then
  echo "expected exit code 4 for output budget exceeded, got $code" >&2
  exit 1
fi
check_run_report_budget_exceeded build/wasm/run.echo.budget_exceeded.json
check_incident_manifest_present build/wasm/run.echo.budget_exceeded.json examples/solve_pure_echo/tests/fixtures/in_hello.bin

echo "==> gate: build + equivalence (json_patch)"
x07-wasm build --project examples/json_patch/x07.json --profile wasm_release \
  --out dist/json_patch.wasm --artifact-out dist/json_patch.wasm.manifest.json \
  --json --report-out build/wasm/build.json_patch.json --quiet-json
check_build_report_invariants build/wasm/build.json_patch.json

x07 run --project examples/json_patch/x07.json --input-b64 "" \
  --report-out build/wasm/native.run.json_patch.json --quiet-json
decode_solve_output_b64 build/wasm/native.run.json_patch.json dist/native.json_patch.out.bin

x07-wasm run --wasm dist/json_patch.wasm --input-hex "" \
  --output-out dist/json_patch.out.bin \
  --json --report-out build/wasm/run.json_patch.json --quiet-json
cmp dist/json_patch.out.bin dist/native.json_patch.out.bin
cmp dist/json_patch.out.bin examples/json_patch/tests/expected.bin

echo "==> gate: build + equivalence (task_sched)"
x07-wasm build --project examples/task_sched/x07.json --profile wasm_release \
  --out dist/task_sched.wasm --artifact-out dist/task_sched.wasm.manifest.json \
  --json --report-out build/wasm/build.task_sched.json --quiet-json
check_build_report_invariants build/wasm/build.task_sched.json

x07 run --project examples/task_sched/x07.json --input-b64 "" \
  --report-out build/wasm/native.run.task_sched.json --quiet-json
decode_solve_output_b64 build/wasm/native.run.task_sched.json dist/native.task_sched.out.bin

x07-wasm run --wasm dist/task_sched.wasm --input-hex "" \
  --output-out dist/task_sched.out.bin \
  --json --report-out build/wasm/run.task_sched.json --quiet-json
cmp dist/task_sched.out.bin dist/native.task_sched.out.bin
cmp dist/task_sched.out.bin examples/task_sched/tests/expected.bin

echo "phase0: PASS"
