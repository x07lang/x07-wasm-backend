const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder("utf-8", { fatal: false });

function alignUp(x, align) {
  if (align <= 1) return x;
  return (x + (align - 1)) & ~(align - 1);
}

function globalU32(exported, name) {
  if (typeof exported === "number") return exported >>> 0;
  if (exported && typeof exported === "object" && "value" in exported) {
    return (exported.value >>> 0) >>> 0;
  }
  throw new Error(`missing wasm global export ${name}`);
}

async function instantiateCoreWasm(wasmUrl, imports) {
  const resp = await fetch(wasmUrl);
  if (!resp.ok) {
    throw new Error(`fetch wasm failed: ${resp.status} ${resp.statusText}`);
  }
  const contentType = resp.headers.get("content-type") || "";
  const want = "application/wasm";
  const mimeOk = contentType.toLowerCase() === want;

  if (WebAssembly.instantiateStreaming) {
    if (!mimeOk) {
      throw new Error(`wasm MIME must be ${want}; got: ${contentType || "(missing)"}`);
    }
    const { instance, module } = await WebAssembly.instantiateStreaming(resp, imports);
    return { instance, module };
  }

  const buf = await resp.arrayBuffer();
  const { instance, module } = await WebAssembly.instantiate(buf, imports);
  return { instance, module };
}

function createSolveV2Core(exports, { arenaCapBytes, maxOutputBytes }) {
  if (!exports || typeof exports !== "object") {
    throw new Error("missing wasm exports");
  }
  const memory = exports.memory;
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("missing wasm memory export");
  }
  const solve = exports.x07_solve_v2;
  if (typeof solve !== "function") {
    throw new Error("missing wasm export x07_solve_v2");
  }

  const heapBase = globalU32(exports.__heap_base, "__heap_base");
  const dataEnd = globalU32(exports.__data_end, "__data_end");

  function ensureMemory(needBytes) {
    const haveBytes = memory.buffer.byteLength;
    if (needBytes <= haveBytes) return;
    const page = 65536;
    const delta = needBytes - haveBytes;
    const pages = Math.ceil(delta / page);
    memory.grow(pages);
  }

  function callSolveV2(inputBytes) {
    const allocBase = Math.max(heapBase, dataEnd) >>> 0;
    const retptr = alignUp(allocBase, 8) >>> 0;
    const inputPtr = alignUp(retptr + 8, 8) >>> 0;
    const arenaPtr = alignUp(inputPtr + inputBytes.length, 8) >>> 0;
    const arenaEnd = arenaPtr + arenaCapBytes;

    ensureMemory(arenaEnd);
    const u8 = new Uint8Array(memory.buffer);
    const dv = new DataView(memory.buffer);

    dv.setUint32(retptr + 0, 0, true);
    dv.setUint32(retptr + 4, 0, true);
    u8.set(inputBytes, inputPtr);

    solve(retptr, arenaPtr, arenaCapBytes, inputPtr, inputBytes.length);

    const outPtr = dv.getUint32(retptr + 0, true) >>> 0;
    const outLen = dv.getUint32(retptr + 4, true) >>> 0;
    if (outLen > maxOutputBytes) {
      throw new Error(`output too large: outLen=${outLen} maxOutputBytes=${maxOutputBytes}`);
    }

    if (outPtr < arenaPtr || outPtr + outLen > arenaEnd) {
      throw new Error(
        `output not within arena: out=[${outPtr},${outPtr + outLen}) arena=[${arenaPtr},${arenaEnd})`,
      );
    }

    const out = u8.slice(outPtr, outPtr + outLen);
    return out;
  }

  return { callSolveV2 };
}

function decodePointerToken(raw) {
  return String(raw).replace(/~1/g, "/").replace(/~0/g, "~");
}

function parseJsonPointer(path) {
  const p = String(path ?? "");
  if (p === "") return [];
  if (!p.startsWith("/")) throw new Error(`invalid JSON Pointer: ${p}`);
  return p
    .slice(1)
    .split("/")
    .map(decodePointerToken);
}

function applyJsonPatch(doc, patchset) {
  let root = doc;
  const ops = Array.isArray(patchset) ? patchset : [];
  for (const op of ops) {
    if (!op || typeof op !== "object") throw new Error("invalid patch op");
    const kind = String(op.op ?? "");
    const path = String(op.path ?? "");
    const tokens = parseJsonPointer(path);

    if (tokens.length === 0) {
      if (kind === "add" || kind === "replace") {
        root = op.value;
        continue;
      }
      if (kind === "remove") {
        root = null;
        continue;
      }
      throw new Error(`unsupported op: ${kind}`);
    }

    let parent = root;
    for (let i = 0; i < tokens.length - 1; i++) {
      const t = tokens[i];
      if (Array.isArray(parent)) {
        const idx = t === "-" ? parent.length : Number(t);
        if (!Number.isFinite(idx) || idx < 0 || idx >= parent.length) {
          throw new Error(`invalid array index in path: ${path}`);
        }
        parent = parent[idx];
      } else if (parent && typeof parent === "object") {
        if (!(t in parent)) throw new Error(`missing object key in path: ${path}`);
        parent = parent[t];
      } else {
        throw new Error(`invalid parent in path: ${path}`);
      }
    }

    const last = tokens[tokens.length - 1];
    if (Array.isArray(parent)) {
      const idx = last === "-" ? parent.length : Number(last);
      if (!Number.isFinite(idx) || idx < 0) throw new Error(`invalid array index in path: ${path}`);
      if (kind === "add") {
        if (idx > parent.length) throw new Error(`add index out of bounds: ${path}`);
        parent.splice(idx, 0, op.value);
        continue;
      }
      if (kind === "remove") {
        if (idx >= parent.length) throw new Error(`remove index out of bounds: ${path}`);
        parent.splice(idx, 1);
        continue;
      }
      if (kind === "replace") {
        if (idx >= parent.length) throw new Error(`replace index out of bounds: ${path}`);
        parent[idx] = op.value;
        continue;
      }
      throw new Error(`unsupported op: ${kind}`);
    }

    if (!parent || typeof parent !== "object") throw new Error(`invalid parent in path: ${path}`);
    if (kind === "add" || kind === "replace") {
      parent[last] = op.value;
      continue;
    }
    if (kind === "remove") {
      if (!(last in parent)) throw new Error(`remove missing key: ${path}`);
      delete parent[last];
      continue;
    }
    throw new Error(`unsupported op: ${kind}`);
  }
  return root;
}

function stableJson(value) {
  if (value === null) return "null";
  const t = typeof value;
  if (t === "boolean") return value ? "true" : "false";
  if (t === "number") return JSON.stringify(value);
  if (t === "string") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(stableJson).join(",")}]`;
  if (t !== "object") return "null";
  const keys = Object.keys(value).sort();
  return `{${keys.map((k) => `${JSON.stringify(k)}:${stableJson(value[k])}`).join(",")}}`;
}

function buildAllowedEventsMap(node, out) {
  if (!node || typeof node !== "object") return;
  const key = node.key != null ? String(node.key) : "";
  if (key) {
    const on = Array.isArray(node.on) ? node.on : [];
    const set = out.get(key) || new Set();
    for (const h of on) {
      if (!h || typeof h !== "object") continue;
      const type = String(h.type || "");
      if (!type) continue;
      set.add(type);
    }
    out.set(key, set);
  }
  const children = Array.isArray(node.children) ? node.children : [];
  for (const c of children) buildAllowedEventsMap(c, out);
}

function setAttrs(el, attrs) {
  const next = attrs && typeof attrs === "object" ? attrs : {};
  const prevNames = new Set();
  for (const name of el.getAttributeNames()) prevNames.add(name);
  for (const [k, v] of Object.entries(next)) {
    prevNames.delete(k);
    if (v == null) continue;
    el.setAttribute(k, String(v));
  }
  for (const k of prevNames) {
    if (k === "data-x07-key") continue;
    el.removeAttribute(k);
  }
}

function setClass(el, cls) {
  if (Array.isArray(cls)) {
    el.className = cls.filter((c) => c != null && String(c).length > 0).map(String).join(" ");
    return;
  }
  if (typeof cls === "string") {
    el.className = cls;
    return;
  }
  el.className = "";
}

function setStyle(el, style) {
  const next = style && typeof style === "object" ? style : {};
  const prev = new Set();
  for (const k of Array.from(el.style)) prev.add(k);
  for (const [k, v] of Object.entries(next)) {
    prev.delete(k);
    if (v == null) continue;
    el.style.setProperty(k, String(v));
  }
  for (const k of prev) {
    el.style.removeProperty(k);
  }
}

function reconcileNode(prevNode, prevDom, nextNode) {
  if (!nextNode || typeof nextNode !== "object") {
    return document.createTextNode("");
  }
  const kind = String(nextNode.k || "");
  const key = nextNode.key != null ? String(nextNode.key) : "";

  if (kind === "text") {
    const text = String(nextNode.text ?? "");
    const reuse = prevDom && prevDom.nodeType === 3 && prevDom.__x07Key === key;
    const node = reuse ? prevDom : document.createTextNode(text);
    node.__x07Key = key;
    if (reuse && node.textContent !== text) node.textContent = text;
    return node;
  }

  if (kind !== "el") {
    const node = document.createTextNode("");
    node.__x07Key = key;
    return node;
  }

  const tag = String(nextNode.tag || "div");
  const canReuse =
    prevDom &&
    prevDom.nodeType === 1 &&
    prevDom.__x07Key === key &&
    String(prevDom.tagName || "").toLowerCase() === tag;
  const el = canReuse ? prevDom : document.createElement(tag);
  el.__x07Key = key;
  if (key) el.dataset.x07Key = key;

  const props = nextNode.props && typeof nextNode.props === "object" ? nextNode.props : {};
  setAttrs(el, props.attrs);
  setClass(el, props.class);
  setStyle(el, props.style);

  const prevChildren = prevNode && typeof prevNode === "object" ? prevNode.children : [];
  const nextChildren = Array.isArray(nextNode.children) ? nextNode.children : [];
  const prevDomChildren = Array.from(el.childNodes);

  const prevByKey = new Map();
  if (Array.isArray(prevChildren)) {
    for (let i = 0; i < prevChildren.length; i++) {
      const pn = prevChildren[i];
      const pd = prevDomChildren[i];
      const pk = pn && typeof pn === "object" && pn.key != null ? String(pn.key) : "";
      if (pk && pd) prevByKey.set(pk, { node: pn, dom: pd });
    }
  }

  const newDomChildren = [];
  for (const cn of nextChildren) {
    const ck = cn && typeof cn === "object" && cn.key != null ? String(cn.key) : "";
    const prev = ck ? prevByKey.get(ck) : null;
    const childDom = reconcileNode(prev ? prev.node : null, prev ? prev.dom : null, cn);
    newDomChildren.push(childDom);
  }
  el.replaceChildren(...newDomChildren);

  return el;
}

function render(root, prevTree, nextTree) {
  if (!nextTree || typeof nextTree !== "object") {
    root.textContent = "";
    return;
  }
  const prevNode = prevTree && typeof prevTree === "object" ? prevTree.root : null;
  const nextNode = nextTree.root;
  const prevDom = root.firstChild;
  const nextDom = reconcileNode(prevNode, prevDom, nextNode);
  root.replaceChildren(nextDom);
}

function downloadJson(filename, doc) {
  const bytes = textEncoder.encode(JSON.stringify(doc, null, 2) + "\n");
  const blob = new Blob([bytes], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export async function mountWebUiApp({
  wasmUrl,
  componentEsmUrl,
  root,
  arenaCapBytes = 1 << 20,
  maxOutputBytes = 1 << 20,
} = {}) {
  if (!wasmUrl && !componentEsmUrl) throw new Error("missing wasmUrl/componentEsmUrl");
  if (!root) throw new Error("missing root");

  const trace = {
    v: 1,
    kind: "x07.web_ui.trace",
    steps: [],
    meta: {
      wasmUrl: wasmUrl || null,
      componentEsmUrl: componentEsmUrl || null,
      startedAtUnixMs: Date.now(),
    },
  };

  let app = null;
  if (componentEsmUrl) {
    const m = await import(componentEsmUrl);
    if (!m || typeof m !== "object") throw new Error("invalid component ESM module");
    const init = m.init;
    const step = m.step;
    if (typeof init !== "function" || typeof step !== "function") {
      throw new Error("component ESM module must export init and step functions");
    }
    app = {
      kind: "component+esm",
      init: async () => {
        const out = await init();
        return out instanceof Uint8Array ? out : textEncoder.encode(String(out ?? ""));
      },
      step: async (inputBytes) => {
        const out = await step(inputBytes);
        return out instanceof Uint8Array ? out : textEncoder.encode(String(out ?? ""));
      },
    };
  } else {
    const { instance } = await instantiateCoreWasm(wasmUrl, {});
    const core = createSolveV2Core(instance.exports, { arenaCapBytes, maxOutputBytes });
    app = {
      kind: "core",
      init: async () => {
        const env = { v: 1, kind: "x07.web_ui.dispatch", state: null, event: { type: "init" } };
        const inputBytes = textEncoder.encode(JSON.stringify(env));
        return core.callSolveV2(inputBytes);
      },
      step: async (inputBytes) => core.callSolveV2(inputBytes),
    };
  }

  let state = null;
  let ui = null;
  let allowedEvents = new Map();

  async function dispatch(event) {
    const env = { v: 1, kind: "x07.web_ui.dispatch", state, event };
    const inputBytes = textEncoder.encode(JSON.stringify(env));

    const started = performance.now();
    const outBytes = await app.step(inputBytes);
    const wallMs = performance.now() - started;

    const frameText = textDecoder.decode(outBytes);
    const frame = JSON.parse(frameText);
    trace.steps.push({ env, frame, wallMs });

    const nextState = frame.state;
    const nextUi = frame.ui;
    const patches = frame.patches;

    if (ui != null) {
      const patched = applyJsonPatch(JSON.parse(stableJson(ui)), patches);
      const a = stableJson(patched);
      const b = stableJson(nextUi);
      if (a !== b) {
        throw new Error(`patchset does not match ui tree: patched!=ui`);
      }
    }

    state = nextState;
    const prevUi = ui;
    ui = nextUi;
    allowedEvents = new Map();
    if (ui && typeof ui === "object") buildAllowedEventsMap(ui.root, allowedEvents);
    render(root, prevUi, ui);
    return frame;
  }

  try {
    const started = performance.now();
    const outBytes = await app.init();
    const wallMs = performance.now() - started;
    const frameText = textDecoder.decode(outBytes);
    const frame = JSON.parse(frameText);
    const env = { v: 1, kind: "x07.web_ui.dispatch", state: null, event: { type: "init" } };
    trace.steps.push({ env, frame, wallMs });

    state = frame.state;
    ui = frame.ui;
    allowedEvents = new Map();
    if (ui && typeof ui === "object") buildAllowedEventsMap(ui.root, allowedEvents);
    render(root, null, ui);
  } catch (err) {
    const incident = {
      v: 1,
      kind: "x07.web_ui.incident",
      error: String(err?.stack || err),
      trace,
    };
    console.error(err);
    downloadJson("incident.json", incident);
    throw err;
  }

  root.addEventListener(
    "click",
    (ev) => {
      const el = ev?.target?.closest?.("[data-x07-key]");
      const target = el?.dataset?.x07Key || "";
      if (!target) return;
      if (!(allowedEvents.get(target)?.has("click") ?? false)) return;
      ev.preventDefault();
      void dispatch({ type: "click", target }).catch((err) => console.error(err));
    },
    true,
  );
  root.addEventListener(
    "input",
    (ev) => {
      const el = ev?.target?.closest?.("[data-x07-key]");
      const target = el?.dataset?.x07Key || "";
      if (!target) return;
      if (!(allowedEvents.get(target)?.has("input") ?? false)) return;
      const value = ev?.target?.value ?? "";
      void dispatch({ type: "input", target, value: String(value) }).catch((err) =>
        console.error(err),
      );
    },
    true,
  );
  root.addEventListener(
    "change",
    (ev) => {
      const el = ev?.target?.closest?.("[data-x07-key]");
      const target = el?.dataset?.x07Key || "";
      if (!target) return;
      if (!(allowedEvents.get(target)?.has("change") ?? false)) return;
      const value = ev?.target?.value ?? "";
      void dispatch({ type: "change", target, value: String(value) }).catch((err) =>
        console.error(err),
      );
    },
    true,
  );
  root.addEventListener(
    "submit",
    (ev) => {
      const el = ev?.target?.closest?.("[data-x07-key]");
      const target = el?.dataset?.x07Key || "";
      if (!target) return;
      if (!(allowedEvents.get(target)?.has("submit") ?? false)) return;
      ev.preventDefault();
      void dispatch({ type: "submit", target }).catch((err) => console.error(err));
    },
    true,
  );

  return {
    dispatch,
    getState: () => state,
    getUi: () => ui,
    trace,
    downloadTrace: () => downloadJson("trace.json", trace),
  };
}
