# Extend emit-matrix fixtures for fuller GFM coverage

## Why

Today the emit-matrix harness (`tests/emit_matrix.rs`) runs every
`(unit × action)` cell — 5 units × 5 actions = 25 cells per fixture —
against just 4 fixtures:

- `headings-h1-h2-h3`
- `list-task`
- `parser-footnote-ref-inline`
- `prose-soft-wrap`

That gives full Section/Paragraph/Line/Sentence/Word × change/feedback/
insert-before/insert-after/strike coverage for headings, task lists,
footnote-refs, and prose paragraphs.

Most other GFM element types only appear in `tests/fixtures/transcripts/`
(navigation replays). They get exercised in *one* mode each, typically
Sentence — leaving Section and Word almost entirely untested for those
elements.

The cheapest way to close the gap is to add more inputs to
`tests/fixtures/emit/`: each new `input.md` automatically multiplies into
25 cells via the existing harness, and goldens are seeded with
`UPDATE_GOLDENS=1`.

## Coverage targets

Add five emit fixtures to fill the highest-value holes. Each one's
input is small (≤ 6 source lines) so the resulting goldens stay
reviewable.

| Fixture name        | Element under test               | Source seed (markdown body)                                                                               |
| ------------------- | -------------------------------- | --------------------------------------------------------------------------------------------------------- |
| `parser-gfm-table`  | GFM table (rows + separator)     | reuse `tests/fixtures/transcripts/parser-gfm-table/input.md` — already exists, copy the file              |
| `blockquote`        | Blockquote w/ multi-line body    | reuse `tests/fixtures/transcripts/blockquote/input.md`                                                    |
| `code-fenced`       | Fenced code block                | reuse `tests/fixtures/transcripts/code-fenced/input.md`                                                   |
| `list-nested`       | Nested ordered/unordered list    | reuse `tests/fixtures/transcripts/list-nested/input.md`                                                   |
| `inline-formatting` | Bold / italic / inline-code / strike inside one paragraph | reuse `tests/fixtures/transcripts/inline-formatting/input.md`            |

All five inputs already exist as transcript fixtures; the work is
duplicating the `input.md` into the `tests/fixtures/emit/<name>/`
directory and seeding goldens. Reusing the same source markdown
between transcript and emit fixtures keeps the two harnesses
testing the same surface area from different angles.

### Why these five

- **Tables** — currently zero Section, zero Word, zero strike emit
  coverage. Tables are the most-likely GFM element to surface emit
  bugs because the parser collapses cells/rows into a single
  `DocNode::Paragraph` (see `combine_sentence_line_modes_plan.md`
  §Tables).
- **Blockquote** — only tested in Sentence mode today.
- **Fenced code** — currently has Line + Sentence transcript coverage
  only; the emit shape for code anchors is untested.
- **Nested list** — verifies that Section/Paragraph anchors traverse
  nested children correctly, and that Word emit walks across nesting.
- **Inline formatting** — exercises the segmenter against `**`, `*`,
  `` ` ``, `~~` markers; today's transcript fixture only walks one
  Sentence.

Stretch fixtures (defer unless quick): `images`, `parser-html-block`,
`parser-yaml-frontmatter`. These have niche emit shapes; add them in a
follow-up if the first five surface no surprises.

## Mechanics

For each fixture in the table above:

1. Create directory `tests/fixtures/emit/<name>/`.
2. Copy the corresponding `input.md` from
   `tests/fixtures/transcripts/<name>/input.md` into it.
3. Seed goldens:
   ```
   UPDATE_GOLDENS=1 cargo test --test emit_matrix
   ```
   This walks the `(unit × action)` cartesian product and writes
   `<unit>/<action>.golden.txt` for every cell that produces output.
   Cells that have no anchor for that unit (e.g. Word on a fixture
   with no words) are skipped silently — the harness is presence-
   driven (`tests/emit_matrix.rs:166-217`).
4. **Review every generated golden by hand** before committing.
   `UPDATE_GOLDENS=1` will happily bake whatever the current code
   emits, including bugs. Look specifically at:
   - The `target:` text — does it match the human reading of that
     unit? (E.g. for `parser-gfm-table`, does Section-mode `target:`
     contain the joined cells in the expected order?)
   - `WHERE: line N` — does N point at the source line a user would
     expect for that anchor?
   - Strike: `ACTION: delete this` should target the same span the
     visible highlight covers.
5. Re-run without `UPDATE_GOLDENS` to confirm the goldens are stable:
   ```
   cargo test --test emit_matrix
   ```
6. Commit `input.md` + the entire generated `<unit>/` tree as one
   commit per fixture so a future bisect can point at the exact
   element type that regressed.

## Interaction with the pending Sentence/Line merge

`combine_sentence_line_modes_plan.md` collapses Sentence + Line into a
single auto-flipping `Sentence` mode and drops `SelectionUnit::Line`.
After that change:

- The emit-matrix harness loses its `line` row
  (`tests/emit_matrix.rs:55-60`) — every `<fixture>/line/` directory
  becomes dead.
- Sentence-flavored goldens for Lines-flavored nodes (code blocks,
  tables, single-line headings) will be the same byte ranges as today's
  Line goldens, just under the `sentence` directory.

**Recommended ordering:** land the merge first, *then* add these
fixtures. Otherwise we author 5 new `line/` golden trees that get
deleted in the very next change. If the merge is more than a sprint
away, author the fixtures now and accept the churn — the goldens are
mechanical to regenerate (`UPDATE_GOLDENS=1`).

## Acceptance

- `tests/fixtures/emit/` contains 9 fixture directories (4 existing +
  5 new).
- `cargo test --test emit_matrix` passes with no `UPDATE_GOLDENS` flag.
- Each new fixture has at least one cell present in every unit
  directory (`section/`, `paragraph/`, `line/`, `sentence/`, `word/`).
  If a unit has no anchor for a fixture (e.g. Word on an empty code
  block), document the absence in a one-line comment in the fixture's
  `input.md` so a future reader understands the gap is intentional.
- `git diff --stat` for the change shows only `input.md` files and
  `*.golden.txt` files — no source changes.

## Out of scope

- Hardening the segmenter for new edge cases the fixtures expose. If
  a generated golden looks wrong, file a separate issue and either
  don't commit the fixture, or commit it with a `.actual.txt`
  reference and a follow-up tracked.
- Setext headings, autolinks, hard line breaks — none have transcript
  fixtures today, so seeding emit fixtures for them is a separate
  authoring task (write the input from scratch rather than copy).
- Per-cell or per-row table selection. Tables remain a single
  `DocNode::Paragraph`; cell-granular anchors are out of scope.
