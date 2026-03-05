#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Re-run Phase0..9 gates first.
bash "${ROOT_DIR}/scripts/ci/check_phase9.sh"

IOS_TEMPLATE_DIR="${ROOT_DIR}/crates/x07-wasm/src/support/mobile/ios/template"
ANDROID_TEMPLATE_DIR="${ROOT_DIR}/crates/x07-wasm/src/support/mobile/android/template"
VENDORED_HOST_DIR="${ROOT_DIR}/vendor/x07-web-ui/host"

test -d "${IOS_TEMPLATE_DIR}"
test -d "${ANDROID_TEMPLATE_DIR}"
test -d "${VENDORED_HOST_DIR}"

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

"$PYTHON" - "${IOS_TEMPLATE_DIR}" "${ANDROID_TEMPLATE_DIR}" "${VENDORED_HOST_DIR}" <<'PY'
import pathlib
import sys

ios_root = pathlib.Path(sys.argv[1])
android_root = pathlib.Path(sys.argv[2])
vendored_host = pathlib.Path(sys.argv[3])

host_files = ["index.html", "bootstrap.js", "main.mjs", "app-host.mjs"]

def require_bytes_equal(a: pathlib.Path, b: pathlib.Path) -> None:
    if not a.is_file():
        raise SystemExit(f"missing file: {a}")
    if not b.is_file():
        raise SystemExit(f"missing file: {b}")
    ba = a.read_bytes()
    bb = b.read_bytes()
    if ba != bb:
        raise SystemExit(f"host file mismatch: {a} != {b}")

ios_host = ios_root / "X07DeviceApp" / "x07"
android_host = android_root / "app" / "src" / "main" / "assets" / "x07"

for f in host_files:
    ref = vendored_host / f
    require_bytes_equal(ref, ios_host / f)
    require_bytes_equal(ref, android_host / f)

token_targets = [
    (
        "ios",
        ios_root,
        ["__X07_DISPLAY_NAME__", "__X07_IOS_BUNDLE_ID__", "__X07_VERSION__", "__X07_BUILD__"],
    ),
    (
        "android",
        android_root,
        [
            "__X07_DISPLAY_NAME__",
            "__X07_ANDROID_APPLICATION_ID__",
            "__X07_ANDROID_MIN_SDK__",
            "__X07_VERSION__",
            "__X07_BUILD__",
        ],
    ),
]

def iter_files(d: pathlib.Path) -> list[pathlib.Path]:
    out: list[pathlib.Path] = []
    for p in d.rglob("*"):
        if not p.is_file():
            continue
        if p.name == ".DS_Store" or p.name.startswith("._"):
            continue
        out.append(p)
    return out

for platform, dir_path, tokens in token_targets:
    files = iter_files(dir_path)
    hay = b""
    for p in files:
        hay += p.read_bytes()
    for tok in tokens:
        if tok.encode("utf-8") not in hay:
            print(f"missing token {tok} under {dir_path}", file=sys.stderr)
            sys.exit(1)

print("ok: phase10 templates")
PY

# Phase10 examples: iOS/Android project generation.
bash "${ROOT_DIR}/scripts/ci/check_phase10_examples.sh"

# Phase10 diagcode allowlist (Phase5..10).
bash "${ROOT_DIR}/scripts/ci/check_phase10_diagcodes.sh"
