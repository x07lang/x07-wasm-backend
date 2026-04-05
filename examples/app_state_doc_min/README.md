# app_state_doc_min

Minimal app example for `wasi_http_proxy_state_doc_v1`.

- Frontend emits the same GET `/ping` request on each `init` step.
- Backend uses the host-retained state document to return `count:1` on the first request and `count:2` on the second request in the same `app test` session.
- Golden trace in `tests/trace_0001.json` proves state survives across requests without frontend storage.

Commands:

Build app bundle:
  x07-wasm app build \
    --profile-file examples/app_state_doc_min/app_release.json \
    --out-dir dist/app_state_doc_min \
    --clean

Test via trace replay:
  x07-wasm app test \
    --dir dist/app_state_doc_min \
    --trace examples/app_state_doc_min/tests/trace_0001.json
