import { spawnSync } from "node:child_process";
import {
  access,
  copyFile,
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
  for (const preserved of [
    "<!doctype html>",
    "<style>",
    'id="ownership"',
    'id="launch-gate"',
    'class="page"',
  ]) {
    if (!revised.includes(preserved)) {
      failures.push(`the revised plan lost ${preserved}`);
    }
  }
  if (failures.length > 0) {
    throw new Error(`Invalid Claude revision: ${failures.join("; ")}`);
  }
}

async function main() {
  const repo = path.resolve(import.meta.dirname, "../..");
  const output = path.resolve(
    repo,
    process.argv[2] || "docs/rep-claude-html-skill-demo.webm",
  );
  const plan = requiredEnvironment("REP_CLAUDE_DEMO_PLAN");
  const fixture = requiredEnvironment("REP_CLAUDE_DEMO_FIXTURE");
  const settings = requiredEnvironment("REP_CLAUDE_DEMO_SETTINGS");
  const diagnosticsPath = requiredEnvironment("REP_DEMO_DIAGNOSTICS");
  const tmux = requiredEnvironment("REP_CLAUDE_DEMO_TMUX_BIN");
  const tmuxSocket = requiredEnvironment("REP_CLAUDE_DEMO_TMUX_SOCKET");
  const model = process.env.REP_CLAUDE_DEMO_MODEL || "sonnet";
  const timeout = parseTimeout(
    process.env.REP_CLAUDE_DEMO_TIMEOUT_MS || "240000",
  );
  const session = "claude";
  const readyMarker = path.join(path.dirname(diagnosticsPath), "plan-created");
  const videoDirectory = path.join(repo, "target", "claude-html-demo-video");
  await mkdir(path.dirname(output), { recursive: true });
  await mkdir(videoDirectory, { recursive: true });
  await writeFile(diagnosticsPath, "");

  let browser;
  let context;
  let page;
  let video;
  try {
    startClaudeSession({ model, repo, session, settings, tmux, tmuxSocket });
    await waitForClaudeReady({ session, timeout, tmux, tmuxSocket });
    sendToClaude(
      { session, tmux, tmuxSocket },
      [
        "Create a concise, polished HTML rollout plan for checkout recovery and write it to demo-plan.html.",
        "Use semantic HTML and embedded responsive CSS.",
        `When the file is complete, run: touch ${shellQuote(readyMarker)}`,
      ].join(" "),
    );
    await waitForFile(readyMarker, timeout, "Claude plan creation marker");
    await waitForClaudePrompt({ session, timeout, tmux, tmuxSocket });
    const creation = captureClaude({ session, tmux, tmuxSocket });

    // Claude performs the real create turn. Normalize its output to the
    // checked-in fixture afterward so browser navigation stays deterministic.
    await copyFile(fixture, plan);
    const original = await readFile(plan, "utf8");
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
      recordVideo: { dir: videoDirectory, size: { width: 1280, height: 800 } },
      viewport: { width: 1280, height: 800 },
    });
    page = await context.newPage();
    video = page.video();

    await showClaudeScreen(page, {
      phase: "review",
      creation,
      command: "/rep @demo-plan.html",
      detail:
        "Claude Code launched the bundled Rep skill and is waiting for your browser review.",
    });
    await pause(2_600);

    await page.goto(url);
    await page.waitForFunction(
      () => window.__repTest?.state?.status === "ready",
    );
    await pause(1_200);

    await page.keyboard.press("I");
    await pause(1_000);
    await page.keyboard.press("I");
    await page.keyboard.press("/");
    await page.locator("#modal-input").fill("Launch to all customers");
    await pause(500);
    await commitModal(page);
    await pause(800);
    await page.keyboard.press("c");
    await page.locator("#modal-input").fill(REQUIRED_GATE);
    await pause(900);
    await commitModal(page);
    await pause(900);

    await page.keyboard.press("/");
    await page.locator("#modal-input").fill("checkout platform group");
    await pause(500);
    await commitModal(page);
    await pause(800);
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
    await pause(1_000);
    await showClaudeScreen(page, {
      phase: "apply",
      creation,
      command: "/rep @demo-plan.html",
      detail:
        "Rep returned two fresh actions. Claude Code is applying them to the original HTML now.",
    });

    const revised = await waitForRevision(plan, original, timeout);
    await waitForClaudePrompt({ session, timeout, tmux, tmuxSocket });
    const reviewResult = captureClaude({ session, tmux, tmuxSocket });

    await showClaudeScreen(page, {
      phase: "complete",
      creation,
      command: "/rep @demo-plan.html",
      detail: reviewResult,
    });
    await pause(4_500);
    await showRevisedPlan(page, revised);
    await pause(2_800);
    await page.locator("#launch-gate").scrollIntoViewIfNeeded();
    await pause(2_800);
  } finally {
    if (context) await context.close();
    if (browser) await browser.close();
    killTmux({ tmux, tmuxSocket });
  }

  await rename(await video.path(), output);
  process.stdout.write(`Recorded ${output}\n`);
}

function requiredEnvironment(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
}

function parseTimeout(value) {
  if (!/^\d+$/.test(value) || Number(value) < 1_000) {
    throw new Error(`REP_CLAUDE_DEMO_TIMEOUT_MS must be at least 1000: ${value}`);
  }
  return Number(value);
}

function startClaudeSession({ model, repo, session, settings, tmux, tmuxSocket }) {
  const command = [
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
  runTmux(tmux, tmuxSocket, [
    "new-session",
    "-d",
    "-x",
    "128",
    "-y",
    "36",
    "-s",
    session,
    "-c",
    repo,
    command,
  ]);
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
    if (pane.split("\n").some((line) => line.trim() === "❯")) return;
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

function killTmux({ tmux, tmuxSocket }) {
  spawnSync(tmux, ["-L", tmuxSocket, "kill-server"], tmuxOptions());
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

async function showClaudeScreen(page, { phase, creation, command, detail }) {
  await loadStandaloneHtml(page, `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><style>
  :root { color-scheme: dark; font: 16px/1.5 Menlo, Monaco, monospace; background: #161412; }
  * { box-sizing: border-box; }
  body { margin: 0; min-height: 100vh; padding: 44px; background: radial-gradient(circle at 80% 10%, #392922, transparent 34rem), #161412; color: #eee9df; }
  .terminal { max-width: 1120px; min-height: 680px; margin: 0 auto; overflow: hidden; border: 1px solid #504942; border-radius: 16px; background: rgb(23 21 19 / 94%); box-shadow: 0 32px 90px #0008; }
  .bar { display: flex; align-items: center; gap: 8px; height: 48px; padding: 0 18px; border-bottom: 1px solid #3b3631; background: #211e1b; }
  .dot { width: 11px; height: 11px; border-radius: 50%; background: #e76f51; }
  .dot:nth-child(2) { background: #e9c46a; } .dot:nth-child(3) { background: #6fa873; }
  .title { margin-left: 12px; color: #aaa096; font: 700 12px/1.2 Inter, sans-serif; letter-spacing: .12em; text-transform: uppercase; }
  .body { padding: 28px 34px 34px; }
  .prompt { color: #e9c46a; } .command { color: #fff; }
  .claude { max-height: 180px; margin: 22px 0; overflow: hidden; padding-left: 18px; border-left: 3px solid #d97757; color: #d8d1c6; white-space: pre-wrap; }
  .skill { max-height: 260px; margin-top: 28px; overflow: hidden; padding: 16px 18px; border: 1px solid #51473f; border-radius: 10px; background: #201c19; color: #f0b79f; white-space: pre-wrap; }
  .complete { color: #90cf99; }
  .footer { margin-top: 26px; color: #827970; font: 13px/1.4 Inter, sans-serif; }
  .cursor { display: inline-block; width: 9px; height: 1.1em; margin-left: 4px; background: #e9c46a; vertical-align: -2px; animation: blink 1s steps(1) infinite; }
  @keyframes blink { 50% { opacity: 0; } }
</style></head><body><main class="terminal">
  <div class="bar"><i class="dot"></i><i class="dot"></i><i class="dot"></i><span class="title">Claude Code · Rep HTML plan demo</span></div>
  <div class="body">
    <div><span class="prompt">rep-demo %</span> <span class="command">Create a polished HTML rollout plan for checkout recovery.</span></div>
    <div class="claude">${escapeHtml(compact(creation, 520) || "Created demo-plan.html with responsive embedded CSS.")}</div>
    <div><span class="prompt">rep-demo %</span> <span class="command">${escapeHtml(command)}</span></div>
    <div class="skill ${phase === "complete" ? "complete" : ""}">${escapeHtml(compact(detail, phase === "complete" ? 2_600 : 900))}${phase === "complete" ? "" : '<span class="cursor"></span>'}</div>
    <div class="footer">Real Claude Code CLI · bundled /rep skill · original HTML/CSS preserved</div>
  </div>
</main></body></html>`);
}

async function showRevisedPlan(page, revised) {
  const banner = `<style>
    #rep-demo-result {
      position: fixed; z-index: 2147483647; top: 18px; right: 18px;
      padding: 11px 16px; border-radius: 999px; color: #fff;
      background: #292621; box-shadow: 0 8px 30px #0004;
      font: 700 13px/1.2 Inter, ui-sans-serif, system-ui, sans-serif;
    }
  </style><div id="rep-demo-result">✓ Claude Code applied the Rep review</div>`;
  await loadStandaloneHtml(
    page,
    revised.replace(/<body([^>]*)>/i, `<body$1>${banner}`),
  );
}

async function loadStandaloneHtml(page, html) {
  await page.goto(`data:text/html;charset=utf-8,${encodeURIComponent(html)}`, {
    waitUntil: "load",
  });
}

function compact(value, maximum) {
  const normalized = value
    .replace(/\u001b\[[0-9;]*m/g, "")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
  if (normalized.length <= maximum) return normalized;
  return `…${normalized.slice(-(maximum - 1))}`;
}

function escapeHtml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
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
