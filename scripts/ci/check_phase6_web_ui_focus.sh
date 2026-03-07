#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_FILE="${ROOT_DIR}/vendor/x07-web-ui/host/tests/focus_retention.test.mjs"

if ! command -v node >/dev/null 2>&1; then
  echo "node not found on PATH" >&2
  exit 1
fi

node --test "${TEST_FILE}"
