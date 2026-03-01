#!/usr/bin/env bash
set -euo pipefail

# Guardrail: shipped wasm profiles must be safe-by-default (finite CPU + memory).
#
# No external validators: uses Python only for JSON parsing.

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

"$PYTHON" - <<'PY'
import json
import pathlib
import sys

paths = [
    pathlib.Path("arch/wasm/profiles/wasm_debug.json"),
    pathlib.Path("arch/wasm/profiles/wasm_release.json"),
    pathlib.Path("arch/wasm/profiles/wasm_web_ui_debug.json"),
    pathlib.Path("arch/wasm/profiles/wasm_web_ui_release.json"),
]

violations = []
for p in paths:
    try:
        doc = json.loads(p.read_text(encoding="utf-8"))
    except Exception as e:
        violations.append(f"{p}: invalid_json:{e.__class__.__name__}")
        continue

    rt = doc.get("runtime")
    if not isinstance(rt, dict):
        violations.append(f"{p}: missing runtime object")
        continue

    for key in ["max_fuel", "max_memory_bytes", "max_table_elements", "max_wasm_stack_bytes"]:
        v = rt.get(key)
        if v is None:
            violations.append(f"{p}: runtime.{key} is null")
            continue
        if not isinstance(v, int) or v < 1:
            violations.append(f"{p}: runtime.{key} must be int >= 1 (got {v!r})")

if violations:
    print("profile_defaults: FAIL", file=sys.stderr)
    for v in violations:
        print("  -", v, file=sys.stderr)
    sys.exit(1)

print("profile_defaults: PASS")
PY
