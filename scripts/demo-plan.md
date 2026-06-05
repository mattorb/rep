# Open Source Release Plan

Ship `rep` as a small, reliable tool for reviewing Markdown plans with an agent.

## Blockers

- Bundle the agent skill in release archives so binary installers can hand off actions.
- Keep platform support honest by publishing only artifacts covered by CI.
- Check that release tags match the Cargo package version before packaging.

## Follow-up

Add focused snapshot tests for the main TUI states.
