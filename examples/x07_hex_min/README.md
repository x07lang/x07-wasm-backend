# `x07 Hex Min`

Reference tactics app for the minimal select/move/end-turn lane in `x07-wasm-backend`.

This example keeps the reducer intentionally narrow while proving the same `std.web_ui` program across browser replay, desktop bundle smoke, and iOS/Android project generation. It is the canonical reference for:

- a deterministic three-action tactics lane on a tiny hex board
- audio cues through `state.__x07_device.audio.result`
- haptics through `state.__x07_device.haptics.result`
- clipboard/share/export reuse through the existing device result paths
- target-specific device capabilities, telemetry sidecars, and Android haptics permission projection

- CI gate: `bash scripts/ci/check_hex_min.sh`
- Alias gate: `bash scripts/ci/check_native_surface.sh`
- Trace notes: [`tests/README.md`](tests/README.md)
- Native packaging notes: [`tests/native_incidents/README.md`](tests/native_incidents/README.md)

## What It Demonstrates

- `std-web-ui@0.2.6` audio/haptics helpers from the vendored package path
- One reducer packaged for desktop, iOS, and Android
- Deterministic replay for select/move/victory, clipboard/share/export, and unsupported haptics outcomes
- Device capability sidecars that match the tactics audio/haptics/clipboard/files/share host contract

## Run With Released Tools

Install the required components once:

```sh
x07up component add wasm
x07up component add device-host
```

Run the proving gate:

```sh
bash scripts/ci/check_hex_min.sh
```

Compile the generated Android project:

```sh
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
./dist/hex_min/device_android_dev_package/android_project/gradlew \
  -p ./dist/hex_min/device_android_dev_package/android_project \
  assembleDebug
```

## Run From The Workspace

```sh
PATH="<workspace>/x07/target/debug:<workspace>/x07-wasm-backend/target/debug:<workspace>/x07-device-host/target/debug:$PATH" \
  X07_DEVICE_HOST_DESKTOP="<workspace>/x07-device-host/target/debug/x07-device-host-desktop" \
  bash scripts/ci/check_hex_min.sh
```

## Files To Start With

- Reducer source: [`frontend/src/app.x07.json`](frontend/src/app.x07.json)
- Device index: [`arch/device/index.x07device.json`](arch/device/index.x07device.json)
- Desktop profile: [`arch/device/profiles/device_desktop_dev.json`](arch/device/profiles/device_desktop_dev.json)
- iOS profile: [`arch/device/profiles/device_ios_dev.json`](arch/device/profiles/device_ios_dev.json)
- Android profile: [`arch/device/profiles/device_android_dev.json`](arch/device/profiles/device_android_dev.json)
- Turn-flow replay: [`tests/web_ui/turn_flow.trace.json`](tests/web_ui/turn_flow.trace.json)
- Clipboard replay: [`tests/web_ui/clipboard_success.trace.json`](tests/web_ui/clipboard_success.trace.json)
- Share/export replay: [`tests/web_ui/share_export_success.trace.json`](tests/web_ui/share_export_success.trace.json)
- Negative replay: [`tests/web_ui/negative.trace.json`](tests/web_ui/negative.trace.json)
