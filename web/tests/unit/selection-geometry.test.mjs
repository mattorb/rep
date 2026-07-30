import assert from "node:assert/strict";
import test from "node:test";

import { CHAR_WIDTH, LINE_HEIGHT, buildDocument, el, txt } from "./dom-stub.mjs";
import {
  SelectionOverlay,
  domRangeForSlice,
  extractDocument,
  focusSelectionRect,
  selectionPoint,
  textRectsForSlice,
} from "../../../src/web/document.js";

// "Ship it" lays out as one 10px-wide box per character on a single 20px line,
// so a rect assertion reads directly as a character span.
function planDocument() {
  const doc = buildDocument([
    el("p", { source: 1 }, txt("Ship it")),
    el("p", { source: 2 }, txt("Then ship"), el("br"), txt("again")),
  ]);
  return { doc, models: extractDocument(doc).models };
}

function boxes(rects) {
  return rects.map((rect) => [rect.left, rect.top, rect.width, rect.height]);
}

test("slice ranges reject out-of-bounds and empty spans", () => {
  const { models } = planDocument();
  const [model] = models;

  assert.equal(domRangeForSlice(model, 0, 0), null);
  assert.equal(domRangeForSlice(model, -1, 3), null);
  assert.equal(domRangeForSlice(model, 3, 2), null);
  assert.equal(domRangeForSlice(model, 0, model.characters.length + 1), null);
  assert.equal(domRangeForSlice(model, 1.5, 3), null);
  assert.notEqual(domRangeForSlice(model, 0, model.characters.length), null);
});

test("text rects coalesce runs and break at whitespace", () => {
  const { models } = planDocument();

  assert.deepEqual(boxes(textRectsForSlice(models[0], 0, 7)), [
    [0, 0, 4 * CHAR_WIDTH, LINE_HEIGHT],
    [5 * CHAR_WIDTH, 0, 2 * CHAR_WIDTH, LINE_HEIGHT],
  ]);
  assert.deepEqual(boxes(textRectsForSlice(models[0], 1, 4)), [
    [CHAR_WIDTH, 0, 3 * CHAR_WIDTH, LINE_HEIGHT],
  ]);
  assert.deepEqual(textRectsForSlice(models[0], 4, 5), []);
  assert.deepEqual(textRectsForSlice(models[0], 0, 0), []);
});

test("line breaks contribute no rects because they are not text positions", () => {
  const { models } = planDocument();
  const model = models[1];
  const newline = model.characters.findIndex(
    (character) => character.character === "\n",
  );

  assert.ok(newline > 0);
  assert.deepEqual(textRectsForSlice(model, newline, newline + 1), []);
  // "Then", "ship", and "again" each keep their own rect across the break.
  assert.equal(textRectsForSlice(model, 0, model.characters.length).length, 3);
});

test("caret positions map to scalars inside the owning model", () => {
  const { doc, models } = planDocument();
  const textNode = models[0].owner.childNodes[0];
  doc.caretPositionFromPoint = () => ({ offsetNode: textNode, offset: 0 });

  assert.deepEqual(selectionPoint(models, doc, 0, 0), { node: 0, scalar: 0 });

  doc.caretPositionFromPoint = () => ({ offsetNode: textNode, offset: 3 });
  assert.deepEqual(selectionPoint(models, doc, 0, 0), { node: 0, scalar: 2 });
});

test("a caret before trimmed leading whitespace maps to the first character", () => {
  const doc = buildDocument([el("p", { source: 1 }, txt("  Ship"))]);
  const { models } = extractDocument(doc);
  const textNode = models[0].owner.childNodes[0];
  doc.caretPositionFromPoint = () => ({ offsetNode: textNode, offset: 0 });

  assert.equal(models[0].characters[0].start.offset, 2);
  assert.deepEqual(selectionPoint(models, doc, 0, 0), { node: 0, scalar: 0 });
});

test("a caret on an element that holds no characters resolves to nothing", () => {
  const doc = buildDocument([el("p", { source: 1 }, el("em", {}, txt("Ship")))]);
  const { models } = extractDocument(doc);
  const emphasis = models[0].owner.children[0];
  doc.caretPositionFromPoint = () => ({ offsetNode: emphasis, offset: 0 });

  assert.equal(selectionPoint(models, doc, 0, 0), null);
});

test("caretRangeFromPoint is used when caretPositionFromPoint is absent", () => {
  const { doc, models } = planDocument();
  const textNode = models[1].owner.childNodes[0];
  doc.caretRangeFromPoint = () => ({
    startContainer: textNode,
    startOffset: 0,
  });

  assert.deepEqual(selectionPoint(models, doc, 0, 0), { node: 1, scalar: 0 });
});

test("no caret API and no match both resolve to no selection point", () => {
  const { doc, models } = planDocument();

  assert.equal(selectionPoint(models, doc, 0, 0), null);

  doc.caretPositionFromPoint = () => null;
  assert.equal(selectionPoint(models, doc, 0, 0), null);

  doc.caretPositionFromPoint = () => ({ offsetNode: doc.body, offset: 0 });
  assert.equal(selectionPoint(models, doc, 0, 0), null);
});

test("a caret outside every model falls back to the target's coordinates", () => {
  const { doc, models } = planDocument();
  doc.caretPositionFromPoint = () => ({ offsetNode: doc.body, offset: 0 });
  const target = models[0].owner;

  assert.deepEqual(selectionPoint(models, doc, 5, 10, target), {
    node: 0,
    scalar: 0,
  });
  assert.deepEqual(selectionPoint(models, doc, 55, 10, target), {
    node: 0,
    scalar: 5,
  });
  // Far outside the text: the nearest character wins rather than nothing.
  assert.deepEqual(selectionPoint(models, doc, 10_000, 10_000, target), {
    node: 0,
    scalar: 6,
  });
});

test("focus rects clamp to the viewport and reject unusable input", () => {
  const viewport = { width: 100, height: 100 };

  assert.deepEqual(
    focusSelectionRect([{ left: 30, top: 30, width: 20, height: 20 }], viewport),
    { left: 20, top: 20, right: 60, bottom: 60, width: 40, height: 40 },
  );
  assert.equal(focusSelectionRect([{ left: 10, top: 10 }], viewport), null);
  assert.equal(
    focusSelectionRect([{ left: 10, top: 10, right: 5, bottom: 20 }], viewport),
    null,
  );
  assert.equal(
    focusSelectionRect([{ left: 500, top: 10, right: 600, bottom: 20 }], viewport),
    null,
  );
  assert.equal(focusSelectionRect([{ left: 1, top: 1, right: 2, bottom: 2 }], {
    width: 0,
    height: 100,
  }), null);
  assert.equal(focusSelectionRect([{ left: 1, top: 1, right: 2, bottom: 2 }], {
    width: "wide",
    height: 100,
  }), null);
});

test("the overlay isolates itself in a shadow root above the page", () => {
  const { doc, models } = planDocument();
  const overlay = new SelectionOverlay(doc, models);

  assert.equal(doc.body.childNodes.at(-1), overlay.host);
  assert.equal(overlay.host.dataset.repOverlay, "true");
  assert.equal(overlay.host.style.position, "fixed");
  assert.equal(overlay.host.style["z-index"], "2147483647");
  assert.equal(overlay.host.style["pointer-events"], "none");
  assert.equal(overlay.host.style["all:priority"], "important");
  assert.deepEqual(
    overlay.host.shadowRoot.children.map((child) => child.localName),
    ["style", "div"],
  );
  assert.match(overlay.host.shadowRoot.children[0].textContent, /focus-scrim/);
});

test("painting draws one scrim, the annotations, then the selection marker", () => {
  const { doc, models } = planDocument();
  const overlay = new SelectionOverlay(doc, models);

  overlay.paint(
    [{ node: 0, start: 0, end: 7 }],
    [{ node: 0, start: 0, end: 4, kind: "change", first: true }],
    true,
  );

  assert.deepEqual(models[0].owner.scrolledIntoView, {
    block: "center",
    inline: "nearest",
  });
  assert.deepEqual(
    overlay.layer.children.map((child) => child.className),
    ["focus-scrim", "annotation change", "selection"],
  );

  const [scrim, annotation, selection] = overlay.layer.children;
  assert.match(scrim.style.clipPath, /^polygon\(evenodd, /);
  assert.equal(annotation.style.left, "0px");
  assert.equal(annotation.style.width, `${4 * CHAR_WIDTH}px`);
  assert.deepEqual(
    annotation.children.map((child) => [child.className, child.textContent]),
    [["badge", "C"]],
  );
  // The selection marker is the padded union of the slice's text rects.
  assert.equal(selection.style.left, "0px");
  assert.equal(selection.style.width, `${7 * CHAR_WIDTH + 10}px`);
  assert.equal(overlay.selection.length, 1);
});

test("a selection with no painted text draws no scrim or marker", () => {
  const { doc, models } = planDocument();
  const overlay = new SelectionOverlay(doc, models);

  overlay.paint([{ node: 0, start: 4, end: 5 }], [
    { node: 0, start: 0, end: 4, kind: "feedback", first: false },
  ]);

  assert.deepEqual(
    overlay.layer.children.map((child) => child.className),
    ["annotation feedback"],
  );
  assert.deepEqual(overlay.layer.children[0].children, []);
  assert.equal(models[0].owner.scrolledIntoView, null);
});

test("painting an unknown node index is a no-op", () => {
  const { doc, models } = planDocument();
  const overlay = new SelectionOverlay(doc, models);

  assert.equal(overlay.paintSlice({ node: 99, start: 0, end: 1 }, "selection"), 0);
  overlay.paint([{ node: 99, start: 0, end: 1 }]);
  assert.deepEqual(overlay.layer.children, []);
});

test("repainting replaces the previous markers", () => {
  const { doc, models } = planDocument();
  const overlay = new SelectionOverlay(doc, models);

  overlay.paint([{ node: 0, start: 0, end: 7 }]);
  overlay.paint([{ node: 1, start: 0, end: 4 }]);

  assert.deepEqual(
    overlay.layer.children.map((child) => child.className),
    ["focus-scrim", "selection"],
  );
});
