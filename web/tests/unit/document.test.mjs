import assert from "node:assert/strict";
import test from "node:test";

import {
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
