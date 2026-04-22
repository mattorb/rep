---
name: rep
description: Run rep against a Markdown plan file, interpret the emitted action list, and apply the requested edits to the plan in the same turn. Use when a user asks to run rep on a plan or roadmap file and then update that file based on rep output.
---

# Rep Plan Updater

Run rep, capture fresh output, and apply requested edits directly to the plan file.

## Non-Negotiable Rules

1. Execute a fresh rep run every invocation; never reuse prior conversation output.
2. Do not edit any file unless the current turn produced a new `REP_CAPTURE_FILE=...` path.
3. Parse only that capture file for actions.
4. If rep fails to launch or exits non-zero, stop and report the failure instead of guessing.
5. For tool execution, launch `run_rep_and_capture.sh` without forcing PTY so rep can trigger its own fallback launcher (tmux/new terminal window) immediately, then poll.
6. In the final assistant response, include the full captured rep output from that `REP_CAPTURE_FILE` in a fenced code block (not just a summary).

## Workflow

1. Resolve the plan file path from the user request.
2. Run (default/non-PTY):
   - `scripts/run_rep_and_capture.sh <plan-file>`
   - Use short initial `yield_time_ms` (around 200-500ms) so the command starts immediately, then poll until completion.
3. Read the emitted `REP_CAPTURE_FILE` path.
4. Parse action blocks from that capture file:
   - If file contains `No actions.`, stop and report no edits.
   - Otherwise process each `ACTION:` block in order.
5. Apply edits to the same plan file.
6. Re-open and sanity-check the modified file to confirm edits landed correctly.
7. In the response, include:
   - the capture file path used for parsing
   - the full captured rep output (verbatim) from the capture file

## Action Handling Rules

Use these rules for each block from rep output.

### `ACTION: change`

1. Use `WHERE` line number as a hint, not the sole source of truth.
2. Locate `CONTEXT.target` sentence text; if line hint is stale, search nearby lines.
3. Replace only that targeted sentence/text span with the `CHANGE` value.
4. Preserve surrounding Markdown structure, indentation, and list formatting.
5. If target text cannot be located unambiguously, stop and ask before risky edits.

### `ACTION: revise-to-incorporate-feedback`

1. Use `WHERE` line number as a hint, not the sole source of truth.
2. Locate `CONTEXT.target` sentence text; if line hint is stale, search nearby lines.
3. Treat `FEEDBACK` as intent, not a literal replacement string.
4. Revise the targeted sentence/text span to incorporate that feedback while preserving local structure and numbering.
5. Keep nearby context coherent (`prev`/`next`) and preserve Markdown/list formatting.
6. If the intended revision is ambiguous, stop and ask before risky edits.

### `ACTION: delete this`

1. Use `WHERE` line number as a hint, not the sole source of truth.
2. Prioritize `CONTEXT.target` exact text match on that line.
3. If line hint is stale, search nearby lines for the same `target` text.
4. Remove only the targeted sentence/text span.
5. Preserve surrounding Markdown structure and list formatting.

If target text cannot be located unambiguously, stop and ask the user before making a risky edit.

## Runner Scripts

Use these scripts from this skill directory:

- `scripts/run_rep_and_capture.sh` for normal operation (required by this skill)
- `scripts/rep.sh` for direct/manual debugging

`rep.sh` resolves rep with this precedence:

1. `REP_BIN` environment variable (if executable)
2. nearest `target/release/rep` or `target/debug/rep`
3. `rep` found on `PATH`
4. `cargo run -- <plan-file>` in the nearest Cargo package named `rep`

Examples:

- `scripts/run_rep_and_capture.sh OPENSOURCE_PLAN.md`
