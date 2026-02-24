# solve_pure_echo

A minimal `solve-pure` + freestanding ABI smoke test.

- Project world is `solve-pure` (pure compute only).
- Entry module returns `view.to_bytes(input)` to produce an exact byte-for-byte echo.

This is used as a golden CI target for freestanding/wasm pipelines.

## Run the freestanding smoke test

```bash
bash examples/solve_pure_echo/ci/freestanding_smoke.sh
```

It builds freestanding C via `x07 build --freestanding` (exports `x07_solve_v2`)
and runs a small C harness over `tests/fixtures/*`.

