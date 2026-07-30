import { spawn, spawnSync } from "node:child_process";
import {
  access,
  copyFile,
  readFile,
  stat,
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
const BROWSER_TYPING_DELAY_MS = 35;
const NATIVE_WINDOW_LOOKUP_SOURCE = `
import CoreGraphics
import Foundation

let browserPid = Int(CommandLine.arguments[1]) ?? -1
let windows = CGWindowListCopyWindowInfo(
  [.optionOnScreenOnly, .excludeDesktopElements],
  kCGNullWindowID
) as? [[String: Any]] ?? []
let candidates = windows.filter { window in
  let ownerPid = window[kCGWindowOwnerPID as String] as? Int ?? -1
  let layer = window[kCGWindowLayer as String] as? Int ?? -1
  let alpha = window[kCGWindowAlpha as String] as? Double ?? 0
  return ownerPid == browserPid && layer == 0 && alpha > 0
}
let largest = candidates.max { left, right in
  func area(_ window: [String: Any]) -> Double {
    let bounds = window[kCGWindowBounds as String] as? [String: Any] ?? [:]
    let width = bounds["Width"] as? Double ?? 0
    let height = bounds["Height"] as? Double ?? 0
    return width * height
  }
  return area(left) < area(right)
}
if
  let window = largest,
  let windowId = window[kCGWindowNumber as String] as? Int,
  let boundsDictionary = window[kCGWindowBounds as String] as? [String: Any],
  let windowFrame = CGRect(
    dictionaryRepresentation: boundsDictionary as CFDictionary
  )
{
  var displayCount: UInt32 = 0
  _ = CGGetActiveDisplayList(0, nil, &displayCount)
  var displays = Array(repeating: CGDirectDisplayID(), count: Int(displayCount))
  _ = CGGetActiveDisplayList(displayCount, &displays, &displayCount)
  let windowCenter = CGPoint(x: windowFrame.midX, y: windowFrame.midY)
  if let displayIndex = displays.firstIndex(where: {
    CGDisplayBounds($0).contains(windowCenter)
  }) {
    let displayFrame = CGDisplayBounds(displays[displayIndex])
    let capture = [
      "windowId": windowId,
      "displayNumber": displayIndex + 1,
      "x": windowFrame.minX - displayFrame.minX,
      "y": windowFrame.minY - displayFrame.minY,
      "width": windowFrame.width,
      "height": windowFrame.height,
      "displayWidth": displayFrame.width,
      "displayHeight": displayFrame.height,
    ] as [String: Any]
    let data = try! JSONSerialization.data(withJSONObject: capture)
    print(String(data: data, encoding: .utf8)!)
  }
}
`;

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

export function browserProcessId(processInfo) {
  const browser = processInfo?.find((process) => process.type === "browser");
  if (!Number.isInteger(browser?.id) || browser.id < 1) {
    throw new Error("Chromium did not report a valid browser process id");
  }
  return browser.id;
}

export function parseNativeBrowserCapture(output) {
  let capture;
  try {
    capture = JSON.parse(output);
  } catch {
    throw new Error(`Could not identify the headed Chromium window: ${output.trim()}`);
  }
  const positiveIntegers = ["windowId", "displayNumber"];
  const finitePositive = ["width", "height", "displayWidth", "displayHeight"];
  if (
    positiveIntegers.some(
      (key) => !Number.isInteger(capture?.[key]) || capture[key] < 1,
    ) ||
    finitePositive.some(
      (key) => !Number.isFinite(capture?.[key]) || capture[key] <= 0,
    ) ||
    !Number.isFinite(capture?.x) ||
    !Number.isFinite(capture?.y) ||
    capture.x < 0 ||
    capture.y < 0 ||
    capture.x + capture.width > capture.displayWidth + 1 ||
    capture.y + capture.height > capture.displayHeight + 1
  ) {
    throw new Error(`Could not identify the headed Chromium window: ${output.trim()}`);
  }
  return capture;
}

export function browserCropFilter(capture) {
  const {
    displayHeight,
    displayWidth,
    height,
    width,
    x,
    y,
  } = parseNativeBrowserCapture(JSON.stringify(capture));
  return [
    `crop=w='floor(iw*${width}/${displayWidth}/2)*2'`,
    `h='floor(ih*${height}/${displayHeight}/2)*2'`,
    `x='round(iw*${x}/${displayWidth})'`,
    `y='round(ih*${y}/${displayHeight})'`,
  ].join(":");
}

export function validateGeneratedHtml(html) {
  const failures = [];
  for (const [description, present] of [
    ["an HTML doctype", /<!doctype html>/i.test(html)],
    ["an html element", /<html(?:\s|>)/i.test(html)],
    ["a body element", /<body(?:\s|>)/i.test(html)],
    ["substantive content", html.trim().length >= 200],
  ]) {
    if (!present) failures.push(`missing ${description}`);
  }
  if (failures.length > 0) {
    throw new Error(`Invalid Claude-generated draft: ${failures.join("; ")}`);
  }
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
    throw new Error(`Invalid deterministic demo plan: ${failures.join("; ")}`);
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
  const planFixture = requiredEnvironment("REP_CLAUDE_DEMO_PLAN_FIXTURE");
  const settings = requiredEnvironment("REP_CLAUDE_DEMO_SETTINGS");
  const diagnosticsPath = requiredEnvironment("REP_DEMO_DIAGNOSTICS");
  const browserVideo = requiredEnvironment("REP_CLAUDE_DEMO_BROWSER_VIDEO");
  const displayRecorder = requiredEnvironment(
    "REP_CLAUDE_DEMO_DISPLAY_RECORDER",
  );
  const timingFile = requiredEnvironment("REP_CLAUDE_DEMO_TIMING_FILE");
  const vhsStartFile = requiredEnvironment("REP_CLAUDE_DEMO_VHS_START_FILE");
  const tmux = requiredEnvironment("REP_CLAUDE_DEMO_TMUX_BIN");
  const tmuxConfig = requiredEnvironment("REP_CLAUDE_DEMO_TMUX_CONFIG");
  const tmuxSocket = requiredEnvironment("REP_CLAUDE_DEMO_TMUX_SOCKET");
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
  await writeFile(diagnosticsPath, "");

  let browser;
  let context;
  let nativeRecording;
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
    await waitForClaudePlan({
      file: plan,
      session,
      timeout,
      tmux,
      tmuxSocket,
    });
    await copyFile(planFixture, plan);
    const original = await readFile(plan, "utf8");
    validateOriginalHtml(original);

    const planReadyEpochMs = Date.now();
    setDemoStage({ session, tmux, tmuxSocket }, "plan-ready");
    const url = await waitForReviewUrl({
      diagnosticsPath,
      session,
      timeout,
      tmux,
      tmuxSocket,
    });

    browser = await chromium.launch({
      headless: false,
      args: ["--window-position=40,40", "--window-size=1120,780"],
    });
    context = await browser.newContext({
      viewport: { width: 1120, height: 700 },
    });
    const page = await context.newPage();
    await page.goto(url);
    await page.waitForFunction(
      () => window.__repTest?.state?.status === "ready",
    );
    await installDemoActionCue(page);

    const cdp = await browser.newBrowserCDPSession();
    const { processInfo } = await cdp.send("SystemInfo.getProcessInfo");
    const browserPid = browserProcessId(processInfo);
    const browserCapture = await waitForNativeBrowserCapture(browserPid, timeout);
    await page.bringToFront();
    const recordingStart = startNativeDisplayRecording({
      capture: browserCapture,
      output: browserVideo,
      recorder: displayRecorder,
      timeout,
    });
    await page.bringToFront();
    nativeRecording = await recordingStart;
    const browserStartEpochMs = Date.now();
    const browserVideoTrimSeconds = Math.max(
      0,
      (browserStartEpochMs - nativeRecording.startedEpochMs) / 1_000,
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
    await typeBrowserDialogText(page, REQUIRED_GATE);
    await pause(900);
    await commitModal(page);
    await pause(900);

    await focusPlanElement(page, "#ownership");
    await page.keyboard.press("f");
    await typeBrowserDialogText(
      page,
      "Name the owning team and the authoritative checkout-session state source.",
    );
    await pause(900);
    await commitModal(page);
    await page.waitForFunction(
      () => window.__repTest?.state?.annotationCount === 2,
    );
    await pause(1_200);

    await page.keyboard.press("q");
    await page.locator("#modal").waitFor();
    await pause(700);
    await page.locator("#modal-confirm").click();
    await page.locator("#completion").waitFor();
    await pause(1_200);
    await stopNativeDisplayRecording(nativeRecording);
    nativeRecording = null;
    await context.close();
    context = null;
    await browser.close();
    browser = null;
    const browserEndEpochMs = Date.now();
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
          browserCapture,
          browserVideoTrimSeconds,
          planReadyEpochMs,
        },
        null,
        2,
      )}\n`,
    );

    setDemoStage({ session, tmux, tmuxSocket }, "apply-running");
    const revised = await waitForRevision({
      original,
      plan,
      session,
      timeout,
      tmux,
      tmuxSocket,
    });
    await waitForClaudePrompt({ session, timeout, tmux, tmuxSocket });
    validateRevisedHtml(original, revised);
    setDemoStage({ session, tmux, tmuxSocket }, "revision-ready");
    await waitForCompletionMarker({ session, timeout, tmux, tmuxSocket });
    completed = true;
  } finally {
    if (nativeRecording) {
      await stopNativeDisplayRecording(nativeRecording).catch(() => {});
    }
    if (context) await context.close();
    if (browser) await browser.close();
    if (!completed) abortClaude({ session, tmux, tmuxSocket });
  }
}

async function typeBrowserDialogText(page, text) {
  await page
    .locator("#modal-input")
    .pressSequentially(text, { delay: BROWSER_TYPING_DELAY_MS });
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
    "--setting-sources",
    "project",
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

async function waitForClaudePlan({
  file,
  session,
  timeout,
  tmux,
  tmuxSocket,
}) {
  const deadline = Date.now() + timeout;
  let lastError;
  while (Date.now() < deadline) {
    const pane = captureClaude({ session, tmux, tmuxSocket });
    if (claudeAtPrompt(pane)) {
      if (pane.includes("API Error:")) {
        throw new Error(`Claude Code could not create the demo plan:\n${pane}`);
      }
      try {
        const html = await readFile(file, "utf8");
        validateGeneratedHtml(html);
        return html;
      } catch (error) {
        lastError = error;
      }
    }
    if (!tmuxSessionExists({ session, tmux, tmuxSocket })) {
      throw new Error(`Claude Code exited before creating the demo plan:\n${pane}`);
    }
    await pause(100);
  }
  throw new Error(
    `Timed out waiting for Claude plan creation after ${timeout}ms: ${lastError?.message || "no valid plan was written"}`,
  );
}

function claudeAtPrompt(pane) {
  return pane.split("\n").some((line) => line.trim() === "❯");
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

async function waitForNativeBrowserCapture(browserPid, timeout) {
  const deadline = Date.now() + Math.min(timeout, 30_000);
  while (Date.now() < deadline) {
    const result = spawnSync(
      "xcrun",
      ["swift", "-e", NATIVE_WINDOW_LOOKUP_SOURCE, String(browserPid)],
      { encoding: "utf8" },
    );
    if (result.error) throw result.error;
    if (result.status !== 0) {
      throw new Error(
        `Could not query the headed Chromium window (${result.status}): ${result.stderr || result.stdout}`,
      );
    }
    if (result.stdout.trim()) return parseNativeBrowserCapture(result.stdout);
    await pause(200);
  }
  throw new Error(
    `Timed out waiting for headed Chromium process ${browserPid} to expose a window`,
  );
}

export function nativeRecorderArguments({
  displayNumber,
  output,
  ready,
  stop,
}) {
  if (!Number.isInteger(displayNumber) || displayNumber < 1) {
    throw new Error(`Invalid macOS display number: ${displayNumber}`);
  }
  return [String(displayNumber), output, ready, stop];
}

export function shouldConfirmRepSuggestions(pane) {
  return (
    pane.includes("Would you like me to apply these rep suggestions") &&
    pane.split("\n").some((line) => line.trimStart().startsWith("❯"))
  );
}

async function startNativeDisplayRecording({
  capture,
  output,
  recorder,
  timeout,
}) {
  const child = spawn(
    recorder,
    nativeRecorderArguments({
      displayNumber: capture.displayNumber,
      output,
      ready: `${output}.ready`,
      stop: `${output}.stop`,
    }),
    { stdio: ["ignore", "ignore", "pipe"] },
  );
  let result = null;
  let settle;
  let stderr = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  const finished = new Promise((resolve) => {
    settle = resolve;
  });
  const finish = (value) => {
    if (result) return;
    result = value;
    settle(value);
  };
  child.once("error", (error) => finish({ error }));
  child.once("exit", (code, signal) => finish({ code, signal }));
  const recording = {
    child,
    displayNumber: capture.displayNumber,
    finished,
    get result() {
      return result;
    },
    output,
    ready: `${output}.ready`,
    recorder,
    stderr: () => stderr.trim(),
    stop: `${output}.stop`,
    startedEpochMs: Date.now(),
  };
  try {
    await waitForNativeRecordingStart(recording, Math.min(timeout, 30_000));
    return recording;
  } catch (error) {
    child.kill("SIGTERM");
    await finished;
    throw error;
  }
}

async function stopNativeDisplayRecording(recording) {
  await writeFile(recording.stop, "");
  const result = await Promise.race([
    recording.finished,
    pause(30_000).then(() => null),
  ]);
  if (!result) {
    recording.child.kill("SIGTERM");
    throw new Error("Built-in macOS browser recording did not stop within 30 seconds");
  }
  if (result.error || result.code !== 0) {
    throw nativeRecordingError(
      "Built-in macOS browser recording failed",
      recording,
      result,
    );
  }
  const output = await stat(recording.output);
  if (output.size === 0) {
    throw new Error("Built-in macOS browser recording produced an empty file");
  }
}

async function waitForNativeRecordingStart(recording, timeout) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await fileExists(recording.ready)) return;
    if (recording.result) {
      throw nativeRecordingError(
        "Built-in macOS browser recording exited before capture began",
        recording,
        recording.result,
      );
    }
    await pause(100);
  }
  throw new Error(
    `Built-in macOS browser recording did not start within ${timeout}ms`,
  );
}

function nativeRecordingError(message, recording, result) {
  const detail =
    result.error?.message ||
    recording.stderr() ||
    `exit=${result.code ?? "none"} signal=${result.signal ?? "none"}`;
  return new Error(`${message}: ${detail}`);
}

async function fileExists(file) {
  try {
    await access(file);
    return true;
  } catch {
    return false;
  }
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

async function waitForRevision({
  original,
  plan,
  session,
  timeout,
  tmux,
  tmuxSocket,
}) {
  const deadline = Date.now() + timeout;
  let confirmedRepSuggestions = false;
  let lastError;
  while (Date.now() < deadline) {
    const revised = await readFile(plan, "utf8");
    try {
      validateRevisedHtml(original, revised);
      return revised;
    } catch (error) {
      lastError = error;
    }
    if (!confirmedRepSuggestions) {
      const pane = captureClaude({ session, tmux, tmuxSocket });
      if (shouldConfirmRepSuggestions(pane)) {
        runTmux(tmux, tmuxSocket, [
          "send-keys",
          "-t",
          session,
          "-l",
          "Apply both Rep suggestions exactly as captured; they intentionally supersede the original placeholder text.",
        ]);
        runTmux(tmux, tmuxSocket, ["send-keys", "-t", session, "Enter"]);
        confirmedRepSuggestions = true;
      }
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

async function installDemoActionCue(page) {
  await page.evaluate(() => {
    const cue = document.createElement("div");
    cue.id = "rep-demo-action-cue";
    cue.setAttribute("aria-hidden", "true");
    Object.assign(cue.style, {
      background: "rgba(15, 23, 42, 0.92)",
      border: "1px solid rgba(255, 255, 255, 0.24)",
      borderRadius: "10px",
      bottom: "18px",
      boxShadow: "0 8px 24px rgba(15, 23, 42, 0.3)",
      color: "#ffffff",
      font: "600 22px -apple-system, BlinkMacSystemFont, sans-serif",
      letterSpacing: "0.01em",
      opacity: "0",
      padding: "10px 15px",
      pointerEvents: "none",
      position: "fixed",
      right: "18px",
      transform: "translateY(8px)",
      transition: "opacity 120ms ease, transform 120ms ease",
      zIndex: "2147483647",
    });
    document.documentElement.append(cue);

    let hideTimer;
    const show = (label) => {
      cue.textContent = label;
      cue.style.opacity = "1";
      cue.style.transform = "translateY(0)";
      clearTimeout(hideTimer);
      hideTimer = setTimeout(() => {
        cue.style.opacity = "0";
        cue.style.transform = "translateY(8px)";
      }, 1_400);
    };
    window.addEventListener(
      "keydown",
      (event) => {
        const key =
          event.key === " "
            ? "Space"
            : event.key === "Backspace"
              ? "Backspace"
              : event.key;
        show(`Press “${key}”`);
      },
      true,
    );
    window.addEventListener("pointerdown", () => show("Click"), true);
  });
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
  const targetNode = await page.evaluate((target) => {
    const rep = window.__repTest;
    const node = rep?.manifest?.nodes?.findIndex(
      (candidate) => candidate.selector === target,
    );
    if (node < 0) throw new Error(`Review target is missing: ${target}`);
    const iframe = document.querySelector("#plan");
    iframe.contentDocument.querySelector(target).scrollIntoView({
      behavior: "instant",
      block: "center",
    });
    return node;
  }, selector);
  await pause(700);
  for (const horizontalPosition of [0.05, 0.5, 0.95]) {
    const point = await page.evaluate(
      ({ position, target }) => {
        const iframe = document.querySelector("#plan");
        const element = iframe.contentDocument.querySelector(target);
        const iframeRect = iframe.getBoundingClientRect();
        const elementRect = element.getBoundingClientRect();
        return {
          x:
            iframeRect.left +
            elementRect.left +
            elementRect.width * position,
          y: iframeRect.top + elementRect.top + elementRect.height / 2,
        };
      },
      { position: horizontalPosition, target: selector },
    );
    await page.mouse.click(point.x, point.y);
    try {
      await page.waitForFunction(
        (node) => window.__repTest?.state?.anchor?.node === node,
        targetNode,
        { timeout: 2_000 },
      );
      await pause(500);
      return;
    } catch {
      // Try a different point in case another rendered element overlaps the text.
    }
  }
  const diagnostics = await page.evaluate(() => ({
    anchor: window.__repTest?.state?.anchor,
    clicks: window.__repTest?.clickEvents?.slice(-3),
  }));
  throw new Error(
    `Could not focus ${selector}: ${JSON.stringify(diagnostics)}`,
  );
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
