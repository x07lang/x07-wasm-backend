# http_echo

Minimal `wasi:http/proxy` echo example.

- World: `solve-pure`
- Contract: input is `x07.http.request.envelope@0.1.0` JSON bytes; output is `x07.http.response.envelope@0.1.0` JSON bytes.
- Behavior: echoes the request body back in the response body.
