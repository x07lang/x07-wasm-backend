`x07 Capture Min` keeps its behavioral coverage in deterministic reducer-call traces under `tests/web_ui/`.

Recommended local loop:

```sh
x07 check --project frontend/x07.json
x07-wasm web-ui build --project frontend/x07.json --profile web_ui_debug --out-dir dist/m0_capture/web_ui_debug --clean --json
x07-wasm web-ui test --dist-dir dist/m0_capture/web_ui_debug --case tests/web_ui/m0_success.trace.json --json
bash scripts/ci/check_capture_min.sh
```

Use `--update-golden` after changing the reducer shape and keep the env sequence stable so native-effect replay remains explicit.
