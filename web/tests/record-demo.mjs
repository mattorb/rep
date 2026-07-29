import { once } from "node:events";
import { spawn } from "node:child_process";
import { mkdir, rename } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { chromium } from "@playwright/test";

const repo = path.resolve(import.meta.dirname, "../..");
const output = path.resolve(repo, process.argv[2] || "docs/rep-web-demo.webm");
const videoDirectory = path.join(repo, "target", "web-demo-video");
await mkdir(path.dirname(output), { recursive: true });
await mkdir(videoDirectory, { recursive: true });

const child = spawn(
  path.join(repo, "target", "debug", "rep"),
  ["--web", "--no-open", path.join(repo, "examples", "demo-plan.html")],
  { cwd: repo, stdio: ["ignore", "pipe", "pipe"] },
);
let diagnostics = "";
child.stderr.setEncoding("utf8");
child.stderr.on("data", (chunk) => {
  diagnostics += chunk;
});

const url = await waitForUrl();
const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({
  recordVideo: { dir: videoDirectory, size: { width: 1280, height: 800 } },
  viewport: { width: 1280, height: 800 },
});
const page = await context.newPage();
const video = page.video();

try {
  await page.goto(url);
  await page.waitForFunction(() => window.__repTest?.state?.status === "ready");
  await pause(1_000);
  await page.keyboard.press("Space");
  await page.keyboard.press("j");
  await pause(800);
  await page.keyboard.press("c");
  await page.locator("#modal-input").fill("Make the release conditional on the completed browser checklist.");
  await pause(800);
  await page.locator("#modal-input").press("Enter");
  await pause(1_000);
  await page.keyboard.press("I");
  await pause(1_000);
  await page.keyboard.press("Escape");
  await page.locator("#submit").click();
  await page.locator("#completion").waitFor();
  await pause(1_000);
} finally {
  await context.close();
  await browser.close();
  if (child.exitCode === null) {
    child.kill("SIGTERM");
    await once(child, "exit");
  }
}

if (child.exitCode !== 0) {
  throw new Error(`Rep exited with ${child.exitCode}:\n${diagnostics}`);
}
await rename(await video.path(), output);
process.stdout.write(`Recorded ${output}\n`);

async function waitForUrl() {
  const deadline = Date.now() + 8_000;
  while (Date.now() < deadline) {
    const match = diagnostics.match(/^Review URL: (http:\/\/[^\s]+)$/m);
    if (match) return match[1];
    if (child.exitCode !== null) {
      throw new Error(`Rep exited before launch (${child.exitCode}):\n${diagnostics}`);
    }
    await pause(50);
  }
  child.kill("SIGTERM");
  throw new Error(`Timed out waiting for Rep URL:\n${diagnostics}`);
}

function pause(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
