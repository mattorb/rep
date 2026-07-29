import { once } from "node:events";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { expect, test } from "@playwright/test";

import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../../..");
const binary = path.join(repo, "target", "debug", "rep");
const runner = path.join(
  repo,
  ".agents",
  "skills",
  "rep",
  "scripts",
  "run_rep_and_capture.sh",
);

test("@skill bundled runner launches HTML review, captures output, and supports a verified edit", async ({
  page,
}) => {
  const temporary = await mkdtemp(path.join(os.tmpdir(), "rep-skill-e2e-"));
  const plan = path.join(temporary, "plan with spaces.html");
  const original = `<!doctype html>
<html><body>
  <h1 id="title">Original plan</h1>
  <p>Keep <strong>inline markup</strong> intact.</p>
</body></html>
`;
  await writeFile(plan, original);
  const child = spawn(runner, [plan, "--web", "--no-open"], {
    cwd: repo,
    env: {
      ...process.env,
      REP_BIN: binary,
      REP_CAPTURE_DIR: temporary,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stderr = "";
  let stdout = "";
  child.stderr.setEncoding("utf8");
  child.stdout.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  child.stdout.on("data", (chunk) => {
    stdout += chunk;
  });

  try {
    const url = await expect
      .poll(() => stderr.match(/^Review URL: (http:\/\/[^\s]+)$/m)?.[1])
      .not.toBeUndefined()
      .then(() => stderr.match(/^Review URL: (http:\/\/[^\s]+)$/m)[1]);
    await page.goto(url);
    await page.waitForFunction(
      () => window.__repTest?.state?.status === "ready",
    );
    await page.keyboard.press("c");
    await page.locator("#modal-input").fill("Revised plan");
    await page.locator("#modal-input").press("Enter");
    await expect(page.locator("#modal")).not.toBeVisible();

    await page.locator("#submit").click();
    await expect(page.locator("#completion")).toBeVisible();
    if (child.exitCode === null) await once(child, "exit");
    expect(child.exitCode).toBe(0);

    const capturePath = stderr
      .split("\n")
      .find((line) => line.startsWith("REP_CAPTURE_FILE="))
      ?.slice("REP_CAPTURE_FILE=".length);
    expect(capturePath).toBeTruthy();
    const capture = await readFile(capturePath, "utf8");
    expect(capture).toBe(stdout);
    expect(capture).toContain("FORMAT: html");
    expect(capture).toContain("LOCATOR: #title");
    expect(capture).toContain('target: "Original plan"');
    expect(capture).toContain('CHANGE: "Revised plan"');

    const edited = original.replace(
      '<h1 id="title">Original plan</h1>',
      '<h1 id="title">Revised plan</h1>',
    );
    await writeFile(plan, edited);
    expect(await readFile(plan, "utf8")).toContain(
      '<h1 id="title">Revised plan</h1>',
    );
    expect(await readFile(plan, "utf8")).toContain(
      "<strong>inline markup</strong>",
    );
    await page.setContent(edited);
    await expect(page.locator("#title")).toHaveText("Revised plan");
  } finally {
    if (child.exitCode === null) child.kill("SIGTERM");
    await rm(temporary, { recursive: true, force: true });
  }
});
