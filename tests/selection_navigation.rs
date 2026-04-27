//! Integration tests for the selection navigator + index, exercising the
//! public API (`rep::selection::*`) end-to-end on parsed real-shape
//! Markdown documents.

mod common;

use common::build_index as build;
use rep::selection::model::{NavOutcome, SelectionAnchor, SelectionUnit};
use rep::selection::navigator;

#[test]
fn full_doc_walk_visits_every_sentence_anchor() {
    let src = "First. Second.\n\nThird.\n\n# Heading\n\nLast sentence.";
    let idx = build(src);
    let mut anchor = SelectionAnchor::new(0, SelectionUnit::Sentence, 0);
    let mut visited = vec![(anchor.node_idx, anchor.unit_idx)];
    while let NavOutcome::Moved(a) = navigator::next(&idx, anchor) {
        visited.push((a.node_idx, a.unit_idx));
        anchor = a;
    }
    // sentences linear order: same as visited.
    assert_eq!(visited, idx.sentences);
}

#[test]
fn roundtrip_invariant_holds_for_every_sentence_anchor() {
    let src = "Alpha. Beta.\n\n## Sub\n\nGamma.\n\n- Item.\n";
    let idx = build(src);
    for (n, u) in &idx.sentences {
        let a = SelectionAnchor::new(*n, SelectionUnit::Sentence, *u);
        if let NavOutcome::Moved(b) = navigator::next(&idx, a) {
            if let NavOutcome::Moved(c) = navigator::prev(&idx, b) {
                assert_eq!(a, c, "prev(next({a:?})) must roundtrip");
            }
        }
    }
}

#[test]
fn line_walk_visits_every_line_in_a_multiline_paragraph() {
    let src = "Line one\nline two\nline three\nline four";
    let idx = build(src);
    let mut anchor = SelectionAnchor::new(0, SelectionUnit::Line, 0);
    let mut count = 1;
    while let NavOutcome::Moved(a) = navigator::next(&idx, anchor) {
        anchor = a;
        count += 1;
    }
    assert!(count >= 1, "should visit at least one line");
    assert_eq!(count, idx.lines.len());
}

#[test]
fn word_walk_skips_punctuation_between_words() {
    let src = "First, second; third!";
    let idx = build(src);
    let words: Vec<String> =
        idx.sentences
            .iter()
            .map(|(n, _)| n)
            .fold(Vec::new(), |mut acc, &n| {
                for r in &idx.nodes[n].word_ranges {
                    acc.push(idx.nodes[n].selection_plain_text[r.clone()].to_string());
                }
                acc
            });
    // No punctuation should appear inside the word ranges.
    for w in &words {
        assert!(
            w.chars().all(|c| c != ',' && c != ';' && c != '!'),
            "word {w:?} contains terminator punct"
        );
    }
}

/// Walk every word anchor forward via navigator::next from the first
/// anchor; collect the (node_idx, word_text) sequence and compare to
/// expected. Helps assert linear-order word coverage on real docs.
fn word_walk(idx: &rep::selection::index::SelectionIndex) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let Some(&(start_n, start_u)) = idx.words.first() else {
        return out;
    };
    let mut anchor = SelectionAnchor::new(start_n, SelectionUnit::Word, start_u);
    let word_text = |n: usize, u: usize| {
        let r = idx.nodes[n].word_ranges[u].clone();
        idx.nodes[n].selection_plain_text[r].to_string()
    };
    out.push((start_n, word_text(start_n, start_u)));
    while let NavOutcome::Moved(a) = navigator::next(idx, anchor) {
        out.push((a.node_idx, word_text(a.node_idx, a.unit_idx)));
        anchor = a;
    }
    out
}

#[test]
fn word_walk_crosses_paragraph_boundaries() {
    // Two paragraphs; word j should bridge the node boundary cleanly,
    // visiting every word in document order with no skips and no
    // duplicates. Checks the navigator's cross-node fall-through for
    // the Word unit specifically.
    let idx = build("alpha beta.\n\ngamma delta.");
    let walk = word_walk(&idx);
    let words: Vec<&str> = walk.iter().map(|(_, w)| w.as_str()).collect();
    assert_eq!(words, vec!["alpha", "beta", "gamma", "delta"]);
    // Boundary check: nodes are 0 and 1; ensure both contributed.
    let nodes: Vec<usize> = walk.iter().map(|(n, _)| *n).collect();
    assert!(nodes.contains(&0) && nodes.contains(&1));
}

#[test]
fn word_walk_visits_every_node_kind_in_document_order() {
    // Mixed-shape doc: heading + soft-wrapped paragraph + list item +
    // fenced code block. Word j walks every word across all four
    // content kinds in source order, in one continuous sequence.
    let src = "\
# Title head\n\
\n\
prose alpha\n\
prose beta.\n\
\n\
- list item words\n\
\n\
```rust\n\
fn code() {}\n\
```\n";
    let idx = build(src);
    let walk = word_walk(&idx);
    let words: Vec<&str> = walk.iter().map(|(_, w)| w.as_str()).collect();
    // Expected in source order:
    //   heading: "Title", "head"
    //   paragraph: "prose", "alpha", "prose", "beta"
    //   list item: "list", "item", "words"
    //   code block: "fn", "code"
    assert_eq!(
        words,
        vec![
            "Title", "head", "prose", "alpha", "prose", "beta", "list", "item", "words", "fn",
            "code"
        ],
        "{walk:?}"
    );
}

#[test]
fn word_walk_round_trip_holds_for_every_word() {
    // For every word anchor, prev(next(x)) == x. Covers cross-node
    // transitions (last word of node N → first word of node N+1, then
    // back to last of N) — the trickiest case for the cross-node
    // fallback in navigator::step.
    let idx = build("alpha beta.\n\ngamma delta epsilon.\n\nzeta.");
    for (n, u) in &idx.words {
        let a = SelectionAnchor::new(*n, SelectionUnit::Word, *u);
        if let NavOutcome::Moved(b) = navigator::next(&idx, a)
            && let NavOutcome::Moved(c) = navigator::prev(&idx, b)
        {
            assert_eq!(a, c, "prev(next({a:?})) must roundtrip");
        }
    }
}

#[test]
fn toml_frontmatter_is_a_codeblock_not_a_setext_heading() {
    // Sibling to the YAML frontmatter case: TOML frontmatter delimited
    // by `+++` produces an mdast Toml node, which we fold into a
    // CodeBlock with `language="toml"`. Less common in the wild but
    // used by Hugo, Zola, Jekyll alternatives.
    let src = "+++\n\
title = \"Example post\"\n\
draft = true\n\
+++\n\
\n\
# First heading\n\
\n\
Body paragraph here.\n";
    let idx = build(src);
    let plain0 = &idx.nodes[0].selection_plain_text;
    assert!(
        plain0.contains("title") && plain0.contains("Example post"),
        "TOML frontmatter content missing from node 0: {plain0:?}"
    );
    let kinds_have_real_heading = idx
        .nodes
        .iter()
        .skip(1)
        .any(|n| n.selection_plain_text.contains("First heading"));
    assert!(
        kinds_have_real_heading,
        "expected the actual `# First heading` to land on a later node"
    );
}

#[test]
fn word_walk_visits_code_block_content_lines() {
    // Per modular_plan §"Block-type coverage": code blocks are excluded
    // from sentence-level navigation but allowed at word level. Verify
    // each non-fence content line contributes word anchors and they're
    // walked in order.
    let src = "Prose first.\n\n```rust\nfn alpha() {}\nfn beta() {}\n```\n\nProse last.";
    let idx = build(src);
    let walk = word_walk(&idx);
    let words: Vec<&str> = walk.iter().map(|(_, w)| w.as_str()).collect();
    // Expected: "Prose", "first" (paragraph 1), "fn", "alpha", "fn", "beta" (code), "Prose", "last" (paragraph 2).
    // Code-block words appear between the two prose paragraphs.
    let code_start = words.iter().position(|w| *w == "fn").expect("fn in walk");
    assert_eq!(words[code_start], "fn");
    assert_eq!(words[code_start + 1], "alpha");
    assert_eq!(words[code_start + 2], "fn");
    assert_eq!(words[code_start + 3], "beta");
    // Prose words must appear on both sides of the code block.
    assert!(words[..code_start].contains(&"Prose"));
    assert!(words[code_start + 4..].contains(&"Prose"));
}

#[test]
fn word_walk_through_inline_formatting() {
    // Bold / italic / inline code don't introduce word boundaries
    // beyond what segment_words sees in selection plain text. The
    // markdown markers (`**`, `*`, backticks) are stripped during
    // index construction, so word boundaries fall on whitespace and
    // punctuation only.
    let src = "alpha **bold word** italic *more* and `code` end.";
    let idx = build(src);
    let walk = word_walk(&idx);
    let words: Vec<&str> = walk.iter().map(|(_, w)| w.as_str()).collect();
    assert_eq!(
        words,
        vec![
            "alpha", "bold", "word", "italic", "more", "and", "code", "end"
        ],
        "inline formatting markers must not corrupt word boundaries: {walk:?}"
    );
}

#[test]
fn line_walk_through_blockquote() {
    // Blockquote contents flatten into Paragraph anchors per the
    // modular_plan flatten rule. Each source line of a multi-line
    // blockquote should still get its own line anchor — so line-mode
    // j walks blockquote lines individually.
    let src = "> first quoted line\n> second quoted line\n> third quoted line";
    let idx = build(src);
    // Blockquote produces one paragraph node; line anchors per source
    // line.
    let line_count = idx.lines.len();
    assert!(
        line_count >= 3,
        "expected at least 3 line anchors for a 3-line blockquote, got {line_count}"
    );
    // Each anchor's source line should be sequential 0, 1, 2.
    let source_lines: Vec<usize> = idx
        .lines
        .iter()
        .map(|&(n, u)| idx.nodes[n].source_line_ranges[u].0)
        .collect();
    for w in source_lines.windows(2) {
        assert!(
            w[0] < w[1],
            "blockquote line anchors must be in source order: {source_lines:?}"
        );
    }
}

#[test]
fn section_walk_through_nested_headings() {
    // # A / ## sub / # B — section nav (next/prev on Section unit)
    // visits all three section starters in source order. Even though
    // ## sub is subordinate and # A's section span includes it, sub
    // is itself addressable as a section.
    let src = "# A\n\nbody A\n\n## sub of A\n\nsub body\n\n# B\n\nbody B";
    let idx = build(src);
    let mut anchor = SelectionAnchor::new(0, SelectionUnit::Section, 0);
    let mut walk = vec![anchor.node_idx];
    while let NavOutcome::Moved(a) = navigator::next(&idx, anchor) {
        walk.push(a.node_idx);
        anchor = a;
    }
    // Three section starters; their start_node_idx values should be
    // strictly increasing.
    assert_eq!(walk.len(), 3, "expected 3 sections walked, got {walk:?}");
    for w in walk.windows(2) {
        assert!(
            w[0] < w[1],
            "section walk must be in source order: {walk:?}"
        );
    }
}

#[test]
fn yaml_frontmatter_is_a_codeblock_not_a_setext_heading() {
    // Regression: real-world Astro / Hugo posts open with a YAML
    // frontmatter block delimited by `---`. Without explicit
    // frontmatter handling, the markdown crate parses the closing
    // `---` as a setext H2 underline for the YAML body, turning the
    // entire frontmatter into one giant Heading. That made
    // word-mode highlight invisible: the index's word ranges
    // referenced bytes deep inside the heading's selection plain
    // text, but the renderer only displayed the heading's first
    // source line, so the selection→display mapping returned None
    // for most words and nothing got painted.
    //
    // With frontmatter enabled the YAML is a Yaml node we fold into
    // a CodeBlock with `language="yaml"`. Selection / word-nav still
    // works through the frontmatter tokens (CodeBlock contributes
    // word anchors per modular_plan §"Block-type coverage"), and the
    // rest of the document parses as expected.
    let src = "---\n\
title: Example post\n\
date: 2026-04-27\n\
draft: true\n\
---\n\
\n\
# First heading\n\
\n\
Body paragraph here.\n";
    let idx = build(src);
    // Node 0 must be the frontmatter block — emitted as CodeBlock
    // (not Heading) so word-mode highlight can map ranges correctly.
    let plain0 = &idx.nodes[0].selection_plain_text;
    assert!(
        plain0.contains("title") && plain0.contains("Example post"),
        "frontmatter content missing from node 0: {plain0:?}"
    );
    // Subsequent nodes: the actual heading + paragraph.
    let kinds_have_real_heading = idx
        .nodes
        .iter()
        .skip(1)
        .any(|n| n.selection_plain_text.contains("First heading"));
    assert!(
        kinds_have_real_heading,
        "expected the actual `# First heading` to land on a later node"
    );
}

#[test]
fn word_walk_through_frontmatter_then_into_body() {
    // End-to-end repro of the macbook-neo file's word-mode behavior:
    // walk word anchors from the start of the doc and verify they
    // pass through the YAML frontmatter and continue into the body.
    let src = "---\n\
title: Hello world\n\
draft: true\n\
---\n\
\n\
First body word here.\n";
    let idx = build(src);
    let walk = word_walk(&idx);
    let words: Vec<&str> = walk.iter().map(|(_, w)| w.as_str()).collect();
    // Exact sequence: frontmatter words first, then body words.
    assert_eq!(
        words,
        vec![
            "title", "Hello", "world", "draft", "true", "First", "body", "word", "here"
        ],
        "{walk:?}"
    );
    // Frontmatter words live on node 0 (CodeBlock), body on node 1.
    assert_eq!(walk[0].0, 0);
    assert_eq!(walk[5].0, 1, "body words should start on a different node");
}

#[test]
fn boundary_at_last_sentence_returns_boundary() {
    // Single-sentence document: next on the only anchor is Boundary,
    // not Moved. Mirrors the app-level "stays put" coverage that used
    // to live in src/app.rs but moves the assertion to the navigator
    // API where it belongs.
    let idx = build("Single sentence.");
    let only = SelectionAnchor::new(0, SelectionUnit::Sentence, 0);
    assert_eq!(navigator::next(&idx, only), NavOutcome::Boundary);
    assert_eq!(navigator::prev(&idx, only), NavOutcome::Boundary);
}

#[test]
fn boundary_within_multi_sentence_node_returns_boundary_only_at_doc_end() {
    // `One. Two. Three.` — cursor on sentence 2 is the doc's last
    // anchor; next returns Boundary. Cursor on sentence 1 advances
    // forward to sentence 2 normally.
    let idx = build("One. Two. Three.");
    let last = SelectionAnchor::new(0, SelectionUnit::Sentence, 2);
    assert_eq!(navigator::next(&idx, last), NavOutcome::Boundary);
    let middle = SelectionAnchor::new(0, SelectionUnit::Sentence, 1);
    let moved = navigator::next(&idx, middle);
    assert!(matches!(moved, NavOutcome::Moved(a) if a.unit_idx == 2));
}

#[test]
fn prev_from_first_of_node_lands_on_last_of_previous_node() {
    // `First. Second.\n\nThird.\n` — node 0 has 2 sentences, node 1
    // has 1. prev from (1, 0) lands on (0, 1) — the last sentence of
    // the previous node — not on (0, 0).
    let idx = build("First. Second.\n\nThird.\n");
    let first_of_node1 = SelectionAnchor::new(1, SelectionUnit::Sentence, 0);
    match navigator::prev(&idx, first_of_node1) {
        NavOutcome::Moved(a) => {
            assert_eq!(a.node_idx, 0);
            assert_eq!(a.unit_idx, 1, "must land on last sentence of node 0");
        }
        o => panic!("unexpected: {o:?}"),
    }
}

#[test]
fn clamp_round_trips_through_every_unit() {
    let src = "# Heading\n\nFirst. Second.\n\nWrapped\nparagraph here.";
    let idx = build(src);
    let units = [
        SelectionUnit::Section,
        SelectionUnit::Paragraph,
        SelectionUnit::Line,
        SelectionUnit::Sentence,
        SelectionUnit::Word,
    ];
    let start = SelectionAnchor::new(1, SelectionUnit::Sentence, 0);
    for &to in &units {
        let a = navigator::clamp(&idx, start, to);
        assert_eq!(a.unit, to, "clamp ought to land on {to:?}");
        // Round-trip back to Sentence.
        let b = navigator::clamp(&idx, a, SelectionUnit::Sentence);
        assert_eq!(b.unit, SelectionUnit::Sentence);
    }
}
