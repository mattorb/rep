---
name: rep
description: Run rep against a Markdown or local HTML plan, capture the fresh review action list, and apply the requested edits to that plan in the same turn. Use when a user asks to review, annotate, or update a plan or roadmap through rep; retain the Markdown TUI and route .html/.htm plans to the browser UI.
---

# Rep Plan Updater

Run rep, wait for the interactive review to finish, then apply only the newly
captured actions to the source plan.

## Non-Negotiable Rules

1. Execute a fresh run for every invocation. Never reuse prior output.
2. Resolve the source as Markdown or HTML before launch:
   - `.html` and `.htm`, case-insensitively: HTML browser review.
   - `.md`, `.markdown`, or an extensionless filename: Markdown TUI review.
   - any other extension or an ambiguous path: stop before launch.
3. Do not edit unless this turn produces a new `REP_CAPTURE_FILE=...` path.
4. Parse actions only from that capture file.
5. Launch `run_rep_and_capture.sh` without forcing a PTY. For HTML, the runner
   owns an isolated temporary browser session and closes it after receiving
   the capture.
6. Keep polling indefinitely until the foreground process exits. Quiet output
   means the user is still reviewing; it is not evidence of a hang.
7. Never inspect, drive, kill, or send keys to rep's tmux panes, windows,
   browser, or server. Let the runner perform its scoped browser cleanup.
8. If launch or rep exits non-zero, stop and report the failure.
9. Treat an empty capture as silent discard and make no edits. Treat
   `No actions.` as a completed review with no edits.
10. Include the capture path and the full captured output verbatim in the
    final response. For silent discard, explicitly state that the capture was
    empty.

## Workflow

1. Resolve the plan path.
2. From this skill directory, run:
   - Markdown: `scripts/run_rep_and_capture.sh <plan-file>`
   - HTML: `scripts/run_rep_and_capture.sh <plan-file> --web`
   - Use `scripts/plan_mode.sh <plan-file>` when deterministic format routing
     is useful.
3. Start with a short yield (about 200–500 ms), then poll the same process
   until it exits and emits `REP_CAPTURE_FILE=...`. In HTML mode, the user
   presses `q` and confirms; the page shows the handoff state, and the runner
   closes its temporary browser before emitting the capture marker.
4. Read only that capture:
   - empty: report silent discard;
   - `No actions.`: report no edits;
   - otherwise process every `ACTION:` block in emitted order.
5. Apply edits to the original plan and re-open it to verify structure and
   requested visible text.
6. Return the capture path and full captured output.

## Common Action Semantics

- Treat `WHERE: line N` as a hint, never as sole identity.
- Confirm `CONTEXT.target`; use `prev` and `next` to disambiguate.
- `change`: replace only the target with `CHANGE`.
- `revise-to-incorporate-feedback`: treat `FEEDBACK` as intent, not literal
  replacement text.
- `insert-before` / `insert-after`: insert `INSERT` at the requested side of
  the target while preserving the surrounding structure.
- `delete this`: remove only the selected unit.
- Stop and ask when the locator and context do not identify one source
  location.

## Markdown Rules

Locate the exact target near the line hint, preserve indentation, list
markers, numbering, fences, and surrounding Markdown, and keep neighboring
context coherent. For section actions, respect the heading boundary described
by the captured target and context.

## HTML Rules

HTML blocks contain `FORMAT: html` and `LOCATOR:`.

1. Resolve `LOCATOR` against the original HTML first:
   - prefer its unique original `#id`;
   - otherwise follow the tag/`:nth-of-type` path;
   - use `::text-fragment(N)` to identify the emitted visible fragment when
     one element owns several fragments.
2. Confirm the exact normalized visible `CONTEXT.target`. Decode entities and
   use neighboring visible context to disambiguate repeated text.
3. Preserve indentation, element structure, attributes, CSS classes, and
   unrelated inline markup.
4. For a word or sentence spanning inline elements, edit the smallest
   containing element necessary. Retain meaningful emphasis, link, code, and
   other inline markup when compatible with the requested result.
5. Delete a paragraph by removing only its owning review fragment or element.
   Delete a section from its heading through the emitted equal-or-shallower
   boundary, never through a nested or following sibling section.
6. Insert structurally valid HTML for the container: list content remains in
   `li`, and table content remains in the appropriate cell/row structure.
7. Never search for or add transient `data-rep-*` IDs; they exist only in the
   served review copy.
8. If locator, target, and neighboring context do not resolve uniquely, stop
   and ask instead of guessing.

## Runner Scripts

- `scripts/run_rep_and_capture.sh`: required foreground runner, fresh capture
  writer, and owner of the temporary HTML browser lifecycle; accepts extra rep
  arguments after the plan path.
- `scripts/browser_session.sh`: launches an isolated HTML review window and
  terminates only that temporary browser profile when the runner signals.
- `scripts/plan_mode.sh`: deterministic Markdown/HTML/unsupported classifier.
- `scripts/rep.sh`: executable resolver for direct/manual debugging.

`rep.sh` resolves `REP_BIN`, nearby release/debug binaries, `rep` on `PATH`,
then a nearby Cargo package named `rep`.
