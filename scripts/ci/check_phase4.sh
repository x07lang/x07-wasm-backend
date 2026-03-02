#!/usr/bin/env bash
set -euo pipefail

mkdir -p build/wasm dist target/x07-wasm/component .x07-wasm/incidents

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

get_profile_budget_u64() {
  local profile_path="$1"
  local dotted_key="$2"
  "$PYTHON" - "$profile_path" "$dotted_key" <<'PY'
import json
import pathlib
import sys

p = pathlib.Path(sys.argv[1])
key = sys.argv[2]
doc = json.loads(p.read_text(encoding="utf-8"))

cur = doc
for part in key.split("."):
    if not isinstance(cur, dict) or part not in cur:
        print(f"missing key in profile: {key} (at {part})", file=sys.stderr)
        sys.exit(1)
    cur = cur[part]

if not isinstance(cur, int):
    print(f"expected int budget at {key}, got: {type(cur).__name__}", file=sys.stderr)
    sys.exit(1)

if cur < 0:
    print(f"expected non-negative budget at {key}, got: {cur}", file=sys.stderr)
    sys.exit(1)

print(cur)
PY
}

write_bytes_file() {
  local out_path="$1"
  local n_bytes="$2"
  "$PYTHON" - "$out_path" "$n_bytes" <<'PY'
import pathlib
import sys

out = pathlib.Path(sys.argv[1])
n = int(sys.argv[2])
out.parent.mkdir(parents=True, exist_ok=True)
out.write_bytes(b"\x00" * n)
print(f"wrote {n} bytes to {out}")
PY
}

check_report_exit_code_and_has_code() {
  local report_path="$1"
  local expected_exit_code="$2"
  local expected_code="$3"
  "$PYTHON" - "$report_path" "$expected_exit_code" "$expected_code" <<'PY'
import json
import pathlib
import sys

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

check_component_run_has_incident_manifest() {
  local report_path="$1"
  "$PYTHON" - "$report_path" <<'PY'
import json
import pathlib
import sys

p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
inc = doc.get("result", {}).get("incident_dir")
if not isinstance(inc, str) or not inc:
    print("expected non-empty result.incident_dir")
    print("report:", p)
    sys.exit(1)

manifest = pathlib.Path(inc) / "incident.manifest.json"
if not manifest.is_file():
    print("missing component incident manifest:", manifest)
    sys.exit(1)

first = manifest.read_text(encoding="utf-8").splitlines()[0]
if "\"schema_version\":\"x07.wasm.incident.manifest@0.1.0\"" not in first:
    print("unexpected incident manifest first line:", first[:200])
    sys.exit(1)

print("ok: component incident manifest:", manifest)
PY
}

check_serve_has_incident_manifest() {
  local report_path="$1"
  "$PYTHON" - "$report_path" <<'PY'
import json
import pathlib
import sys

p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
incs = doc.get("result", {}).get("incidents", [])
if not isinstance(incs, list) or len(incs) < 1:
    print("expected at least 1 incident in serve report")
    print("report:", p)
    print(json.dumps(doc.get("diagnostics", []), indent=2))
    sys.exit(1)

inc0 = pathlib.Path(incs[0])
manifest = inc0 / "incident.manifest.json"
if not manifest.is_file():
    print("missing serve incident manifest:", manifest)
    sys.exit(1)

first = manifest.read_text(encoding="utf-8").splitlines()[0]
if "\"schema_version\":\"x07.wasm.incident.manifest@0.1.0\"" not in first:
    print("unexpected incident manifest first line:", first[:200])
    sys.exit(1)

print("ok: serve incident manifest:", manifest)
PY
}

echo "==> gate: toolchain"
x07-wasm doctor --json --report-out build/wasm/doctor.json --quiet-json
x07-wasm profile validate --json --report-out build/wasm/profile.validate.json --quiet-json
x07-wasm cli specrows check --json --report-out build/wasm/cli.specrows.check.json --quiet-json
x07-wasm wit validate --json --report-out build/wasm/wit.validate.json --quiet-json
x07-wasm component profile validate --json --report-out build/wasm/component.profile.validate.json --quiet-json

# Pull budgets directly from the pinned release component profile (Phase-0 style: CI derives test sizes from arch config).
COMPONENT_RELEASE_PROFILE="arch/wasm/component/profiles/component_release.json"
CLI_STDIN_BUDGET="$(get_profile_budget_u64 "$COMPONENT_RELEASE_PROFILE" "cfg.native_targets.cli.budgets.max_stdin_bytes")"
HTTP_REQ_BODY_BUDGET="$(get_profile_budget_u64 "$COMPONENT_RELEASE_PROFILE" "cfg.native_targets.http.budgets.max_request_body_bytes")"

CLI_STDIN_OVER="$((CLI_STDIN_BUDGET + 1))"
HTTP_REQ_BODY_OVER="$((HTTP_REQ_BODY_BUDGET + 1))"

write_bytes_file dist/cli.stdin.over.bin "$CLI_STDIN_OVER"
write_bytes_file dist/http.request_body.over.bin "$HTTP_REQ_BODY_OVER"

echo "==> gate: native CLI component build + run (cli echo)"
x07-wasm component build --project examples/solve_pure_echo/x07.json --emit cli-native --out-dir target/x07-wasm/component/cli_echo_native --clean \
  --json --report-out build/wasm/component.build.cli_echo_native.json --quiet-json

x07-wasm component targets --component target/x07-wasm/component/cli_echo_native/cli.component.wasm \
  --wit wit/deps/wasi/cli/0.2.8/command.wit --world command \
  --json --report-out build/wasm/component.targets.cli_echo_native.json --quiet-json

x07-wasm component run --component target/x07-wasm/component/cli_echo_native/cli.component.wasm \
  --stdin examples/solve_pure_echo/tests/fixtures/in_hello.bin \
  --stdout-out dist/cli_echo_native.stdout.bin \
  --json --report-out build/wasm/component.run.cli_echo_native.json --quiet-json
cmp dist/cli_echo_native.stdout.bin examples/solve_pure_echo/tests/fixtures/out_hello.bin

echo "==> gate: native CLI budgets + incidents (stdin cap)"
set +e
x07-wasm component run --component target/x07-wasm/component/cli_echo_native/cli.component.wasm \
  --stdin dist/cli.stdin.over.bin \
  --json --report-out build/wasm/component.run.cli_echo_native.budget_stdin.json --quiet-json
code=$?
set -e
if [ "$code" -ne 4 ]; then
  echo "expected exit code 4 for CLI stdin budget exceeded, got $code" >&2
  exit 1
fi
check_report_exit_code_and_has_code build/wasm/component.run.cli_echo_native.budget_stdin.json 4 X07WASM_BUDGET_EXCEEDED_CLI_STDIN
check_component_run_has_incident_manifest build/wasm/component.run.cli_echo_native.budget_stdin.json

echo "==> gate: native HTTP component build + serve canary (http echo)"
x07-wasm component build --project examples/http_echo/x07.json --emit http-native --out-dir target/x07-wasm/component/http_echo_native --clean \
  --json --report-out build/wasm/component.build.http_echo_native.json --quiet-json

x07-wasm component targets --component target/x07-wasm/component/http_echo_native/http.component.wasm \
  --wit wit/deps/wasi/http/0.2.8/proxy.wit --world proxy \
  --json --report-out build/wasm/component.targets.http_echo_native.json --quiet-json

x07-wasm serve --mode canary --component target/x07-wasm/component/http_echo_native/http.component.wasm \
  --request-body @examples/http_echo/tests/fixtures/request_body.bin \
  --stop-after 1 \
  --json --report-out build/wasm/serve.http_echo_native.json --quiet-json
decode_first_serve_response_body_b64 build/wasm/serve.http_echo_native.json dist/http_echo_native.response_body.bin
cmp dist/http_echo_native.response_body.bin examples/http_echo/tests/fixtures/response_body.bin

echo "==> gate: native HTTP budgets + incidents (request body cap)"
set +e
x07-wasm serve --mode canary --component target/x07-wasm/component/http_echo_native/http.component.wasm \
  --request-body @dist/http.request_body.over.bin \
  --stop-after 1 \
  --json --report-out build/wasm/serve.http_echo_native.budget_req_body.json --quiet-json
code=$?
set -e
if [ "$code" -ne 4 ]; then
  echo "expected exit code 4 for HTTP request body budget exceeded, got $code" >&2
  exit 1
fi
check_report_exit_code_and_has_code build/wasm/serve.http_echo_native.budget_req_body.json 4 X07WASM_BUDGET_EXCEEDED_HTTP_REQUEST_BODY
check_serve_has_incident_manifest build/wasm/serve.http_echo_native.budget_req_body.json

echo "==> gate: native HTTP glue parse errors + incidents (bad response envelope)"
x07-wasm component build --project examples/http_bad_response/x07.json --emit http-native --out-dir target/x07-wasm/component/http_bad_response_native --clean \
  --json --report-out build/wasm/component.build.http_bad_response_native.json --quiet-json

set +e
x07-wasm serve --mode canary --component target/x07-wasm/component/http_bad_response_native/http.component.wasm \
  --request-body "" \
  --stop-after 1 \
  --json --report-out build/wasm/serve.http_bad_response_native.parse_failed.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for native HTTP response envelope parse failure, got $code" >&2
  exit 1
fi
check_report_exit_code_and_has_code build/wasm/serve.http_bad_response_native.parse_failed.json 1 X07WASM_NATIVE_HTTP_RESPONSE_ENVELOPE_PARSE_FAILED
check_serve_has_incident_manifest build/wasm/serve.http_bad_response_native.parse_failed.json

echo "phase4: PASS"
