import assert from "node:assert/strict";
import test from "node:test";

import { __x07_host_private as host } from "../app-host.mjs";

globalThis.document = { baseURI: "https://example.com/app/" };
globalThis.location = { origin: "https://example.com" };

test("sanitizeTag() rejects non-allowlisted tags", () => {
  assert.equal(host.sanitizeTag("script"), "div");
  assert.equal(host.sanitizeTag("div"), "div");
});

test("sanitizeAttr() rejects on* handlers", () => {
  assert.equal(host.sanitizeAttr("div", "onload", "alert(1)"), null);
  assert.equal(host.sanitizeAttr("div", "onclick", "alert(1)"), null);
});

test("sanitizeAttr() rejects javascript: hrefs", () => {
  assert.equal(host.sanitizeAttr("a", "href", "javascript:alert(1)"), null);
});

test("sanitizeAttr() rejects cross-origin hrefs", () => {
  assert.equal(host.sanitizeAttr("a", "href", "https://evil.com/"), null);
});

test("sanitizeAttr() allows same-origin path hrefs", () => {
  assert.deepEqual(host.sanitizeAttr("a", "href", "/ok?x=1#y"), { name: "href", value: "/ok?x=1#y" });
});

