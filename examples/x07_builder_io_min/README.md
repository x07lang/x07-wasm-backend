# `x07 Builder IO Min`

Reference builder app for the Forge M0 import/edit/export/share surface in `x07-wasm-backend`.

This example keeps the reducer intentionally small while proving the same `std.web_ui` program across browser replay, desktop bundle smoke, and iOS/Android project generation. It is the canonical reference for:

- multi-file import through reducer-visible file result paths
- draft editing through reducer-safe input events
- clipboard copy/read flows through `state.__x07_device.clipboard.result`
- file save/export through `state.__x07_device.files.result`
- share flows through `state.__x07_device.share.result`
- target-specific device capabilities and telemetry sidecars

- CI gate: `bash scripts/ci/check_builder_io_min.sh`
- Alias gate: `bash scripts/ci/check_m0_native_surface.sh`
- Trace notes: [`tests/README.md`](tests/README.md)
- Native packaging notes: [`tests/native_incidents/README.md`](tests/native_incidents/README.md)

## What It Demonstrates

- `std-web-ui@0.2.6` builder-I/O helpers from the vendored package path
- One reducer packaged for desktop, iOS, and Android
- Deterministic replay for import, edit, clipboard, export, share, and negative host outcomes
- Device capability sidecars that match the Forge M0 clipboard/files/share host contract

## Run With Released Tools

Install the required components once:

```sh
x07up component add wasm
x07up component add device-host
```

Run the proving gate:

```sh
bash scripts/ci/check_builder_io_min.sh
```

Compile the generated Android project:

```sh
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
./dist/builder_io_min/device_android_dev_package/android_project/gradlew \
  -p ./dist/builder_io_min/device_android_dev_package/android_project \
  assembleDebug
```

## Run From The Workspace

```sh
PATH="<workspace>/x07/target/debug:<workspace>/x07-wasm-backend/target/debug:<workspace>/x07-device-host/target/debug:$PATH" \
  X07_DEVICE_HOST_DESKTOP="<workspace>/x07-device-host/target/debug/x07-device-host-desktop" \
  bash scripts/ci/check_builder_io_min.sh
```

## Files To Start With

- Reducer source: [`frontend/src/app.x07.json`](frontend/src/app.x07.json)
- Device index: [`arch/device/index.x07device.json`](arch/device/index.x07device.json)
- Desktop profile: [`arch/device/profiles/device_desktop_dev.json`](arch/device/profiles/device_desktop_dev.json)
- iOS profile: [`arch/device/profiles/device_ios_dev.json`](arch/device/profiles/device_ios_dev.json)
- Android profile: [`arch/device/profiles/device_android_dev.json`](arch/device/profiles/device_android_dev.json)
- Import/export replay: [`tests/web_ui/m0_import_export.trace.json`](tests/web_ui/m0_import_export.trace.json)
- Clipboard replay: [`tests/web_ui/m0_clipboard_roundtrip.trace.json`](tests/web_ui/m0_clipboard_roundtrip.trace.json)
- Share replay: [`tests/web_ui/m0_share_success.trace.json`](tests/web_ui/m0_share_success.trace.json)
- Negative replay: [`tests/web_ui/m0_negative.trace.json`](tests/web_ui/m0_negative.trace.json)
