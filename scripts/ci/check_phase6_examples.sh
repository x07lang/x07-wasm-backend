#!/usr/bin/env bash
set -euo pipefail

# Phase-6 examples-only gate:
#  - Builds on Phase-5 examples (solve_pure_spin + app_min pack/verify + fuel exceeded).
#  - Adds Phase-6 ops/caps/slo/deploy/provenance checks using pinned fixtures.
#
# Assumptions (pinned semantics for CI):
#  - `x07-wasm slo eval` exit codes:
#      promote      -> exit 0, ok=true,  decision="promote"
#      rollback     -> exit 2, ok=false, decision="rollback", diag includes X07WASM_SLO_VIOLATION
#      inconclusive -> exit 3, ok=false, decision="inconclusive", diag includes X07WASM_SLO_EVAL_INCONCLUSIVE
#
#  - `x07-wasm provenance verify` exits:
#      ok -> exit 0
#      mismatch -> exit 1 + diag X07WASM_PROVENANCE_DIGEST_MISMATCH
#
# This script depends only on:
#   - examples/solve_pure_spin/
#   - examples/app_min/
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

require_deploy_plan_outputs_exist() {
  local report_path="$1"
  "$PYTHON" - "$report_path" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))

res = doc.get("result", {})
pm = res.get("plan_manifest", {})
pm_path = pm.get("path")
if not isinstance(pm_path, str) or not pm_path:
    print("missing result.plan_manifest.path in", p, file=sys.stderr)
    sys.exit(1)

pm_file = pathlib.Path(pm_path)
if not pm_file.is_file():
    print("plan_manifest missing:", pm_file, file=sys.stderr)
    sys.exit(1)

outs = res.get("outputs", [])
if not isinstance(outs, list) or len(outs) < 1:
    print("expected >=1 output in result.outputs", file=sys.stderr)
    sys.exit(1)

missing = []
for o in outs:
    if not isinstance(o, dict) or not isinstance(o.get("path"), str):
        missing.append("<invalid>")
        continue
    fp = pathlib.Path(o["path"])
    if not fp.is_file():
        missing.append(o["path"])

if missing:
    print("missing deploy outputs:", missing, file=sys.stderr)
    sys.exit(1)

print("ok: deploy plan outputs exist")
PY
}

corrupt_first_pack_asset_file() {
  local pack_manifest_path="$1"
  "$PYTHON" - "$pack_manifest_path" <<'PY'
import json, pathlib, sys

manifest = pathlib.Path(sys.argv[1])
doc = json.loads(manifest.read_text(encoding="utf-8"))

assets = doc.get("assets", [])
if not isinstance(assets, list) or len(assets) < 1:
    print("pack manifest has no assets:", manifest, file=sys.stderr)
    sys.exit(1)

asset0 = None
for a in assets:
    if isinstance(a, dict) and isinstance(a.get("file"), dict) and isinstance(a["file"].get("path"), str):
        asset0 = a
        break
if asset0 is None:
    print("pack manifest assets missing file.path", file=sys.stderr)
    sys.exit(1)

rel = asset0["file"]["path"]
p = (manifest.parent / rel).resolve()
if not p.is_file():
    print("asset file missing:", p, file=sys.stderr)
    sys.exit(1)

p.write_bytes(p.read_bytes() + b"\x00")
print("tampered:", str(p))
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

echo "==> phase6_examples: slo validate"
x07-wasm slo validate --profile arch/slo/slo_min.json \
  --json --report-out build/phase6_examples/slo.validate.json --quiet-json
require_report_ok build/phase6_examples/slo.validate.json

echo "==> phase6_examples: slo eval (promote)"
x07-wasm slo eval --profile arch/slo/slo_min.json \
  --metrics examples/app_min/tests/metrics_canary_ok.json \
  --json --report-out build/phase6_examples/slo.eval.ok.json --quiet-json
require_report_ok build/phase6_examples/slo.eval.ok.json
require_report_result_field build/phase6_examples/slo.eval.ok.json result.decision promote

echo "==> phase6_examples: slo eval (rollback)"
set +e
x07-wasm slo eval --profile arch/slo/slo_min.json \
  --metrics examples/app_min/tests/metrics_canary_bad.json \
  --json --report-out build/phase6_examples/slo.eval.bad.json --quiet-json
code=$?
set -e
if [ "$code" -ne 2 ]; then
  echo "expected exit code 2 for SLO rollback, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/slo.eval.bad.json 2 X07WASM_SLO_VIOLATION
require_report_result_field build/phase6_examples/slo.eval.bad.json result.decision rollback

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

echo "==> phase6_examples: build app_min -> pack -> verify (fresh, phase6 dir)"
rm -rf dist/phase6_examples/app_min dist/phase6_examples/app_min.pack dist/phase6_examples/deploy_plan
x07-wasm app build --profile-file examples/app_min/app_release.json \
  --out-dir dist/phase6_examples/app_min --clean \
  --json --report-out build/phase6_examples/app.build.app_min.json --quiet-json
require_report_ok build/phase6_examples/app.build.app_min.json
test -f dist/phase6_examples/app_min/app.bundle.json

x07-wasm app pack --bundle-manifest dist/phase6_examples/app_min/app.bundle.json \
  --out-dir dist/phase6_examples/app_min.pack \
  --profile-id app_min_release \
  --json --report-out build/phase6_examples/app.pack.app_min.json --quiet-json
require_report_ok build/phase6_examples/app.pack.app_min.json
test -f dist/phase6_examples/app_min.pack/app.pack.json

x07-wasm app verify --pack-manifest dist/phase6_examples/app_min.pack/app.pack.json \
  --json --report-out build/phase6_examples/app.verify.app_min.json --quiet-json
require_report_ok build/phase6_examples/app.verify.app_min.json

echo "==> phase6_examples: provenance attest -> verify (ok)"
x07-wasm provenance attest \
  --pack-manifest dist/phase6_examples/app_min.pack/app.pack.json \
  --ops arch/app/ops/ops_release.json \
  --out dist/phase6_examples/app_min.pack/provenance.slsa.json \
  --json --report-out build/phase6_examples/provenance.attest.json --quiet-json
require_report_ok build/phase6_examples/provenance.attest.json
test -f dist/phase6_examples/app_min.pack/provenance.slsa.json

x07-wasm provenance verify \
  --attestation dist/phase6_examples/app_min.pack/provenance.slsa.json \
  --pack-dir dist/phase6_examples/app_min.pack \
  --json --report-out build/phase6_examples/provenance.verify.ok.json --quiet-json
require_report_ok build/phase6_examples/provenance.verify.ok.json

echo "==> phase6_examples: provenance verify (negative - tamper asset)"
rm -rf dist/phase6_examples/app_min.pack.tampered
cp -a dist/phase6_examples/app_min.pack dist/phase6_examples/app_min.pack.tampered
corrupt_first_pack_asset_file dist/phase6_examples/app_min.pack.tampered/app.pack.json >/dev/null

set +e
x07-wasm provenance verify \
  --attestation dist/phase6_examples/app_min.pack/provenance.slsa.json \
  --pack-dir dist/phase6_examples/app_min.pack.tampered \
  --json --report-out build/phase6_examples/provenance.verify.bad.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for provenance digest mismatch, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/provenance.verify.bad.json 1 X07WASM_PROVENANCE_DIGEST_MISMATCH

echo "==> phase6_examples: deploy plan"
x07-wasm deploy plan \
  --pack-manifest dist/phase6_examples/app_min.pack/app.pack.json \
  --ops arch/app/ops/ops_release_policy_patch.json \
  --out-dir dist/phase6_examples/deploy_plan \
  --json --report-out build/phase6_examples/deploy.plan.json --quiet-json
require_report_ok build/phase6_examples/deploy.plan.json
require_deploy_plan_outputs_exist build/phase6_examples/deploy.plan.json

echo "==> phase6_examples: deploy plan (policy denied)"
set +e
x07-wasm deploy plan \
  --pack-manifest dist/phase6_examples/app_min.pack/app.pack.json \
  --ops arch/app/ops/ops_release_policy_deny.json \
  --out-dir dist/phase6_examples/deploy_plan.policy_denied \
  --json --report-out build/phase6_examples/deploy.plan.policy_denied.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for deploy plan policy denied, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/deploy.plan.policy_denied.json 1 X07WASM_POLICY_DECISION_DENY
require_report_exit_and_has_code build/phase6_examples/deploy.plan.policy_denied.json 1 X07WASM_DEPLOY_PLAN_POLICY_DENIED

echo "phase6_examples: PASS"
