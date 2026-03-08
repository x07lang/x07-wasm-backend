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

require_deploy_plan_outputs_absent() {
  local report_path="$1"
  "$PYTHON" - "$report_path" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))

res = doc.get("result", {})
out_dir = res.get("out_dir")
if not isinstance(out_dir, str) or not out_dir:
    print("missing result.out_dir in", p, file=sys.stderr)
    sys.exit(1)

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
if not isinstance(outs, list) or len(outs) != 0:
    print("expected 0 outputs in result.outputs", file=sys.stderr)
    print("got:", outs, file=sys.stderr)
    sys.exit(1)

out_dir_path = pathlib.Path(out_dir)
unexpected = []
for name in ["rollout.yaml", "analysis-template.yaml", "service.yaml", "ingress.yaml"]:
    fp = out_dir_path / name
    if fp.exists():
        unexpected.append(str(fp))
if unexpected:
    print("unexpected deploy outputs:", unexpected, file=sys.stderr)
    sys.exit(1)

print("ok: deploy plan outputs absent")
PY
}

has_fullstack_showcase_toolchain() {
  command -v clang >/dev/null 2>&1 || return 1
  command -v wasm-ld >/dev/null 2>&1 || return 1
  clang --version >/dev/null 2>&1 || return 1
  wasm-ld --version >/dev/null 2>&1 || return 1
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

get_report_result_compatibility_hash() {
  local report_path="$1"
  "$PYTHON" - "$report_path" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
v = doc.get("result", {}).get("compatibility_hash")
if not isinstance(v, str) or len(v) != 64:
    print("missing result.compatibility_hash", file=sys.stderr)
    sys.exit(1)
print(v)
PY
}

get_attestation_x07_compatibility_hash() {
  local attestation_path="$1"
  "$PYTHON" - "$attestation_path" <<'PY'
import base64, json, pathlib, sys
p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
payload_b64 = doc.get("payload")
if not isinstance(payload_b64, str) or not payload_b64:
    print("missing payload (DSSE)", file=sys.stderr)
    sys.exit(1)
payload_bytes = base64.b64decode(payload_b64)
payload_doc = json.loads(payload_bytes.decode("utf-8"))
v = payload_doc.get("predicate", {}).get("x07", {}).get("compatibility_hash")
if not isinstance(v, str) or len(v) != 64:
    print("missing predicate.x07.compatibility_hash", file=sys.stderr)
    sys.exit(1)
print(v)
PY
}

corrupt_dsse_signature_in_place() {
  local envelope_path="$1"
  "$PYTHON" - "$envelope_path" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
sigs = doc.get("signatures")
if not isinstance(sigs, list) or len(sigs) < 1 or not isinstance(sigs[0], dict):
    print("missing signatures[0]", file=sys.stderr)
    sys.exit(1)
sig = sigs[0].get("sig")
if not isinstance(sig, str) or len(sig) < 4:
    print("missing signatures[0].sig", file=sys.stderr)
    sys.exit(1)
sigs[0]["sig"] = "AA" + sig[2:]
p.write_text(json.dumps(doc, indent=2) + "\n", encoding="utf-8")
print("ok: corrupted dsse signature")
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

symlink_first_pack_asset_file() {
  local pack_manifest_path="$1"
  local target_path="$2"
  "$PYTHON" - "$pack_manifest_path" "$target_path" <<'PY'
import json, os, pathlib, sys

manifest = pathlib.Path(sys.argv[1])
target = pathlib.Path(sys.argv[2]).resolve()
doc = json.loads(manifest.read_text(encoding="utf-8"))

assets = doc.get("assets", [])
if not isinstance(assets, list) or len(assets) < 1:
    raise SystemExit(f"pack manifest has no assets: {manifest}")

asset0 = None
for a in assets:
    if isinstance(a, dict) and isinstance(a.get("file"), dict) and isinstance(a["file"].get("path"), str):
        asset0 = a
        break
if asset0 is None:
    raise SystemExit("pack manifest assets missing file.path")

rel = asset0["file"]["path"]
p = manifest.parent / rel
p.parent.mkdir(parents=True, exist_ok=True)
try:
    p.unlink()
except FileNotFoundError:
    pass

os.symlink(str(target), str(p))
print("symlinked:", str(p), "->", str(target))
PY
}

oversize_first_pack_asset_file() {
  local pack_manifest_path="$1"
  local size="$2"
  "$PYTHON" - "$pack_manifest_path" "$size" <<'PY'
import json, os, pathlib, sys

manifest = pathlib.Path(sys.argv[1])
size = int(sys.argv[2])
doc = json.loads(manifest.read_text(encoding="utf-8"))

assets = doc.get("assets", [])
if not isinstance(assets, list) or len(assets) < 1:
    raise SystemExit(f"pack manifest has no assets: {manifest}")

asset0 = None
for a in assets:
    if isinstance(a, dict) and isinstance(a.get("file"), dict) and isinstance(a["file"].get("path"), str):
        asset0 = a
        break
if asset0 is None:
    raise SystemExit("pack manifest assets missing file.path")

rel = asset0["file"]["path"]
p = (manifest.parent / rel).resolve()
if not p.is_file():
    raise SystemExit(f"asset file missing: {p}")

os.truncate(p, size)
print("oversized:", str(p), "size=", size)
PY
}

corrupt_pack_backend_component_file() {
  local pack_manifest_path="$1"
  "$PYTHON" - "$pack_manifest_path" <<'PY'
import json, pathlib, sys

manifest = pathlib.Path(sys.argv[1])
doc = json.loads(manifest.read_text(encoding="utf-8"))

backend = doc.get("backend", {})
if not isinstance(backend, dict):
    print("pack manifest backend invalid:", manifest, file=sys.stderr)
    sys.exit(1)

component = backend.get("component", {})
if not isinstance(component, dict) or not isinstance(component.get("path"), str):
    print("pack manifest backend.component.path missing:", manifest, file=sys.stderr)
    sys.exit(1)

rel = component["path"]
p = (manifest.parent / rel).resolve()
if not p.is_file():
    print("backend component file missing:", p, file=sys.stderr)
    sys.exit(1)

p.write_bytes(p.read_bytes() + b"\x00")
print("tampered:", str(p))
PY
}

corrupt_pack_bundle_manifest_file() {
  local pack_manifest_path="$1"
  "$PYTHON" - "$pack_manifest_path" <<'PY'
import json, pathlib, sys

manifest = pathlib.Path(sys.argv[1])
doc = json.loads(manifest.read_text(encoding="utf-8"))

bundle = doc.get("bundle_manifest", {})
if not isinstance(bundle, dict) or not isinstance(bundle.get("path"), str):
    print("pack manifest bundle_manifest.path missing:", manifest, file=sys.stderr)
    sys.exit(1)

rel = bundle["path"]
p = (manifest.parent / rel).resolve()
if not p.is_file():
    print("bundle manifest file missing:", p, file=sys.stderr)
    sys.exit(1)

p.write_bytes(p.read_bytes() + b"\x00")
print("tampered:", str(p))
PY
}

symlink_pack_backend_component_file() {
  local pack_manifest_path="$1"
  local target_path="$2"
  "$PYTHON" - "$pack_manifest_path" "$target_path" <<'PY'
import json, os, pathlib, sys

manifest = pathlib.Path(sys.argv[1])
target = pathlib.Path(sys.argv[2]).resolve()
doc = json.loads(manifest.read_text(encoding="utf-8"))

backend = doc.get("backend", {})
if not isinstance(backend, dict):
    raise SystemExit(f"pack manifest backend invalid: {manifest}")

component = backend.get("component", {})
if not isinstance(component, dict) or not isinstance(component.get("path"), str):
    raise SystemExit(f"pack manifest backend.component.path missing: {manifest}")

rel = component["path"]
p = manifest.parent / rel
p.parent.mkdir(parents=True, exist_ok=True)
try:
    p.unlink()
except FileNotFoundError:
    pass

os.symlink(str(target), str(p))
print("symlinked:", str(p), "->", str(target))
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

echo "==> phase6_examples: slo eval (inconclusive - missing metric)"
set +e
x07-wasm slo eval --profile arch/slo/slo_min.json \
  --metrics examples/app_min/tests/metrics_canary_missing.json \
  --json --report-out build/phase6_examples/slo.eval.missing.json --quiet-json
code=$?
set -e
if [ "$code" -ne 3 ]; then
  echo "expected exit code 3 for SLO inconclusive, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/slo.eval.missing.json 3 X07WASM_SLO_METRIC_MISSING
require_report_exit_and_has_code build/phase6_examples/slo.eval.missing.json 3 X07WASM_SLO_EVAL_INCONCLUSIVE
require_report_result_field build/phase6_examples/slo.eval.missing.json result.decision inconclusive

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

echo "==> phase6_examples: app verify (negative - tamper backend component)"
rm -rf dist/phase6_examples/app_min.pack.backend_tampered
cp -a dist/phase6_examples/app_min.pack dist/phase6_examples/app_min.pack.backend_tampered
corrupt_pack_backend_component_file dist/phase6_examples/app_min.pack.backend_tampered/app.pack.json >/dev/null

set +e
x07-wasm app verify --pack-manifest dist/phase6_examples/app_min.pack.backend_tampered/app.pack.json \
  --json --report-out build/phase6_examples/app.verify.app_min.backend_tampered.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for app verify backend digest mismatch, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/app.verify.app_min.backend_tampered.json 1 X07WASM_APP_VERIFY_BACKEND_COMPONENT_DIGEST_MISMATCH

echo "==> phase6_examples: app verify (negative - tamper bundle manifest)"
rm -rf dist/phase6_examples/app_min.pack.bundle_manifest_tampered
cp -a dist/phase6_examples/app_min.pack dist/phase6_examples/app_min.pack.bundle_manifest_tampered
corrupt_pack_bundle_manifest_file dist/phase6_examples/app_min.pack.bundle_manifest_tampered/app.pack.json >/dev/null

set +e
x07-wasm app verify --pack-manifest dist/phase6_examples/app_min.pack.bundle_manifest_tampered/app.pack.json \
  --json --report-out build/phase6_examples/app.verify.app_min.bundle_manifest_tampered.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for app verify bundle manifest digest mismatch, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/app.verify.app_min.bundle_manifest_tampered.json 1 X07WASM_APP_VERIFY_BUNDLE_MANIFEST_DIGEST_MISMATCH

echo "==> phase6_examples: app verify (negative - unsafe symlink path)"
rm -rf dist/phase6_examples/app_min.pack.backend_symlink
cp -a dist/phase6_examples/app_min.pack dist/phase6_examples/app_min.pack.backend_symlink
printf "outside" > dist/phase6_examples/outside.bin
symlink_pack_backend_component_file dist/phase6_examples/app_min.pack.backend_symlink/app.pack.json dist/phase6_examples/outside.bin >/dev/null

set +e
x07-wasm app verify --pack-manifest dist/phase6_examples/app_min.pack.backend_symlink/app.pack.json \
  --json --report-out build/phase6_examples/app.verify.app_min.backend_symlink.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for app verify unsafe symlink path, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/app.verify.app_min.backend_symlink.json 1 X07WASM_APP_VERIFY_PATH_UNSAFE

echo "==> phase6_examples: app verify (negative - asset too large)"
rm -rf dist/phase6_examples/app_min.pack.asset_too_large
cp -a dist/phase6_examples/app_min.pack dist/phase6_examples/app_min.pack.asset_too_large
APP_VERIFY_MAX_FILE_BYTES=$((256 * 1024 * 1024))
oversize_first_pack_asset_file dist/phase6_examples/app_min.pack.asset_too_large/app.pack.json $((APP_VERIFY_MAX_FILE_BYTES + 1)) >/dev/null

set +e
x07-wasm app verify --pack-manifest dist/phase6_examples/app_min.pack.asset_too_large/app.pack.json \
  --json --report-out build/phase6_examples/app.verify.app_min.asset_too_large.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for app verify asset too large, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/app.verify.app_min.asset_too_large.json 1 X07WASM_APP_VERIFY_FILE_TOO_LARGE

echo "==> phase6_examples: build app_min_spin -> pack -> verify -> canary (budget exceeded)"
rm -rf dist/phase6_examples/app_min_spin dist/phase6_examples/app_min_spin.pack
x07-wasm app build --profile-file examples/app_min/app_release_spin.json \
  --out-dir dist/phase6_examples/app_min_spin --clean \
  --json --report-out build/phase6_examples/app.build.app_min_spin.json --quiet-json
require_report_ok build/phase6_examples/app.build.app_min_spin.json
test -f dist/phase6_examples/app_min_spin/app.bundle.json

x07-wasm app pack --bundle-manifest dist/phase6_examples/app_min_spin/app.bundle.json \
  --out-dir dist/phase6_examples/app_min_spin.pack \
  --profile-id app_min_release_spin \
  --json --report-out build/phase6_examples/app.pack.app_min_spin.json --quiet-json
require_report_ok build/phase6_examples/app.pack.app_min_spin.json
test -f dist/phase6_examples/app_min_spin.pack/app.pack.json

x07-wasm app verify --pack-manifest dist/phase6_examples/app_min_spin.pack/app.pack.json \
  --json --report-out build/phase6_examples/app.verify.app_min_spin.json --quiet-json
require_report_ok build/phase6_examples/app.verify.app_min_spin.json

set +e
x07-wasm app serve --dir dist/phase6_examples/app_min_spin --mode canary \
  --json --report-out build/phase6_examples/app.serve.app_min_spin.canary.json --quiet-json
code=$?
set -e
if [ "$code" -ne 4 ]; then
  echo "expected exit code 4 for app_min_spin budget exceeded, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/app.serve.app_min_spin.canary.json 4 X07WASM_BUDGET_EXCEEDED_CPU_FUEL

echo "==> phase6_examples: provenance attest -> verify (ok)"
x07-wasm provenance attest \
  --pack-manifest dist/phase6_examples/app_min.pack/app.pack.json \
  --ops arch/app/ops/ops_release.json \
  --signing-key arch/provenance/dev.ed25519.signing_key.b64 \
  --out dist/phase6_examples/app_min.pack/provenance.dsse.json \
  --json --report-out build/phase6_examples/provenance.attest.json --quiet-json
require_report_ok build/phase6_examples/provenance.attest.json
test -f dist/phase6_examples/app_min.pack/provenance.dsse.json

ops_hash="$(get_report_result_compatibility_hash build/phase6_examples/ops.validate.release.json)"
att_hash="$(get_attestation_x07_compatibility_hash dist/phase6_examples/app_min.pack/provenance.dsse.json)"
if [ "$ops_hash" != "$att_hash" ]; then
  echo "provenance compatibility_hash mismatch: ops=$ops_hash att=$att_hash" >&2
  exit 1
fi

x07-wasm provenance verify \
  --attestation dist/phase6_examples/app_min.pack/provenance.dsse.json \
  --pack-dir dist/phase6_examples/app_min.pack \
  --trusted-public-key arch/provenance/dev.ed25519.public_key.b64 \
  --json --report-out build/phase6_examples/provenance.verify.ok.json --quiet-json
require_report_ok build/phase6_examples/provenance.verify.ok.json

echo "==> phase6_examples: provenance attest (negative - unsafe symlink subject path)"
rm -rf dist/phase6_examples/app_min.pack.asset_symlink
cp -a dist/phase6_examples/app_min.pack dist/phase6_examples/app_min.pack.asset_symlink
printf "outside" > dist/phase6_examples/outside_asset.bin
symlink_first_pack_asset_file dist/phase6_examples/app_min.pack.asset_symlink/app.pack.json dist/phase6_examples/outside_asset.bin >/dev/null

# Fail-closed invariant: if attest fails, it MUST NOT leave a DSSE output behind.
# Create stale files to ensure the command removes them.
printf "stale" > dist/phase6_examples/app_min.pack.asset_symlink/provenance.bad.dsse.json
printf "stale" > dist/phase6_examples/app_min.pack.asset_symlink/provenance.bad.dsse.json.tmp

set +e
x07-wasm provenance attest \
  --pack-manifest dist/phase6_examples/app_min.pack.asset_symlink/app.pack.json \
  --ops arch/app/ops/ops_release.json \
  --signing-key arch/provenance/dev.ed25519.signing_key.b64 \
  --out dist/phase6_examples/app_min.pack.asset_symlink/provenance.bad.dsse.json \
  --json --report-out build/phase6_examples/provenance.attest.asset_symlink.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for provenance attest unsafe symlink subject path, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/provenance.attest.asset_symlink.json 1 X07WASM_PROVENANCE_SUBJECT_PATH_UNSAFE
test ! -f dist/phase6_examples/app_min.pack.asset_symlink/provenance.bad.dsse.json
test ! -f dist/phase6_examples/app_min.pack.asset_symlink/provenance.bad.dsse.json.tmp

echo "==> phase6_examples: provenance verify (negative - tamper signature)"
cp dist/phase6_examples/app_min.pack/provenance.dsse.json dist/phase6_examples/app_min.pack/provenance.dsse.bad_sig.json
corrupt_dsse_signature_in_place dist/phase6_examples/app_min.pack/provenance.dsse.bad_sig.json

set +e
x07-wasm provenance verify \
  --attestation dist/phase6_examples/app_min.pack/provenance.dsse.bad_sig.json \
  --pack-dir dist/phase6_examples/app_min.pack \
  --trusted-public-key arch/provenance/dev.ed25519.public_key.b64 \
  --json --report-out build/phase6_examples/provenance.verify.bad_sig.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for provenance signature invalid, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/provenance.verify.bad_sig.json 1 X07WASM_PROVENANCE_SIGNATURE_INVALID

echo "==> phase6_examples: provenance verify (negative - tamper asset)"
rm -rf dist/phase6_examples/app_min.pack.tampered
cp -a dist/phase6_examples/app_min.pack dist/phase6_examples/app_min.pack.tampered
corrupt_first_pack_asset_file dist/phase6_examples/app_min.pack.tampered/app.pack.json >/dev/null

set +e
x07-wasm provenance verify \
  --attestation dist/phase6_examples/app_min.pack/provenance.dsse.json \
  --pack-dir dist/phase6_examples/app_min.pack.tampered \
  --trusted-public-key arch/provenance/dev.ed25519.public_key.b64 \
  --json --report-out build/phase6_examples/provenance.verify.bad.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for provenance digest mismatch, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/provenance.verify.bad.json 1 X07WASM_PROVENANCE_DIGEST_MISMATCH

echo "==> phase6_examples: provenance verify (negative - tamper backend component)"
rm -rf dist/phase6_examples/app_min.pack.backend_tampered2
cp -a dist/phase6_examples/app_min.pack dist/phase6_examples/app_min.pack.backend_tampered2
corrupt_pack_backend_component_file dist/phase6_examples/app_min.pack.backend_tampered2/app.pack.json >/dev/null

set +e
x07-wasm provenance verify \
  --attestation dist/phase6_examples/app_min.pack/provenance.dsse.json \
  --pack-dir dist/phase6_examples/app_min.pack.backend_tampered2 \
  --trusted-public-key arch/provenance/dev.ed25519.public_key.b64 \
  --json --report-out build/phase6_examples/provenance.verify.backend_tampered.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for provenance digest mismatch (backend component), got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/provenance.verify.backend_tampered.json 1 X07WASM_PROVENANCE_DIGEST_MISMATCH

echo "==> phase6_examples: provenance verify (negative - tamper bundle manifest)"
rm -rf dist/phase6_examples/app_min.pack.bundle_manifest_tampered2
cp -a dist/phase6_examples/app_min.pack dist/phase6_examples/app_min.pack.bundle_manifest_tampered2
corrupt_pack_bundle_manifest_file dist/phase6_examples/app_min.pack.bundle_manifest_tampered2/app.pack.json >/dev/null

set +e
x07-wasm provenance verify \
  --attestation dist/phase6_examples/app_min.pack/provenance.dsse.json \
  --pack-dir dist/phase6_examples/app_min.pack.bundle_manifest_tampered2 \
  --trusted-public-key arch/provenance/dev.ed25519.public_key.b64 \
  --json --report-out build/phase6_examples/provenance.verify.bundle_manifest_tampered.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for provenance digest mismatch (bundle manifest), got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/provenance.verify.bundle_manifest_tampered.json 1 X07WASM_PROVENANCE_DIGEST_MISMATCH

echo "==> phase6_examples: provenance verify (negative - subject too large)"
rm -rf dist/phase6_examples/app_min.pack.subject_too_large
cp -a dist/phase6_examples/app_min.pack dist/phase6_examples/app_min.pack.subject_too_large
PROVENANCE_MAX_SUBJECT_FILE_BYTES=$((256 * 1024 * 1024))
oversize_first_pack_asset_file dist/phase6_examples/app_min.pack.subject_too_large/app.pack.json $((PROVENANCE_MAX_SUBJECT_FILE_BYTES + 1)) >/dev/null

set +e
x07-wasm provenance verify --attestation dist/phase6_examples/app_min.pack/provenance.dsse.json \
  --pack-dir dist/phase6_examples/app_min.pack.subject_too_large \
  --trusted-public-key arch/provenance/dev.ed25519.public_key.b64 \
  --json --report-out build/phase6_examples/provenance.verify.subject_too_large.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for provenance verify subject too large, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/provenance.verify.subject_too_large.json 1 X07WASM_PROVENANCE_FILE_TOO_LARGE

echo "==> phase6_examples: provenance verify (negative - unsafe symlink subject path)"
set +e
x07-wasm provenance verify \
  --attestation dist/phase6_examples/app_min.pack/provenance.dsse.json \
  --pack-dir dist/phase6_examples/app_min.pack.backend_symlink \
  --trusted-public-key arch/provenance/dev.ed25519.public_key.b64 \
  --json --report-out build/phase6_examples/provenance.verify.subject_path_unsafe.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for provenance subject path unsafe, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/provenance.verify.subject_path_unsafe.json 1 X07WASM_PROVENANCE_SUBJECT_PATH_UNSAFE

echo "==> phase6_examples: provenance verify (negative - unsupported predicateType)"
x07-wasm provenance attest \
  --pack-manifest dist/phase6_examples/app_min.pack/app.pack.json \
  --ops arch/app/ops/ops_release.json \
  --signing-key arch/provenance/dev.ed25519.signing_key.b64 \
  --predicate-type "https://example.com/unsupported" \
  --out dist/phase6_examples/app_min.pack/provenance.dsse.unsupported.json \
  --json --report-out build/phase6_examples/provenance.attest.unsupported_predicate.json --quiet-json
require_report_ok build/phase6_examples/provenance.attest.unsupported_predicate.json

set +e
x07-wasm provenance verify \
  --attestation dist/phase6_examples/app_min.pack/provenance.dsse.unsupported.json \
  --pack-dir dist/phase6_examples/app_min.pack \
  --trusted-public-key arch/provenance/dev.ed25519.public_key.b64 \
  --json --report-out build/phase6_examples/provenance.verify.unsupported_predicate.json --quiet-json
code=$?
set -e
if [ "$code" -ne 1 ]; then
  echo "expected exit code 1 for provenance predicateType unsupported, got $code" >&2
  exit 1
fi
require_report_exit_and_has_code build/phase6_examples/provenance.verify.unsupported_predicate.json 1 X07WASM_PROVENANCE_PREDICATE_TYPE_UNSUPPORTED

echo "==> phase6_examples: deploy plan"
x07-wasm deploy plan \
  --pack-manifest dist/phase6_examples/app_min.pack/app.pack.json \
  --ops arch/app/ops/ops_release_policy_patch.json \
  --out-dir dist/phase6_examples/deploy_plan \
  --json --report-out build/phase6_examples/deploy.plan.json --quiet-json
require_report_ok build/phase6_examples/deploy.plan.json
require_deploy_plan_outputs_exist build/phase6_examples/deploy.plan.json

echo "==> phase6_examples: deploy plan (plan-only; no k8s outputs)"
x07-wasm deploy plan \
  --pack-manifest dist/phase6_examples/app_min.pack/app.pack.json \
  --ops arch/app/ops/ops_release_policy_patch.json \
  --emit-k8s false \
  --out-dir dist/phase6_examples/deploy_plan.no_k8s \
  --json --report-out build/phase6_examples/deploy.plan.no_k8s.json --quiet-json
require_report_ok build/phase6_examples/deploy.plan.no_k8s.json
require_deploy_plan_outputs_absent build/phase6_examples/deploy.plan.no_k8s.json

echo "==> phase6_examples: deploy plan (policy warn)"
x07-wasm deploy plan \
  --pack-manifest dist/phase6_examples/app_min.pack/app.pack.json \
  --ops arch/app/ops/ops_release_policy_warn.json \
  --out-dir dist/phase6_examples/deploy_plan.policy_warn \
  --json --report-out build/phase6_examples/deploy.plan.policy_warn.json --quiet-json
require_report_exit_and_has_code build/phase6_examples/deploy.plan.policy_warn.json 0 X07WASM_POLICY_DECISION_WARN
require_deploy_plan_outputs_exist build/phase6_examples/deploy.plan.policy_warn.json

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

if has_fullstack_showcase_toolchain; then
  echo "==> phase6_examples: official full-stack showcase"
  bash examples/x07_atlas/scripts/ci/check_showcase_fullstack.sh
else
  echo "==> phase6_examples: skipping official full-stack showcase (usable clang/wasm-ld not available)"
fi

echo "phase6_examples: PASS"
