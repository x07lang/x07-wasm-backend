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

echo "==> gate: toolchain"
x07-wasm doctor --json --report-out build/wasm/doctor.json --quiet-json
x07-wasm cli specrows check --json --report-out build/wasm/cli.specrows.check.json --quiet-json
x07-wasm wit validate --json --report-out build/wasm/wit.validate.json --quiet-json
x07-wasm component profile validate --json --report-out build/wasm/component.profile.validate.json --quiet-json

echo "==> gate: contracts"
x07-wasm app contracts validate --json --report-out build/wasm/app.contracts.validate.json --quiet-json
x07-wasm app profile validate --json --report-out build/wasm/app.profile.validate.json --quiet-json

echo "==> gate: build + serve + test (app_dev)"
x07-wasm app build --profile app_dev --clean --json --report-out build/wasm/app.build.app_dev.json --quiet-json
x07-wasm app serve --mode smoke --strict-mime --json --report-out build/wasm/app.serve.app_dev.json --quiet-json
x07-wasm app test --trace examples/app_fullstack_hello/tests/trace_0001.json --json --report-out build/wasm/app.test.trace_0001.json --quiet-json

echo "==> gate: regress-from-incident (fixture)"
incident_dir="build/wasm/app_incident_fixture"
rm -rf "${incident_dir}"
mkdir -p "${incident_dir}"
cp examples/app_fullstack_hello/tests/trace_0001.json "${incident_dir}/trace.json"
x07-wasm app regress from-incident "${incident_dir}" --out-dir build/wasm/regress --name app_fullstack_hello \
  --json --report-out build/wasm/app.regress.from_incident.json --quiet-json
test -f build/wasm/regress/app_fullstack_hello.trace.json
test -f build/wasm/regress/app_fullstack_hello.final.ui.json

echo "phase3: PASS"

