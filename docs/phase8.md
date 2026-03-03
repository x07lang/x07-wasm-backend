# WASM Phase 8 (Device bundles: system WebView host + pinned host ABI)

Phase 8 introduces a device contract layer for running `std.web_ui` reducers in a system WebView host (desktop + mobile).

The device bundle format pins the host ABI hash from `x07-device-host` so that a device app can reject incompatible hosts deterministically.

## Contracts-as-data

- Device profile registry: `arch/device/index.x07device.json`
- Device profiles: `arch/device/profiles/*.json`
- Bundle manifest: `bundle.manifest.json` (`x07.device.bundle.manifest@0.1.0`)

## CLI

Validate contracts:

```sh
x07-wasm device index validate --json
x07-wasm device profile validate --json
```

Build a device bundle (web-ui reducer wasm + pinned host ABI):

```sh
x07-wasm device build --profile device_dev --out-dir dist/device --clean --json
```

Bundle layout notes:

- The resolved device profile is embedded into the bundle under `profile/device.profile.json` and is digest-verified by `x07-wasm device verify`.

Verify a device bundle:

```sh
x07-wasm device verify --dir dist/device --json
```

Create and verify a signed provenance attestation for a device bundle:

```sh
x07-wasm device provenance attest --dir dist/device --signing-key arch/provenance/dev.ed25519.signing_key.b64 --out dist/device.provenance.dsse.json --json
x07-wasm device provenance verify --attestation dist/device.provenance.dsse.json --bundle-dir dist/device --trusted-public-key arch/provenance/dev.ed25519.public_key.b64 --json
```

## CI gate

```sh
bash scripts/ci/check_phase8.sh
```
