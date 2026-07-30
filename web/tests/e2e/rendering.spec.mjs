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

test("@rendering demo viewport keeps the command legend in two contained rows", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1120, height: 620 });
  const { running } = await openPlan(page, "layout.html");
  try {
    const hud = page.locator("#review-hud");
    const modeBox = await hud.locator("#mode").boundingBox();
    const commandBoxes = await hud
      .locator(".review-hud-command")
      .evaluateAll((commands) =>
        commands.map((command) => {
          const box = command.getBoundingClientRect();
          return { bottom: box.bottom, top: box.top };
        }),
      );
    expect(commandBoxes).toHaveLength(7);
    const commandRows = new Set(
      commandBoxes.map((box) => Math.round(box.top)),
    );
    expect(commandRows.size).toBe(2);
    expect(
      Math.abs(
        (commandBoxes[0].top + commandBoxes[0].bottom) / 2 -
          (modeBox.y + modeBox.height / 2),
      ),
    ).toBeLessThanOrEqual(1);
    expect(
      await page.evaluate(
        () =>
          document.documentElement.scrollWidth <=
          document.documentElement.clientWidth,
      ),
    ).toBe(true);
  } finally {
    await finishRep(page, running);
  }
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
        await expect(hud).toHaveCSS(
          "grid-template-columns",
          /.+ .+/,
        );
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

test("@gallery focus spotlight modes", async ({ page }) => {
  await page.setViewportSize({ width: 1120, height: 780 });
  const { running } = await openPlan(page, "semantic.html");
  try {
    const capture = async (mode) => {
      await expect
        .poll(() => page.evaluate(() => window.__repTest.state.mode))
        .toBe(mode);
      await page.screenshot({
        path: `gallery/focus-${mode}.png`,
        fullPage: true,
      });
    };
    await page.keyboard.press("j");
    await page.keyboard.press("j");
    await expect
      .poll(() => page.evaluate(() => window.__repTest.state.anchor.node))
      .toBe(2);
    await capture("sentence");
    await page.keyboard.press("Space");
    await capture("word");
    await page.keyboard.press("Backspace");
    await page.keyboard.press("Backspace");
    await capture("line");
    await page.keyboard.press("Backspace");
    await capture("paragraph");
    await page.keyboard.press("Backspace");
    await capture("section");
  } finally {
    await finishRep(page, running);
  }
});
