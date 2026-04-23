# Modular plan for selection/navigation (section, paragraph, line, sentence, word)

## Requirements

Hard constraints, gathered from a design-review dialogue. Where any section further down conflicts with these, these win.

### Functional

1. **Read-only on the plan file.** Rep never writes the markdown back to disk; applying edits is the consumer LLM's job.
2. **Five selection units.** The user navigates across section, paragraph, line, sentence, and word — in that containment order.
3. **Word-level unlocks (a) and (b) only.**
   - `delete this` on a single word.
   - `change` / `revise-to-incorporate-feedback` targeting a single word (or a contiguous phrase selected as a single unit).
   - Quoting precise phrases back to the LLM, wrapping spans, and multi-word extension are explicitly out of scope for this iteration.
4. **One unit per action, always.** No range-extension across units (no "word 5 → word 9"). Selection state is a single anchor, not a (start, end) pair.
5. **Full behavioral parity with current rep.** All existing keybindings, navigation behavior, output format, and annotation flows continue to work unchanged. The refactor is strictly additive (word-level) plus internal cleanup.

### Output contract

6. **Output shape stays as-is.** Per-line `ACTION` blocks with `WHERE` line number, `CONTEXT` (prev / current / next lines), `target` text, and `change` / `feedback` / reaction payload — identical to current rep emit.
7. **Annotations identify targets by line + text + context, not byte ranges.** The consuming LLM reconstructs edits from text and line numbers. No source byte range or character offset is emitted in the output.

### Internal representation

8. **AST carries accurate source byte spans per node** so selection can render correct highlights, resolve containment queries, and produce unambiguous `target` text. Byte-level precision is an internal invariant only — it never leaks into the output.
9. **One coordinate for positions: source byte offset** (Rust-idiomatic `Range<usize>`). No separate codepoint, grapheme, or column space is required internally.

### Scope

10. **Markdown support = `markdown::ParseOptions::gfm()`.** CommonMark core plus GFM tables, task list items, strikethrough, autolink literals, and footnotes. Front matter, math, MDX, and arbitrary HTML extensions are out of scope.

### Performance

11. **Design target: plan files up to ~5,000 lines.** The selection index may be built eagerly at load time; no lazy/incremental indexing is needed at this size.

### Implications applied below

These requirements drove the following revisions to the architecture sections:

- Req 7 dissolves any need for non-contiguous source spans in the data model — the segment engine operates on rendered plain text and emits single-`Range<usize>` segments; source byte mapping is only needed at the AST-node level, not per segment.
- Req 1 narrows "renders back" language to the **TUI render pipeline**, not file write-back.
- Req 4 removes `SelectionRange` as a state type; `SelectionState` carries a single anchor, and projection computes the highlight on demand.
- Req 5 caps the refactor's blast radius: existing tests are the regression baseline.

## Goals

1. Add **word-level selection** without destabilizing existing section/paragraph/line/sentence navigation.
2. Make selection movement a **pure, testable domain** independent from TUI rendering.
3. Keep AST parsing, text segmentation, navigation, and rendering concerns separated.
4. Ensure each selection unit has deterministic `next`/`prev` semantics across node boundaries and across selection-unit changes.

## Current issues to address

1. Selection logic is split between `src/document.rs` and `src/app.rs` (navigation in both places).
2. Sentence splitting behavior is duplicated (`text_to_sentences` in `document.rs`, `sentence_ranges_from_plain` in `app.rs`, and test-only `split_sentences` in `markdown.rs`).
3. `App` owns cursor state (`cursor_node`, `cursor_sentence`) and movement details, making behavior hard to test without UI coupling.
4. Render-time text (`RenderedNode.plain`) can diverge from parse-time text, but there is no explicit contract layer for selection anchors across those representations.

## Target architecture

### 1) `selection::model` (types + invariants)

- `SelectionUnit`: `Section | Paragraph | Line | Sentence | Word`
- `SelectionAnchor`: stable location, resolved via the AST
  - `node_idx: usize` — index into the `Document`'s flat node list; stable for a loaded file
  - `unit: SelectionUnit`
  - `unit_idx: usize` — index within the node for line/sentence/word; unused for paragraph and section (node identity suffices)
- `SelectionState`: the current anchor — nothing else

The highlight span is **not** stored on state. It is computed on demand by `projection` from `(anchor, index)`. Per req 4, there is no end-anchor and no range-extension state.

Key invariant: navigation consumes and produces `SelectionAnchor`. UI never computes anchor math directly.

### 2) `selection::segment` (single text segmentation engine)

Unify sentence and word segmentation in one module:

- `segment_sentences(text) -> Vec<Range<usize>>`
- `segment_words(text) -> Vec<Range<usize>>`
- helper rules for abbreviations/list markers/newline continuation.

This replaces all ad-hoc split logic and ensures AST and render paths use the same segmentation behavior.

### 3) `selection::index` (derived navigation cache)

Built eagerly from the parsed AST + rendered plain text at load time. Not a parallel document model — a derived cache keyed by `node_idx`.

- per node (`node_idx`):
  - reference to the AST node's source byte span (not copied)
  - source-line ranges — `(source_line_number, range_in_node_plain_text)` pairs, so "Line" selection can map a source line to a span of rendered text
  - sentence ranges — `Vec<Range<usize>>` in the node's rendered plain text
  - word ranges — `Vec<Range<usize>>` in the node's rendered plain text
- per document:
  - linear order tables per unit (section, paragraph, line, sentence, word) for O(1) `next` / `prev`

Notes:
- All ranges are **scoped to a node**; they index into that node's rendered plain text and never serve as global document offsets.
- The index holds no long-lived plain-text copies — ranges reference the render pipeline's existing per-node plain text (`RenderedNode.plain`).
- Valid only for the AST it was built from; discard on reload. Per req 1 the file is read-only, so invalidation reduces to reload.
- Build is eager at load time (req 11 caps the input at ~5k lines).

### 4) `selection::navigator` (pure next/prev logic)

Single pure API:

- `next(index, anchor, unit) -> SelectionAnchor` — always returns a valid anchor (wrap-around at document edges, see below).
- `prev(index, anchor, unit) -> SelectionAnchor` — same, with wrap.
- `clamp(index, anchor, target_unit) -> SelectionAnchor` — re-anchor onto a different unit type without introducing "between units" state.

#### Movement rules

- **Boundary crossing is silent.** `next` at the end of the current containing unit advances into the next containing unit with no status message. Word-next at end-of-sentence jumps straight to the first word of the next sentence; sentence-next at end-of-paragraph jumps to the first sentence of the next paragraph; etc. Same at paragraph and section boundaries.
- **Wrap-around at document edges.** `next` on the last unit wraps to the first; `prev` on the first wraps to the last. Applies uniformly to all five units.
- **Wordless / unit-less nodes are skipped.** When walking a unit's linear order table, nodes whose rendered plain text contributes no entries of that unit type (thematic break, image-only paragraph if the renderer emits nothing, empty code block, etc.) produce no anchors and are silently stepped over.
- **Code blocks are excluded from the sentence-level linear order.** Sentence-next / sentence-prev skip fenced and indented code blocks entirely. Users navigate code blocks via line or word units, or select them as a whole paragraph. Code blocks still participate normally in paragraph, line, word, and (via containment) section traversal.
- **Section nav is a no-op when the document has no headings.** `next` / `prev` in section mode do nothing; switching to section mode does nothing. No wrap, no status, no error. (A document with one heading wraps to itself under the normal wrap rule, which is also effectively a no-op — this rule just generalizes it.)
- **Roundtrip invariant.** `prev(next(x)) == x` for any anchor not at a wrap point.

#### Unit-switch rules (`clamp`)

- **Upgrade (finer → coarser):** return the anchor for the containing unit at the requested coarser level. Word `fast` in sentence S in paragraph P in section Sec → clamp(word → sentence) = S; clamp(word → paragraph) = P; clamp(word → section) = Sec.
- **Downgrade (coarser → finer):** return the first-child anchor of the current unit at the requested finer level. Sentence `Dogs run fast through the park.` → clamp(sentence → word) = word `Dogs`. Section with heading + 3 paragraphs → clamp(section → paragraph) = first paragraph under the heading.
- **Same rule everywhere.** No remembered history of previously-selected child; downgrades always land on first child.
- **Target unit unavailable in current node.** If the requested target unit has no representatives in the anchor's node (e.g. switching to sentence mode while inside a code block, or switching to section mode when the document has no headings), walk outward to the nearest enclosing node that does have the target unit and clamp there. If no such node exists in the document, the unit switch is a no-op — selection stays on the current anchor and current unit.

#### Visibility rule (shared with `segment`)

A word or sentence exists only where the renderer emitted visible text. URLs in `[label](url)`, image source URLs, and any markdown syntax stripped by the renderer are invisible to selection. Alt text is selectable iff the renderer emits it. This is the single predicate that decides whether a node contributes anchors at the word/sentence level.

### 5) `selection::projection` (anchor → highlight)

Given `(SelectionAnchor, SelectionIndex)`, return what the render layer should paint. One anchor resolves to exactly one highlight (req 4).

- **Word / Sentence** → `(node_idx, Range<usize>)` — a range in the node's rendered plain text.
- **Line** → `(node_idx, Range<usize>)` — the plain-text range corresponding to the selected source markdown line.
- **Paragraph** → `(node_idx, full_plain_text_range)` — the whole node.
- **Section** → `Vec<node_idx>` — the heading node through the last node of the section; render paints each as a whole-block highlight.

The render layer paints exactly what projection returns. Anchors are never resolved in the render layer; no anchor math happens outside this module.

The same module also exposes the **annotation emit view**: given an anchor, return `(source_line_number, target_text)` derived from the AST node's source span + the node's rendered plain text sliced by the anchor's range. This is what the output contract (req 6) consumes — no byte offsets.

### 6) Thin app integration

`App` changes:

- Replace `cursor_node/cursor_sentence` ownership with a single `SelectionState`.
- Key handlers call navigator only.
- Existing annotation APIs query current sentence/word via projection/context helpers.

## Architecture refinements (from design review)

These refinements sharpen the target architecture above and take precedence where they conflict.

### Three-layer framing

The app decomposes into three layers, in this order:

1. **AST** — parses the source markdown and carries source byte spans per node as the authoritative position data. Read-only on disk per req 1; "round-trip" applies to the TUI render pipeline, not file write-back.
2. **Selection** — an adjustable mechanic over the AST. Anchors identify a node + unit + unit_idx; selection never carries byte offsets of its own.
3. **Viewport / render** — projects AST + selection to the terminal. Owns rendered plain text, wrapping, grapheme/cell width, and highlight painting.

### Coordinate spaces

Exactly four global coordinate-ish things exist. Anything claiming to be a fifth is a bug.

1. **source-offset** — byte position in the original markdown file. AST node spans live here; annotation line numbers derive from here. Per req 7, annotations emit line numbers + text — not raw byte offsets.
2. **AST** — structured nodes keyed by `node_idx`, each carrying a source byte span.
3. **Selection** — anchors expressed as `(node_idx, unit, unit_idx)`; resolution to characters goes through the AST + index.
4. **Viewport / lens** — terminal cell geometry, grapheme → cell width, wrap decisions. Must not leak into selection.

Rendered plain text, and ranges into it, are **per-node and scoped** — they live inside the render / segment / index pipeline and never cross node boundaries. They are not a global coordinate space, and they never appear in the output contract.

### Segmenter contract

Shape: `(rendered_plain_text: &str) -> Vec<Range<usize>>` — each range identifies one logical unit (word or sentence) in the input plain text.

- The segmenter operates on the **rendered plain text of a single node** — the text a user actually sees in the TUI, with markdown inline syntax already stripped by the renderer.
- Ranges are plain-text byte offsets *within that node*; they are not global document offsets.
- One range per segment (single-`Range<usize>`). Because inline formatting is stripped before segmentation, `**milk**`, `[label](url)`, `*italic*`, and `` `code` `` all reduce to their visible letters before word / sentence boundaries are computed — there is no `mi**lk**` two-range case to handle.
- Source byte positions, when needed for annotations, come from the containing AST node's span — not from the segmenter.
- Per answer to the code-block question: the same rules apply inside fenced and indented code blocks. No language-aware tokenization.

## Definition clarifications (to lock before implementation)

1. **Line** means **source markdown line**, not wrapped terminal row. A paragraph whose source spans multiple lines (soft-wrapped by the author) contributes one line anchor per source line, each mapped to the portion of the node's rendered plain text covering that line. A paragraph written on one source line contributes one line anchor covering the whole node.
2. **Word** should be segmented on rendered plain text but respect markdown stripping rules already used by renderer.
3. Word selection **excludes leading and trailing punctuation**. `word.` selects `word`; the period is not itself a selectable word. Punctuation tokens are boundaries, not units. Apostrophes and hyphens *internal* to a word are preserved as part of the word (`don't`, `word-level` remain one word each); edge cases (abbreviations like `U.S.A.`, ellipses, em-dashes) are pinned via test fixtures.
4. For code blocks (fenced and indented), word segmentation uses the same rules as prose — the segment engine runs over the rendered plain text of the code block with no language-aware tokenization.

## Test strategy

### A) Unit tests: segmentation

Module: `selection::segment`

1. Sentence splitting fixtures:
   - abbreviations (`vs.`, `Dr.`)
   - numbered list markers (`1. item`)
   - wrapped continuation lines (lowercase after newline)
   - uppercase/new paragraph boundaries
2. Word splitting fixtures:
   - punctuation (`word,`, `word.`)
   - contractions (`don't`)
   - hyphenated terms (`word-level`)
   - markdown-derived text (`[label](url)`, inline code)
3. Invariants:
   - ranges sorted, non-overlapping, in-bounds, non-empty after trim.

### B) Unit tests: index building

Module: `selection::index`

1. Given markdown fixtures, assert expected node-to-segments mapping.
2. Assert section/paragraph/block boundaries align with AST-derived nodes.
3. Assert index stability: rebuilding same input yields identical boundary tables.

### C) Unit tests: navigator behavior (core)

Module: `selection::navigator`

Table-driven tests across all units (`section/paragraph/line/sentence/word`):

1. `next` at document end wraps to document start; `prev` at document start wraps to document end.
2. Cross-container transitions are silent (no status message) and correct (last word of sentence N → first word of sentence N+1, etc.).
3. Wordless/empty nodes are skipped silently in same-unit traversal.
4. Roundtrip: `prev(next(anchor)) == anchor` for any interior (non-wrap) anchor.
5. `clamp` upgrades to containing unit: word → sentence = containing sentence; word → paragraph = containing paragraph; etc.
6. `clamp` downgrades to first child: sentence → word = first word in sentence; section → paragraph = first paragraph under heading; etc.
7. Sentence-next skips code blocks entirely; word-next and line-next walk through them normally.
8. Section nav on a heading-less document is a no-op (no movement, no wrap).
9. Line-nav on a soft-wrapped multi-source-line paragraph visits each source line in order.
10. `clamp` to an unavailable unit walks outward to the nearest enclosing node that has the target unit; falls back to no-op if none.

### D) Integration tests: AST + index + projection

1. Parse markdown -> build index -> navigate full document, asserting emitted anchors sequence.
2. For each anchor, projection range slices valid text and expected substring.
3. Section/paragraph highlight ranges match block boundaries.

### E) App-level tests (minimal, behavior contract)

Keep only thin tests in `app.rs`:

1. keypress -> navigator call -> state updated.
2. correct status messages at boundaries.
3. existing output behavior still uses selected context correctly.

## Suggested file/module layout

- `src/selection/mod.rs`
- `src/selection/model.rs`
- `src/selection/segment.rs`
- `src/selection/index.rs`
- `src/selection/navigator.rs`
- `src/selection/projection.rs`
- `tests/selection_navigation.rs`
- `tests/selection_segmentation.rs`
- `tests/selection_projection.rs`

## Phased implementation plan

Phases 0–3 are a **parity refactor** — no observable behavior changes. Phases 4–5 are **additive**. The phase 0 harness is the oracle for req 5 parity throughout.

0. **Regression harness (no production code changes).** Capture current behavior as a test suite before any refactoring: existing navigation keybindings, annotation output fixtures for representative markdown inputs, and `RenderedNode.plain` snapshots. This becomes the parity oracle for phases 1–3.
1. **Extract `selection::model` + `selection::index`.** Introduce the types and build the eager index over the existing AST; wire it into `App` to replace `cursor_node` / `cursor_sentence` internally. No new features. All phase-0 tests green.
2. **Extract `selection::segment`.** Move `text_to_sentences`, `sentence_ranges_from_plain`, and the test-only `split_sentences` into one module; delete duplicates. Phase-0 tests green.
3. **Extract `selection::navigator`.** Move navigation logic out of `App` and `Document` into one pure module driven by the index. Phase-0 tests green.
4. **Additive: add `Line` unit.** Use source-line data already captured on AST nodes; wire into navigator tables and projection. Update help/keymap. Add line-unit tests to the harness.
5. **Additive: add `Word` unit.** Word segmenter in `selection::segment`; projection highlights; new keybindings for word navigation (or a mode switch — decide at implementation time). Add word-unit tests to the harness.
6. **Test migration.** Move high-value app tests into `selection` unit/integration tests; keep app tests thin and behavioral.
7. **Regression sweep.** Annotation context and link-extraction paths exercised with the new line/word units selected.

## Acceptance criteria

1. One canonical segmentation implementation is used everywhere.
2. All `next/prev` behavior for 5 units is covered by unit/integration tests.
3. Word-level selection works across paragraphs/lists/code blocks with deterministic boundary behavior.
4. `App` no longer contains navigation algorithms; it only dispatches and renders.
5. Existing section/paragraph/line/sentence behavior remains unchanged unless explicitly documented.
6. Word selection over inline-formatted text (`**milk**`, `[label](url)`, `*italic*`, `` `code` ``) identifies a single word matching the rendered-visible text. Annotations reference it by rendered text + line number — not source byte ranges.
7. Viewport/render concerns (terminal cell width, wrapping, grapheme handling) do not appear in selection or annotation APIs.

## Open questions (next to pin)

These are unresolved decisions that don't block the architecture but will block specific implementation phases. Ordered by when they're needed.

### 1. Phase 0 regression fixture scope (blocks phase 0)

Need a concrete list of markdown fixtures the harness must cover before the parity refactor can safely start. Candidate categories — confirm or trim:

- plain prose paragraphs (single line; soft-wrapped multi-source-line)
- headings at multiple depths (`#`, `##`, `###`), and documents with zero headings
- unordered, ordered, and nested lists
- task list items (GFM)
- inline formatting: `**bold**`, `*italic*`, `` `code` ``, strikethrough
- links `[label](url)`, autolinks, reference-style links
- images `![alt](src)` — inline and block-level
- fenced code blocks (with and without info-strings), indented code blocks
- GFM tables
- footnotes
- thematic breaks (`---`)
- mixed: list item containing inline-formatted text, code blocks inside list items, headings with inline formatting
- edge cases: empty document, single-word document, single-heading document, document that is one code block

And a paired set of **annotation golden-file outputs** for a fixed keystroke sequence against each fixture, so phase-1–3 refactors can be verified byte-for-byte.

### 2. Word-mode key assignments (blocks phase 5)

Two shapes to choose between:

- **(A) Mode switch.** A dedicated key toggles the current selection unit (section / paragraph / line / sentence / word), and the existing `next`/`prev` keys act on the current unit. Adds one key; keeps the navigation keyset small.
- **(B) Dedicated word keys.** New keys for word-next / word-prev (e.g. analogous to how sentence/line nav are keyed today), unit stays implicit. No mode state; more keys.
- **(C) Mixed.** Keep existing per-unit keys for section/paragraph/line/sentence, add two new keys specifically for word.

Questions: which shape? And for (A) or (C), what key toggles unit / invokes word-nav — avoiding collision with current bindings?

### 3. Edge-case fixtures for navigator behavior (blocks phase 4–5)

Beyond the phase-0 corpus, the new unit-level behaviors need their own fixtures:

- line-nav on a paragraph spanning 5 source lines: assert 5 distinct line anchors, each highlighting the right plain-text slice.
- sentence-nav crossing a code block: assert the code block is silently skipped.
- section-nav on a heading-less document: assert no movement and no status.
- clamp from word inside a code block to sentence mode: assert walk-outward to nearest non-code sentence (or no-op if none).
- wrap-around at both ends for each of the five units.

Confirm this list, or extend.
