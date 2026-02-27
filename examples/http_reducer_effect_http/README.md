# http_reducer_effect_http

Solve-pure HTTP reducer example that emits deterministic `http.fetch` effects.

- Matches `spec/fixtures/http/trace.effect_http.min.json`.

```sh
x07-wasm build --project examples/http_reducer_effect_http/x07.json --profile wasm_release --out dist/http_reducer_effect_http.wasm --json
x07-wasm http test --component dist/http_reducer_effect_http.wasm --trace spec/fixtures/http/trace.effect_http.min.json --json
```

