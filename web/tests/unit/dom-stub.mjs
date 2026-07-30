// A deliberately small DOM, built to unit-test `src/web/document.js` without a
// browser. jsdom is not a fit here: it has no layout engine, so every
// `getClientRects()` returns nothing and the extractor's visibility filter
// discards the whole document. This stub instead models layout explicitly —
// every character occupies a `CHAR_WIDTH` x `LINE_HEIGHT` box on its text
// node's own line — which keeps rect and visibility assertions deterministic
// and lets a test place a node exactly where it wants it.
//
// Only the surface `document.js` touches is implemented. Selector support is
// limited to comma-separated `tag` and `#id` terms, which covers every
// selector that module passes to `matches`, `closest`, and `querySelectorAll`.

export const CHAR_WIDTH = 10;
export const LINE_HEIGHT = 20;

globalThis.Node ??= { ELEMENT_NODE: 1, TEXT_NODE: 3 };
globalThis.NodeFilter ??= { SHOW_ELEMENT: 1, SHOW_TEXT: 4 };

const BLOCK_TAGS = new Set([
  "article",
  "aside",
  "blockquote",
  "body",
  "dd",
  "div",
  "dl",
  "dt",
  "figcaption",
  "figure",
  "footer",
  "form",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "header",
  "html",
  "main",
  "nav",
  "ol",
  "p",
  "pre",
  "section",
  "template",
  "ul",
]);

function defaultDisplay(localName) {
  if (BLOCK_TAGS.has(localName)) return "block";
  if (localName === "li" || localName === "summary") return "list-item";
  if (localName === "td" || localName === "th") return "table-cell";
  if (localName === "table") return "table";
  if (localName === "tr") return "table-row";
  return "inline";
}

function rect(left, top, width, height) {
  return {
    left,
    top,
    right: left + width,
    bottom: top + height,
    width,
    height,
    x: left,
    y: top,
  };
}

function matchesSelector(element, selector) {
  return selector
    .split(",")
    .map((term) => term.trim())
    .some((term) =>
      term.startsWith("#")
        ? element.id === term.slice(1).replace(/\\(.)/g, "$1")
        : element.localName === term,
    );
}

class StubStyle {
  setProperty(name, value, priority = "") {
    this[name] = value;
    this[`${name}:priority`] = priority;
  }
}

class StubNode {
  constructor(ownerDocument) {
    this.ownerDocument = ownerDocument;
    this.parentNode = null;
  }

  get parentElement() {
    return this.parentNode?.nodeType === 1 ? this.parentNode : null;
  }
}

export class StubText extends StubNode {
  constructor(ownerDocument, data, { origin, renders = true } = {}) {
    super(ownerDocument);
    this.nodeType = 3;
    this.data = data;
    this.origin = origin ?? { x: 0, y: 0 };
    this.renders = renders;
  }

  // UTF-16 offsets, matching the offsets `normalizePieces` records.
  rectsForRange(start, end) {
    if (!this.renders || !(end > start)) return [];
    return [
      rect(
        this.origin.x + start * CHAR_WIDTH,
        this.origin.y,
        (end - start) * CHAR_WIDTH,
        LINE_HEIGHT,
      ),
    ];
  }
}

export class StubElement extends StubNode {
  constructor(ownerDocument, localName, options = {}) {
    super(ownerDocument);
    this.nodeType = 1;
    this.localName = localName;
    this.id = options.id ?? "";
    this.classList = [...(options.class ?? [])];
    this.dataset = { ...(options.dataset ?? {}) };
    this.hidden = Boolean(options.hidden);
    this.attributes = new Map(Object.entries(options.attrs ?? {}));
    this.styleOverrides = options.style ?? {};
    this.style = new StubStyle();
    this.childNodes = [];
    this.elementRects = options.rects;
    this.scrolledIntoView = null;
    this.shadowRoot = null;
  }

  get children() {
    return this.childNodes.filter((node) => node.nodeType === 1);
  }

  get className() {
    return this.classList.join(" ");
  }

  set className(value) {
    this.classList = value ? value.split(/\s+/u) : [];
  }

  get textContent() {
    return this.childNodes
      .map((node) => (node.nodeType === 3 ? node.data : node.textContent))
      .join("");
  }

  set textContent(value) {
    this.childNodes = [];
    if (value) this.append(new StubText(this.ownerDocument, value));
  }

  matches(selector) {
    return matchesSelector(this, selector);
  }

  closest(selector) {
    for (let current = this; current; current = current.parentElement) {
      if (current.matches(selector)) return current;
    }
    return null;
  }

  contains(node) {
    for (let current = node; current; current = current.parentNode) {
      if (current === this) return true;
    }
    return false;
  }

  getAttribute(name) {
    return this.attributes.get(name) ?? null;
  }

  getClientRects() {
    // A `br` reports a zero-width caret box; other elements report nothing
    // unless a test supplies rects, since only `br` layout drives extraction.
    if (this.elementRects) return this.elementRects;
    if (this.localName !== "br") return [];
    return [rect(0, 0, 0, LINE_HEIGHT)];
  }

  append(...nodes) {
    for (const node of nodes) {
      node.parentNode = this;
      this.childNodes.push(node);
    }
  }

  replaceChildren(...nodes) {
    this.childNodes = [];
    this.append(...nodes);
  }

  attachShadow() {
    this.shadowRoot = new StubElement(this.ownerDocument, "#shadow-root");
    return this.shadowRoot;
  }

  scrollIntoView(options) {
    this.scrolledIntoView = options;
  }

  querySelectorAll(selector) {
    const found = [];
    const visit = (node) => {
      for (const child of node.childNodes) {
        if (child.nodeType !== 1) continue;
        if (child.matches(selector)) found.push(child);
        visit(child);
      }
    };
    visit(this);
    return found;
  }
}

class StubRange {
  setStart(node, offset) {
    this.startContainer = node;
    this.startOffset = offset;
  }

  setEnd(node, offset) {
    this.endContainer = node;
    this.endOffset = offset;
  }

  selectNodeContents(node) {
    this.setStart(node, 0);
    this.setEnd(
      node,
      node.nodeType === 3 ? node.data.length : node.childNodes.length,
    );
  }

  getClientRects() {
    const node = this.startContainer;
    if (!node || node !== this.endContainer || node.nodeType !== 3) return [];
    return node.rectsForRange(this.startOffset, this.endOffset);
  }
}

export class StubDocument {
  constructor({ baseURI = "https://plan.test/plan.html", viewport } = {}) {
    this.nodeType = 9;
    this.baseURI = baseURI;
    this.documentElement = new StubElement(this, "html");
    this.body = new StubElement(this, "body");
    this.documentElement.append(this.body);
    this.defaultView = {
      innerWidth: viewport?.width ?? 800,
      innerHeight: viewport?.height ?? 600,
      getComputedStyle: (element) => this.computedStyle(element),
    };
  }

  computedStyle(element) {
    // `white-space` inherits, so walk ancestors for it; `display` does not.
    let whiteSpace = null;
    for (let current = element; current && !whiteSpace; current = current.parentElement) {
      whiteSpace =
        current.styleOverrides?.whiteSpace ??
        (current.localName === "pre" ? "pre" : null);
    }
    return {
      display: element.styleOverrides?.display ?? defaultDisplay(element.localName),
      visibility: element.styleOverrides?.visibility ?? "visible",
      whiteSpace: whiteSpace ?? "normal",
    };
  }

  createElement(localName) {
    return new StubElement(this, localName);
  }

  createTextNode(data) {
    return new StubText(this, data);
  }

  createRange() {
    return new StubRange();
  }

  createTreeWalker(root, whatToShow = 0xffffffff) {
    const ordered = [];
    const visit = (node) => {
      for (const child of node.childNodes) {
        ordered.push(child);
        if (child.nodeType === 1) visit(child);
      }
    };
    visit(root);
    const shown = ordered.filter((node) =>
      node.nodeType === 1
        ? whatToShow & NodeFilter.SHOW_ELEMENT
        : whatToShow & NodeFilter.SHOW_TEXT,
    );
    let index = 0;
    return {
      nextNode: () => shown[index++] ?? null,
    };
  }

  querySelectorAll(selector) {
    return this.documentElement.matches(selector)
      ? [this.documentElement, ...this.documentElement.querySelectorAll(selector)]
      : this.documentElement.querySelectorAll(selector);
  }
}

/// Element spec. `source` is shorthand for the `data-rep-*` markers the HTML
/// transform injects: a number sets both id and line, a pair sets them apart.
export function el(localName, options = {}, ...children) {
  return { kind: "element", localName, options, children: children.flat() };
}

export function txt(data, options = {}) {
  return { kind: "text", data, options };
}

function sourceDataset(source) {
  if (source === undefined) return {};
  const [id, line] = Array.isArray(source) ? source : [source, source];
  return { repSourceId: String(id), repSourceLine: String(line) };
}

/// Materializes a spec tree into `doc.body`. Each text node lands on its own
/// line unless the spec pins an `origin`, so rects never overlap by accident.
export function buildDocument(specs, options = {}) {
  const doc = new StubDocument(options);
  let line = 0;
  const materialize = (spec, parent) => {
    if (spec.kind === "text") {
      const node = new StubText(doc, spec.data, {
        origin: spec.options.origin ?? { x: 0, y: line * LINE_HEIGHT },
        renders: spec.options.renders ?? true,
      });
      line += 1;
      parent.append(node);
      return node;
    }
    const { source, ...rest } = spec.options;
    const element = new StubElement(doc, spec.localName, {
      ...rest,
      dataset: { ...sourceDataset(source), ...(rest.dataset ?? {}) },
    });
    parent.append(element);
    for (const child of spec.children) materialize(child, element);
    return element;
  };
  for (const spec of [specs].flat()) materialize(spec, doc.body);
  return doc;
}
