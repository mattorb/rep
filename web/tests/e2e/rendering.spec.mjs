import { readFile } from "node:fs/promises";
import { expect, test } from "@playwright/test";

import { finishRep, fixture, openPlan } from "./helpers.mjs";

test("@rendering preserves original linked CSS, SVG, and layout", async ({ page }) => {
  const { frame, running } = await openPlan(page, "layout.html");
  try {
    await expect(frame.locator("h1")).toHaveText("Styled Implementation Plan");
    await expect(frame.locator("main.board")).toHaveCSS("max-width", "1024px");
    await expect(frame.locator(".columns")).toHaveCSS("display", "grid");
    await expect(frame.locator("header")).toHaveCSS("display", "flex");
    await expect
      .poll(() => frame.locator("header img").evaluate((image) => image.naturalWidth))
      .toBeGreaterThan(0);
    await expect(page.locator(".toolbar")).toHaveCSS("display", "grid");
    await expect(page.locator("#submit, #discard")).toHaveCount(0);
  } finally {
    await finishRep(page, running);
  }
});

test("@rendering blocks document code and unsafe resources without rewriting source", async ({
  page,
}) => {
  const original = await readFile(fixture("security.html"), "utf8");
  const remoteRequests = [];
  page.on("request", (request) => {
    if (!request.url().startsWith("http://127.0.0.1:")) {
      remoteRequests.push(request.url());
    }
  });
  const { frame, running } = await openPlan(page, "security.html");
  try {
    await expect(frame.locator("h1")).toHaveText("Security Boundary");
    await expect(frame.locator("script")).toHaveCount(0);
    await expect(frame.locator("iframe, object, base")).toHaveCount(0);
    await expect(frame.locator("[onload], [onclick]")).toHaveCount(0);
    await expect(frame.locator("form")).not.toHaveAttribute("action");
    await expect(frame.locator("a")).not.toHaveAttribute("href");
    expect(
      await frame.locator("body").evaluate(() => ({
        click: window.repClickHandlerRan,
        document: window.repDocumentScriptRan,
        load: window.repEventHandlerRan,
      })),
    ).toEqual({ click: undefined, document: undefined, load: undefined });
    expect(remoteRequests).toEqual([]);
    await expect(page.locator("#status")).toContainText("unsafe resource");
  } finally {
    await finishRep(page, running);
  }
  expect(await readFile(fixture("security.html"), "utf8")).toBe(original);
});

for (const viewport of [
  { name: "desktop", width: 1440, height: 1000 },
  { name: "narrow", width: 430, height: 860 },
]) {
  test(`@gallery original layout ${viewport.name}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    const { running } = await openPlan(page, "layout.html");
    try {
      const hud = page.locator("#review-hud");
      const hudBox = await hud.boundingBox();
      const viewportSize = page.viewportSize();
      expect(Math.abs(hudBox.x)).toBeLessThanOrEqual(1);
      expect(Math.abs(hudBox.width - viewportSize.width)).toBeLessThanOrEqual(1);
      expect(
        Math.abs(viewportSize.height - hudBox.y - hudBox.height),
      ).toBeLessThanOrEqual(1);
      expect(
        await page.evaluate(
          () =>
            document.documentElement.scrollWidth <=
              document.documentElement.clientWidth &&
            document.documentElement.scrollHeight <=
              document.documentElement.clientHeight,
        ),
      ).toBe(true);
      if (viewport.name === "narrow") {
        const modeBox = await hud.locator("#mode").boundingBox();
        const commandBox = await hud
          .locator(".review-hud-command")
          .first()
          .boundingBox();
        expect(commandBox.y).toBeGreaterThan(modeBox.y);
      }
      await page.screenshot({
        path: `gallery/layout-${viewport.name}.png`,
        fullPage: true,
      });
    } finally {
      await finishRep(page, running);
    }
  });
}
