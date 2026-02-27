# app_min

Minimal Phase-3 app bundle example (web-ui frontend + wasi:http/proxy backend).

- Frontend renders a title and a response line.
- On init, frontend emits a single GET /ping effect (id=req0).
- Backend returns a fixed JSON http.response envelope with body="pong".
- Golden trace in tests/trace_0001.json can be replayed.

Commands:

Build app bundle:
  x07-wasm app build \
    --profile-file examples/app_min/app_release.json \
    --out-dir dist/app_min \
    --clean

Serve canary:
  x07-wasm app serve \
    --dir dist/app_min \
    --mode canary \
    --strict-mime

Test via trace replay:
  x07-wasm app test \
    --dir dist/app_min \
    --trace examples/app_min/tests/trace_0001.json
