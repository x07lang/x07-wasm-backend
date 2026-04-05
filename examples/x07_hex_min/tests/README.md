`x07 Hex Min` keeps its behavioral coverage in deterministic reducer-call traces under `tests/web_ui/`.

Recommended local loop:

```sh
x07 check --project frontend/x07.json
x07-wasm web-ui build --project frontend/x07.json --profile web_ui_debug --out-dir dist/hex_min/web_ui_debug --clean --json
x07-wasm web-ui test --dist-dir dist/hex_min/web_ui_debug --case tests/web_ui/turn_flow.trace.json --json
bash scripts/ci/check_hex_min.sh
```

Use `--update-golden` after changing the reducer shape and keep the env sequence stable so native-effect replay remains explicit.

Current proving traces:

- `tests/web_ui/turn_flow.trace.json`: select, move, and end-turn with audio/haptics success
- `tests/web_ui/clipboard_success.trace.json`: clipboard copy of the current tactics status
- `tests/web_ui/share_export_success.trace.json`: share followed by export success from the same victory state
- `tests/web_ui/negative.trace.json`: invalid move plus deterministic desktop-style haptics unsupported handling

Native incident fixtures live under `tests/native_incidents/`:

- this example does not ship native incident replay fixtures; use the directory to document packaged-host notes that stay specific to the tactics line
