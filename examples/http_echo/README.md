# http_echo

Minimal Phase 1 `wasi:http/proxy` echo example.

- World: `solve-pure`
- Contract: input is `x07.http.request` envelope bytes; output is `x07.http.response` envelope bytes.
- Behavior: echoes the request body back in the response body.

