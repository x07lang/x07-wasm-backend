# WASM iOS/Android project generation

This surface extends `x07-wasm device package` with store-safe mobile project generation:

- `--target ios`: generate an Xcode project directory with the device bundle embedded.
- `--target android`: generate a Gradle project directory with the device bundle embedded.

This does not require Xcode/Gradle in CI; it only generates projects.

The generated project skeletons come from the vendored `x07-device-host` mobile templates, including the native OTLP telemetry bridge used by device release observability.

The examples and CI cover this surface in two directions:

- [`examples/x07_builder_io_min`](../examples/x07_builder_io_min/README.md) is the reference reducer for import/edit/export, clipboard, and share flows backed by the `std-web-ui@0.2.6` builder-I/O helpers.
- [`examples/x07_hex_min`](../examples/x07_hex_min/README.md) is the Tactics reference reducer for deterministic select/move/end-turn flow backed by the `std-web-ui@0.2.6` audio/haptics helpers.
- when a sibling `../x07-mcp` checkout is available, CI also drives the repo-local `x07lang-mcp` reference server through `x07-mcp inspect --command` to verify the machine-readable inspect surface end to end

The machine-readable device reports cover:

- `x07-wasm device verify --json` emits `x07.wasm.device.verify.report@0.2.0` with `result.native_summary` and `result.release_readiness`
- `x07-wasm device package --json` emits `x07.wasm.device.package.report@0.2.0` with the same normalized summary plus the sealed `package.manifest.json` digest
- `x07-wasm device regress from-incident` emits `x07.wasm.device.regress.from_incident.report@0.2.0` and consumes platform incident directories (`incident.bundle.json`, optional incident meta files, and `regression.request.json`) to synthesize deterministic native replay fixtures

## CLI

Generate an iOS project:

```sh
x07-wasm device package --bundle dist/device --target ios --out-dir dist/device_package_ios --json
```

Generate an Android project:

```sh
x07-wasm device package --bundle dist/device --target android --out-dir dist/device_package_android --json
```

Generated Android projects include a pinned Gradle wrapper. Compile them with `./gradlew` on a supported JDK (17 or 21).

Generate a deterministic regression fixture set from a captured device incident:

```sh
x07-wasm device regress from-incident .x07-wasm/incidents/device/<YYYY-MM-DD>/<id> --out-dir tests/regress --name device_incident --json
```

When consuming a platform incident directory containing `incident.bundle.json` and `regression.request.json`, the generated replay fixture is written as `<name>.native.replay.json`.

## CI gate

```sh
bash scripts/ci/check_release_ready.sh
```

Proving references for this surface are [`examples/x07_builder_io_min`](../examples/x07_builder_io_min/README.md) for builder I/O, [`examples/x07_capture_min`](../examples/x07_capture_min/README.md) for the broader native surface, and [`examples/x07_hex_min`](../examples/x07_hex_min/README.md) for the Tactics audio/haptics line. [`examples/x07_field_notes`](../examples/x07_field_notes/README.md) remains the richer offline/mobile showcase.
