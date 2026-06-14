# Operational contracts, capabilities, policy

This guide covers machine-readable operational contracts and outputs that are safe to consume by autonomous deployers:

- Ops profiles (`x07-wasm ops validate`)
- Capability contracts (`x07-wasm caps validate`)
- Policy cards (`x07-wasm policy validate`)

All commands in this surface support:

- `--json` / `--report-out` (machine report emission)
- `--json-schema` / `--json-schema-id` (report contract discovery)
- `--report-out` is honored even on Clap argument parsing errors (`x07.wasm.cli.parse.report@0.1.0`).

## Operational contracts

This repo includes an ops profile registry under:

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

Evaluation semantics:

- Rules are evaluated in file order (cards list order, then rules list order).
- For a matching `target`, assertions are evaluated against the current JSON doc.
- If assertions fail and `patches[]` is non-empty, the patches are applied (in order) and assertions are re-evaluated.
- If assertions still fail, the rule emits a diagnostic according to `effect` (`deny|warn|require`).

Validate one or more cards:

```sh
x07-wasm policy validate --card path/to/card.json --json
```

## CI coverage

CI validates these contracts and reports in the presence of pinned toolchains and deterministic fixtures, including the legacy C toolchain path when `WASI_SDK_DIR` is configured.
