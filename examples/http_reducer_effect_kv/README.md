# http_reducer_effect_kv

Solve-pure HTTP reducer example that emits deterministic KV effects.

- Matches `spec/fixtures/http/trace.effect_kv.min.json`.

```sh
x07-wasm build --project examples/http_reducer_effect_kv/x07.json --profile wasm_release --out dist/http_reducer_effect_kv.wasm --json
x07-wasm http test --component dist/http_reducer_effect_kv.wasm --trace spec/fixtures/http/trace.effect_kv.min.json --json
```

