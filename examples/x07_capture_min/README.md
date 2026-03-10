# `x07 Capture Min`

Strict M0 proving app for the native device surface in `x07-wasm-backend`.

This example keeps the reducer intentionally small while proving the same `std.web_ui` program across browser replay, desktop bundle smoke, and iOS/Android project generation. It is the canonical M0 reference for:

- permission query and request
- camera capture and file import
- blob-manifest follow-up via `blobs.stat`
- deterministic blob quota failures for `blob_item_too_large` and `blob_total_too_large`
- foreground location reads
- local notification schedule, cancel, and `notification.opened`
- target-specific device capabilities and telemetry sidecars
- strict-M1 native incident bundle -> replay synthesis fixtures

- CI gate: `bash scripts/ci/check_capture_min.sh`
- Alias gate: `bash scripts/ci/check_m0_native_surface.sh`
- Trace notes: [`tests/README.md`](tests/README.md)
- Native incident fixtures: [`tests/native_incidents/README.md`](tests/native_incidents/README.md)

## What It Demonstrates

- `std-web-ui@0.2.0` device helpers from the vendored package path
- One reducer packaged for desktop, iOS, and Android
- Target-specific capability contracts instead of fake feature parity
- Deterministic replay of both positive and negative native outcomes
- Deterministic replay of blob quota and notification cancel outcomes
- Deterministic synthesis of strict-M1 native replay fixtures from platform incident bundles

## Run With Released Tools

Install the required components once:

```sh
x07up component add wasm
x07up component add device-host
```

Run the proving gate:

```sh
bash scripts/ci/check_capture_min.sh
```

Compile the generated Android project:

```sh
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
./dist/m0_capture/device_android_dev_package/android_project/gradlew \
  -p ./dist/m0_capture/device_android_dev_package/android_project \
  assembleDebug
```

## Run From The Workspace

```sh
PATH="<workspace>/x07/target/debug:<workspace>/x07-wasm-backend/target/debug:<workspace>/x07-device-host/target/debug:$PATH" \
  X07_DEVICE_HOST_DESKTOP="<workspace>/x07-device-host/target/debug/x07-device-host-desktop" \
  bash scripts/ci/check_capture_min.sh
```

## Files To Start With

- Reducer source: [`frontend/src/app.x07.json`](frontend/src/app.x07.json)
- Device index: [`arch/device/index.x07device.json`](arch/device/index.x07device.json)
- Desktop profile: [`arch/device/profiles/device_desktop_dev.json`](arch/device/profiles/device_desktop_dev.json)
- iOS profile: [`arch/device/profiles/device_ios_dev.json`](arch/device/profiles/device_ios_dev.json)
- Android profile: [`arch/device/profiles/device_android_dev.json`](arch/device/profiles/device_android_dev.json)
- Success replay: [`tests/web_ui/m0_success.trace.json`](tests/web_ui/m0_success.trace.json)
- Negative replay: [`tests/web_ui/m0_negative.trace.json`](tests/web_ui/m0_negative.trace.json)
- Blob quota replay: [`tests/web_ui/m0_blob_quota.trace.json`](tests/web_ui/m0_blob_quota.trace.json)
- Notification cancel replay: [`tests/web_ui/m0_notification_cancel.trace.json`](tests/web_ui/m0_notification_cancel.trace.json)
- Native incident fixtures: [`tests/native_incidents/`](tests/native_incidents/)
