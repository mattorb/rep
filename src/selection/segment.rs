//! Single canonical home for `plain_text_for_node` and the sentence/word
//! segmenters. Phase-1 stubs route through existing helpers; phase-2 lands the
//! full canonical implementation.

#![allow(unused)]

use std::ops::Range;

use crate::document::DocNode;

/// Selection plain text for a single node.
///
/// Phase-1 stub: defers to today's renderer-side text where possible. Phase-2
/// owns the full visibility-rule implementation (footnote refs, task markers,
/// `[image: ]` wrappers, code-block fences stripped).
pub fn plain_text_for_node(node: &DocNode, source_lines: &[String]) -> String {
    crate::selection::index::node_selection_plain_text(node, source_lines)
}

/// Sentence byte ranges within selection plain text.
///
/// Phase-1 stub re-exports today's `sentence_ranges_from_plain` from `app.rs`;
/// phase-2 makes this the canonical implementation and deletes the duplicate.
pub fn segment_sentences(plain: &str) -> Vec<Range<usize>> {
    crate::selection::index::segment_sentences_internal(plain)
}

/// Word byte ranges within selection plain text. Phase-5 lands this.
pub fn segment_words(_plain: &str) -> Vec<Range<usize>> {
    Vec::new()
}
