`x07 Capture Min` carries the strict-M1 native incident fixtures used by the device regression generator.

Each case directory contains:

- `incident.bundle.json`: the platform incident artifact
- `incident.meta.local.json`: local incident metadata with target/classification linkage
- `regression.request.json`: the regression request written by `x07-platform`
- `expected/*.native.replay.json`: the deterministic replay fixture that `x07-wasm device regress from-incident` must synthesize

Covered classes:

- `permission_blocked`
- `location_timeout`
- `policy_violation`
- `host_webview_crash`
