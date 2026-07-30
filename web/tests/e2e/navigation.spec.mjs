import { readFile } from "node:fs/promises";
import { expect, test } from "@playwright/test";

import { finishRep, fixture, openPlan } from "./helpers.mjs";

async function browserState(page) {
  return page.evaluate(() => window.__repTest.state);
}

async function expectSelectionSpotlight(frame) {
  const geometry = await frame
    .locator("[data-rep-overlay]")
    .evaluate((host) => {
      const scrims = Array.from(
        host.shadowRoot.querySelectorAll(".focus-scrim"),
        (element) => {
          const rect = element.getBoundingClientRect();
          return {
            bottom: rect.bottom,
            left: rect.left,
            right: rect.right,
            top: rect.top,
          };
        },
      );
      const style = getComputedStyle(
        host.shadowRoot.querySelector(".focus-scrim"),
      );
      return {
        backdropFilter: style.backdropFilter,
        background: style.backgroundColor,
        clipPath: style.clipPath,
        scrimCount: scrims.length,
        selectionCount:
          host.shadowRoot.querySelectorAll(".selection").length,
      };
    });
  expect(geometry.scrimCount).toBe(1);
  expect(geometry.selectionCount).toBe(1);
  expect(geometry.background).not.toBe("rgba(0, 0, 0, 0)");
  expect(geometry.backdropFilter).not.toBe("none");
  expect(geometry.clipPath).toContain("evenodd");
}

async function clickPlanElement(page, selector, clickCount, xOffset) {
  const point = await page.evaluate((selector) => {
    const iframe = document.querySelector("#plan");
    const element = iframe.contentDocument.querySelector(selector);
    const iframeRect = iframe.getBoundingClientRect();
    const elementRect = element.getBoundingClientRect();
    return {
      x: iframeRect.left + elementRect.left,
      y: iframeRect.top + elementRect.top + 8,
    };
  }, selector);
  for (let index = 0; index < clickCount; index += 1) {
    await page.mouse.click(point.x + xOffset, point.y);
  }
}

for (const name of [
  "layout",
  "malformed",
  "security",
  "section-cards",
  "semantic",
  "unicode",
  "visibility",
  "empty",
]) {
  test(`@navigation manifest matches ${name} acceptance fixture`, async ({
    page,
  }) => {
    const expected = JSON.parse(
      await readFile(fixture(`${name}.expected.json`), "utf8"),
    );
    const { running } = await openPlan(page, `${name}.html`);
    try {
      const manifest = await page.evaluate(() => window.__repTest.manifest);
      const visible = manifest.nodes.map((node) => node.text);
      if (expected.visible_text) expect(visible).toEqual(expected.visible_text);
      if (expected.visible_text_contains) {
        for (const text of expected.visible_text_contains) {
          expect(visible).toContain(text);
        }
      }
      for (const text of expected.hidden_text || []) {
        expect(visible).not.toContain(text);
      }
      if (expected.section_headings) {
        expect(
          manifest.nodes
            .filter((node) => node.headingLevel)
            .map((node) => node.text),
        ).toEqual(expected.section_headings);
      }
      if (expected.unique_locators) {
        expect(
          manifest.nodes
            .filter((node) => expected.unique_locators.includes(node.selector))
            .map((node) => node.selector),
        ).toEqual(expected.unique_locators);
      }
      if (name === "empty") {
        await expect(page.locator("#review-hud")).toBeVisible();
        await expect(page.locator("#mode")).toHaveText("Mode: No selection");
      }
    } finally {
      await finishRep(page, running);
    }
  });
}

test("@navigation keyboard units, boundaries, focus, and reload are authoritative", async ({
  page,
}) => {
  const { frame, running } = await openPlan(page, "semantic.html");
  try {
    expect(await browserState(page)).toMatchObject({
      revision: 1,
      mode: "section",
      anchor: { node: 0, unit: "section", unitIndex: 0 },
    });
    const hud = page.locator("#review-hud");
    await expect(hud).toBeVisible();
    await expect(hud.locator("#mode")).toHaveText("Mode: section", {
      ignoreCase: true,
    });
    await expectSelectionSpotlight(frame);
    await expect(hud.locator(".review-hud-command")).toHaveText([
      "Spacebar = change mode",
      "j/k = next/prev",
      "x = strike",
      "c = change literal",
      "f = feedback intent",
      "b/a = insert before/after",
      "q = submit & quit",
      "? = help",
    ]);
    const hudBox = await hud.boundingBox();
    const viewport = page.viewportSize();
    const hudTypography = await hud.evaluate((element) => ({
      borderRadius: getComputedStyle(element).borderRadius,
      commandFontSize: getComputedStyle(
        element.querySelector(".review-hud-command"),
      ).fontSize,
      modeFontSize: getComputedStyle(element.querySelector("#mode")).fontSize,
    }));
    expect(Math.abs(hudBox.x)).toBeLessThanOrEqual(1);
    expect(Math.abs(hudBox.width - viewport.width)).toBeLessThanOrEqual(1);
    expect(
      Math.abs(viewport.height - hudBox.y - hudBox.height),
    ).toBeLessThanOrEqual(1);
    expect(hudTypography.borderRadius).toBe("0px");
    expect(Number.parseFloat(hudTypography.modeFontSize)).toBeGreaterThanOrEqual(
      23,
    );
    expect(
      Number.parseFloat(hudTypography.commandFontSize),
    ).toBeGreaterThanOrEqual(16);

    await page.keyboard.press("j");
    await expect.poll(() => browserState(page)).toMatchObject({
      revision: 2,
      anchor: { node: 1, unit: "section" },
    });
    await page.keyboard.press("Space");
    await expect.poll(() => browserState(page)).toMatchObject({
      revision: 3,
      mode: "paragraph",
      anchor: { node: 1, unit: "paragraph", unitIndex: 0 },
    });
    await expect(hud.locator("#mode")).toHaveText("Mode: paragraph", {
      ignoreCase: true,
    });
    await expectSelectionSpotlight(frame);
    await page.keyboard.press("Space");
    await expect.poll(() => browserState(page)).toMatchObject({
      revision: 4,
      mode: "line",
    });
    await expectSelectionSpotlight(frame);
    await page.keyboard.press("Space");
    await expect.poll(() => browserState(page)).toMatchObject({
      revision: 5,
      mode: "sentence",
    });
    await expectSelectionSpotlight(frame);
    await page.keyboard.press("Space");
    await expect.poll(() => browserState(page)).toMatchObject({
      revision: 6,
      mode: "word",
    });
    await expectSelectionSpotlight(frame);

    await page.locator("#interaction-layer").focus();
    await page.keyboard.press("Backspace");
    await expect.poll(() => browserState(page)).toMatchObject({
      revision: 7,
      mode: "sentence",
    });
    await expectSelectionSpotlight(frame);
    await page.keyboard.press("Backspace");
    await expect.poll(() => browserState(page)).toMatchObject({
      revision: 8,
      mode: "line",
    });
    await expectSelectionSpotlight(frame);
    await page.keyboard.press("Backspace");
    await expect.poll(() => browserState(page)).toMatchObject({
      revision: 9,
      mode: "paragraph",
    });
    await expectSelectionSpotlight(frame);
    await page.keyboard.press("Backspace");
    await expect.poll(() => browserState(page)).toMatchObject({
      revision: 10,
      mode: "section",
      anchor: { node: 1, unit: "section" },
    });
    await expectSelectionSpotlight(frame);
    await page.keyboard.press("j");
    await expect.poll(() => browserState(page)).toMatchObject({
      revision: 11,
      anchor: { node: 7, unit: "section" },
    });
    await expect(hud.locator("#mode")).toHaveText("Mode: section", {
      ignoreCase: true,
    });

    const beforeReload = await browserState(page);
    await page.reload();
    await page.waitForFunction(
      (revision) => window.__repTest?.state?.revision === revision,
      beforeReload.revision,
    );
    expect(await browserState(page)).toMatchObject({
      revision: beforeReload.revision,
      anchor: beforeReload.anchor,
      mode: beforeReload.mode,
    });
  } finally {
    await finishRep(page, running);
  }
});

test("@navigation closing and reopening the review URL restores server state", async ({
  page,
  context,
}) => {
  const { running } = await openPlan(page, "semantic.html");
  let reopened = null;
  const reviewUrl = page.url();
  try {
    await page.keyboard.press("j");
    await expect.poll(() => browserState(page)).toMatchObject({
      revision: 2,
      mode: "section",
      anchor: { node: 1, unit: "section", unitIndex: 0 },
    });
    const beforeClose = await browserState(page);
    await page.close();

    reopened = await context.newPage();
    await reopened.goto(reviewUrl);
    await reopened.waitForFunction(
      (revision) => window.__repTest?.state?.revision === revision,
      beforeClose.revision,
    );
    expect(await browserState(reopened)).toMatchObject({
      revision: beforeClose.revision,
      anchor: beforeClose.anchor,
      mode: beforeClose.mode,
    });
  } finally {
    if (!reopened && page.isClosed()) {
      reopened = await context.newPage();
      await reopened.goto(reviewUrl);
    }
    await finishRep(reopened || page, running);
  }
});

test("@navigation mouse selection, logical lines, Unicode, and overlays remain aligned", async ({
  page,
}) => {
  const { frame, running } = await openPlan(page, "unicode.html");
  try {
    await clickPlanElement(page, "#second", 1, 12);
    await expect
      .poll(() =>
        page.evaluate(() => window.__repTest.clickEvents.slice(-1)),
      )
      .toMatchObject([
        { clickCount: 1, node: 2, status: "selected", unit: "word" },
      ]);
    await expect.poll(() => browserState(page)).toMatchObject({
      mode: "word",
      anchor: { node: 2, unit: "word" },
    });
    await clickPlanElement(page, "#second", 2, 28);
    await expect
      .poll(() =>
        page.evaluate(() => window.__repTest.clickEvents.slice(-2)),
      )
      .toMatchObject([
        { clickCount: 1, node: 2, status: "selected", unit: "word" },
        { clickCount: 2, node: 2, status: "selected", unit: "sentence" },
      ]);
    await expect.poll(() => browserState(page)).toMatchObject({
      mode: "sentence",
      anchor: { node: 2, unit: "sentence" },
    });
    await clickPlanElement(page, "#second", 3, 44);
    await expect
      .poll(() =>
        page.evaluate(() => window.__repTest.clickEvents.slice(-3)),
      )
      .toMatchObject([
        { clickCount: 1, node: 2, status: "selected", unit: "word" },
        { clickCount: 2, node: 2, status: "selected", unit: "sentence" },
        { clickCount: 3, node: 2, status: "selected", unit: "paragraph" },
      ]);
    await expect.poll(() => browserState(page)).toMatchObject({
      mode: "paragraph",
      anchor: { node: 2, unit: "paragraph" },
    });

    await page.setViewportSize({ width: 520, height: 220 });
    await expect
      .poll(() =>
        frame
          .locator("[data-rep-overlay]")
          .evaluate(
            (host) =>
              host.shadowRoot.querySelectorAll(".selection").length,
          ),
      )
      .toBeGreaterThan(0);
    const beforeScroll = await frame
      .locator("[data-rep-overlay]")
      .evaluate((host) => ({
        scrollY: host.ownerDocument.defaultView.scrollY,
        top: host.shadowRoot
          .querySelector(".selection")
          .getBoundingClientRect().top,
      }));
    const layer = await page.locator("#interaction-layer").boundingBox();
    await page.mouse.move(layer.x + layer.width / 2, layer.y + layer.height / 2);
    await page.mouse.wheel(0, 120);
    await expect
      .poll(() =>
        frame
          .locator("[data-rep-overlay]")
          .evaluate((host) => host.ownerDocument.defaultView.scrollY),
      )
      .toBeGreaterThan(beforeScroll.scrollY);
    await page.evaluate(
      () => new Promise((resolve) => requestAnimationFrame(resolve)),
    );
    const afterScroll = await frame
      .locator("[data-rep-overlay]")
      .evaluate((host) => ({
        scrollY: host.ownerDocument.defaultView.scrollY,
        top: host.shadowRoot
          .querySelector(".selection")
          .getBoundingClientRect().top,
      }));
    expect(
      Math.abs(
        beforeScroll.top -
          afterScroll.top -
          (afterScroll.scrollY - beforeScroll.scrollY),
      ),
    ).toBeLessThan(2);
    const selectedText = await frame
      .locator("#second")
      .evaluate((element) => element.textContent);
    expect(selectedText).toContain("🚀");
  } finally {
    await finishRep(page, running);
  }
});

test("@navigation selection overlay paints one rounded focus region", async ({
  page,
}) => {
  const { frame, running } = await openPlan(page, "semantic.html");
  try {
    await page.keyboard.press("j");
    await page.keyboard.press("Space");
    await page.keyboard.press("Space");
    await page.keyboard.press("Space");
    await expect.poll(() => browserState(page)).toMatchObject({
      mode: "sentence",
      anchor: { node: 1, unit: "sentence" },
    });
    const geometry = await frame
      .locator("[data-rep-overlay]")
      .evaluate((host) => {
        const scrim = host.shadowRoot.querySelector(".focus-scrim");
        const scrimStyle = getComputedStyle(scrim);
        const marker = host.shadowRoot.querySelector(".selection");
        const markerRect = marker.getBoundingClientRect();
        const markerStyle = getComputedStyle(marker);
        const selected = host.ownerDocument.querySelector("#delivery");
        const range = host.ownerDocument.createRange();
        range.selectNodeContents(selected);
        const textRects = Array.from(range.getClientRects()).filter(
          (rect) => rect.width > 0 && rect.height > 0,
        );
        return {
          marker: {
            background: markerStyle.backgroundColor,
            borderRadius: markerStyle.borderRadius,
            borderWidth: markerStyle.borderTopWidth,
            boxShadow: markerStyle.boxShadow,
            containsSelection: textRects.every(
              (rect) =>
                markerRect.left <= rect.left &&
                markerRect.top <= rect.top &&
                markerRect.right >= rect.right &&
                markerRect.bottom >= rect.bottom,
            ),
          },
          markerCount:
            host.shadowRoot.querySelectorAll(".selection").length,
          scrim: {
            backdropFilter: scrimStyle.backdropFilter,
            background: scrimStyle.backgroundColor,
            position: scrimStyle.position,
          },
        };
      });
    expect(geometry.markerCount).toBe(1);
    expect(geometry.scrim).toMatchObject({
      position: "fixed",
    });
    expect(geometry.scrim.background).not.toBe("rgba(0, 0, 0, 0)");
    expect(geometry.scrim.backdropFilter).not.toBe("none");
    expect(geometry.marker).toMatchObject({
      borderRadius: "18px",
      borderWidth: "2px",
      containsSelection: true,
    });
    expect(geometry.marker.background).not.toBe("rgba(0, 0, 0, 0)");
    expect(geometry.marker.boxShadow).not.toBe("none");
  } finally {
    await finishRep(page, running);
  }
});

test("@navigation adjacent card sections keep their own lead-in badges", async ({
  page,
}) => {
  const { frame, running } = await openPlan(page, "section-cards.html");
  try {
    const manifest = await page.evaluate(() => window.__repTest.manifest);
    expect(
      manifest.nodes
        .filter((node) => node.headingLevel)
        .map((node) => ({
          sectionStart: node.sectionStart,
          text: node.text,
        })),
    ).toEqual([
      { sectionStart: 0, text: "Instrument" },
      { sectionStart: 3, text: "Shadow" },
    ]);
    expect(await browserState(page)).toMatchObject({
      mode: "section",
      anchor: { node: 1, unit: "section" },
      selection: [{ node: 0 }, { node: 1 }, { node: 2 }],
    });

    const geometry = await frame
      .locator("[data-rep-overlay]")
      .evaluate((host) => {
        const marker = host.shadowRoot.querySelector(".selection");
        const markerRect = marker.getBoundingClientRect();
        const firstCard = host.ownerDocument.querySelector("#card-one");
        const secondCard = host.ownerDocument.querySelector("#card-two");
        const firstRect = firstCard.getBoundingClientRect();
        const secondRect = secondCard.getBoundingClientRect();
        return {
          markerLeft: markerRect.left,
          markerRight: markerRect.right,
          firstLeft: firstRect.left,
          secondLeft: secondRect.left,
        };
      });
    expect(geometry.markerLeft).toBeGreaterThanOrEqual(geometry.firstLeft);
    expect(geometry.markerRight).toBeLessThan(geometry.secondLeft);
  } finally {
    await finishRep(page, running);
  }
});

test("@navigation logical lines do not depend on viewport wrapping", async ({
  page,
}) => {
  const { running } = await openPlan(page, "semantic.html");
  try {
    const before = await page.evaluate(() =>
      window.__repTest.manifest.nodes.map((node) => node.logicalLines),
    );
    await page.setViewportSize({ width: 360, height: 600 });
    const after = await page.evaluate(() =>
      window.__repTest.manifest.nodes.map((node) => node.logicalLines),
    );
    expect(after).toEqual(before);
    expect(before.some((ranges) => ranges.length === 2)).toBe(true);
  } finally {
    await finishRep(page, running);
  }
});
