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

8. **AST nodes carry source line ranges; per-node rendered plain text carries byte ranges within that node.** No byte spans on `DocNode` itself. Annotation output emits line numbers + text only — byte offsets never leak out.
9. **Internal selection coordinate is `(node_idx, source_line, range_within_node_plain_text)`.** No global byte offsets, no codepoint/grapheme/column space is required.

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
- `SelectionIndex` owns its own `Vec<Range<usize>>` data per unit per node — no borrows from `RenderedNode`, no self-referential lifetimes (see Pinned decisions).
- Build is eager at load time (req 11 caps the input at ~5k lines). The index lives for the process — there is no reload or file-watcher in this iteration.

### 4) `selection::navigator` (pure next/prev logic)

Single pure API:

- `next(index, anchor, unit) -> SelectionAnchor` — always returns a valid anchor (wrap-around at document edges, see below).
- `prev(index, anchor, unit) -> SelectionAnchor` — same, with wrap.
- `clamp(index, anchor, target_unit) -> SelectionAnchor` — re-anchor onto a different unit type without introducing "between units" state.

#### Movement rules

- **Fully silent.** Navigator returns no status messages — not on within-document moves, not on boundary crossings, not on wrap. `App` does not write a movement string. (Status line is reserved for annotation feedback and errors.)
- **Boundary crossing is implicit.** `next` at the end of the current containing unit advances into the next containing unit. Word-next at end-of-sentence jumps straight to the first word of the next sentence; sentence-next at end-of-paragraph jumps to the first sentence of the next paragraph; etc.
- **Wrap-around is document-global.** `next` on the last anchor of a unit wraps to the first anchor of that unit in the document — *not* the first anchor in the current containing unit. `prev` on the first wraps to the last. Applies uniformly to all five units.
- **Headings count as paragraphs** for paragraph-unit traversal. Paragraph-next/prev visits headings, paragraphs, list items, code blocks, tables, footnote defs, and HTML blocks (the full block-level domain), in document order.
- **Wordless / unit-less nodes are skipped.** When walking a unit's linear order table, nodes whose rendered plain text contributes no entries of that unit type (thematic break, image-only paragraph if the renderer emits nothing, empty code block, etc.) produce no anchors and are silently stepped over.
- **Code blocks are excluded from the sentence-level linear order.** Sentence-next / sentence-prev skip fenced and indented code blocks entirely. Users navigate code blocks via line or word units, or select them as a whole paragraph. Code blocks still participate normally in paragraph, line, word, and (via containment) section traversal.
- **Section nav is a no-op when the document has no sections.** A document with no headings *and* no top-level ordered list (per the section-unit pinned decisions) has no sections; `next` / `prev` / mode-cycle into section mode are silent no-ops.
- **Word-to-word punctuation skip.** Word-next at the last word of a sentence advances to the first word of the next sentence — the sentence-terminating period/`!`/`?` is not visited. Same for `,`, `;`, `:`, em/en-dash, ellipsis at any word boundary.
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
  - Legacy per-unit keys (`J K H L h l`, `Right`/`Left`) are **removed**.
  - `Space` and `Backspace` cycle modes only in normal mode; in input/edit mode they're literal characters.
- Existing annotation APIs query current sentence/word via projection/context helpers.

## Architecture refinements (from design review)

These refinements sharpen the target architecture above and take precedence where they conflict.

### Three-layer framing

The app decomposes into three layers, in this order:

1. **AST** — parses the source markdown and carries source byte spans per node as the authoritative position data. Read-only on disk per req 1; "round-trip" applies to the TUI render pipeline, not file write-back.
2. **Selection** — an adjustable mechanic over the AST. Anchors identify a node + unit + unit_idx; selection never carries byte offsets of its own.
3. **Viewport / render** — projects AST + selection to the terminal. Owns rendered plain text, wrapping, grapheme/cell width, and highlight painting.

### Coordinate spaces

Exactly four coordinate-ish things exist. Anything claiming to be a fifth is a bug.

1. **source-line** — 1-based line number in the original markdown file. AST node line ranges live here; annotation line numbers come from here.
2. **AST** — structured nodes keyed by `node_idx`, each carrying a `source_lines: Range<usize>`.
3. **Selection** — anchors expressed as `(node_idx, unit, unit_idx)`; resolution to characters goes through the AST + index.
4. **Viewport / lens** — terminal cell geometry, grapheme → cell width, wrap decisions. Must not leak into selection.

Rendered plain text, and `Range<usize>` byte ranges into it, are **per-node and scoped** — they live inside the render / segment / index pipeline and never cross node boundaries. They are not a global coordinate space, and they never appear in the output contract.

### Segmenter contract

Shape: `(rendered_plain_text: &str) -> Vec<Range<usize>>` — each range identifies one logical unit (word or sentence) in the input plain text.

- The segmenter operates on the **rendered plain text of a single node** — the text a user actually sees in the TUI, with markdown inline syntax already stripped by the renderer.
- Ranges are plain-text byte offsets *within that node*; they are not global document offsets.
- One range per segment (single-`Range<usize>`). Because inline formatting is stripped before segmentation, `**milk**`, `[label](url)`, `*italic*`, and `` `code` `` all reduce to their visible letters before word / sentence boundaries are computed — there is no `mi**lk**` two-range case to handle.
- Source byte positions, when needed for annotations, come from the containing AST node's span — not from the segmenter.
- Per answer to the code-block question: the same rules apply inside fenced and indented code blocks. No language-aware tokenization.

## Definition clarifications (to lock before implementation)

1. **Line** means **source markdown line**, not wrapped terminal row. Per-node line-anchor counts: Heading = 1; Paragraph / Table / CodeBlock = N (one per source line); ListItem = **1** (regardless of source-line span — known limitation). See Pinned decisions for the full table.
2. **Word** is segmented on **rendered plain text** (markdown syntax already stripped by the renderer). Fenced code-block fences (` ``` `) are excluded from rendered plain text and not selectable; indented code blocks have no fences.
3. Word selection **excludes leading and trailing punctuation**. `word.` selects `word`. Punctuation tokens are boundaries, not units. Specific edge cases — internal periods (`U.S.A`), em/en-dash, ellipsis, decimals (`3.14`), thousands separators (`1,000`), dates (`2026-04-24`), Unicode alphabetic characters, internal-only apostrophes — are pinned in the Pinned decisions section.
4. For code blocks (fenced and indented), word segmentation uses the same rules as prose — the segment engine runs over the rendered plain text of the code block (fences excluded) with no language-aware tokenization.

## Pinned decisions

Authoritative answers to questions that would otherwise be ambiguous during implementation. Where any earlier section conflicts with this one, this one wins. Implementer should treat this section as a checklist.

### Selection-mode meta-rule

Every selection mode traverses contiguously over content. Word-to-word movement skips intervening punctuation across sentence boundaries (the period at end of sentence A is not visited when going from A's last word to B's first word).

### Section unit

- Section is a **span**, not a single node. It runs from a heading (or top-level ordered list) through the last node before the next section start at equal-or-shallower depth.
- A **top-level ordered list** (depth 0) is treated as a single section spanning the entire list — not one section per item.
- **Nested heading levels nest.** `## sub` inside a `# parent` does not end `# parent`'s section; the section ends at the next `#`-or-shallower heading.
- **Pre-heading content** is an implicit "section 0" — addressable by section nav. Section-prev from the first real section wraps to it; section-next from it goes to the first heading.

### Block-type coverage (`DocNode` variants)

- **Heading, Paragraph, ListItem, CodeBlock, ThematicBreak**: as today.
- **Blockquote**: children flattened to top level (current behavior). No `Blockquote` variant.
- **GFM table**: whole table = one Paragraph node. Plain text = newline-joined cell text. Each row maps to one source line for line-unit nav.
- **Footnote definition**: Paragraph node carrying the body text.
- **Footnote reference** (`[^1]` inline): stripped from rendered plain text; not selectable.
- **Task list items** (`- [ ]` / `- [x]`): the `[ ] ` / `[x] ` marker is part of the prefix, stripped from rendered plain text.
- **HTML block**: treated as a CodeBlock variant (whole-selectable, no sentence/word breakdown).

### `node_idx` domain and order

- **Domain**: block-level nodes only (the variants listed above). Inline nodes (emphasis, links, code spans, footnote refs) are not addressable by `node_idx`; they only contribute to a containing block's rendered plain text.
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
- Parser **drops** the `sentences: Vec<String>` field from `DocNode::Paragraph` and `DocNode::ListItem`. Sentences are computed only on rendered plain text, only when the index is built.

### Word boundary rules

- Internal periods stay: `U.S.A` is one word.
- Em-dash and en-dash are boundaries: `foo—bar` → two words.
- Ellipsis (`...` or `…`) is a boundary.
- Numbers with internal punctuation are one word: `3.14`, `1,000`, `2026-04-24`.
- Word characters: any Unicode `\p{Alphabetic}` plus digits.
- Apostrophes: internal only. Leading apostrophe (`'tis`) and trailing apostrophe (`users'`) are stripped along with other leading/trailing punctuation.

### Code block plain text

- Fenced code block fences (` ``` `) are **excluded** from the rendered plain text the segmenter and index see. Fence lines are not addressable by line/sentence/word units.
- Indented code blocks have no fences; whole content is the plain text.

### Annotation `WHERE` line number

- For sentence/word selections inside a multi-source-line node, emit the source line where the selection's text **begins**, not the node's first source line.
- For ListItem selections (1 line anchor per item per the rule above), emit the item's start line regardless of which sentence/word inside.
- For Heading, Paragraph, Table, CodeBlock, FootnoteDef: emit the line containing the start of the selected sentence/word.

### Initial selection on load

- Initial node: first node with content (skips a leading thematic break or empty heading). Matches current `next_node_with_sentences(0)` behavior.
- Initial unit: **sentence**.

### Status line

- **Fully silent on navigation.** Navigator returns no status; `App` writes no movement strings.
- Status line is reserved for annotation feedback (e.g. `"queued change for line 12"`) and errors.

### Reload semantics

- **No reload, no file watcher.** Index is built once at startup and lives for the process. User exits and reopens to re-read.

### Phase 0 parity oracle

- Phase 0 goldens lock **current behavior exactly**, including any quirks of the existing sentence segmenter.
- Bug fixes ("this looks wrong") are **not** part of phases 1–3. They go in separate post-refactor PRs with their own goldens diffs and justification.

### `SelectionIndex` storage

- Owns its own `Vec<Range<usize>>` per node per unit, plus the linear order tables.
- **No borrows** from `RenderedNode` — no self-referential lifetimes. Memory cost is negligible at the 5k-line cap.

### Wrap-around scope

- **Document-global.** `next` at the last anchor of a unit wraps to the first anchor of that unit in the document, not the first anchor in the current containing unit.

### Key bindings (mode-switch model)

Selection nav uses a **mode-switch UX**, not per-unit keys.

- **`Space`**: cycle unit coarsest → finest (`section → paragraph → line → sentence → word → section …`).
- **`Backspace`**: cycle unit finest → coarsest (reverse of above).
- **`j` / `Down`**: next anchor in the current unit.
- **`k` / `Up`**: prev anchor in the current unit.
- Mode change re-anchors per the navigator `clamp` rule (upgrade = containing unit; downgrade = first child).
- **Removed** from the legacy keymap: `J`, `K`, `H`, `L`, `h`, `l` (their per-unit semantics are now subsumed by mode + `j/k`). The `Right`/`Left` arrows currently bound to sentence nav are also removed.
- `Space` and `Backspace` are mode-cycle keys **only in normal mode**. In any input/edit mode they remain literal characters.

### Highlighting visuals

- **Uniform style across all units.** Selection paints today's `bg(Blue) + fg(Black) + Modifier::BOLD` on the selected range, regardless of unit. The user disambiguates mode via a textual indicator (see below), not by visual style.
- **Section span** (multi-node): paint each constituent node with the same style. Today's `section_highlight_range` already implements this — keep as-is.
- **Mode indicator**: surface the active selection unit somewhere on screen (e.g. status line: `mode: word`). This is the only way the user tells word vs. sentence vs. paragraph apart at a glance.
- **Sub-node autoscroll.** Viewport scrolls to keep the selected line/sentence/word visible within a tall paragraph or code block. Today's node-level autoscroll is preserved and extended.
- **No containing-unit indicator.** When in word mode, only the word is painted — the surrounding sentence/line is not separately styled.

### Output schema

The per-payload selection shape uniformly carries a `target` sub-struct:

```rust
target: {
    unit: "word" | "sentence" | "line" | "paragraph" | "section",
    text: String,
    index: Option<usize>,            // word/sentence: index within the line
    line_span: Option<Range<usize>>, // paragraph/section: multi-line extent
}
```

- Replaces `sentence_index` + `sentence_text` in `ChangeOutput` / `FeedbackOutput` / `InsertOutput` / `ReactionOutput`.
- `index` is populated for **word** and **sentence** (its position within the keyed line). Unused (`None`) for **line**, **paragraph**, and **section**.
- `line_span` is populated for **paragraph** and **section** (multi-line extent, half-open). Unused (`None`) for **word**, **sentence**, and **line**.
- **Multi-line targets are keyed on their first line.** A paragraph spanning lines 5–6 emits one annotation with `line_number = 5` and `line_span = 5..7`. A section spanning lines 8–11 emits one annotation with `line_number = 8` and `line_span = 8..12`. The LLM uses `line_span` to locate the rest.
- **Context is always line-based.** `previous_line` / `current_line` / `next_line` are the source lines around `line_number`, regardless of unit. No unit-aware context.
- **Section `text` is rendered plain text** — markdown markers (e.g. `## `, list bullets, table pipes) stripped. Consistent with "markdown syntax is invisible to selection."

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

A `change` annotation in each unit (TOML-ish for readability; real serialization is whatever the existing emit uses):

**Word** — `first` on line 3, change to `initial`:

```
[[annotations]]
line_number = 3
line_text = "This is the first paragraph. It has two sentences."
context = { previous_line = "", current_line = "This is the first paragraph. It has two sentences.", next_line = "" }
[[annotations.changes]]
target = { unit = "word", text = "first", index = 3 }
change = "initial"
```

**Sentence** — `It has two sentences.` on line 3:

```
target = { unit = "sentence", text = "It has two sentences.", index = 1 }
```

**Line** — line 3 as a whole:

```
target = { unit = "line", text = "This is the first paragraph. It has two sentences." }
# index and line_span both None
```

**Paragraph** — paragraph spanning lines 5–6:

```
[[annotations]]
line_number = 5
line_text = "Here is a second paragraph that"
context = { previous_line = "", current_line = "Here is a second paragraph that", next_line = "wraps across multiple source lines." }
[[annotations.changes]]
target = { unit = "paragraph", text = "Here is a second paragraph that wraps across multiple source lines.", line_span = [5, 7] }
change = "..."
```

**Section** — `## Section Two` and its content (lines 8–11), markers stripped:

```
[[annotations]]
line_number = 8
line_text = "## Section Two"
context = { previous_line = "", current_line = "## Section Two", next_line = "" }
[[annotations.changes]]
target = { unit = "section", text = "Section Two\n\nContent of section two.\nMore content.", line_span = [8, 12] }
change = "..."
```

Notes on these examples:
- `line_text` is the **source markdown line** verbatim (preserves markers like `##`), but `target.text` is the **rendered plain text** of the selection (markers stripped). The two diverge on heading lines and section selections.
- `previous_line` / `next_line` are `Option<String>`; out-of-range becomes `None`.
- `line_span` is a half-open range `[start, end)` matching Rust's `Range<usize>`.

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

1. keypress → navigator call → state updated (covers `Space` / `Backspace` mode cycling and `j` / `k` / arrows movement).
2. **No status messages on navigation.** Assert the status line is unchanged after any nav keypress. Status line only changes on annotation actions or errors.
3. Mode indicator on screen reflects the current `SelectionState.unit`.
4. Existing annotation flows (change / feedback / insert / reaction) emit the new `target` sub-struct with correct `unit`, `text`, `index`, and `line_span` for the current selection.

### F) Output emit tests (worked examples)

Module-level tests that verify the documented worked examples produce the documented output. One test per (unit × payload-type) cell:

1. **Word change** → emits `target = { unit = "word", text, index }` with `line_span = None`.
2. **Sentence change** → emits `target = { unit = "sentence", text, index }` with `line_span = None`.
3. **Line change** → emits `target = { unit = "line", text }` with `index = None`, `line_span = None`.
4. **Paragraph change** (multi-line) → emits one annotation keyed on first line, with `target.line_span = [first, last+1)`, `target.text` = full paragraph rendered plain text, `index = None`.
5. **Section change** (multi-line, multi-paragraph) → emits one annotation keyed on first line (the heading), with `target.line_span = [first, last+1)`, `target.text` = full section rendered plain text **with markdown markers stripped** (no `##`, no list bullets, no table pipes), `index = None`.
6. **Multi-line target context.** For the paragraph and section cases, `previous_line` / `current_line` / `next_line` are the source lines around the keyed first line — *not* unit-aware (e.g. previous_line is the source line before the section's heading, not the previous section).

These tests use the sample document from "Worked output examples" as the fixture and assert byte-exact output.

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

- **Pinned decisions win.** Tests asserting removed legacy behavior (e.g. `J` jumps to next section, status messages on movement, `sentence_index`/`sentence_text` fields) are **deleted, not rewritten**. The replacement behavior is covered by the new tests in sections A–F under the new keymap, silent-nav, and `target` sub-struct rules.

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

0. **Regression harness (no production code changes).** Capture current behavior as a test suite before any refactoring: existing navigation keybindings, annotation output fixtures for representative markdown inputs, and `RenderedNode.plain` snapshots. This becomes the parity oracle for phases 1–3. **Goldens lock current behavior exactly**, including any quirks of the existing sentence segmenter — bug fixes are post-refactor PRs with their own goldens diffs and justification, not part of this refactor.
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

## Required fixtures

The previous "Open questions" section is resolved — see **Pinned decisions** for the answers. The fixture corpus that an implementer must build is listed below.

### Phase 0 corpus (parity oracle)

Markdown fixtures the harness must cover, paired with annotation golden-file outputs for a fixed keystroke sequence so phase-1–3 refactors can be verified byte-for-byte:

- plain prose paragraphs (single line; soft-wrapped multi-source-line)
- headings at multiple depths (`#`, `##`, `###`), and documents with zero headings
- unordered, ordered, and nested lists
- task list items (GFM) — verifies marker-stripping rule
- inline formatting: `**bold**`, `*italic*`, `` `code` ``, strikethrough
- links `[label](url)`, autolinks, one reference-style link (smoke test only)
- images `![alt](src)` — single fixture covering inline and block-level (plain-text smoke test)
- fenced code blocks (with and without info-strings), indented code blocks
- GFM tables — verifies whole-table-as-paragraph rule
- footnotes — verifies definition-as-paragraph + reference-stripping rules
- thematic breaks (`---`)
- blockquotes — verifies flatten rule
- multi-paragraph list item — locks current join-children behavior as known limitation
- HTML block — verifies HTML-as-CodeBlock rule
- mixed: list item containing inline-formatted text, code blocks inside list items, headings with inline formatting
- a real existing plan file (e.g. a copy of `modular_plan.md` itself) — covers end-to-end "use rep on actual plans"
- edge cases: empty document, single-word document, single-heading document, document that is one code block

### Phase 4–5 fixtures (new unit behaviors)

- line-nav on a paragraph spanning 5 source lines: 5 distinct line anchors, each highlighting the right plain-text slice.
- line-nav on a multi-source-line list item: 1 anchor only (per Pinned decisions); line-next from inside it advances to the next node.
- sentence-nav crossing a code block: code block silently skipped.
- section-nav on a heading-less, top-level-ordered-list-less document: silent no-op (no wrap, no status, no movement).
- `clamp` from word inside a code block to sentence mode: walk outward to nearest non-code sentence (or no-op if none).
- wrap-around (document-global) at both ends for each of the five units.
- mode-cycle clamp matrix: every (from-unit, to-unit) transition tested for upgrade and downgrade behavior.
- word-prev punctuation skip: from "the." in sentence A, word-prev lands on the last word of sentence A — not the period; same for `,`, `;`, `:`, em/en-dash, ellipsis.
- footnote reference invisibility: `[^1]` inline is stripped from rendered plain text; word-nav skips it; sentence-nav segments around it.
- code-block fence invisibility: line-nav inside a fenced code block visits content lines only.
- mode-cycle keys disabled in input mode: in any annotation-edit field, `Space` and `Backspace` are literal characters and do not cycle modes.
