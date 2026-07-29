# Agent Guidance

- Use `mise exec -- <command>` for Rust, Node, and browser-test commands when
  `mise.toml` is present.
- Run `./build.sh` before submitting code changes.
- Install locked web dependencies with `mise exec -- npm --prefix web ci` and
  run `mise exec -- npm --prefix web run test:e2e` for HTML web UI changes.
- Keep overall code coverage level >=80%, focused on the most critical and riskiest areas.
- Keep public API additions narrow; prefer `pub(crate)` unless integration tests or binary boundaries require public access.
- Do not edit generated artifacts or caches.
- Keep release support claims aligned with CI and installer behavior.
