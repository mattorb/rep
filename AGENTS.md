# Agent Guidance

- Use `mise exec -- <command>` for Rust commands when `mise.toml` is present.
- Run `./build.sh` before submitting code changes.
- Keep overall code coverage level >=80%, focused on the most critical and riskiest areas.
- Keep public API additions narrow; prefer `pub(crate)` unless integration tests or binary boundaries require public access.
- Do not edit generated artifacts or caches.
- Keep release support claims aligned with CI and installer behavior.
