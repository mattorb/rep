const SEMANTIC_SELECTOR =
  "h1,h2,h3,h4,h5,h6,p,li,pre,td,th,dt,dd,figcaption,summary";
const BLOCK_DISPLAYS = new Set([
  "block",
  "list-item",
  "table-cell",
  "flex",
  "grid",
  "flow-root",
]);

export function normalizePieces(pieces) {
  const output = [];
  let pending = null;

  const append = (character, start, end, link) => {
    output.push({ character, start, end, link });
  };
  const flushSpace = () => {
    if (pending && output.length && output.at(-1).character !== "\n") {
      append(" ", pending.start, pending.end, pending.link);
    }
    pending = null;
  };
  const appendNewline = (start, end) => {
    pending = null;
    while (output.at(-1)?.character === " ") output.pop();
    if (output.length) append("\n", start, end, null);
  };

  for (const piece of pieces) {
    if (piece.type === "break") {
      appendNewline(piece.start, piece.end);
      continue;
    }
    let offset = 0;
    for (const character of piece.text) {
      const start = { node: piece.node, offset };
      offset += character.length;
      const end = { node: piece.node, offset };
      if (character === "\r") continue;
      if (character === "\n" && piece.preformatted) {
        appendNewline(start, end);
      } else if (/\s/u.test(character)) {
        if (!pending) pending = { start, end, link: piece.link };
        else pending.end = end;
      } else {
        flushSpace();
        append(character, start, end, piece.link);
      }
    }
  }
  while (output.at(-1) && /\s/u.test(output.at(-1).character)) output.pop();

  return {
    text: output.map((entry) => entry.character).join(""),
    characters: output,
  };
}

export function logicalLineRanges(text) {
  const characters = Array.from(text);
  const ranges = [];
  let start = 0;
  for (let index = 0; index < characters.length; index += 1) {
    if (characters[index] === "\n") {
      ranges.push({ start, end: index });
      start = index + 1;
    }
  }
  ranges.push({ start, end: characters.length });
  return ranges;
}

export function scalarToUtf16(text, scalar) {
  if (!Number.isSafeInteger(scalar) || scalar < 0) return null;
  const characters = Array.from(text);
  if (scalar > characters.length) return null;
  return characters.slice(0, scalar).join("").length;
}

export function stableSelector(element, doc = element.ownerDocument) {
  if (element.id) {
    const escaped = globalThis.CSS?.escape
      ? CSS.escape(element.id)
      : element.id.replace(/([^a-zA-Z0-9_-])/g, "\\$1");
    try {
      if (doc.querySelectorAll(`#${escaped}`).length === 1) return `#${escaped}`;
    } catch {
      // Fall through to a structural selector.
    }
  }
  const parts = [];
  for (
    let current = element;
    current && current.nodeType === Node.ELEMENT_NODE;
    current = current.parentElement
  ) {
    const tag = current.localName;
    if (!tag) break;
    const siblings = current.parentElement
      ? Array.from(current.parentElement.children).filter(
          (candidate) => candidate.localName === tag,
        )
      : [current];
    const suffix =
      siblings.length > 1
        ? `:nth-of-type(${siblings.indexOf(current) + 1})`
        : "";
    parts.unshift(`${tag}${suffix}`);
    if (tag === "html") break;
  }
  return parts.join(" > ");
}

export function elementSummary(element) {
  const id = element.id ? `#${element.id}` : "";
  const classes = Array.from(element.classList || [])
    .slice(0, 8)
    .map((name) => `.${name}`)
    .join("");
  return `${element.localName}${id}${classes}`.slice(0, 4096);
}

function elementIsVisible(element, view) {
  for (let current = element; current; current = current.parentElement) {
    if (current.hidden || current.localName === "template") return false;
    const style = view.getComputedStyle(current);
    if (
      style.display === "none" ||
      style.visibility === "hidden" ||
      style.visibility === "collapse"
    ) {
      return false;
    }
  }
  return true;
}

function textIsRendered(node, view) {
  if (!node.parentElement || !elementIsVisible(node.parentElement, view)) {
    return false;
  }
  const range = node.ownerDocument.createRange();
  range.selectNodeContents(node);
  return Array.from(range.getClientRects()).some(
    (rect) => rect.width > 0 || rect.height > 0,
  );
}

function nearestOwner(node, view) {
  for (let element = node.parentElement; element; element = element.parentElement) {
    if (element.matches(SEMANTIC_SELECTOR)) return element;
  }
  for (let element = node.parentElement; element; element = element.parentElement) {
    if (BLOCK_DISPLAYS.has(view.getComputedStyle(element).display)) return element;
  }
  return null;
}

function breakOwner(element, view) {
  for (let current = element.parentElement; current; current = current.parentElement) {
    if (current.matches(SEMANTIC_SELECTOR)) return current;
  }
  for (let current = element.parentElement; current; current = current.parentElement) {
    if (BLOCK_DISPLAYS.has(view.getComputedStyle(current).display)) return current;
  }
  return null;
}

function ownerSource(owner) {
  const sourceId = Number(owner.dataset.repSourceId);
  const sourceLine = Number(owner.dataset.repSourceLine);
  if (
    !Number.isSafeInteger(sourceId) ||
    sourceId < 0 ||
    !Number.isSafeInteger(sourceLine) ||
    sourceLine < 1
  ) {
    return null;
  }
  return { sourceId, sourceLine };
}

function originalLink(node, owner) {
  const element =
    node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement;
  const anchor = element?.closest("a,area");
  if (!anchor || !owner.contains(anchor)) return null;
  const original = anchor.getAttribute("data-rep-original-href");
  if (!original) return null;
  try {
    const resolved = new URL(original, owner.ownerDocument.baseURI).href;
    return resolved === original ? original : `${original} → ${resolved}`;
  } catch {
    return original;
  }
}

function orderedListMetadata(owner, listIds) {
  if (owner.localName !== "li") {
    return { listId: null, topLevelOrderedListItem: false };
  }
  const orderedAncestors = [];
  for (let current = owner.parentElement; current; current = current.parentElement) {
    if (current.localName === "ol") orderedAncestors.push(current);
  }
  if (!orderedAncestors.length) {
    return { listId: null, topLevelOrderedListItem: false };
  }
  const root = orderedAncestors.at(-1);
  if (!listIds.has(root)) listIds.set(root, listIds.size + 1);
  return {
    listId: listIds.get(root),
    topLevelOrderedListItem:
      owner.parentElement === root && !root.parentElement?.closest("ol,ul"),
  };
}

function linkRanges(characters) {
  const ranges = [];
  let active = null;
  characters.forEach((entry, index) => {
    if (entry.link !== active?.url) {
      if (active) {
        active.end = index;
        ranges.push(active);
      }
      active = entry.link
        ? { start: index, end: index + 1, url: entry.link }
        : null;
    } else if (active) {
      active.end = index + 1;
    }
  });
  if (active) ranges.push(active);
  return ranges;
}

export function extractDocument(doc) {
  const view = doc.defaultView;
  const groups = [];
  let current = null;
  const walker = doc.createTreeWalker(
    doc.body || doc.documentElement,
    NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT,
  );

  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    let owner = null;
    let piece = null;
    if (node.nodeType === Node.TEXT_NODE) {
      if (!node.data || !textIsRendered(node, view)) continue;
      owner = nearestOwner(node, view);
      if (!owner) continue;
      const preformatted =
        owner.localName === "pre" ||
        /^(pre|pre-wrap|break-spaces)$/.test(
          view.getComputedStyle(node.parentElement).whiteSpace,
        );
      piece = {
        type: "text",
        text: node.data,
        node,
        preformatted,
        link: originalLink(node, owner),
      };
    } else if (
      node.localName === "br" &&
      elementIsVisible(node, view) &&
      node.getClientRects().length
    ) {
      owner = breakOwner(node, view);
      if (!owner) continue;
      const parent = node.parentNode;
      const offset = Array.prototype.indexOf.call(parent.childNodes, node);
      piece = {
        type: "break",
        start: { node: parent, offset },
        end: { node: parent, offset: offset + 1 },
      };
    } else {
      continue;
    }

    if (!ownerSource(owner)) continue;
    if (!current || current.owner !== owner) {
      current = { owner, pieces: [] };
      groups.push(current);
    }
    current.pieces.push(piece);
  }

  const normalized = groups
    .map((group) => ({ ...group, ...normalizePieces(group.pieces) }))
    .filter((group) => group.text.trim().length > 0);
  const ownerTotals = new Map();
  for (const group of normalized) {
    ownerTotals.set(group.owner, (ownerTotals.get(group.owner) || 0) + 1);
  }
  const ownerIndexes = new Map();
  const listIds = new Map();
  const models = normalized.map((group, nodeIndex) => {
    const source = ownerSource(group.owner);
    const occurrence = (ownerIndexes.get(group.owner) || 0) + 1;
    ownerIndexes.set(group.owner, occurrence);
    const heading = /^h[1-6]$/.test(group.owner.localName)
      ? Number(group.owner.localName.slice(1))
      : null;
    const list = orderedListMetadata(group.owner, listIds);
    return {
      nodeIndex,
      owner: group.owner,
      characters: group.characters,
      manifest: {
        sourceId: source.sourceId,
        sourceLine: source.sourceLine,
        tag: group.owner.localName,
        elementSummary: elementSummary(group.owner),
        text: group.text,
        logicalLines: logicalLineRanges(group.text),
        selector: stableSelector(group.owner, doc),
        textFragment:
          ownerTotals.get(group.owner) > 1 ? occurrence : null,
        headingLevel: heading,
        listId: list.listId,
        topLevelOrderedListItem: list.topLevelOrderedListItem,
        links: linkRanges(group.characters),
      },
    };
  });

  return {
    manifest: {
      version: 1,
      nodes: models.map((model) => model.manifest),
    },
    models,
  };
}

export function domRangeForSlice(model, start, end) {
  const count = model.characters.length;
  if (
    !Number.isSafeInteger(start) ||
    !Number.isSafeInteger(end) ||
    start < 0 ||
    start > end ||
    end > count ||
    start === end
  ) {
    return null;
  }
  const range = model.owner.ownerDocument.createRange();
  range.setStart(
    model.characters[start].start.node,
    model.characters[start].start.offset,
  );
  range.setEnd(
    model.characters[end - 1].end.node,
    model.characters[end - 1].end.offset,
  );
  return range;
}

export function textRectsForSlice(model, start, end) {
  if (!domRangeForSlice(model, start, end)) return [];
  const rects = [];
  let run = null;
  const flush = () => {
    if (!run) return;
    const range = model.owner.ownerDocument.createRange();
    range.setStart(run.node, run.start);
    range.setEnd(run.node, run.end);
    for (const rect of range.getClientRects()) {
      if (rect.width > 0 && rect.height > 0) rects.push(rect);
    }
    run = null;
  };
  for (const character of model.characters.slice(start, end)) {
    const textNode =
      character.start.node?.nodeType === Node.TEXT_NODE &&
      character.start.node === character.end.node;
    if (!textNode || /\s/u.test(character.character)) {
      flush();
      continue;
    }
    if (
      run &&
      run.node === character.start.node &&
      run.end === character.start.offset
    ) {
      run.end = character.end.offset;
    } else {
      flush();
      run = {
        node: character.start.node,
        start: character.start.offset,
        end: character.end.offset,
      };
    }
  }
  flush();
  return rects;
}

function pointToScalar(model, domNode, offset) {
  for (let index = 0; index < model.characters.length; index += 1) {
    const character = model.characters[index];
    if (character.start.node !== domNode && character.end.node !== domNode) continue;
    if (
      character.start.node === domNode &&
      character.end.node === domNode &&
      character.start.offset <= offset &&
      offset <= character.end.offset
    ) {
      return index;
    }
    if (character.start.node === domNode && offset <= character.start.offset) {
      return index;
    }
  }
  return null;
}

function scalarAtCoordinates(model, x, y) {
  let nearest = null;
  for (let index = 0; index < model.characters.length; index += 1) {
    const range = domRangeForSlice(model, index, index + 1);
    if (!range) continue;
    for (const rect of range.getClientRects()) {
      if (
        rect.left <= x &&
        x <= rect.right &&
        rect.top <= y &&
        y <= rect.bottom
      ) {
        return index;
      }
      const distance = Math.abs(rect.left - x) + Math.abs(rect.top - y);
      if (!nearest || distance < nearest.distance) nearest = { index, distance };
    }
  }
  return nearest?.index ?? null;
}

export function selectionPoint(models, doc, x, y, target = null) {
  let point = null;
  if (doc.caretPositionFromPoint) {
    const position = doc.caretPositionFromPoint(x, y);
    if (position) point = { node: position.offsetNode, offset: position.offset };
  } else if (doc.caretRangeFromPoint) {
    const range = doc.caretRangeFromPoint(x, y);
    if (range) point = { node: range.startContainer, offset: range.startOffset };
  }
  if (!point) return null;
  for (const model of models) {
    if (
      model.owner === point.node ||
      model.owner.contains(
        point.node.nodeType === Node.ELEMENT_NODE
          ? point.node
          : point.node.parentElement,
      )
    ) {
      const scalar = pointToScalar(model, point.node, point.offset);
      if (scalar !== null) return { node: model.nodeIndex, scalar };
    }
  }
  const targetElement =
    target?.nodeType === Node.ELEMENT_NODE ? target : target?.parentElement;
  for (const model of models) {
    if (
      targetElement &&
      (model.owner === targetElement || model.owner.contains(targetElement))
    ) {
      const scalar = scalarAtCoordinates(model, x, y);
      if (scalar !== null) return { node: model.nodeIndex, scalar };
    }
  }
  return null;
}

const FOCUS_PADDING_PX = 5;
const FOCUS_INLINE_GAP_PX = 12;

export function focusScrimRects(rects, viewport) {
  const width = Number(viewport?.width);
  const height = Number(viewport?.height);
  if (
    !Number.isFinite(width) ||
    !Number.isFinite(height) ||
    width <= 0 ||
    height <= 0
  ) {
    return [];
  }
  const holes = rects
    .map((rect) => {
      const rawLeft = Number(rect?.left);
      const rawTop = Number(rect?.top);
      const rawRight = Number.isFinite(Number(rect?.right))
        ? Number(rect.right)
        : rawLeft + Number(rect?.width);
      const rawBottom = Number.isFinite(Number(rect?.bottom))
        ? Number(rect.bottom)
        : rawTop + Number(rect?.height);
      if (
        ![rawLeft, rawTop, rawRight, rawBottom].every(Number.isFinite)
      ) {
        return null;
      }
      return {
        left: Math.max(0, rawLeft - FOCUS_PADDING_PX),
        top: Math.max(0, rawTop - FOCUS_PADDING_PX),
        right: Math.min(width, rawRight + FOCUS_PADDING_PX),
        bottom: Math.min(height, rawBottom + FOCUS_PADDING_PX),
      };
    })
    .filter(
      (rect) =>
        rect &&
        rect.left < rect.right &&
        rect.top < rect.bottom,
    );
  if (!holes.length) return [];

  const yEdges = Array.from(
    new Set([0, height, ...holes.flatMap((rect) => [rect.top, rect.bottom])]),
  ).sort((left, right) => left - right);
  const scrims = [];
  for (let index = 0; index < yEdges.length - 1; index += 1) {
    const top = yEdges[index];
    const bottom = yEdges[index + 1];
    if (bottom <= top) continue;
    const intervals = holes
      .filter((rect) => rect.top < bottom && rect.bottom > top)
      .map((rect) => ({ left: rect.left, right: rect.right }))
      .sort((left, right) => left.left - right.left);
    let cursor = 0;
    for (const interval of intervals) {
      if (interval.left > cursor + FOCUS_INLINE_GAP_PX) {
        scrims.push({
          left: cursor,
          top,
          width: interval.left - cursor,
          height: bottom - top,
        });
      }
      cursor = Math.max(cursor, interval.right);
    }
    if (cursor < width) {
      scrims.push({
        left: cursor,
        top,
        width: width - cursor,
        height: bottom - top,
      });
    }
  }
  return scrims;
}

export class SelectionOverlay {
  constructor(doc, models) {
    this.doc = doc;
    this.models = models;
    this.host = doc.createElement("div");
    this.host.dataset.repOverlay = "true";
    for (const [property, value] of [
      ["all", "initial"],
      ["display", "block"],
      ["position", "fixed"],
      ["inset", "0"],
      ["pointer-events", "none"],
      ["z-index", "2147483647"],
    ]) {
      this.host.style.setProperty(property, value, "important");
    }
    (doc.body || doc.documentElement).append(this.host);
    const shadow = this.host.attachShadow({ mode: "open" });
    const style = doc.createElement("style");
    style.textContent = `
      :host { all: initial !important; }
      .focus-scrim {
        backdrop-filter: grayscale(.28) saturate(.5) brightness(.58);
        background: rgb(15 23 42 / 34%);
        box-sizing: border-box;
        position: fixed;
      }
      .selection {
        backdrop-filter: brightness(1.28) saturate(1.65);
        background: color-mix(in srgb, #6366f1 48%, transparent);
        border: 3px solid color-mix(in srgb, #4338ca 88%, CanvasText);
        border-radius: 5px;
        box-sizing: border-box;
        box-shadow:
          0 0 0 2px color-mix(in srgb, Canvas 92%, transparent),
          0 0 0 7px color-mix(in srgb, #818cf8 42%, transparent),
          0 5px 18px rgb(30 27 75 / 46%);
        outline: 2px solid color-mix(in srgb, #c7d2fe 88%, transparent);
        outline-offset: 2px;
        position: fixed;
      }
      .selection.focus-start::after {
        background: color-mix(in srgb, #312e81 92%, CanvasText);
        border: 1px solid color-mix(in srgb, Canvas 92%, transparent);
        border-radius: 999px;
        bottom: 1px;
        box-shadow:
          0 0 0 3px color-mix(in srgb, #818cf8 48%, transparent),
          0 0 16px 4px color-mix(in srgb, #6366f1 44%, transparent);
        content: "";
        left: -10px;
        position: absolute;
        top: 1px;
        width: 6px;
      }
      .annotation {
        border-bottom: 3px solid;
        box-sizing: border-box;
        position: fixed;
      }
      .change {
        background: repeating-linear-gradient(
          45deg,
          rgb(34 197 94 / 18%) 0 3px,
          transparent 3px 7px
        );
        border-color: #16a34a;
      }
      .feedback {
        background: radial-gradient(circle, rgb(202 138 4 / 30%) 1px, transparent 2px)
          0 0 / 7px 7px;
        border-color: #ca8a04;
      }
      .insertBefore {
        background: repeating-linear-gradient(
          90deg,
          rgb(14 165 233 / 19%) 0 2px,
          transparent 2px 7px
        );
        border-color: #0284c7;
      }
      .insertAfter {
        background: repeating-linear-gradient(
          0deg,
          rgb(14 165 233 / 19%) 0 2px,
          transparent 2px 7px
        );
        border-color: #0369a1;
      }
      .strike {
        background:
          linear-gradient(to bottom right, transparent 47%, #dc2626 48% 52%, transparent 53%);
        border-color: #dc2626;
      }
      .badge {
        align-items: center;
        background: Canvas;
        border: 1px solid currentColor;
        border-radius: 999px;
        color: CanvasText;
        display: flex;
        font: 700 9px/1 ui-sans-serif, system-ui, sans-serif;
        height: 14px;
        justify-content: center;
        left: 0;
        min-width: 14px;
        padding: 1px 3px;
        position: absolute;
        top: -15px;
      }
    `;
    this.layer = doc.createElement("div");
    shadow.append(style, this.layer);
    this.selection = [];
  }

  paint(selection, annotations = [], scroll = false) {
    this.selection = selection;
    const firstModel = selection
      .map((slice) => this.models[slice.node])
      .find(Boolean);
    if (scroll && firstModel) {
      firstModel.owner.scrollIntoView({ block: "center", inline: "nearest" });
    }
    this.layer.replaceChildren();
    if (firstModel) {
      const focusRects = selection.flatMap((slice) => {
        const model = this.models[slice.node];
        return model
          ? textRectsForSlice(model, slice.start, slice.end)
          : [];
      });
      const view = this.doc.defaultView;
      for (const rect of focusScrimRects(focusRects, {
        width: view.innerWidth,
        height: view.innerHeight,
      })) {
        const scrim = this.doc.createElement("div");
        scrim.className = "focus-scrim";
        scrim.style.left = `${rect.left}px`;
        scrim.style.top = `${rect.top}px`;
        scrim.style.width = `${rect.width}px`;
        scrim.style.height = `${rect.height}px`;
        this.layer.append(scrim);
      }
    }
    const badges = {
      change: "C",
      feedback: "F",
      insertBefore: "B",
      insertAfter: "A",
      strike: "×",
    };
    for (const slice of annotations) {
      this.paintSlice(
        slice,
        `annotation ${slice.kind}`,
        slice.first ? badges[slice.kind] : null,
      );
    }
    let focusPainted = false;
    for (const slice of selection) {
      const model = this.models[slice.node];
      if (!model) continue;
      const painted = this.paintSlice(
        slice,
        "selection",
        null,
        !focusPainted,
      );
      focusPainted ||= painted > 0;
    }
  }

  paintSlice(slice, className, badge = null, markFocus = false) {
    const model = this.models[slice.node];
    if (!model) return 0;
    const rects = textRectsForSlice(
      model,
      slice.start,
      slice.end,
    );
    for (const [index, rect] of rects.entries()) {
      const marker = this.doc.createElement("div");
      marker.className =
        markFocus && index === 0 ? `${className} focus-start` : className;
      marker.style.left = `${rect.left}px`;
      marker.style.top = `${rect.top}px`;
      marker.style.width = `${rect.width}px`;
      marker.style.height = `${rect.height}px`;
      if (badge && index === 0) {
        const label = this.doc.createElement("span");
        label.className = "badge";
        label.textContent = badge;
        marker.append(label);
      }
      this.layer.append(marker);
    }
    return rects.length;
  }
}
