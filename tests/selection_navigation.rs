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
