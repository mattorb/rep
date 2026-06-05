use std::ops::Range;

use anyhow::{Context, Result};
use ratatui::prelude::*;
use unicode_width::UnicodeWidthChar;

use crate::document::{DocNode, Document};
use crate::markdown::{MarkdownLinkRange, render_markdown_line};
use crate::selection::index::SelectionIndex;
use crate::selection::model::{NavOutcome, SelectionAnchor, SelectionUnit};

/// Owns the parsed source and the derived views used by app input, rendering,
/// and output. Initial migration keeps most coordinate logic at call sites;
/// later passes should move those conversions behind methods here.
#[derive(Debug)]
pub(crate) struct DocumentView {
    document: Document,
    source_lines: Vec<String>,
    rendered_nodes: Vec<RenderedNode>,
    selection_index: SelectionIndex,
    visible_rows: Vec<Option<VisibleRowMap>>,
}

impl DocumentView {
    pub(crate) fn parse(source: &str) -> Result<Self> {
        let source_lines: Vec<String> = source.lines().map(ToOwned::to_owned).collect();
        let document = Document::parse(source).context("failed to parse markdown document")?;
        let rendered_nodes = build_rendered_nodes(&document, &source_lines);
        let selection_index = SelectionIndex::build(&document, &source_lines);

        Ok(Self {
            document,
            source_lines,
            rendered_nodes,
            selection_index,
            visible_rows: Vec::new(),
        })
    }

    #[cfg(test)]
    pub(crate) const fn document(&self) -> &Document {
        &self.document
    }

    #[cfg(test)]
    pub(crate) fn rendered_nodes(&self) -> &[RenderedNode] {
        &self.rendered_nodes
    }

    pub(crate) fn node_count(&self) -> usize {
        self.document.node_count()
    }

    pub(crate) fn rendered_node(&self, node_idx: usize) -> Option<&RenderedNode> {
        self.rendered_nodes.get(node_idx)
    }

    #[cfg(test)]
    pub(crate) fn visible_rows(&self) -> &[Option<VisibleRowMap>] {
        &self.visible_rows
    }

    pub(crate) fn clear_visible_rows(&mut self) {
        self.visible_rows.clear();
    }

    pub(crate) fn push_visible_row(
        &mut self,
        node_idx: usize,
        byte_range: Range<usize>,
        gutter_cols: u16,
    ) {
        self.visible_rows.push(Some(VisibleRowMap {
            node_idx,
            byte_range,
            gutter_cols,
        }));
    }

    pub(crate) fn is_block_start(&self, node_idx: usize) -> bool {
        self.document.is_block_start(node_idx)
    }

    pub(crate) fn next_content_node(&self, from: usize) -> Option<usize> {
        self.document.next_content_node(from)
    }

    pub(crate) fn prev_content_node(&self, before: usize) -> Option<usize> {
        self.document.prev_content_node(before)
    }

    pub(crate) fn navigate(&self, anchor: SelectionAnchor, forward: bool) -> NavOutcome {
        if forward {
            crate::selection::navigator::next(&self.selection_index, anchor)
        } else {
            crate::selection::navigator::prev(&self.selection_index, anchor)
        }
    }

    pub(crate) fn clamp_anchor(
        &self,
        anchor: SelectionAnchor,
        target: SelectionUnit,
    ) -> SelectionAnchor {
        crate::selection::navigator::clamp(&self.selection_index, anchor, target)
    }

    pub(crate) fn has_any_anchor(&self, unit: SelectionUnit) -> bool {
        match unit {
            SelectionUnit::Section => !self.selection_index.sections.is_empty(),
            SelectionUnit::Paragraph => !self.selection_index.paragraphs.is_empty(),
            SelectionUnit::Line => !self.selection_index.lines.is_empty(),
            SelectionUnit::Sentence => !self.selection_index.sentences.is_empty(),
            SelectionUnit::Word => !self.selection_index.words.is_empty(),
        }
    }

    pub(crate) fn section_span_for_start(&self, node_idx: usize) -> Range<usize> {
        let end = self
            .selection_index
            .sections
            .iter()
            .find(|s| s.start_node_idx == node_idx)
            .map_or_else(|| self.node_count(), |s| s.end_node_idx + 1);
        node_idx..end
    }

    pub(crate) fn sentence_count_for_node(&self, node_idx: usize) -> usize {
        self.rendered_nodes
            .get(node_idx)
            .map_or(0, |rn| rn.sentence_ranges.len())
    }

    /// Find every search hit across rendered nodes as `(node, sentence)` pairs.
    /// Smart-case: case-sensitive iff the query contains an ASCII uppercase letter.
    pub(crate) fn search_matches(&self, query: &str) -> Vec<(usize, usize)> {
        if query.is_empty() {
            return Vec::new();
        }
        let case_sensitive = query.chars().any(|c| c.is_ascii_uppercase());
        let needle = if case_sensitive {
            query.to_owned()
        } else {
            let mut s = query.to_owned();
            s.make_ascii_lowercase();
            s
        };
        let mut matches: Vec<(usize, usize)> = Vec::new();
        for (ni, rn) in self.rendered_nodes.iter().enumerate() {
            let mut hay = rn.plain.clone();
            if !case_sensitive {
                hay.make_ascii_lowercase();
            }
            let mut cursor = 0usize;
            while cursor <= hay.len() {
                let Some(offset) = hay[cursor..].find(&needle) else {
                    break;
                };
                let abs = cursor + offset;
                let sentence_idx = rn
                    .sentence_ranges
                    .iter()
                    .position(|r| abs >= r.start && abs < r.end)
                    .unwrap_or(0);
                matches.push((ni, sentence_idx));
                cursor = abs + needle.len();
            }
        }
        matches
    }

    pub(crate) fn links_for_anchor(&self, anchor: SelectionAnchor) -> Vec<String> {
        let Some(rn) = self.rendered_nodes.get(anchor.node_idx) else {
            return Vec::new();
        };
        let scope: Option<Range<usize>> = if anchor.unit == SelectionUnit::Sentence {
            rn.sentence_ranges.get(anchor.unit_idx).cloned()
        } else {
            None
        };
        let mut urls = Vec::new();
        for link in &rn.links {
            let overlaps = scope
                .as_ref()
                .is_none_or(|r| link.end > r.start && link.start < r.end);
            if overlaps && !urls.iter().any(|u: &String| u == &link.url) {
                urls.push(link.url.clone());
            }
        }
        urls
    }

    /// Map a selection unit on `node_idx` to bytes in the rendered display
    /// plain text. Section returns None because section paint covers whole
    /// nodes in the caller.
    pub(crate) fn display_range_for_unit(
        &self,
        node_idx: usize,
        unit: SelectionUnit,
        unit_idx: usize,
    ) -> Option<Range<usize>> {
        let rn = self.rendered_nodes.get(node_idx)?;
        match unit {
            SelectionUnit::Sentence => rn.sentence_ranges.get(unit_idx).cloned(),
            SelectionUnit::Paragraph => Some(0..rn.plain.len()),
            SelectionUnit::Line => rn.line_ranges.get(unit_idx).cloned(),
            SelectionUnit::Word => {
                let index_node = self.selection_index.nodes.get(node_idx)?;
                let word_range = index_node.word_ranges.get(unit_idx)?;
                let word_text = index_node.selection_plain_text.get(word_range.clone())?;
                let occurrence = count_occurrences_before(
                    &index_node.selection_plain_text,
                    word_text,
                    word_range.start,
                );
                let pos = nth_occurrence(&rn.plain, word_text, occurrence)?;
                Some(pos..pos + word_text.len())
            }
            SelectionUnit::Section => None,
        }
    }

    pub(crate) fn selection_range_for_unit(
        &self,
        node_idx: usize,
        unit: SelectionUnit,
        unit_idx: usize,
    ) -> Option<Range<usize>> {
        let node = self.selection_index.nodes.get(node_idx)?;
        match unit {
            SelectionUnit::Word => node.word_ranges.get(unit_idx).cloned(),
            SelectionUnit::Sentence => node.sentence_ranges.get(unit_idx).cloned(),
            SelectionUnit::Line => node
                .source_line_ranges
                .get(unit_idx)
                .map(|(_, r)| r.clone()),
            SelectionUnit::Paragraph => Some(0..node.selection_plain_text.len()),
            SelectionUnit::Section => None,
        }
    }

    pub(crate) fn code_line_local_range(
        &self,
        node_idx: usize,
        source_line: usize,
        range: Range<usize>,
    ) -> Option<Range<usize>> {
        let node = self.selection_index.nodes.get(node_idx)?;
        let (_, line_range) = node
            .source_line_ranges
            .iter()
            .find(|(line, _)| *line == source_line)?;
        if range.end <= line_range.start || range.start >= line_range.end {
            return None;
        }
        let start = range.start.max(line_range.start) - line_range.start;
        let end = range.end.min(line_range.end) - line_range.start;
        if end <= start {
            return None;
        }
        Some(start..end)
    }

    pub(crate) fn source_line_for_anchor(&self, anchor: SelectionAnchor) -> usize {
        let node_first_line = self
            .document
            .nodes
            .get(anchor.node_idx)
            .map_or(0, |n| n.source_start_line());
        self.where_for_annotation(
            anchor.unit,
            anchor.node_idx,
            Some(anchor.unit_idx),
            node_first_line,
        )
    }

    pub(crate) fn sentence_context(&self, anchor: SelectionAnchor) -> Option<(usize, String)> {
        let unit_idx = anchor.unit_idx;
        let node = self.selection_index.nodes.get(anchor.node_idx)?;
        let range = node.sentence_ranges.get(unit_idx)?;
        let text = node
            .selection_plain_text
            .get(range.clone())?
            .trim()
            .to_string();
        Some((unit_idx, text))
    }

    pub(crate) fn target_capture(&self, anchor: SelectionAnchor) -> Option<(usize, String)> {
        match anchor.unit {
            SelectionUnit::Line => self.line_capture(anchor.node_idx, anchor.unit_idx),
            SelectionUnit::Word => self.word_capture(anchor.node_idx, anchor.unit_idx),
            SelectionUnit::Paragraph => self.paragraph_capture(anchor.node_idx),
            SelectionUnit::Section => self.section_capture(anchor.node_idx),
            SelectionUnit::Sentence => self.sentence_context(anchor),
        }
    }

    pub(crate) fn target_text_for_unit(
        &self,
        node_idx: usize,
        unit: SelectionUnit,
        unit_idx: usize,
    ) -> Option<String> {
        let node = self.selection_index.nodes.get(node_idx)?;
        match unit {
            SelectionUnit::Sentence => {
                let r = node.sentence_ranges.get(unit_idx)?;
                Some(node.selection_plain_text.get(r.clone())?.trim().to_string())
            }
            SelectionUnit::Word => {
                let r = node.word_ranges.get(unit_idx)?;
                Some(node.selection_plain_text.get(r.clone())?.to_string())
            }
            SelectionUnit::Line => {
                if let DocNode::ListItem { .. } = self.document.nodes.get(node_idx)? {
                    Some(node.selection_plain_text.clone())
                } else {
                    let (line, _) = node.source_line_ranges.get(unit_idx)?.clone();
                    Some(self.source_lines.get(line)?.clone())
                }
            }
            SelectionUnit::Paragraph => Some(node.selection_plain_text.replace('\n', " ")),
            SelectionUnit::Section => {
                let section = self
                    .selection_index
                    .sections
                    .iter()
                    .find(|s| s.start_node_idx == node_idx)?;
                let mut parts: Vec<String> = Vec::new();
                for i in section.start_node_idx..=section.end_node_idx {
                    if let Some(n) = self.selection_index.nodes.get(i)
                        && !n.selection_plain_text.is_empty()
                    {
                        parts.push(n.selection_plain_text.replace('\n', " "));
                    }
                }
                Some(parts.join(" "))
            }
        }
    }

    pub(crate) fn node_line_context(&self, node_idx: usize) -> SourceLineContext {
        let source_line = self
            .document
            .nodes
            .get(node_idx)
            .map_or(0, |n| n.source_start_line());
        let line_text = self
            .source_lines
            .get(source_line)
            .cloned()
            .unwrap_or_default();
        let previous_line = source_line
            .checked_sub(1)
            .and_then(|i| self.source_lines.get(i))
            .cloned();
        let next_line = self.source_lines.get(source_line + 1).cloned();

        SourceLineContext {
            source_line,
            line_text,
            previous_line,
            next_line,
        }
    }

    pub(crate) fn where_for_annotation(
        &self,
        target_unit: SelectionUnit,
        node_idx: usize,
        sentence_index: Option<usize>,
        node_first_line: usize,
    ) -> usize {
        match target_unit {
            SelectionUnit::Line => {
                let unit_idx = sentence_index.unwrap_or(0);
                self.selection_index
                    .nodes
                    .get(node_idx)
                    .and_then(|n| n.source_line_ranges.get(unit_idx).map(|p| p.0))
                    .unwrap_or(node_first_line)
            }
            SelectionUnit::Word => {
                let unit_idx = sentence_index.unwrap_or(0);
                self.word_source_line(node_idx, unit_idx)
                    .unwrap_or(node_first_line)
            }
            SelectionUnit::Sentence => sentence_index
                .and_then(|si| {
                    let rn = self.rendered_nodes.get(node_idx)?;
                    let r = rn.sentence_ranges.get(si)?;
                    Some(node_first_line + newlines_before_byte(&rn.plain, r.start))
                })
                .unwrap_or(node_first_line),
            SelectionUnit::Paragraph | SelectionUnit::Section => node_first_line,
        }
    }

    pub(crate) fn neighboring_source_lines(&self, source_line: usize) -> (&str, &str) {
        let prev = source_line
            .checked_sub(1)
            .and_then(|i| self.source_lines.get(i))
            .map_or("", String::as_str);
        let next = self
            .source_lines
            .get(source_line + 1)
            .map_or("", String::as_str);
        (prev, next)
    }

    pub(crate) fn code_block_render_lines(
        &self,
        node_idx: usize,
    ) -> Option<Vec<CodeBlockRenderLine<'_>>> {
        let DocNode::CodeBlock {
            source_lines: range,
            ..
        } = self.document.nodes.get(node_idx)?
        else {
            return None;
        };
        let range = clamp_range(range, self.source_lines.len());
        let mut cursor = 0usize;
        let mut rows = Vec::with_capacity(range.len());
        for (offset, line) in self.source_lines[range.clone()].iter().enumerate() {
            let end = cursor + line.len();
            rows.push(CodeBlockRenderLine {
                source_line: range.start + offset,
                text: line.as_str(),
                byte_range: cursor..end,
                is_fence: line.trim_start().starts_with("```"),
            });
            cursor = end + 1;
        }
        Some(rows)
    }

    pub(crate) fn selection_anchor_for_row_click(
        &self,
        node_idx: usize,
        byte_range: Range<usize>,
        col_in_text: usize,
        click_count: u8,
    ) -> Option<SelectionAnchor> {
        if byte_range.start >= byte_range.end {
            return None;
        }
        let plain = self.rendered_nodes.get(node_idx)?.plain.as_str();
        let row_text = plain.get(byte_range.clone())?;
        let local_byte = col_to_byte(row_text, col_in_text);
        let display_byte = byte_range.start + local_byte;
        let (unit, unit_idx) = self.click_target_unit(node_idx, display_byte, click_count);
        Some(SelectionAnchor::new(node_idx, unit, unit_idx))
    }

    /// Resolve a terminal mouse coordinate against the current visible row map.
    /// Returns None for clicks outside `list_inner`, missing rows, spacer rows,
    /// or non-text cells to the left of the row content.
    pub(crate) fn hit_test(
        &self,
        list_inner: Rect,
        row: u16,
        col: u16,
        click_count: u8,
    ) -> Option<SelectionAnchor> {
        if row < list_inner.y
            || row >= list_inner.y.saturating_add(list_inner.height)
            || col < list_inner.x
            || col >= list_inner.x.saturating_add(list_inner.width)
        {
            return None;
        }
        let visual_row = (row - list_inner.y) as usize;
        let map = self.visible_rows.get(visual_row)?.as_ref()?;
        let col_in_text = (col - list_inner.x).saturating_sub(map.gutter_cols) as usize;
        self.selection_anchor_for_row_click(
            map.node_idx,
            map.byte_range.clone(),
            col_in_text,
            click_count,
        )
    }

    fn paragraph_capture(&self, node_idx: usize) -> Option<(usize, String)> {
        let plain = self
            .selection_index
            .nodes
            .get(node_idx)
            .map(|n| n.selection_plain_text.clone())?;
        Some((0, plain.replace('\n', " ")))
    }

    fn section_capture(&self, node_idx: usize) -> Option<(usize, String)> {
        let section = self
            .selection_index
            .sections
            .iter()
            .find(|s| s.start_node_idx == node_idx)?;
        let mut parts: Vec<String> = Vec::new();
        for i in section.start_node_idx..=section.end_node_idx {
            if let Some(n) = self.selection_index.nodes.get(i)
                && !n.selection_plain_text.is_empty()
            {
                parts.push(n.selection_plain_text.replace('\n', " "));
            }
        }
        Some((0, parts.join(" ")))
    }

    fn word_capture(&self, node_idx: usize, unit_idx: usize) -> Option<(usize, String)> {
        let node = self.selection_index.nodes.get(node_idx)?;
        let range = node.word_ranges.get(unit_idx)?;
        let text = node.selection_plain_text.get(range.clone())?.to_string();
        Some((unit_idx, text))
    }

    fn line_capture(&self, node_idx: usize, unit_idx: usize) -> Option<(usize, String)> {
        if let DocNode::ListItem { .. } = self.document.nodes.get(node_idx)? {
            let plain = self
                .selection_index
                .nodes
                .get(node_idx)
                .map(|n| n.selection_plain_text.clone())?;
            Some((unit_idx, plain))
        } else {
            let (line, _) = self
                .selection_index
                .nodes
                .get(node_idx)?
                .source_line_ranges
                .get(unit_idx)?
                .clone();
            let line_text = self.source_lines.get(line)?.clone();
            Some((unit_idx, line_text))
        }
    }

    fn word_source_line(&self, node_idx: usize, word_idx: usize) -> Option<usize> {
        let index_node = self.selection_index.nodes.get(node_idx)?;
        let word_range = index_node.word_ranges.get(word_idx)?;
        let word_text = index_node.selection_plain_text.get(word_range.clone())?;
        let first_line = index_node.source_line_ranges.first().map_or_else(
            || {
                self.document
                    .nodes
                    .get(node_idx)
                    .map_or(0, |n| n.source_start_line())
            },
            |(l, _)| *l,
        );
        let rn = self.rendered_nodes.get(node_idx)?;
        let occurrence = count_occurrences_before(
            &index_node.selection_plain_text,
            word_text,
            word_range.start,
        );
        let pos = nth_occurrence(&rn.plain, word_text, occurrence).unwrap_or(0);
        Some(first_line + newlines_before_byte(&rn.plain, pos))
    }

    fn click_target_unit(
        &self,
        node_idx: usize,
        display_byte: usize,
        count: u8,
    ) -> (SelectionUnit, usize) {
        match count {
            1 => {
                let idx = self.find_word_at(node_idx, display_byte).unwrap_or(0);
                (SelectionUnit::Word, idx)
            }
            2 => {
                if self.node_has_sentence_semantics(node_idx) {
                    let idx = self.find_sentence_at(node_idx, display_byte).unwrap_or(0);
                    (SelectionUnit::Sentence, idx)
                } else {
                    let idx = self.find_line_at(node_idx, display_byte).unwrap_or(0);
                    (SelectionUnit::Line, idx)
                }
            }
            _ => (SelectionUnit::Paragraph, 0),
        }
    }

    fn find_word_at(&self, node_idx: usize, display_byte: usize) -> Option<usize> {
        let rn = self.rendered_nodes.get(node_idx)?;
        find_unit_at(&rn.display_word_ranges, display_byte)
    }

    fn find_sentence_at(&self, node_idx: usize, display_byte: usize) -> Option<usize> {
        let rn = self.rendered_nodes.get(node_idx)?;
        find_unit_at(&rn.sentence_ranges, display_byte)
    }

    fn find_line_at(&self, node_idx: usize, display_byte: usize) -> Option<usize> {
        let rn = self.rendered_nodes.get(node_idx)?;
        find_unit_at(&rn.line_ranges, display_byte)
    }

    fn node_has_sentence_semantics(&self, node_idx: usize) -> bool {
        let Some(rn) = self.rendered_nodes.get(node_idx) else {
            return false;
        };
        if rn.sentence_ranges.is_empty() {
            return false;
        }
        match self.document.nodes.get(node_idx) {
            Some(DocNode::CodeBlock { .. }) => false,
            Some(DocNode::Heading { .. }) | Some(DocNode::ListItem { .. }) => {
                rn.plain.chars().any(|c| matches!(c, '.' | '!' | '?'))
            }
            _ => true,
        }
    }
}

/// Maps a single visible terminal row to a slice of one rendered node's display
/// plain text. Built each `draw()` from node rows so `handle_mouse` can resolve
/// click coordinates without re-walking the wrap pipeline.
#[derive(Debug, Clone)]
pub(crate) struct VisibleRowMap {
    pub(crate) node_idx: usize,
    /// Byte range in `rendered_nodes[node_idx].plain` covering the chars shown
    /// on this row after the gutter prefix. Zero-width for spacer rows.
    pub(crate) byte_range: Range<usize>,
    /// Terminal columns to skip from the left edge of `list_inner` before text.
    pub(crate) gutter_cols: u16,
}

#[derive(Debug, Clone)]
pub(crate) struct SourceLineContext {
    pub(crate) source_line: usize,
    pub(crate) line_text: String,
    pub(crate) previous_line: Option<String>,
    pub(crate) next_line: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct CodeBlockRenderLine<'a> {
    pub(crate) source_line: usize,
    pub(crate) text: &'a str,
    pub(crate) byte_range: Range<usize>,
    pub(crate) is_fence: bool,
}

/// Per-node rendering cache: styled spans for the joined source text plus
/// sentence byte-range boundaries within `plain`.
#[derive(Clone)]
pub(crate) struct RenderedNode {
    pub(crate) plain: String,
    pub(crate) spans: Vec<Span<'static>>,
    pub(crate) sentence_ranges: Vec<Range<usize>>,
    /// Byte ranges in `plain` for each line as the Line projection sees
    /// it. Aligned 1:1 with `SelectionIndex.nodes[i].source_line_ranges`.
    /// Empty when the node has no per-line breakdown (e.g. ThematicBreak).
    /// Populated at render time because pulldown-cmark applies
    /// smart-punctuation and emphasis-marker stripping, so a source line
    /// can't be searched verbatim inside display plain.
    pub(crate) line_ranges: Vec<Range<usize>>,
    /// Word byte ranges in display `plain` (re-segmented from display).
    /// Used by mouse-click resolution to map a clicked display byte to a
    /// word_idx; the existing index word ranges are in selection plain
    /// (markers stripped, smart-punctuation NOT applied), so their byte
    /// values don't index into display plain.
    pub(crate) display_word_ranges: Vec<Range<usize>>,
    pub(crate) links: Vec<MarkdownLinkRange>,
}

impl std::fmt::Debug for RenderedNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderedNode")
            .field("plain", &self.plain)
            .field("sentence_ranges", &self.sentence_ranges)
            .field("line_ranges", &self.line_ranges)
            .field("display_word_ranges", &self.display_word_ranges)
            .finish_non_exhaustive()
    }
}

fn build_rendered_nodes(doc: &Document, source_lines: &[String]) -> Vec<RenderedNode> {
    doc.nodes
        .iter()
        .map(|n| build_rendered_node(n, source_lines))
        .collect()
}

fn build_rendered_node(node: &DocNode, source_lines: &[String]) -> RenderedNode {
    let mut rn = match node {
        DocNode::Heading { source_line, .. } => {
            let text = source_lines.get(*source_line).cloned().unwrap_or_default();
            let r = render_markdown_line(&text);
            let ranges = single_range(&r.plain);
            let line_ranges = ranges.clone();
            RenderedNode {
                plain: r.plain,
                spans: r.spans,
                sentence_ranges: ranges,
                line_ranges,
                display_word_ranges: Vec::new(),
                links: r.links,
            }
        }
        DocNode::Paragraph {
            source_lines: range,
            ..
        } => {
            let src = &source_lines[clamp_range(range, source_lines.len())];
            let (plain, spans, links, line_ranges) = render_source_lines_with_breaks(src);
            if plain.is_empty() {
                let joined = join_node_source_lines(src);
                let r = render_markdown_line(&joined);
                let sentence_ranges = single_range(&r.plain);
                let line_ranges = sentence_ranges.clone();
                RenderedNode {
                    plain: r.plain,
                    spans: r.spans,
                    sentence_ranges,
                    line_ranges,
                    display_word_ranges: Vec::new(),
                    links: r.links,
                }
            } else {
                let sentence_ranges = crate::selection::segment::segment_sentences(&plain);
                RenderedNode {
                    plain,
                    spans,
                    sentence_ranges,
                    line_ranges,
                    display_word_ranges: Vec::new(),
                    links,
                }
            }
        }
        DocNode::ListItem {
            source_lines: range,
            ordered,
            depth,
            ..
        } => {
            let joined =
                join_node_source_lines(&source_lines[clamp_range(range, source_lines.len())]);
            let r = render_markdown_line(&joined);
            let ranges = single_range(&r.plain);
            let line_ranges = ranges.clone();
            // Top-level ordered items act as section headings - style them like one.
            let spans = if *ordered && *depth == 0 {
                let style = Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD);
                vec![Span::styled(r.plain.clone(), style)]
            } else {
                r.spans
            };
            RenderedNode {
                plain: r.plain,
                spans,
                sentence_ranges: ranges,
                line_ranges,
                display_word_ranges: Vec::new(),
                links: r.links,
            }
        }
        DocNode::CodeBlock {
            source_lines: range,
            ..
        } => {
            let raw = &source_lines[clamp_range(range, source_lines.len())];
            let plain = raw.join("\n");
            let ranges = single_range(&plain);
            let mut line_ranges = Vec::new();
            let mut cursor = 0usize;
            for line in raw {
                let is_fence = line.trim_start().starts_with("```");
                let end = cursor + line.len();
                if !is_fence {
                    line_ranges.push(cursor..end);
                }
                cursor = end + 1;
            }
            let spans = raw
                .iter()
                .flat_map(|line| {
                    let style = if line.trim_start().starts_with("```") {
                        Style::default().fg(Color::DarkGray)
                    } else {
                        Style::default().fg(Color::White).bg(Color::DarkGray)
                    };
                    [
                        Span::styled(line.clone(), style),
                        Span::raw("\n".to_owned()),
                    ]
                })
                .collect();
            RenderedNode {
                plain,
                spans,
                sentence_ranges: ranges,
                line_ranges,
                display_word_ranges: Vec::new(),
                links: vec![],
            }
        }
        DocNode::ThematicBreak { .. } => {
            let r = render_markdown_line("---");
            RenderedNode {
                plain: r.plain,
                spans: r.spans,
                sentence_ranges: vec![],
                line_ranges: vec![],
                display_word_ranges: Vec::new(),
                links: r.links,
            }
        }
    };
    rn.display_word_ranges = crate::selection::segment::segment_words(&rn.plain);
    rn
}

#[allow(clippy::single_range_in_vec_init)]
fn single_range(s: &str) -> Vec<Range<usize>> {
    if s.is_empty() {
        vec![]
    } else {
        vec![0..s.len()]
    }
}

fn join_node_source_lines(lines: &[String]) -> String {
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| if i == 0 { l.as_str() } else { l.trim() })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_source_lines_with_breaks(
    src_lines: &[String],
) -> (
    String,
    Vec<Span<'static>>,
    Vec<MarkdownLinkRange>,
    Vec<Range<usize>>,
) {
    let mut plain = String::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut links: Vec<MarkdownLinkRange> = Vec::new();
    let mut line_ranges: Vec<Range<usize>> = Vec::new();
    let first_indent = src_lines
        .first()
        .map_or(0, |l| l.len() - l.trim_start().len());
    for (i, line) in src_lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !plain.is_empty() {
            plain.push('\n');
            spans.push(Span::raw("\n".to_owned()));
        }
        let relative_indent = if i == 0 {
            0
        } else {
            let line_indent = line.len() - line.trim_start().len();
            line_indent.saturating_sub(first_indent)
        };
        if relative_indent > 0 {
            let prefix = " ".repeat(relative_indent);
            plain.push_str(&prefix);
            spans.push(Span::raw(prefix));
        }
        let line_start = plain.len() - relative_indent;
        let offset = plain.len();
        let r = render_markdown_line(trimmed);
        for link in r.links {
            links.push(MarkdownLinkRange {
                start: link.start + offset,
                end: link.end + offset,
                url: link.url,
            });
        }
        plain.push_str(&r.plain);
        spans.extend(r.spans);
        line_ranges.push(line_start..plain.len());
    }
    (plain, spans, links, line_ranges)
}

pub(crate) fn clamp_range(r: &Range<usize>, len: usize) -> Range<usize> {
    r.start.min(len)..r.end.min(len)
}

/// Count `\n` bytes in `plain[..byte]` - the number of source lines the
/// byte position is past inside the rendered display plain text.
pub(crate) fn newlines_before_byte(plain: &str, byte: usize) -> usize {
    plain
        .get(..byte)
        .map_or(0, |p| p.bytes().filter(|&b| b == b'\n').count())
}

/// Count occurrences of `needle` in `haystack[..before_byte]`.
pub(crate) fn count_occurrences_before(haystack: &str, needle: &str, before_byte: usize) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let stop = before_byte.min(haystack.len());
    let mut count = 0usize;
    let mut cursor = 0usize;
    while cursor < stop {
        match haystack[cursor..stop].find(needle) {
            Some(off) => {
                count += 1;
                cursor += off + needle.len();
            }
            None => break,
        }
    }
    count
}

/// Return the byte offset of the `n`th occurrence (0-indexed) of `needle`.
pub(crate) fn nth_occurrence(haystack: &str, needle: &str, n: usize) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let mut cursor = 0usize;
    for i in 0..=n {
        let off = haystack[cursor..].find(needle)?;
        let abs = cursor + off;
        if i == n {
            return Some(abs);
        }
        cursor = abs + needle.len();
    }
    None
}

/// Pick the unit_idx whose range contains `byte`, or when `byte` falls in a
/// gap, the closest preceding unit.
fn find_unit_at(ranges: &[Range<usize>], byte: usize) -> Option<usize> {
    if ranges.is_empty() {
        return None;
    }
    let count = ranges.partition_point(|r| r.start <= byte);
    Some(count.saturating_sub(1))
}

/// Walk `text` from byte 0 and return the byte index whose preceding chars sum
/// to strictly more than `target_cols` terminal columns.
fn col_to_byte(text: &str, target_cols: usize) -> usize {
    let mut used = 0usize;
    for (idx, ch) in text.char_indices() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w == 0 {
            continue;
        }
        if used >= target_cols {
            return idx;
        }
        used += w;
    }
    text.len()
}
