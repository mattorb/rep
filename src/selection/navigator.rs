//! Pure navigation: `next`, `prev`, `clamp`. Operates against an immutable
//! `SelectionIndex` and a `SelectionAnchor`. Returns `NavOutcome::Boundary`
//! when there is no further anchor of the requested unit; selection state is
//! unchanged on a boundary outcome.
//!
//! Parity-preserving phase 3a: Sentence and Section units use the linear-order
//! tables from the index. Paragraph traversal walks block-level nodes with
//! paragraph anchors. Line and Word are no-ops in phase 3a (Line lands in
//! phase 4, Word in phase 5).

use crate::selection::index::SelectionIndex;
use crate::selection::model::{NavOutcome, SelectionAnchor, SelectionUnit};

pub fn next(index: &SelectionIndex, anchor: SelectionAnchor) -> NavOutcome {
    step(index, anchor, true)
}

pub fn prev(index: &SelectionIndex, anchor: SelectionAnchor) -> NavOutcome {
    step(index, anchor, false)
}

fn step(index: &SelectionIndex, anchor: SelectionAnchor, forward: bool) -> NavOutcome {
    let table = unit_table(index, anchor.unit);
    if table.is_empty() {
        return NavOutcome::Boundary;
    }
    let current = locate(&table, anchor.node_idx, anchor.unit_idx);
    let next_pos = match (current, forward) {
        (Some(p), true) if p + 1 < table.len() => p + 1,
        (Some(_), true) => return NavOutcome::Boundary,
        (Some(p), false) if p > 0 => p - 1,
        (Some(_), false) => return NavOutcome::Boundary,
        // Anchor is not in the table (e.g. node has zero anchors of this unit)
        // — fall back to the closest table entry in the requested direction.
        (None, true) => match table.iter().position(|&(n, u)| {
            n > anchor.node_idx || (n == anchor.node_idx && u > anchor.unit_idx)
        }) {
            Some(p) => p,
            None => return NavOutcome::Boundary,
        },
        (None, false) => {
            let mut found: Option<usize> = None;
            for (i, &(n, u)) in table.iter().enumerate() {
                if n < anchor.node_idx || (n == anchor.node_idx && u < anchor.unit_idx) {
                    found = Some(i);
                } else {
                    break;
                }
            }
            match found {
                Some(p) => p,
                None => return NavOutcome::Boundary,
            }
        }
    };
    let (n, u) = table[next_pos];
    NavOutcome::Moved(SelectionAnchor::new(n, anchor.unit, u))
}

fn unit_table(index: &SelectionIndex, unit: SelectionUnit) -> Vec<(usize, usize)> {
    match unit {
        SelectionUnit::Sentence => index.sentences.clone(),
        SelectionUnit::Paragraph => index.paragraphs.clone(),
        SelectionUnit::Line => index.lines.clone(),
        SelectionUnit::Word => index.words.clone(),
        SelectionUnit::Section => index
            .sections
            .iter()
            .map(|s| (s.start_node_idx, 0))
            .collect(),
    }
}

fn locate(table: &[(usize, usize)], node_idx: usize, unit_idx: usize) -> Option<usize> {
    table
        .iter()
        .position(|&(n, u)| n == node_idx && u == unit_idx)
}

/// Re-anchor onto the requested target unit per the pinned `clamp` rules.
/// Phase 3a uses the linear tables for upgrade/downgrade; out-of-current-node
/// targets walk backward then forward in document order.
pub fn clamp(
    index: &SelectionIndex,
    anchor: SelectionAnchor,
    target: SelectionUnit,
) -> SelectionAnchor {
    if anchor.unit == target {
        return anchor;
    }
    // Containing-unit upgrade and first-child downgrade are both expressible
    // as: look up the target unit's anchor that is within the current node;
    // if absent in this node, walk backward first, then forward, per pinned
    // rules. For containing-unit cases (Word -> Sentence -> Paragraph ->
    // Section) the anchor at unit_idx 0 of the same node trivially satisfies
    // "containing" semantics with the node-scoped table layouts.
    let table = unit_table(index, target);
    if table.is_empty() {
        return anchor;
    }
    if let Some(&(n, u)) = table.iter().find(|&&(n, _)| n == anchor.node_idx) {
        return SelectionAnchor::new(n, target, u);
    }
    // Walk backward in document order for a node with the target unit, land
    // on its last anchor in that unit.
    if let Some(&(n, u)) = table
        .iter()
        .rev()
        .find(|&&(n, _)| n < anchor.node_idx)
        .and_then(|prev| {
            // Find the LAST (n, _) entry for this node, i.e. its highest unit_idx.
            let target_node = prev.0;
            table
                .iter()
                .filter(|&&(nn, _)| nn == target_node)
                .last()
        })
    {
        return SelectionAnchor::new(n, target, u);
    }
    // Walk forward; land on the first anchor of the first node with the unit.
    if let Some(&(n, u)) = table.iter().find(|&&(nn, _)| nn > anchor.node_idx) {
        return SelectionAnchor::new(n, target, u);
    }
    anchor
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::selection::index::SelectionIndex;

    fn build(src: &str) -> (Document, SelectionIndex, Vec<String>) {
        let lines: Vec<String> = src.lines().map(ToOwned::to_owned).collect();
        let doc = Document::parse(src);
        let idx = SelectionIndex::build(&doc, &lines);
        (doc, idx, lines)
    }

    #[test]
    fn sentence_next_within_node_advances() {
        let (_doc, idx, _lines) = build("First sentence. Second sentence. Third.");
        let a = SelectionAnchor::new(0, SelectionUnit::Sentence, 0);
        match next(&idx, a) {
            NavOutcome::Moved(a2) => {
                assert_eq!(a2.unit_idx, 1);
                assert_eq!(a2.node_idx, 0);
            }
            o => panic!("unexpected: {:?}", o),
        }
    }

    #[test]
    fn sentence_next_at_doc_end_returns_boundary() {
        let (_doc, idx, _lines) = build("Only sentence.");
        let a = SelectionAnchor::new(0, SelectionUnit::Sentence, 0);
        assert_eq!(next(&idx, a), NavOutcome::Boundary);
        assert_eq!(prev(&idx, a), NavOutcome::Boundary);
    }

    #[test]
    fn sentence_next_crosses_node_boundary() {
        let (_doc, idx, _lines) = build("First. Second.\n\nThird. Fourth.");
        let last_of_first_node = SelectionAnchor::new(0, SelectionUnit::Sentence, 1);
        match next(&idx, last_of_first_node) {
            NavOutcome::Moved(a) => {
                assert_eq!(a.node_idx, 1);
                assert_eq!(a.unit_idx, 0);
            }
            o => panic!("unexpected: {:?}", o),
        }
    }

    #[test]
    fn roundtrip_invariant() {
        let (_doc, idx, _lines) = build("A.\n\nB.\n\nC.");
        let mut a = SelectionAnchor::new(0, SelectionUnit::Sentence, 0);
        for _ in 0..3 {
            if let NavOutcome::Moved(b) = next(&idx, a) {
                if let NavOutcome::Moved(c) = prev(&idx, b) {
                    assert_eq!(a, c);
                    a = b;
                }
            }
        }
    }

    #[test]
    fn section_nav_walks_headings() {
        let (_doc, idx, _lines) = build("# A\n\nx\n\n# B\n\ny.");
        // Sections at node 0 and node 2.
        let a = SelectionAnchor::new(0, SelectionUnit::Section, 0);
        let n = next(&idx, a);
        match n {
            NavOutcome::Moved(b) => assert_eq!(b.node_idx, 2),
            o => panic!("unexpected: {:?}", o),
        }
    }

    #[test]
    fn section_nav_no_sections_is_boundary() {
        let (_doc, idx, _lines) = build("Plain prose.");
        let a = SelectionAnchor::new(0, SelectionUnit::Section, 0);
        assert_eq!(next(&idx, a), NavOutcome::Boundary);
    }

    #[test]
    fn clamp_to_unavailable_unit_is_noop() {
        let (_doc, idx, _lines) = build("Plain prose.");
        let a = SelectionAnchor::new(0, SelectionUnit::Sentence, 0);
        let b = clamp(&idx, a, SelectionUnit::Word);
        assert_eq!(a, b);
    }
}
