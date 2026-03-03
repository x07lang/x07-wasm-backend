# WASM Phase 10 (iOS/Android project generation)

Phase 10 extends `x07-wasm device package` with store-safe mobile project generation:

- `--target ios`: generate an Xcode project directory with the device bundle embedded.
- `--target android`: generate a Gradle project directory with the device bundle embedded.

This phase does not require Xcode/Gradle in CI; it only generates projects.

## CLI

Generate an iOS project:

```sh
x07-wasm device package --bundle dist/device --target ios --out-dir dist/device_package_ios --json
```

Generate an Android project:

```sh
x07-wasm device package --bundle dist/device --target android --out-dir dist/device_package_android --json
```

## CI gate

```sh
bash scripts/ci/check_phase10.sh
```

