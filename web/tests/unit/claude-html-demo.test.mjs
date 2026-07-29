import assert from "node:assert/strict";
import test from "node:test";

import {
  browserOverlayOffsetSeconds,
  browserProcessId,
  extractReviewUrl,
  parseNativeWindowId,
  validateOriginalHtml,
  validateRevisedHtml,
} from "../record-claude-html-demo.mjs";

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
  assert.equal(parseNativeWindowId("731\n"), 731);
  assert.throws(() => browserProcessId([]), /browser process id/);
  assert.throws(() => parseNativeWindowId(""), /headed Chromium window/);
  assert.throws(() => parseNativeWindowId("12\n13\n"), /headed Chromium window/);
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
    /Invalid Claude-created plan/,
  );
  assert.throws(
    () => validateRevisedHtml(original, original),
    /did not change the plan.*launch-gate change.*ownership feedback/s,
  );
});
