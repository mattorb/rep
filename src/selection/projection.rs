//! Anchor → highlight projection. Given a `SelectionAnchor` and the
//! `SelectionIndex`, return what the render layer should paint:
//! `Highlight::Range(node, byte_range)` for Word / Sentence / Line /
//! Paragraph and `Highlight::Section(Vec<node_idx>)` for Section.
//!
//! The byte ranges live in selection plain text — the renderer's display
//! plain text may differ for nodes with markers (footnote refs, task
//! markers). Today the app's `unit_highlight_for` does its own display
//! plain text lookup; this module is the canonical "what should be
//! painted" answer for any future renderer that wants to consume it
//! directly.

use std::ops::Range;

use crate::selection::index::SelectionIndex;
use crate::selection::model::{SelectionAnchor, SelectionUnit};

/// What the render layer paints for a single anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Highlight {
    /// `(node_idx, byte_range_in_selection_plain_text)` — sub-node highlight.
    Range(usize, Range<usize>),
    /// Section span — list of node indices to paint as a whole-block highlight.
    Section(Vec<usize>),
}

/// Resolve an anchor to a highlight per Req 4 + Section 5 of `modular_plan.md`.
pub fn highlight_for(anchor: SelectionAnchor, index: &SelectionIndex) -> Highlight {
    match anchor.unit {
        SelectionUnit::Word => {
            let r = index
                .nodes
                .get(anchor.node_idx)
                .and_then(|n| n.word_ranges.get(anchor.unit_idx))
                .cloned()
                .unwrap_or(0..0);
            Highlight::Range(anchor.node_idx, r)
        }
        SelectionUnit::Sentence => {
            let r = index
                .nodes
                .get(anchor.node_idx)
                .and_then(|n| n.sentence_ranges.get(anchor.unit_idx))
                .cloned()
                .unwrap_or(0..0);
            Highlight::Range(anchor.node_idx, r)
        }
        SelectionUnit::Line => {
            let r = index
                .nodes
                .get(anchor.node_idx)
                .and_then(|n| n.source_line_ranges.get(anchor.unit_idx))
                .map(|(_, r)| r.clone())
                .unwrap_or(0..0);
            Highlight::Range(anchor.node_idx, r)
        }
        SelectionUnit::Paragraph => {
            let r = index
                .nodes
                .get(anchor.node_idx)
                .map(|n| 0..n.selection_plain_text.len())
                .unwrap_or(0..0);
            Highlight::Range(anchor.node_idx, r)
        }
        SelectionUnit::Section => {
            // Find the containing section by start_node_idx.
            let nodes: Vec<usize> = index
                .sections
                .iter()
                .find(|s| s.start_node_idx == anchor.node_idx)
                .map(|s| (s.start_node_idx..=s.end_node_idx).collect())
                .unwrap_or_else(Vec::new);
            Highlight::Section(nodes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::selection::model::SelectionUnit;

    fn build(src: &str) -> SelectionIndex {
        let lines: Vec<String> = src.lines().map(ToOwned::to_owned).collect();
        let doc = Document::parse(src);
        SelectionIndex::build(&doc, &lines)
    }

    #[test]
    fn sentence_highlight_returns_byte_range_in_node_plain() {
        let idx = build("First. Second.");
        let a = SelectionAnchor::new(0, SelectionUnit::Sentence, 0);
        match highlight_for(a, &idx) {
            Highlight::Range(n, r) => {
                assert_eq!(n, 0);
                assert!(!r.is_empty());
            }
            o => panic!("unexpected: {:?}", o),
        }
    }

    #[test]
    fn paragraph_highlight_covers_full_node_text() {
        let idx = build("Some prose paragraph.");
        let a = SelectionAnchor::new(0, SelectionUnit::Paragraph, 0);
        match highlight_for(a, &idx) {
            Highlight::Range(_n, r) => {
                assert_eq!(r.start, 0);
                assert!(r.end > 0);
            }
            o => panic!("unexpected: {:?}", o),
        }
    }

    #[test]
    fn word_highlight_returns_word_byte_range() {
        let idx = build("alpha beta gamma");
        let a = SelectionAnchor::new(0, SelectionUnit::Word, 1);
        match highlight_for(a, &idx) {
            Highlight::Range(n, r) => {
                assert_eq!(n, 0);
                let plain = &idx.nodes[0].selection_plain_text;
                assert_eq!(&plain[r], "beta");
            }
            o => panic!("unexpected: {:?}", o),
        }
    }

    #[test]
    fn line_highlight_for_heading_covers_full_plain() {
        // Heading nodes have a single source line; Line projection returns
        // the whole node range. Multi-line paragraphs return the same coarse
        // node range per source line (the renderer does its own per-line
        // slicing); pin the heading case here as the precise contract.
        let idx = build("# Title");
        let a = SelectionAnchor::new(0, SelectionUnit::Line, 0);
        match highlight_for(a, &idx) {
            Highlight::Range(n, r) => {
                assert_eq!(n, 0);
                let plain = &idx.nodes[0].selection_plain_text;
                assert_eq!(&plain[r], plain.as_str());
            }
            o => panic!("unexpected: {:?}", o),
        }
    }

    #[test]
    fn out_of_range_unit_idx_yields_empty_range() {
        // Anchor pointing past the last word still returns Range so callers
        // can paint a no-op highlight rather than panicking.
        let idx = build("one two");
        let a = SelectionAnchor::new(0, SelectionUnit::Word, 99);
        match highlight_for(a, &idx) {
            Highlight::Range(_, r) => assert!(r.is_empty()),
            o => panic!("unexpected: {:?}", o),
        }
    }

    #[test]
    fn section_highlight_returns_node_list() {
        let idx = build("# A\n\nbody.\n\n# B\n\nmore.");
        let a = SelectionAnchor::new(0, SelectionUnit::Section, 0);
        match highlight_for(a, &idx) {
            Highlight::Section(nodes) => {
                assert!(nodes.contains(&0));
            }
            o => panic!("unexpected: {:?}", o),
        }
    }
}
