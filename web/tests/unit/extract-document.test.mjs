import assert from "node:assert/strict";
import test from "node:test";

import { buildDocument, el, txt } from "./dom-stub.mjs";
import { extractDocument, stableSelector } from "../../../src/web/document.js";

function texts(manifest) {
  return manifest.nodes.map((node) => node.text);
}

test("manifest nodes carry the full shape the Rust validator consumes", () => {
  const doc = buildDocument([
    el("h1", { source: [1, 3], id: "plan" }, txt("Delivery Plan")),
    el("p", { source: [2, 5], class: ["lead", "intro"] }, txt("  Ship it.  ")),
  ]);

  const { manifest, models } = extractDocument(doc);

  assert.equal(manifest.version, 1);
  assert.deepEqual(manifest.nodes, [
    {
      sourceId: 1,
      sourceLine: 3,
      tag: "h1",
      elementSummary: "h1#plan",
      text: "Delivery Plan",
      logicalLines: [{ start: 0, end: 13 }],
      selector: "#plan",
      textFragment: null,
      headingLevel: 1,
      listId: null,
      topLevelOrderedListItem: false,
      links: [],
    },
    {
      sourceId: 2,
      sourceLine: 5,
      tag: "p",
      elementSummary: "p.lead.intro",
      text: "Ship it.",
      logicalLines: [{ start: 0, end: 8 }],
      selector: "html > body > p",
      textFragment: null,
      headingLevel: null,
      listId: null,
      topLevelOrderedListItem: false,
      links: [],
    },
  ]);
  assert.deepEqual(
    models.map((model) => [model.nodeIndex, model.owner.localName, model.characters.length]),
    [
      [0, "h1", 13],
      [1, "p", 8],
    ],
  );
});

test("heading levels are captured for every rank", () => {
  const doc = buildDocument(
    [1, 2, 3, 4, 5, 6].map((level) =>
      el(`h${level}`, { source: level }, txt(`Rank ${level}`)),
    ),
  );

  const { manifest } = extractDocument(doc);

  assert.deepEqual(
    manifest.nodes.map((node) => node.headingLevel),
    [1, 2, 3, 4, 5, 6],
  );
});

test("br and preformatted newlines both become logical lines", () => {
  const doc = buildDocument([
    el(
      "p",
      { source: 4 },
      txt("Run browser tests."),
      el("br"),
      txt("Run the Rust build."),
    ),
    el("pre", { source: 6 }, txt("first line\nsecond line")),
  ]);

  const { manifest } = extractDocument(doc);

  assert.deepEqual(texts(manifest), [
    "Run browser tests.\nRun the Rust build.",
    "first line\nsecond line",
  ]);
  assert.deepEqual(manifest.nodes[0].logicalLines, [
    { start: 0, end: 18 },
    { start: 19, end: 38 },
  ]);
  assert.deepEqual(manifest.nodes[1].logicalLines, [
    { start: 0, end: 10 },
    { start: 11, end: 22 },
  ]);
});

test("white-space: pre-wrap on an ancestor keeps newlines outside pre", () => {
  const doc = buildDocument([
    el(
      "div",
      { style: { whiteSpace: "pre-wrap" } },
      el("p", { source: 7 }, el("span", {}, txt("kept\nbreak"))),
    ),
  ]);

  assert.deepEqual(texts(extractDocument(doc).manifest), ["kept\nbreak"]);
});

test("elements without source markers are skipped entirely", () => {
  const doc = buildDocument([
    el("p", {}, txt("No markers here.")),
    el("p", { source: 8 }, txt("Kept.")),
  ]);

  assert.deepEqual(texts(extractDocument(doc).manifest), ["Kept."]);
});

test("hidden, collapsed, templated, and unrendered text never reaches the manifest", () => {
  const doc = buildDocument([
    el("p", { source: 9, hidden: true }, txt("Hidden by attribute")),
    el("p", { source: 10, style: { display: "none" } }, txt("Hidden by display")),
    el(
      "div",
      { style: { visibility: "hidden" } },
      el("p", { source: 11 }, txt("Hidden by ancestor")),
    ),
    el(
      "div",
      { style: { visibility: "collapse" } },
      el("p", { source: 12 }, txt("Collapsed by ancestor")),
    ),
    el("template", {}, el("p", { source: 13 }, txt("Template content"))),
    el("p", { source: 14 }, txt("Zero size", { renders: false })),
    el("p", { source: 15 }, txt("   ")),
    el("p", { source: 16 }, txt("Visible")),
  ]);

  assert.deepEqual(texts(extractDocument(doc).manifest), ["Visible"]);
});

test("text with no semantic ancestor falls back to the nearest block owner", () => {
  const doc = buildDocument([
    el("div", { source: 17 }, el("span", {}, txt("Loose text in a block"))),
    // The nearest block ancestor here is the unmarked body, so the text is
    // dropped rather than attributed to an owner with no source location.
    el("span", { source: 18 }, txt("Inline-only text has no owner")),
  ]);

  const { manifest } = extractDocument(doc);

  assert.deepEqual(
    manifest.nodes.map((node) => [node.tag, node.text]),
    [["div", "Loose text in a block"]],
  );
});

test("a break with no semantic ancestor is owned by the nearest block", () => {
  const doc = buildDocument([
    el("div", { source: 19 }, el("span", {}, txt("above"), el("br"), txt("below"))),
  ]);

  const { manifest } = extractDocument(doc);

  assert.deepEqual(manifest.nodes.map((node) => [node.tag, node.text]), [
    ["div", "above\nbelow"],
  ]);
});

test("text and breaks with no block ancestor at all are dropped", () => {
  const doc = buildDocument([
    el("span", { source: 26 }, txt("above"), el("br"), txt("below")),
  ]);
  doc.body.styleOverrides = { display: "inline" };
  doc.documentElement.styleOverrides = { display: "inline" };

  assert.deepEqual(extractDocument(doc).manifest.nodes, []);
});

test("ordered list identity spans nested lists and marks only top-level items", () => {
  const doc = buildDocument([
    el(
      "ol",
      {},
      el("li", { source: 20 }, txt("First")),
      el(
        "li",
        { source: 21 },
        txt("Second"),
        el("ol", {}, el("li", { source: 22 }, txt("Nested"))),
      ),
    ),
    el("ol", {}, el("li", { source: 23 }, txt("Other list"))),
    el("ul", {}, el("li", { source: 24 }, txt("Unordered"))),
  ]);

  const { manifest } = extractDocument(doc);

  assert.deepEqual(
    manifest.nodes.map((node) => [
      node.text,
      node.listId,
      node.topLevelOrderedListItem,
    ]),
    [
      ["First", 1, true],
      ["Second", 1, true],
      ["Nested", 1, false],
      ["Other list", 2, true],
      ["Unordered", null, false],
    ],
  );
});

test("an ordered list nested under a list is not top level", () => {
  const doc = buildDocument([
    el("ul", {}, el("li", {}, el("ol", {}, el("li", { source: 25 }, txt("Deep"))))),
  ]);

  const { manifest } = extractDocument(doc);

  assert.equal(manifest.nodes[0].listId, 1);
  assert.equal(manifest.nodes[0].topLevelOrderedListItem, false);
});

test("link ranges use scalar offsets and show the resolved target", () => {
  const doc = buildDocument([
    el(
      "p",
      { source: 30 },
      txt("See "),
      el("a", { attrs: { "data-rep-original-href": "./spec.html" } }, txt("the spec")),
      txt(" now."),
    ),
  ]);

  const { manifest } = extractDocument(doc);

  assert.equal(manifest.nodes[0].text, "See the spec now.");
  assert.deepEqual(manifest.nodes[0].links, [
    {
      start: 4,
      end: 12,
      url: "./spec.html → https://plan.test/spec.html",
    },
  ]);
});

test("already-absolute and unresolvable hrefs are reported verbatim", () => {
  const doc = buildDocument([
    el(
      "p",
      { source: 31 },
      el(
        "a",
        { attrs: { "data-rep-original-href": "https://example.test/x" } },
        txt("absolute"),
      ),
    ),
    el(
      "p",
      { source: 32 },
      el("a", { attrs: { "data-rep-original-href": "http://[" } }, txt("broken")),
    ),
    el("p", { source: 33 }, el("a", {}, txt("unmarked"))),
  ]);

  const { manifest } = extractDocument(doc);

  assert.deepEqual(
    manifest.nodes.map((node) => node.links.map((link) => link.url)),
    [["https://example.test/x"], ["http://["], []],
  );
});

test("an anchor outside the owner does not colour the owner's text", () => {
  const doc = buildDocument([
    el(
      "a",
      { attrs: { "data-rep-original-href": "./outer.html" } },
      el("p", { source: 34 }, txt("Wrapped block")),
    ),
  ]);

  assert.deepEqual(extractDocument(doc).manifest.nodes[0].links, []);
});

test("an owner split by a nested semantic block gets numbered text fragments", () => {
  const doc = buildDocument([
    el(
      "ul",
      {},
      el(
        "li",
        { source: 40 },
        txt("Before "),
        el("ul", {}, el("li", { source: 41 }, txt("inner"))),
        txt(" after"),
      ),
    ),
  ]);

  const { manifest } = extractDocument(doc);

  assert.deepEqual(
    manifest.nodes.map((node) => [node.sourceId, node.text, node.textFragment]),
    [
      [40, "Before", 1],
      [41, "inner", null],
      [40, "after", 2],
    ],
  );
});

test("logical line ranges count Unicode scalars, not UTF-16 units", () => {
  const doc = buildDocument([el("p", { source: 60 }, txt("café 🚀 ok"))]);

  const { manifest } = extractDocument(doc);

  assert.equal(manifest.nodes[0].text, "café 🚀 ok");
  assert.deepEqual(manifest.nodes[0].logicalLines, [{ start: 0, end: 9 }]);
});

test("astral characters keep link ranges aligned with logical lines", () => {
  const doc = buildDocument([
    el(
      "p",
      { source: 61 },
      txt("🚀 "),
      el("a", { attrs: { "data-rep-original-href": "https://example.test/" } }, txt("go")),
    ),
  ]);

  const { manifest } = extractDocument(doc);

  assert.equal(manifest.nodes[0].text, "🚀 go");
  assert.deepEqual(manifest.nodes[0].links, [
    { start: 2, end: 4, url: "https://example.test/" },
  ]);
});

test("a document with no body still walks the document element", () => {
  const doc = buildDocument([el("p", { source: 70 }, txt("Body-less"))]);
  const body = doc.body;
  doc.body = null;

  assert.deepEqual(texts(extractDocument(doc).manifest), ["Body-less"]);
  doc.body = body;
});

test("an empty document yields an empty manifest", () => {
  assert.deepEqual(extractDocument(buildDocument([])).manifest, {
    version: 1,
    nodes: [],
  });
});

test("malformed source markers are rejected rather than coerced", () => {
  const doc = buildDocument([
    el("p", { dataset: { repSourceId: "abc", repSourceLine: "4" } }, txt("bad id")),
    el("p", { dataset: { repSourceId: "-1", repSourceLine: "4" } }, txt("negative id")),
    el("p", { dataset: { repSourceId: "1", repSourceLine: "0" } }, txt("zero line")),
    el("p", { dataset: { repSourceId: "1.5", repSourceLine: "4" } }, txt("fractional id")),
    el("p", { source: [2, 4] }, txt("good")),
  ]);

  assert.deepEqual(texts(extractDocument(doc).manifest), ["good"]);
});

test("stable selectors prefer a unique id and fall back to structure", () => {
  const doc = buildDocument([
    el("section", { id: "unique" }, el("p", { source: 1 }, txt("a"))),
    el("section", { id: "dup" }, el("p", { source: 2 }, txt("b"))),
    el("section", { id: "dup" }, el("p", { source: 3 }, txt("c"))),
  ]);
  const [unique, firstDup, secondDup] = doc.querySelectorAll("section");

  assert.equal(stableSelector(unique, doc), "#unique");
  assert.equal(
    stableSelector(firstDup, doc),
    "html > body > section:nth-of-type(2)",
  );
  assert.equal(
    stableSelector(secondDup, doc),
    "html > body > section:nth-of-type(3)",
  );
});

test("ids needing escapes survive without CSS.escape and with it", () => {
  const doc = buildDocument([el("p", { id: "step 1", source: 1 }, txt("a"))]);
  const [paragraph] = doc.querySelectorAll("p");
  const previous = globalThis.CSS;

  try {
    delete globalThis.CSS;
    assert.equal(stableSelector(paragraph, doc), "#step\\ 1");

    globalThis.CSS = { escape: (value) => value.replace(/ /gu, "\\ ") };
    assert.equal(stableSelector(paragraph, doc), "#step\\ 1");
  } finally {
    if (previous === undefined) delete globalThis.CSS;
    else globalThis.CSS = previous;
  }
});

test("an id that cannot be queried falls back to a structural selector", () => {
  const doc = buildDocument([el("p", { id: "boom", source: 1 }, txt("a"))]);
  const [paragraph] = doc.querySelectorAll("p");
  doc.querySelectorAll = () => {
    throw new Error("invalid selector");
  };

  assert.equal(stableSelector(paragraph, doc), "html > body > p");
});

test("element summaries are bounded and selectors stop at the root", () => {
  const doc = buildDocument([
    el(
      "div",
      { class: ["a"] },
      el("div", { class: ["b"] }, el("p", { source: 1 }, txt("deep"))),
    ),
  ]);

  const { manifest } = extractDocument(doc);

  assert.equal(manifest.nodes[0].selector, "html > body > div > div > p");
  assert.equal(manifest.nodes[0].elementSummary, "p");
});
