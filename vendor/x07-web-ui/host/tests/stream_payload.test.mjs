import assert from "node:assert/strict";
import test from "node:test";

import { __x07_host_private as host } from "../app-host.mjs";

test("bytesToStreamPayload() emits text without redundant base64 for UTF-8 bodies", () => {
  const bytes = new TextEncoder().encode('{"ok":true,"message":"dispatch ready"}');
  const payload = host.bytesToStreamPayload(bytes);

  assert.deepEqual(payload, {
    bytes_len: bytes.length,
    text: '{"ok":true,"message":"dispatch ready"}',
  });
  assert.deepEqual(host.streamPayloadToBytes(payload), bytes);
});

test("bytesToStreamPayload() falls back to base64 for invalid UTF-8", () => {
  const bytes = Uint8Array.from([0xff, 0xfe, 0x61, 0x62]);
  const payload = host.bytesToStreamPayload(bytes);

  assert.equal(payload.bytes_len, bytes.length);
  assert.equal(typeof payload.base64, "string");
  assert.equal(Object.prototype.hasOwnProperty.call(payload, "text"), false);
  assert.deepEqual(host.streamPayloadToBytes(payload), bytes);
});
