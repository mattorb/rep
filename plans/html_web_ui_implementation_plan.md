# HTML Web UI Implementation Plan

## Goal

Add a local browser review experience for HTML plan files without changing
Rep's existing Markdown TUI experience.

The finished command contract is:

```text
rep plan.md           # existing Markdown TUI
rep --web plan.html   # local browser review for HTML
```

The HTML review must:

- render the plan with its original HTML layout and CSS;
- retain Rep's selection units, annotation types, keyboard-driven workflow,
  search, annotation jumping, help, link inspection, copy, `q` completion, and
  silent-discard behaviors wherever the concepts apply to rendered HTML;
- print the same human-readable action protocol to stdout when the user
  presses `q` and confirms;
- work with the bundled Rep skill so the invoking agent waits for the browser
  review, captures the output, closes the temporary browser it launched,
  applies the requested HTML edits, and then returns to the user;
- keep the web server local, session-scoped, authenticated by an unguessable
  URL token, and alive only until the review is sent, discarded, aborted, or
  timed out;
- render document-provided CSS and local assets while preventing the reviewed
  HTML from executing JavaScript or controlling Rep's application shell.

This plan intentionally favors small, gated iterations. Every stage leaves the
repository buildable and testable. A stage is complete only after its gate
passes; do not begin the next stage while a gate is red.

## Fixed Product Decisions

These are implementation requirements, not open questions.

### Development branch policy

- All implementation, tests, fixtures, documentation, generated review
  artifacts, and follow-up fixes for this plan happen on the
  `html_and_browser_support` branch.
- Before beginning any stage or resuming work, verify that the checked-out
  branch is `html_and_browser_support`. Do not make feature commits directly
  on `main`.
- Treat `html_and_browser_support` as the shared integration branch for every
  stage in this plan. Commit and push each completed, passing stage there.
- When the branch needs updates from `main`, merge `origin/main` into
  `html_and_browser_support` and rerun the current stage gate. Do not rewrite
  or force-push the shared branch.
- Merge to `main` only after the Stage 9 final acceptance gate passes and the
  complete branch is reviewed.

### Input and mode routing

- `rep <path>` keeps today's behavior and treats the input as Markdown.
- A path ending in `.html` or `.htm` (case-insensitive) without `--web` exits
  non-zero and says that HTML plans require `--web`.
- `rep --web <path>` accepts only `.html` and `.htm`.
- `rep --web <markdown-or-other-file>` exits non-zero before opening a browser
  or binding a server.
- Existing extensionless and non-`.md` Markdown use remains supported when
  `--web` is absent; only `.html` and `.htm` receive special routing.
- `--web` conflicts with `--demo`. The built-in demo remains Markdown/TUI-only.
- `--web --debug <file.html>` prints web diagnostics and exits without binding
  a server or launching a browser.
- Add `--no-open`, valid only with `--web`, to start the server and print the
  review URL to stderr without invoking an OS browser command. This is a
  supported headless/testing/manual-open path, not a hidden test hook.
- Flags are accepted before or after the positional path so the existing
  skill runner can continue to append arguments after the plan path.

### HTML rendering and security

- The reviewed HTML renders inside a sandboxed iframe. Rep's toolbar, modals,
  help, status, and completion UI remain in the parent document.
- The iframe keeps original document markup and CSS as closely as the safety
  boundary permits.
- Document JavaScript never executes. Do not add a switch to enable it in this
  implementation.
- Inline `<style>` elements and `style` attributes are supported.
- Local linked stylesheets, images, and fonts are supported only when their
  canonical paths remain beneath the HTML plan's containing directory.
- Data-URI images and fonts are supported.
- Remote network resources, forms, nested frames, plugins/objects, media
  capture, downloads, and navigation are blocked.
- Root-relative asset URLs such as `/assets/app.css` are unsupported because
  they do not have a safe filesystem-root interpretation. The UI reports
  blocked resources in a non-modal status area.
- Original `<script>`, `<base>`, CSP meta, refresh meta, and inline event
  handler attributes are removed from the served review copy. The source file
  itself is never rewritten by the server.
- The server injects internal source IDs into the served review copy, but those
  IDs are never written to the source file and are not emitted as the sole
  locator in review output.
- The HTML source must be UTF-8 and no larger than 10 MiB. Larger or invalid
  UTF-8 files fail before launch with a clear error.

### HTML review semantics

- Review order is DOM order, not visual left-to-right/top-to-bottom geometry.
- Hidden content is not selectable. A node is hidden when it or an ancestor is
  `display: none`, `visibility: hidden|collapse`, the `hidden` attribute is
  present, or it has no rendered text rectangles after layout.
- CSS generated content (`::before`/`::after`), canvas pixels, image pixels,
  and shadow-DOM content owned by the plan are not selectable in this version.
- Browser extraction groups visible text under its nearest rendered block
  owner. Explicit semantic blocks (`h1`-`h6`, `p`, `li`, `pre`, `td`, `th`,
  `dt`, `dd`, `figcaption`, and `summary`) win over generic computed
  block-like containers. Direct text not covered by one of those elements is
  grouped under the nearest element whose computed display is block,
  list-item, table-cell, flex, grid, or flow-root.
- Nested semantic elements do not duplicate text. Every visible DOM text node
  belongs to exactly one review node.
- Whitespace is normalized to one space except that `<br>` and newlines in
  preformatted content produce a logical `\n`.
- Selection units retain the existing cycle and keys:
  `Section -> Paragraph -> Line -> Sentence -> Word`.
- In HTML:
  - **Section** starts at each heading and ends immediately before the next
    equal-or-shallower heading. Pre-heading content is a section only when a
    later heading exists. A heading-less document has no Section anchors.
    Top-level ordered lists before the first heading retain the existing Rep
    rule and form one section per contiguous ordered list.
  - **Paragraph** means one extracted HTML review node, even when its owning
    element is a list item, table cell, or generic block container.
  - **Line** means a logical line separated by `<br>` or a preformatted
    newline. CSS viewport wrapping never creates Line anchors. A block without
    an explicit logical break has one Line anchor.
  - **Sentence** uses Rep's existing canonical Rust sentence segmenter over the
    normalized review-node text.
  - **Word** uses Rep's existing canonical Rust word segmenter.
- Keyboard movement, unit cycling, finer/coarser adjustment, search, annotation
  jumps, and boundary behavior are server-authoritative and use the same
  review-session implementation as the TUI.
- Mouse behavior mirrors the TUI:
  - single click selects a word;
  - double click selects a sentence when the node has sentence semantics,
    otherwise its logical line;
  - triple click selects the paragraph/review node.
- Current selection and annotation marks are rendered as non-interactive
  overlay rectangles in an isolated shadow root. Do not wrap or rewrite the
  plan's DOM text to highlight it; DOM mutation can break the page's original
  CSS and layout.

### HTML output and edit targeting

- Markdown output remains byte-for-byte compatible with the current output
  fixtures.
- HTML uses the existing action names and payload keys:
  `change`, `revise-to-incorporate-feedback`, `insert-before`,
  `insert-after`, and `delete this`.
- HTML output adds `FORMAT: html` after `FILE:` and a `LOCATOR:` line in each
  action block. Markdown output does not add either line.
- `WHERE: line N` for HTML is the one-based source line of the owning source
  element's start tag. It is a hint, just as Markdown line numbers are hints.
- `LOCATOR:` is generated from the original DOM:
  1. a unique original `id` selector when available;
  2. otherwise a stable CSS path using tag names and `:nth-of-type`;
  3. append `::text-fragment(N)` when one element owns multiple separately
     extracted review-node fragments.
- `CONTEXT.prev`, `target`, and `next` use neighboring visible review-node
  text, not neighboring raw HTML source lines.
- The target is the exact normalized visible text captured when the
  annotation is created. Annotation capture is immutable: later navigation
  or viewport changes do not change it.
- Output ordering remains deterministic by document order and then annotation
  creation order.
- A completed review with no actions prints the existing `No actions.` form.
- Silent discard produces no action output, matching the TUI's `Q` behavior.

### Browser and server lifecycle

- Bind only to IPv4 loopback `127.0.0.1` on an OS-assigned port.
- Generate a fresh 256-bit random session token for every run and include it
  in every application, API, document, and asset URL.
- Print startup information and the review URL to stderr. Reserve stdout for
  the final action protocol.
- On macOS, launch with `open`. On Linux, try `xdg-open`, then `gio open`.
  Browser command execution is abstracted behind an injectable launcher so
  tests never open a real browser.
- If browser launch fails, keep the server alive, print the URL and manual-open
  instructions to stderr, and continue normally.
- `q` asks for confirmation and then sends annotations back to the caller.
- `Q` silently discards without confirmation, matching the TUI.
- Do not expose separate Submit or Discard buttons; the persistent bottom HUD
  advertises `q` for completion and `?` for help.
- After `q` confirmation, the browser shows “Sending feedback to Rep skill”
  and explains that the tab closes automatically after receipt. The page
  updates the message when the finish response is acknowledged and makes a
  best-effort `window.close()` call.
- The bundled skill runner starts Rep with `--no-open`, parses the authenticated
  loopback URL from diagnostics, and launches a dedicated temporary Chromium-
  or Firefox-family profile. When Rep exits with the fresh capture, the runner
  closes only that managed browser process before emitting
  `REP_CAPTURE_FILE`.
- Explicit `--no-open` is still the manual/SSH mode and does not claim managed
  browser ownership. Demo automation can set an internal external-ownership
  switch when its recorder already owns the headed browser.
- If the skill runner cannot resolve a supported browser executable, it fails
  preflight before starting Rep so it cannot orphan a local server.
- After finish/discard, the browser shows a self-contained completion state.
  The foreground Rep process closes the listener, joins the server thread,
  prints output when appropriate, cleans session memory, and exits.
- Closing or reloading the tab does not implicitly discard. The server keeps
  the in-memory session so reopening the same URL restores the current review.
- A session with no HTTP or browser-heartbeat activity for 24 hours exits
  non-zero with a timeout error. This matches the existing long-running
  interactive fallback expectation while preventing permanent orphan servers.
- SIGINT/SIGTERM stop the server and exit non-zero without printing a partial
  action list.

## Non-Goals

- A Markdown web UI.
- An HTML TUI.
- Executing or debugging scripts embedded in the reviewed HTML.
- Editing the HTML file directly from the browser.
- Automatically applying annotations inside the Rep server.
- Reviewing arbitrary live URLs.
- Serving assets outside the HTML file's directory.
- Supporting multiple source files or multiple simultaneous review sessions
  in one Rep process.
- Pixel, SVG-path, canvas, or image-region annotations.
- Exact preservation of behavior that depends on JavaScript.
- Closing or manipulating an existing user browser session. Automatic closure
  is guaranteed only for the isolated process owned by the skill runner;
  direct/manual tabs use best-effort self-closure.
- Windows support in this feature; release support remains aligned with the
  current macOS/Linux matrix.

## Target Architecture

### Process flow

```text
agent skill
  -> run_rep_and_capture.sh plan.html --web
       -> preflight a supported browser executable
       -> append --no-open
       -> foreground rep process
       -> validate file and mode
       -> transform an in-memory served copy
       -> bind 127.0.0.1:0 and generate session token
       -> print authenticated review URL
  -> runner launches an isolated temporary browser profile
       -> browser loads parent app + sandboxed document iframe
       -> browser posts visible-text manifest
       -> Rust builds HtmlReviewDocument + shared ReviewSession
       -> browser sends navigation/annotation commands
       -> Rust returns authoritative state snapshots
       -> browser paints selections/annotations as overlays
       -> q + confirmation, or silent discard
       -> show feedback handoff state
       -> HTTP response completes
       -> server shuts down
       -> rep prints actions to stdout and exits
  -> runner closes only its isolated browser process
  -> capture file becomes available
  -> agent applies requested edits to the original HTML
```

The server is not detached and normal completion remains graceful shutdown of
the foreground Rep process. The runner tracks only the separate browser process
and unique profile that it created, allowing it to close the temporary review
without touching the user's existing browser windows.

### Shared review core

Create a UI-neutral `src/review/` module. It owns behavior that must remain
consistent between the Markdown TUI and HTML web UI:

```text
src/review/
  mod.rs
  annotation.rs       annotation types and AnnotationStore
  command.rs          navigation/search/annotation commands
  document.rs         ReviewDocument trait and source-locator types
  session.rs          ReviewSession state machine
  emit.rs             EmitModel construction from a session
```

`ReviewDocument` supplies document-specific facts while `ReviewSession` owns
interaction semantics. Keep the trait crate-private and narrow:

```rust
pub(crate) trait ReviewDocument {
    fn source_path(&self) -> &Path;
    fn format(&self) -> DocumentFormat;
    fn selection_index(&self) -> &SelectionIndex;
    fn initial_anchor(&self) -> SelectionAnchor;
    fn node_count(&self) -> usize;
    fn has_any_anchor(&self, unit: SelectionUnit) -> bool;
    fn section_span(&self, start_node: usize) -> Range<usize>;
    fn capture_target(&self, anchor: SelectionAnchor) -> TargetCapture;
    fn action_context(&self, target: &CapturedTarget) -> ActionContext;
    fn search_matches(&self, query: &str) -> Vec<(usize, usize)>;
    fn links_for(&self, anchor: SelectionAnchor) -> Vec<ReviewLink>;
    fn node_outline(&self) -> Vec<OutlineRow>;
}
```

Use these trait and method names as the implementation contract; introduce
strictly internal ownership helpers where Rust requires them, but do not widen
the public crate API. `ReviewSession` contains:

- canonical `SelectionState`;
- section highlight range;
- `AnnotationStore`;
- last search and search-jump behavior;
- navigation boundary feedback;
- move/cycle/adjust/jump/search methods;
- annotation add/edit/clear/strike operations;
- capture-at-creation semantics;
- conversion to `EmitModel`.

Frontend-only concerns remain outside the shared core:

- TUI terminal setup, Ratatui layout, input buffers, popups, key-event decoding,
  mouse coordinates, and clipboard backends remain in `src/app/` and `src/ui/`.
- Browser DOM extraction, DOM offset maps, overlays, HTML modals, fetch calls,
  and Clipboard API handling remain in the embedded web assets.
- HTTP routing, authentication, CSP, assets, browser launch, timeout, and
  shutdown remain in `src/web/`.

### Neutral selection index input

Refactor `SelectionIndex` so it can be built from a neutral list of selection
nodes in addition to the existing Markdown `Document`.

```rust
pub(crate) struct SelectionNodeInput {
    pub kind: SelectionNodeKind,
    pub plain_text: String,
    pub logical_line_ranges: Vec<Range<usize>>,
    pub source_lines: Vec<usize>,
    pub heading_level: Option<u8>,
    pub pre_heading_ordered_list_id: Option<u64>,
}
```

- `SelectionIndex::build(&Document, ...)` becomes a Markdown adapter that
  produces `SelectionNodeInput` values and delegates to one neutral builder.
- Sentence and word segmentation remain in Rust and remain canonical.
- Section construction is generalized from Markdown `DocNode` variants to
  neutral heading/list metadata without changing current Markdown behavior.
- HTML manifest nodes become `SelectionNodeInput` values after server-side
  validation.
- Byte ranges remain the canonical internal ranges. Web state responses also
  include Unicode scalar-value start/end offsets derived from those byte
  ranges, because JavaScript DOM offsets use UTF-16 and cannot consume Rust
  byte offsets directly.

### Web modules and embedded assets

Add:

```text
src/web/
  mod.rs
  server.rs            listener, routes, request limits, response headers
  session.rs           AwaitingManifest/Active/Finished lifecycle
  protocol.rs          serde request/response models
  security.rs          token/origin/host/path validation and CSP
  html_source.rs       tokenization, source IDs, sanitizing transform
  assets.rs            confined local asset resolution and MIME allowlist
  browser.rs           macOS/Linux browser launcher

web/
  index.html            parent application shell
  app.css               Rep-owned shell/modal/status styling
  app.js                bootstrap and command dispatch
  document.js           iframe extraction and text/DOM range maps
  overlay.js            shadow-root rectangle overlays
  input.js              keyboard and mouse event normalization
  protocol.js           fetch helpers and response validation
  test/                 JavaScript unit tests and Playwright specifications
```

Use vanilla JavaScript modules with no runtime framework or bundler. Embed the
assets with `include_str!`/`include_bytes!` so release archives still contain a
single executable. Add a small development-only script that verifies every
asset referenced by `index.html` is embedded.

Selected Rust dependencies:

- `serde` with derive and `serde_json` for the browser protocol;
- `tiny_http` for the loopback HTTP server and explicit unblocking during
  shutdown;
- `getrandom` for the 256-bit session token;
- `html5tokenizer` for spec-oriented HTML tokenization with source spans;
- `percent-encoding` for strict asset URL decoding;
- `mime_guess` for response content types, followed by Rep's own allowlist;
- `signal-hook` for coordinated SIGINT/SIGTERM shutdown without emitting a
  partial review result.

Do not add an async runtime. One foreground coordinator thread and one server
thread are sufficient for a single local session. Route handlers serialize
mutations through `Arc<Mutex<WebSessionState>>`; poison errors become clean
500 responses and terminate the session rather than panicking across threads.

### HTTP surface

All paths include the session token:

```text
GET  /s/<token>/                         parent app shell
GET  /s/<token>/app.css
GET  /s/<token>/app.js
GET  /s/<token>/document.js
GET  /s/<token>/overlay.js
GET  /s/<token>/input.js
GET  /s/<token>/protocol.js
GET  /s/<token>/assets/__rep_document__.html
GET  /s/<token>/assets/<relative-path>
GET  /s/<token>/api/state
POST /s/<token>/api/manifest
POST /s/<token>/api/command
POST /s/<token>/api/annotate
POST /s/<token>/api/edit
POST /s/<token>/api/finish
POST /s/<token>/api/discard
POST /s/<token>/api/heartbeat
```

Contracts:

- Unknown paths return 404 without revealing whether another token exists.
- Wrong/missing tokens return 404, not 401.
- Only `GET` and listed `POST` methods are accepted.
- Mutation requests require `Content-Type: application/json`, an exact local
  `Origin`, and a body at or below the route's configured limit.
- Do not send CORS headers.
- Reject Host headers other than the exact bound loopback host/port.
- Set `Cache-Control: no-store`, `Referrer-Policy: no-referrer`,
  `X-Content-Type-Options: nosniff`, and a restrictive CSP on Rep-owned pages.
- Iframe document responses set a separate CSP that allows original inline and
  local styles/images/fonts but denies scripts, connections, forms, objects,
  frames, workers, and remote origins.
- The iframe carries `sandbox="allow-same-origin"` and no other sandbox
  capabilities. Keeping same-origin lets the trusted parent inspect and
  highlight the frame DOM; omitting `allow-scripts`, `allow-forms`,
  `allow-popups`, and navigation capabilities keeps plan behavior inert.
- A manifest may initialize the session once. A byte-for-byte equivalent
  manifest from a reloaded tab is idempotent. A conflicting second manifest
  returns 409 and leaves the original session active.
- Every successful mutation response includes a monotonically increasing
  revision. The browser ignores stale responses and requests `/api/state` if
  it detects a revision gap.

### Source transformation

The source transformer operates only on an owned in-memory string:

1. Tokenize the original HTML and retain byte spans and one-based source lines.
2. Remove any existing `data-rep-*` attributes from the served copy.
3. Assign a monotonically increasing numeric source ID to every explicit start
   tag that can own visible content.
4. Inject `data-rep-source-id="<N>"` into the served start tag.
5. Record source ID -> original tag, source start byte, source end byte,
   one-based start line, and selected original attributes.
6. Remove `<script>` elements, `<base>` elements, CSP/refresh meta elements,
   and inline event-handler attributes from the served copy.
7. Neutralize `javascript:` URLs and deny unsupported URL-bearing elements.
8. Preserve all other source bytes exactly when possible; do not parse and
   reserialize the entire DOM because reserialization can change CSS-sensitive
   structure and malformed-but-renderable HTML.

The transformed document is served at a URL ending in
`/assets/__rep_document__.html`, so normal relative asset URLs resolve beneath
the tokenized `/assets/` route without injecting a `<base>` element.
Root-relative URLs deliberately resolve outside the token route and receive
404.

If the selected tokenizer version cannot expose a safe insertion span for a
start tag, implement a small tracing emitter on top of its public tokenizer
API. Do not fall back to regex-based HTML rewriting. This is an implementation
constraint, not a future decision.

### Browser manifest and DOM mapping

After the iframe's `load` event:

1. Walk visible text nodes with `TreeWalker`.
2. Skip Rep overlay content, script/style/template content, hidden ancestors,
   and nodes with no rendered rectangles.
3. Assign each text node to exactly one block owner using the fixed ownership
   rules above.
4. Build review nodes in DOM order.
5. Normalize text while retaining a client-only boundary table from every
   normalized Unicode scalar offset to a concrete DOM
   `(textNode, UTF-16 offset)` start/end boundary.
6. Preserve explicit `<br>` and preformatted newlines as logical `\n`.
7. Generate the stable CSS locator from original IDs or tag/`:nth-of-type`
   ancestry, ignoring Rep's injected attributes.
8. Post only serializable facts to `/api/manifest`: node ID, source ID,
   fragment ordinal, tag, kind, heading level, normalized text, logical-line
   scalar ranges, locator, ordered-list identity, and link metadata.
9. The server validates counts, lengths, source IDs, heading levels, ranges,
   locators, and total manifest size before constructing `HtmlReviewDocument`.

State responses identify a highlight by review-node ID, selection unit,
unit index, and Unicode scalar start/end. The browser resolves those scalar
offsets through its boundary table and paints `Range.getClientRects()` results.
Recompute rectangles after iframe scroll, parent resize, font load, image load,
and a debounced `ResizeObserver` notification.

### Overlay and interaction

- Attach one Rep-owned overlay host to the iframe body and put all overlay
  elements inside a closed shadow root.
- Use `position: absolute`, `pointer-events: none`, and a reserved maximum
  z-index on the host. Rectangle geometry is document-relative.
- Paint current selection, strikes, changes, feedback, and insert markers with
  distinct accessible colors and patterns; do not rely on color alone.
- Put annotation badges at the first rectangle for the target and keep them
  non-interactive. Annotation editing is opened with `e`, `c`, or `f` in the
  parent app.
- Call `scrollIntoView({block: "center", inline: "nearest"})` on the owning
  element after keyboard jumps, then repaint after scrolling settles.
- Listen for key and mouse events in both the parent and iframe documents.
  Route both through one key normalizer.
- Prevent browser defaults for Rep-handled keys, especially Space,
  Backspace, arrows, `/`, and single-letter commands, except while a text
  input/textarea/modal editor owns focus.
- Preserve normal typing, selection, copy, and accessibility behavior inside
  modal inputs.
- Intercept link clicks in capture phase so the plan cannot navigate. `O`
  shows original and resolved link targets in a Rep-owned modal.

### Browser state and accessibility

- The Rust session is authoritative. The browser stores DOM boundary maps and
  presentation state only.
- Parent app status contains the current mode, node/unit position, server
  messages, and blocked-resource count.
- Modals trap focus, have labeled controls, use `role="dialog"`, restore focus
  on close, and support Escape.
- Help lists the same keymap as the README, with HTML-specific explanations
  for logical Line, DOM outline, and blocked link navigation.
- `I` opens a read-only HTML outline showing tag, id/class summary, source line,
  and normalized text preview. It is the HTML analogue of the Markdown AST
  view.
- `r` attempts `navigator.clipboard.writeText`. If permission is denied, show
  a modal containing selectable action output and a conventional Copy button.
- On reload, the parent fetches server state, rebuilds the manifest and DOM
  maps, accepts an idempotent manifest response, and restores the selection and
  annotations.

## Staged Implementation

## Stage 0 — Baseline Characterization and Fixtures

### Goal

Create a reliable safety net before moving state out of the TUI and establish
the HTML fixture corpus used by every later stage.

### Work

1. Run the current full validation command and record the baseline test count,
   snapshot count, and line coverage in the implementation PR/notes.
2. Add missing transcript tests for current Markdown behavior that the shared
   core will absorb:
   - forward/back movement in every unit;
   - Space/Backspace and `i`/`o` clamping;
   - search and `n`/`N`;
   - annotation creation/edit/clear/strike;
   - `[`/`]`;
   - q confirmation and Q discard;
   - empty document and boundary behavior;
   - emitted action ordering.
3. Add `tests/fixtures/web/` HTML source fixtures and local assets:
   - semantic headings/paragraphs/lists;
   - nested heading levels;
   - pre-heading content and pre-heading ordered lists;
   - inline markup splitting a sentence across text nodes;
   - repeated identical text;
   - `<br>` logical lines and `<pre>` newlines;
   - tables and nested lists;
   - CSS grid/flex layouts and generic div-based plans;
   - local stylesheet, image, SVG, and font references;
   - spaces and percent-encoded asset names;
   - hidden/invisible content;
   - Unicode, emoji, combining marks, entities, and RTL text;
   - malformed but browser-renderable HTML;
   - scripts, event handlers, forms, frames, base tags, refresh, remote URLs,
     `javascript:` URLs, root-relative paths, traversal paths, and symlinks
     escaping the fixture root;
   - an HTML file larger than the configured limit generated during the test
     rather than committed.
4. Add an expected manifest/output description beside each fixture. These can
   begin as hand-reviewed JSON/text fixtures and become executable in later
   stages.

### Tests

- Existing unit, integration, transcript, snapshot, and emit golden tests.
- New TUI characterization tests.
- Fixture inventory test asserting every HTML fixture has its expected
  metadata file and no fixture symlink accidentally resolves into the real
  workspace.

### Gate

```sh
mise exec -- ./build.sh
```

- All pre-existing snapshots and emit goldens are unchanged.
- Coverage remains at least 80%.
- No production behavior changes.

### Outcome

A frozen Markdown compatibility baseline and a reviewable HTML acceptance
corpus exist before architecture changes begin.

## Stage 1 — Extract the Shared Review Core Without Behavior Changes

### Goal

Separate interaction semantics from Ratatui/crossterm so the web UI can reuse
them, while proving that Markdown TUI behavior and output remain identical.

### Work

Perform this as small internal refactors, running the Stage 0 gate after each:

1. Move annotation structs and maps from `src/app/state.rs`/`src/app/mod.rs`
   into `review::annotation::AnnotationStore`. Preserve ordering and timestamp
   behavior.
2. Introduce `CapturedTarget`, `ActionContext`, `SourceLocator`, and
   `DocumentFormat`. Adapt `DocumentView` to implement the initial
   `ReviewDocument` trait.
3. Move canonical selection state, section highlight state, navigation,
   mode-cycle/adjust, search, and annotation jumping into `ReviewSession`.
4. Move annotation add/edit/clear/strike behavior into `ReviewSession`.
   TUI input buffers and modal state remain in `App`; pressing Enter delegates
   a completed payload to the session.
5. Move `EmitModel` construction to `review::emit`. Leave string rendering in
   `src/output.rs` until Stage 2 adds optional format/locator fields.
6. Change TUI render and input code to read/delegate through
   `App.review_session` rather than direct maps and anchors.
7. Keep clipboard and quit confirmation in the frontend.
8. Keep new APIs `pub(crate)` and avoid exposing review internals from
   `lib.rs`.

### Tests

- Move existing navigation and annotation tests to the narrowest applicable
  module; do not merely delete App-level coverage.
- Add command-transcript tests that construct a `ReviewSession` with a
  Markdown document and assert selection/state/output after every command.
- Retain TUI snapshot tests as frontend integration coverage.
- Add a test that the old and refactored Markdown output strings are exactly
  equal for every emit fixture.

### Gate

```sh
mise exec -- ./build.sh
git diff --exit-code -- tests/fixtures/emit tests/fixtures/tui_snapshots
```

- Every Stage 0 behavior test passes.
- No existing golden or snapshot changes are accepted in this stage.
- Coverage remains at least 80%.
- `cargo doc --no-deps` remains warning-free.

### Outcome

The Markdown TUI looks and behaves exactly as before, but its review semantics
are reusable without terminal dependencies.

## Stage 2 — Generalize Selection Index Construction

### Goal

Allow HTML manifest nodes to use the same selection/navigation engine without
altering Markdown selection behavior.

### Work

1. Add `SelectionNodeInput` and a neutral `SelectionIndex::from_nodes`.
2. Convert the current Markdown builder into an adapter that produces neutral
   nodes and delegates to `from_nodes`.
3. Generalize section building to neutral heading level and ordered-list
   identity metadata.
4. Preserve Markdown's exact rules for:
   - headings/list items as one Sentence anchor;
   - code blocks excluded from Sentence but included in Line/Word;
   - list source-line collapsing;
   - GFM table separator filtering;
   - top-level ordered-list sections before headings;
   - pre-heading and nested-heading section spans.
5. Add a constructor used only by tests that creates synthetic HTML-like
   neutral nodes with explicit logical line ranges.
6. Add byte-range-to-Unicode-scalar-range conversion helpers with Unicode and
   combining-mark coverage.

### Tests

- Existing selection index, navigator, projection, and integration tests.
- Differential tests: old Markdown-derived expectations versus the neutral
  builder for the complete Markdown fixture corpus.
- Synthetic HTML tests for all five units, nested sections, logical lines,
  empty nodes, repeated text, Unicode, and navigation boundaries.
- Property-style invariant tests:
  - ranges are ordered, non-overlapping, and within node text;
  - linear tables are in document order;
  - every table entry references an existing range;
  - section endpoints are valid;
  - byte/scalar round trips never split UTF-8.

### Gate

```sh
mise exec -- ./build.sh
git diff --exit-code -- tests/fixtures/emit tests/fixtures/tui_snapshots
```

- Markdown output and UI snapshots remain unchanged.
- Neutral-node tests cover every HTML selection rule fixed above.

### Outcome

One tested selection engine can accept Markdown parser nodes or browser-derived
HTML review nodes.

## Stage 3 — Add CLI Routing and a Testable Server Lifecycle

### Goal

Introduce the web command contract and foreground server lifecycle without yet
serving the plan document.

### Work

1. Extend `CliArgs`/`CliCommand` with `web` and `no_open`.
2. Add extension validation and all conflicts specified in Fixed Product
   Decisions.
3. Add `LaunchMode::{MarkdownTui, HtmlWeb}` after CLI parsing so `main.rs`
   routes before terminal detection. HTML web mode must never invoke terminal
   fallback.
4. Create `src/web/server.rs`, `session.rs`, `protocol.rs`, `security.rs`, and
   `browser.rs`.
5. Bind `127.0.0.1:0`, generate a 256-bit token, build the tokenized URL, and
   start the request loop.
6. Serve an embedded placeholder parent page and a health/state response.
7. Implement `--no-open`, injected browser launching, stderr-only startup
   diagnostics, 24-hour inactivity timeout, explicit server unblock, thread
   join, and signal-safe coordinator cleanup.
8. Add a temporary finish endpoint that returns `No actions.` so process-level
   lifecycle can be tested before review semantics exist.
9. Extend `--debug` output with:
   - `ui_mode: web`;
   - `source_format: html`;
   - canonical source path;
   - source size;
   - browser launcher candidate;
   - loopback bind address description;
   - whether `--no-open` is active.
   Do not print a live token or bind a server in debug mode.

### Tests

- CLI parse/unit tests for every valid and invalid flag/path combination,
  including uppercase extensions and flags after the positional path.
- Existing Markdown CLI and terminal fallback tests unchanged.
- Token uniqueness/length/hex-encoding tests with injectable randomness for
  deterministic failure coverage.
- Browser launcher command construction tests for macOS/Linux and fallback.
- Route tests for token, method, Host, Origin, content type, and unknown path.
- Process integration test:
  - start `rep --web --no-open fixture.html`;
  - parse the URL from stderr;
  - GET the shell;
  - POST temporary finish;
  - assert response completion, listener closure, exit zero, and `No actions.`
    on stdout.
- Timeout test with an injected clock and short duration; never wait in real
  time.

### Gate

```sh
mise exec -- ./build.sh
```

- `rep plan.md` still reaches the TUI path.
- `rep plan.html` fails with the `--web` instruction.
- `rep --web plan.html --no-open` can complete without opening a browser.
- No server route is reachable without the exact token.
- No server/thread remains after the process exits.

### Outcome

The public CLI and skill-compatible foreground lifecycle are stable before
HTML parsing or browser interaction is added.

## Stage 4 — Safely Serve the Original HTML Layout and Local CSS/Assets

### Goal

Render the original HTML/CSS in a browser iframe while enforcing the complete
local-resource and no-script boundary.

### Work

1. Implement the span-aware source transformer in `html_source.rs`.
2. Add source ID injection and source-line metadata.
3. Remove/neutralize the forbidden elements, meta directives, attributes, and
   URLs fixed above.
4. Serve the transformed document at
   `/assets/__rep_document__.html`.
5. Implement canonicalized asset confinement:
   - percent-decode exactly once;
   - reject NUL, absolute paths, empty traversal components, and invalid UTF-8;
   - join beneath the canonical plan directory;
   - canonicalize the result;
   - require it to start with the canonical plan directory;
   - require a regular file;
   - reject symlinks whose targets escape;
   - allow only CSS, common raster/SVG image, and common web-font MIME types;
   - apply a per-asset 20 MiB response limit;
   - never list directories.
6. Add parent shell and iframe CSP/security headers.
7. Show blocked-resource counts in the parent status API.
8. Verify that original CSS cannot style the parent app because it remains
   inside the iframe.

### Tests

- Golden transformed-source tests for every security fixture.
- Assert transformation preserves untouched byte ranges and source line maps.
- Asset route table tests for success, missing files, wrong MIME, traversal,
  double encoding, symlinks, root-relative paths, spaces, Unicode, and case.
- CSP/header snapshot tests for parent, document, API, and asset responses.
- A browser security suite proving:
  - inline and external scripts do not run;
  - event handlers do not run;
  - forms and navigation do not leave the document;
  - remote fetch/image/font/style requests do not succeed;
  - local CSS, images, SVG-as-image, and fonts render;
  - plan CSS cannot alter the parent toolbar;
  - source files are unchanged after serving.
- Visual gallery screenshots at desktop and narrow widths for the layout
  fixtures. Screenshots are review artifacts, not pixel-perfect CI gates.

### Gate

```sh
mise exec -- ./build.sh
mise exec -- npm --prefix web ci
mise exec -- npm --prefix web run test:e2e -- --grep @rendering
mise exec -- npm --prefix web run gallery
```

- Security browser tests pass in Chromium CI.
- A human reviews the generated gallery in Chromium and at least one of Safari
  or Firefox on a supported OS.
- No document script execution or out-of-root asset read is observed.
- Original-layout fixtures retain their intended CSS layout.

### Outcome

Rep safely displays realistic HTML plans with their original static
presentation before review controls are layered on.

## Stage 5 — Build the Browser Manifest, Selection, and Navigation

### Goal

Turn the rendered DOM into a server-authoritative review document and deliver
complete keyboard/mouse selection navigation without annotations.

### Work

1. Implement visible text ownership, normalization, logical newline handling,
   locator generation, ordered-list identity, link extraction, and DOM
   boundary maps in `document.js`.
2. Post and validate the manifest with explicit total limits:
   - maximum 100,000 review nodes;
   - maximum 20 MiB JSON body;
   - maximum 1 MiB normalized text per node;
   - all scalar ranges ordered and in bounds.
3. Construct `HtmlReviewDocument` and `ReviewSession` after manifest
   validation.
4. Implement idempotent reload manifests and conflicting-manifest 409s.
5. Implement `/api/state` and `/api/command` with revision handling.
6. Implement all movement keys, unit cycle/adjust keys, click mapping,
   boundary feedback, scroll-to-selection, and server-state restoration after
   reload.
7. Implement shadow-root rectangle overlays and repaint triggers.
8. Add top-level status/mode display and a loading state that distinguishes:
   loading document, waiting for fonts/images, extracting text, and ready.
9. Show a clear non-fatal empty-state page if the HTML has no selectable text.

### Tests

- JavaScript unit tests for whitespace normalization, ownership, locators,
  scalar/UTF-16 mapping, logical lines, visibility, and revision handling.
- Rust protocol validation tests for every malformed manifest field/range and
  configured limit.
- Golden manifest tests for every Stage 0 fixture.
- Shared `ReviewSession` command transcripts run against synthetic Markdown
  and HTML documents to verify navigation parity.
- Playwright tests for:
  - every unit and key;
  - click/double/triple click;
  - repeated text selecting the correct occurrence;
  - inline markup across several text nodes;
  - Unicode/emoji/combining marks;
  - hidden content omission;
  - nested headings and ordered-list sections;
  - line semantics independent of viewport width;
  - scroll/resize/font/image overlay repaint;
  - reload restoration;
  - empty documents;
  - parent and iframe focus transitions.

### Gate

```sh
mise exec -- ./build.sh
mise exec -- npm --prefix web ci
mise exec -- npm --prefix web test
mise exec -- npm --prefix web run test:e2e -- --grep @navigation
```

- The complete HTML fixture manifest matches hand-reviewed expectations.
- All navigation is usable without a mouse.
- Highlight rectangles still align after resize and scrolling.
- Markdown transcript, TUI snapshot, and output fixtures remain unchanged.

### Outcome

Users can reliably traverse and select content in a faithfully rendered HTML
plan, but cannot yet create annotations.

## Stage 6 — Add Search, Popups, Annotations, and Final Output

### Goal

Reach functional parity with Rep's annotation workflow and produce
source-addressable HTML action output.

### Work

1. Implement search input, server-side matching, `n`/`N`, status messages, and
   scroll/highlight updates.
2. Implement `[`/`]` annotation jumps.
3. Implement modal input and shared-core commands for:
   - `c` literal change;
   - `f` feedback/intent;
   - `b` insert before;
   - `a` insert after;
   - `x` clear-then-strike/delete;
   - `e` edit the applicable change/feedback.
4. Render distinct selection and annotation overlays.
5. Implement `?`, `I`, `O`, `r`, Escape, Enter, `q` confirmation, and `Q`
   discard. Keep a persistent bottom HUD with mode, `q` completion, and `?`
   help indicators; do not add Submit/Discard controls.
6. Extend `EmitModel` with `DocumentFormat` and optional `SourceLocator`.
7. Render `FORMAT: html` and `LOCATOR:` only for HTML actions.
8. Add HTML `ActionContext` generation from captured target and neighboring
   visible review nodes.
9. Ensure target capture stores source line, locator, unit/index, normalized
   target text, neighbor text, and timestamp at annotation creation.
10. Replace the Stage 3 temporary finish route:
    - finish freezes the session;
    - build output once;
    - respond successfully;
    - transition to Finished;
    - signal graceful shutdown;
    - print the frozen output to stdout after the server thread joins.
11. Implement silent discard and clipboard fallback.
12. Guard duplicate finish/discard requests with an idempotent terminal state;
    the first terminal action wins.

### Tests

- Shared-core unit tests for HTML capture, edit selection, clear/strike,
  deterministic ordering, context, locators, and finish freeze.
- New HTML emit golden matrix covering every selection unit x every action,
  including inline markup, repeated targets, nested sections, and Unicode.
- Explicit assertion that every existing Markdown emit golden is unchanged.
- Browser tests for every key, modal focus/escape/enter behavior, editing,
  search, jumps, popups, copy success/fallback, `q` confirmation, `Q` discard,
  handoff acknowledgement/self-close, and duplicate-finish races.
- Process tests asserting:
  - stdout stays empty during review;
  - stderr contains startup diagnostics only;
  - finish prints exactly one complete action list;
  - no-action finish prints `No actions.`;
  - discard prints nothing;
  - server listener closes after response;
  - non-zero server errors never print partial actions.
- Reload tests with annotations already present.

### Gate

```sh
mise exec -- ./build.sh
mise exec -- npm --prefix web ci
mise exec -- npm --prefix web test
mise exec -- npm --prefix web run test:e2e
```

- Every current Rep key has a tested HTML behavior or the fixed HTML-specific
  analogue.
- Full HTML action output is stable under golden tests.
- Markdown output remains byte-for-byte unchanged.
- Finish/discard always terminates the server process cleanly.

### Outcome

`rep --web plan.html` is a complete standalone HTML review workflow with
action output suitable for an agent.

## Stage 7 — Integrate the Agent Skill and HTML Edit Application Rules

### Goal

Make `$rep` route Markdown to the TUI and HTML to the browser, capture fresh
results, and safely apply HTML annotations in the same agent turn.

### Work

1. Update `.agents/skills/rep/SKILL.md`:
   - description covers Markdown and HTML plans;
   - detect `.html`/`.htm` case-insensitively;
   - Markdown runs the existing command unchanged;
   - HTML appends `--web`;
   - unsupported/ambiguous format stops before launch;
   - continue non-PTY launch and indefinite polling rules;
   - parse only the fresh capture file;
   - recognize silent discard as no edits;
   - require full captured output in the final response.
2. Extend `run_rep_and_capture.sh` to own the HTML skill lifecycle:
   - preserve generic Markdown forwarding;
   - for HTML without an explicit `--no-open`, preflight a supported browser,
     append `--no-open`, and launch an isolated temporary browser profile from
     the emitted Review URL;
   - after receipt, close only the browser process/profile it owns before
     emitting the capture marker;
   - preserve an explicit `--no-open` as manual mode;
   - preserve the exit status and fresh capture marker in every mode.
3. Update HTML action rules:
   - use source line as a hint;
   - resolve `LOCATOR` against the original HTML first;
   - confirm the exact normalized visible `CONTEXT.target`;
   - use neighboring visible context to disambiguate;
   - preserve indentation, element structure, attributes, CSS classes, and
     unrelated inline markup;
   - for word/sentence changes spanning inline elements, edit the smallest
     containing element necessary and retain meaningful emphasis/link/code
     markup where compatible with the requested change;
   - paragraph deletion removes only the owning review fragment/element;
   - section deletion removes the heading and its section content up to the
     emitted boundary, never a nested/sibling section outside the target;
   - insertions use structurally valid HTML appropriate to the containing
     element (for example, list content remains in `li`, table content remains
     in cells);
   - never edit Rep's transient `data-rep-*` IDs because they are absent from
     the source;
   - if locator, target, and context cannot identify one source location,
     stop and ask rather than guessing.
4. Add a skill harness with a fake/test browser client that sends known
   annotations so CI can exercise launch -> capture -> parse -> fixture edit
   without human interaction.
5. Add Markdown skill regression coverage so tmux/terminal fallback behavior
   is not changed by HTML routing.

### Tests

- Shell tests for extension routing, argument forwarding, capture path,
  non-zero propagation, spaces in paths, and silent discard.
- Skill fixture tests applying each action to:
  - plain text elements;
  - nested inline markup;
  - repeated text with different locators;
  - lists;
  - tables;
  - nested sections;
  - Unicode/entities.
- End-to-end automated harness:
  - invoke the same runner the skill uses;
  - connect to the emitted web URL;
  - initialize the manifest;
  - press `q`, confirm, and send annotations;
  - verify the handoff state and temporary-browser shutdown;
  - wait for `REP_CAPTURE_FILE`;
  - assert captured output;
  - apply rules to a temporary HTML copy;
  - render the edited result and assert requested visible-text changes.
- Existing Markdown runner and fallback tests.

### Gate

```sh
mise exec -- ./build.sh
mise exec -- npm --prefix web ci
mise exec -- npm --prefix web run test:e2e -- --grep @skill
```

- A real manual `$rep` Markdown run still opens the TUI and returns captured
  Markdown actions.
- A real manual `$rep` HTML run opens the browser, shuts down after `q`
  confirmation, automatically closes the skill-owned temporary browser, and
  returns captured HTML actions without manual app switching.
- Automated HTML fixture edits are structurally valid and render with the
  expected visible changes.

### Outcome

The agent loop chooses the correct UI from the plan format and completes the
full review/apply cycle for either Markdown or HTML.

## Stage 8 — Documentation, Packaging, CI, and Cross-Browser Hardening

### Goal

Make the feature releasable under the repository's existing platform,
coverage, installer, and static-package guarantees.

### Work

1. Update README:
   - overview of Markdown TUI vs HTML web UI;
   - exact CLI examples;
   - HTML safety/rendering model;
   - local asset rules and JavaScript limitation;
   - lifecycle and manual URL behavior;
   - HTML-specific selection semantics;
   - keybinding table generated from the shared keymap;
   - troubleshooting for browser launch, blocked resources, SSH, and stale
     sessions.
2. Update CLI help/usage from `markdown-file` to `plan-file` without obscuring
   which mode supports which format.
3. Add an HTML demo fixture and an opt-in `scripts/record-web-demo.sh`; do not
   overload the existing `--demo` Markdown behavior.
4. Add Node to `mise.toml` at a pinned supported version and commit the web
   test lockfile. Runtime/release users still need only the Rep binary.
5. Extend `build.sh`:
   - keep Rust fmt/clippy/test/coverage/build;
   - run fast JavaScript unit tests when web dependencies are installed;
   - print an explicit local skip message when they are absent;
   - require them in CI.
6. Add a dedicated Chromium Playwright CI job with cached browser/dependencies.
   Upload traces/screenshots only on failure and upload the generated visual
   gallery as a review artifact on relevant PRs.
7. Keep Rust line coverage at least 80%; focus added coverage on routing,
   session state, security, source transformation, asset confinement, output,
   and shutdown.
8. Add manual release checklist entries for current Safari on macOS and Firefox
   on Linux/macOS. Chromium remains the automated browser gate.
9. Extend release archive smoke tests:
   - binary help includes `--web`;
   - embedded app assets are served from the packaged static binary;
   - `--web --no-open` can start and complete against a fixture without files
     from the source checkout;
   - bundled skill contains HTML routing instructions.
10. Verify both MUSL targets have no dynamic dependency introduced by the HTTP,
    token, tokenizer, or embedding dependencies.
11. Update platform support claims only after Linux x86_64/arm64 archive smoke
    tests and macOS x86_64/arm64 builds pass.

### Tests

- Full local validation.
- Full JavaScript and Playwright suites.
- `cargo audit`.
- `cargo doc --no-deps` with warnings denied.
- Linux MUSL cross-build and archive smoke tests.
- macOS native tests and cross-target release build.
- Manual Safari and Firefox checklist.
- Clean-machine install/skill smoke test.

### Gate

```sh
mise exec -- ./build.sh
mise exec -- npm --prefix web ci
mise exec -- npm --prefix web test
mise exec -- npm --prefix web run test:e2e
mise exec -- cargo audit
mise exec -- cargo doc --no-deps
```

Then require every CI and release workflow job to pass from a clean checkout.

### Outcome

The HTML web UI is documented, packaged inside the static Rep binary, covered
by automated security/navigation/skill tests, and validated on the currently
supported release platforms.

## Stage 9 — Final Acceptance and Release Readiness

### Goal

Prove the complete feature against user-visible outcomes rather than only
module-level tests.

### Acceptance scenarios

1. **Markdown compatibility**
   - Run `rep representative.md`.
   - Confirm the existing TUI opens.
   - Exercise every key group.
   - Submit actions.
   - Confirm output matches the pre-feature baseline.

2. **HTML launch and fidelity**
   - Run `rep --web representative.html`.
   - Confirm the default browser opens.
   - Confirm original layout, inline CSS, linked local CSS, images, and fonts.
   - Confirm plan JavaScript does not run and remote resources remain blocked.

3. **HTML navigation**
   - Navigate the complete plan with keyboard only.
   - Exercise all five units, search, annotation jumps, help, outline, links,
     and boundary behavior.
   - Resize and scroll while confirming highlight alignment.

4. **HTML annotation matrix**
   - Add/change/edit/clear each annotation type at section, paragraph, line,
     sentence, and word units.
   - Press `q`, confirm, and inspect stable `WHERE`, `LOCATOR`, context, and
     payload output.

5. **Skill loop**
   - Invoke the bundled skill once for Markdown and once for HTML.
   - Confirm the right UI launches.
   - Confirm the agent waits without attempting to manipulate the UI.
   - Press `q` and confirm the feedback handoff appears.
   - Confirm the server exits, the temporary browser closes, capture becomes
     available, and the agent applies only the fresh actions to the correct
     source file without requiring manual app switching.

6. **Failure paths**
   - Browser opener missing.
   - Wrong file/flag combination.
   - Missing/oversized/non-UTF-8 file.
   - Bad Host, Origin, token, method, MIME, and request size.
   - Asset traversal/symlink escape.
   - Malformed/conflicting manifest.
   - Browser tab reload and close/reopen.
   - SIGINT and injected server failure.
   - Session timeout.

### Final gate

- All Stage 8 automated gates pass from a clean checkout.
- All acceptance scenarios pass on one supported macOS machine and one
  supported Linux machine.
- Chromium automated tests pass.
- Safari and Firefox manual checklists pass.
- No existing Markdown output golden or TUI snapshot changed without a
  separately documented intentional Markdown behavior change.
- Rust line coverage is at least 80%.
- No high/critical audit findings remain.
- The static release archive contains only the expected binary/docs/skill
  payload and works without Node or source-tree web assets.

### Outcome

Rep has two deliberately separate frontends sharing one review engine:

- a stable terminal UI for Markdown plans; and
- a secure, foreground, local browser UI for rendered HTML plans.

Both produce the same actionable review protocol and both complete the same
skill-driven human-in-the-loop workflow.

## Required Test Matrix

| Area | Unit | Rust integration | Browser E2E | Manual |
| --- | --- | --- | --- | --- |
| Markdown TUI compatibility | Yes | Yes | N/A | One acceptance pass |
| CLI routing | Yes | Yes | N/A | Smoke |
| Shared review session | Yes | Transcript/golden | Command parity | N/A |
| HTML token/source mapping | Yes | Fixture golden | Manifest cross-check | Spot-check |
| Asset confinement | Yes | HTTP route | Blocked/allowed loads | DevTools check |
| CSP/sandbox/no-script | Header golden | HTTP route | Required | DevTools check |
| DOM extraction/text mapping | JS unit | Manifest golden | Required | Gallery |
| Keyboard/mouse navigation | Command unit | Protocol | Required | Accessibility pass |
| Overlay geometry | Geometry unit | N/A | Required | Gallery/resize |
| Annotation/output | Yes | Emit golden/process | Required | Matrix pass |
| Finish/discard/shutdown | Yes | Child process | Required | Skill pass |
| Skill routing/application | Shell/fixture | Harness | Required | Markdown + HTML |
| Packaging/static assets | N/A | Archive smoke | Headless smoke | Clean install |
| Browser coverage | N/A | N/A | Chromium | Safari + Firefox |

## Per-Stage Working Rules

Apply these rules during every stage:

1. Read `AGENTS.md` before implementation work.
2. Verify the active branch is `html_and_browser_support`; all work governed
   by this plan must be committed and pushed there, never directly to `main`.
3. Use `mise exec --` for Rust, Node, and browser test commands.
4. Run focused tests while iterating, then the stage's full gate.
5. Stop at the first failing gate and fix it before expanding scope.
6. Keep each commit scoped to one stage or one clearly named substep.
7. Do not mix mechanical snapshot/golden refreshes with semantic source changes.
8. Inspect every changed golden; never approve all snapshots blindly.
9. Keep stdout clean: only final action output belongs there during a review.
10. Treat plan HTML, URLs, manifests, and asset paths as untrusted input.
11. Never weaken path confinement or CSP to make a fixture pass; update the
    fixture or document an intentionally unsupported resource.
12. Preserve unrelated user/worktree changes.
13. Run `mise exec -- ./build.sh` before completing any implementation stage.

## Definition of Done

The project is done only when all of the following are true:

- `rep plan.md` retains the current Markdown TUI and current output.
- `rep --web plan.html` opens a local browser review with original static
  layout/CSS and confined local assets.
- HTML-provided JavaScript cannot execute.
- All documented keys and annotation operations work for HTML.
- HTML actions contain a reliable line hint, DOM locator, exact visible target,
  neighboring visible context, and payload.
- Finish/discard shuts down the foreground server deterministically; a skill
  run also closes only its managed temporary browser.
- The bundled skill chooses TUI for Markdown and web for HTML, waits for fresh
  output, and applies changes to the correct format.
- Reload, browser-launch failure, malformed input, blocked assets, interruption,
  and timeout have tested, understandable behavior.
- Rust coverage remains at least 80%.
- Rust, JavaScript, browser, security, archive, installer, and skill tests pass.
- The release remains a single self-contained binary plus the existing
  documentation and bundled skill; Node is not a runtime requirement.
- README, CLI help, skill instructions, platform claims, and release smoke tests
  describe the behavior that actually shipped.
