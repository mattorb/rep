# Contributing

Thanks for improving `rep`. Keep changes focused and include tests for behavior changes.

## Local Checks

Run:

```sh
./build.sh
cargo doc --no-deps
cargo package --allow-dirty
cargo publish --dry-run
```

Run `cargo audit` and `shellcheck` when those tools are installed.

## Pull Requests

- Explain the user-facing behavior change.
- Include screenshots, terminal captures, or fixture diffs for TUI changes.
- Keep generated files and caches out of commits.
- Do not broaden platform support claims without CI coverage or an explicit support tier.
