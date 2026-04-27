# Modular plan for selection/navigation (section, paragraph, line, sentence, word)

## Requirements

Hard constraints, gathered from a design-review dialogue. Where any section further down conflicts with these, these win.

### Functional

1. **Read-only on the plan file.** Rep never writes the markdown back to disk; applying edits is the consumer LLM's job.
2. **Five selection units.** The user navigates across section, paragraph, line, sentence, and word — in that containment order.
3. **Word-level unlocks two action paths.**
   - `delete this` on a single word.
   - `change` / `revise-to-incorporate-feedback` targeting a single word.
   - Phrase selection (selecting two or more contiguous words as one unit), quoting precise phrases back to the LLM, and wrapping spans are **out of scope for this iteration**.
4. **One unit per action, always.** No range-extension across units (no "word 5 → word 9"). Selection state is a single anchor, not a (start, end) pair.
   - **Forward-compatibility note.** A future iteration may add arbitrary fragment / mouse-selected ranges. When that happens, range-extension state lives on a **new `SelectionRange` type** alongside `SelectionAnchor` — `SelectionAnchor` does **not** gain optional end-anchor fields. The single-anchor APIs (`navigator::next`/`prev`/`clamp`, current `projection` shapes) stay unchanged; range support adds parallel operations on the new type. This iteration's data structures are designed to be additive in that direction (per-node owned `Vec<Range<usize>>` already supports byte-range projections; the emit format already accepts free-text `target:`).
5. **Full behavioral parity with current rep.** All existing keybindings, navigation behavior, output format, and annotation flows continue to work unchanged. The refactor is strictly additive (word-level) plus internal cleanup.

### Output contract

6. **Plain-text emit is authoritative.** The output schema is the plain-text format produced by `App::to_human_output` in `src/app.rs` (`ACTION:` / `WHERE:` / `CONTEXT:` (with `prev` / `target` / `next`) / payload line). This refactor preserves that format. The `target = { unit, text, index, line_span }` sub-struct that appears in this document's history is **not** part of the contract — only the plain text is.
7. **`WHERE:` is `WHERE: line N` for every unit.** No sentence/word indices, no line ranges. The LLM matches the selection by the quoted `target:` text. The `, sentence M` suffix that production emits today is **removed** as part of phase 5 (see Phased implementation plan).
8. **Annotations identify targets by line + text + context, not byte ranges.** The consuming LLM reconstructs edits from text and line numbers. No source byte range or character offset is emitted in the output.

### Internal representation

9. **AST nodes carry source line ranges; per-node text views carry byte ranges within that node.** No byte spans on `DocNode` itself. Both the display plain text (renderer's view) and the selection plain text (index's view, see Req 11) are scoped per node. Annotation output emits line numbers + text only — byte offsets never leak out.
10. **Canonical selection anchor = `(node_idx, unit, unit_idx)`.** This is the stored shape on `SelectionAnchor`. `(source_line, range_within_node_plain_text)` is **derived** from anchor + index when projection or emit needs it; it is not stored on the anchor. No global byte offsets, no codepoint/grapheme/column space is required.
11. **Two plain-text views per node.** The renderer keeps emitting today's display plain text (with footnote-ref `[^N]`, task `[ ]`/`[x]`, and code-block fences visible) for the TUI — `RenderedNode.plain` is unchanged. The selection layer reads from a separate **selection plain text** view, derived once at index-build time per the pinned visibility rules (markers stripped). `selection::segment::plain_text_for_node` is the **single canonical entrypoint** that produces the selection view; index, projection, segmenter, and emit consume only that function. The renderer never sees the selection view; selection-layer code never reads `RenderedNode.plain`.

### Scope

12. **Markdown support = `markdown::ParseOptions::gfm()`.** CommonMark core plus GFM tables, task list items, strikethrough, autolink literals, and footnotes. Front matter, math, MDX, and arbitrary HTML extensions are out of scope.

### Performance

13. **Design target: plan files up to ~5,000 lines.** The selection index may be built eagerly at load time; no lazy/incremental indexing is needed at this size.

### Implications applied below

These requirements drove the following revisions to the architecture sections:

- Req 8 dissolves any need for non-contiguous source spans in the data model — the segment engine operates on selection plain text and emits single-`Range<usize>` segments; source byte mapping is only needed at the AST-node level, not per segment.
- Req 11 separates display from selection: the parity oracle in phase 0 locks the renderer's display output, while the selection layer is built fresh against the new view. The two never share a representation.
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
  - `node_idx: usize` — index into the `Document`'s flat **block-level** node list; stable for a loaded file (see Pinned decisions for domain and order)
  - `unit: SelectionUnit`
  - `unit_idx: usize` — index within the node for line/sentence/word; unused for paragraph (node identity suffices) and section (the section's start node is the anchor; section spans are derived from the start node's `node_idx` plus the document-level section table)
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

Built eagerly from the parsed AST at load time. Not a parallel document model — a derived cache keyed by `node_idx`. **Fully owned**: no borrowed fields from `RenderedNode`, the AST, or `Document` (see Pinned decisions → `SelectionIndex` storage).

- per node (`node_idx`):
  - **selection plain text** — owned `String`, computed via the canonical `selection::segment::plain_text_for_node` (per Req 11). Markdown-syntax markers stripped per the pinned visibility rules.
  - source-line ranges — `Vec<(source_line: usize, range_in_selection_plain_text: Range<usize>)>` pairs, so "Line" selection can map a source line to a span of selection plain text.
  - sentence ranges — owned `Vec<Range<usize>>` in the node's selection plain text.
  - word ranges — owned `Vec<Range<usize>>` in the node's selection plain text.
- per document:
  - linear order tables per unit (paragraph, line, sentence, word). Each entry is a value pair `(node_idx, unit_idx)`; resolving the entry's range is an O(1) index lookup. (Section unit uses the dedicated section table below.)
  - **section table** — `Vec<Section { start_node_idx, end_node_idx, kind }>` where `kind` is `Heading`, `Ol` (top-level OL section starter), or `PreHeading` (section 0). Both endpoints are inclusive. Contiguity (a section spans a contiguous run of `node_idx`) is asserted at build time.

Notes:
- All ranges are **scoped to a node**; they index into that node's selection plain text and never serve as global document offsets.
- Build is eager at load time (req 13 caps the input at ~5k lines). The index lives for the process — there is no reload or file-watcher in this iteration.

### 4) `selection::navigator` (pure next/prev logic)

Single pure API:

- `next(index, anchor, unit) -> NavOutcome` — returns either the new anchor or `Boundary` if there is no further anchor of the unit in the document.
- `prev(index, anchor, unit) -> NavOutcome` — same at the document start.
- `clamp(index, anchor, target_unit) -> SelectionAnchor` — re-anchor onto a different unit type without introducing "between units" state.

`NavOutcome` is `enum { Moved(SelectionAnchor), Boundary }`. Boundary outcomes are the only signal `App` uses to render the boundary-feedback string in the status line; the navigator itself never formats user-facing text.

#### Movement rules

- **Fully silent on movement.** Navigator returns no status messages on within-document moves or on boundary crossings between contained units. `App` writes nothing to the status line for a successful `Moved(_)` outcome.
- **Boundary crossing is implicit.** `next` at the end of the current containing unit advances into the next containing unit. Word-next at end-of-sentence jumps straight to the first word of the next sentence; sentence-next at end-of-paragraph jumps to the first sentence of the next paragraph; etc.
- **No wrap-around.** `next` on the last anchor of a unit returns `Boundary`; `prev` on the first anchor of a unit returns `Boundary`. Selection state stays on the current anchor — it does not change. `App` writes a brief feedback message in the status line's right zone (`"at end"` for `Boundary` from `next`, `"at start"` from `prev`); the message clears on the next keypress. This applies uniformly to all five units.
- **Headings count as paragraphs** for paragraph-unit traversal. Paragraph-next/prev visits headings, paragraphs, list items, code blocks, tables, footnote defs, and HTML blocks (the full block-level domain), in document order.
- **Wordless / unit-less nodes are skipped.** When walking a unit's linear order table, nodes whose selection plain text contributes no entries of that unit type (thematic break, image-only paragraph, empty code block, etc.) produce no anchors and are silently stepped over.
- **Code blocks are excluded from the sentence-level linear order.** Sentence-next / sentence-prev skip fenced and indented code blocks entirely. Users navigate code blocks via line or word units, or select them as a whole paragraph. Code blocks still participate normally in paragraph, line, word, and (via containment) section traversal.
- **Section nav is a no-op when the document has no sections.** A document with no headings *and* no section-starting top-level ordered list has no sections; `next` / `prev` return `Boundary` immediately and mode-cycle into section mode is a silent no-op (no boundary feedback either, since there's nothing to bound).
- **Word-to-word punctuation skip.** Word-next at the last word of a sentence advances to the first word of the next sentence — the sentence-terminating period/`!`/`?` is not visited. Same for `,`, `;`, `:`, em/en-dash, ellipsis at any word boundary.
- **Roundtrip invariant.** `prev(next(x)) == x` for any non-boundary anchor x. (At a boundary, `next(x)` returns `Boundary` and selection state is unchanged, so the roundtrip is trivially `x → x`.)

#### Unit-switch rules (`clamp`)

- **Upgrade (finer → coarser):** return the anchor for the containing unit at the requested coarser level. Word `fast` in sentence S in paragraph P in section Sec → clamp(word → sentence) = S; clamp(word → paragraph) = P; clamp(word → section) = Sec.
- **Downgrade (coarser → finer):** return the first-child anchor of the current unit at the requested finer level. Sentence `Dogs run fast through the park.` → clamp(sentence → word) = word `Dogs`. Section with heading + 3 paragraphs → clamp(section → paragraph) = first paragraph under the heading.
- **Same rule everywhere.** No remembered history of previously-selected child; downgrades always land on first child.
- **Target unit unavailable in current node.** If the requested target unit has no representatives in the anchor's node (e.g. switching to sentence mode while inside a code block), search **backward in document order first** for the nearest preceding node that has the target unit and clamp there — to that node's **last** anchor (the one closest to where the user was). If no preceding node has it, search **forward** and clamp to that node's **first** anchor. If neither direction finds a node with the target unit, the unit switch is a no-op — selection stays on the current anchor and current unit.

#### Visibility rule (shared with `segment`)

A word or sentence exists only where the **selection plain text** for the node has it (per Req 11). The selection plain text is produced by `selection::segment::plain_text_for_node` and has all markdown markers stripped — link URLs, image source URLs, footnote references, task markers, code-block fences. The display plain text on `RenderedNode.plain` may keep some of these (the renderer's choice for the TUI); selection-layer code never reads it. This is the single predicate that decides whether a node contributes anchors at the word/sentence level.

### 5) `selection::projection` (anchor → highlight)

Given `(SelectionAnchor, SelectionIndex)`, return what the render layer should paint. One anchor resolves to exactly one highlight (req 4).

- **Word / Sentence** → `(node_idx, Range<usize>)` — a range in the node's selection plain text. The render layer maps that to a paint span on its display plain text via the source-line mapping the index already carries (since the rendered display and selection views share source-line breaks).
- **Line** → `(node_idx, Range<usize>)` — the selection-plain-text range corresponding to the selected source markdown line.
- **Paragraph** → `(node_idx, full_plain_text_range)` — the whole node's selection plain text.
- **Section** → `Vec<node_idx>` — the constituent nodes drawn from the section table (`start_node_idx..=end_node_idx`); render paints each as a whole-block highlight.

The render layer paints exactly what projection returns. Anchors are never resolved in the render layer; no anchor math happens outside this module.

The same module also exposes the **annotation emit view**: given an anchor, return `(source_line_number, target_text)` where `source_line_number` is the line on which the *selection's text* begins (not the node's first source line). For ListItem selections, the line is always the item's start line (per the line-unit decision: list items have one line anchor). This is what the output contract (req 6) consumes — no byte offsets.

### 6) Thin app integration

`App` changes:

- Replace `cursor_node/cursor_sentence` ownership with a single `SelectionState`.
- Initial `SelectionState`: first node with content (`next_node_with_sentences(0)`), `unit = Sentence`, `unit_idx = 0`. Initial mode (the unit `j/k` acts on) is **sentence**.
- Key handlers call navigator only. Bindings are mode-switch-style:
  - `Space` — cycle unit coarsest → finest (calls navigator `clamp` on each step).
  - `Backspace` — cycle unit finest → coarsest (same).
  - `j` / `Down` — `navigator::next`.
  - `k` / `Up` — `navigator::prev`.
  - `Right` — synonym for `j` (`navigator::next`).
  - `Left` — synonym for `k` (`navigator::prev`).
  - Legacy per-unit keys (`J K H L h l`) are **removed**. The arrow keys' previous sentence-only binding is also removed; they now follow whatever unit is active.
  - `Space` and `Backspace` cycle modes only in normal mode; in input/edit mode they're literal characters.
- **All non-movement keybindings preserved unchanged.** `change`, `feedback`, `insert_before`, `insert_after`, `strike`, `reveal_link`, `annotation_prev`, `annotation_next`, `help`, `quit`, `quit_silent` — every binding outside of the movement family keeps its current key and behavior. The refactor's blast radius is the movement keys only.
- Existing annotation APIs query current sentence/word via projection/context helpers.

## Architecture refinements (from design review)

These refinements sharpen the target architecture above and take precedence where they conflict.

### Three-layer framing

The app decomposes into three layers, in this order:

1. **AST** — parses the source markdown and carries source byte spans per node as the authoritative position data. Read-only on disk per req 1; "round-trip" applies to the TUI render pipeline, not file write-back.
2. **Selection** — an adjustable mechanic over the AST. Anchors identify a node + unit + unit_idx; selection never carries byte offsets of its own.
3. **Viewport / render** — projects AST + selection to the terminal. Owns the **display plain text** (`RenderedNode.plain`), wrapping, grapheme/cell width, and highlight painting. The display plain text is distinct from the **selection plain text** owned by the index (per Req 11); selection-layer code never reads display plain text.

### Coordinate spaces

Exactly four coordinate-ish things exist. Anything claiming to be a fifth is a bug.

1. **source-line** — 1-based line number in the original markdown file. AST node line ranges live here; annotation line numbers come from here.
2. **AST** — structured nodes keyed by `node_idx`, each carrying a `source_lines: Range<usize>`.
3. **Selection** — anchors expressed as `(node_idx, unit, unit_idx)`; resolution to characters goes through the AST + index.
4. **Viewport / lens** — terminal cell geometry, grapheme → cell width, wrap decisions. Must not leak into selection.

Selection plain text and display plain text are both **per-node and scoped** — they index into a single node's text and never serve as global document offsets. Selection plain text never appears in the output contract; display plain text is for the TUI only.

### Segmenter contract

Shape: `(selection_plain_text: &str) -> Vec<Range<usize>>` — each range identifies one logical unit (word or sentence) in the input.

- The segmenter operates on the **selection plain text of a single node**, produced by the canonical `selection::segment::plain_text_for_node` (per Req 11). All markdown syntax is stripped: link URLs, image source URLs, footnote references, task markers (`[ ]` / `[x]`), code-block fences (` ``` `).
- Ranges are byte offsets *within that node's selection plain text*; they are not global document offsets and they do not match offsets into `RenderedNode.plain`.
- One range per segment (single-`Range<usize>`). Inline formatting is stripped before segmentation, so `**milk**`, `[label](url)`, `*italic*`, `` `code` `` reduce to their visible letters; no `mi**lk**` two-range case.
- Source byte positions, when needed for annotations, come from the containing AST node's span and the index's source-line mapping — not from the segmenter.
- Code blocks (fenced and indented) follow the same word/sentence rules as prose. No language-aware tokenization.

## Definition clarifications (to lock before implementation)

1. **Line** means **source markdown line**, not wrapped terminal row. Per-node line-anchor counts: Heading = 1; Paragraph / Table / CodeBlock = N (one per source line); ListItem = **1** (regardless of source-line span — known limitation). See Pinned decisions for the full table.
2. **Word** is segmented on **selection plain text** (markdown syntax stripped by the canonical `selection::segment::plain_text_for_node`, per Req 11). Fenced code-block fences (` ``` `) are excluded from selection plain text and not selectable; indented code blocks have no fences. Display plain text on `RenderedNode.plain` may keep markers visible for the TUI; selection-layer code never reads it.
3. Word selection **excludes leading and trailing punctuation**. `word.` selects `word`. Punctuation tokens are boundaries, not units. Specific edge cases — internal periods (`U.S.A`), em/en-dash, ellipsis, decimals (`3.14`), thousands separators (`1,000`), dates (`2026-04-24`), Unicode alphabetic characters, internal-only apostrophes — are pinned in the Pinned decisions section.
4. For code blocks (fenced and indented), word segmentation uses the same rules as prose — the segment engine runs over the selection plain text of the code block (fences excluded) with no language-aware tokenization.

## Pinned decisions

Authoritative answers to questions that would otherwise be ambiguous during implementation. Where any earlier section conflicts with this one, this one wins. Implementer should treat this section as a checklist.

### Selection-mode meta-rule

Every selection mode traverses contiguously over content. Word-to-word movement skips intervening punctuation across sentence boundaries (the period at end of sentence A is not visited when going from A's last word to B's first word).

### Section unit

- Section is a **span**, not a single node. It runs from a heading (or section-starting top-level ordered list) through the last node before the next section start at equal-or-shallower depth.
- A **top-level ordered list** counts as a section start **only when no `#`-level heading appears anywhere before it in the document** (heading-less plans frequently start with a numbered list — this rule gives them a navigable section structure). Once any heading has appeared, top-level ordered lists are just paragraph-level nodes inside whatever section contains them, not section starters. `depth 0` for this rule means "not nested inside another list" — depth is measured by list nesting, not source-column indentation.
- A section started by a top-level OL spans the entire list (not one section per item) and ends at the next heading or the end of the document.
- **Nested heading levels nest.** `## sub` inside a `# parent` does not end `# parent`'s section; the section ends at the next `#`-or-shallower heading.
- **Pre-heading content** is an implicit "section 0" — addressable by section nav, **only when at least one pre-heading node has ≥1 paragraph-unit anchor** (i.e., the node participates in the paragraph-unit linear order table per the wordless-skip rule). A document whose pre-heading region is empty or contains only thematic breaks / empty headings / wordless nodes has no section 0; section nav goes straight to the first heading. Section-prev from the first real section moves to section 0 if it exists, else returns `Boundary`. Section-next from section 0 goes to the first heading; section-prev from section 0 returns `Boundary`.
- **Section 0 keying line** = the source line of the first node with selectable content (the same line the initial cursor lands on at startup, via `next_node_with_sentences(0)`-equivalent logic on the new index).

### Block-type coverage (`DocNode` variants)

- **Heading, Paragraph, ListItem, CodeBlock, ThematicBreak**: as today.
- **Blockquote**: children flattened to top level (current behavior). No `Blockquote` variant.
- **GFM table**: whole table = one Paragraph node. Each row maps to one source line for line-unit nav. **Per-row plain text** = cells joined by a single space, each cell trimmed. The header-separator row (`|---|---|`) is excluded from selection plain text and is not addressable by line/sentence/word units. **Per-table plain text** in the index = rows joined by `\n` (newline). **`target:` text in emit** = rows joined by a single space (no embedded newlines, per the output schema rule for all units).
- **Footnote definition**: Paragraph node carrying the body text.
- **Footnote reference** (`[^1]` inline): stripped from **selection plain text**; not selectable. The segmenter sees only the surrounding prose, with no marker between left and right neighbors (e.g. `Hello[^1] world.` becomes `Hello world.` — two words: `Hello`, `world`). Display plain text from the renderer keeps `[^1]` visible in the TUI, per current `markdown.rs` behavior.
- **Task list items** (`- [ ]` / `- [x]`): the `[ ] ` / `[x] ` marker is stripped from **selection plain text**. Display plain text retains the marker for the TUI.
- **HTML block**: parsed into the existing `DocNode::CodeBlock` variant (no new enum arm). Selection treats it the same as a code block: whole-selectable, no sentence/word breakdown beyond the per-line / per-word rules already pinned for code blocks. The renderer is responsible for any presentational difference.
- **Inline images**: display plain text shows `[image: <alt>]` (or `[image]` when alt is empty), per `markdown.rs::active_image_alt`. Selection plain text contains only the alt text words — the `[image: ` prefix and trailing `]` are stripped (treated as markers, like footnote refs and task markers). Image source URLs are not in either view.
- **Inline links**: link label text is in both display and selection plain text (selectable). Link URLs and reference-link definition lines (`[ref]: url`) are stripped from selection plain text and are not navigable. Reference-style links (`[label][ref]`) render with their resolved label text in both views.

### `node_idx` domain and order

- **Domain**: block-level nodes only (the variants listed above). Inline nodes (emphasis, links, code spans, footnote refs) are not addressable by `node_idx`; they only contribute to a containing block's text views.
- **Order**: document order — first appearance in the source file, matching the current `Vec<DocNode>` push order.

### Paragraph-unit nav

- **Headings count as paragraphs.** Paragraph-next/prev visits headings, paragraphs, list items, code blocks, tables, footnote defs, and HTML blocks. (ThematicBreak has no content so it is skipped per the wordless-node rule.)
- `clamp(section → paragraph)` returns the section's heading as the first paragraph.
- **Multi-paragraph list items**: known limitation. A `<li>` containing multiple paragraphs collapses to one ListItem node with joined text and one set of source lines. Defer structural fix.

### Line-unit anchors per node

- **Heading**: 1 anchor.
- **Paragraph**: N anchors (one per source line).
- **CodeBlock**: N anchors (one per source line; fence lines excluded — see code-block plain text below).
- **Table**: N anchors (one per source row).
- **ListItem**: **1 anchor** regardless of source-line span.

### Sentence segmenter

- `sentence_ranges_from_plain` (today's render-side segmenter) is the canonical implementation.
- Parser **drops** the `sentences: Vec<String>` field from `DocNode::Paragraph` and `DocNode::ListItem` **in phase 1** (when the index is introduced). Sentences are computed only on selection plain text, only when the index is built. No transient duplicate state is permitted across phases.

### Word boundary rules

- **Word characters** = `\p{Alphabetic}` ∪ `\p{Mark}` ∪ ASCII digits `0-9`. Combining marks attach to the preceding word character (so `cafe\u{301}` → `café` is one word). **Underscore is not a word character** — treated as punctuation, like a hyphen at a word boundary.
- **Hyphen between alphabetic characters is internal**: `word-level` is one word, `state-of-the-art` is one word. Hyphen at the boundary of an alphabetic run (e.g., `-foo`, `foo-`) is stripped as leading/trailing punctuation.
- Internal periods stay: `U.S.A` is one word.
- Em-dash and en-dash are boundaries: `foo—bar` → two words.
- Ellipsis (`...` or `…`) is a boundary.
- Numbers with internal punctuation are one word: `3.14`, `1,000`, `2026-04-24`.
- Apostrophes: internal only. Leading apostrophe (`'tis`) and trailing apostrophe (`users'`) are stripped along with other leading/trailing punctuation.
- **No Unicode normalization.** Input bytes are passed to the segmenter as-is. NFC inputs (the common case) work as expected; NFD inputs (e.g. `cafe\u{301}`) work because combining marks are word characters. Other normalization edge cases (precomposed vs decomposed sequences that change visible boundaries) are documented limitations; rep's input corpus is hand-written or AI-written markdown, essentially always NFC in practice.

### Code block plain text

- Fenced code block fences (` ``` `) are **excluded** from the selection plain text the segmenter and index see. Fence lines are not addressable by line/sentence/word units. Display plain text from the renderer may include the fence lines (current behavior, see `app.rs:171`).
- Indented code blocks have no fences; whole content is the plain text.

### Annotation `WHERE` line number

- `WHERE: line N` for every unit (no `, sentence M`, no line range).
- For sentence/word selections inside a multi-source-line node, emit the source line where the selection's text **begins**, not the node's first source line.
- For ListItem selections (1 line anchor per item per the rule above), emit the item's start line regardless of which sentence/word inside.
- For Heading, Paragraph, Table, CodeBlock, FootnoteDef: emit the line containing the start of the selected sentence/word.
- For Paragraph and Section selections: emit the first source line of the selection (paragraph's first line; heading's line, or section 0's keying line per the Section unit rules).

### Initial selection on load

- Initial node: first node with content (skips a leading thematic break or empty heading). Matches current `next_node_with_sentences(0)` behavior.
- Initial unit: **sentence**.

### Status line

- **Fully silent on navigation.** Navigator returns no status; `App` writes no movement strings.
- **Two zones.** Status line is split: **left zone** carries a persistent **mode indicator** (e.g. `mode: word`); **right zone** carries transient annotation feedback (e.g. `queued change for line 12`) and errors.
- The mode indicator is always visible. Feedback messages appear in the right zone and clear after their normal lifetime — they do not mask the mode indicator.
- On a narrow terminal where both zones can't fit, the right zone (feedback) is truncated first; the mode indicator is never truncated.

### Reload semantics

- **No reload, no file watcher.** Index is built once at startup and lives for the process. User exits and reopens to re-read.

### Phase 0 parity oracle

- Phase 0 goldens lock **current behavior exactly**, including any quirks of the existing sentence segmenter.
- Each phase-0 fixture is a **transcript** (`input.md` + `keys.txt` + `emit.golden.txt` + `anchor.golden.txt`). The emit golden is byte-exact `to_human_output`; the anchor golden is the final `(node_idx, unit, unit_idx)` of `SelectionState`. **Both** must match — emit-only goldens leave a navigation-regression blind spot.
- Bug fixes ("this looks wrong") are **not** part of phases 1–3a. They go in separate post-refactor PRs with their own goldens diffs and justification. Phases 3b/4/5 are explicit-behavior-change phases with named regenerated transcripts; see Phased implementation plan.

### `SelectionIndex` storage

- **Fully owned, no borrowed fields anywhere** — not from `RenderedNode`, not from the AST, not from `Document`. No `<'a>` lifetime on `SelectionIndex` or its sub-structs. Memory cost is negligible at the 5k-line cap; cloning small scalar metadata is cheaper than the lifetime gymnastics it avoids.
- Per node: owned `selection_plain_text: String`, owned `Vec<Range<usize>>` for word ranges, sentence ranges, and source-line ranges.
- Per document: linear order tables per unit. Each entry is a value pair `(node_idx, unit_idx)` — **never** a reference into per-node range tables. Resolving an entry to a `Range<usize>` is an index lookup at navigation time.
- Per document: explicit **section table** — `Vec<Section { start_node_idx, end_node_idx, kind: Heading | Ol | PreHeading }>`. Section nav reads this table directly; it never re-walks `Vec<DocNode>`. The `(start, end)` pair is inclusive on both ends. The contiguity invariant ("a section is a contiguous run of `node_idx` values") is enforced at index-build time with a debug assert and tested directly.
- **Selection plain text per node** is computed once at build time via `selection::segment::plain_text_for_node` (the canonical entrypoint per Req 11). The renderer's `RenderedNode.plain` is never read by selection code.

### Boundary behavior (no wrap-around)

- **No wrap-around in this iteration.** `next` at the last anchor of a unit returns `Boundary`; `prev` at the first returns `Boundary`. Selection state is unchanged on a boundary outcome.
- **App-side feedback.** On `Boundary`, `App` writes `"at end"` (next) or `"at start"` (prev) in the status line's right zone. The message clears on the next keypress. The mode indicator in the left zone is unaffected.
- **Zero-anchor units.** When a unit has zero anchors in the document (e.g., section nav with no headings), `next`/`prev` return `Boundary` immediately and `App` writes no feedback (there's nothing to bound). Mode-cycle into a zero-anchor unit is a silent no-op.
- **Single-anchor units.** When a unit has exactly one anchor, `next` and `prev` both return `Boundary` from that anchor. Selection state stays put.

### Key bindings (mode-switch model)

Selection nav uses a **mode-switch UX**, not per-unit keys.

- **`Space`**: cycle unit coarsest → finest (`section → paragraph → line → sentence → word → section …`).
- **`Backspace`**: cycle unit finest → coarsest (reverse of above).
- **`j` / `Down` / `Right`**: next anchor in the current unit.
- **`k` / `Up` / `Left`**: prev anchor in the current unit.
- Mode change re-anchors per the navigator `clamp` rule (upgrade = containing unit; downgrade = first child).
- **Removed** from the legacy keymap: `J`, `K`, `H`, `L`, `h`, `l`. Their per-unit semantics are subsumed by mode + `j/k`. The arrow keys' previous *sentence-only* binding is removed; arrows now follow the active unit alongside `j` / `k`.
- **All non-movement keybindings are preserved unchanged.** `change`, `feedback`, `insert_before`, `insert_after`, `strike`, `reveal_link`, `annotation_prev`, `annotation_next`, `help`, `quit`, `quit_silent` keep their current keys and behavior. The refactor's keymap blast radius is the movement keys only.
- `Space` and `Backspace` are mode-cycle keys **only in normal mode**. In any input/edit mode they remain literal characters.

### Highlighting visuals

- **Uniform style across all units.** Selection paints today's `bg(Blue) + fg(Black) + Modifier::BOLD` on the selected range, regardless of unit. The user disambiguates mode via the status-line mode indicator (see Status line), not by visual style.
- **Section span** (multi-node): paint each constituent node with the same style. Today's `section_highlight_range` already implements this — keep as-is.
- **Sub-node autoscroll.** Viewport scrolls to keep the selected line/sentence/word visible within a tall paragraph or code block. Today's node-level autoscroll is preserved and extended.
- **No containing-unit indicator.** When in word mode, only the word is painted — the surrounding sentence/line is not separately styled.

### Empty / degenerate documents

- **Empty document** (zero block-level nodes, or only nodes with no selectable content): all unit nav is a silent no-op (immediate `Boundary` with no feedback string written, since there's no anchor to bound). No initial selection is made; the cursor has no anchor. Mode cycling is a no-op. Annotation actions report a "nothing to annotate" error in the status line's right zone.
- **Single-anchor unit on any document**: `next` and `prev` from the only anchor return `Boundary` with `"at end"` / `"at start"` in the right zone; selection state is unchanged.
- **One-code-block document**: section nav is a no-op (no headings, no top-level OL); paragraph/line/word nav has anchors within the code block (boundary at both ends per the rule above); sentence nav is a silent no-op (sentences excluded from code blocks).

### `SelectionAnchor` equality

- `SelectionAnchor` derives `PartialEq` and `Eq` over `(node_idx, unit, unit_idx)`.
- For paragraph and section units, `unit_idx` is canonically `0`. Constructors that build paragraph/section anchors must zero this field; comparison is therefore well-defined and matches "node identity suffices."

### Fixture tooling and goldens

- **Two fixture families** under `tests/fixtures/`:
  - `tests/fixtures/transcripts/<name>/` — phase-0 / phase-0.5 parity oracle. Each transcript has three files: `input.md` (markdown source), `keys.txt` (canonical keypress sequence), `emit.golden.txt` (stdout from `to_human_output` after replay), and `anchor.golden.txt` (final `(node_idx, unit, unit_idx)` of `SelectionState`). The test driver replays `keys.txt` against a headless `App` initialized from `input.md` and asserts byte-exact match for both goldens.
  - `tests/fixtures/emit/<fixture>/<unit>/<action>.golden.txt` — phases 4–5 emit matrix per §F. The fixture's `input.md` lives at `tests/fixtures/emit/<fixture>/input.md`; per-cell goldens are the byte-exact emit stream (FILE header + one ACTION block).
- `UPDATE_GOLDENS=1 cargo test` overwrites mismatching golden files and exits success. Without that flag, drift is a test failure. No `insta` dependency.
- The `UPDATE_GOLDENS=1` flow is implemented in a single test helper (e.g. `assert_golden(actual, path)`); per-test plumbing is not required.
- Regenerating goldens is a deliberate commit with its own diff. The commit message must state the case (intended schema change vs. uncovered bug — see Phase 0 oracle failures).
- **Keypress transcript format.** `keys.txt` is one canonical key name per line, e.g. `j`, `k`, `Space`, `Right`, `c`, `t`, `e`, `x`, `t`, `Enter`. Modifier-prefixed keys are written `Shift+J`, `Ctrl+C`, etc. Comments start with `#`. The driver tolerates trailing whitespace and blank lines.

### Output schema

Production emit is the plain-text format produced by `App::to_human_output` (`src/app.rs`). The refactor preserves that format. Per-unit changes affect only the `WHERE:` line and the contents of the `target:` line.

**File header.** Production emit always prepends a single `FILE: <path>\n` line at the top of the output stream, before any `ACTION:` blocks. The worked examples below show individual `ACTION:` blocks for clarity; in actual emit they are preceded by the `FILE:` header once.

**`WHERE:`** — `WHERE: line N` for **every** selection unit. `N` is the source line where the selection's text **begins**, with one exception: ListItem selections always key to the item's start line (per the line-unit rule that ListItem has 1 line anchor regardless of source-line span). The current `, sentence M` suffix is **removed** in phase 5 and is not reintroduced for any other unit.

**`target:`** — quoted, single-line text. **No embedded newlines in any unit's `target:`.** Per unit:

- **Word**: the word's selection plain text, leading/trailing punctuation already stripped per the word-boundary rules.
- **Sentence**: the sentence's selection plain text. Matches today's behavior.
- **Line**: the **source line verbatim** — preserves `##` heading markers, `|` table pipes, `- ` / `1. ` list bullets. Matches today's behavior. **Exception:** when the selected line is a ListItem (1 line anchor regardless of source-line span), `target:` carries the **full item text, soft-wrapped lines space-joined, list/task markers stripped**. Phase 4 — fixes today's "first line only" footgun for multi-line items.
- **Paragraph**: selection plain text of the whole node, soft-wrapped lines space-joined. For tables (one Paragraph node), rows are space-joined (the in-index newline-joined view is for line-unit nav, not for emit).
- **Section**: selection plain text of all constituent nodes, joined with **single space** between nodes — markdown markers already stripped by the canonical `plain_text_for_node`. No embedded newlines, no blank-line separators.

**`CONTEXT:`** — unchanged. `prev` / `target` / `next` lines are the **source lines** around `WHERE` line `N`, regardless of unit. Out-of-range neighbors emit as empty (omitted). No unit-aware context.

**Single-line target invariant.** Every unit's `target:` is on one quoted line with no embedded newlines. The LLM uses `WHERE: line N` plus the quoted `target:` text to find the selection in the source; structural detail (paragraph wrap points, table rows, section node boundaries) is recovered by reading the source from line `N` forward.

## Worked output examples

Concrete examples for each unit against this sample document:

```
 1: # Project Plan
 2:
 3: This is the first paragraph. It has two sentences.
 4:
 5: Here is a second paragraph that
 6: wraps across multiple source lines.
 7:
 8: ## Section Two
 9:
10: Content of section two.
11: More content.
```

All examples below are **byte-exact production emit** for a `change` action with payload `"<change-text>"`. Production emit prepends a single `FILE: /path/to/plan.md\n` line at the top of the stream before any `ACTION:` block; that header is omitted from the per-unit examples below for brevity. The same shape applies to `revise-to-incorporate-feedback`, `insert-before`, `insert-after`, and reactions — only the `ACTION:` and trailing payload line differ.

**Word** — `first` on line 3, change to `initial`:

```
ACTION: change
WHERE: line 3
CONTEXT:
  prev: ""
  target: "first"
  next: ""
CHANGE: "initial"
```

**Sentence** — `It has two sentences.` on line 3:

```
ACTION: change
WHERE: line 3
CONTEXT:
  prev: ""
  target: "It has two sentences."
  next: ""
CHANGE: "<change-text>"
```

**Line** — line 3 as a whole:

```
ACTION: change
WHERE: line 3
CONTEXT:
  prev: ""
  target: "This is the first paragraph. It has two sentences."
  next: ""
CHANGE: "<change-text>"
```

**Paragraph** — paragraph spanning lines 5–6:

```
ACTION: change
WHERE: line 5
CONTEXT:
  prev: ""
  target: "Here is a second paragraph that wraps across multiple source lines."
  next: "wraps across multiple source lines."
CHANGE: "<change-text>"
```

**Section** — `## Section Two` and its content (lines 8–11), markers stripped:

```
ACTION: change
WHERE: line 8
CONTEXT:
  prev: ""
  target: "Section Two Content of section two. More content."
  next: ""
CHANGE: "<change-text>"
```

Notes on these examples:
- `WHERE: line N` carries the line where the selection's text begins (or the item's start line, for ListItems). No `, sentence M`; no line range.
- `target:` is single-line, no embedded newlines, for every unit. For Line, it's the source line verbatim; for everything else it's selection plain text with markers stripped, soft-wrapped lines / table rows / section nodes all space-joined.
- `CONTEXT.prev` / `CONTEXT.next` are the **source lines** at `N-1` and `N+1`, regardless of unit. Empty source lines emit as `""`; out-of-range neighbors are omitted entirely (the line is not printed).
- The LLM uses `WHERE: line N` and the quoted `target:` text to locate the selection. Structural detail (paragraph wrap points, table rows, section node boundaries) is recovered by reading source from line `N` forward — emit does not encode it.

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
   - contractions (`don't`), leading apostrophe (`'tis`), trailing apostrophe (`users'`)
   - hyphenated terms (`word-level`)
   - markdown-derived text (`[label](url)`, inline code)
   - internal periods: `U.S.A`, `e.g.`
   - em-dash / en-dash boundary: `foo—bar`, `foo–bar`
   - ellipsis boundary: `foo... bar`, `foo…bar`
   - numbers with internal punctuation: `3.14`, `1,000`, `2026-04-24`
   - Unicode alphabetic: `café`, `naïve`, `日本語`
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

1. `next` at document end returns `Boundary` and selection state is unchanged. `prev` at document start does the same. App writes `"at end"` / `"at start"` in the status line right zone; mode indicator in the left zone is unchanged.
2. Cross-container transitions are silent (no status message) and correct (last word of sentence N → first word of sentence N+1, etc.).
3. Wordless/empty nodes are skipped silently in same-unit traversal.
4. Roundtrip: `prev(next(anchor)) == anchor` for any non-boundary anchor.
5. `clamp` upgrades to containing unit: word → sentence = containing sentence; word → paragraph = containing paragraph; etc.
6. `clamp` downgrades to first child: sentence → word = first word in sentence; section → paragraph = first paragraph under heading; etc.
7. Sentence-next skips code blocks entirely; word-next and line-next walk through them normally.
8. Section nav on a heading-less document is a no-op (no movement, no wrap).
9. Line-nav on a soft-wrapped multi-source-line paragraph visits each source line in order.
10. `clamp` to an unavailable unit walks outward — backward in document order first, then forward, then no-op if neither direction finds a node with the target unit.

### D) Integration tests: AST + index + projection

1. Parse markdown -> build index -> navigate full document, asserting emitted anchors sequence.
2. For each anchor, projection range slices valid text and expected substring.
3. Section/paragraph highlight ranges match block boundaries.

### E) App-level tests (minimal, behavior contract)

Keep only thin tests in `app.rs`:

1. keypress → navigator call → state updated (covers `Space` / `Backspace` mode cycling and `j` / `k` / arrow-key movement, with `Right` and `Left` as `j` / `k` synonyms).
2. **No status messages on navigation.** Assert the status line's right zone (feedback) is unchanged after any nav keypress. Status line right zone only changes on annotation actions or errors. The left zone (mode indicator) updates only on mode change.
3. Mode indicator in the status line's left zone reflects the current `SelectionState.unit`.
4. Existing annotation flows (change / feedback / insert / reaction) produce the documented plain-text emit (`ACTION:` / `WHERE: line N` / `CONTEXT:` / `target:` per unit) for the current selection. App-level test asserts wiring only — byte-exact emit content lives in section F.

### F) Output emit tests (full coverage matrix)

Byte-exact tests against fixture files in `tests/fixtures/emit/`. **Coverage is the full Cartesian:** every fixture in the Required-Fixtures corpus × every selection unit (5) × every action type (`change`, `revise-to-incorporate-feedback`, `insert-before`, `insert-after`, `delete this`, reactions). Each cell of that matrix has its own checked-in `.golden.txt`. Fixture file layout:

```
tests/fixtures/emit/<fixture-name>/<unit>/<action>.golden.txt
```

A test driver iterates the matrix; missing combinations (e.g. word-unit `delete this` on a fixture that doesn't contain words) skip with a typed reason rather than silently passing. Fixtures and their existence-of-cells are declared in a single source-of-truth manifest so the matrix is regenerable.

Per-unit invariants the goldens encode (verified across the entire matrix, not just one sample doc):

1. **Word** — `WHERE: line N`; `target:` carries the word's selection plain text (leading/trailing punctuation stripped); no embedded newlines.
2. **Sentence** — `WHERE: line N` (no `, sentence M`); `target:` carries the sentence's selection plain text; no embedded newlines.
3. **Line** — `WHERE: line N`; `target:` carries the source line verbatim (markers preserved) — except ListItem, where `target:` is the full item text, space-joined, list/task markers stripped.
4. **Paragraph** (multi-line) — `WHERE: line N` keyed on the paragraph's first source line; `target:` is the full paragraph selection plain text, soft-wrapped lines / table rows space-joined; no embedded newlines.
5. **Section** — `WHERE: line N` keyed on the heading's source line (or section 0's first content line); `target:` is the full section selection plain text, constituent nodes space-joined, markdown markers stripped, no embedded newlines.
6. **CONTEXT is line-based, not unit-aware.** `prev` / `next` are the source lines at `N-1` and `N+1` — *not* the previous section / paragraph. Matches today's emit.
7. **`FILE:` header.** Each test asserts the production stream begins with `FILE: <path>\n` once before any `ACTION:` block. The per-cell goldens encode the leading `FILE:` line plus the single `ACTION:` block under test.

The full matrix is the only way to catch unit × action × fixture-family regressions; phase 4's ListItem change and phase 5's sentence-suffix strip are detected here even when they only affect a few cells. `UPDATE_GOLDENS=1 cargo test` regenerates mismatching `.golden.txt` files; without that flag, drift is a test failure.

## Testing posture

Authoritative rules for what "phase done" means and how tests are written and maintained. Where this section conflicts with implementer judgment, this section wins.

### Phase gating

- A phase is **done** when, on a clean checkout, `cargo test` passes with zero failures and zero `#[ignore]`d tests, all prior phases' tests still pass, and the phase's own new tests are green.
- No phase ships with new `todo!()` in production paths or new `#[allow(dead_code)]` markers added by the phase. (Existing markers may persist; the phase doesn't add more.)

### Test direction (TDD vs. test-after)

- Phase-N tests and phase-N code may be written in either order **as long as both are committed together** for the phase.
- Exception: **Phase 0** captures current behavior — tests are written first by definition.

### Goldens discipline

- Goldens are **byte-exact**. Any drift fails the test.
- Goldens are regenerated only when an explicit `UPDATE_GOLDENS=1` environment variable is set. Without that flag, a golden mismatch is a test failure.
- Regenerating goldens is a deliberate act with its own commit — not a side-effect of running tests.

### Snapshot / fixture tooling

- Implementer picks the tool that's easiest to test, extend, and maintain (`insta`, plain string equality against fixture files, or otherwise). Required behavior: byte-exact comparison plus the `UPDATE_GOLDENS` flow above.

### Test runner

- `cargo test` is sufficient. No separate test binaries beyond standard `tests/`. No feature flags gating tests.

### Test placement rule

- **Prefer unit tests pushed down** to the smallest module that owns the behavior — faster, easier to iterate.
- **Integration tests** (section D) verify that segment + index + projection compose correctly across an AST. They live in `tests/`.
- **App-level tests** (section E) assert *wiring only*: keypress dispatches to navigator, mode indicator reflects state, emit calls the right helper. They do **not** re-test navigation behavior — that lives in section C.
- Rule of thumb: if a test could be written against `selection::*` directly, it should be — the app layer should be too thin to need its own coverage of selection rules.

### Coverage

- Not a strict line-coverage threshold. The bar is **behavior coverage**: every rule in the Pinned decisions section has at least one test that would fail if the rule were violated.
- "All behaviors covered" should naturally produce ≥80% line coverage. A coverage tool dropping well below that is a signal of untested behavior, not a separate target to chase.

### Phase 0 oracle failures during phases 1–3

- A Phase 0 golden failure during a parity phase is **the implementer's call**. Investigate root cause first.
- If it surfaces a real bug in current behavior (the refactor exposed a previously-hidden flaw), document it, regenerate the golden under `UPDATE_GOLDENS=1`, and ship — with the resolution called out in the commit message.
- If it's an unintended behavior change introduced by the refactor, fix the refactor.
- Either way, **silent golden updates are not acceptable** — the commit message must state which case applies and why.

### Conflicts between existing tests and Pinned decisions

- **Pinned decisions win.** Tests asserting removed legacy behavior (e.g. `J` jumps to next section, status messages on movement, `sentence_index`/`sentence_text` test-only structs) are **deleted, not rewritten**. The replacement behavior is covered by the new tests in sections A–F under the new keymap, silent-nav, and plain-text-emit rules.

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

The plan splits into three categories:

- **Phases 0 / 0.5** — set up oracles and pre-refactor parser work.
- **Phases 1 / 2 / 3a** — parity refactor. No observable behavior changes; phase-0 oracles stay green throughout.
- **Phase 3b** — keymap / status-line UX redesign. Observable change with regenerated goldens.
- **Phases 4 / 5** — additive units (line, word). Each phase has a single targeted behavior change with explicit golden regeneration.
- **Phases 6 / 7** — test migration and regression sweep.

Each phase ships as one or more atomic commits; tests pass at each phase boundary; targeted behavior changes carry commit-message notes per the Phase 0 oracle failures rule.

0. **Regression harness (no production code changes).** Capture current behavior as a test suite before any refactoring. Each fixture is a triple: (markdown input, keypress transcript, expected output) with two golden artifacts per fixture — an emit golden (`stdout` from `to_human_output`, byte-exact) and an anchor-state snapshot (`(node_idx, unit, unit_idx)` at end of run). Existing navigation keybindings, annotation output, and `RenderedNode.plain` snapshots are all locked here. Phase-0 goldens lock **current behavior exactly**, including any quirks of the existing sentence segmenter — bug fixes are post-refactor PRs with their own goldens diffs.
0.5. **Parser-domain phase.** Extend `document.rs` parsing to cover the full `DocNode` domain pinned in Block-type coverage: `mdast::Node::Table` → Paragraph variant (with the table-as-Paragraph rule applied at parse time), `mdast::Node::FootnoteDefinition` → Paragraph variant carrying body text, `mdast::Node::Html` (block-level) → CodeBlock variant. Inline `mdast::Node::FootnoteReference` is handled by the renderer (already emits `[^N]`); selection plain text strips it later in phase 1. New phase-0-style transcript fixtures lock parser behavior for table / footnote-def / HTML-block constructs (these were previously dropped by `_ => {}`, so the goldens are net-new; no regenerating existing goldens). Phase 0 fixtures from the previous step stay green: nothing the existing parser handled changes shape.
1. **Extract `selection::model` + `selection::index`; drop `sentences: Vec<String>`; introduce two plain-text views.** Define `SelectionAnchor`/`SelectionState` per Req 10. Build the eager owned index per Req 11 and the `SelectionIndex` storage rules (selection plain text per node via `selection::segment::plain_text_for_node`; section table; linear order tables by value). Remove the `sentences: Vec<String>` field from `DocNode::Paragraph` and `DocNode::ListItem`. Wire the index into `App` to replace `cursor_node` / `cursor_sentence` internally. No new units, no keymap changes. All phase-0 transcripts green.
2. **Extract `selection::segment`.** Consolidate `text_to_sentences`, `sentence_ranges_from_plain`, and the test-only `split_sentences` into one module; delete duplicates. The canonical `plain_text_for_node` lives here. Phase-0 transcripts green.
3a. **Extract `selection::navigator` (parity-preserving).** Move navigation logic from `App` and `Document` into one pure module driven by the index. **No keymap changes**: the legacy `J K H L h l` and arrow-key sentence binding remain in place; navigator implements them via the appropriate unit-clamp + `next` / `prev` calls under the hood. Phase-0 transcripts green.
3b. **Keymap + status-line UX redesign (observable change).** Replace the legacy keymap with mode-switch (`Space` / `Backspace` / `j` / `k` / `Down` / `Up` / `Right` / `Left`); remove `J K H L h l`; rebind arrows to active-unit `next` / `prev`. Introduce the two-zone status line (mode indicator left, feedback right). Phase-0 keymap goldens are regenerated under `UPDATE_GOLDENS=1` in this same commit; commit message states which keys changed and why. Phase-0 navigation goldens (the ones that didn't depend on a removed key) stay green.
4. **Additive: add `Line` unit.** Wire into navigator tables and projection. Add line-unit tests. **Targeted change:** ListItem at line unit emits the full item text (space-joined, markers stripped) in `target:`, not just the first source line. The phase-0 ListItem goldens that locked "first line only" are regenerated; commit message states this is the planned multi-line ListItem fix.
5. **Additive: add `Word` unit.** Word segmenter in `selection::segment`; projection highlights; word participates in mode-switch (no new keys). Add word-unit tests. **Targeted change:** strip the `, sentence M` suffix from `WHERE:` for sentence selections; every unit now emits `WHERE: line N`. Phase-0 sentence goldens that locked `, sentence M` are regenerated; commit message states this is the planned schema simplification.
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

## Required fixtures

Two fixture families, each with its own directory under `tests/fixtures/`. See "Fixture tooling and goldens" for file layout and the keypress transcript format.

### Phase 0 transcript corpus (parity oracle)

Each entry is a transcript fixture (`input.md` + `keys.txt` + `emit.golden.txt` + `anchor.golden.txt`). The keypress sequence exercises navigation, an action keypress (`c` / `f` / `i` / `s` / `r`), input mode text, and `Enter` to commit; the emit golden captures byte-exact `to_human_output`, the anchor golden captures the final selection state. Phase 1–3a refactors must keep all transcripts green; phase 3b regenerates the keymap-affected transcripts; phase 4 and phase 5 regenerate only the targeted-change transcripts.

Required fixtures:

- plain prose paragraphs (single line; soft-wrapped multi-source-line)
- headings at multiple depths (`#`, `##`, `###`), and documents with zero headings
- unordered, ordered, and nested lists
- task list items (GFM) — verifies marker-stripping rule
- inline formatting: `**bold**`, `*italic*`, `` `code` ``, strikethrough
- links `[label](url)`, autolinks, one reference-style link
- images `![alt](src)` — covers `[image: alt]` rendering rule
- fenced code blocks (with and without info-strings), indented code blocks
- thematic breaks (`---`)
- blockquotes — verifies flatten rule
- multi-paragraph list item — locks current join-children behavior as known limitation
- mixed: list item containing inline-formatted text, code blocks inside list items, headings with inline formatting
- a real existing plan file (e.g. a copy of `modular_plan.md` itself) — covers end-to-end "use rep on actual plans"
- edge cases: empty document, single-word document, single-heading document, document that is one code block, document that is a single multi-source-line ListItem

### Phase 0.5 transcript corpus (new parser domain)

Net-new transcripts for node types the current parser drops (no regenerated goldens, only new ones):

- GFM tables — verifies whole-table-as-Paragraph rule, header-separator stripping, single-space cell join in `target:`
- footnote definitions — verifies definition-as-Paragraph rule, body-text addressability
- footnote references inline — verifies stripping from selection plain text
- HTML block — verifies HTML-as-CodeBlock-variant rule

### Phase 4–5 transcript corpus (new unit behaviors)

Transcripts and emit-matrix fixtures for line/word units. Each is a transcript fixture (input.md + keys.txt + emit golden + anchor golden); some (the boundary, clamp, and mode-cycle ones) are pure navigation tests with no action and only verify the anchor golden. Required:

- line-nav on a paragraph spanning 5 source lines: 5 distinct line anchors, each highlighting the right selection-plain-text slice.
- line-nav on a multi-source-line list item: 1 anchor only (per Pinned decisions); line-next from inside it advances to the next node. Phase 4: `target:` for that line emits the full item text (space-joined, markers stripped).
- line-nav on a heading: 1 anchor; `target:` is the source line verbatim (e.g., `## Section Two`).
- line-nav on a GFM table: N anchors (one per row), header-separator row excluded; per-row `target:` is cells space-joined.
- sentence-nav crossing a code block: code block silently skipped.
- section-nav on a heading-less, top-level-ordered-list-less document: silent no-op into section mode; `next`/`prev` return `Boundary` immediately with no feedback string.
- section-nav on a heading-less document that **does** start with a top-level OL: OL is section 1; document has no section 0 (no pre-OL prose).
- section-nav on a doc with pre-heading prose then `# Heading`: section 0 (the prose) and section 1 (the heading); section-prev from section 1 lands on section 0; section-prev from section 0 returns `Boundary` with `"at start"` feedback.
- section-nav on a doc with `# Heading` followed later by a top-level OL: OL is **not** a section starter (heading already appeared); it sits inside the heading's section.
- `clamp` from word inside a code block to sentence mode: walks **backward** to the nearest preceding sentence-bearing node and lands on its **last** anchor; if no preceding node has sentences, walks **forward** and lands on the **first** anchor; no-op if neither direction finds one.
- boundary at both ends for each of the five units: `next` at last anchor and `prev` at first anchor return `Boundary`; selection state unchanged; `"at end"` / `"at start"` written to right zone of status line.
- zero-anchor unit boundary: section nav on a doc with no sections returns `Boundary` with no feedback string written.
- single-anchor unit boundary: paragraph-nav on a one-paragraph document; both `next` and `prev` return `Boundary`.
- mode-cycle clamp matrix: every (from-unit, to-unit) transition tested for upgrade and downgrade behavior.
- word-prev punctuation skip: from "the." in sentence A, word-prev lands on the last word of sentence A — not the period; same for `,`, `;`, `:`, em/en-dash, ellipsis.
- hyphenated alphabetic compound: `state-of-the-art` segments as a single word.
- underscore boundary: `foo_bar` segments as two words (underscore is not a word character).
- footnote reference invisibility: `[^1]` inline is stripped from selection plain text; word-nav skips it; sentence-nav segments around it. (Display plain text from the renderer keeps `[^1]` visible — that is verified by the phase-0 transcript suite, not this one.)
- task-marker invisibility: `[ ]` / `[x]` are stripped from selection plain text; word-nav segments only over the post-marker text.
- code-block fence invisibility (in selection plain text): line-nav inside a fenced code block visits content lines only; fence lines (` ``` `) are not addressable. Display plain text still shows them.
- mode-cycle keys disabled in input mode: in any annotation-edit field, `Space` and `Backspace` are literal characters and do not cycle modes.
- arrow-key parity: `Right` produces the same anchor as `j`; `Left` produces the same anchor as `k` — across all five units.
- status-line zones: navigator keypress leaves the right zone (feedback) untouched on a `Moved(_)` outcome; updates the right zone with `"at end"` / `"at start"` on a `Boundary` outcome; mode change updates only the left zone; annotation action updates only the right zone.
- empty document: load → no anchor; all keypresses are no-ops; annotation action emits a "nothing to annotate" error in the right zone.
- phase-5 schema strip: sentence selection emits `WHERE: line N` (no `, sentence M`); regenerated phase-0 sentence golden is the new fixture.

The **emit matrix** (per §F) is generated from these transcripts plus the phase-0 corpus. Every transcript that exercises an action becomes one or more cells of `tests/fixtures/emit/<fixture>/<unit>/<action>.golden.txt`; pure-navigation transcripts contribute only to the phase-0 / phase-4–5 transcript suite, not to the emit matrix.
