//! Selection index — the eager, owned navigation cache built from a parsed
//! `Document` at load time.
//!
//! Holds per-node selection plain text, sentence ranges, source-line ranges,
//! word ranges, the document-level paragraph/line/sentence/word linear-order
//! tables, and the section table. See `modular_plan.md` § "Internal
//! representation" for the contract.

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

/// UI-neutral input used to build the canonical selection index.
///
/// Markdown adapts parsed `DocNode`s into this shape. The HTML web frontend
/// supplies the same facts from its validated browser manifest, which keeps
/// navigation and clamping independent from either parser or renderer.
#[derive(Debug, Clone, Default)]
pub(crate) struct SelectionNodeInput {
    pub plain_text: String,
    pub source_line_ranges: Vec<(usize, Range<usize>)>,
    pub sentence_ranges: Vec<Range<usize>>,
    pub word_ranges: Vec<Range<usize>>,
    pub heading_level: Option<u8>,
    pub list_id: Option<usize>,
    pub is_top_level_ordered_list_item: bool,
    pub has_content: bool,
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
    pub(crate) fn build(doc: &Document, source_lines: &[String]) -> Self {
        let inputs = doc
            .nodes
            .iter()
            .map(|node| markdown_node_input(node, source_lines))
            .collect();
        Self::from_nodes(inputs)
    }

    /// Build from UI-neutral nodes. All ranges are expected to be byte ranges
    /// within each node's `plain_text`; debug assertions pin those invariants
    /// for adapters while keeping production parsing total.
    pub(crate) fn from_nodes(inputs: Vec<SelectionNodeInput>) -> Self {
        let mut nodes: Vec<NodeIndex> = Vec::with_capacity(inputs.len());
        let mut paragraphs: Vec<(usize, usize)> = Vec::new();
        let mut lines: Vec<(usize, usize)> = Vec::new();
        let mut sentences: Vec<(usize, usize)> = Vec::new();
        let mut words: Vec<(usize, usize)> = Vec::new();

        for (node_idx, input) in inputs.iter().enumerate() {
            debug_assert!(ranges_are_valid(
                &input.source_line_ranges,
                &input.plain_text
            ));
            debug_assert!(byte_ranges_are_valid(
                &input.sentence_ranges,
                &input.plain_text
            ));
            debug_assert!(byte_ranges_are_valid(&input.word_ranges, &input.plain_text));

            // Linear-order tables.
            if input.has_content && !input.plain_text.trim().is_empty() {
                paragraphs.push((node_idx, 0));
            }
            for li in 0..input.source_line_ranges.len() {
                lines.push((node_idx, li));
            }
            for si in 0..input.sentence_ranges.len() {
                sentences.push((node_idx, si));
            }
            for wi in 0..input.word_ranges.len() {
                words.push((node_idx, wi));
            }

            nodes.push(NodeIndex {
                selection_plain_text: input.plain_text.clone(),
                source_line_ranges: input.source_line_ranges.clone(),
                sentence_ranges: input.sentence_ranges.clone(),
                word_ranges: input.word_ranges.clone(),
            });
        }

        let sections = build_section_table(&inputs);

        debug_assert!(
            sections
                .iter()
                .all(|s| s.start_node_idx <= s.end_node_idx && s.end_node_idx < inputs.len()),
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

fn markdown_node_input(node: &DocNode, source_lines: &[String]) -> SelectionNodeInput {
    let plain_text = node_selection_plain_text(node, source_lines);
    let source_line_ranges = node_source_line_ranges(node, source_lines, &plain_text);
    // Sentence-bearing rules per modular_plan:
    //   - Paragraph: canonical segmentation.
    //   - Heading / ListItem: one full-range anchor.
    //   - CodeBlock / ThematicBreak: no sentence anchors.
    let sentence_ranges = if plain_text.is_empty() {
        Vec::new()
    } else {
        match node {
            DocNode::Paragraph { .. } => crate::selection::segment::segment_sentences(&plain_text),
            DocNode::Heading { .. } | DocNode::ListItem { .. } => {
                std::iter::once(0..plain_text.len()).collect()
            }
            DocNode::CodeBlock { .. } | DocNode::ThematicBreak { .. } => Vec::new(),
        }
    };
    let word_ranges = match node {
        DocNode::ThematicBreak { .. } => Vec::new(),
        _ => crate::selection::segment::segment_words(&plain_text),
    };
    let heading_level = match node {
        DocNode::Heading { level, .. } => Some(*level),
        _ => None,
    };
    let list_id = match node {
        DocNode::ListItem { list_id, .. } => Some(*list_id),
        _ => None,
    };
    let is_top_level_ordered_list_item = matches!(
        node,
        DocNode::ListItem {
            ordered: true,
            depth: 0,
            ..
        }
    );

    SelectionNodeInput {
        has_content: node.has_content(),
        plain_text,
        source_line_ranges,
        sentence_ranges,
        word_ranges,
        heading_level,
        list_id,
        is_top_level_ordered_list_item,
    }
}

fn byte_ranges_are_valid(ranges: &[Range<usize>], text: &str) -> bool {
    ranges.iter().all(|range| {
        range.start <= range.end
            && range.end <= text.len()
            && text.is_char_boundary(range.start)
            && text.is_char_boundary(range.end)
    })
}

fn ranges_are_valid(ranges: &[(usize, Range<usize>)], text: &str) -> bool {
    byte_ranges_are_valid(
        &ranges
            .iter()
            .map(|(_, range)| range.clone())
            .collect::<Vec<_>>(),
        text,
    )
}

/// Convert a UTF-8 byte range into Unicode scalar offsets for the browser
/// protocol. JavaScript converts these scalar offsets to its UTF-16 DOM
/// boundaries using the manifest's client-side text map.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn byte_range_to_scalar_range(text: &str, range: Range<usize>) -> Option<Range<usize>> {
    if range.start > range.end
        || range.end > text.len()
        || !text.is_char_boundary(range.start)
        || !text.is_char_boundary(range.end)
    {
        return None;
    }
    let start = text[..range.start].chars().count();
    let end = start + text[range].chars().count();
    Some(start..end)
}

/// Compute selection plain text for a node, stripping markers per Req 11.
/// Called once per node during `SelectionIndex::build`; readers consume
/// the resulting `NodeIndex::selection_plain_text` directly.
pub(crate) fn node_selection_plain_text(node: &DocNode, source_lines: &[String]) -> String {
    match node {
        DocNode::Heading { text, .. } | DocNode::Paragraph { text, .. } => text.clone(),
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
    let stripped = strip_ordered_marker(trimmed)
        .or_else(|| trimmed.strip_prefix("- "))
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
        .unwrap_or(trimmed);
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
            // One Line anchor per source line; the GFM-table-separator
            // line (`| --- | --- |` shape) is excluded since it's not
            // part of the selection plain text per modular_plan
            // §"Block-type coverage / GFM table".
            //
            // Each anchor's byte range is the source line's slice within
            // the node's selection plain text — needed so projection /
            // clamp / strike rendering can paint per-line precisely.
            // Plain text is the parser-joined paragraph text (soft-wraps
            // collapsed to spaces; tables joined with `\n`); we locate
            // each source line by scanning forward from the previous
            // line's end with progressively-trimmed content.
            let lines: Vec<usize> = range
                .clone()
                .filter(|l| *l < source_lines.len())
                .filter(|l| !is_table_separator_line(&source_lines[*l]))
                .collect();
            paragraph_per_line_ranges(&lines, source_lines, plain)
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
            // One anchor per non-fence source line. Plain text for code
            // blocks preserves source lines joined by `\n`, so per-line
            // ranges split cleanly on those newline boundaries.
            let lines: Vec<usize> = range
                .clone()
                .filter(|&l| {
                    source_lines
                        .get(l)
                        .is_some_and(|s| !s.trim_start().starts_with("```"))
                })
                .collect();
            code_block_per_line_ranges(&lines, plain)
        }
        DocNode::ThematicBreak { .. } => Vec::new(),
    }
}

/// Scan paragraph plain text to locate each source line's contribution.
/// Returns a `(source_line_no, byte_range_in_plain)` per kept line. A
/// source line that can't be found falls back to `0..plain.len()` so the
/// caller never sees a missing anchor.
fn paragraph_per_line_ranges(
    lines: &[usize],
    source_lines: &[String],
    plain: &str,
) -> Vec<(usize, Range<usize>)> {
    let mut out = Vec::with_capacity(lines.len());
    let mut cursor = 0usize;
    for &l in lines {
        let needle = source_lines.get(l).map(|s| s.trim()).unwrap_or("");
        if needle.is_empty() {
            // Empty source line contributes no real text; collapse to a
            // zero-width range at the cursor.
            out.push((l, cursor..cursor));
            continue;
        }
        if let Some(rel) = plain[cursor..].find(needle) {
            let start = cursor + rel;
            let end = start + needle.len();
            out.push((l, start..end));
            cursor = end;
        } else {
            // Soft-wrap may have collapsed leading/trailing punct; fall
            // back to whole-node range so callers can still reason about
            // this line at all.
            out.push((l, 0..plain.len()));
        }
    }
    out
}

/// Code-block plain text preserves source lines joined by `\n`. Walk
/// the byte string and map each non-fence source line to its slice.
fn code_block_per_line_ranges(lines: &[usize], plain: &str) -> Vec<(usize, Range<usize>)> {
    let mut out = Vec::with_capacity(lines.len());
    let mut cursor = 0usize;
    for &l in lines {
        let next_nl = plain[cursor..].find('\n').map(|i| cursor + i);
        let end = next_nl.unwrap_or(plain.len());
        out.push((l, cursor..end));
        cursor = end + 1; // step past the `\n`
        if cursor > plain.len() {
            cursor = plain.len();
        }
    }
    out
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
fn build_section_table(inputs: &[SelectionNodeInput]) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let n = inputs.len();
    if n == 0 {
        return sections;
    }

    // Carry the heading level alongside each starter so end_node_idx can
    // later be computed using "next equal-or-shallower heading" per
    // modular_plan §"Section unit". OL starters use level u8::MAX (no
    // heading-nesting interaction; OL section ends at next heading or
    // end of doc). PreHeading uses level 0 conceptually but never plays
    // into the close-out logic since it's emitted separately below.
    let mut starters: Vec<(usize, SectionKind, u8)> = Vec::new();
    let mut seen_heading = false;
    let mut current_top_ol_list_id: Option<usize> = None;
    for (i, input) in inputs.iter().enumerate() {
        if let Some(level) = input.heading_level {
            starters.push((i, SectionKind::Heading, level));
            seen_heading = true;
            current_top_ol_list_id = None;
        } else if input.is_top_level_ordered_list_item {
            let list_id = input
                .list_id
                .expect("top-level ordered list item must carry a list id");
            if !seen_heading && current_top_ol_list_id != Some(list_id) {
                starters.push((i, SectionKind::Ol, u8::MAX));
                current_top_ol_list_id = Some(list_id);
            }
        } else if input.list_id == current_top_ol_list_id {
            // Nested item of the active ordered list keeps the run open.
        } else {
            current_top_ol_list_id = None;
        }
    }

    // Pre-heading "section 0" — present only when at least one pre-starter
    // node would carry a paragraph-unit anchor (i.e. has selectable
    // content per the wordless-skip rule). A heading-less document with
    // no OL starter falls through this check (first_starter == n,
    // first_starter > 0 is true but no real sections follow); skip
    // emitting a PreHeading in that case so prose-only docs end up with
    // an empty section table.
    let first_starter = starters.first().map_or(n, |(i, _, _)| *i);
    let pre_has_content = inputs[..first_starter]
        .iter()
        .any(|input| input.has_content && !input.plain_text.is_empty());
    let has_real_starters = !starters.is_empty();
    if first_starter > 0 && pre_has_content && has_real_starters {
        sections.push(Section {
            start_node_idx: 0,
            end_node_idx: first_starter - 1,
            kind: SectionKind::PreHeading,
        });
    }

    for (i, &(start, kind, level)) in starters.iter().enumerate() {
        // A heading at level L ends at the NEXT starter at level <= L,
        // so subordinate (deeper) headings don't close the parent's
        // section span. OL starters and (conceptually) PreHeading use
        // level u8::MAX so the next ANY starter closes them.
        let next_start = starters
            .iter()
            .skip(i + 1)
            .find(|(_, _, l)| *l <= level)
            .map_or(n, |(j, _, _)| *j);
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
    use crate::selection::build_test_index as build;

    fn neutral_node(
        text: &str,
        heading_level: Option<u8>,
        source_line: usize,
    ) -> SelectionNodeInput {
        let sentence_ranges = crate::selection::segment::segment_sentences(text);
        let word_ranges = crate::selection::segment::segment_words(text);
        SelectionNodeInput {
            plain_text: text.to_string(),
            source_line_ranges: (!text.is_empty())
                .then_some((source_line, 0..text.len()))
                .into_iter()
                .collect(),
            sentence_ranges,
            word_ranges,
            heading_level,
            list_id: None,
            is_top_level_ordered_list_item: false,
            has_content: !text.is_empty(),
        }
    }

    #[test]
    fn empty_doc_index_is_empty() {
        let idx = build("");
        assert!(idx.nodes.is_empty());
        assert!(idx.sections.is_empty());
        assert!(idx.paragraphs.is_empty());
        assert!(idx.sentences.is_empty());
    }

    #[test]
    fn paragraph_sentences_round_trip() {
        let idx = build("First sentence here. Second one too.");
        assert_eq!(idx.nodes.len(), 1);
        assert_eq!(idx.nodes[0].sentence_ranges.len(), 2);
        assert_eq!(idx.sentences.len(), 2);
    }

    #[test]
    fn neutral_nodes_build_all_linear_tables_and_nested_sections() {
        let inputs = vec![
            neutral_node("Preface.", None, 0),
            neutral_node("Top", Some(1), 1),
            neutral_node("First sentence. Second sentence.", None, 2),
            neutral_node("Nested", Some(2), 3),
            neutral_node("Nested body.", None, 4),
            neutral_node("Next", Some(1), 5),
        ];

        let idx = SelectionIndex::from_nodes(inputs);

        assert_eq!(idx.nodes.len(), 6);
        assert_eq!(idx.paragraphs.len(), 6);
        assert_eq!(idx.lines.len(), 6);
        assert_eq!(idx.sentences.len(), 7);
        assert!(idx.words.len() >= 8);
        assert_eq!(idx.sections.len(), 4);
        assert_eq!(idx.sections[0].kind, SectionKind::PreHeading);
        assert_eq!(idx.sections[0].start_node_idx, 0);
        assert_eq!(idx.sections[0].end_node_idx, 0);
        assert_eq!(idx.sections[1].start_node_idx, 1);
        assert_eq!(idx.sections[1].end_node_idx, 4);
        assert_eq!(idx.sections[2].start_node_idx, 3);
        assert_eq!(idx.sections[2].end_node_idx, 4);
        assert_eq!(idx.sections[3].start_node_idx, 5);
    }

    #[test]
    fn neutral_ordered_list_metadata_keeps_nested_items_in_one_section() {
        let mut first = neutral_node("First", None, 0);
        first.list_id = Some(7);
        first.is_top_level_ordered_list_item = true;
        let mut nested = neutral_node("Nested", None, 1);
        nested.list_id = Some(7);
        let mut second = neutral_node("Second", None, 2);
        second.list_id = Some(7);
        second.is_top_level_ordered_list_item = true;

        let idx = SelectionIndex::from_nodes(vec![first, nested, second]);

        assert_eq!(idx.sections.len(), 1);
        assert_eq!(idx.sections[0].kind, SectionKind::Ol);
        assert_eq!(idx.sections[0].start_node_idx, 0);
        assert_eq!(idx.sections[0].end_node_idx, 2);
    }

    #[test]
    fn byte_ranges_convert_to_unicode_scalar_offsets() {
        let text = "A🚀e\u{301}Z";
        let rocket_start = "A".len();
        let combining_end = "A🚀e\u{301}".len();

        assert_eq!(
            byte_range_to_scalar_range(text, rocket_start..combining_end),
            Some(1..4)
        );
        assert_eq!(
            byte_range_to_scalar_range(text, (rocket_start + 1)..combining_end),
            None,
            "a range that splits the rocket's UTF-8 encoding must be rejected"
        );
    }

    #[test]
    fn section_table_pre_heading_then_heading() {
        let idx = build("Pre-heading prose.\n\n# Heading\n\nUnder heading.");
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
        let idx = build("1. first\n2. second\n3. third");
        assert_eq!(idx.sections.len(), 1, "{:?}", idx.sections);
        assert_eq!(idx.sections[0].kind, SectionKind::Ol);
        assert_eq!(idx.sections[0].start_node_idx, 0);
        assert_eq!(idx.sections[0].end_node_idx, idx.nodes.len() - 1);
    }

    #[test]
    fn section_table_ol_after_heading_does_not_start_section() {
        // Once any heading is seen, a later top-level OL no longer starts
        // its own section — it folds into the surrounding heading section.
        let idx = build("# Top\n\n1. a\n2. b");
        assert_eq!(idx.sections.len(), 1);
        assert_eq!(idx.sections[0].kind, SectionKind::Heading);
    }

    #[test]
    fn section_table_pre_heading_skipped_when_only_thematic_break() {
        // `---` alone before a heading does not contribute selectable
        // content, so no PreHeading section is emitted.
        let idx = build("---\n\n# Heading");
        assert_eq!(idx.sections.len(), 1);
        assert_eq!(idx.sections[0].kind, SectionKind::Heading);
    }

    #[test]
    fn section_table_prose_only_doc_has_no_sections() {
        // Per modular_plan §"Section unit": a document with no headings
        // and no top-level OL has no section starters at all — section
        // nav is a no-op, not a single-PreHeading-section walk.
        let idx = build("Just plain prose. No headings.");
        assert!(
            idx.sections.is_empty(),
            "prose-only doc should have zero sections, got {:?}",
            idx.sections
        );
    }

    #[test]
    fn section_table_subordinate_heading_does_not_close_parent_section() {
        // Per modular_plan §"Section unit": "Nested heading levels nest.
        // ## sub inside a # parent does not end # parent's section; the
        // section ends at the next #-or-shallower heading."
        let idx = build("# A\n\nbody\n\n## sub\n\nmore body\n\n# B\n\nbody B");
        let kinds: Vec<_> = idx.sections.iter().map(|s| s.kind).collect();
        // Three section starters: A, sub, B. (Each subordinate heading
        // is itself addressable as a section.)
        assert_eq!(kinds.len(), 3);
        // Section A spans through the body of sub up to (but not
        // including) section B's start.
        let sec_a = idx
            .sections
            .iter()
            .find(|s| s.kind == SectionKind::Heading && s.start_node_idx == 0)
            .expect("section A");
        let sec_b = idx
            .sections
            .iter()
            .filter(|s| s.kind == SectionKind::Heading)
            .nth(2)
            .expect("section B");
        assert!(
            sec_a.end_node_idx >= sec_b.start_node_idx - 1,
            "A.end ({}) should reach up to B.start - 1 ({})",
            sec_a.end_node_idx,
            sec_b.start_node_idx - 1
        );
        assert!(
            sec_a.end_node_idx > 0,
            "section A must include nodes after the heading; ended at {}",
            sec_a.end_node_idx
        );
    }

    #[test]
    fn section_table_top_level_ol_with_nested_items_stays_one_section() {
        // Per modular_plan §"Section unit": "A section started by a
        // top-level OL spans the entire list (not one section per
        // item)." Nested items between top-level items are still part
        // of the same list and must not split the section.
        let idx = build("1. first\n   - nested bullet\n2. second\n3. third");
        let ol_sections: Vec<_> = idx
            .sections
            .iter()
            .filter(|s| s.kind == SectionKind::Ol)
            .collect();
        assert_eq!(
            ol_sections.len(),
            1,
            "expected one OL section spanning the whole list, got {ol_sections:?}"
        );
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
        let doc = Document::parse("# My Heading").unwrap();
        assert_eq!(
            node_selection_plain_text(&doc.nodes[0], &lines),
            "My Heading"
        );

        // Paragraph: parsed plain text.
        let lines: Vec<String> = vec!["A paragraph here.".into()];
        let doc = Document::parse("A paragraph here.").unwrap();
        assert_eq!(
            node_selection_plain_text(&doc.nodes[0], &lines),
            "A paragraph here."
        );

        // ListItem: source-line join with markers stripped.
        let src = "- the item text";
        let lines: Vec<String> = src.lines().map(ToOwned::to_owned).collect();
        let doc = Document::parse(src).unwrap();
        assert_eq!(
            node_selection_plain_text(&doc.nodes[0], &lines),
            "the item text"
        );

        // CodeBlock: fence lines excluded.
        let src = "```\nfn x() {}\n```";
        let lines: Vec<String> = src.lines().map(ToOwned::to_owned).collect();
        let doc = Document::parse(src).unwrap();
        assert_eq!(
            node_selection_plain_text(&doc.nodes[0], &lines),
            "fn x() {}"
        );

        // ThematicBreak: empty.
        let src = "---";
        let lines: Vec<String> = src.lines().map(ToOwned::to_owned).collect();
        let doc = Document::parse(src).unwrap();
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
