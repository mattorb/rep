import { spawnSync } from "node:child_process";
import {
  access,
  mkdir,
  readFile,
  rename,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";
import { chromium } from "@playwright/test";

const REQUIRED_GATE =
  "Launch at 10% only after recovered-cart deltas remain below 0.5% for 24 hours.";
const ORIGINAL_GATE =
  "Launch to all customers as soon as integration tests pass.";
const ORIGINAL_OWNERSHIP =
  "The checkout platform group will monitor failures after launch.";

export function extractReviewUrl(diagnostics) {
  return diagnostics.match(/^Review URL: (http:\/\/127\.0\.0\.1:\d+\/[^\s]+)$/m)?.[1];
}

export function browserOverlayOffsetSeconds(timing, visibleLeadMs) {
  const values = [
    timing?.browserStartEpochMs,
    timing?.planReadyEpochMs,
    visibleLeadMs,
  ];
  if (values.some((value) => !Number.isFinite(value))) {
    throw new Error("Browser overlay timing values must be finite numbers");
  }
  return Math.max(
    0,
    (visibleLeadMs +
      timing.browserStartEpochMs -
      timing.planReadyEpochMs) /
      1000,
  );
}

export function validateOriginalHtml(html) {
  const failures = [];
  for (const [description, present] of [
    ["an HTML doctype", /<!doctype html>/i.test(html)],
    ["embedded CSS", /<style(?:\s|>)/i.test(html)],
    ['class "page"', hasAttribute(html, "class", "page")],
    ['id "ownership"', hasAttribute(html, "id", "ownership")],
    ['id "launch-gate"', hasAttribute(html, "id", "launch-gate")],
    [ORIGINAL_OWNERSHIP, html.includes(ORIGINAL_OWNERSHIP)],
    [ORIGINAL_GATE, html.includes(ORIGINAL_GATE)],
  ]) {
    if (!present) failures.push(`missing ${description}`);
  }
  if (failures.length > 0) {
    throw new Error(`Invalid Claude-created plan: ${failures.join("; ")}`);
  }
}

export function validateRevisedHtml(original, revised) {
  const failures = [];
  if (original === revised) failures.push("Claude Code did not change the plan");
  if (!revised.includes(REQUIRED_GATE)) {
    failures.push("the literal launch-gate change was not applied");
  }
  if (revised.includes(ORIGINAL_GATE)) {
    failures.push("the original launch gate is still present");
  }
  if (revised.includes(ORIGINAL_OWNERSHIP)) {
    failures.push("the ownership feedback was not incorporated");
  }
  for (const [description, present] of [
    ["the HTML doctype", /<!doctype html>/i.test(revised)],
    ["embedded CSS", /<style(?:\s|>)/i.test(revised)],
    ['id "ownership"', hasAttribute(revised, "id", "ownership")],
    ['id "launch-gate"', hasAttribute(revised, "id", "launch-gate")],
    ['class "page"', hasAttribute(revised, "class", "page")],
  ]) {
    if (!present) failures.push(`the revised plan lost ${description}`);
  }
  if (failures.length > 0) {
    throw new Error(`Invalid Claude revision: ${failures.join("; ")}`);
  }
}

function hasAttribute(html, name, value) {
  const escapedValue = value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(
    `\\b${name}\\s*=\\s*["'][^"']*\\b${escapedValue}\\b[^"']*["']`,
    "i",
  ).test(html);
}

async function main() {
  const repo = path.resolve(import.meta.dirname, "../..");
  const plan = requiredEnvironment("REP_CLAUDE_DEMO_PLAN");
  const settings = requiredEnvironment("REP_CLAUDE_DEMO_SETTINGS");
  const diagnosticsPath = requiredEnvironment("REP_DEMO_DIAGNOSTICS");
  const browserVideo = requiredEnvironment("REP_CLAUDE_DEMO_BROWSER_VIDEO");
  const timingFile = requiredEnvironment("REP_CLAUDE_DEMO_TIMING_FILE");
  const vhsStartFile = requiredEnvironment("REP_CLAUDE_DEMO_VHS_START_FILE");
  const tmux = requiredEnvironment("REP_CLAUDE_DEMO_TMUX_BIN");
  const tmuxConfig = requiredEnvironment("REP_CLAUDE_DEMO_TMUX_CONFIG");
  const tmuxSocket = requiredEnvironment("REP_CLAUDE_DEMO_TMUX_SOCKET");
  const driverDelay = parsePositiveInteger(
    process.env.REP_CLAUDE_DEMO_VHS_DRIVER_DELAY_MS || "1650",
    "REP_CLAUDE_DEMO_VHS_DRIVER_DELAY_MS",
  );
  const visibleLead = parsePositiveInteger(
    process.env.REP_CLAUDE_DEMO_VHS_VISIBLE_LEAD_MS || "4000",
    "REP_CLAUDE_DEMO_VHS_VISIBLE_LEAD_MS",
  );
  const model = process.env.REP_CLAUDE_DEMO_MODEL || "sonnet";
  const timeout = parsePositiveInteger(
    process.env.REP_CLAUDE_DEMO_TIMEOUT_MS || "300000",
    "REP_CLAUDE_DEMO_TIMEOUT_MS",
  );
  const session = "claude";
  const readyMarker = path.join(path.dirname(diagnosticsPath), "plan-created");
  const videoDirectory = path.join(repo, "target", "claude-html-vhs-browser");
  await mkdir(videoDirectory, { recursive: true });
  await writeFile(diagnosticsPath, "");

  let browser;
  let context;
  let page;
  let video;
  let completed = false;
  try {
    startClaudeSession({
      model,
      repo,
      session,
      settings,
      tmux,
      tmuxConfig,
      tmuxSocket,
    });
    await waitForClaudeReady({ session, timeout, tmux, tmuxSocket });
    await waitForFile(vhsStartFile, timeout, "VHS capture marker");
    await pause(driverDelay);

    sendToClaude(
      { session, tmux, tmuxSocket },
      [
        "Create a polished, responsive HTML rollout plan for checkout recovery and write it to demo-plan.html.",
        "Use semantic HTML, embedded CSS, a top-level wrapper with class=\"page\",",
        `and include exactly <p id="ownership">${ORIGINAL_OWNERSHIP}</p>`,
        `and <p id="launch-gate">${ORIGINAL_GATE}</p>.`,
        `When every file edit is complete, run: touch ${shellQuote(readyMarker)}`,
      ].join(" "),
    );
    await waitForClaudePlanMarker({
      file: readyMarker,
      session,
      timeout,
      tmux,
      tmuxSocket,
    });
    await waitForClaudePrompt({ session, timeout, tmux, tmuxSocket });
    const original = await readFile(plan, "utf8");
    validateOriginalHtml(original);

    const planReadyEpochMs = Date.now();
    setDemoStage({ session, tmux, tmuxSocket }, "plan-ready");
    await pause(1_200);
    sendToClaude({ session, tmux, tmuxSocket }, "/rep @demo-plan.html");
    const url = await waitForReviewUrl({
      diagnosticsPath,
      session,
      timeout,
      tmux,
      tmuxSocket,
    });

    browser = await chromium.launch({ headless: true });
    context = await browser.newContext({
      recordVideo: {
        dir: videoDirectory,
        size: { width: 1120, height: 700 },
        showActions: {
          cursor: "pointer",
          duration: 1_400,
          fontSize: 22,
          position: "bottom-right",
        },
      },
      viewport: { width: 1120, height: 700 },
    });
    const browserStartEpochMs = Date.now();
    page = await context.newPage();
    video = page.video();
    await page.goto(url);
    await page.waitForFunction(
      () => window.__repTest?.state?.status === "ready",
    );
    await pause(1_000);

    await page.keyboard.press("?");
    await pause(2_800);
    await page.keyboard.press("?");
    await pause(600);
    for (const key of ["j", "j", "Space", "j", "Backspace"]) {
      await page.keyboard.press(key);
      await pause(800);
    }

    await focusPlanElement(page, "#launch-gate");
    await page.keyboard.press("c");
    await page.locator("#modal-input").fill(REQUIRED_GATE);
    await pause(900);
    await commitModal(page);
    await pause(900);

    await focusPlanElement(page, "#ownership");
    await page.keyboard.press("f");
    await page
      .locator("#modal-input")
      .fill(
        "Name the owning team and the authoritative checkout-session state source.",
      );
    await pause(900);
    await commitModal(page);
    await page.waitForFunction(
      () => window.__repTest?.state?.annotationCount === 2,
    );
    await pause(1_200);

    await page.locator("#submit").click();
    await page.locator("#completion").waitFor();
    await pause(1_200);
    await context.close();
    context = null;
    await browser.close();
    browser = null;
    const browserEndEpochMs = Date.now();
    await rename(await video.path(), browserVideo);
    await writeFile(
      timingFile,
      `${JSON.stringify(
        {
          browserEndEpochMs,
          browserStartEpochMs,
          browserOverlayOffsetSeconds: browserOverlayOffsetSeconds(
            { browserStartEpochMs, planReadyEpochMs },
            visibleLead,
          ),
          planReadyEpochMs,
        },
        null,
        2,
      )}\n`,
    );

    setDemoStage({ session, tmux, tmuxSocket }, "apply-running");
    const revised = await waitForRevision(plan, original, timeout);
    await waitForClaudePrompt({ session, timeout, tmux, tmuxSocket });
    validateRevisedHtml(original, revised);
    setDemoStage({ session, tmux, tmuxSocket }, "revision-ready");
    await pause(3_500);
    sendToClaude({ session, tmux, tmuxSocket }, "/quit");
    await waitForCompletionMarker({ session, timeout, tmux, tmuxSocket });
    completed = true;
  } finally {
    if (context) await context.close();
    if (browser) await browser.close();
    if (!completed) abortClaude({ session, tmux, tmuxSocket });
  }
}

function requiredEnvironment(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
}

function parsePositiveInteger(value, name) {
  if (!/^\d+$/.test(value) || Number(value) < 1) {
    throw new Error(`${name} must be a positive integer: ${value}`);
  }
  return Number(value);
}

function startClaudeSession({
  model,
  repo,
  session,
  settings,
  tmux,
  tmuxConfig,
  tmuxSocket,
}) {
  const claude = [
    "claude",
    "--model",
    model,
    "--permission-mode",
    "bypassPermissions",
    "--settings",
    settings,
    "--no-chrome",
  ]
    .map(shellQuote)
    .join(" ");
  const command = `${claude}; claude_rc=$?; printf '\\nREP_HTML_DEMO_COMPLETE\\n'; sleep 5; exit "$claude_rc"`;
  runTmux(
    tmux,
    tmuxSocket,
    [
      "-f",
      tmuxConfig,
      "new-session",
      "-d",
      "-x",
      "180",
      "-y",
      "48",
      "-s",
      session,
      "-c",
      repo,
      command,
    ],
  );
}

async function waitForClaudeReady({ session, timeout, tmux, tmuxSocket }) {
  const deadline = Date.now() + Math.min(timeout, 30_000);
  while (Date.now() < deadline) {
    const pane = captureClaude({ session, tmux, tmuxSocket });
    if (pane.includes("Claude Code") && pane.includes("permissions on")) return;
    await pause(100);
  }
  throw new Error(
    `Claude Code did not become interactive:\n${captureClaude({
      session,
      tmux,
      tmuxSocket,
    })}`,
  );
}

async function waitForClaudePrompt({ session, timeout, tmux, tmuxSocket }) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const pane = captureClaude({ session, tmux, tmuxSocket });
    if (claudeAtPrompt(pane)) return;
    if (!tmuxSessionExists({ session, tmux, tmuxSocket })) {
      throw new Error(`Claude Code exited before returning to its prompt:\n${pane}`);
    }
    await pause(100);
  }
  throw new Error(
    `Claude Code did not return to its prompt:\n${captureClaude({
      session,
      tmux,
      tmuxSocket,
    })}`,
  );
}

async function waitForClaudePlanMarker({
  file,
  session,
  timeout,
  tmux,
  tmuxSocket,
}) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try {
      await access(file);
      return;
    } catch {
      const pane = captureClaude({ session, tmux, tmuxSocket });
      if (claudeAtPrompt(pane) && pane.includes("API Error:")) {
        throw new Error(`Claude Code could not create the demo plan:\n${pane}`);
      }
      if (!tmuxSessionExists({ session, tmux, tmuxSocket })) {
        throw new Error(`Claude Code exited before creating the demo plan:\n${pane}`);
      }
      await pause(100);
    }
  }
  throw new Error(`Timed out waiting for Claude plan creation after ${timeout}ms`);
}

function claudeAtPrompt(pane) {
  return pane.split("\n").some((line) => line.trim() === "❯");
}

function sendToClaude({ session, tmux, tmuxSocket }, text) {
  runTmux(tmux, tmuxSocket, ["load-buffer", "-b", "rep-demo-input", "-"], text);
  runTmux(tmux, tmuxSocket, [
    "paste-buffer",
    "-d",
    "-b",
    "rep-demo-input",
    "-t",
    session,
  ]);
  runTmux(tmux, tmuxSocket, ["send-keys", "-t", session, "Enter"]);
}

function setDemoStage({ session, tmux, tmuxSocket }, stage) {
  runTmux(tmux, tmuxSocket, [
    "rename-window",
    "-t",
    session,
    stage,
  ]);
}

function captureClaude({ session, tmux, tmuxSocket }) {
  return runTmux(tmux, tmuxSocket, [
    "capture-pane",
    "-p",
    "-S",
    "-1000",
    "-t",
    session,
  ]).trim();
}

async function waitForReviewUrl({
  diagnosticsPath,
  session,
  timeout,
  tmux,
  tmuxSocket,
}) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const diagnostics = await readFile(diagnosticsPath, "utf8");
    const url = extractReviewUrl(diagnostics);
    if (url) return url;
    if (!tmuxSessionExists({ session, tmux, tmuxSocket })) {
      throw new Error(
        `Claude Code exited before Rep launched:\n${diagnostics}`,
      );
    }
    await pause(100);
  }
  throw new Error(`Timed out waiting for the Rep review URL after ${timeout}ms`);
}

async function waitForCompletionMarker({
  session,
  timeout,
  tmux,
  tmuxSocket,
}) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (!tmuxSessionExists({ session, tmux, tmuxSocket })) return;
    const pane = captureClaude({ session, tmux, tmuxSocket });
    if (pane.includes("REP_HTML_DEMO_COMPLETE")) return;
    await pause(100);
  }
  throw new Error("Timed out waiting for Claude Code to exit");
}

async function waitForFile(file, timeout, description) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try {
      await access(file);
      return;
    } catch {
      await pause(100);
    }
  }
  throw new Error(`Timed out waiting for ${description} after ${timeout}ms`);
}

async function waitForRevision(plan, original, timeout) {
  const deadline = Date.now() + timeout;
  let lastError;
  while (Date.now() < deadline) {
    const revised = await readFile(plan, "utf8");
    try {
      validateRevisedHtml(original, revised);
      return revised;
    } catch (error) {
      lastError = error;
    }
    await pause(200);
  }
  throw new Error(
    `Timed out waiting for Claude Code to apply the review: ${lastError?.message}`,
  );
}

function tmuxSessionExists({ session, tmux, tmuxSocket }) {
  const result = spawnSync(
    tmux,
    ["-L", tmuxSocket, "has-session", "-t", session],
    tmuxOptions(),
  );
  return result.status === 0;
}

function abortClaude({ session, tmux, tmuxSocket }) {
  try {
    runTmux(tmux, tmuxSocket, ["send-keys", "-t", session, "C-c"]);
    runTmux(tmux, tmuxSocket, [
      "send-keys",
      "-t",
      session,
      "-l",
      "/quit",
    ]);
    runTmux(tmux, tmuxSocket, ["send-keys", "-t", session, "Enter"]);
  } catch {
    // The shell cleanup owns the isolated tmux server if Claude already exited.
  }
}

function runTmux(tmux, socket, args, input = undefined) {
  const result = spawnSync(
    tmux,
    ["-L", socket, ...args],
    tmuxOptions({ input }),
  );
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      `tmux ${args[0]} failed (${result.status}): ${result.stderr || result.stdout}`,
    );
  }
  return result.stdout;
}

function tmuxOptions(extra = {}) {
  return {
    env: {
      ...process.env,
      TMUX: "",
    },
    encoding: "utf8",
    ...extra,
  };
}

function shellQuote(value) {
  return `'${value.replaceAll("'", `'\\''`)}'`;
}

async function commitModal(page) {
  const modal = page.locator("#modal");
  await page.locator("#modal-input").press("Enter");
  try {
    await modal.waitFor({ state: "hidden", timeout: 5_000 });
  } catch {
    const status = await page.locator("#status").textContent();
    if (status?.includes("retry your command")) {
      await page.locator("#modal-input").press("Enter");
      await modal.waitFor({ state: "hidden", timeout: 5_000 });
      return;
    }
    const title = await page.locator("#modal-title").textContent();
    throw new Error(
      `Rep modal "${title}" remained open after submit (status: ${status})`,
    );
  }
}

async function focusPlanElement(page, selector) {
  await page.evaluate((target) => {
    const iframe = document.querySelector("#plan");
    iframe.contentDocument.querySelector(target).scrollIntoView({
      behavior: "instant",
      block: "center",
    });
  }, selector);
  await pause(700);
  const point = await page.evaluate((target) => {
    const iframe = document.querySelector("#plan");
    const element = iframe.contentDocument.querySelector(target);
    const iframeRect = iframe.getBoundingClientRect();
    const elementRect = element.getBoundingClientRect();
    return {
      x: iframeRect.left + elementRect.left + Math.min(24, elementRect.width / 2),
      y: iframeRect.top + elementRect.top + Math.min(8, elementRect.height / 2),
    };
  }, selector);
  await page.mouse.click(point.x, point.y);
  await page.waitForFunction(
    (target) => {
      const rep = window.__repTest;
      const node = rep?.manifest?.nodes?.[rep.state?.anchor?.node];
      return node?.selector === target;
    },
    selector,
  );
  await pause(500);
}

function pause(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

const isMain =
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href;
if (isMain) {
  main().catch((error) => {
    process.stderr.write(`${error.stack || error}\n`);
    process.exitCode = 1;
  });
}
