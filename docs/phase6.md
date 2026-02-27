# WASM Phase 6 (Operational Contracts + Policy + SLO + Deploy Plans + Provenance)

Phase 6 builds on Phase 5 by adding machine-readable operational contracts and outputs that are safe to consume by autonomous deployers:

- Ops profiles (`x07-wasm ops validate`)
- Capability contracts (`x07-wasm caps validate`)
- Policy cards (`x07-wasm policy validate`)
- SLO-as-code + offline evaluation (`x07-wasm slo validate`, `x07-wasm slo eval`)
- Progressive delivery plan generation (`x07-wasm deploy plan`)
- Pack provenance attest/verify (`x07-wasm provenance attest`, `x07-wasm provenance verify`)

All Phase 6 commands support:

- `--json` / `--report-out` (machine report emission)
- `--json-schema` / `--json-schema-id` (report contract discovery)

## Operational contracts

Phase 6 adds an ops profile registry under:

- `arch/app/ops/index.x07ops.json`

and example ops profiles:

- `arch/app/ops/ops_dev.json`
- `arch/app/ops/ops_release.json`

Validate an ops profile:

```sh
x07-wasm ops validate --profile arch/app/ops/ops_release.json --json
```

## Capabilities

Capabilities are deny-by-default allowlists in `x07.app.capabilities@0.1.0`:

- filesystem preopens (ro/rw)
- environment variable allowlist
- secret IDs allowlist
- network allowlist (or deny)
- clocks/random modes

Validate a capabilities profile:

```sh
x07-wasm caps validate --profile arch/app/ops/caps_release.json --json
```

Runtime enforcement:

- `x07-wasm serve --ops <ops.json>` applies capabilities to WASI 0.2 HTTP components (WASI Preview 2 host config + outgoing `wasi:http` allowlist).
- `x07-wasm http serve --ops <ops.json>` applies capabilities to the core-wasm HTTP reducer effects (for example `http.fetch` and `time.now`).
- `x07-wasm app serve --ops <ops.json>` applies capabilities to backend requests served via the in-proc component host.

## Policy cards

Policy cards (`x07.policy.card@0.1.0`) support:

- allow/deny/warn/require via assertions
- optional RFC 6902 JSON Patch mutation when assertions fail

Validate one or more cards:

```sh
x07-wasm policy validate --card path/to/card.json --json
```

## SLO evaluation

SLO profiles (`x07.slo.profile@0.1.0`) can be evaluated against offline metrics snapshots (`x07.metrics.snapshot@0.1.0`):

```sh
x07-wasm slo eval --profile arch/slo/slo_min.json --metrics examples/app_min/tests/metrics_canary_ok.json --json
```

Canary integration:

- `x07-wasm app serve --mode canary --ops <ops.json>` evaluates the referenced SLO profile (if present) and includes the decision under `result.stdout_json.canary.slo_decision`.

Pinned exit codes:

- `promote` → exit 0
- `rollback` (violations) → exit 2
- `inconclusive` (missing metrics) → exit 3

## Deploy plan generation

Generate a deploy plan and emit Kubernetes YAMLs (Argo Rollouts concepts):

```sh
x07-wasm deploy plan --pack-manifest dist/app.pack.json --ops arch/app/ops/ops_release.json --out-dir dist/deploy_plan --json
```

## Provenance

Create and verify a hash-first provenance attestation for a pack:

```sh
x07-wasm provenance attest --pack-manifest dist/app.pack.json --ops arch/app/ops/ops_release.json --out dist/provenance.slsa.json --json
x07-wasm provenance verify --attestation dist/provenance.slsa.json --pack-dir dist --json
```

## CI gate

Run the Phase 6 gate locally:

```sh
export PATH="${WASI_SDK_DIR}/bin:${PATH}"
bash scripts/ci/check_phase6.sh
```
