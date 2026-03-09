#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Re-run Phase0..9 gates first.
bash "${ROOT_DIR}/scripts/ci/check_phase9.sh"

IOS_TEMPLATE_DIR="${ROOT_DIR}/vendor/x07-device-host/mobile/ios/template"
ANDROID_TEMPLATE_DIR="${ROOT_DIR}/vendor/x07-device-host/mobile/android/template"

test -d "${IOS_TEMPLATE_DIR}"
test -d "${ANDROID_TEMPLATE_DIR}"

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

"$PYTHON" - "${IOS_TEMPLATE_DIR}" "${ANDROID_TEMPLATE_DIR}" <<'PY'
import pathlib
import sys

ios_root = pathlib.Path(sys.argv[1])
android_root = pathlib.Path(sys.argv[2])

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
    require_bytes_equal(ios_host / f, android_host / f)

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

android_main = android_root / "app" / "src" / "main" / "java" / "org" / "x07" / "deviceapp" / "MainActivity.kt"
android_main_src = android_main.read_text(encoding="utf-8")
for needle in [
    "x07.device.telemetry.configure",
    "x07.device.telemetry.event",
    "host.webview_crash",
    "application/x-protobuf",
    "application/json",
]:
    if needle not in android_main_src:
        print(f"android vendored template missing telemetry hook: {needle}", file=sys.stderr)
        print(android_main, file=sys.stderr)
        sys.exit(1)

ios_webview = ios_root / "X07DeviceApp" / "X07WebView.swift"
ios_webview_src = ios_webview.read_text(encoding="utf-8")
for needle in [
    "x07.device.telemetry.configure",
    "x07.device.telemetry.event",
    "host.webview_crash",
    "application/x-protobuf",
    "application/json",
]:
    if needle not in ios_webview_src:
        print(f"ios vendored template missing telemetry hook: {needle}", file=sys.stderr)
        print(ios_webview, file=sys.stderr)
        sys.exit(1)

print("ok: phase10 templates")
PY

# Phase10 examples: iOS/Android project generation.
bash "${ROOT_DIR}/scripts/ci/check_phase10_examples.sh"

# Phase10 diagcode allowlist (Phase5..10).
bash "${ROOT_DIR}/scripts/ci/check_phase10_diagcodes.sh"
