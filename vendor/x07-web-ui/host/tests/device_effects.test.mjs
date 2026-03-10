import assert from "node:assert/strict";
import test from "node:test";

import { __x07_host_private as host } from "../app-host.mjs";

test("parseAnyDeviceEffect() parses permissions and camera requests", () => {
  const permissions = host.parseAnyDeviceEffect({
    v: 1,
    kind: "x07.web_ui.effect.device.permissions.query",
    request_id: "req1",
    op: "permissions.query",
    capability: "device.permissions",
    payload: { permission: "camera" },
  });
  assert.deepEqual(permissions, {
    family: "permissions",
    kind: "x07.web_ui.effect.device.permissions.query",
    request_id: "req1",
    op: "permissions.query",
    capability: "camera.photo",
    payload: { permission: "camera" },
  });

  const camera = host.parseAnyDeviceEffect({
    v: 1,
    kind: "x07.web_ui.effect.device.camera.capture",
    request_id: "req2",
    op: "camera.capture",
    capability: "camera.photo",
    payload: { quality: "medium" },
  });
  assert.deepEqual(camera, {
    family: "camera",
    kind: "x07.web_ui.effect.device.camera.capture",
    request_id: "req2",
    op: "camera.capture",
    capability: "camera.photo",
    payload: { quality: "medium" },
  });
});

test("capabilityAllowed() follows the M0 device capability sidecar", () => {
  const capabilities = {
    device: {
      camera: { photo: true },
      files: { pick: true },
      blob_store: { enabled: true },
      location: { foreground: false },
      notifications: { local: true },
    },
  };

  assert.equal(host.capabilityAllowed(capabilities, "camera.photo"), true);
  assert.equal(host.capabilityAllowed(capabilities, "files.pick"), true);
  assert.equal(host.capabilityAllowed(capabilities, "blob_store"), true);
  assert.equal(host.capabilityAllowed(capabilities, "location.foreground"), false);
  assert.equal(host.capabilityAllowed(capabilities, "notifications.local"), true);
});

test("normalizeDeviceResult() normalizes host metadata and status", () => {
  const request = {
    family: "location",
    request_id: "req5",
    op: "location.get_current",
    capability: "location.foreground",
  };
  const normalized = host.normalizeDeviceResult(
    request,
    {
      status: "ok",
      payload: { latitude: 1, longitude: 2, accuracy_m: 3 },
      host_meta: { provider: "browser" },
    },
    "location",
    "web",
  );
  assert.equal(normalized.family, "location");
  assert.equal(normalized.result.request_id, "req5");
  assert.equal(normalized.result.status, "ok");
  assert.equal(normalized.result.host_meta.platform, "web");
  assert.equal(normalized.result.host_meta.provider, "browser");
  assert.deepEqual(normalized.result.payload, { latitude: 1, longitude: 2, accuracy_m: 3 });
});

test("normalizeDeviceHostEvent() requires an event type", () => {
  assert.throws(() => host.normalizeDeviceHostEvent({}), /missing type/);
  assert.deepEqual(host.normalizeDeviceHostEvent({ type: "connectivity.online" }), {
    type: "connectivity.online",
  });
});
