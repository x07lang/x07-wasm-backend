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

decode_first_serve_response_body_b64() {
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

if doc.get("ok") is not True:
    print("serve report not ok:", report)
    print(json.dumps(doc.get("diagnostics", []), indent=2))
    sys.exit(1)

responses = doc.get("result", {}).get("responses", [])
if not isinstance(responses, list) or len(responses) < 1:
    print("missing serve responses")
    sys.exit(1)

body = responses[0].get("body", {})
if not isinstance(body, dict) or not isinstance(body.get("base64"), str):
    print("missing response body.base64")
    sys.exit(1)

data = base64.b64decode(body["base64"])
out.parent.mkdir(parents=True, exist_ok=True)
out.write_bytes(data)
PY
}

check_component_run_report_budget_exceeded() {
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
inc = doc.get("result", {}).get("incident_dir")
if not isinstance(inc, str) or not inc:
    print("expected non-empty incident_dir")
    sys.exit(1)
manifest = pathlib.Path(inc) / "incident.manifest.json"
if not manifest.is_file():
    print("missing incident manifest:", manifest)
    sys.exit(1)
print("ok: component-run budget exceeded:", p)
PY
}

echo "==> gate: toolchain"
x07-wasm doctor --json --report-out build/wasm/doctor.json --quiet-json
x07-wasm profile validate --json --report-out build/wasm/profile.validate.json --quiet-json
x07-wasm cli specrows check --json --report-out build/wasm/cli.specrows.check.json --quiet-json
x07-wasm wit validate --json --report-out build/wasm/wit.validate.json --quiet-json
x07-wasm component profile validate --json --report-out build/wasm/component.profile.validate.json --quiet-json

echo "==> gate: embedded adapter snapshots"
if ! command -v wasm-tools >/dev/null 2>&1; then
  echo "wasm-tools not found on PATH (required to canonicalize adapter snapshots)" >&2
  exit 1
fi

canon_tmp_dir="$(mktemp -d)"
cleanup_canon_tmp_dir() {
  rm -rf "${canon_tmp_dir}"
}
trap cleanup_canon_tmp_dir EXIT

cargo build --release --locked --target wasm32-wasip2 --manifest-path guest/http-adapter/Cargo.toml
cargo build --release --locked --target wasm32-wasip2 --manifest-path guest/cli-adapter/Cargo.toml

wasm-tools strip -a guest/http-adapter/target/wasm32-wasip2/release/x07_wasm_http_adapter.wasm -o "${canon_tmp_dir}/http-adapter.guest.wasm"
wasm-tools strip -a crates/x07-wasm/src/support/adapters/http-adapter.component.wasm -o "${canon_tmp_dir}/http-adapter.embedded.wasm"
if ! cmp -s "${canon_tmp_dir}/http-adapter.guest.wasm" "${canon_tmp_dir}/http-adapter.embedded.wasm"; then
  echo "ERROR: embedded http-adapter.component.wasm is out of sync with guest/http-adapter output" >&2
  exit 1
fi

wasm-tools strip -a guest/cli-adapter/target/wasm32-wasip2/release/x07_wasm_cli_adapter.wasm -o "${canon_tmp_dir}/cli-adapter.guest.wasm"
wasm-tools strip -a crates/x07-wasm/src/support/adapters/cli-adapter.component.wasm -o "${canon_tmp_dir}/cli-adapter.embedded.wasm"
if ! cmp -s "${canon_tmp_dir}/cli-adapter.guest.wasm" "${canon_tmp_dir}/cli-adapter.embedded.wasm"; then
  echo "ERROR: embedded cli-adapter.component.wasm is out of sync with guest/cli-adapter output" >&2
  exit 1
fi

echo "==> gate: adapters"
x07-wasm component build --project examples/solve_pure_echo/x07.json --emit http-adapter --out-dir target/x07-wasm/component/adapters --clean \
  --json --report-out build/wasm/component.build.http_adapter.json --quiet-json
x07-wasm component build --project examples/solve_pure_echo/x07.json --emit cli-adapter --out-dir target/x07-wasm/component/adapters \
  --json --report-out build/wasm/component.build.cli_adapter.json --quiet-json

echo "==> gate: component run (cli echo)"
x07-wasm component build --project examples/solve_pure_echo/x07.json --emit solve --out-dir target/x07-wasm/component/cli_echo --clean \
  --json --report-out build/wasm/component.build.cli_echo.json --quiet-json
x07-wasm component compose --adapter cli \
  --solve target/x07-wasm/component/cli_echo/solve.component.wasm \
  --adapter-component target/x07-wasm/component/adapters/cli-adapter.component.wasm \
  --out dist/cli_echo.component.wasm \
  --targets-check \
  --json --report-out build/wasm/component.compose.cli_echo.json --quiet-json
x07-wasm component targets --component dist/cli_echo.component.wasm --wit wit/deps/wasi/cli/0.2.8/command.wit --world command \
  --json --report-out build/wasm/component.targets.cli_echo.json --quiet-json
x07-wasm component run --component dist/cli_echo.component.wasm \
  --stdin examples/solve_pure_echo/tests/fixtures/in_hello.bin \
  --stdout-out dist/cli_echo.stdout.bin \
  --json --report-out build/wasm/component.run.cli_echo.json --quiet-json
cmp dist/cli_echo.stdout.bin examples/solve_pure_echo/tests/fixtures/out_hello.bin

echo "==> gate: component run budgets + incidents (cli echo)"
set +e
x07-wasm component run --component dist/cli_echo.component.wasm \
  --stdin examples/solve_pure_echo/tests/fixtures/in_hello.bin \
  --max-output-bytes 1 \
  --json --report-out build/wasm/component.run.cli_echo.budget_exceeded.json --quiet-json
code=$?
set -e
if [ "$code" -ne 4 ]; then
  echo "expected exit code 4 for output budget exceeded, got $code" >&2
  exit 1
fi
check_component_run_report_budget_exceeded build/wasm/component.run.cli_echo.budget_exceeded.json

echo "==> gate: serve canary (http echo)"
x07-wasm component build --project examples/http_echo/x07.json --emit solve --out-dir target/x07-wasm/component/http_echo --clean \
  --json --report-out build/wasm/component.build.http_echo.json --quiet-json
x07-wasm component compose --adapter http \
  --solve target/x07-wasm/component/http_echo/solve.component.wasm \
  --adapter-component target/x07-wasm/component/adapters/http-adapter.component.wasm \
  --out dist/http_echo.component.wasm \
  --targets-check \
  --json --report-out build/wasm/component.compose.http_echo.json --quiet-json
x07-wasm component targets --component dist/http_echo.component.wasm --wit wit/deps/wasi/http/0.2.8/proxy.wit --world proxy \
  --json --report-out build/wasm/component.targets.http_echo.json --quiet-json

x07-wasm serve --mode canary --component dist/http_echo.component.wasm \
  --request-body @examples/http_echo/tests/fixtures/request_body.bin \
  --stop-after 1 \
  --json --report-out build/wasm/serve.http_echo.json --quiet-json
decode_first_serve_response_body_b64 build/wasm/serve.http_echo.json dist/http_echo.response_body.bin
cmp dist/http_echo.response_body.bin examples/http_echo/tests/fixtures/response_body.bin

echo "phase1: PASS"
