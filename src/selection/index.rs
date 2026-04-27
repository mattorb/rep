//! Selection index — the eager, owned navigation cache built from a parsed
//! `Document` at load time. Phase-1 ships per-node selection plain text,
//! sentence ranges, source-line ranges, paragraph/line/sentence linear-order
//! tables, and the section table. Word ranges come in phase 5.

#![allow(unused)]

use std::ops::Range;

use crate::document::{DocNode, Document};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    Heading,
    Ol,
    PreHeading,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub start_node_idx: usize,
    pub end_node_idx: usize,
    pub kind: SectionKind,
}

#[derive(Debug, Clone, Default)]
pub struct NodeIndex {
    /// Selection plain text — markers stripped.
    pub selection_plain_text: String,
    /// Pairs of `(source_line, range_in_selection_plain_text)`.
    pub source_line_ranges: Vec<(usize, Range<usize>)>,
    /// Sentence byte ranges within `selection_plain_text`.
    pub sentence_ranges: Vec<Range<usize>>,
    /// Word byte ranges within `selection_plain_text`. Empty until phase 5.
    pub word_ranges: Vec<Range<usize>>,
}

#[derive(Debug, Clone, Default)]
pub struct SelectionIndex {
    pub nodes: Vec<NodeIndex>,
    pub paragraphs: Vec<(usize, usize)>,
    pub lines: Vec<(usize, usize)>,
    pub sentences: Vec<(usize, usize)>,
    pub words: Vec<(usize, usize)>,
    pub sections: Vec<Section>,
}

impl SelectionIndex {
    /// Eager build at load time per Req 11.
    pub fn build(doc: &Document, source_lines: &[String]) -> Self {
        let mut nodes: Vec<NodeIndex> = Vec::with_capacity(doc.nodes.len());
        let mut paragraphs: Vec<(usize, usize)> = Vec::new();
        let mut lines: Vec<(usize, usize)> = Vec::new();
        let mut sentences: Vec<(usize, usize)> = Vec::new();

        for (node_idx, node) in doc.nodes.iter().enumerate() {
            let plain = node_selection_plain_text(node, source_lines);
            let source_line_ranges = node_source_line_ranges(node, source_lines, &plain);
            let sentence_ranges = if node_is_sentence_bearing(node) {
                segment_sentences_internal(&plain)
            } else {
                Vec::new()
            };

            // Linear-order tables.
            if node_contributes_paragraph_anchor(node, &plain) {
                paragraphs.push((node_idx, 0));
            }
            for li in 0..source_line_ranges.len() {
                lines.push((node_idx, li));
            }
            for si in 0..sentence_ranges.len() {
                sentences.push((node_idx, si));
            }

            nodes.push(NodeIndex {
                selection_plain_text: plain,
                source_line_ranges,
                sentence_ranges,
                word_ranges: Vec::new(),
            });
        }

        let sections = build_section_table(doc);

        debug_assert!(
            sections
                .iter()
                .all(|s| s.start_node_idx <= s.end_node_idx && s.end_node_idx < doc.nodes.len()),
            "section endpoints out of range"
        );

        Self {
            nodes,
            paragraphs,
            lines,
            sentences,
            words: Vec::new(),
            sections,
        }
    }
}

/// Compute selection plain text for a node, stripping markers per Req 11.
///
/// Phase-1 implementation routes through existing helpers where it can.
/// Phase-2 will own the canonical visibility-rule implementation and delete
/// duplicates elsewhere.
pub(crate) fn node_selection_plain_text(node: &DocNode, source_lines: &[String]) -> String {
    match node {
        DocNode::Heading { text, .. } => text.clone(),
        DocNode::Paragraph { text, .. } => text.clone(),
        DocNode::ListItem { source_lines: range, .. } => {
            // Reuse the join logic that `app.rs::join_node_source_lines` performs:
            // strip the leading bullet/number marker and task marker on the first
            // line, then space-join with subsequent lines. We re-implement here so
            // selection-layer code does not depend on app internals.
            let slice = source_lines
                .get(range.start..range.end.min(source_lines.len()))
                .unwrap_or(&[]);
            let joined = slice
                .iter()
                .map(|s| s.trim())
                .collect::<Vec<_>>()
                .join(" ");
            strip_list_marker(&joined)
        }
        DocNode::CodeBlock {
            source_lines: range,
            ..
        } => {
            let slice = source_lines
                .get(range.start..range.end.min(source_lines.len()))
                .unwrap_or(&[]);
            // Exclude fence lines.
            slice
                .iter()
                .filter(|l| !l.trim_start().starts_with("```"))
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        }
        DocNode::ThematicBreak { .. } => String::new(),
    }
}

fn strip_list_marker(text: &str) -> String {
    let trimmed = text.trim_start();
    let stripped = if let Some(rest) = strip_ordered_marker(trimmed) {
        rest
    } else if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        rest
    } else {
        trimmed
    };
    let stripped = stripped
        .strip_prefix("[ ] ")
        .or_else(|| stripped.strip_prefix("[x] "))
        .or_else(|| stripped.strip_prefix("[X] "))
        .unwrap_or(stripped);
    stripped.to_string()
}

fn strip_ordered_marker(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        return None;
    }
    if i < bytes.len() && (bytes[i] == b'.' || bytes[i] == b')') && i + 1 < bytes.len() && bytes[i + 1] == b' ' {
        Some(&s[i + 2..])
    } else {
        None
    }
}

fn node_source_line_ranges(
    node: &DocNode,
    source_lines: &[String],
    plain: &str,
) -> Vec<(usize, Range<usize>)> {
    match node {
        DocNode::Heading { source_line, .. } => vec![(*source_line, 0..plain.len())],
        DocNode::Paragraph { source_lines: range, .. } => {
            // Phase-1 simplification: one (line, full-range) entry per source line. The
            // exact per-line slice mapping into selection plain text isn't needed for
            // the operations phase 0 / phase 1 perform; line-unit selection ships in
            // phase 4 and refines this to byte-exact slices.
            range
                .clone()
                .filter(|l| *l < source_lines.len())
                .map(|l| (l, 0..plain.len()))
                .collect()
        }
        DocNode::ListItem {
            source_lines: range,
            ..
        } => {
            // ListItem has 1 line anchor regardless of source-line span.
            let start = range.start;
            vec![(start, 0..plain.len())]
        }
        DocNode::CodeBlock {
            source_lines: range,
            ..
        } => {
            // One anchor per non-fence source line. We emit `(line, full-range)` as a
            // phase-1 simplification; phase 4 narrows to per-line slices.
            range
                .clone()
                .filter(|&l| {
                    source_lines
                        .get(l)
                        .map(|s| !s.trim_start().starts_with("```"))
                        .unwrap_or(false)
                })
                .map(|l| (l, 0..plain.len()))
                .collect()
        }
        DocNode::ThematicBreak { .. } => Vec::new(),
    }
}

fn node_is_sentence_bearing(node: &DocNode) -> bool {
    matches!(node, DocNode::Heading { .. } | DocNode::Paragraph { .. } | DocNode::ListItem { .. })
}

fn node_contributes_paragraph_anchor(node: &DocNode, plain: &str) -> bool {
    match node {
        DocNode::ThematicBreak { .. } => false,
        _ => !plain.trim().is_empty(),
    }
}

fn build_section_table(doc: &Document) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let n = doc.nodes.len();
    if n == 0 {
        return sections;
    }

    let any_heading = doc.nodes.iter().any(|node| matches!(node, DocNode::Heading { .. }));

    // Find candidate section starters in document order.
    let mut starters: Vec<(usize, SectionKind)> = Vec::new();
    let mut seen_heading = false;
    for (i, node) in doc.nodes.iter().enumerate() {
        match node {
            DocNode::Heading { .. } => {
                starters.push((i, SectionKind::Heading));
                seen_heading = true;
            }
            DocNode::ListItem {
                ordered: true,
                depth: 0,
                ..
            } => {
                if !any_heading {
                    // Top-level OL counts as section start only when no heading precedes it.
                    if !seen_heading
                        && (sections.is_empty() // first OL becomes section starter
                            || !matches!(starters.last(), Some((_, SectionKind::Ol))))
                    {
                        starters.push((i, SectionKind::Ol));
                    }
                }
            }
            _ => {}
        }
    }

    // Pre-heading "section 0" — only when at least one pre-starter node has paragraph anchor.
    let first_starter = starters.first().map(|(i, _)| *i).unwrap_or(n);
    let pre_has_content = (0..first_starter).any(|i| match &doc.nodes[i] {
        DocNode::ThematicBreak { .. } => false,
        DocNode::Heading { text, .. } => !text.is_empty(),
        DocNode::Paragraph { text, .. } => !text.is_empty(),
        DocNode::ListItem { text, .. } => !text.is_empty(),
        DocNode::CodeBlock { content, .. } => !content.is_empty(),
    });
    if first_starter > 0 && pre_has_content {
        sections.push(Section {
            start_node_idx: 0,
            end_node_idx: first_starter - 1,
            kind: SectionKind::PreHeading,
        });
    }

    for (i, &(start, kind)) in starters.iter().enumerate() {
        let next_start = starters.get(i + 1).map(|(j, _)| *j).unwrap_or(n);
        let end = next_start - 1;
        sections.push(Section {
            start_node_idx: start,
            end_node_idx: end,
            kind,
        });
    }

    sections
}

/// Sentence segmenter operating on selection plain text.
///
/// Phase-1 internal duplicate of `app.rs::sentence_ranges_from_plain`.
/// Phase-2 will move this into `selection::segment` as the canonical
/// implementation and delete the duplicate.
pub(crate) fn segment_sentences_internal(plain: &str) -> Vec<Range<usize>> {
    if plain.trim().is_empty() {
        return Vec::new();
    }
    let mut ranges: Vec<Range<usize>> = Vec::new();
    let bytes = plain.as_bytes();
    let len = bytes.len();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < len {
        if bytes[i] == b'\n' {
            let mut j = i + 1;
            while j < len && bytes[j] == b' ' {
                j += 1;
            }
            if j < len && bytes[j].is_ascii_lowercase() {
                i += 1;
                continue;
            }
            // Trim trailing whitespace on the segment.
            let mut end = i;
            while end > start && (bytes[end - 1] == b' ' || bytes[end - 1] == b'\n') {
                end -= 1;
            }
            if end > start {
                ranges.push(start..end);
            }
            start = j;
            i = j;
            continue;
        }
        if matches!(bytes[i], b'.' | b'!' | b'?')
            && i + 2 < len
            && bytes[i + 1] == b' '
            && bytes[i + 2].is_ascii_uppercase()
        {
            ranges.push(start..i + 1);
            i += 2;
            start = i;
            continue;
        }
        i += 1;
    }
    if start < len {
        let mut end = len;
        while end > start && (bytes[end - 1] == b' ' || bytes[end - 1] == b'\n') {
            end -= 1;
        }
        if end > start {
            ranges.push(start..end);
        }
    }
    if ranges.is_empty() {
        let trimmed_start = plain.bytes().take_while(|b| *b == b' ' || *b == b'\n').count();
        let mut end = plain.len();
        while end > trimmed_start
            && (plain.as_bytes()[end - 1] == b' ' || plain.as_bytes()[end - 1] == b'\n')
        {
            end -= 1;
        }
        if end > trimmed_start {
            ranges.push(trimmed_start..end);
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_sentences_splits_on_period_space_uppercase() {
        let r = segment_sentences_internal("First sentence. Second sentence.");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], 0..15);
    }

    #[test]
    fn segment_sentences_empty_yields_empty() {
        assert!(segment_sentences_internal("").is_empty());
        assert!(segment_sentences_internal("   ").is_empty());
    }

    #[test]
    fn segment_sentences_no_terminal_punct_one_segment() {
        let r = segment_sentences_internal("just words no period");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0], 0..20);
    }
}
