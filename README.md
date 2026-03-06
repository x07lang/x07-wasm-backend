# x07-wasm-backend

WASM build pipeline, host runners, and full-stack app tooling for [X07](https://github.com/x07lang/x07) — covering Phases 0–10 of the [WASM roadmap](https://github.com/x07lang/x07/blob/main/dev-docs/phases/x07-wasm-plan.md).

x07-wasm-backend is designed for **100% agentic coding** — an AI coding agent builds, tests, packages, deploys, and verifies WASM artifacts entirely on its own using structured contracts, deterministic runners, and machine-readable outputs. No human needs to write X07 by hand.

## Prerequisites

The [X07 toolchain](https://github.com/x07lang/x07) must be installed before using x07-wasm-backend. If you (or your agent) are new to X07, start with the **[Agent Quickstart](https://x07lang.org/docs/getting-started/agent-quickstart)** — it covers toolchain setup, project structure, and the workflow conventions an agent needs to be productive.

Rust is pinned via `rust-toolchain.toml` for deterministic outputs (including embedded WASM adapter snapshots). Run `cargo`/CI gate scripts from this repo root so `rustup` applies the pin.

## Install

```sh
x07up component add wasm
x07 wasm doctor --json
```

Fallbacks:

```sh
cargo install --locked x07-wasm --version 0.1.2
```

Use `cargo install --locked --git https://github.com/x07lang/x07-wasm-backend.git x07-wasm` only when you need unreleased development state from this repo.

## Quickstart

```sh
x07-wasm doctor --json
x07-wasm profile validate --json

x07-wasm build \
  --project examples/solve_pure_echo/x07.json \
  --profile wasm_release \
  --out dist/echo.wasm \
  --artifact-out dist/echo.wasm.manifest.json \
  --json

x07-wasm run \
  --wasm dist/echo.wasm \
  --input examples/solve_pure_echo/tests/fixtures/in_hello.bin \
  --output-out dist/out.bin \
  --json
```

## Command surface

### Phase 0 — solve-pure WASM modules

- `x07-wasm build` — build solve-pure wasm modules (Phase 7 defaults to native `x07 build --emit-wasm`; legacy `clang`/`wasm-ld` path available via `--codegen-backend c_toolchain_v1`)
- `x07-wasm run` — deterministic runner for Phase 0 ABI (`x07_solve_v2` via WASM Basic C ABI sret)
- `x07-wasm doctor`, `x07-wasm profile validate`, `x07-wasm cli specrows check`

### Phase 1 — WASI 0.2 components

- `x07-wasm wit validate`
- `x07-wasm component profile validate` / `build` / `compose` / `targets`
- `x07-wasm serve` / `x07-wasm component run`

### Phase 2 — web UI

- `x07-wasm web-ui contracts validate` / `profile validate`
- `x07-wasm web-ui build` / `serve` / `test` / `regress-from-incident`

### Phase 3 — full-stack app bundle

- `x07-wasm app contracts validate` / `profile validate`
- `x07-wasm app build` / `serve` / `test` / `regress from-incident`

### Phase 4 — native backend targets

- `x07-wasm component build --emit http-native` / `--emit cli-native`

### Phase 5 — production hardening

- `x07-wasm toolchain validate`
- `x07-wasm app pack` / `app verify`
- `x07-wasm http contracts validate` / `http serve` / `http test` / `http regress from-incident`

### Phase 6 — ops / policy / SLO / deploy / provenance

- `x07-wasm ops validate` / `caps validate` / `policy validate`
- `x07-wasm slo validate` / `slo eval`
- `x07-wasm deploy plan`
- `x07-wasm provenance attest` / `provenance verify`

### Phase 7 — native x07 → wasm backend

- Wasm profiles add `codegen_backend` (default: `native_x07_wasm_v1`)
- `--codegen-backend native_x07_wasm_v1` (native) or `--codegen-backend c_toolchain_v1` (legacy)

### Phase 8–10 — device apps

- `x07-wasm device index validate` / `device profile validate`
- `x07-wasm device build` / `device verify`
- `x07-wasm device run` / `device package`
- `x07-wasm device package --target ios` / `--target android`

## Contracts-as-data

- WASM profile registry: `arch/wasm/index.x07wasm.json`
- Profiles: `arch/wasm/profiles/*.json`
- WIT registry: `arch/wit/index.x07wit.json`
- Component profile registry: `arch/wasm/component/index.x07wasm.component.json`
- Web UI profile registry: `arch/web_ui/index.x07webui.json`
- App profile registry: `arch/app/index.x07app.json`
- Device profile registry: `arch/device/index.x07device.json`
- Schemas (published to `https://x07.io/spec/`): `crates/x07-wasm/spec/schemas/*.schema.json`

## CI gates

| Phase | Gate |
|-------|------|
| 0 | `scripts/ci/check_phase0.sh` |
| 1 | `scripts/ci/check_phase1.sh` |
| 2 | `scripts/ci/check_phase2.sh` |
| 3 | `scripts/ci/check_phase3.sh` |
| 4 | `scripts/ci/check_phase4.sh` |
| 5 | `scripts/ci/check_phase5.sh` |
| 6 | `scripts/ci/check_phase6.sh` |
| 7 | `scripts/ci/check_phase7.sh` |
| 8 | `scripts/ci/check_phase8.sh` |
| 9 | `scripts/ci/check_phase9.sh` |
| 10 | `scripts/ci/check_phase10.sh` |

Example freestanding smoke: `examples/solve_pure_echo/ci/freestanding_smoke.sh`

## Avoiding CI reruns (pre-push checklist)

The CI workflow runs `cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets -- -D warnings` on every push. Run these locally before pushing (especially to `main`) to avoid “fix-and-push-again” loops:

```sh
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Then run the phase gate(s) that match what you changed:
- Phase 0–1: WASM / components toolchain changes.
- Phase 2–3: web-ui and app pipeline changes.
- Phase 4–7: native backend / hardening / ops / provenance changes.
- Phase 8–10: device pipeline / templates / host ABI changes.

If CI fails in a phase gate, run the corresponding `scripts/ci/check_phase*.sh` locally; they are the same entry points CI uses.

Some phase gates (notably Phase 1–2) also validate that embedded adapter snapshots under `crates/x07-wasm/src/support/adapters/` match what `guest/*` builds produce. If you change `guest/*` or bump `rust-toolchain.toml`, refresh the snapshots:

```sh
bash scripts/update_adapter_snapshots.sh
```

Notes:
- Adapter WASM bytes are not stable across build environments. To keep CI deterministic, the snapshot drift check builds adapters inside a pinned `rust:<channel>` container and runs only on Linux (Ubuntu CI, Docker required).
- `scripts/update_adapter_snapshots.sh` requires Docker; it builds the guest adapters in a linux/amd64 container using the pinned `rust-toolchain.toml` channel, then copies the outputs into `crates/x07-wasm/src/support/adapters/*.component.wasm`.

## Phase docs

- `docs/phase0.md` through `docs/phase10.md`

## Incidents

On `x07-wasm run` failures, a deterministic incident bundle is written under `.x07-wasm/incidents/<YYYY-MM-DD>/<run_id>/` containing `input.bin`, `run.report.json`, `wasm.manifest.json`, and `stderr.txt`.

## Links

- [X07 Agent Quickstart](https://x07lang.org/docs/getting-started/agent-quickstart) — start here
- [X07 toolchain](https://github.com/x07lang/x07)
- [X07 website](https://x07lang.org)

## License

Dual-licensed under [Apache 2.0](LICENSE-APACHE) and [MIT](LICENSE).
