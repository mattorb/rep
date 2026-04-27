# Implementation plan for the selection refactor

Sibling to `modular_plan.md`. That doc pins the **what** (architecture, schema, pinned decisions). This doc pins the **how** (file targets, phase boundaries, fixture corpus, commit shape) — at the level a contributor can pick up a phase and start typing.

`modular_plan.md` is authoritative on architecture; this doc must defer to it. Where this doc names a function or line number, that is a snapshot of the codebase as of this writing — verify before editing.

---

## Glossary

- **Anchor coordinates** — `(node_idx: usize, unit: SelectionUnit, unit_idx: usize)`. Stored on `SelectionAnchor`. Other coordinate forms (source line, plain-text byte range) are **derived** at projection / emit time, not stored.
- **Display plain text** — `RenderedNode.plain` produced by `markdown.rs`. Includes today's footnote refs (`[^N]`), task markers (`[ ]` / `[x]`), `[image: alt]` wrappers, code-block fence lines. Read only by the TUI render layer.
- **Selection plain text** — produced by `selection::segment::plain_text_for_node`. Markers stripped per the pinned visibility rules. Read by the index, segmenter, projection, and emit. Selection-layer code never reads display plain text.
- **Boundary outcome** — `NavOutcome::Boundary` returned by `navigator::next` / `prev` when no further anchor exists. Selection state stays put; `App` writes `"at end"` / `"at start"` to the status line right zone.
- **Transcript fixture** — `tests/fixtures/transcripts/<name>/` containing `input.md`, `keys.txt`, `emit.golden.txt`, `anchor.golden.txt`. The phase-0 oracle.
- **Emit matrix** — `tests/fixtures/emit/<fixture>/<unit>/<action>.golden.txt`. The phase-4/5 byte-exact emit coverage.

---

## Scaffolding (before any phase ships)

These items don't belong to a single phase and can land in any order before phase 1 begins.

1. **Empty selection module skeleton** (`src/selection/mod.rs` only) — declares `pub mod model;` `pub mod segment;` `pub mod index;` `pub mod navigator;` `pub mod projection;` and re-exports the public API. Each sub-module is a stub with `#![allow(unused)]` initially. Lets phase 0/0.5 land without a half-built `selection::` tree on disk.
2. **Test driver crate setup** — add `tests/common/mod.rs` (or `tests/transcript_driver.rs`) implementing the transcript runner: parse `keys.txt` into `KeyEvent`s, replay against a headless `App`, capture `to_human_output()` and the final `SelectionState` (or its phase-0 adapter equivalent), compare against goldens. `UPDATE_GOLDENS=1` overwrites mismatches.
3. **`assert_golden` helper** — a single function `assert_golden(actual: &str, path: &str)` used by every phase's tests. Reads the file, compares byte-exact, overwrites on `UPDATE_GOLDENS=1`. Lives under `tests/common/`.
4. **`KeyEvent` parser** — `keys.txt` syntax: one canonical key per line, blank lines and `# comment` lines ignored. Recognized: lowercase ASCII (`j`, `k`, `c`, etc.), `Space`, `Backspace`, `Enter`, `Escape`, `Tab`, `Up` / `Down` / `Left` / `Right`, `Shift+J`, `Ctrl+C`, etc. One small `parse_key(line: &str) -> KeyEvent` function in the driver.

These four items are scaffolding for everything that follows. They land as one or two PRs before phase 0 begins, or fold into phase 0's first commit.

---

## Phase 0 — Regression harness (no production code changes)

### Goal

Lock current behavior with transcript fixtures so phases 1–3a can be verified byte-exact. Lock the anchor-state representation in the canonical `(node_idx, unit, unit_idx)` shape from day one (a one-time adapter converts today's `(cursor_node, cursor_sentence)` to that shape inside the test driver).

### New files

- `tests/transcripts.rs` — `#[test]` per fixture, iterating `tests/fixtures/transcripts/`.
- `tests/common/mod.rs` — transcript driver, `assert_golden`, `parse_key`.
- `tests/fixtures/transcripts/<name>/{input.md, keys.txt, emit.golden.txt, anchor.golden.txt}` — one transcript per fixture in the corpus below.

### Existing files touched

- `src/app.rs` — add a single public method `pub fn current_anchor(&self) -> (usize, &'static str, usize)` that returns `(self.cursor_node, "Sentence", self.cursor_sentence)`. This is the phase-0 adapter; it stays until phase 1 replaces `cursor_*` with `SelectionState`. **One method, one line range; no logic changes.**
- `src/main.rs` — no change. The transcript driver constructs `App` directly.

### Fixture corpus (phase-0 transcripts)

Each is `(input.md, keys.txt, emit.golden.txt, anchor.golden.txt)`. Author one transcript per item below; the `keys.txt` exercises the relevant nav + at least one annotation action (so emit golden is non-empty). Two pure-navigation transcripts at the end.

- `prose-single-line/` — single-paragraph doc, sentence nav within line.
- `prose-soft-wrap/` — paragraph wrapping 3 source lines.
- `headings-h1-h2-h3/` — three-level heading nesting.
- `headings-zero/` — doc with no headings, just paragraphs.
- `list-unordered/` — bullets, depth 1.
- `list-ordered/` — numbered list, depth 0 (top-level OL — section starter).
- `list-nested/` — nested unordered list inside an ordered list.
- `list-task/` — `[ ]` / `[x]` markers.
- `inline-formatting/` — `**bold**`, `*italic*`, `` `code` ``, strikethrough.
- `links/` — `[label](url)`, autolink, one reference-style link.
- `images/` — `![alt](src)` inline.
- `code-fenced/` — fenced code with language tag.
- `code-indented/` — 4-space-indented code.
- `thematic-break/` — `---` between sections.
- `blockquote/` — quoted prose (verifies flatten rule).
- `multi-paragraph-list-item/` — list item containing two paragraphs (locks current join-children behavior).
- `mixed/` — list item with inline formatting + a code block in another list item.
- `real-plan/` — copy of `modular_plan.md` itself.
- `edge-empty/` — empty file.
- `edge-single-word/` — one-word file.
- `edge-single-heading/` — one-line file with just `# heading`.
- `edge-one-code-block/` — entire doc is one code block.
- `nav-only-prose/` — pure navigation transcript (no actions); exercises sentence/section/block boundary crossing within a multi-paragraph doc.
- `nav-only-list/` — pure navigation transcript; exercises section nav over a top-level OL.

### Verification gate

`cargo test transcripts` — all transcripts green. No phase-0 fixture is allowed to be `#[ignore]`'d.

### Commit boundary

One commit per scaffolding item (1–4 above), then one commit per ~5 fixtures, or one fat commit "phase 0 corpus" if you'd rather review it as a unit. The `current_anchor` adapter on `App` lands in the first commit of this phase.

---

## Phase 0.5 — Parser domain coverage

### Goal

Make `document.rs` recognize the four mdast variants currently dropped at `document.rs:221` (`_ => {}`): `Table`, `FootnoteDefinition`, `FootnoteReference`, `Html`. Per the pinned Block-type coverage and Q6 answer:

- Table → folds into `DocNode::Paragraph` (one Paragraph per table; cells joined per the table canonical-serialization rule).
- FootnoteDefinition → folds into `DocNode::Paragraph` (carrying the body text).
- FootnoteReference → handled inline by the renderer (it already emits `[^N]` to display plain text per `markdown.rs:291`); selection plain text strips it. **No `DocNode` impact.**
- Html (block-level) → folds into `DocNode::CodeBlock` (HTML-as-CodeBlock-variant rule, recycling existing variant per Q6 (i)).

No new `DocNode` variants. No changes to the renderer in this phase.

### Existing files touched

- `src/document.rs:128` — `collect_nodes`: add four arms before the `_ => {}` catch-all.
  - `mdast::Node::Table(t)` — flatten cell text to space-joined per row, rows joined `\n`; produce `DocNode::Paragraph { text, sentences: vec![], source_lines }`. Header-separator row excluded. (The empty `sentences` vector is a temporary holdover; phase 1 drops the field entirely.)
  - `mdast::Node::FootnoteDefinition(fd)` — extract body text via `extract_plain_text`; produce `DocNode::Paragraph` similarly.
  - `mdast::Node::Html(h)` — produce `DocNode::CodeBlock { language: None, content: h.value, source_lines }`.
- `src/document.rs:236` — `extract_plain_text`: ensure footnote-reference inline children render to empty string (or a known sentinel). Today they're handled by recursion into `.children()` which returns nothing for `FootnoteReference`. Verify and pin with a test.
- `src/markdown.rs` — verify renderer handles tables / HTML blocks. The current renderer is line-based (`render_markdown_line`); confirm `pulldown-cmark`'s table events are emitted in a sensible way for the TUI, or document a known visual limitation. **No selection-layer change.**

### New fixtures

Net-new transcripts (these node types weren't producible before this phase, so no goldens regenerate):

- `parser-gfm-table/` — 2-column, 2-row table.
- `parser-footnote-def/` — paragraph with `[^1]` reference plus a `[^1]: definition.` line.
- `parser-footnote-ref-inline/` — multiple footnote refs inline in prose.
- `parser-html-block/` — `<details>...</details>` block.

### Verification gate

`cargo test transcripts` — all phase-0 transcripts green (none of those involve the new node types, so they shouldn't regress) and all four new transcripts green. Manual: load a real plan with a table in the TUI and visually verify the renderer doesn't blow up.

### Commit boundary

One commit per new variant arm (4 commits) or one combined "0.5: parser domain coverage" commit. New fixtures land in the same commit(s).

---

## Phase 1 — `selection::model` + `selection::index`; drop `sentences` field

### Goal

Land the canonical anchor and the owned, eager index per Req 10 + Req 11 of `modular_plan.md`. Replace `App.cursor_node` / `App.cursor_sentence` with a single `SelectionState`. Remove the `sentences: Vec<String>` field from `DocNode::Paragraph` and `DocNode::ListItem` — sentences move to the index.

This is the largest phase by line count. It stays as one phase (per Q6) to avoid the duplicate-state risk of an interim 1a/1b.

### New files

- `src/selection/model.rs`:
  - `enum SelectionUnit { Section, Paragraph, Line, Sentence, Word }`
  - `struct SelectionAnchor { node_idx: usize, unit: SelectionUnit, unit_idx: usize }` — derive `Copy, Clone, PartialEq, Eq, Debug`. Constructors must zero `unit_idx` when `unit` is `Paragraph` or `Section`.
  - `struct SelectionState { anchor: SelectionAnchor }` — single-field for now; the wrapper exists so future state (active-mode-cycle history, multi-cursor, etc.) has a home without changing call sites.
  - `enum NavOutcome { Moved(SelectionAnchor), Boundary }`.
- `src/selection/segment.rs`:
  - `pub fn plain_text_for_node(node: &DocNode, source_lines: &[String]) -> String` — the canonical entrypoint per Req 11. Implementation strips footnote refs, task markers, image `[image: ]` wrappers, code-block fences. Phase 1 may stub this with an initial implementation that calls into an existing helper; phase 2 owns the full canonical implementation.
- `src/selection/index.rs`:
  - `struct Section { start_node_idx: usize, end_node_idx: usize, kind: SectionKind }` and `enum SectionKind { Heading, Ol, PreHeading }`.
  - `struct NodeIndex { selection_plain_text: String, source_line_ranges: Vec<(usize, Range<usize>)>, sentence_ranges: Vec<Range<usize>>, word_ranges: Vec<Range<usize>> }` — fully owned, no `<'a>`.
  - `struct SelectionIndex { nodes: Vec<NodeIndex>, paragraphs: Vec<(usize, usize)>, lines: Vec<(usize, usize)>, sentences: Vec<(usize, usize)>, words: Vec<(usize, usize)>, sections: Vec<Section> }` — linear order tables hold `(node_idx, unit_idx)` by value.
  - `pub fn build(doc: &Document, source_lines: &[String]) -> SelectionIndex` — eager build at load time.
  - Phase-1 scope includes paragraph + line + sentence linear-order tables and the section table. **Word ranges and word linear-order table are computed but empty until phase 5.**
  - Build asserts the contiguity invariant for sections (debug-only).

### Existing files touched

- `src/document.rs`:
  - Remove `sentences: Vec<String>` from `DocNode::Paragraph` (lines 18–51).
  - Remove `sentences: Vec<String>` from `DocNode::ListItem` (lines 18–51).
  - Drop `text_to_sentences` (lines 253–286) — no longer needed; index computes sentences. Phase 2 deletes the test-only `split_sentences` and `app.rs::sentence_ranges_from_plain` formally; phase 1 may leave them as private helpers temporarily if `selection::segment` calls into them.
  - Keep `next_node_with_sentences`, `prev_node_with_sentences`, `next_section`, `prev_section`, `next_block`, `prev_block` for now — phase 3a deletes them.
- `src/app.rs`:
  - Remove `cursor_node: usize` (line 354) and `cursor_sentence: usize` (line 355).
  - Add `selection_state: SelectionState` and `index: SelectionIndex` fields.
  - `App::load` builds the index once after parsing the doc.
  - Replace `current_anchor` adapter (added in phase 0) with one that reads from `selection_state` directly.
  - **Movement helpers** (`move_node` line 860, `move_sentence` line 895, `move_section` line 965, `move_block` line 994) are rewritten to update `selection_state` instead of `cursor_*`. They still live in `app.rs`; phase 3a moves them out. Internally they call into the index for sentence/section/block lookups instead of the old `Document` helpers.
  - **`to_human_output`** (line 2295) updated to read selection from `selection_state` and look up `target:` text via the index. Output format is unchanged: `WHERE: line N, sentence M` for sentence selection still emits per phase-0 parity.

### Tests added

- `src/selection/model.rs` unit tests: anchor equality, `unit_idx` zero invariant for paragraph/section.
- `src/selection/index.rs` unit tests: build a small fixture document, assert per-node sentence ranges, source-line ranges, section table contents (start/end/kind), linear-order tables.
- `src/selection/segment.rs` unit tests: `plain_text_for_node` stripping rules — footnote ref, task marker, code fence, image wrapper. (Initial coverage; phase 2 expands.)

### Verification gate

`cargo test transcripts` — phase-0 + phase-0.5 transcripts byte-exact green. Phase-1's anchor goldens automatically work since phase 0 wrote them in the canonical shape; phase 0's adapter is now redundant and removed in this phase.

### Commit boundary

Three or four commits in order:

1. **Module skeleton + types** — `src/selection/{mod,model,segment,index}.rs` with types defined, `build` returning an empty index, `plain_text_for_node` stubbed.
2. **Index population** — fill in source-line, sentence, section ranges. Tests for index assertions.
3. **`App` migration** — swap `cursor_*` for `selection_state` + `index`; rewrite movement helpers; `to_human_output` consumes the new state. Phase-0 transcripts green.
4. **`DocNode.sentences` field removal** — drop the field; remove `text_to_sentences`. Phase-0 transcripts still green (sentence text now comes from the index).

The four commits ship together (phase 1 atomic-ish). Each commit individually compiles and runs `cargo test`.

---

## Phase 2 — `selection::segment` consolidation

### Goal

Make `selection::segment` the single canonical home for sentence and word segmentation and `plain_text_for_node`. Delete every duplicate.

### Existing files touched

- `src/selection/segment.rs`:
  - Add `pub fn segment_sentences(plain: &str) -> Vec<Range<usize>>` — moved from `app.rs:222` `sentence_ranges_from_plain`.
  - Phase 2 does **not** add `segment_words`; that's phase 5.
  - Round out `plain_text_for_node` to the canonical implementation (footnote refs stripped, task markers stripped, image wrapper stripped, code-block fences excluded).
- `src/app.rs`:
  - Delete `sentence_ranges_from_plain` (line 222).
  - Replace call sites with `selection::segment::segment_sentences`.
- `src/markdown.rs`:
  - Delete the test-only `split_sentences` / equivalent if present (the report mentions it indirectly via test coverage for sentence splitting).
- `src/selection/index.rs`:
  - `build` calls `selection::segment::segment_sentences` and `plain_text_for_node` directly. Phase 1 may have routed through a temporary stub; phase 2 finalizes.

### Tests added

- `src/selection/segment.rs` unit tests: every fixture in §A of `modular_plan.md` (abbreviations, numbered list markers, wrapped continuation, uppercase boundaries, hyphenated words, internal periods, em-dash / en-dash, ellipsis, numbers with internal punctuation, Unicode alphabetic).
- Inherit existing markdown.rs sentence-segmentation tests by reference (move them to `selection::segment` or call into the new function from the existing module).

### Verification gate

`cargo test transcripts` — phase-0 + phase-0.5 transcripts byte-exact green. `cargo test selection` — segment unit tests green.

### Commit boundary

Two commits:

1. **Segmenter consolidation** — move sentence segmenter into `selection::segment::segment_sentences`; route all readers through the new function.
2. **Duplicate cleanup** — delete the dead helpers in `app.rs`, `document.rs`, `markdown.rs`. Phase-0 transcripts green.

---

## Phase 3a — `selection::navigator` extraction (parity-preserving)

### Goal

Move all navigation logic out of `App` and `Document` into one pure module. Implement the `next` / `prev` / `clamp` API per Movement rules in `modular_plan.md`. **Keymap, status line, and observable behavior unchanged.** Phase-0 keymap goldens stay green.

### New files

- `src/selection/navigator.rs`:
  - `pub fn next(index: &SelectionIndex, anchor: SelectionAnchor) -> NavOutcome` — uses the linear-order table for `anchor.unit`. Returns `Boundary` when at the end.
  - `pub fn prev(index: &SelectionIndex, anchor: SelectionAnchor) -> NavOutcome` — symmetric.
  - `pub fn clamp(index: &SelectionIndex, anchor: SelectionAnchor, target: SelectionUnit) -> SelectionAnchor` — implements upgrade (finer→coarser containing unit) and downgrade (coarser→finer first child) per pinned rules. Backward-then-forward fallback on unavailable target unit, with last-anchor / first-anchor selection per pinned `clamp` rule.

### Existing files touched

- `src/app.rs`:
  - `move_node` (line 860), `move_sentence` (line 895), `move_section` (line 965), `move_block` (line 994) collapse into thin wrappers. Each is one or two lines: read current anchor, call `navigator::next` or `prev` with the appropriate unit, on `Moved(a)` update `selection_state.anchor = a` and update `section_highlight_range` if needed, on `Boundary` do nothing. **No status-line changes** — phase 3a keeps current "silent on boundary" behavior to preserve goldens.
  - `handle_normal_key` (line 453) keeps current key bindings unchanged. `J K H L h l` still navigate as today via the wrapper helpers.
- `src/document.rs`:
  - Delete `next_node_with_sentences`, `prev_node_with_sentences`, `next_section`, `prev_section`, `next_block`, `prev_block`. The navigator owns this logic now.
  - Delete `is_block_start`, `block_end`, `is_section`, `is_paragraph`, `is_heading` if unused after the removals (the index has equivalent information).

### Tests added

- `src/selection/navigator.rs` unit tests: §C of `modular_plan.md` table-driven cases, scoped to the units that exist in phase 3a (Section / Paragraph / Sentence). Line and Word are no-ops in phase 3a.
- Boundary tests: `next` at last anchor returns `Boundary`; `prev` at first returns `Boundary`. (App-level boundary feedback string is phase 3b.)
- Roundtrip: `prev(next(x)) == x` for non-boundary anchors.
- Wordless skip: thematic break, image-only paragraph contribute no anchors.

### Verification gate

`cargo test transcripts` — all phase-0 / phase-0.5 transcripts byte-exact green. `cargo test selection` — navigator unit tests green.

### Commit boundary

One commit. Phase 3a is mechanical extraction; bisecting inside it has limited value.

---

## Phase 3b — Keymap + status-line UX redesign (observable change)

### Goal

Switch to mode-switch keymap. Two-zone status line. Phase-0 transcripts that exercise removed keys regenerate; transcripts that don't, stay green.

### Existing files touched

- `src/app.rs`:
  - `handle_normal_key` (line 453):
    - Remove `J`, `K`, `H`, `L`, `h`, `l` arms.
    - Rebind `Right` and `Left` to active-unit `next` / `prev` (synonyms for `j` / `k`).
    - Add `Space` arm: `mode_cycle(&mut self, forward: true)` — cycles `section → paragraph → line → sentence → word → section…`. Each cycle step calls `navigator::clamp` with the new unit.
    - Add `Backspace` arm: `mode_cycle(&mut self, forward: false)`.
    - In input modes (search, change, feedback, insert, edit), `Space` and `Backspace` remain literal characters.
  - Add `nav_feedback: Option<String>` field next to `notification`. Populated on `Boundary` outcome (`"at end"` for `next`, `"at start"` for `prev`); cleared at the top of `handle_normal_key` so it shows for exactly one keypress.
  - Add `mode_indicator(&self) -> &'static str` returning `"section"` / `"paragraph"` / `"line"` / `"sentence"` / `"word"`.
- `src/ui.rs:1577–1595`:
  - Two-zone footer. Left zone: `mode: <unit>` (always visible). Right zone: `nav_feedback` if set, else `notification` if set, else the help hint. Truncate the right zone first under width pressure; mode is never truncated.
- `src/app.rs::move_*` helpers:
  - On `Boundary` outcome from the navigator, set `nav_feedback = Some(...)`.

### Tests added

- App-level: keypress → status-line zones snapshot. `Space` cycles mode; left zone updates; right zone untouched. `j` at last anchor sets right zone to `"at end"`; subsequent `j` clears it (stale message gone) and either moves or repaints the same boundary message.
- §E thin tests: keypress dispatches to navigator; mode indicator reflects state; non-movement keys (`change`, `feedback`, `reveal_link`, etc.) unchanged.

### Goldens regenerated

Run `UPDATE_GOLDENS=1 cargo test transcripts` once the implementation lands. Inspect the diff:

- Transcripts that used `J K H L h l` will fail at the keypress-parse step (those keys no longer exist) — those transcripts must have their `keys.txt` rewritten by hand to use the new keymap before regenerating goldens.
- Transcripts that used arrow keys for sentence nav (when active unit was sentence) keep the same emit since arrows are still bound; the anchor goldens stay byte-exact.
- Status-line zones differ in any transcript that captured the footer; regenerate.

Commit message must enumerate which transcripts changed and why (per Phase 0 oracle failures rule).

### Verification gate

`cargo test transcripts` — green after regeneration. Manual smoke test: load a real plan in the TUI, exercise mode cycling and boundary feedback.

### Commit boundary

One commit for the implementation; one commit for the goldens regeneration. The goldens commit must include only `.golden.txt` and `keys.txt` diffs (no code), so the implementation's correctness is reviewable separately.

---

## Phase 4 — `Line` unit

### Goal

Add line-unit selection across all block-level node types per Line-unit anchors per node pinned rules. Implement the multi-line ListItem `target:` change.

### Existing files touched

- `src/selection/index.rs`:
  - `build`: populate `lines` linear-order table. Per-node line anchors per the pinned table (Heading=1, Paragraph=N, CodeBlock=N excluding fences, Table=N excluding header-separator, ListItem=1).
- `src/selection/navigator.rs`:
  - Line-unit `next` / `prev` (already handled by the linear-order table; no new logic).
- `src/selection/projection.rs`:
  - New file. `pub fn highlight_for(anchor: SelectionAnchor, index: &SelectionIndex) -> Highlight` returning `(node_idx, Range<usize>)` for line / sentence / word / paragraph and `Vec<usize>` for section. Actually, projection has been implicitly used in earlier phases — phase 4 may be where it gets carved out. Pin the carve-out here if not already done.
- `src/app.rs::to_human_output` (line 2295):
  - Add Line case for `WHERE:` and `target:`. ListItem at line unit emits the **full item text**, soft-wrapped lines space-joined, list/task markers stripped (per the pinned schema rule). Non-ListItem line emits the source line verbatim.

### Tests added

- Phase 4–5 transcripts (per `modular_plan.md` Required Fixtures § Phase 4–5):
  - `line-paragraph-5lines/`, `line-listitem-multi-source-line/` (with the **regenerated** target text), `line-heading/`, `line-table/`, `line-codeblock-fenced/`, etc.
- §F emit matrix expansion: every existing fixture × `line` unit × every action type. Heavy; matrix driver iterates.

### Goldens regenerated

`tests/fixtures/transcripts/list-*/` ListItem goldens: their `target:` was `"first line of item"` and is now the full joined item text. Regenerate. Commit message states this is the planned multi-line ListItem fix per the Output schema rule.

### Verification gate

`cargo test transcripts` — all green. `cargo test emit_matrix` — line cells green.

### Commit boundary

Two commits: implementation, then goldens regeneration.

---

## Phase 5 — `Word` unit

### Goal

Word-level selection. Strip the `, sentence M` suffix from sentence emit (last targeted schema change).

### Existing files touched

- `src/selection/segment.rs`:
  - `pub fn segment_words(plain: &str) -> Vec<Range<usize>>` — implements the word-boundary rules: word chars = `\p{Alphabetic}` ∪ `\p{Mark}` ∪ ASCII digits; underscore is punctuation; hyphen between alphabetics is internal; em/en-dash boundaries; ellipsis boundary; internal periods/apostrophes preserved; numbers with internal punctuation stay as one word.
- `src/selection/index.rs`:
  - `build`: populate per-node `word_ranges` and the `words` linear-order table.
- `src/selection/navigator.rs`:
  - Word-unit transitions; punctuation skip when crossing sentences (period/`!`/`?` not visited).
- `src/selection/projection.rs`:
  - Word case.
- `src/app.rs::to_human_output` (line 2295):
  - Add Word case (`target:` = the word's selection plain text).
  - **Strip `, sentence M` suffix** from sentence selections (lines 2339–2345, 2371–2377, 2407–2413, 2441–2443). Every unit now emits `WHERE: line N` only.

### Tests added

- §A word-segmentation tests (per `modular_plan.md`): hyphenated, contractions, leading/trailing apostrophe, hyphenated terms, markdown-derived text, internal periods, em-dash, ellipsis, numbers, Unicode alphabetic.
- Phase 4–5 transcripts: word-prev punctuation skip, hyphenated alphabetic compound, underscore boundary.
- §F emit matrix: every fixture × `word` unit × every action type.

### Goldens regenerated

Every transcript that emitted a sentence-keyed `, sentence M` suffix regenerates. Commit message states this is the planned schema simplification.

### Verification gate

`cargo test transcripts` — all green. `cargo test emit_matrix` — full matrix green.

### Commit boundary

Three commits: word segmenter + index + projection (selection-layer only); `to_human_output` word case + suffix strip (app-layer); goldens regeneration. Splitting the schema change from the additive work makes the diff easier to review.

---

## Phase 6 — Test migration

### Goal

Move high-value tests out of `src/app.rs`'s 1144-line test module into the appropriate `selection::*` unit tests or `tests/` integration tests. App-level tests stay thin (wiring, status-line zones, key dispatch).

### Process

1. Categorize the 75 tests in `app.rs:2576–3719`:
   - **Selection mechanics** (sentence nav, node nav, block nav, section nav, sentence-context, byte-range validity, hard-wrap handling, sentence movement edge cases) → move to `tests/selection_navigation.rs` or to a `selection::navigator` unit test.
   - **Output format** (sentence indices, WHERE format, FILE: header) → tested in §F emit matrix already; delete or keep as a smoke test in `app.rs`.
   - **Search / input modes / scrolling / layout** — stay in `app.rs` (truly app-level).
   - **Gutter rendering / annotation display** — stay in `app.rs` or move to a `tests/gutter.rs` integration test.
2. For each moved test: rewrite to use the `selection::` API directly rather than `App::test_app() + key replays`. Faster, less brittle.
3. After each chunk of moves, `cargo test` should still pass.

### Verification gate

`cargo test` — all tests green. App-level test count down to ~25 (app-specific behaviors only).

### Commit boundary

One commit per category (~5 commits). Easy to review individually.

---

## Phase 7 — Regression sweep

### Goal

Exercise the full annotation flow with line and word selections in real scenarios. Verify link extraction (`reveal_link`) still works when the selected unit is line / word, not just sentence.

### Process

1. Run the TUI on `modular_plan.md` itself. Cycle through all five units. Apply one of each annotation action at each unit. Quit; inspect emitted output. (Manual.)
2. Add transcript fixtures for any flow not yet covered: `reveal_link` with word-unit selection, `delete this` (strike) with word-unit selection.
3. Run `clippy --all-targets -- -D warnings`. Fix any new warnings introduced by the refactor.
4. Run `cargo fmt --check`. Format if needed.

### Verification gate

`cargo test` clean. `cargo clippy` clean. Manual TUI smoke pass on a real plan file.

### Commit boundary

One commit for clippy / format fixes. One commit for added regression-sweep transcripts.

---

## Risk register

Items most likely to bite during implementation. Each has a mitigation already in `modular_plan.md` or this doc; flagging them here for visibility.

- **Phase 1 cross-file diff size.** Touching `app.rs`, `document.rs`, and the new `selection::` tree in one phase is large. Mitigation: the four sub-commits inside phase 1 each compile and pass `cargo test`; reviewers can step through them.
- **Phase 0.5 parser changes silently shift `node_idx` ordering.** Adding Table / FootnoteDef / Html arms means more nodes are produced. Phase-0 transcripts that exercised an unhandled-variant document would regress (their `node_idx`-keyed anchor goldens shift). Mitigation: phase-0 corpus deliberately excludes those node types (they go in phase 0.5's net-new corpus), and transcripts that contain inline `[^N]` references pre-0.5 see those refs hit the renderer the same way regardless of parser handling.
- **Phase 3b goldens regeneration must inspect, not blanket-accept.** Running `UPDATE_GOLDENS=1` blindly will overwrite every drift, including unintended ones. Mitigation: regenerate, then `git diff` and read every changed file before committing. The two-commit split (impl, then goldens) makes this enforceable in code review.
- **`selection::projection` carve-out timing.** This module is referenced by phase 4 above but doesn't have a dedicated phase. Either phase 1 or phase 3a should carve it out as part of its commit; the index's per-unit lookup logic is the natural seam. Pick a phase before phase 4 and commit `src/selection/projection.rs` then.
- **Multi-paragraph list items remain a known limitation.** `extract_list_item_text` joins children with spaces, and `<li>` containing two paragraphs becomes one `ListItem` node with one `source_lines` range. Phase 0 locks current behavior; phase 4's ListItem `target:` change does not fix the structural issue. Document as deferred.
- **`section_highlight_range` field on `App`.** Today it's set inside `move_section`. After phase 3a, it should be derived from `selection_state.unit == Section` + the section table — not stored separately. Either keep it as a derived helper or move it to projection. Decide before phase 3a.
- **Future fragment / mouse selection compatibility** (per Req 4 forward-compatibility note in `modular_plan.md`). Phase 1's `SelectionAnchor` must remain a flat 3-field `Copy + Eq` value type. Do **not** add `Option<EndAnchor>` or any range-state field to `SelectionAnchor`. When fragment selection is added in a future iteration, it lives on a new `SelectionRange` type with parallel APIs; existing single-anchor APIs stay untouched. Reviewers of phase 1 should reject any PR that introduces optional range fields on `SelectionAnchor` itself.

---

## Branching strategy

- `main` is always shippable. Each phase's commits land on a feature branch (`refactor/phase-1-index`, etc.) and merge into `main` only when its verification gate passes.
- A phase that requires multiple commits (1, 3a, 3b, 4, 5) keeps them on its branch; the merge is a single rebase or fast-forward, no merge commit, so `main` history stays linear and bisectable.
- Goldens-regeneration commits are always second (after the implementation commit) on a phase branch — this keeps the implementation diff and the test-data diff separately reviewable.
- No `--no-verify`. No force-push to `main`.

