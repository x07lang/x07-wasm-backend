#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/phase7_examples"
TMP_DIR="${ROOT_DIR}/build/.tmp_phase7_no_c"

rm -rf "${OUT_DIR}" "${TMP_DIR}"
mkdir -p "${OUT_DIR}" "${TMP_DIR}/bin" build/phase7_examples .x07-wasm/incidents

# Shadow clang/wasm-ld so any accidental invocation hard-fails.
cat > "${TMP_DIR}/bin/clang" <<'SH'
#!/usr/bin/env bash
echo "phase7 gate: clang must not be invoked (native backend)" >&2
exit 97
SH
chmod +x "${TMP_DIR}/bin/clang"

cat > "${TMP_DIR}/bin/wasm-ld" <<'SH'
#!/usr/bin/env bash
echo "phase7 gate: wasm-ld must not be invoked (native backend)" >&2
exit 98
SH
chmod +x "${TMP_DIR}/bin/wasm-ld"

export PATH="${TMP_DIR}/bin:${PATH}"

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

require_build_uses_native_backend() {
  local report_path="$1"
  "$PYTHON" - "$report_path" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
doc = json.loads(p.read_text(encoding="utf-8"))
backend = doc.get("result", {}).get("flags", {}).get("codegen_backend")
if backend != "native_x07_wasm_v1":
    print("expected native_x07_wasm_v1, got:", backend, "report:", p, file=sys.stderr)
    sys.exit(1)
print("ok:", p, "codegen_backend=native_x07_wasm_v1")
PY
}

echo "==> phase7_examples: solve_pure_echo build + run (no clang/wasm-ld)"
mkdir -p "${OUT_DIR}/solve_pure_echo"
x07-wasm build \
  --project examples/solve_pure_echo/x07.json \
  --profile wasm_release \
  --out "${OUT_DIR}/solve_pure_echo/solve.wasm" \
  --artifact-out "${OUT_DIR}/solve_pure_echo/solve.wasm.manifest.json" \
  --json --report-out build/phase7_examples/build.solve_pure_echo.json --quiet-json
require_build_uses_native_backend build/phase7_examples/build.solve_pure_echo.json

x07-wasm run \
  --wasm "${OUT_DIR}/solve_pure_echo/solve.wasm" \
  --input examples/solve_pure_echo/tests/fixtures/in_hello.bin \
  --output-out "${OUT_DIR}/solve_pure_echo/out.bin" \
  --json --report-out build/phase7_examples/run.solve_pure_echo.json --quiet-json
cmp examples/solve_pure_echo/tests/fixtures/in_hello.bin "${OUT_DIR}/solve_pure_echo/out.bin"

echo "==> phase7_examples: solve_pure_spin build + run (no clang/wasm-ld)"
mkdir -p "${OUT_DIR}/solve_pure_spin"
x07-wasm build \
  --project examples/solve_pure_spin/x07.json \
  --profile wasm_release \
  --out "${OUT_DIR}/solve_pure_spin/solve.wasm" \
  --artifact-out "${OUT_DIR}/solve_pure_spin/solve.wasm.manifest.json" \
  --json --report-out build/phase7_examples/build.solve_pure_spin.json --quiet-json
require_build_uses_native_backend build/phase7_examples/build.solve_pure_spin.json

x07-wasm run \
  --wasm "${OUT_DIR}/solve_pure_spin/solve.wasm" \
  --input examples/solve_pure_spin/tests/fixtures/in_small.bin \
  --output-out "${OUT_DIR}/solve_pure_spin/out.bin" \
  --json --report-out build/phase7_examples/run.solve_pure_spin.json --quiet-json
cmp examples/solve_pure_spin/tests/fixtures/out_small.bin "${OUT_DIR}/solve_pure_spin/out.bin"

echo "==> phase7_examples: component build + app build (no clang/wasm-ld)"
mkdir -p "${OUT_DIR}/app_min"
x07-wasm component build \
  --project examples/app_min/backend/x07.json \
  --profile component_release \
  --emit http \
  --out-dir "${OUT_DIR}/app_min/component_backend" \
  --clean \
  --json --report-out build/phase7_examples/component.build.app_min_backend.json --quiet-json
test -f "${OUT_DIR}/app_min/component_backend/http.component.wasm"

x07-wasm app build \
  --profile-file examples/app_min/app_release.json \
  --out-dir "${OUT_DIR}/app_min/app_bundle" \
  --clean \
  --json --report-out build/phase7_examples/app.build.app_min.json --quiet-json
test -f "${OUT_DIR}/app_min/app_bundle/app.bundle.json"
test -f "${OUT_DIR}/app_min/app_bundle/frontend/index.html"
test -f "${OUT_DIR}/app_min/app_bundle/backend/app.http.component.wasm"

