# Mouse-click selection plan

## Goal

Make left-clicks place the selection at the clicked text and pick the
unit by click count:

| Clicks | Prose paragraph         | Code block / heading / list item |
| ------ | ----------------------- | -------------------------------- |
| 1×     | Word at cursor          | Word at cursor                   |
| 2×     | Sentence containing it  | Line containing it               |
| 3×     | Paragraph (whole node)  | Paragraph (whole node)           |

A click should leave the user as if they had pressed `i` until the
desired unit was active, with the unit_idx pinned to whatever they
clicked on. Today a left-click only snaps `node_idx` and forces
`Sentence` mode (`handle_mouse` → `clamp_sentence`).

The "word→sentence or line→paragraph" progression matches the existing
`mode_cycle` order (Section ↔ Paragraph ↔ Line ↔ Sentence ↔ Word) — we
just want the click count to short-circuit to the right step.

## Pieces to build

### 1. Click-count detection

crossterm reports a single `Down(Left)` per press; multi-click counting
is on us. Track in `App`:

```
last_click: Option<{
    at: Instant,
    row: u16,
    col: u16,
    count: u8,        // 1, 2, or 3 — saturates at 3
}>
```

A `Down(Left)` is a continuation when:
- `now - at <= 500ms`, AND
- `(row, col) == last`  (exact cell — no fuzzy radius needed in a TUI),
- prior `count < 3`.

Otherwise count resets to 1. After 3 the next click drops back to 1
(matches macOS/iTerm behaviour). Threshold 500ms beats 250ms for users
on remote terminals where a few hundred ms of network latency between
key events is normal.

### 2. Spatial mapping: (row, col) → (node, byte offset)

Today `click_to_node` maps row → node by walking `cached_node_heights`,
and column is ignored. We need column precision too.

Approach: capture a per-visible-row map during `draw()`.

Add to `App`:

```
visible_rows: Vec<RowMap>
struct RowMap {
    node_idx: usize,
    byte_start: usize,   // byte offset in rendered_nodes[node_idx].plain
    byte_end:   usize,   // exclusive
    gutter_cols: u16,    // width of indicator/gutter prefix on this row
}
```

Populate it inside the existing draw loop where node lines are built
(`node_lines` construction in `draw`):

- For the special code-block path that builds spans per source line,
  the byte range is exactly the per-source-line range from
  `RenderedNode.line_ranges`.
- For the wrapped path (`render_node_spans` → `wrap_styled_spans`),
  walk the produced segments tracking byte offsets in the original
  node plain — `wrap_styled_spans` already knows where each segment
  came from; we just need it to expose the cumulative byte range per
  produced `Line`. (If it doesn't today, add a parallel return:
  `Vec<(Line, Range<usize>)>`.)
- Empty spacer rows after a node get a `RowMap` with `byte_start ==
  byte_end` and the prior node_idx, so a click on a spacer is a no-op.

Mouse mapping then becomes:

```
fn mouse_to_target(&self, row: u16, col: u16)
    -> Option<(usize /* node_idx */, usize /* byte_offset */)>
{
    let r = self.visible_rows.get(/* row - inner.y */)?;
    let plain = &self.rendered_nodes[r.node_idx].plain;
    let col_after_gutter = col.saturating_sub(inner.x + r.gutter_cols) as usize;
    // walk chars in plain[byte_start..byte_end] accumulating
    // unicode-width::UnicodeWidthChar, return the byte at which width
    // first exceeds col_after_gutter (or byte_end if past-end)
    Some((r.node_idx, byte_offset))
}
```

Past-end (clicked in trailing whitespace of a wrapped line) clamps to
the last byte that's part of a word/sentence — the existing
"contains-or-nearest" idiom in `selection/navigator.rs` is the right
reference.

### 3. Byte offset → (unit, unit_idx)

For each candidate unit on the resolved `node_idx`, we need to find
the unit_idx whose range contains the clicked byte. All four tables
are already available:

| Unit      | Source                                                   |
| --------- | -------------------------------------------------------- |
| Word      | `index.nodes[node_idx].word_ranges` (selection-plain)    |
| Sentence  | `rendered_nodes[node_idx].sentence_ranges` (display)     |
| Line      | `rendered_nodes[node_idx].line_ranges` (display)         |
| Paragraph | whole node (unit_idx = 0)                                |

Word ranges live in selection plain text, the rest in display plain.
The mouse click resolves to a *display* byte. Selection ↔ display can
diverge (smart-punctuation, marker stripping) — same divergence we
just fixed for Line highlight. We need a small mapper:

```
fn display_byte_to_selection_byte(&self, node_idx, display_byte) -> usize
```

Cheapest correct version: do a parallel walk of display vs selection
plain, advancing both, treating runs of `[smart-punct vs straight]`
and `[marker-stripped]` as equivalence classes. Where they line up
trivially (most bytes), they advance in lockstep. Or — simpler —
compute and cache a `display_to_selection: Vec<usize>` map per node at
render time, populated alongside `line_ranges`. Prefer the cached
table; the bookkeeping during render is cheap and makes the click
path trivial.

Picking the unit by click count:

```
let (target_unit, target_idx) = match click_count {
    1 => (Word, find_word_at(...)),
    2 => if node_has_real_sentences(node_idx) {
             (Sentence, find_sentence_at(...))
         } else {
             (Line, find_line_at(...))
         },
    3 => (Paragraph, 0),
    _ => unreachable!(),
};
```

`node_has_real_sentences` returns true for paragraphs/headings whose
selection plain contains sentence-ending punctuation, false for code
blocks, list items rendered as one logical line, and headings without
a terminator. Use the existing `sentence_ranges.len() > 1` heuristic
plus "contains terminal punctuation" as the gate. (For a one-sentence
paragraph the 2-click result is the whole paragraph as a single
sentence — same as triple-click — which is fine.)

### 4. Wiring into `handle_mouse`

Replace the current `Down(Left)` branch:

```
MouseEventKind::Down(MouseButton::Left) => {
    let count = self.bump_click_count(mouse.row, mouse.column);
    let Some((node_idx, display_byte)) =
        self.mouse_to_target(mouse.row, mouse.column) else { return };
    let (unit, unit_idx) = self.click_target_unit(node_idx, display_byte, count);
    self.selection_state.anchor = SelectionAnchor::new(node_idx, unit, unit_idx);
    self.refresh_section_highlight(self.selection_state.anchor);
    self.status = format!("Node {}/{}  {}",
        node_idx + 1, self.doc.node_count(),
        self.unit_label(unit));
}
```

Notes:
- Don't call `clamp_sentence` — that resets to Sentence/0 and would
  undo the click's unit choice.
- Section mode is intentionally not reachable via click; users get to
  it via `i`/`u` key cycling. Triple-click stops at Paragraph.
- Mouse activity continues to clear `notification`/`nav_feedback` (as
  it does today).

### 5. Edge cases

- **Click on annotation gutter / spacer row** — RowMap zero-width or
  out-of-text → no-op (don't move selection, don't bump click count).
- **Click outside `list_inner`** — return early as today (footer,
  popups, borders).
- **Popups visible** — current `handle_mouse` doesn't gate on popups.
  Decide: when `link_popup_urls`/`show_help`/`ast_view_scroll` is
  active, swallow the click rather than moving the underlying
  selection. (Likely a separate pass; flag here so we don't forget.)
- **Click during pending input mode** (`Change`/`Feedback`/`Search`
  buffers active) — same: swallow, since the user is mid-text-entry.
- **Wrapped line whose byte_range is empty** (a pure prefix/suffix
  decoration line) — skip, advance to nearest non-empty row.
- **Combining marks at the click column** — `unicode-width` reports
  width 0; the mapper should attach them to the preceding base char,
  matching how `truncate_to_columns` already handles them.

### 6. Test plan

Unit tests in `src/app.rs`:

- `single_click_word_in_paragraph` — click in the middle of a word,
  assert `(Word, n)` where word range contains the byte.
- `double_click_sentence_in_paragraph` — same paragraph, assert
  `(Sentence, n)`.
- `double_click_line_in_codeblock` — fenced code block, assert
  `(Line, n)` with the right line picked.
- `triple_click_paragraph` — assert `(Paragraph, 0)`.
- `click_on_node_with_emphasis_smart_punct` — paragraph carrying
  `*emph*` and `'`; click after the emphasis run, verify the byte
  resolves to the right *selection* byte (regression for the
  display↔selection mismatch).
- `click_count_resets_on_position_change` — Down at (5, 10) twice in
  100ms, then Down at (5, 11): the third is treated as count=1.
- `click_count_resets_after_timeout` — two clicks 600ms apart count
  as two singles.
- `click_outside_list_inner_is_noop` — selection unchanged.

Integration test in `tests/selection_navigation.rs` (or new file): drive
`handle_mouse` from a known terminal width, render once, click at a
recorded coordinate, assert resulting anchor is what was clicked.

A transcript test isn't a great fit (transcripts are key-event based);
keep mouse coverage in Rust tests.

### 7. Open questions

1. **Does crossterm's `MouseEventKind::Up`/`Drag` matter for selection
   ranges?** Currently no — `rep` doesn't have range-selection. If we
   ever add shift-click range expansion, click-count detection has to
   coexist with drag tracking. Not in scope here.

2. **Right-click semantics?** Could open the link under the cursor
   (Word + has-link → mimic `U`). Out of scope; mention in case a
   later pass revisits.

3. **Do we need a `sentence` vs `line` choice for headings?** A
   heading is one line and usually one sentence — pick whichever the
   gate evaluates to (Sentence when terminal punctuation is present,
   else Line). Either is acceptable visually since they cover the
   same byte range.

4. **`wrap_styled_spans` byte-range exposure** — verify the helper
   already tracks byte offsets per produced segment, or extend it.
   This is the only piece that might require touching code outside
   `app.rs`.

## Suggested commit shape

1. Refactor: `wrap_styled_spans` returns segments + byte ranges (no
   behavioural change).
2. Render: build & cache `visible_rows: Vec<RowMap>` during `draw`;
   add `display_to_selection` map on `RenderedNode`.
3. Click handling: introduce `bump_click_count` + `mouse_to_target` +
   `click_target_unit`; replace `Down(Left)` body in `handle_mouse`.
4. Tests: unit + integration tests above.

Each step compiles and tests independently.
