//! Selection index — the eager, owned navigation cache built from a parsed
//! `Document` at load time. Holds per-node selection plain text, sentence
//! ranges, source-line ranges, word ranges, the document-level
//! paragraph/line/sentence/word linear-order tables, and the section table.
//! See `modular_plan.md` § "Internal representation" for the contract.

use std::ops::Range;

use crate::document::{DocNode, Document};

/// What kind of node started a section: a `#`-level heading, a top-level
/// ordered list (when no heading appears earlier), or the implicit
/// pre-heading "section 0" of a doc whose first content lives before any
/// section starter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SectionKind {
    Heading,
    Ol,
    PreHeading,
}

/// A section spans a contiguous run of `node_idx` values. Both endpoints
/// are inclusive; the contiguity invariant is asserted at index-build time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Section {
    pub start_node_idx: usize,
    pub end_node_idx: usize,
    pub kind: SectionKind,
}

/// Per-node owned cache: selection plain text and the byte-range tables
/// (source_line, sentence, word) used by navigation, capture, and emit.
#[derive(Debug, Clone, Default)]
pub struct NodeIndex {
    /// Selection plain text — markers stripped.
    pub selection_plain_text: String,
    /// Pairs of `(source_line, range_in_selection_plain_text)`.
    pub source_line_ranges: Vec<(usize, Range<usize>)>,
    /// Sentence byte ranges within `selection_plain_text`.
    pub sentence_ranges: Vec<Range<usize>>,
    /// Word byte ranges within `selection_plain_text`.
    pub word_ranges: Vec<Range<usize>>,
}

/// Eagerly-built navigation cache for a parsed `Document`. Holds owned
/// per-node text + per-unit linear-order tables; built once at load time
/// per `modular_plan.md` Req 11 and lives for the process.
#[derive(Debug, Clone, Default)]
pub struct SelectionIndex {
    pub nodes: Vec<NodeIndex>,
    pub paragraphs: Vec<(usize, usize)>,
    pub lines: Vec<(usize, usize)>,
    pub sentences: Vec<(usize, usize)>,
    pub words: Vec<(usize, usize)>,
    pub(crate) sections: Vec<Section>,
}

impl SelectionIndex {
    /// Eager build at load time per Req 11.
    pub fn build(doc: &Document, source_lines: &[String]) -> Self {
        let mut nodes: Vec<NodeIndex> = Vec::with_capacity(doc.nodes.len());
        let mut paragraphs: Vec<(usize, usize)> = Vec::new();
        let mut lines: Vec<(usize, usize)> = Vec::new();
        let mut sentences: Vec<(usize, usize)> = Vec::new();
        let mut words: Vec<(usize, usize)> = Vec::new();

        for (node_idx, node) in doc.nodes.iter().enumerate() {
            let plain = node_selection_plain_text(node, source_lines);
            let source_line_ranges = node_source_line_ranges(node, source_lines, &plain);
            // Sentence-bearing rules per modular_plan:
            //   - Paragraph: segment with the canonical segmenter.
            //   - Heading / ListItem: one anchor covering the full plain
            //     text (matches today's rendered_nodes single_range count).
            //     Multi-paragraph list items remain a known limitation; the
            //     full item is one sentence anchor.
            //   - CodeBlock: zero anchors (excluded from sentence-level
            //     navigation per Pinned decisions § Movement rules).
            //   - ThematicBreak: zero anchors.
            let sentence_ranges: Vec<Range<usize>> = if plain.is_empty() {
                Vec::new()
            } else {
                match node {
                    DocNode::Paragraph { .. } => {
                        crate::selection::segment::segment_sentences(&plain)
                    }
                    DocNode::Heading { .. } | DocNode::ListItem { .. } => {
                        // Single full-range anchor; clippy::single_range_in_vec_init
                        // would suggest std::iter::once but the call site wants
                        // an owned Vec<Range<usize>>.
                        #[allow(clippy::single_range_in_vec_init)]
                        let v = vec![0..plain.len()];
                        v
                    }
                    DocNode::CodeBlock { .. } | DocNode::ThematicBreak { .. } => Vec::new(),
                }
            };

            // Word ranges per modular_plan: code blocks excluded from
            // sentence-level navigation but allowed at word level (their
            // selection plain text already excludes fence lines, so words
            // come from the content lines). ListItem and Heading: words
            // segmented from selection plain text. Paragraph: same.
            // ThematicBreak: empty.
            let word_ranges: Vec<Range<usize>> = match node {
                DocNode::ThematicBreak { .. } => Vec::new(),
                _ => crate::selection::segment::segment_words(&plain),
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
            for wi in 0..word_ranges.len() {
                words.push((node_idx, wi));
            }

            nodes.push(NodeIndex {
                selection_plain_text: plain,
                source_line_ranges,
                sentence_ranges,
                word_ranges,
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
            words,
            sections,
        }
    }
}

/// Compute selection plain text for a node, stripping markers per Req 11.
/// This is the canonical implementation; `selection::segment::plain_text_for_node`
/// re-exports it as the single visible entrypoint.
pub(crate) fn node_selection_plain_text(node: &DocNode, source_lines: &[String]) -> String {
    match node {
        DocNode::Heading { text, .. } => text.clone(),
        DocNode::Paragraph { text, .. } => text.clone(),
        DocNode::ListItem {
            source_lines: range,
            ..
        } => {
            // Reuse the join logic that `app.rs::join_node_source_lines` performs:
            // strip the leading bullet/number marker and task marker on the first
            // line, then space-join with subsequent lines. We re-implement here so
            // selection-layer code does not depend on app internals.
            let slice = source_lines
                .get(range.start..range.end.min(source_lines.len()))
                .unwrap_or(&[]);
            let joined = slice.iter().map(|s| s.trim()).collect::<Vec<_>>().join(" ");
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
    if i < bytes.len()
        && (bytes[i] == b'.' || bytes[i] == b')')
        && i + 1 < bytes.len()
        && bytes[i + 1] == b' '
    {
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
        DocNode::Paragraph {
            source_lines: range,
            ..
        } => {
            // Phase-4 line anchors per source line; the GFM-table-separator
            // line (`| --- | --- |` shape) is excluded since it's not part
            // of the selection plain text per modular_plan §"Block-type
            // coverage / GFM table".
            range
                .clone()
                .filter(|l| *l < source_lines.len())
                .filter(|l| !is_table_separator_line(&source_lines[*l]))
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
            // One anchor per non-fence source line. We emit `(line, full-range)` —
            // the byte range covers the whole node and the (line, _) pair is what
            // line-unit lookups consult. Per-line slice mapping isn't needed
            // because where_for_annotation Line case reads the line number from
            // this entry and `current_line_capture` reads the source line text
            // directly via `source_lines[line]`.
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

/// True when a source line is a GFM table header-separator row, e.g.
/// `| --- | --- |` with optional alignment colons.
fn is_table_separator_line(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') || !trimmed.ends_with('|') {
        return false;
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return false;
    }
    inner.split('|').all(|cell| {
        let s = cell.trim();
        !s.is_empty()
            && s.chars()
                .all(|c| c == '-' || c == ':' || c.is_ascii_whitespace())
            && s.contains('-')
    })
}

fn node_contributes_paragraph_anchor(node: &DocNode, plain: &str) -> bool {
    match node {
        DocNode::ThematicBreak { .. } => false,
        _ => !plain.trim().is_empty(),
    }
}

/// Build the section table per the pinned modular_plan rules.
///
/// - Headings always start a section.
/// - A top-level ordered list counts as a section start **only when no
///   `#`-level heading appears anywhere before it**. The OL section spans
///   the whole list (all contiguous top-level OL items), not one section per
///   item.
/// - Pre-heading content (a "section 0") is present iff at least one node
///   in the pre-starter region has selectable content.
/// - Section endpoints are inclusive on both ends and run contiguously over
///   `node_idx` values.
fn build_section_table(doc: &Document) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let n = doc.nodes.len();
    if n == 0 {
        return sections;
    }

    let mut starters: Vec<(usize, SectionKind)> = Vec::new();
    let mut seen_heading = false;
    let mut last_starter_was_ol = false;
    for (i, node) in doc.nodes.iter().enumerate() {
        match node {
            DocNode::Heading { .. } => {
                starters.push((i, SectionKind::Heading));
                seen_heading = true;
                last_starter_was_ol = false;
            }
            DocNode::ListItem {
                ordered: true,
                depth: 0,
                ..
            } if !seen_heading => {
                // Open a new OL-section starter only at the first OL item of
                // a top-level OL run; subsequent items fold into the same
                // section.
                if !last_starter_was_ol {
                    starters.push((i, SectionKind::Ol));
                    last_starter_was_ol = true;
                }
            }
            _ => {
                // Anything that's not a contiguous OL item resets the
                // run-fold. The next OL would start a new section starter
                // again — but only if `seen_heading` is still false.
                last_starter_was_ol = false;
            }
        }
    }

    // Pre-heading "section 0" — present only when the pre-starter region has
    // any selectable node.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    #[test]
    fn empty_doc_index_is_empty() {
        let doc = Document::parse("");
        let idx = SelectionIndex::build(&doc, &[]);
        assert!(idx.nodes.is_empty());
        assert!(idx.sections.is_empty());
        assert!(idx.paragraphs.is_empty());
        assert!(idx.sentences.is_empty());
    }

    #[test]
    fn paragraph_sentences_round_trip() {
        let src = "First sentence here. Second one too.";
        let lines: Vec<String> = src.lines().map(ToOwned::to_owned).collect();
        let doc = Document::parse(src);
        let idx = SelectionIndex::build(&doc, &lines);
        assert_eq!(idx.nodes.len(), 1);
        assert_eq!(idx.nodes[0].sentence_ranges.len(), 2);
        assert_eq!(idx.sentences.len(), 2);
    }

    #[test]
    fn section_table_pre_heading_then_heading() {
        let src = "Pre-heading prose.\n\n# Heading\n\nUnder heading.";
        let lines: Vec<String> = src.lines().map(ToOwned::to_owned).collect();
        let doc = Document::parse(src);
        let idx = SelectionIndex::build(&doc, &lines);
        // Sections: PreHeading (node 0..0), Heading (1..2)
        assert_eq!(idx.sections.len(), 2);
        assert_eq!(idx.sections[0].kind, SectionKind::PreHeading);
        assert_eq!(idx.sections[1].kind, SectionKind::Heading);
    }

    #[test]
    fn section_table_top_level_ol_is_one_section_pre_heading() {
        // Top-level OL with no preceding heading is a single section that
        // spans every contiguous top-level OL item; the items don't each
        // become their own section starter.
        let src = "1. first\n2. second\n3. third";
        let lines: Vec<String> = src.lines().map(ToOwned::to_owned).collect();
        let doc = Document::parse(src);
        let idx = SelectionIndex::build(&doc, &lines);
        assert_eq!(idx.sections.len(), 1, "{:?}", idx.sections);
        assert_eq!(idx.sections[0].kind, SectionKind::Ol);
        assert_eq!(idx.sections[0].start_node_idx, 0);
        assert_eq!(idx.sections[0].end_node_idx, doc.nodes.len() - 1);
    }

    #[test]
    fn section_table_ol_after_heading_does_not_start_section() {
        // Once any heading is seen, a later top-level OL no longer starts
        // its own section — it folds into the surrounding heading section.
        let src = "# Top\n\n1. a\n2. b";
        let lines: Vec<String> = src.lines().map(ToOwned::to_owned).collect();
        let doc = Document::parse(src);
        let idx = SelectionIndex::build(&doc, &lines);
        assert_eq!(idx.sections.len(), 1);
        assert_eq!(idx.sections[0].kind, SectionKind::Heading);
    }

    #[test]
    fn section_table_pre_heading_skipped_when_only_thematic_break() {
        // `---` alone before a heading does not contribute selectable
        // content, so no PreHeading section is emitted.
        let src = "---\n\n# Heading";
        let lines: Vec<String> = src.lines().map(ToOwned::to_owned).collect();
        let doc = Document::parse(src);
        let idx = SelectionIndex::build(&doc, &lines);
        assert_eq!(idx.sections.len(), 1);
        assert_eq!(idx.sections[0].kind, SectionKind::Heading);
    }

    #[test]
    fn is_table_separator_recognizes_canonical_shapes() {
        assert!(is_table_separator_line("| --- | --- |"));
        assert!(is_table_separator_line("|---|---|"));
        assert!(is_table_separator_line("| :--- | ---: | :---: |"));
        assert!(is_table_separator_line("  | --- | --- |  "));
    }

    #[test]
    fn node_selection_plain_text_per_variant() {
        // Heading: returns the parsed text (markers stripped by parser).
        let lines: Vec<String> = vec!["# My Heading".into()];
        let doc = Document::parse("# My Heading");
        assert_eq!(
            node_selection_plain_text(&doc.nodes[0], &lines),
            "My Heading"
        );

        // Paragraph: parsed plain text.
        let lines: Vec<String> = vec!["A paragraph here.".into()];
        let doc = Document::parse("A paragraph here.");
        assert_eq!(
            node_selection_plain_text(&doc.nodes[0], &lines),
            "A paragraph here."
        );

        // ListItem: source-line join with markers stripped.
        let src = "- the item text";
        let lines: Vec<String> = src.lines().map(ToOwned::to_owned).collect();
        let doc = Document::parse(src);
        assert_eq!(
            node_selection_plain_text(&doc.nodes[0], &lines),
            "the item text"
        );

        // CodeBlock: fence lines excluded.
        let src = "```\nfn x() {}\n```";
        let lines: Vec<String> = src.lines().map(ToOwned::to_owned).collect();
        let doc = Document::parse(src);
        assert_eq!(
            node_selection_plain_text(&doc.nodes[0], &lines),
            "fn x() {}"
        );

        // ThematicBreak: empty.
        let src = "---";
        let lines: Vec<String> = src.lines().map(ToOwned::to_owned).collect();
        let doc = Document::parse(src);
        assert_eq!(node_selection_plain_text(&doc.nodes[0], &lines), "");
    }

    #[test]
    fn strip_list_marker_handles_bullets_numbers_and_tasks() {
        // Plain bullet markers.
        assert_eq!(strip_list_marker("- item"), "item");
        assert_eq!(strip_list_marker("* item"), "item");
        assert_eq!(strip_list_marker("+ item"), "item");
        // Ordered markers (period and right-paren).
        assert_eq!(strip_list_marker("1. alpha"), "alpha");
        assert_eq!(strip_list_marker("23. beta"), "beta");
        assert_eq!(strip_list_marker("4) gamma"), "gamma");
        // Task markers without a list prefix.
        assert_eq!(strip_list_marker("[ ] open"), "open");
        assert_eq!(strip_list_marker("[x] done"), "done");
        assert_eq!(strip_list_marker("[X] done caps"), "done caps");
        // Bullet + task together.
        assert_eq!(strip_list_marker("- [ ] open task"), "open task");
        assert_eq!(strip_list_marker("1. [x] done task"), "done task");
        // No-marker input passes through.
        assert_eq!(strip_list_marker("plain text"), "plain text");
        // Leading whitespace before marker is fine.
        assert_eq!(strip_list_marker("  - indented item"), "indented item");
    }

    #[test]
    fn is_table_separator_rejects_non_separator_rows() {
        assert!(!is_table_separator_line("| Col A | Col B |"));
        assert!(!is_table_separator_line("| a1 | b1 |"));
        assert!(!is_table_separator_line("not a table"));
        // Cells must be non-empty, contain at least one '-', and only
        // hyphens / colons / whitespace.
        assert!(!is_table_separator_line("| | |"));
        assert!(!is_table_separator_line("| :: | :: |"));
        assert!(!is_table_separator_line("| -a | -- |"));
    }
}
