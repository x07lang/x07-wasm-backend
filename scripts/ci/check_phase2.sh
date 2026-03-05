#!/usr/bin/env bash
set -euo pipefail

mkdir -p build/wasm dist target .x07-wasm/incidents

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

read_snapshot_git_sha() {
  "$PYTHON" - <<'PY'
import json
import pathlib
doc = json.loads(pathlib.Path("vendor/x07-web-ui/snapshot.json").read_text(encoding="utf-8"))
print(doc.get("git_sha",""))
PY
}

write_incident_from_trace() {
  local trace_path="$1"
  local out_path="$2"
  "$PYTHON" - "$trace_path" "$out_path" <<'PY'
import json
import pathlib
import sys

trace = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
incident = {
  "v": 1,
  "kind": "x07.web_ui.incident",
  "error": "fixture",
  "trace": trace,
}
out = pathlib.Path(sys.argv[2])
out.parent.mkdir(parents=True, exist_ok=True)
out.write_text(json.dumps(incident, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

echo "==> gate: toolchain"
x07-wasm doctor --json --report-out build/wasm/doctor.json --quiet-json
x07-wasm profile validate --json --report-out build/wasm/profile.validate.json --quiet-json
x07-wasm cli specrows check --json --report-out build/wasm/cli.specrows.check.json --quiet-json
x07-wasm wit validate --json --report-out build/wasm/wit.validate.json --quiet-json
x07-wasm component profile validate --json --report-out build/wasm/component.profile.validate.json --quiet-json

echo "==> gate: embedded web-ui adapter snapshot"
if ! command -v wasm-tools >/dev/null 2>&1; then
  echo "wasm-tools not found on PATH (required to canonicalize adapter snapshots)" >&2
  exit 1
fi

canon_tmp_dir="$(mktemp -d)"
cleanup_canon_tmp_dir() {
  rm -rf "${canon_tmp_dir}"
}
trap cleanup_canon_tmp_dir EXIT

cargo build --release --locked --target wasm32-wasip2 --manifest-path guest/web-ui-adapter/Cargo.toml

wasm-tools strip -a guest/web-ui-adapter/target/wasm32-wasip2/release/x07_wasm_web_ui_adapter.wasm -o "${canon_tmp_dir}/web-ui-adapter.guest.wasm"
wasm-tools strip -a crates/x07-wasm/src/support/adapters/web-ui-adapter.component.wasm -o "${canon_tmp_dir}/web-ui-adapter.embedded.wasm"
if ! cmp -s "${canon_tmp_dir}/web-ui-adapter.guest.wasm" "${canon_tmp_dir}/web-ui-adapter.embedded.wasm"; then
  echo "ERROR: embedded web-ui-adapter.component.wasm is out of sync with guest/web-ui-adapter output" >&2
  exit 1
fi

echo "==> gate: contracts"
x07-wasm web-ui contracts validate --json --report-out build/wasm/web-ui.contracts.validate.json --quiet-json
x07-wasm web-ui profile validate --json --report-out build/wasm/web-ui.profile.validate.json --quiet-json

echo "==> gate: vendored snapshots"
sha="$(read_snapshot_git_sha)"
if [[ "${sha}" == "" ]]; then
  echo "missing vendor/x07-web-ui/snapshot.json git_sha" >&2
  exit 1
fi
${PYTHON} scripts/vendor_x07_web_ui.py check

web_ui_root="$(cd "vendor/x07-web-ui" && pwd)"
export X07_WORKSPACE_ROOT="${web_ui_root}"

counter_project="${web_ui_root}/examples/web_ui_counter/x07.json"
counter_trace="${web_ui_root}/examples/web_ui_counter/tests/counter.trace.json"
form_project="${web_ui_root}/examples/web_ui_form/x07.json"
form_trace="${web_ui_root}/examples/web_ui_form/tests/form.trace.json"

echo "==> gate: core build + serve + test (counter)"
counter_core_dir="dist/web_ui_counter_core"
x07-wasm web-ui build --project "${counter_project}" --profile web_ui_debug --out-dir "${counter_core_dir}" --clean \
  --json --report-out build/wasm/web-ui.build.counter.core.json --quiet-json
x07-wasm web-ui serve --dir "${counter_core_dir}" --mode smoke --strict-mime \
  --json --report-out build/wasm/web-ui.serve.counter.core.json --quiet-json
x07-wasm web-ui test --dist-dir "${counter_core_dir}" --case "${counter_trace}" \
  --json --report-out build/wasm/web-ui.test.counter.core.json --quiet-json

echo "==> gate: component build + test (counter)"
counter_component_dir="dist/web_ui_counter_component"
x07-wasm web-ui build --project "${counter_project}" --profile web_ui_debug --out-dir "${counter_component_dir}" --clean --format component \
  --json --report-out build/wasm/web-ui.build.counter.component.json --quiet-json
x07-wasm web-ui test --dist-dir "${counter_component_dir}" --case "${counter_trace}" \
  --json --report-out build/wasm/web-ui.test.counter.component.json --quiet-json

echo "==> gate: core build + test (form)"
form_core_dir="dist/web_ui_form_core"
x07-wasm web-ui build --project "${form_project}" --profile web_ui_debug --out-dir "${form_core_dir}" --clean \
  --json --report-out build/wasm/web-ui.build.form.core.json --quiet-json
x07-wasm web-ui test --dist-dir "${form_core_dir}" --case "${form_trace}" \
  --json --report-out build/wasm/web-ui.test.form.core.json --quiet-json

echo "==> gate: component build + test (form)"
form_component_dir="dist/web_ui_form_component"
x07-wasm web-ui build --project "${form_project}" --profile web_ui_debug --out-dir "${form_component_dir}" --clean --format component \
  --json --report-out build/wasm/web-ui.build.form.component.json --quiet-json
x07-wasm web-ui test --dist-dir "${form_component_dir}" --case "${form_trace}" \
  --json --report-out build/wasm/web-ui.test.form.component.json --quiet-json

echo "==> gate: regress-from-incident"
incident="build/wasm/counter.incident.json"
write_incident_from_trace "${counter_trace}" "${incident}"
x07-wasm web-ui regress-from-incident --incident "${incident}" --out-dir build/wasm/regress --name counter \
  --json --report-out build/wasm/web-ui.regress.counter.json --quiet-json
test -f build/wasm/regress/counter.trace.json
test -f build/wasm/regress/counter.final.ui.json

echo "phase2: PASS"
