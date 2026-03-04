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
- `--report-out` is honored even on Clap argument parsing errors (`x07.wasm.cli.parse.report@0.1.0`).

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

Capabilities are deny-by-default allowlists in `x07.app.capabilities@0.2.0`:

- filesystem preopens (ro/rw)
- environment variable allowlist
- secret IDs allowlist
- network allowlist (or deny)
- clocks/random modes
- network hardening (deny IP literals/private IPs by default; allow overrides via CIDRs)

Validate a capabilities profile:

```sh
x07-wasm caps validate --profile arch/app/ops/caps_release.json --json
```

Runtime enforcement:

- `x07-wasm serve --ops <ops.json>` applies capabilities to WASI 0.2 HTTP components (WASI Preview 2 host config + outgoing `wasi:http` allowlist).
- `x07-wasm http serve --ops <ops.json>` applies capabilities to the core-wasm HTTP reducer effects (for example `http.fetch` and `time.now`).
- `x07-wasm app serve --ops <ops.json>` applies capabilities to backend requests served via the in-proc component host.

Clocks/random record+replay:

- If `clocks.mode=record` or `random.mode=record`, `x07-wasm serve` requires either:
  - `--evidence-out <path>` (record), or
  - `--evidence-in <path>` (replay).
- Evidence schema: `x07.wasm.caps.evidence@0.1.0` (`https://x07.io/spec/x07-wasm.caps.evidence.schema.json`).

Secrets provider (v1):

- Allow secret IDs via `caps.secrets.allow[]`.
- Values are injected as environment variables `X07_SECRET_<ID>` (uppercase, non-alnum becomes `_`).
- Sources:
  - file: `.x07/secrets/<id>` (default; base dir overridable via `X07_SECRETS_DIR`), or
  - env: `X07_SECRET_<ID>`.
- Evidence records `{id, source}` only (never secret bytes).

## Policy cards

Policy cards (`x07.policy.card@0.1.0`) support:

- allow/deny/warn/require via assertions
- optional RFC 6902 JSON Patch mutation when assertions fail

Evaluation semantics (Phase 6):

- Rules are evaluated in file order (cards list order, then rules list order).
- For a matching `target`, assertions are evaluated against the current JSON doc.
- If assertions fail and `patches[]` is non-empty, the patches are applied (in order) and assertions are re-evaluated.
- If assertions still fail, the rule emits a diagnostic according to `effect` (`deny|warn|require`).

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

Generate a deploy plan (optionally emit Kubernetes YAMLs; Argo Rollouts concepts):

```sh
x07-wasm deploy plan --pack-manifest dist/app.pack.json --ops arch/app/ops/ops_release.json --out-dir dist/deploy_plan --json
```

Plan-only mode (no Kubernetes YAML outputs):

```sh
x07-wasm deploy plan --pack-manifest dist/app.pack.json --ops arch/app/ops/ops_release.json --emit-k8s false --out-dir dist/deploy_plan --json
```

## Provenance

Create and verify a signed provenance attestation for a pack (DSSE + Ed25519):

```sh
x07-wasm provenance attest --pack-manifest dist/app.pack.json --ops arch/app/ops/ops_release.json --signing-key arch/provenance/dev.ed25519.signing_key.b64 --out dist/provenance.dsse.json --json
x07-wasm provenance verify --attestation dist/provenance.dsse.json --pack-dir dist --trusted-public-key arch/provenance/dev.ed25519.public_key.b64 --json
```

Notes:

- `x07-wasm provenance attest` includes `predicate.x07.compatibility_hash` (matches `x07-wasm ops validate`).
- Attestations are emitted as `x07.provenance.dsse.envelope@0.1.0` (`https://x07.io/spec/x07-provenance.dsse.envelope.schema.json`) where the payload is an in-toto Statement.
- `predicateType` is schema-validated as a non-empty string; `x07-wasm provenance verify` enforces the supported SLSA v1 predicate type after signature verification.
- `x07-wasm provenance attest` fails closed (no DSSE envelope is written) when any subject path is unsafe and emits `X07WASM_PROVENANCE_SUBJECT_PATH_UNSAFE` (exit code 1).

## Platform handoff (Phase 6)

Phase 6 outputs are intended to be consumed by an autonomous deployer (for example `x07-platform`) as a closed-loop contract:

- **Deploy intent**: `x07-wasm deploy plan` emits `deploy.plan.json`. By default it also emits Kubernetes YAML outputs under `--out-dir` (disable via `--emit-k8s false`).
- **Authorization**: `x07-wasm provenance verify` recomputes digests against the pack directory; platforms can gate deployment on a verified attestation and record `predicate.x07.compatibility_hash`.
- **Promotion**: `x07-wasm app serve --mode canary --ops <ops.json>` evaluates SLOs (if referenced by ops) and emits a pinned `promote|rollback|inconclusive` decision.
- **Incidents → regressions**: incident bundles under `.x07-wasm/incidents/...` can be converted into replayable cases via `* regress-from-incident` commands.

## CI gate

Run the Phase 6 gate locally:

```sh
# Only required for legacy C toolchain builds.
export PATH="${WASI_SDK_DIR}/bin:${PATH}"
bash scripts/ci/check_phase6.sh
```
