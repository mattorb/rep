# Agent Guidance

- Use `mise exec -- <command>` for Rust, Node, and browser-test commands when
  `mise.toml` is present.
- Run `./build.sh` before submitting code changes.
- Install locked web dependencies with `mise exec -- npm --prefix web ci` and
  the browser with `mise exec -- npm --prefix web exec -- playwright install
  chromium`. With both present, `./build.sh` runs the browser tests itself,
  minus the `@gallery` captures. Run `mise exec -- npm --prefix web run gallery`
  when an HTML web UI change should refresh the committed screenshots.
- Keep overall code coverage level >=80%, focused on the most critical and riskiest areas.
- Browser-side JavaScript has its own floor: `npm --prefix web test` enforces
  line, branch, and function thresholds over `src/web/*.js`. Only modules a unit
  test imports are measured, so a new browser module needs a unit test to count.
  `src/web/document.js` is unit-tested through the stub DOM in
  `web/tests/unit/dom-stub.mjs`; jsdom cannot serve here because it has no
  layout engine and the extractor's visibility filter depends on client rects.
- Keep public API additions narrow; prefer `pub(crate)` unless integration tests or binary boundaries require public access.
- Do not edit generated artifacts or caches.
- Keep release support claims aligned with CI and installer behavior.
