# http_hello

Minimal Phase 1 `wasi:http/proxy` example.

- World: `solve-pure`
- Contract: input is `x07.http.request` envelope bytes; output is `x07.http.response` envelope bytes.
- Behavior: ignores the request and returns a fixed `200` response body.

