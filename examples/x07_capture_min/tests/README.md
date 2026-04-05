`x07 Capture Min` keeps its behavioral coverage in deterministic reducer-call traces under `tests/web_ui/`.

Recommended local loop:

```sh
x07 check --project frontend/x07.json
x07-wasm web-ui build --project frontend/x07.json --profile web_ui_debug --out-dir dist/capture_min/web_ui_debug --clean --json
x07-wasm web-ui test --dist-dir dist/capture_min/web_ui_debug --case tests/web_ui/success.trace.json --json
bash scripts/ci/check_capture_min.sh
```

Use `--update-golden` after changing the reducer shape and keep the env sequence stable so native-effect replay remains explicit.

Current proving traces:

- `tests/web_ui/success.trace.json`: schedule success plus `notification.opened`
- `tests/web_ui/negative.trace.json`: denied, cancelled, timeout, unsupported, and connectivity offline outcomes
- `tests/web_ui/blob_quota.trace.json`: deterministic `blob_item_too_large` and `blob_total_too_large` import failures
- `tests/web_ui/notification_cancel.trace.json`: schedule followed by `notifications.cancel`

Native incident fixtures live under `tests/native_incidents/`:

- `permission_blocked`: denied permission path with explicit replay hints
- `location_timeout`: synthesized replay from `native_bridge_timeout` breadcrumbs
- `policy_violation`: explicit replay hints for a policy denial path
- `host_webview_crash`: synthesized replay from host crash context
