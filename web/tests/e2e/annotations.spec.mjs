import { expect, test } from "@playwright/test";

import { finishRep, openPlan, waitForRep } from "./helpers.mjs";

async function state(page) {
  return page.evaluate(() => window.__repTest.state);
}

async function saveModal(page, key, text) {
  await page.keyboard.press(key);
  await expect(page.locator("#modal")).toBeVisible();
  await page.locator("#modal-input").fill(text);
  await page.locator("#modal-input").press("Enter");
  await expect(page.locator("#modal")).not.toBeVisible();
}

test("@annotations search, help, outline, links, and annotation jumps use the shared session", async ({
  page,
}) => {
  const { frame, running } = await openPlan(page, "semantic.html");
  try {
    await page.keyboard.press("/");
    await page.locator("#modal-input").fill("terminal workflow");
    await page.locator("#modal-input").press("Enter");
    await expect.poll(() => state(page)).toMatchObject({
      mode: "sentence",
      anchor: { node: 2, unit: "sentence", unitIndex: 1 },
    });
    await expect(page.locator("#status")).toContainText("Match 1/1");
    await page.keyboard.press("n");
    await expect(page.locator("#status")).toContainText("Match 1/1");
    await page.keyboard.press("N");
    await expect(page.locator("#status")).toContainText("Match 1/1");

    await page.keyboard.press("O");
    await expect(page.locator("#modal")).toBeVisible();
    await expect(page.locator("#modal-content")).toContainText("guide.html");
    await expect(page.locator("#modal-content")).toContainText(
      /guide\.html → http:\/\/127\.0\.0\.1:/,
    );
    await page.keyboard.press("O");
    await expect(page.locator("#modal")).not.toBeVisible();

    await page.keyboard.press("I");
    await expect(page.locator("#modal-content")).toContainText("Delivery Plan");
    await expect(page.locator("#modal-content")).toContainText("Verification");
    await expect(page.locator("#modal-content")).toContainText("h1#delivery");
    await expect(page.locator("#modal-content")).toContainText("line 15");
    await expect(page.locator("#modal-content")).toContainText("p");
    await page.keyboard.press("I");
    await expect(page.locator("#modal")).not.toBeVisible();

    await page.keyboard.press("?");
    await expect(page.locator("#modal-content")).toContainText("change / feedback");
    await page.keyboard.press("?");
    await expect(page.locator("#modal")).not.toBeVisible();

    await saveModal(page, "c", "Make the compatibility promise explicit.");
    expect((await state(page)).annotationCount).toBe(1);
    await page.keyboard.press("j");
    await saveModal(page, "f", "Explain the browser fallback.");
    await page.keyboard.press("[");
    await expect.poll(() => state(page)).toMatchObject({
      anchor: { node: 2, unit: "sentence" },
    });
    await page.keyboard.press("]");
    await expect(page.locator("#status")).toContainText("Annotated node 4");
    await page.keyboard.press("]");
    await expect(page.locator("#status")).toContainText("No annotated nodes after");

    await expect
      .poll(() =>
        frame
          .locator("[data-rep-overlay]")
          .evaluate(
            (host) =>
              host.shadowRoot.querySelectorAll(".annotation").length,
          ),
      )
      .toBeGreaterThanOrEqual(2);
    const annotationPresentation = await frame
      .locator("[data-rep-overlay]")
      .evaluate((host) => {
        const annotations = Array.from(
          host.shadowRoot.querySelectorAll(".annotation"),
        );
        return {
          backgrounds: annotations.map(
            (annotation) => getComputedStyle(annotation).backgroundImage,
          ),
          badges: Array.from(
            host.shadowRoot.querySelectorAll(".annotation .badge"),
            (badge) => badge.textContent,
          ),
        };
      });
    expect(annotationPresentation.badges).toEqual(expect.arrayContaining(["C", "F"]));
    expect(new Set(annotationPresentation.backgrounds).size).toBeGreaterThan(1);

    const beforeReload = await state(page);
    await page.reload();
    await page.waitForFunction(
      (revision) => window.__repTest?.state?.revision === revision,
      beforeReload.revision,
    );
    expect((await state(page)).annotationCount).toBe(2);
  } finally {
    await finishRep(page, running);
  }
});

test("@annotations create, edit, clear, strike, and copy work", async ({
  page,
  context,
  browserName,
}) => {
  if (browserName !== "chromium") {
    await page.addInitScript(() => {
      Object.defineProperty(navigator, "clipboard", {
        configurable: true,
        value: {
          writeText(text) {
            window.__repClipboardText = text;
            return Promise.resolve();
          },
        },
      });
    });
  }
  const { running } = await openPlan(page, "unicode.html");
  try {
    await saveModal(page, "c", "Use a clearer heading.");
    await saveModal(page, "f", "Keep the Unicode examples.");
    await page.keyboard.press("e");
    await expect(page.locator("#modal-input")).toHaveValue(
      "Keep the Unicode examples.",
    );
    await page.locator("#modal-input").fill("Keep every Unicode example.");
    await page.locator("#modal-input").press("Enter");
    await expect(page.locator("#modal")).not.toBeVisible();
    await saveModal(page, "b", "Context before.");
    await saveModal(page, "a", "Context after.");

    const countBeforeClear = (await state(page)).annotationCount;
    await page.keyboard.press("x");
    await expect.poll(async () => (await state(page)).annotationCount).toBe(
      countBeforeClear - 1,
    );
    await page.keyboard.press("x");
    await page.keyboard.press("x");
    await expect
      .poll(async () =>
        (await state(page)).annotations.some(
          (annotation) => annotation.kind === "strike",
        ),
      )
      .toBe(true);

    if (browserName === "chromium") {
      await context.grantPermissions(["clipboard-read", "clipboard-write"], {
        origin: new URL(page.url()).origin,
      });
    }
    await page.keyboard.press("r");
    await expect(page.locator("#status")).toContainText("Copied");
    const copied = await page.evaluate((usesSystemClipboard) => {
      if (usesSystemClipboard) return navigator.clipboard.readText();
      return window.__repClipboardText;
    }, browserName === "chromium");
    expect(copied).toContain("FORMAT: html");

  } finally {
    await finishRep(page, running);
  }
  expect(running.output()).toBe("");
});

test("@annotations denied clipboard access exposes selectable output fallback", async ({
  page,
}) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText() {
          return Promise.reject(new DOMException("denied", "NotAllowedError"));
        },
      },
    });
  });
  const { running } = await openPlan(page, "semantic.html");
  try {
    await saveModal(page, "c", "Use explicit release criteria.");
    await page.keyboard.press("r");
    await expect(page.locator("#modal")).toBeVisible();
    await expect(page.locator("#modal-title")).toHaveText("Copy action output");
    await expect(page.locator(".copy-output")).toHaveValue(/FORMAT: html/);
    await expect(page.locator("#modal-confirm")).toHaveText("Copy");
    await expect(page.locator(".copy-output")).toHaveAttribute("readonly", "");
  } finally {
    await finishRep(page, running);
  }
});

test("@annotations finish emits one HTML action protocol and closes the listener", async ({
  page,
}) => {
  const { running } = await openPlan(page, "unicode.html");
  expect(running.output()).toBe("");

  await saveModal(page, "c", "Rename this heading.");
  await page.keyboard.press("j");
  await saveModal(page, "f", "Preserve all scripts and diacritics.");
  await page.keyboard.press("j");
  await saveModal(page, "b", "Add a short setup note.");
  await saveModal(page, "a", "Add a short outcome note.");
  await page.keyboard.press("j");
  await expect.poll(() => state(page)).toMatchObject({
    anchor: { node: 2, unit: "sentence" },
  });
  await page.keyboard.press("x");
  await expect
    .poll(async () =>
      (await state(page)).annotations.some(
        (annotation) => annotation.kind === "strike",
      ),
    )
    .toBe(true);

  await page.keyboard.press("q");
  await expect(page.locator("#modal")).toBeVisible();
  await expect(page.locator("#modal-title")).toHaveText("Send feedback?");
  await expect(page.locator("#modal-confirm")).toHaveText("Send");
  await page.locator("#modal-confirm").click();
  await expect(page.locator("#completion")).toBeVisible();
  await expect(page.locator("#completion-title")).toHaveText(
    "Sending feedback to Rep skill",
  );
  await expect(page.locator("#completion-message")).toContainText(
    "Feedback received",
  );
  const output = await waitForRep(running);

  expect(output).toContain("FILE:");
  expect(output).toContain("FORMAT: html");
  expect(output).toContain("ACTION: change");
  expect(output).toContain("ACTION: revise-to-incorporate-feedback");
  expect(output).toContain("ACTION: insert-before");
  expect(output).toContain("ACTION: insert-after");
  expect(output).toContain("ACTION: delete this");
  expect(output).toMatch(/LOCATOR: (#[^\n]+|html > body[^\n]*)/);
  expect(output.match(/FORMAT: html/g)).toHaveLength(1);
});

test("@annotations uppercase Q silently discards without confirmation", async ({
  page,
}) => {
  const { running } = await openPlan(page, "layout.html");
  await saveModal(page, "c", "Change this.");
  await page.keyboard.press("Shift+Q");
  await expect(page.locator("#completion")).toBeVisible();
  await expect(page.locator("#completion-message")).toContainText(
    "Review discarded",
  );
  expect(await waitForRep(running)).toBe("");
});

test("@annotations no-action q handoff emits the HTML no-actions form", async ({
  page,
}) => {
  const { running } = await openPlan(page, "empty.html");
  await page.keyboard.press("q");
  await expect(page.locator("#modal-title")).toHaveText("Send feedback?");
  await page.locator("#modal-confirm").click();
  await expect(page.locator("#completion")).toBeVisible();
  await expect(page.locator("#completion-message")).toContainText(
    "Feedback received",
  );
  const output = await waitForRep(running);
  expect(output).toContain("FORMAT: html");
  expect(output).toContain("No actions.");
  expect(output).not.toContain("ACTION:");
});
