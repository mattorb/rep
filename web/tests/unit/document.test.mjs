import assert from "node:assert/strict";
import test from "node:test";

import {
  elementSummary,
  focusScrimRects,
  logicalLineRanges,
  normalizePieces,
  scalarToUtf16,
} from "../../../src/web/document.js";

function textPiece(text, preformatted = false) {
  return {
    type: "text",
    text,
    node: { name: text },
    preformatted,
    link: null,
  };
}

test("normalization collapses layout whitespace but retains explicit lines", () => {
  const normalized = normalizePieces([
    textPiece("  First \t phrase "),
    {
      type: "break",
      start: { node: {}, offset: 0 },
      end: { node: {}, offset: 1 },
    },
    textPiece(" second  phrase "),
  ]);
  assert.equal(normalized.text, "First phrase\nsecond phrase");
  assert.deepEqual(logicalLineRanges(normalized.text), [
    { start: 0, end: 12 },
    { start: 13, end: 26 },
  ]);
});

test("preformatted newlines and Unicode scalar conversion are stable", () => {
  const normalized = normalizePieces([
    textPiece("café 🚀\nnext", true),
  ]);
  assert.equal(normalized.text, "café 🚀\nnext");
  assert.equal(scalarToUtf16(normalized.text, 5), 5);
  assert.equal(scalarToUtf16(normalized.text, 6), 7);
  assert.equal(scalarToUtf16(normalized.text, 99), null);
});

test("element summaries retain tag, id, and bounded class context", () => {
  assert.equal(
    elementSummary({
      localName: "section",
      id: "delivery",
      classList: ["board", "priority"],
    }),
    "section#delivery.board.priority",
  );
  assert.equal(
    elementSummary({
      localName: "p",
      id: "",
      classList: Array.from({ length: 10 }, (_, index) => `c${index}`),
    }),
    "p.c0.c1.c2.c3.c4.c5.c6.c7",
  );
});

test("focus scrims dim the viewport complement without covering selected text", () => {
  const scrims = focusScrimRects(
    [
      { left: 20, top: 30, right: 40, bottom: 50 },
      { left: 46, top: 30, right: 70, bottom: 50 },
    ],
    { width: 100, height: 80 },
  );
  const contains = (rect, x, y) =>
    rect.left <= x &&
    x < rect.left + rect.width &&
    rect.top <= y &&
    y < rect.top + rect.height;

  assert.ok(scrims.length > 1);
  assert.equal(scrims.some((rect) => contains(rect, 30, 40)), false);
  assert.equal(scrims.some((rect) => contains(rect, 43, 40)), false);
  assert.equal(scrims.some((rect) => contains(rect, 10, 40)), true);
  assert.equal(scrims.some((rect) => contains(rect, 90, 70)), true);
  assert.deepEqual(focusScrimRects([], { width: 100, height: 80 }), []);
  assert.deepEqual(focusScrimRects([], { width: 0, height: 80 }), []);
});
