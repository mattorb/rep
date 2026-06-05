use super::*;
use crate::selection::build_test_index as build;

#[test]
fn line_unit_paints_per_source_line_range() {
    let idx = build("line one\nline two\nline three");
    let a = SelectionAnchor::new(0, SelectionUnit::Line, 1);
    match highlight_for(a, &idx) {
        Highlight::Range(node, r) => {
            assert_eq!(node, 0);
            assert_eq!(&idx.nodes[0].selection_plain_text[r], "line two");
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
            assert!(nodes.len() >= 2);
        }
        h => panic!("expected Section, got {h:?}"),
    }
}
