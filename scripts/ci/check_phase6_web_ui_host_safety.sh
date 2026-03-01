#!/usr/bin/env bash
set -euo pipefail

# Guardrail: web-ui host must be XSS-hardened (tag/attr sanitization + CSP, no inline module script).

HOST_DIR="vendor/x07-web-ui/host"
APP_HOST="${HOST_DIR}/app-host.mjs"
INDEX_HTML="${HOST_DIR}/index.html"
MAIN_MJS="${HOST_DIR}/main.mjs"

if [ ! -f "${APP_HOST}" ]; then
  echo "missing vendored host file: ${APP_HOST}" >&2
  exit 1
fi
if [ ! -f "${INDEX_HTML}" ]; then
  echo "missing vendored host file: ${INDEX_HTML}" >&2
  exit 1
fi
if [ ! -f "${MAIN_MJS}" ]; then
  echo "missing vendored host file: ${MAIN_MJS}" >&2
  exit 1
fi

grep -q "const TAG_ALLOWLIST" "${APP_HOST}"
grep -q "function sanitizeTag" "${APP_HOST}"
grep -q "function sanitizeAttrs" "${APP_HOST}"
grep -q "name.startsWith(\\\"on\\\")" "${APP_HOST}"

if grep -q "innerHTML" "${APP_HOST}"; then
  echo "web-ui host uses innerHTML (not allowed): ${APP_HOST}" >&2
  exit 1
fi

grep -q "Content-Security-Policy" "${INDEX_HTML}"
grep -q "<script type=\\\"module\\\" src=\\\"\\./main\\.mjs\\\"></script>" "${INDEX_HTML}"

if grep -q "<script type=\\\"module\\\">" "${INDEX_HTML}"; then
  echo "inline module script is not allowed: ${INDEX_HTML}" >&2
  exit 1
fi

echo "web_ui_host_safety: PASS"
