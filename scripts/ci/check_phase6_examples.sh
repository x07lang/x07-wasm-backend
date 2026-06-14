#!/usr/bin/env bash
set -euo pipefail

# Phase-6 examples-only gate:
#  - Builds on Phase-5 examples (solve_pure_spin + fuel exceeded).
#  - Adds Phase-6 ops/caps/policy checks using pinned fixtures.
#
# This script depends only on:
#   - examples/solve_pure_spin/
#   - pinned Phase-6 fixtures in arch/...
#   - Phase-5/6 x07-wasm commands

mkdir -p build/phase6_examples dist/phase6_examples .x07-wasm/incidents

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

require_report_exit_and_has_code() {
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
if want_code and want_code not in codes:
    print("missing diagnostic code:", want_code)
    print("got codes:", codes)
    print("report:", p)
    print(json.dumps(doc.get("diagnostics", []), indent=2))
    sys.exit(1)
print("ok:", p, "exit_code=", want_exit, "has", want_code)
PY
}

require_report_result_field() {
  local report_path="$1"
  local field="$2"
  local expected="$3"
  "$PYTHON" - "$report_path" "$field" "$expected" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
field = sys.argv[2]
expected = sys.argv[3]
doc = json.loads(p.read_text(encoding="utf-8"))

cur = doc
for part in field.split("."):
    if not isinstance(cur, dict) or part not in cur:
        print("missing field:", field, "in", p)
        print("at part:", part, "cur keys:", list(cur.keys()) if isinstance(cur, dict) else type(cur))
        sys.exit(1)
    cur = cur[part]

# normalize expected types
if expected == "true":
    exp = True
elif expected == "false":
    exp = False
else:
    exp = expected

if cur != exp:
    print("field mismatch:", field, "got:", cur, "expected:", exp, "report:", p)
    sys.exit(1)
print("ok:", p, field, "==", expected)
PY
}

get_first_response_body_sha256() {
  local report_path="$1"
  "$PYTHON" - "$report_path" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
responses = doc.get("result", {}).get("responses", [])
if not isinstance(responses, list) or len(responses) < 1:
    print("missing result.responses[0]", file=sys.stderr)
    sys.exit(1)
body = responses[0].get("body", {})
sha = body.get("sha256") if isinstance(body, dict) else None
if not isinstance(sha, str) or len(sha) != 64:
    print("missing result.responses[0].body.sha256", file=sys.stderr)
    sys.exit(1)
print(sha)
PY
}

echo "==> phase6_examples: Phase-5 examples gate"
bash scripts/ci/check_phase5_examples.sh

echo "==> phase6_examples: caps validate (dev + release)"
x07-wasm caps validate --profile arch/app/ops/caps_dev.json \
  --json --report-out build/phase6_examples/caps.validate.dev.json --quiet-json
require_report_ok build/phase6_examples/caps.validate.dev.json

x07-wasm caps validate --profile arch/app/ops/caps_release.json \
  --json --report-out build/phase6_examples/caps.validate.release.json --quiet-json
require_report_ok build/phase6_examples/caps.validate.release.json

echo "==> phase6_examples: caps validate (allowlist parse only)"
x07-wasm caps validate --profile arch/app/ops/caps_allow_example.json \
  --json --report-out build/phase6_examples/caps.validate.allow_example.json --quiet-json
require_report_ok build/phase6_examples/caps.validate.allow_example.json

echo "==> phase6_examples: ops validate (index resolution dev + release)"
x07-wasm ops validate --index arch/app/ops/index.x07ops.json --profile-id ops_dev \
  --json --report-out build/phase6_examples/ops.validate.dev.json --quiet-json
require_report_ok build/phase6_examples/ops.validate.dev.json

x07-wasm ops validate --index arch/app/ops/index.x07ops.json --profile-id ops_release \
  --json --report-out build/phase6_examples/ops.validate.release.json --quiet-json
require_report_ok build/phase6_examples/ops.validate.release.json

echo "==> phase6_examples: policy validate (strict)"
x07-wasm policy validate \
  --card arch/app/ops/policy_deploy_patch_id.json \
  --card arch/app/ops/policy_deploy_deny_blue_green.json \
  --strict \
  --json --report-out build/phase6_examples/policy.validate.json --quiet-json
require_report_ok build/phase6_examples/policy.validate.json

echo "==> phase6_examples: caps fs denied/allowed (wasi:http component)"
cargo build --release --locked --target wasm32-wasip2 \
  --manifest-path guest/phase6-caps-fixture/Cargo.toml
test -f guest/phase6-caps-fixture/target/wasm32-wasip2/release/x07_wasm_phase6_caps_fixture.wasm

set +e
x07-wasm serve \
  --component guest/phase6-caps-fixture/target/wasm32-wasip2/release/x07_wasm_phase6_caps_fixture.wasm \
  --mode canary \
  --path /fs \
  --ops arch/app/ops/ops_release.json \
  --json --report-out build/phase6_examples/serve.caps_fs_denied.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for caps fs denied, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/serve.caps_fs_denied.json 1 X07WASM_CAPS_FS_DENIED

x07-wasm serve \
  --component guest/phase6-caps-fixture/target/wasm32-wasip2/release/x07_wasm_phase6_caps_fixture.wasm \
  --mode canary \
  --path /fs \
  --ops arch/app/ops/ops_fs_ro.json \
  --json --report-out build/phase6_examples/serve.caps_fs_allowed.json --quiet-json
require_report_ok build/phase6_examples/serve.caps_fs_allowed.json

echo "==> phase6_examples: caps clocks/random record+replay (evidence)"
rm -f dist/phase6_examples/caps.evidence.json
x07-wasm serve \
  --component guest/phase6-caps-fixture/target/wasm32-wasip2/release/x07_wasm_phase6_caps_fixture.wasm \
  --mode canary \
  --path /time_rand \
  --ops arch/app/ops/ops_time_random_record.json \
  --evidence-out dist/phase6_examples/caps.evidence.json \
  --json --report-out build/phase6_examples/serve.time_rand.record.json --quiet-json
require_report_ok build/phase6_examples/serve.time_rand.record.json
test -f dist/phase6_examples/caps.evidence.json

x07-wasm serve \
  --component guest/phase6-caps-fixture/target/wasm32-wasip2/release/x07_wasm_phase6_caps_fixture.wasm \
  --mode canary \
  --path /time_rand \
  --ops arch/app/ops/ops_time_random_record.json \
  --evidence-in dist/phase6_examples/caps.evidence.json \
  --json --report-out build/phase6_examples/serve.time_rand.replay.json --quiet-json
require_report_ok build/phase6_examples/serve.time_rand.replay.json

sha1="$(get_first_response_body_sha256 build/phase6_examples/serve.time_rand.record.json)"
sha2="$(get_first_response_body_sha256 build/phase6_examples/serve.time_rand.replay.json)"
if [ "$sha1" != "$sha2" ]; then
  echo "time_rand record/replay mismatch: $sha1 != $sha2" >&2
  exit 1
fi

echo "==> phase6_examples: caps evidence missing (negative)"
set +e
x07-wasm serve \
  --component guest/phase6-caps-fixture/target/wasm32-wasip2/release/x07_wasm_phase6_caps_fixture.wasm \
  --mode canary \
  --path /time_rand \
  --ops arch/app/ops/ops_time_random_record.json \
  --evidence-in dist/phase6_examples/__missing__.json \
  --json --report-out build/phase6_examples/serve.time_rand.evidence_missing.json --quiet-json
code=$?
set -e
if [ "$code" -ne 3 ]; then
  echo "expected exit code 3 for missing evidence, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/serve.time_rand.evidence_missing.json 3 X07WASM_POLICY_OBLIGATION_UNSATISFIED

echo "==> phase6_examples: caps secrets denied/missing"
set +e
x07-wasm serve \
  --component guest/phase6-caps-fixture/target/wasm32-wasip2/release/x07_wasm_phase6_caps_fixture.wasm \
  --mode canary \
  --path /secret \
  --ops arch/app/ops/ops_release.json \
  --json --report-out build/phase6_examples/serve.caps_secret_denied.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for caps secret denied, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/serve.caps_secret_denied.json 1 X07WASM_CAPS_SECRET_DENIED

mkdir -p dist/phase6_examples/secrets_empty
unset X07_SECRET_API_KEY || true
set +e
X07_SECRETS_DIR="dist/phase6_examples/secrets_empty" x07-wasm serve \
  --component guest/phase6-caps-fixture/target/wasm32-wasip2/release/x07_wasm_phase6_caps_fixture.wasm \
  --mode canary \
  --path /secret \
  --ops arch/app/ops/ops_secret_allow.json \
  --json --report-out build/phase6_examples/serve.caps_secret_missing.json --quiet-json
code=$?
set -e
if [ "$code" -ne 3 ]; then
  echo "expected exit code 3 for caps secret missing, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/serve.caps_secret_missing.json 3 X07WASM_CAPS_SECRET_MISSING

echo "==> phase6_examples: caps net denied (http serve canary)"
x07-wasm build --project examples/http_reducer_effect_http/x07.json --profile wasm_release \
  --out dist/phase6_examples/http_reducer_effect_http.wasm \
  --json --report-out build/phase6_examples/build.http_reducer_effect_http.json --quiet-json
require_report_ok build/phase6_examples/build.http_reducer_effect_http.json
test -f dist/phase6_examples/http_reducer_effect_http.wasm

set +e
x07-wasm http serve --component dist/phase6_examples/http_reducer_effect_http.wasm --mode canary \
  --ops arch/app/ops/ops_release.json \
  --json --report-out build/phase6_examples/http.serve.caps_net_denied.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for caps net denied, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/http.serve.caps_net_denied.json 1 X07WASM_CAPS_NET_DENIED

echo "==> phase6_examples: caps net hardening denied (private ip literal)"
set +e
x07-wasm serve \
  --component guest/phase6-caps-fixture/target/wasm32-wasip2/release/x07_wasm_phase6_caps_fixture.wasm \
  --mode canary \
  --path /net_ip_literal \
  --ops arch/app/ops/ops_allow_loopback_denied.json \
  --json --report-out build/phase6_examples/serve.caps_net_ip_literal_denied.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for caps net hardening denied, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/serve.caps_net_ip_literal_denied.json 1 X07WASM_CAPS_NET_PRIVATE_IP_DENIED

echo "==> phase6_examples: caps net allowlist ok (default port http)"
x07-wasm serve \
  --component guest/phase6-caps-fixture/target/wasm32-wasip2/release/x07_wasm_phase6_caps_fixture.wasm \
  --mode canary \
  --path /net_default_port_http \
  --ops arch/app/ops/ops_allow_localhost_http.json \
  --json --report-out build/phase6_examples/serve.caps_net_default_port_http.ok.json --quiet-json
require_report_ok build/phase6_examples/serve.caps_net_default_port_http.ok.json

echo "==> phase6_examples: always-report (clap parse error)"
set +e
x07-wasm serve --bad-flag \
  --json --report-out build/phase6_examples/cli.parse.bad_flag.json --quiet-json
code=$?
set -e
if [ "$code" -ne 3 ]; then
  echo "expected exit code 3 for clap parse error, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/cli.parse.bad_flag.json 3 X07WASM_CLI_ARGS_INVALID

echo "phase6_examples: PASS"
