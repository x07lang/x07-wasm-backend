import assert from "node:assert/strict";
import test from "node:test";

import { __x07_host_private as host } from "../app-host.mjs";

test("parseAnyDeviceEffect() parses the Forge M0 device request families", () => {
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

  const audio = host.parseAnyDeviceEffect({
    v: 1,
    kind: "x07.web_ui.effect.device.audio.play",
    request_id: "req2a",
    op: "audio.play",
    capability: "audio.playback",
    payload: { cue: "select", channel: "sfx", loop: false },
  });
  assert.deepEqual(audio, {
    family: "audio",
    kind: "x07.web_ui.effect.device.audio.play",
    request_id: "req2a",
    op: "audio.play",
    capability: "audio.playback",
    payload: { cue: "select", channel: "sfx", loop: false },
  });

  const haptics = host.parseAnyDeviceEffect({
    v: 1,
    kind: "x07.web_ui.effect.device.haptics.trigger",
    request_id: "req2b",
    op: "haptics.trigger",
    capability: "haptics.present",
    payload: { pattern: "impact" },
  });
  assert.deepEqual(haptics, {
    family: "haptics",
    kind: "x07.web_ui.effect.device.haptics.trigger",
    request_id: "req2b",
    op: "haptics.trigger",
    capability: "haptics.present",
    payload: { pattern: "impact" },
  });

  const clipboard = host.parseAnyDeviceEffect({
    v: 1,
    kind: "x07.web_ui.effect.device.clipboard.read_text",
    request_id: "req3",
    op: "clipboard.read_text",
    capability: "clipboard.read_text",
    payload: {},
  });
  assert.deepEqual(clipboard, {
    family: "clipboard",
    kind: "x07.web_ui.effect.device.clipboard.read_text",
    request_id: "req3",
    op: "clipboard.read_text",
    capability: "clipboard.read_text",
    payload: {},
  });

  const filesPick = host.parseAnyDeviceEffect({
    v: 1,
    kind: "x07.web_ui.effect.device.files.pick",
    request_id: "req4",
    op: "files.pick",
    capability: "files.pick",
    payload: { accept: ["image/*"], multiple: true },
  });
  assert.deepEqual(filesPick, {
    family: "files",
    kind: "x07.web_ui.effect.device.files.pick",
    request_id: "req4",
    op: "files.pick",
    capability: "files.pick_multiple",
    payload: { accept: ["image/*"], multiple: true },
  });

  const filesSave = host.parseAnyDeviceEffect({
    v: 1,
    kind: "x07.web_ui.effect.device.files.save_text",
    request_id: "req5",
    op: "files.save",
    capability: "files.save",
    payload: { name: "draft.txt", text: "hello" },
  });
  assert.deepEqual(filesSave, {
    family: "files",
    kind: "x07.web_ui.effect.device.files.save_text",
    request_id: "req5",
    op: "files.save",
    capability: "files.save",
    payload: { name: "draft.txt", text: "hello" },
  });

  const share = host.parseAnyDeviceEffect({
    v: 1,
    kind: "x07.web_ui.effect.device.share.share_text",
    request_id: "req6",
    op: "share.present",
    capability: "share.present",
    payload: { title: "CrewOps", text: "Assigned" },
  });
  assert.deepEqual(share, {
    family: "share",
    kind: "x07.web_ui.effect.device.share.share_text",
    request_id: "req6",
    op: "share.present",
    capability: "share.present",
    payload: { title: "CrewOps", text: "Assigned" },
  });
});

test("capabilityAllowed() follows the Forge M0 capability names", () => {
  const capabilities = {
    device: {
      camera: { photo: true },
      audio: { playback: true },
      haptics: { present: true },
      clipboard: { read_text: true, write_text: true },
      files: { pick: true, pick_multiple: true, save: true, drop: true },
      blob_store: { enabled: true },
      location: { foreground: false },
      notifications: { local: true },
      share: { present: true },
    },
  };

  assert.equal(host.capabilityAllowed(capabilities, "camera.photo"), true);
  assert.equal(host.capabilityAllowed(capabilities, "audio.playback"), true);
  assert.equal(host.capabilityAllowed(capabilities, "haptics.present"), true);
  assert.equal(host.capabilityAllowed(capabilities, "clipboard.read_text"), true);
  assert.equal(host.capabilityAllowed(capabilities, "clipboard.write_text"), true);
  assert.equal(host.capabilityAllowed(capabilities, "files.pick"), true);
  assert.equal(host.capabilityAllowed(capabilities, "files.pick_multiple"), true);
  assert.equal(host.capabilityAllowed(capabilities, "files.save"), true);
  assert.equal(host.capabilityAllowed(capabilities, "files.drop"), true);
  assert.equal(host.capabilityAllowed(capabilities, "blob_store"), true);
  assert.equal(host.capabilityAllowed(capabilities, "location.foreground"), false);
  assert.equal(host.capabilityAllowed(capabilities, "notifications.local"), true);
  assert.equal(host.capabilityAllowed(capabilities, "share.present"), true);
});

test("normalizeDeviceResult() normalizes file payload items and host metadata", () => {
  const request = {
    family: "files",
    request_id: "req7",
    op: "files.pick",
    capability: "files.pick_multiple",
  };
  const normalized = host.normalizeDeviceResult(
    request,
    {
      status: "ok",
      payload: {
        blobs: [
          {
            handle: "blob:sha256:abc",
            sha256: "abc",
            mime: "application/pdf",
            byte_size: 42,
            created_at_ms: 1,
            source: "files.pick",
            local_state: "present",
          },
        ],
      },
      host_meta: { provider: "browser" },
    },
    "files",
    "web",
  );
  assert.equal(normalized.family, "files");
  assert.equal(normalized.result.request_id, "req7");
  assert.equal(normalized.result.status, "ok");
  assert.equal(normalized.result.host_meta.platform, "web");
  assert.equal(normalized.result.host_meta.provider, "browser");
  assert.deepEqual(normalized.result.payload.items, [
    {
      name: "",
      mime: "application/pdf",
      byte_size: 42,
      blob: {
        handle: "blob:sha256:abc",
        sha256: "abc",
        mime: "application/pdf",
        byte_size: 42,
        created_at_ms: 1,
        source: "files.pick",
        local_state: "present",
      },
    },
  ]);
});

test("normalizeDeviceHostEvent() normalizes files.drop events", () => {
  assert.throws(() => host.normalizeDeviceHostEvent({}), /missing type/);
  assert.deepEqual(host.normalizeDeviceHostEvent({ type: "connectivity.online" }), {
    type: "connectivity.online",
  });
  assert.deepEqual(
    host.normalizeDeviceHostEvent({
      type: "files.drop",
      target: "dropzone",
      items: [
        {
          name: "a.txt",
          mime: "text/plain",
          byte_size: 5,
          blob: {
            handle: "blob:sha256:abc",
            sha256: "abc",
            mime: "text/plain",
            byte_size: 5,
            created_at_ms: 1,
            source: "files.drop",
            local_state: "present",
          },
        },
      ],
    }),
    {
      type: "files.drop",
      target: "dropzone",
      items: [
        {
          name: "a.txt",
          mime: "text/plain",
          byte_size: 5,
          blob: {
            handle: "blob:sha256:abc",
            sha256: "abc",
            mime: "text/plain",
            byte_size: 5,
            created_at_ms: 1,
            source: "files.drop",
            local_state: "present",
          },
        },
      ],
    },
  );
});

test("createBrowserNativeHost() executes clipboard, save, and share requests", async () => {
  const originalNavigator = globalThis.navigator;
  const originalShowSaveFilePicker = globalThis.showSaveFilePicker;
  const writes = [];
  const shares = [];
  const saves = [];

  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      clipboard: {
        async writeText(text) {
          writes.push(text);
        },
        async readText() {
          return "hello from clipboard";
        },
      },
      async share(data) {
        shares.push(data);
      },
      canShare() {
        return true;
      },
    },
  });
  Object.defineProperty(globalThis, "showSaveFilePicker", {
    configurable: true,
    value: async ({ suggestedName }) => ({
      async createWritable() {
        return {
          async write(bytes) {
            saves.push({
              suggestedName,
              text: new TextDecoder().decode(bytes),
            });
          },
          async close() {},
        };
      },
    }),
  });

  try {
    const browserHost = host.createBrowserNativeHost({
      capabilities: null,
      dispatchHostEvent: async () => {},
      platform: "web",
    });

    const copy = await browserHost.invoke({
      family: "clipboard",
      kind: "x07.web_ui.effect.device.clipboard.copy_text",
      request_id: "req10",
      op: "clipboard.write_text",
      capability: "clipboard.write_text",
      payload: { text: "copied" },
    });
    assert.equal(copy.result.status, "ok");
    assert.deepEqual(writes, ["copied"]);

    const read = await browserHost.invoke({
      family: "clipboard",
      kind: "x07.web_ui.effect.device.clipboard.read_text",
      request_id: "req11",
      op: "clipboard.read_text",
      capability: "clipboard.read_text",
      payload: {},
    });
    assert.equal(read.result.status, "ok");
    assert.equal(read.result.payload.text, "hello from clipboard");

    const save = await browserHost.invoke({
      family: "files",
      kind: "x07.web_ui.effect.device.files.save_text",
      request_id: "req12",
      op: "files.save",
      capability: "files.save",
      payload: { name: "draft.txt", text: "hello save" },
    });
    assert.equal(save.result.status, "ok");
    assert.deepEqual(save.result.payload.items, [
      { name: "draft.txt", mime: "text/plain;charset=utf-8", byte_size: 10 },
    ]);
    assert.deepEqual(saves, [{ suggestedName: "draft.txt", text: "hello save" }]);

    const share = await browserHost.invoke({
      family: "share",
      kind: "x07.web_ui.effect.device.share.share_text",
      request_id: "req13",
      op: "share.present",
      capability: "share.present",
      payload: { title: "CrewOps", text: "Assigned" },
    });
    assert.equal(share.result.status, "ok");
    assert.deepEqual(shares, [{ title: "CrewOps", text: "Assigned", url: undefined }]);
  } finally {
    if (originalNavigator === undefined) {
      delete globalThis.navigator;
    } else {
      Object.defineProperty(globalThis, "navigator", {
        configurable: true,
        value: originalNavigator,
      });
    }
    if (originalShowSaveFilePicker === undefined) {
      delete globalThis.showSaveFilePicker;
    } else {
      Object.defineProperty(globalThis, "showSaveFilePicker", {
        configurable: true,
        value: originalShowSaveFilePicker,
      });
    }
  }
});

test("createBrowserNativeHost() executes audio and haptics requests", async () => {
  const originalNavigator = globalThis.navigator;
  const originalAudioContext = globalThis.AudioContext;
  const audioOps = [];
  const hapticOps = [];

  class FakeAudioParam {
    setValueAtTime() {}
    linearRampToValueAtTime() {}
    exponentialRampToValueAtTime() {}
  }

  class FakeGainNode {
    constructor() {
      this.gain = new FakeAudioParam();
    }
    connect() {}
    disconnect() {}
  }

  class FakeOscillatorNode {
    constructor() {
      this.frequency = {
        setValueAtTime: (value) => {
          this.frequencyValue = value;
        },
      };
    }
    connect() {}
    start(when) {
      audioOps.push({ op: "start", when, type: this.type, frequency: this.frequencyValue });
    }
    stop(when) {
      audioOps.push({ op: "stop", when });
    }
    disconnect() {}
  }

  class FakeAudioContext {
    constructor() {
      this.currentTime = 0;
      this.destination = {};
    }
    async resume() {
      audioOps.push({ op: "resume" });
    }
    createOscillator() {
      return new FakeOscillatorNode();
    }
    createGain() {
      return new FakeGainNode();
    }
  }

  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      vibrate(pattern) {
        hapticOps.push(pattern);
        return true;
      },
    },
  });
  Object.defineProperty(globalThis, "AudioContext", {
    configurable: true,
    value: FakeAudioContext,
  });

  try {
    const browserHost = host.createBrowserNativeHost({
      capabilities: null,
      dispatchHostEvent: async () => {},
      platform: "web",
    });

    const play = await browserHost.invoke({
      family: "audio",
      kind: "x07.web_ui.effect.device.audio.play",
      request_id: "req20",
      op: "audio.play",
      capability: "audio.playback",
      payload: { cue: "select", channel: "sfx", loop: false },
    });
    assert.equal(play.result.status, "ok");
    assert.deepEqual(play.result.payload, { cue: "select", channel: "sfx", loop: false });
    assert.equal(audioOps.some((entry) => entry.op === "start"), true);

    const stop = await browserHost.invoke({
      family: "audio",
      kind: "x07.web_ui.effect.device.audio.stop",
      request_id: "req21",
      op: "audio.stop",
      capability: "audio.playback",
      payload: { channel: "sfx" },
    });
    assert.equal(stop.result.status, "ok");
    assert.equal(stop.result.payload.channel, "sfx");

    const haptics = await browserHost.invoke({
      family: "haptics",
      kind: "x07.web_ui.effect.device.haptics.trigger",
      request_id: "req22",
      op: "haptics.trigger",
      capability: "haptics.present",
      payload: { pattern: "impact" },
    });
    assert.equal(haptics.result.status, "ok");
    assert.deepEqual(hapticOps, [[25]]);
  } finally {
    if (originalNavigator === undefined) {
      delete globalThis.navigator;
    } else {
      Object.defineProperty(globalThis, "navigator", {
        configurable: true,
        value: originalNavigator,
      });
    }
    if (originalAudioContext === undefined) {
      delete globalThis.AudioContext;
    } else {
      Object.defineProperty(globalThis, "AudioContext", {
        configurable: true,
        value: originalAudioContext,
      });
    }
  }
});
