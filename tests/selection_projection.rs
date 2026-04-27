//! Integration tests for `rep::selection::projection::highlight_for`.
//! Exercises the public API the way the renderer would.

use rep::document::Document;
use rep::selection::index::SelectionIndex;
use rep::selection::model::{SelectionAnchor, SelectionUnit};
use rep::selection::projection::{Highlight, highlight_for};

fn build(src: &str) -> SelectionIndex {
    let lines: Vec<String> = src.lines().map(ToOwned::to_owned).collect();
    let doc = Document::parse(src);
    SelectionIndex::build(&doc, &lines)
}

#[test]
fn word_unit_returns_byte_range_inside_node_plain() {
    let idx = build("alpha beta gamma");
    let a = SelectionAnchor::new(0, SelectionUnit::Word, 1);
    match highlight_for(a, &idx) {
        Highlight::Range(node, r) => {
            assert_eq!(node, 0);
            let word = &idx.nodes[0].selection_plain_text[r];
            assert_eq!(word, "beta");
        }
        h => panic!("expected Range, got {h:?}"),
    }
}

#[test]
fn line_unit_paints_per_source_line_range() {
    let idx = build("line one\nline two\nline three");
    let a = SelectionAnchor::new(0, SelectionUnit::Line, 1);
    match highlight_for(a, &idx) {
        Highlight::Range(node, _r) => assert_eq!(node, 0),
        h => panic!("expected Range, got {h:?}"),
    }
}

#[test]
fn paragraph_unit_paints_full_node_range() {
    let idx = build("Some prose here. More prose.");
    let a = SelectionAnchor::new(0, SelectionUnit::Paragraph, 0);
    match highlight_for(a, &idx) {
        Highlight::Range(node, r) => {
            assert_eq!(node, 0);
            assert_eq!(r.start, 0);
            assert_eq!(r.end, idx.nodes[0].selection_plain_text.len());
        }
        h => panic!("expected Range, got {h:?}"),
    }
}

#[test]
fn section_unit_paints_constituent_node_list() {
    let idx = build("# A\n\nbody.\n\n# B\n\nmore body.");
    let a = SelectionAnchor::new(0, SelectionUnit::Section, 0);
    match highlight_for(a, &idx) {
        Highlight::Section(nodes) => {
            assert!(nodes.contains(&0));
            // Section A spans heading + body — at least 2 nodes.
            assert!(nodes.len() >= 2);
        }
        h => panic!("expected Section, got {h:?}"),
    }
}
