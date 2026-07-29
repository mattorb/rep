import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("embedded parent assets remain external-script compatible", async () => {
  const html = await readFile(new URL("../../../src/web/app.html", import.meta.url), "utf8");
  assert.match(html, /sandbox="allow-same-origin"/);
  assert.match(html, /src="app\.js"/);
  assert.doesNotMatch(html, /<script>[^<]/);
});
