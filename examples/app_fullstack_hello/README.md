# app_fullstack_hello

Tiny full-stack app bundle example (web-ui frontend + `wasi:http/proxy` backend).

## Commands

- Build: `x07-wasm app build --profile app_dev --clean`
- Serve (canary): `x07-wasm app serve --mode canary --strict-mime`
- Test (trace replay): `x07-wasm app test --trace examples/app_fullstack_hello/tests/trace_0001.json`
