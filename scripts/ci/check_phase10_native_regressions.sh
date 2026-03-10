#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FIXTURE_ROOT="${ROOT_DIR}/examples/x07_capture_min/tests/native_incidents"
OUT_ROOT="${ROOT_DIR}/build/phase10_native_regressions"

rm -rf "${OUT_ROOT}"
mkdir -p "${OUT_ROOT}"

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

compare_json() {
  local expected_path="$1"
  local actual_path="$2"
  local report_path="$3"

  "$PYTHON" - "$expected_path" "$actual_path" "$report_path" <<'PY'
import json
import pathlib
import sys

expected_path = pathlib.Path(sys.argv[1])
actual_path = pathlib.Path(sys.argv[2])
report_path = pathlib.Path(sys.argv[3])

expected = json.loads(expected_path.read_text(encoding="utf-8"))
actual = json.loads(actual_path.read_text(encoding="utf-8"))
report = json.loads(report_path.read_text(encoding="utf-8"))

if expected != actual:
    raise SystemExit(f"{actual_path}: generated replay fixture drifted from {expected_path}")

if report.get("schema_version") != "x07.wasm.device.regress.from_incident.report@0.2.0":
    raise SystemExit(f"{report_path}: bad schema_version {report.get('schema_version')!r}")

result = report.get("result", {})
if result.get("replay_mode") != "platform_native_v1":
    raise SystemExit(f"{report_path}: bad replay_mode {result.get('replay_mode')!r}")
if result.get("replay_synthesis_status") != "generated":
    raise SystemExit(f"{report_path}: bad replay_synthesis_status {result.get('replay_synthesis_status')!r}")
if not result.get("generated_trace_artifact_refs"):
    raise SystemExit(f"{report_path}: missing generated_trace_artifact_refs")
if not result.get("generated_report_artifact_refs"):
    raise SystemExit(f"{report_path}: missing generated_report_artifact_refs")

print("ok:", actual_path)
PY
}

for case_dir in "${FIXTURE_ROOT}"/*; do
  if [ ! -d "${case_dir}" ]; then
    continue
  fi
  case_name="$(basename "${case_dir}")"
  out_dir="${OUT_ROOT}/${case_name}"
  report_path="${OUT_ROOT}/${case_name}.report.json"
  expected_path="${case_dir}/expected/${case_name}.native.replay.json"
  actual_path="${out_dir}/${case_name}.native.replay.json"

  echo "==> phase10_native_regressions: ${case_name}"
  x07-wasm device regress from-incident \
    "${case_dir}" \
    --out-dir "${out_dir}" \
    --name "${case_name}" \
    --json --report-out "${report_path}" --quiet-json

  test -f "${actual_path}"
  compare_json "${expected_path}" "${actual_path}" "${report_path}"
done

echo "phase10_native_regressions: PASS"
