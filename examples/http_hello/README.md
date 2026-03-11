# http_hello

Minimal `wasi:http/proxy` example.

- World: `solve-pure`
- Contract: input is `x07.http.request.envelope@0.1.0` JSON bytes; output is `x07.http.response.envelope@0.1.0` JSON bytes.
- Behavior: ignores the request and returns a fixed `200` response body.
