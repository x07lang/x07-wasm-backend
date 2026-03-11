#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MCP_ROOT="${ROOT_DIR}/../x07-mcp"
SERVER_DIR="${MCP_ROOT}/servers/x07lang-mcp"
OUT_DIR="${ROOT_DIR}/dist/phase10_mcp_inspect"
REPORT_DIR="${ROOT_DIR}/build/phase10_mcp_inspect"
X07_MCP_CLI="${MCP_ROOT}/dist/x07-mcp"
ROUTER_BIN="${SERVER_DIR}/out/x07lang-mcp"
WORKER_BIN="${SERVER_DIR}/out/mcp-worker"
REPO_X07_EXE="${ROOT_DIR}/../x07/target/debug/x07"

if [[ ! -d "${MCP_ROOT}" ]]; then
  echo "phase10_mcp_inspect: skip (missing sibling repo ../x07-mcp)"
  exit 0
fi

if [[ -x "${REPO_X07_EXE}" ]]; then
  X07_EXE="${REPO_X07_EXE}"
else
  X07_EXE="$(command -v x07 || true)"
fi

if [[ -z "${X07_EXE}" ]]; then
  echo "phase10_mcp_inspect: missing x07 executable" >&2
  exit 1
fi

mkdir -p "${OUT_DIR}" "${REPORT_DIR}"
rm -f "${OUT_DIR}"/*.json

X07_BIN_DIR="$(cd "$(dirname "${X07_EXE}")" && pwd)"

if [[ ! -x "${X07_MCP_CLI}" || "${MCP_ROOT}/cli/src/app.x07.json" -nt "${X07_MCP_CLI}" || "${MCP_ROOT}/scripts/forge/mcp_inspect.py" -nt "${X07_MCP_CLI}" ]]; then
  echo "==> phase10_mcp_inspect: bundle x07-mcp CLI"
  (
    cd "${MCP_ROOT}"
    PATH="${X07_BIN_DIR}:$PATH" x07 bundle --project x07.json --profile os --out dist/x07-mcp >/dev/null
  )
else
  echo "==> phase10_mcp_inspect: reuse x07-mcp CLI"
fi

if [[ ! -x "${ROUTER_BIN}" || ! -x "${WORKER_BIN}" || "${SERVER_DIR}/src/app.x07.json" -nt "${ROUTER_BIN}" || "${SERVER_DIR}/src/mcp/runtime.x07.json" -nt "${ROUTER_BIN}" || "${SERVER_DIR}/src/mcp/user.x07.json" -nt "${WORKER_BIN}" ]]; then
  echo "==> phase10_mcp_inspect: hydrate x07lang-mcp deps"
  (
    cd "${SERVER_DIR}"
    PATH="${X07_BIN_DIR}:$PATH" ../../servers/_shared/ci/install_server_deps.sh . >/dev/null
  )

  echo "==> phase10_mcp_inspect: build x07lang-mcp bins"
  (
    cd "${SERVER_DIR}"
    PATH="${X07_BIN_DIR}:$PATH" X07_MCP_BUILD_BINS_ONLY=1 ./publish/build_mcpb.sh >/dev/null
  )
else
  echo "==> phase10_mcp_inspect: reuse x07lang-mcp bins"
fi

PORT="$(
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
)"

CFG_TMP="$(mktemp "${SERVER_DIR}/config/mcp.server.phase10.XXXXXX.json")"
CFG_TMP_NAME="$(basename "${CFG_TMP}")"
cleanup() {
  local status="$?"
  if [[ -n "${SERVER_PID:-}" ]]; then
    kill -TERM "${SERVER_PID}" >/dev/null 2>&1 || true
    wait "${SERVER_PID}" >/dev/null 2>&1 || true
  fi
  rm -f "${CFG_TMP}"
  exit "${status}"
}
trap cleanup EXIT

python3 - "${SERVER_DIR}/config/mcp.server.dev.json" "${CFG_TMP}" "${PORT}" <<'PY'
import json
import pathlib
import sys

src = pathlib.Path(sys.argv[1])
dst = pathlib.Path(sys.argv[2])
port = int(sys.argv[3])

doc = json.loads(src.read_text(encoding="utf-8"))
doc.setdefault("transports", {}).setdefault("http", {})["bind"] = f"127.0.0.1:{port}"
dst.write_text(json.dumps(doc, separators=(",", ":")), encoding="utf-8")
PY

SERVER_URL="http://127.0.0.1:${PORT}/mcp"
SERVER_LOG="${REPORT_DIR}/x07lang-mcp.http.log"

echo "==> phase10_mcp_inspect: launch x07lang-mcp (${SERVER_URL})"
(
  cd "${SERVER_DIR}"
  X07_MCP_CFG_PATH="config/${CFG_TMP_NAME}" \
  X07_MCP_X07_EXE="${X07_EXE}" \
  ./out/x07lang-mcp >"${SERVER_LOG}" 2>&1
) &
SERVER_PID="$!"

ready=0
for _ in $(seq 1 120); do
  if ! kill -0 "${SERVER_PID}" >/dev/null 2>&1; then
    echo "phase10_mcp_inspect: x07lang-mcp exited early" >&2
    tail -n 200 "${SERVER_LOG}" >&2 || true
    exit 1
  fi
  if python3 - "${PORT}" <<'PY'
import socket
import sys

port = int(sys.argv[1])
s = socket.socket()
s.settimeout(0.2)
try:
    s.connect(("127.0.0.1", port))
except OSError:
    raise SystemExit(1)
finally:
    s.close()
PY
  then
    ready=1
    break
  fi
  sleep 0.1
done

if [[ "${ready}" != "1" ]]; then
  echo "phase10_mcp_inspect: x07lang-mcp did not bind ${SERVER_URL}" >&2
  tail -n 200 "${SERVER_LOG}" >&2 || true
  exit 1
fi

echo "==> phase10_mcp_inspect: inspect initialize"
(
  cd "${MCP_ROOT}"
  "${X07_MCP_CLI}" inspect initialize \
    --url "${SERVER_URL}" \
    --machine json >"${OUT_DIR}/inspect.initialize.json"
)

echo "==> phase10_mcp_inspect: inspect tools"
(
  cd "${MCP_ROOT}"
  "${X07_MCP_CLI}" inspect tools \
    --url "${SERVER_URL}" \
    --machine json >"${OUT_DIR}/inspect.tools.json"
)

python3 - "${OUT_DIR}/inspect.initialize.json" "${OUT_DIR}/inspect.tools.json" <<'PY'
import json
import pathlib
import sys

initialize_path = pathlib.Path(sys.argv[1])
tools_path = pathlib.Path(sys.argv[2])

init_doc = json.loads(initialize_path.read_text(encoding="utf-8"))
tools_doc = json.loads(tools_path.read_text(encoding="utf-8"))

for path, doc, op in [
    (initialize_path, init_doc, "initialize"),
    (tools_path, tools_doc, "tools"),
]:
    if doc.get("schema_version") != "x07.mcp.inspect.result@0.1.0":
        raise SystemExit(f"{path}: bad schema_version {doc.get('schema_version')!r}")
    if doc.get("ok") is not True:
        raise SystemExit(f"{path}: inspect command was not ok")
    if doc.get("operation") != op:
        raise SystemExit(f"{path}: bad operation {doc.get('operation')!r}")
    transport = doc.get("transport")
    if not isinstance(transport, dict) or transport.get("kind") != "http":
        raise SystemExit(f"{path}: bad transport {transport!r}")

server_info = (
    init_doc.get("initialize", {})
    .get("initialize", {})
    .get("response", {})
    .get("body", {})
    .get("json", {})
    .get("result", {})
    .get("serverInfo", {})
)
if server_info.get("name") != "io.x07/x07lang-mcp":
    raise SystemExit(f"{initialize_path}: unexpected server name {server_info!r}")

tools_result = (
    tools_doc.get("rpc", {})
    .get("response", {})
    .get("body", {})
    .get("json", {})
    .get("result", {})
)
tools = tools_result.get("tools")
if not isinstance(tools, list) or not tools:
    raise SystemExit(f"{tools_path}: expected non-empty tools list")

tool_names = {tool.get("name") for tool in tools if isinstance(tool, dict)}
for want_name in ("x07.search_v1", "x07.patch_apply_v1"):
    if want_name not in tool_names:
        raise SystemExit(f"{tools_path}: missing expected tool {want_name!r}")

print("phase10_mcp_inspect: PASS")
PY
