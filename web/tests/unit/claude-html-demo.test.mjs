import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  browserCropFilter,
  browserOverlayOffsetSeconds,
  browserProcessId,
  extractReviewUrl,
  nativeRecorderArguments,
  parseNativeBrowserCapture,
  shouldConfirmRepSuggestions,
  validateGeneratedHtml,
  validateOriginalHtml,
  validateRevisedHtml,
} from "../record-claude-html-demo.mjs";

const deterministicPlan = readFileSync(
  new URL("../../../scripts/claude-rep-html-demo-plan.html", import.meta.url),
  "utf8",
);
const recorderScript = readFileSync(
  new URL(
    "../../../scripts/record-claude-rep-html-demo.sh",
    import.meta.url,
  ),
  "utf8",
);
const recorderTape = readFileSync(
  new URL("../../../scripts/claude-rep-html-demo.tape", import.meta.url),
  "utf8",
);
const recorderOrchestrator = readFileSync(
  new URL("../record-claude-html-demo.mjs", import.meta.url),
  "utf8",
);

const browserCapture = {
  windowId: 731,
  displayNumber: 1,
  x: 40,
  y: 40,
  width: 1120,
  height: 780,
  displayWidth: 1512,
  displayHeight: 982,
};

const original = `<!doctype html>
<html><head><style>p { color: black; }</style></head><body>
<div class="page">
<p id="ownership">The checkout platform group will monitor failures after launch.</p>
<p id="launch-gate">Launch to all customers as soon as integration tests pass.</p>
</div>
</body></html>`;

const revised = original
  .replace(
    "The checkout platform group will monitor failures after launch.",
    "The Checkout Reliability team owns rollout monitoring in the CheckoutSession state store.",
  )
  .replace(
    "Launch to all customers as soon as integration tests pass.",
    "Launch at 10% only after recovered-cart deltas remain below 0.5% for 24 hours.",
  );

test("Claude HTML demo accepts only loopback Rep review URLs", () => {
  assert.equal(
    extractReviewUrl(
      "Review URL: http://127.0.0.1:43117/sufficiently-long-token/\n",
    ),
    "http://127.0.0.1:43117/sufficiently-long-token/",
  );
  assert.equal(
    extractReviewUrl("Review URL: https://example.com/not-local\n"),
    undefined,
  );
});

test("Claude HTML demo maps browser time onto the cropped VHS timeline", () => {
  assert.equal(
    browserOverlayOffsetSeconds(
      {
        browserStartEpochMs: 75_000,
        planReadyEpochMs: 60_000,
      },
      4_000,
    ),
    19,
  );
  assert.equal(
    browserOverlayOffsetSeconds(
      {
        browserStartEpochMs: 59_000,
        planReadyEpochMs: 60_000,
      },
      500,
    ),
    0,
  );
  assert.throws(
    () => browserOverlayOffsetSeconds({}, 4_000),
    /must be finite numbers/,
  );
});

test("Claude HTML demo identifies its native Chromium process and window", () => {
  assert.equal(
    browserProcessId([
      { type: "renderer", id: 15 },
      { type: "browser", id: 42 },
    ]),
    42,
  );
  assert.deepEqual(
    parseNativeBrowserCapture(`${JSON.stringify(browserCapture)}\n`),
    browserCapture,
  );
  assert.throws(() => browserProcessId([]), /browser process id/);
  assert.throws(() => parseNativeBrowserCapture(""), /headed Chromium window/);
  assert.throws(
    () =>
      parseNativeBrowserCapture(
        JSON.stringify({ ...browserCapture, displayWidth: 100 }),
      ),
    /headed Chromium window/,
  );
  assert.equal(
    browserCropFilter(browserCapture),
    "crop=w='floor(iw*1120/1512/2)*2':h='floor(ih*780/982/2)*2':x='round(iw*40/1512)':y='round(ih*40/982)'",
  );
});

test("Claude HTML demo safely builds the macOS recorder arguments", () => {
  assert.deepEqual(
    nativeRecorderArguments({
      displayNumber: 1,
      output: "/tmp/browser.mov",
      ready: "/tmp/browser.ready",
      stop: "/tmp/browser.stop",
    }),
    ["1", "/tmp/browser.mov", "/tmp/browser.ready", "/tmp/browser.stop"],
  );
  assert.throws(
    () =>
      nativeRecorderArguments({
        displayNumber: 0,
        output: "/tmp/browser.mov",
        ready: "/tmp/browser.ready",
        stop: "/tmp/browser.stop",
      }),
    /display number/,
  );
});

test("Claude HTML demo confirms intentional Rep changes when Claude asks", () => {
  assert.equal(
    shouldConfirmRepSuggestions(
      "Would you like me to apply these rep suggestions, or keep the plan as-is?\n❯ Keep the original text.",
    ),
    true,
  );
  assert.equal(
    shouldConfirmRepSuggestions("Applying the captured Rep suggestions."),
    false,
  );
});

test("Claude HTML demo swaps a deterministic plan and hides before cleanup", () => {
  assert.doesNotThrow(() => validateGeneratedHtml(original));
  assert.doesNotThrow(() => validateOriginalHtml(deterministicPlan));
  assert.throws(
    () => validateGeneratedHtml("<html><body>short</body></html>"),
    /Claude-generated draft/,
  );
  assert.match(
    recorderScript,
    /CLAUDE_PLAN_PROMPT="Create a polished, responsive, valid standalone HTML5 rollout plan for checkout recovery and write it to demo-plan\.html\."/,
  );
  const revisionReady = recorderTape.indexOf(
    "Wait+Screen@300s /revision-ready/",
  );
  const hidden = recorderTape.indexOf("\nHide\n", revisionReady);
  const cleanupQuit = recorderTape.indexOf('Type "/quit"', revisionReady);
  assert.ok(revisionReady >= 0);
  assert.ok(hidden > revisionReady);
  assert.ok(cleanupQuit > hidden);
});

test("Claude HTML demo records the q handoff without owning the skill browser", () => {
  assert.match(recorderScript, /REP_BROWSER_MANAGED_EXTERNALLY=1/);
  assert.match(recorderScript, /REP_DIAGNOSTICS_FILE=/);
  assert.match(
    recorderOrchestrator,
    /args: \["--window-position=40,40", "--window-size=1120,780"\]/,
  );
  assert.match(
    recorderOrchestrator,
    /viewport: \{ width: 1120, height: 620 \}/,
  );
  assert.match(recorderOrchestrator, /keyboard\.press\("q"\)/);
  assert.match(recorderOrchestrator, /locator\("#completion"\)\.waitFor\(\)/);
  assert.match(
    recorderOrchestrator,
    /cue\.style\.bottom = `\$\{Math\.ceil\(hudHeight\) \+ 18\}px`/,
  );
  assert.doesNotMatch(recorderOrchestrator, /locator\("#submit"\)/);
});

test("Claude HTML demo visibly types browser annotations and pauses before Rep", () => {
  assert.doesNotMatch(
    recorderOrchestrator,
    /locator\("#modal-input"\)[\s\S]{0,80}\.fill\(/,
  );
  assert.match(
    recorderOrchestrator,
    /locator\("#modal-input"\)[\s\S]{0,80}\.pressSequentially\(text, \{ delay: BROWSER_TYPING_DELAY_MS \}\)/,
  );
  assert.match(recorderOrchestrator, /const BROWSER_TYPING_DELAY_MS = 35;/);
  assert.match(
    recorderTape,
    /Sleep 2500ms\s+Set TypingSpeed 0\.07\s+Type "\/rep @demo-plan\.html"/,
  );
});

test("Claude HTML demo verifies both actions and preserved layout structure", () => {
  assert.doesNotThrow(() => validateOriginalHtml(original));
  assert.doesNotThrow(() =>
    validateOriginalHtml(
      original
        .replace("<!doctype html>", "<!DOCTYPE html>")
        .replace('class="page"', "class='page'")
        .replace('id="ownership"', "id='ownership'")
        .replace('id="launch-gate"', "id='launch-gate'"),
    ),
  );
  assert.doesNotThrow(() => validateRevisedHtml(original, revised));
  assert.throws(
    () => validateOriginalHtml("<html></html>"),
    /Invalid deterministic demo plan/,
  );
  assert.throws(
    () => validateRevisedHtml(original, original),
    /did not change the plan.*launch-gate change.*ownership feedback/s,
  );
});
