`x07 Builder IO Min` keeps its behavioral coverage in deterministic reducer-call traces under `tests/web_ui/`.

Recommended local loop:

```sh
x07 check --project frontend/x07.json
x07-wasm web-ui build --project frontend/x07.json --profile web_ui_debug --out-dir dist/builder_io_min/web_ui_debug --clean --json
x07-wasm web-ui test --dist-dir dist/builder_io_min/web_ui_debug --case tests/web_ui/m0_import_export.trace.json --json
bash scripts/ci/check_builder_io_min.sh
```

Use `--update-golden` after changing the reducer shape and keep the env sequence stable so native-effect replay remains explicit.

Current proving traces:

- `tests/web_ui/m0_import_export.trace.json`: input edits, multi-file import, and save/export success
- `tests/web_ui/m0_clipboard_roundtrip.trace.json`: copy + read-text clipboard roundtrip
- `tests/web_ui/m0_share_success.trace.json`: share request/result success
- `tests/web_ui/m0_negative.trace.json`: cancelled import plus unsupported clipboard/share outcomes

Native incident fixtures live under `tests/native_incidents/`:

- this example does not ship native incident replay fixtures; use the directory to document packaged-host notes that stay specific to the builder-I/O line
