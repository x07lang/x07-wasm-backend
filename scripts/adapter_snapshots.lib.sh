#!/usr/bin/env bash
set -euo pipefail

x07_adapter_manifest_path() {
  local adapter="$1"
  case "$adapter" in
    http) echo "guest/http-adapter/Cargo.toml" ;;
    cli) echo "guest/cli-adapter/Cargo.toml" ;;
    web-ui) echo "guest/web-ui-adapter/Cargo.toml" ;;
    *)
      echo "unknown adapter: $adapter" >&2
      return 2
      ;;
  esac
}

x07_adapter_guest_output_path() {
  local adapter="$1"
  case "$adapter" in
    http) echo "guest/http-adapter/target/wasm32-wasip2/release/x07_wasm_http_adapter.wasm" ;;
    cli) echo "guest/cli-adapter/target/wasm32-wasip2/release/x07_wasm_cli_adapter.wasm" ;;
    web-ui) echo "guest/web-ui-adapter/target/wasm32-wasip2/release/x07_wasm_web_ui_adapter.wasm" ;;
    *)
      echo "unknown adapter: $adapter" >&2
      return 2
      ;;
  esac
}

x07_adapter_embedded_snapshot_path() {
  local adapter="$1"
  case "$adapter" in
    http) echo "crates/x07-wasm/src/support/adapters/http-adapter.component.wasm" ;;
    cli) echo "crates/x07-wasm/src/support/adapters/cli-adapter.component.wasm" ;;
    web-ui) echo "crates/x07-wasm/src/support/adapters/web-ui-adapter.component.wasm" ;;
    *)
      echo "unknown adapter: $adapter" >&2
      return 2
      ;;
  esac
}

