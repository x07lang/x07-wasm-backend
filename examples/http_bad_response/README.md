# http_bad_response

Phase 4 native `wasi:http/proxy` negative fixture.

This program returns invalid JSON bytes so the native HTTP glue must surface:

- `X07WASM_NATIVE_HTTP_RESPONSE_ENVELOPE_PARSE_FAILED`
