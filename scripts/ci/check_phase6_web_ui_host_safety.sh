#!/usr/bin/env bash
set -euo pipefail

# Guardrail: web-ui host must be XSS-hardened (tag/attr sanitization + CSP, no inline module script).

HOST_DIR="vendor/x07-web-ui/host"
APP_HOST="${HOST_DIR}/app-host.mjs"
INDEX_HTML="${HOST_DIR}/index.html"
BOOTSTRAP_JS="${HOST_DIR}/bootstrap.js"
MAIN_MJS="${HOST_DIR}/main.mjs"
SNAPSHOT_JSON="${HOST_DIR}/host.snapshot.json"

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

if [ ! -f "${APP_HOST}" ]; then
  echo "missing vendored host file: ${APP_HOST}" >&2
  exit 1
fi
if [ ! -f "${INDEX_HTML}" ]; then
  echo "missing vendored host file: ${INDEX_HTML}" >&2
  exit 1
fi
if [ ! -f "${BOOTSTRAP_JS}" ]; then
  echo "missing vendored host file: ${BOOTSTRAP_JS}" >&2
  exit 1
fi
if [ ! -f "${MAIN_MJS}" ]; then
  echo "missing vendored host file: ${MAIN_MJS}" >&2
  exit 1
fi
if [ ! -f "${SNAPSHOT_JSON}" ]; then
  echo "missing vendored host file: ${SNAPSHOT_JSON}" >&2
  exit 1
fi

"${PYTHON}" - "${SNAPSHOT_JSON}" "${HOST_DIR}" <<'PY'
import hashlib
import json
import pathlib
import sys

snapshot_path = pathlib.Path(sys.argv[1])
host_dir = pathlib.Path(sys.argv[2])

snap = json.loads(snapshot_path.read_text(encoding="utf-8"))

req_keys = [
    "schema_version",
    "abi_name",
    "abi_version",
    "bridge_protocol_version",
    "host_abi_hash",
    "assets",
]
for k in req_keys:
    if k not in snap:
        print(f"snapshot missing key: {k}", file=sys.stderr)
        sys.exit(1)

assets = snap["assets"]
if not isinstance(assets, list) or len(assets) == 0:
    print("snapshot.assets must be a non-empty array", file=sys.stderr)
    sys.exit(1)

seen = set()
for a in assets:
    if not isinstance(a, dict):
        print("snapshot.assets entries must be objects", file=sys.stderr)
        sys.exit(1)
    for k in ("path", "sha256", "bytes_len"):
        if k not in a:
            print(f"snapshot.assets entry missing key: {k}", file=sys.stderr)
            sys.exit(1)
    path = a["path"]
    if path in seen:
        print(f"duplicate asset path in snapshot: {path}", file=sys.stderr)
        sys.exit(1)
    seen.add(path)

for a in assets:
    p = host_dir / a["path"]
    if not p.exists() or not p.is_file():
        print(f"missing vendored host asset: {p}", file=sys.stderr)
        sys.exit(1)
    data = p.read_bytes()
    sha = hashlib.sha256(data).hexdigest()
    if sha != a["sha256"]:
        print(
            f"sha256 mismatch for {a['path']}: expected {a['sha256']}, got {sha}",
            file=sys.stderr,
        )
        sys.exit(1)
    if len(data) != int(a["bytes_len"]):
        print(
            f"bytes_len mismatch for {a['path']}: expected {a['bytes_len']}, got {len(data)}",
            file=sys.stderr,
        )
        sys.exit(1)

abi = {
    "abi_name": snap["abi_name"],
    "abi_version": snap["abi_version"],
    "assets": [{"path": a["path"], "sha256": a["sha256"]} for a in assets],
    "bridge_protocol_version": snap["bridge_protocol_version"],
}
abi_bytes = json.dumps(abi, separators=(",", ":"), sort_keys=True).encode("utf-8")
abi_hash = hashlib.sha256(abi_bytes).hexdigest()

expected_hash = snap["host_abi_hash"]
if abi_hash != expected_hash:
    print(
        f"host_abi_hash mismatch: expected {expected_hash}, recomputed {abi_hash}",
        file=sys.stderr,
    )
    sys.exit(1)
PY

grep -q "const TAG_ALLOWLIST" "${APP_HOST}"
grep -q "function sanitizeTag" "${APP_HOST}"
grep -q "function sanitizeAttrs" "${APP_HOST}"
grep -q "name.startsWith(\\\"on\\\")" "${APP_HOST}"

if grep -R "innerHTML" -n "${HOST_DIR}"; then
  echo "web-ui host uses innerHTML (not allowed): ${HOST_DIR}" >&2
  exit 1
fi
if grep -R "document\\.write" -n "${HOST_DIR}"; then
  echo "web-ui host uses document.write (not allowed): ${HOST_DIR}" >&2
  exit 1
fi
if grep -R "onerror" -n "${HOST_DIR}"; then
  echo "web-ui host uses onerror attribute (not allowed): ${HOST_DIR}" >&2
  exit 1
fi

grep -q "Content-Security-Policy" "${INDEX_HTML}"
grep -q "<script type=\\\"module\\\" src=\\\"\\./bootstrap\\.js\\\"></script>" "${INDEX_HTML}"

if grep -q "<script type=\\\"module\\\">" "${INDEX_HTML}"; then
  echo "inline module script is not allowed: ${INDEX_HTML}" >&2
  exit 1
fi

echo "web_ui_host_safety: PASS"
