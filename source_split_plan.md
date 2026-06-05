# Source Split Plan

This plan covers the remaining maintainability work after the open-source readiness pass. The goal is to reduce `src/app.rs` from a monolith into clear ownership boundaries without changing user-visible behavior or the emitted action format.

## Goals

- Split `src/app.rs` into focused modules for state, input, rendering, and output.
- Introduce a single `DocumentView` / source-map layer that owns source, selection, and display coordinate conversions.
- Finish making emitted action data a production contract through `EmitModel`.
- Narrow crate visibility where invariants should stay internal.
- Expand snapshot coverage for the major TUI states before and during the refactor.

## Non-Goals

- Do not change keybindings.
- Do not change the plain-text action block format.
- Do not rewrite selection semantics or sentence/word segmentation.
- Do not introduce async, background workers, or a different TUI framework.

## Phase 0: Baseline Safety

1. Add or confirm golden coverage for current behavior:
   - Normal browsing state.
   - Help overlay.
   - Change, feedback, insert-before, insert-after input modes.
   - Search mode and no-match status.
   - Quit confirmation.
   - AST/link popup states.
   - Annotated gutter states for change, feedback, insert, and strike.
2. Add a small test helper that renders `App` into a stable `ratatui::TestBackend` string.
3. Ensure all new snapshots use deterministic temp filenames.
4. Run:
   - `cargo fmt --check`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo test --locked`

Exit criteria: broad snapshots exist and pass without production changes.

## Phase 1: Define Module Boundaries

Create module shells while keeping most code in place:

- `src/app/mod.rs`
  - Public `App` facade and `App::load`.
  - Re-export only what integration tests need.
- `src/app/state.rs`
  - `App` fields.
  - Annotation structs.
  - Input/search/status state enums.
- `src/app/input.rs`
  - Keyboard and mouse event handling.
  - Input mode transitions.
  - Annotation mutation commands.
- `src/app/render.rs`
  - `draw`.
  - Layout helpers.
  - Rendered node construction and highlight styling.
- `src/app/output.rs`
  - `to_human_output`.
  - `to_output` / `EmitModel` conversion.
  - Clipboard helpers.

Keep this phase mostly mechanical. Move code in chunks and preserve existing function names until tests are green.

Exit criteria: `src/app.rs` becomes `src/app/mod.rs`, submodules compile, and behavior snapshots are unchanged.

## Phase 2: Introduce `DocumentView`

Add `src/document_view.rs` or `src/view/document.rs` with one owner for all coordinate conversions.

Responsibilities:

- Hold parsed `Document`, raw `source_lines`, rendered display text, and `SelectionIndex`.
- Map source line / byte ranges to display ranges.
- Map display click positions to selection anchors.
- Provide target text and line context for emit.
- Own rendered-node cache invalidation rules.

Initial API sketch:

```rust
pub(crate) struct DocumentView {
    document: Document,
    source_lines: Vec<String>,
    rendered_nodes: Vec<RenderedNode>,
    selection_index: SelectionIndex,
}

impl DocumentView {
    pub(crate) fn parse(source: &str) -> anyhow::Result<Self>;
    pub(crate) fn node_count(&self) -> usize;
    pub(crate) fn next_content_node(&self, from: usize) -> Option<usize>;
    pub(crate) fn target_for(&self, anchor: SelectionAnchor) -> Option<TargetText>;
    pub(crate) fn line_context_for_node(&self, node_idx: usize) -> LineContext;
    pub(crate) fn hit_test(&self, row: u16, col: u16) -> Option<SelectionAnchor>;
}
```

Move code into `DocumentView` in small steps:

1. Move `source_lines`, `doc`, `rendered_nodes`, and `index` behind `DocumentView`.
2. Redirect read-only app call sites through accessors.
3. Move mouse row mapping and hit testing into the view.
4. Move target/context lookup used by emit into the view.

Exit criteria: app input/output/render code no longer manually converts between source, selection, and display coordinates.

## Phase 3: Promote `EmitModel`

Rename the production output structs into a coherent model:

- `EmitModel`
- `EmitKeymap`
- `EmitAnnotation`
- `EmitLineContext`
- `EmitChange`
- `EmitFeedback`
- `EmitInsert`
- `EmitReaction`

Rules:

- Keep the human text renderer byte-compatible.
- Keep the model `pub(crate)` unless a real external API is required.
- Add unit tests that build `EmitModel` directly from app state and assert key fields before formatting.

Exit criteria: `App::to_human_output` formats from `EmitModel`, and tests prove the model is independent from the final text format.

## Phase 4: Narrow Public API

Review `src/lib.rs` and submodule exports.

Likely target shape:

```rust
pub(crate) mod app;
pub mod cli;
pub(crate) mod document;
pub(crate) mod document_view;
pub(crate) mod markdown;
pub(crate) mod output;
pub(crate) mod selection;
pub mod ui;
```

Adjust integration tests as needed:

- Prefer black-box CLI tests where possible.
- Move white-box tests into module unit tests when they rely on invariants.
- Keep only intentional public surface available to downstream crates.

Exit criteria: public API exposes only CLI/UI boundaries that are intentionally supported.

## Phase 5: Delete Transitional Glue

After modules and `DocumentView` are stable:

1. Remove duplicated coordinate helpers from `render`, `input`, and `output`.
2. Remove compatibility accessors that only existed during the migration.
3. Rename vague helpers around their owning concepts.
4. Keep comments only where conversion invariants are not obvious.

Exit criteria: no module has to know all three coordinate systems unless it is `DocumentView`.

## Validation Matrix

Run after each phase:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
```

Run before merge:

```sh
./build.sh
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked
cargo package --allow-dirty
cargo publish --dry-run --allow-dirty
shellcheck build.sh install.sh install-skills.sh .agents/skills/rep/scripts/*.sh scripts/*.sh
cargo audit
```

## Suggested PR Breakdown

1. Snapshot expansion only.
2. Mechanical `app` module split.
3. `DocumentView` introduction with read-only accessors.
4. Hit-testing and emit context moved into `DocumentView`.
5. `EmitModel` rename and formatter cleanup.
6. Visibility narrowing and final glue removal.

Each PR should preserve existing snapshots unless it explicitly updates a tested behavior.
