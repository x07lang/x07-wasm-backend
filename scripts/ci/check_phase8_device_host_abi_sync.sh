#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SNAPSHOT="$ROOT_DIR/vendor/x07-device-host/host_abi.snapshot.json"
HOST_ABI_RS="$ROOT_DIR/crates/x07-wasm/src/device/host_abi.rs"

if [[ ! -f "$SNAPSHOT" ]]; then
  echo "missing vendored device-host ABI snapshot: $SNAPSHOT" >&2
  exit 1
fi
if [[ ! -f "$HOST_ABI_RS" ]]; then
  echo "missing host_abi.rs: $HOST_ABI_RS" >&2
  exit 1
fi

python3 - "$SNAPSHOT" "$HOST_ABI_RS" <<'PY'
import json
import pathlib
import re
import sys

snapshot_path = pathlib.Path(sys.argv[1])
rs_path = pathlib.Path(sys.argv[2])

snap = json.loads(snapshot_path.read_text(encoding="utf-8"))
expected_hash = snap.get("host_abi_hash")
abi = snap.get("abi") if isinstance(snap, dict) else None
expected_abi_name = abi.get("abi_name") if isinstance(abi, dict) else None
expected_abi_version = abi.get("abi_version") if isinstance(abi, dict) else None

if not isinstance(expected_hash, str) or not re.fullmatch(r"[0-9a-f]{64}", expected_hash):
    print(f"invalid host_abi_hash in snapshot: {expected_hash!r}", file=sys.stderr)
    sys.exit(1)

text = rs_path.read_text(encoding="utf-8")

m = re.search(r'HOST_ABI_HASH_HEX:\s*&str\s*=\s*"([0-9a-f]{64})"', text)
if not m:
    print("cannot parse HOST_ABI_HASH_HEX from host_abi.rs", file=sys.stderr)
    sys.exit(2)
actual_hash = m.group(1)

m = re.search(r'ABI_NAME:\s*&str\s*=\s*"([^"]+)"', text)
if not m:
    print("cannot parse ABI_NAME from host_abi.rs", file=sys.stderr)
    sys.exit(2)
actual_abi_name = m.group(1)

m = re.search(r'ABI_VERSION:\s*&str\s*=\s*"([^"]+)"', text)
if not m:
    print("cannot parse ABI_VERSION from host_abi.rs", file=sys.stderr)
    sys.exit(2)
actual_abi_version = m.group(1)

ok = True
if actual_hash != expected_hash:
    print(
        f"device host ABI hash mismatch:\n  host_abi.rs: {actual_hash}\n  snapshot:   {expected_hash}",
        file=sys.stderr,
    )
    ok = False
if expected_abi_name is not None and actual_abi_name != expected_abi_name:
    print(
        f"device host ABI name mismatch:\n  host_abi.rs: {actual_abi_name}\n  snapshot:   {expected_abi_name}",
        file=sys.stderr,
    )
    ok = False
if expected_abi_version is not None and actual_abi_version != expected_abi_version:
    print(
        f"device host ABI version mismatch:\n  host_abi.rs: {actual_abi_version}\n  snapshot:   {expected_abi_version}",
        file=sys.stderr,
    )
    ok = False

if not ok:
    sys.exit(1)

PY

echo "phase8_device_host_abi_sync: PASS"
