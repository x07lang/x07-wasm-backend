class FakeStyle {
  constructor() {
    this.map = new Map();
  }

  setProperty(name, value) {
    this.map.set(String(name), String(value));
  }

  removeProperty(name) {
    this.map.delete(String(name));
  }

  [Symbol.iterator]() {
    return this.map.keys();
  }
}

class FakeNode {
  constructor(doc, nodeType) {
    this.ownerDocument = doc;
    this.nodeType = nodeType;
    this.parentNode = null;
    this.childNodes = [];
    this.__x07Key = "";
  }

  contains(node) {
    if (this === node) return true;
    for (const child of this.childNodes) {
      if (typeof child.contains === "function" && child.contains(node)) {
        return true;
      }
    }
    return false;
  }
}

class FakeTextNode extends FakeNode {
  constructor(doc, text) {
    super(doc, 3);
    this.textContent = String(text);
  }
}

class FakeElement extends FakeNode {
  constructor(doc, tagName) {
    super(doc, 1);
    this.tagName = String(tagName).toUpperCase();
    this.dataset = {};
    this.attributes = new Map();
    this.style = new FakeStyle();
    this.className = "";
    this.value = "";
    this.selectionStart = null;
    this.selectionEnd = null;
    this.selectionDirection = "none";
  }

  get firstChild() {
    return this.childNodes[0] ?? null;
  }

  get textContent() {
    return this.childNodes
      .map((child) => (child.nodeType === 3 ? child.textContent : child.textContent || ""))
      .join("");
  }

  set textContent(value) {
    this.replaceChildren(this.ownerDocument.createTextNode(String(value)));
  }

  getAttributeNames() {
    return Array.from(this.attributes.keys());
  }

  setAttribute(name, value) {
    const key = String(name);
    const str = String(value);
    this.attributes.set(key, str);
    if (key === "value") this.value = str;
  }

  removeAttribute(name) {
    this.attributes.delete(String(name));
  }

  focus() {
    this.ownerDocument.activeElement = this;
  }

  setSelectionRange(start, end, direction = "none") {
    this.selectionStart = Number(start);
    this.selectionEnd = Number(end);
    this.selectionDirection = String(direction);
  }

  replaceChildren(...nodes) {
    if (
      this.childNodes.some(
        (child) =>
          typeof child.contains === "function" && child.contains(this.ownerDocument.activeElement),
      )
    ) {
      this.ownerDocument.activeElement = null;
    }
    for (const child of this.childNodes) {
      child.parentNode = null;
    }
    this.childNodes = [];
    for (const node of nodes) {
      if (!node) continue;
      if (node.parentNode) {
        const prevParent = node.parentNode;
        prevParent.childNodes = prevParent.childNodes.filter((child) => child !== node);
      }
      node.parentNode = this;
      this.childNodes.push(node);
    }
  }
}

class FakeDocument {
  constructor(baseURI = "http://localhost/") {
    this.activeElement = null;
    this.baseURI = baseURI;
  }

  createElement(tagName) {
    return new FakeElement(this, tagName);
  }

  createTextNode(text) {
    return new FakeTextNode(this, text);
  }
}

export function installFakeDom(baseURI = "http://localhost/") {
  const document = new FakeDocument(baseURI);
  const url = new URL(baseURI);
  globalThis.document = document;
  globalThis.location = {
    origin: url.origin,
    href: url.href,
  };
  return { document };
}
