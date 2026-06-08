# Code Split and Render Decoupling Plan

## Goal

Reduce the maintenance burden in the largest app/view files without changing user-visible behavior:

- `src/app/input.rs` is currently ~1,025 lines.
- `src/document_view.rs` is currently ~960 lines.
- `src/app/render.rs` is currently ~739 lines.

The refactor should preserve all keybindings, emitted output, selection behavior, TUI snapshots, transcript fixtures, and coverage above the existing global gate.

## Non-Goals

- Do not redesign navigation, selection units, annotation semantics, or emitted output.
- Do not rename public/user-facing modes.
- Do not replace either markdown parser in this pass.
- Do not introduce a new UI framework or async runtime.
- Do not move large blocks merely to satisfy line counts if the new module boundaries are unclear.

## Safety Net

Before the first mechanical split, capture the current guardrails:

```sh
mise exec -- cargo test --locked
mise exec -- ./build.sh
```

Every phase below ends with:

```sh
mise exec -- cargo fmt
mise exec -- cargo test --locked
```

Before committing any phase, run:

```sh
mise exec -- ./build.sh
```

Expected invariants:

- TUI snapshots under `tests/fixtures/tui_snapshots/` do not change unless a phase explicitly updates only module paths in snapshot metadata.
- Transcript goldens under `tests/fixtures/transcripts/` do not change.
- `render_human_output` output remains byte-identical for existing fixtures.
- Overall coverage remains above the current 80% gate.

## Phase 1: Split Input Handling by Mode

Target: `src/app/input.rs`.

Keep `App::handle_key` and `App::handle_mouse` as the stable entry points. Split implementation details into mode-focused modules under `src/app/input/`:

- `normal.rs`: normal-mode key dispatch, quit confirmation, popup closing.
- `text.rs`: change, feedback, insert, edit, and search text-entry handlers.
- `navigation.rs`: movement, mode cycling/adjustment, search jumps, annotation jumps.
- `mouse.rs`: mouse event handling, click-count logic, click-to-anchor helpers.

Suggested sequence:

1. Create `src/app/input/mod.rs` and move the current `input.rs` contents there with no behavior changes.
2. Convert `src/app/mod.rs` from `mod input;` pointing at `input.rs` to the directory module.
3. Move one group at a time into child modules using `impl App` blocks.
4. Keep helper visibility at `pub(super)` or private where possible.
5. After each group move, run the phase tests.

Risk points:

- Normal-mode dispatch order matters for popups and quit confirmation.
- `Space` / `Backspace` must remain literal in input modes.
- Mouse click timing and coordinate-to-anchor behavior are snapshot/test sensitive.

Extra tests to add only if needed:

- A focused test for popup-open state swallowing unrelated normal keys.
- A test that confirms mouse handling remains ignored during text input modes.

## Phase 2: Split Document View Responsibilities

Target: `src/document_view.rs`.

Keep `DocumentView` as the caller-facing type. Split pure helpers and rendering/context concerns into smaller files under `src/document_view/`:

- `mod.rs`: `DocumentView` struct, constructor, core accessors, high-level orchestration.
- `context.rs`: `node_line_context`, annotation context, strike context, source-line context helpers.
- `layout.rs`: wrapping/display-row logic and visible-row bookkeeping.
- `links.rs`: link lookup and current-sentence link extraction.
- `code.rs`: code-block style row generation.
- Existing `rendered.rs` and `types.rs` stay in place.

Suggested sequence:

1. Create module files with no type moves first.
2. Move functions by responsibility, keeping method names unchanged.
3. If helper functions need sharing, prefer `pub(super)` free functions over widening public API.
4. Keep `DocumentView::parse` and construction in `mod.rs` until the end.

Risk points:

- Source-line and byte-range mapping are correctness-critical.
- Link popup behavior depends on rendered markdown link ranges.
- Code-block rows bypass normal sentence wrapping and should stay covered by existing viewport/snapshot tests.

Extra tests to add only if needed:

- Direct test for `node_line_context` around first/last lines.
- Direct test for link extraction in a multi-link sentence.

## Phase 3: Split Rendering Without Changing Data Flow

Target: `src/app/render.rs`.

This phase is still an `impl App` renderer. It only separates drawing concerns and keeps mutable cache updates where they are today.

Create `src/app/render/`:

- `mod.rs`: `App::draw` orchestration and shared render helpers.
- `document.rs`: main document/list drawing and row collection.
- `footer.rs`: mode/status/footer rendering.
- `popups.rs`: help, AST, link, and text-input popups.
- `styles.rs`: node indicators, highlight styling, annotation count presentation.

Suggested sequence:

1. Move `draw_help`, `draw_ast_popup`, `draw_link_popup`, and `draw_input_popup` to `popups.rs`.
2. Move footer construction into `footer.rs`.
3. Move node/list rendering into `document.rs`.
4. Keep `App::draw` in `mod.rs` as the only method that mutates `cached_node_heights`, `list_inner`, and visible rows.

Risk points:

- TUI snapshots are the primary guardrail here.
- Footer truncation and popup sizing rely on terminal width calculations.
- `self.view.set_visible_rows` must still be called with the exact visible row ranges used for mouse hit-testing.

Extra tests to add only if needed:

- Snapshot for a narrow-width help popup if wrapping changes unexpectedly.
- Mouse click test after a render with scrolling if visible rows move.

## Phase 4: Introduce Render Snapshot State

Target: prepare for rendering over immutable state.

Add a small immutable view model that captures the data the renderer needs for one frame without borrowing all of `App` mutably:

```rust
pub(crate) struct RenderState<'a> {
    pub source_path: &'a std::path::Path,
    pub view: &'a DocumentView,
    pub selection_state: SelectionState,
    pub section_highlight_range: Option<std::ops::Range<usize>>,
    pub input_mode: &'a InputMode,
    pub status: &'a str,
    pub notification: Option<&'a str>,
    pub nav_feedback: Option<&'a str>,
    pub show_help: bool,
    pub ast_view_scroll: Option<u16>,
    pub ast_lines: &'a [String],
    // Add annotation maps/counts as needed.
}
```

Do not move rendering to `ui/` yet. First, teach `App::draw` to construct `RenderState` and pass it into helper functions while still owning cache mutation.

Suggested sequence:

1. Add `RenderState` in `src/app/render/state.rs`.
2. Convert footer and popup helpers to accept `&RenderState`.
3. Convert pure style/count helpers to accept `&RenderState`.
4. Leave document/list drawing on `&mut App` until visible row cache mutation has a better home.

Risk points:

- Avoid cloning large annotation maps per frame.
- Avoid lifetime complexity spreading outside render modules.
- Keep the frame cache mutation obvious and isolated.

## Phase 5: Move Pure Rendering to `ui/`

Target: decouple rendering from application mutation where it is actually useful.

Create a rendering module that takes immutable state plus explicit mutable frame caches:

- `src/ui/render.rs`: pure drawing functions over `RenderState`.
- `src/ui/render_cache.rs`: `RenderCache` containing `list_inner`, `cached_node_heights`, visible row ranges, and scroll offset if/when it is no longer App-owned.

Candidate API:

```rust
pub(crate) struct RenderCache {
    pub list_inner: Rect,
    pub cached_node_heights: Vec<u16>,
    pub visible_rows: Vec<(usize, std::ops::Range<usize>)>,
}

pub(crate) fn draw_app(frame: &mut Frame, state: &RenderState<'_>, cache: &mut RenderCache);
```

Suggested sequence:

1. Move footer and popup rendering first; they are closest to pure.
2. Move document/list rendering only after `RenderCache` owns the mutable render artifacts.
3. Keep input/navigation state mutation in `App`.
4. Keep `DocumentView` responsible for source/document-derived queries; do not turn `ui` into a data model.

Risk points:

- `DocumentView::set_visible_rows` currently stores mouse hit-test state. Decide whether visible rows belong in `DocumentView`, `App`, or `RenderCache` before moving document rendering.
- Avoid passing half of `App` as individual parameters. If `RenderState` grows too large, pause and re-evaluate boundaries.

## Commit Strategy

Use separate commits per phase:

1. `Split app input handlers by mode`
2. `Split document view helpers by responsibility`
3. `Split app rendering helpers`
4. `Introduce immutable render state`
5. `Move pure rendering into ui module`

Each commit should be mechanically reviewable and should not mix behavior changes with moves.

## Rollback Plan

If a phase causes broad snapshot or transcript changes:

1. Stop before committing.
2. Identify whether the change is a real behavior change or only snapshot metadata.
3. If behavior changed, revert only that phase's working changes and split the move smaller.
4. If only snapshot source paths changed due to module moves, accept those snapshots in the same phase commit and call that out explicitly.

## Completion Criteria

- `src/app/input.rs`, `src/document_view.rs`, and `src/app/render.rs` are either gone or reduced to small module entry points.
- Rendering helpers that do not need mutable `App` state live outside the main `App` implementation.
- `mise exec -- ./build.sh` passes.
- No transcript golden output changes.
- Any TUI snapshot changes are limited to source metadata or explicitly justified render-module path movement.
