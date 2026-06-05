# Combine Sentence + Line into one auto-flipping mode

## Goal

Collapse the user-facing distinction between Sentence mode and Line
mode. The unified mode auto-picks per node:

- Multi-line paragraphs with terminal punctuation → walk by **sentence**.
- Single-line fragments (headings, list items, soft-wrapped paragraphs
  without `.`/`!`/`?`, code blocks) → walk by **line**.

The decision is made once at index-build time and baked into a single
linear-order anchor table, so j/k navigation, clamp, click, projection,
and emit all behave the same way: one unit, one table, no mode flip
mid-walk.

The five-step cycle (Section ↔ Paragraph ↔ Line ↔ Sentence ↔ Word)
becomes four (Section ↔ Paragraph ↔ Sentence ↔ Word). Space cycles
through 4 instead of 5; `i`/`o` adjust by one less step.

## The predicate (per-node decision)

This is the single rule the whole feature hinges on. Decide once at
`SelectionIndex::build`:

```
PrimaryUnit::Lines if node is CodeBlock
PrimaryUnit::None  if node is ThematicBreak  (no anchors)
otherwise (Paragraph / Heading / ListItem):
  if source_line_ranges.len() > 1 AND plain contains any of '.' '!' '?'
    → PrimaryUnit::Sentences
  else
    → PrimaryUnit::Lines
```

Walk-throughs (these motivate the rule — keep them when reviewing
edge-case fixes):

| Node                                      | Source lines | Has `.!?` | Decision  |
| ----------------------------------------- | ------------ | --------- | --------- |
| `# Heading text`                          | 1            | no        | Lines     |
| `# Question?`                             | 1            | yes       | Lines     |
| `Single sentence paragraph.`              | 1            | yes       | Lines     |
| `First.\nSecond.` (two-line paragraph)    | 2            | yes       | Sentences |
| Soft-wrapped paragraph with no terminator | 3            | no        | Lines     |
| `- bullet text`                           | 1            | no        | Lines     |
| `- Item with two sentences. And more.`    | 1*           | yes       | Lines     |
| ` ```\nfn x() {}\n``` `                   | 1            | n/a       | Lines     |

\* ListItem is normalized to a single line range regardless of source
span (`src/selection/index.rs:262-264`).

This differs from today's `node_has_sentence_semantics` (`src/app.rs:
1464-1478`) in two ways:

1. **List items with terminal punctuation now walk as one line** (was:
   one whole-item sentence anchor, behaviorally similar but conceptually
   different — the new rule matches the user's stated intent).
2. **Single-line paragraphs walk as one line** (was: one whole-paragraph
   sentence anchor, byte-identical, just a different label).

In every case the *byte ranges* either match exactly or differ in a way
the user wanted ("walk by sentence in real prose, not by source line").

## Naming the unified unit

Two options:

**A. Reuse `SelectionUnit::Sentence`.** Drop `Line` from the enum.
Pros: minimum golden churn (Sentence appears in ~31 goldens vs Line in
~6); mode label stays "sentence" which reads naturally for prose.
Cons: the variant name is misleading for line-flavored anchors.

**B. Rename the variant to something neutral** (e.g. `Block`,
`Statement`, `LineOrSentence`). Pros: name matches semantics. Cons:
every fixture and test that mentions `Sentence` or `Line` changes; a
fresh string like "block" appears in the footer where users have been
seeing "sentence".

Recommended: **A**. Keep `Sentence`, drop `Line`. The variant is
already documented as "the canonical mid-level unit" in user-facing
docs; we're widening its definition, not changing its role. If we
later want a clearer name, rename in a separate cosmetic pass.

The rest of this plan assumes A.

## Pieces to build

### 1. `PrimaryUnit` tag on `NodeIndex`

`src/selection/index.rs:36-45` — extend `NodeIndex`:

```rust
pub enum PrimaryUnit { Sentences, Lines, None }

pub struct NodeIndex {
    pub selection_plain_text: String,
    pub source_line_ranges: Vec<(usize, Range<usize>)>,
    pub sentence_ranges: Vec<Range<usize>>,
    pub word_ranges: Vec<Range<usize>>,
    pub primary_unit: PrimaryUnit,   // new
}
```

Compute during `SelectionIndex::build` (`index.rs:69-131`) per the
predicate above. Both `sentence_ranges` and `source_line_ranges` keep
their current builders — we use whichever the per-node decision
selected.

### 2. Unified linear-order table

Replace `SelectionIndex::sentences` and `SelectionIndex::lines`
(`index.rs:54-55`) with one table:

```rust
pub struct SelectionIndex {
    pub nodes: Vec<NodeIndex>,
    pub paragraphs: Vec<(usize, usize)>,
    pub anchors: Vec<(usize, usize)>,   // sentence-or-line, per node primary
    pub words: Vec<(usize, usize)>,
    pub(crate) sections: Vec<Section>,
}
```

Build it in the same loop:

```rust
match nodes[node_idx].primary_unit {
    PrimaryUnit::Sentences => for si in 0..sentence_ranges.len()
                                 { anchors.push((node_idx, si)); },
    PrimaryUnit::Lines     => for li in 0..source_line_ranges.len()
                                 { anchors.push((node_idx, li)); },
    PrimaryUnit::None      => {}
}
```

Naming note: keeping the field name short (`anchors`) is fine since
it's the only mid-level table — but `sentences` works too if we want
to preserve grep-ability. Pick one and run with it. (Rest of plan uses
`anchors`.)

### 3. Drop `SelectionUnit::Line`

`src/selection/model.rs:9-15, 19-27, 30-38, 42-48` — remove the `Line`
variant from the enum, the two `*_str` matches, and `CYCLE_ORDER`.

`CYCLE_ORDER` becomes:

```rust
pub const CYCLE_ORDER: [Self; 4] = [
    Self::Section, Self::Paragraph, Self::Sentence, Self::Word,
];
```

Tests at `model.rs:90-153` need updating: drop the line/sentence
distinction asserts; cycle-order assertion is now 4 entries.

### 4. Navigator `unit_table`

`src/selection/navigator.rs:68-82` — drop the `Line` arm and point
`Sentence` at `index.anchors`:

```rust
fn unit_table(index: &SelectionIndex, unit: SelectionUnit)
    -> Cow<'_, [(usize, usize)]>
{
    match unit {
        SelectionUnit::Sentence  => Cow::Borrowed(&index.anchors),
        SelectionUnit::Paragraph => Cow::Borrowed(&index.paragraphs),
        SelectionUnit::Word      => Cow::Borrowed(&index.words),
        SelectionUnit::Section   => Cow::Owned(/* unchanged */),
    }
}
```

j/k now walks the unified table — moving from a paragraph (sentence
anchors) into a heading (line anchor) is just the next entry. No
transition logic, no flicker, no special case in the navigator. This
is the whole payoff of doing the merge at the index layer.

### 5. Projection

`src/selection/projection.rs:30-77` — fold the `Line` arm into
`Sentence`:

```rust
SelectionUnit::Sentence => {
    let n = &index.nodes[anchor.node_idx];
    let r = match n.primary_unit {
        PrimaryUnit::Sentences =>
            n.sentence_ranges.get(anchor.unit_idx).cloned(),
        PrimaryUnit::Lines =>
            n.source_line_ranges.get(anchor.unit_idx).map(|(_, r)| r.clone()),
        PrimaryUnit::None => None,
    }.unwrap_or(0..0);
    Highlight::Range(anchor.node_idx, r)
}
```

### 6. App layer: click, clamp, capture, strike

These are the per-call-site touch-ups. Each one currently branches on
`Sentence` vs `Line`; after the merge they collapse to one path.

- **`click_target_unit`** (`src/app.rs:1430-1441`): replace the
  conditional with an unconditional `Sentence` choice; the per-node
  decision is now baked into the index, so the same `unit_idx` lookup
  works whether the node is sentence-flavored or line-flavored.
  Helpers `find_sentence_at` / `find_line_at` (`app.rs:1449-1457`)
  collapse to one `find_anchor_at` that consults `primary_unit`.

- **`clamp_sentence`** (`src/app.rs:1496-1520`): unchanged in behavior
  — still snaps to the unified Sentence unit. Internally now reads
  from `nodes[node_idx]` according to its `primary_unit` to find the
  legal `unit_idx` count.

- **`current_target_capture`** (`src/app.rs:1565-1573`): drop the
  `Line` arm; `Sentence` arm now returns the right thing for both
  cases. May want to rename to `current_anchor_context` for clarity.

- **`target_text_for_unit`** (`src/app.rs:1649-1691`): same — fold
  the `Line` arm into `Sentence`, dispatching on `primary_unit`.

- **`unit_highlight_for`** / **per-unit branches at app.rs:2811-2813,
  2906-2907, 2926-2927, 3431-3445**: same fold — one `Sentence` arm
  that handles both per-node flavors.

- **Strike (`x`)**: today the help text reads "Sentence-mode only"
  (`src/output.rs:33-35`). After merge, strike is available in the
  unified Sentence mode for *any* node — paragraphs, headings, list
  items, code blocks. Strike emit currently records
  `target_unit: SelectionUnit::Sentence` (six call sites at
  `src/app.rs:3709, 3719, 3729, 4491, 4527, 4881`); these stay.
  Update the user-visible help string to drop the restriction.

  *Decision*: when the strike target is on a Lines-flavored node,
  do we emit `target_unit: "sentence"` or `"line"` in the
  serialized output? Recommend: `"sentence"` always (the user-facing
  unit name is sentence; the line-vs-sentence flavor is an internal
  detail). This keeps strike emit logic untouched and avoids a new
  enum-to-string branch.

### 7. UI label & status

`src/app.rs:1284-1286, 1354-1360` — `mode_str()` returns "sentence"
already; status bar formats the same way. No change needed beyond
removing the now-unreachable "line" label.

Help screen and `KeymapOutput.strike` doc string (`src/output.rs:33-35`)
need wording updates: drop "Sentence-mode only".

### 8. Backward compat for saved annotations

`SelectionUnit::as_str()` is used to serialize `target_unit` in
emitted artifacts. If a previously-saved file contains
`target_unit: "line"`, deserialization needs to accept it for
forward-compat. Check `src/markdown.rs` and `src/output.rs` for
parsers — current code only writes annotation files (no read path
for annotations), so this is likely a non-issue. **Verify** by
grepping for `"line"` and `"sentence"` parses; if any exist, map
both to the unified `Sentence` variant on read.

### 9. Tests & golden updates

Affected:

- **Unit tests in `src/selection/model.rs:90-153`** — drop Line
  asserts, update cycle-order length to 4.
- **Unit tests in `src/selection/index.rs:471-685`** — add tests for
  `PrimaryUnit` decision per node type. Existing
  `paragraph_sentences_round_trip` etc. still pass since we keep
  `sentence_ranges` and `source_line_ranges` populated.
- **Unit tests in `src/selection/projection.rs:79-185`** — rewrite
  `line_highlight_for_heading_covers_full_plain` and
  `line_highlight_for_multi_line_paragraph_returns_per_line_slice`
  using `SelectionUnit::Sentence` with `PrimaryUnit::Lines`-flavored
  nodes; assertions on byte ranges remain correct.
- **App tests at `src/app.rs:5309-5353`** — `double_click_*` tests
  asserting Sentence vs Line both become "asserts Sentence"; the
  byte ranges they verify still differ by node type, just under one
  variant name.
- **`tests/selection_navigation.rs`** — full walks; rerun and update
  any asserted unit-name strings.
- **`tests/emit_matrix.rs:55-60`** — drops the "line" row, keeps
  "sentence". May need `target_unit` golden refresh.
- **Transcript goldens (~37 files)** — `anchor.golden.txt` files
  containing `(N, Line, M)` rewrite to `(N, Sentence, M)`. The
  unit_idx number is the same since the line table for those nodes
  becomes the anchor table for those nodes. Bulk find-and-replace
  works *if* every Line entry was for a Lines-flavored node — verify
  by spot-checking 2-3 fixtures, then commit the bulk update.
- **Golden in `strike-word-mode/emit.golden.txt:6`** — already says
  `target: "sentence"`; no change.

Strategy: do the source changes first (steps 1-7), let the test
suite fail noisily, then update goldens in a single follow-up commit
with `cargo insta` or manual review. Don't try to update both at once
or you'll lose visibility on which assertions changed semantically vs
just-a-rename.

## Edge cases

- **A code block inside a section walked by paragraph** — Paragraph
  unit ignores `primary_unit`, so it still walks one anchor per
  contributing node. Unchanged.
- **A paragraph with `…` ellipsis but no `.`/`!`/`?`** — the
  predicate sees no terminator, so it walks by line. If we want
  `…` to count as terminal punctuation, add it to the check (low
  cost). Recommend: keep ASCII-only for now; revisit if a real doc
  hits the limitation.
- **A paragraph that's >1 source line but contains exactly one
  sentence-ending punct mid-text** — walks by sentence (one
  anchor covering everything up to and including that punct, then
  one for the trailing fragment). Same as today's segmenter
  behavior on Sentence.
- **Empty `source_line_ranges`** — already returns empty
  `sentence_ranges` and contributes no anchors. Unchanged.
- **An anchor saved at `(node, Line, k)` from before the change**
  — the `Line` variant no longer exists. If we're persisting
  selection state across runs (we aren't today), we'd need a
  read-time fallback. Skip for now; the in-memory `SelectionAnchor`
  is rebuilt every session.

### Tables

GFM tables are currently parsed as a single `DocNode::Paragraph`
(`src/document.rs:198-232`): cells are space-joined within a row,
rows are `\n`-joined, and the `| --- |` separator is encoded in
`Table.align` so it never appears in plain text. The index treats
this paragraph like any other — line anchors filtered by
`is_table_separator_line` (`src/selection/index.rs:339-355`),
sentence anchors from the standard segmenter.

Behavior under the merge depends on the segmenter's view of the
joined plain text:

- **Lowercase-content table** (`| alpha | 1 |\n| beta | 2 |`): the
  segmenter doesn't split on `\n` before a lowercase char, so
  `sentence_ranges.len() == 1` (whole table is one sentence). The
  refined predicate (`sentences > 1 AND lines > 1`) → **Lines**.
  Walks row-by-row, same as today's Line mode. ✓
- **Capitalized-content table** (`| Alpha | First |\n| Beta |
  Second |`): segmenter splits on each `\n` (next char uppercase),
  so `sentence_ranges.len() == row_count == source_line_ranges.len()`
  → **Sentences**. Anchor count and byte ranges identical to today's
  Line mode for that table. ✓
- **Table with multi-sentence cell content** (`| Alpha | First.
  Second. |`): segmenter splits inside the body cell on `. ` +
  uppercase. `sentence_ranges.len() > source_line_ranges.len()`,
  both > 1 → **Sentences**. Walks include mid-cell splits.

The third case is a **real regression**: today users can drop into
Line mode to walk row-by-row regardless of cell content; after
the merge that escape hatch is gone for tables whose cells contain
multiple terminator+capital sequences. Rare in practice (most
markdown tables hold short single-sentence-per-cell data) but
real for tables that carry prose-ish notes in cells.

Two ways to handle it:

1. **Accept it.** Ship as-is; mention in release notes that
   row-level table walking is no longer guaranteed when cells
   contain multi-sentence prose. Zero extra code.
2. **Force Lines for tables in the predicate.** Tables today land
   on `DocNode::Paragraph` with no metadata distinguishing them
   from regular paragraphs, so we'd need to either:
   - Add a flag: `DocNode::Paragraph { text, source_lines, is_table: bool }`
     populated at parse time (`src/document.rs:198-232` flips it
     to `true`), then make the predicate force `PrimaryUnit::Lines`
     when the flag is set. ~10 LOC at the parse layer + ~3 LOC
     in the predicate.
   - Or detect at index time by checking whether any source line
     in the node's `source_lines` range matches
     `is_table_separator_line` — already a function in
     `selection/index.rs`, so the cost is ~5 LOC. Slightly
     fragile (depends on the separator surviving in source_lines)
     but no schema change.

   Recommend the metadata flag if we go this route — it makes
   table-ness an explicit property rather than a heuristic, and
   future table-specific features (per-cell selection, anyone?)
   can hang off the same flag.

Recommendation: **start with option 1** (accept the regression).
The byte ranges for the broken case are still correct; the user
just walks more sub-anchors than they used to. If real-world
docs surface the issue, option 2 is a localized follow-up.

## Open questions

1. **Should `i`/`o` snap to a sentence-flavored node when crossing
   into a line-flavored one?** No — the unified unit means there's
   nothing to snap. Currently `mode_adjust` at
   `src/app.rs:1237-1257` passes through `clamp`; verify the same
   call still produces the right anchor with one fewer cycle step.

2. **Naming.** Stick with `Sentence` (recommendation A above) or
   rename? Decide before starting the index-layer changes — if
   renaming, do it as the first commit so subsequent diffs show
   semantic changes only.

3. **Field name `anchors` vs reusing `sentences`.** `sentences` keeps
   the existing field name and avoids cascading renames in tests. But
   it's misleading once it can hold line entries. Recommend
   `anchors` and accept the rename churn. (Roughly 15 references in
   `src/`.)

4. **Predicate refinement.** Should the predicate look at
   *post-segmenter* sentence count (`sentence_ranges.len() > 1`)
   instead of "contains terminal punctuation"? They're nearly
   equivalent but the segmenter is more careful (it requires
   terminator + space + capital). Recommend: use
   `sentence_ranges.len() > 1` as the gate — it's already computed,
   matches the segmenter's view of what counts, and naturally falls
   through to Lines when there's only one sentence anyway.

   Refined rule:
   ```
   PrimaryUnit::Sentences if (not CodeBlock) and sentence_ranges.len() > 1
                              and source_line_ranges.len() > 1
   PrimaryUnit::Lines     otherwise
   ```

   The `> 1` on both axes ensures: single-sentence single-line nodes
   are Lines (saves a redundant whole-node sentence anchor), and
   the predicate doesn't have to peek at characters in the plain
   text.

## Suggested commit shape

1. **Index layer**: add `PrimaryUnit` enum + `NodeIndex.primary_unit`
   field + unified `anchors` table on `SelectionIndex`. Keep the
   old `sentences` and `lines` tables alongside, populated identically.
   No behavior change yet — just new infrastructure. Tests for the
   predicate live with this commit.

2. **Navigator + projection**: switch `unit_table` and
   `highlight_for` to read from `anchors` / `primary_unit`. Verify
   the navigation test suite still passes (the unit name is still
   `Sentence`, and the per-node anchor count is identical to before
   for every node — sentence-flavored nodes use sentence_ranges,
   line-flavored use source_line_ranges).

3. **App layer**: collapse the `Line` / `Sentence` branches in click
   handler, capture, target-text-for-unit, strike, status. Remove
   `find_line_at`, `find_sentence_at` in favor of `find_anchor_at`.

4. **Drop `SelectionUnit::Line`**: remove the variant, drop the old
   `index.lines` and `index.sentences` fields (now redundant with
   `anchors`), update the four-step `CYCLE_ORDER`. Compiler errors
   guide the cleanup. App tests at `src/app.rs:5309-5353` update
   here.

5. **Goldens + integration tests**: bulk rewrite Line → Sentence in
   transcript goldens, update `tests/emit_matrix.rs` to drop the
   "line" row, refresh any anchor.golden files. Verify the strike
   help string change.

6. **Docs**: update the help screen wording (drop "Sentence-mode
   only" on `x`), and any README references to the five-step cycle.

Each commit compiles and passes tests in isolation. Step 4 is where
the user-visible behavior changes (cycle becomes 4 steps) — easy to
revert if we find a regression.

## Effort estimate

- Index + predicate: ~80 LOC + ~30 LOC of unit tests.
- Navigator + projection: ~30 LOC.
- App layer collapse: ~150 LOC removed, ~80 LOC added.
- Test updates: ~37 golden files (mechanical), ~10 unit tests
  rewritten.
- Total: roughly **~250 LOC of source change + golden churn**.

Risk is low because the byte ranges don't change — we're routing the
same data through one variant instead of two. The biggest source of
test breakage is just the unit-name string changing in goldens, which
is mechanical.
