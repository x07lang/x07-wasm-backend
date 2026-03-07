import assert from "node:assert/strict";
import test from "node:test";

import { __x07_host_private as host } from "../app-host.mjs";
import { installFakeDom } from "./fake-dom.mjs";

test("render() preserves focus and selection for keyed inputs", () => {
  const { document } = installFakeDom();
  const root = document.createElement("div");

  const initialTree = {
    root: {
      k: "el",
      tag: "div",
      key: "root",
      props: {},
      children: [
        {
          k: "el",
          tag: "input",
          key: "in_text",
          props: { attrs: { value: "abcdef" } },
          children: [],
        },
        {
          k: "el",
          tag: "span",
          key: "status",
          props: {},
          children: [{ k: "text", key: "status_text", text: "idle" }],
        },
      ],
    },
  };

  const nextTree = {
    root: {
      k: "el",
      tag: "div",
      key: "root",
      props: {},
      children: [
        {
          k: "el",
          tag: "input",
          key: "in_text",
          props: { attrs: { value: "abczef" } },
          children: [],
        },
        {
          k: "el",
          tag: "span",
          key: "status",
          props: {},
          children: [{ k: "text", key: "status_text", text: "updated" }],
        },
      ],
    },
  };

  host.render(root, null, initialTree);

  const container = root.firstChild;
  const input = container.childNodes[0];
  assert.equal(input.tagName, "INPUT");

  input.focus();
  input.setSelectionRange(2, 5, "forward");

  host.render(root, initialTree, nextTree);

  const nextContainer = root.firstChild;
  const nextInput = nextContainer.childNodes[0];
  assert.equal(document.activeElement, nextInput);
  assert.equal(nextInput.selectionStart, 2);
  assert.equal(nextInput.selectionEnd, 5);
  assert.equal(nextInput.selectionDirection, "forward");
  assert.equal(nextInput, input);
});

test("render() skips replaceChildren when reconciled children are reused in place", () => {
  const { document } = installFakeDom();
  const root = document.createElement("div");

  const initialTree = {
    root: {
      k: "el",
      tag: "div",
      key: "root",
      props: {},
      children: [
        {
          k: "el",
          tag: "input",
          key: "in_text",
          props: { attrs: { value: "abcdef" } },
          children: [],
        },
        {
          k: "el",
          tag: "span",
          key: "status",
          props: {},
          children: [{ k: "text", key: "status_text", text: "idle" }],
        },
      ],
    },
  };

  const nextTree = {
    root: {
      k: "el",
      tag: "div",
      key: "root",
      props: {},
      children: [
        {
          k: "el",
          tag: "input",
          key: "in_text",
          props: { attrs: { value: "abcdef" } },
          children: [],
        },
        {
          k: "el",
          tag: "span",
          key: "status",
          props: {},
          children: [{ k: "text", key: "status_text", text: "updated" }],
        },
      ],
    },
  };

  host.render(root, null, initialTree);

  const container = root.firstChild;
  let rootReplaceCalls = 0;
  let containerReplaceCalls = 0;

  const rootReplaceChildren = root.replaceChildren.bind(root);
  root.replaceChildren = (...nodes) => {
    rootReplaceCalls += 1;
    return rootReplaceChildren(...nodes);
  };

  const containerReplaceChildren = container.replaceChildren.bind(container);
  container.replaceChildren = (...nodes) => {
    containerReplaceCalls += 1;
    return containerReplaceChildren(...nodes);
  };

  host.render(root, initialTree, nextTree);

  assert.equal(rootReplaceCalls, 0);
  assert.equal(containerReplaceCalls, 0);
});
