use super::*;
use crate::selection::build_test_index as build;

#[test]
fn full_doc_walk_visits_every_sentence_anchor() {
    let src = "First. Second.\n\nThird.\n\n# Heading\n\nLast sentence.";
    let idx = build(src);
    let mut anchor = SelectionAnchor::new(0, SelectionUnit::Sentence, 0);
    let mut visited = vec![(anchor.node_idx, anchor.unit_idx)];
    while let NavOutcome::Moved(a) = next(&idx, anchor) {
        visited.push((a.node_idx, a.unit_idx));
        anchor = a;
    }
    assert_eq!(visited, idx.sentences);
}

#[test]
fn roundtrip_invariant_holds_for_every_sentence_anchor() {
    let src = "Alpha. Beta.\n\n## Sub\n\nGamma.\n\n- Item.\n";
    let idx = build(src);
    for (n, u) in &idx.sentences {
        let a = SelectionAnchor::new(*n, SelectionUnit::Sentence, *u);
        if let NavOutcome::Moved(b) = next(&idx, a)
            && let NavOutcome::Moved(c) = prev(&idx, b)
        {
            assert_eq!(a, c, "prev(next({a:?})) must roundtrip");
        }
    }
}

#[test]
fn line_walk_visits_every_line_in_a_multiline_paragraph() {
    let src = "Line one\nline two\nline three\nline four";
    let idx = build(src);
    let mut anchor = SelectionAnchor::new(0, SelectionUnit::Line, 0);
    let mut count = 1;
    while let NavOutcome::Moved(a) = next(&idx, anchor) {
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
    for w in &words {
        assert!(
            w.chars().all(|c| c != ',' && c != ';' && c != '!'),
            "word {w:?} contains terminator punct"
        );
    }
}

fn word_walk(idx: &SelectionIndex) -> Vec<(usize, String)> {
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
    while let NavOutcome::Moved(a) = next(idx, anchor) {
        out.push((a.node_idx, word_text(a.node_idx, a.unit_idx)));
        anchor = a;
    }
    out
}

#[test]
fn word_walk_crosses_paragraph_boundaries() {
    let idx = build("alpha beta.\n\ngamma delta.");
    let walk = word_walk(&idx);
    let words: Vec<&str> = walk.iter().map(|(_, w)| w.as_str()).collect();
    assert_eq!(words, vec!["alpha", "beta", "gamma", "delta"]);
    let nodes: Vec<usize> = walk.iter().map(|(n, _)| *n).collect();
    assert!(nodes.contains(&0) && nodes.contains(&1));
}

#[test]
fn word_walk_visits_every_node_kind_in_document_order() {
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
    let idx = build("alpha beta.\n\ngamma delta epsilon.\n\nzeta.");
    for (n, u) in &idx.words {
        let a = SelectionAnchor::new(*n, SelectionUnit::Word, *u);
        if let NavOutcome::Moved(b) = next(&idx, a)
            && let NavOutcome::Moved(c) = prev(&idx, b)
        {
            assert_eq!(a, c, "prev(next({a:?})) must roundtrip");
        }
    }
}

#[test]
fn toml_frontmatter_is_a_codeblock_not_a_setext_heading() {
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
    let src = "Prose first.\n\n```rust\nfn alpha() {}\nfn beta() {}\n```\n\nProse last.";
    let idx = build(src);
    let walk = word_walk(&idx);
    let words: Vec<&str> = walk.iter().map(|(_, w)| w.as_str()).collect();
    let code_start = words.iter().position(|w| *w == "fn").expect("fn in walk");
    assert_eq!(words[code_start], "fn");
    assert_eq!(words[code_start + 1], "alpha");
    assert_eq!(words[code_start + 2], "fn");
    assert_eq!(words[code_start + 3], "beta");
    assert!(words[..code_start].contains(&"Prose"));
    assert!(words[code_start + 4..].contains(&"Prose"));
}

#[test]
fn word_walk_through_inline_formatting() {
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
    let src = "> first quoted line\n> second quoted line\n> third quoted line";
    let idx = build(src);
    let line_count = idx.lines.len();
    assert!(
        line_count >= 3,
        "expected at least 3 line anchors for a 3-line blockquote, got {line_count}"
    );
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
    let src = "# A\n\nbody A\n\n## sub of A\n\nsub body\n\n# B\n\nbody B";
    let idx = build(src);
    let mut anchor = SelectionAnchor::new(0, SelectionUnit::Section, 0);
    let mut walk = vec![anchor.node_idx];
    while let NavOutcome::Moved(a) = next(&idx, anchor) {
        walk.push(a.node_idx);
        anchor = a;
    }
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
    let plain0 = &idx.nodes[0].selection_plain_text;
    assert!(
        plain0.contains("title") && plain0.contains("Example post"),
        "frontmatter content missing from node 0: {plain0:?}"
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
fn word_walk_through_frontmatter_then_into_body() {
    let src = "---\n\
title: Hello world\n\
draft: true\n\
---\n\
\n\
First body word here.\n";
    let idx = build(src);
    let walk = word_walk(&idx);
    let words: Vec<&str> = walk.iter().map(|(_, w)| w.as_str()).collect();
    assert_eq!(
        words,
        vec![
            "title", "Hello", "world", "draft", "true", "First", "body", "word", "here"
        ],
        "{walk:?}"
    );
    assert_eq!(walk[0].0, 0);
    assert_eq!(walk[5].0, 1, "body words should start on a different node");
}

#[test]
fn boundary_at_last_sentence_returns_boundary() {
    let idx = build("Single sentence.");
    let only = SelectionAnchor::new(0, SelectionUnit::Sentence, 0);
    assert_eq!(next(&idx, only), NavOutcome::Boundary);
    assert_eq!(prev(&idx, only), NavOutcome::Boundary);
}

#[test]
fn boundary_within_multi_sentence_node_returns_boundary_only_at_doc_end() {
    let idx = build("One. Two. Three.");
    let last = SelectionAnchor::new(0, SelectionUnit::Sentence, 2);
    assert_eq!(next(&idx, last), NavOutcome::Boundary);
    let middle = SelectionAnchor::new(0, SelectionUnit::Sentence, 1);
    let moved = next(&idx, middle);
    assert!(matches!(moved, NavOutcome::Moved(a) if a.unit_idx == 2));
}

#[test]
fn prev_from_first_of_node_lands_on_last_of_previous_node() {
    let idx = build("First. Second.\n\nThird.\n");
    let first_of_node1 = SelectionAnchor::new(1, SelectionUnit::Sentence, 0);
    match prev(&idx, first_of_node1) {
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
        let a = clamp(&idx, start, to);
        assert_eq!(a.unit, to, "clamp ought to land on {to:?}");
        let b = clamp(&idx, a, SelectionUnit::Sentence);
        assert_eq!(b.unit, SelectionUnit::Sentence);
    }
}
