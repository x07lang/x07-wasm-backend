# http_reducer_echo

Solve-pure HTTP reducer example for the core-wasm HTTP reducer loop.

- Matches `spec/fixtures/http/trace.min.json`.

```sh
x07-wasm build --project examples/http_reducer_echo/x07.json --profile wasm_release --out dist/http_reducer_echo.wasm --json
x07-wasm http test --component dist/http_reducer_echo.wasm --trace spec/fixtures/http/trace.min.json --json
```
