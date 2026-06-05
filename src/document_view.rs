use std::ops::Range;

use anyhow::{Context, Result};
use ratatui::prelude::*;

use crate::document::{DocNode, Document};
use crate::selection::index::SelectionIndex;
use crate::selection::model::{NavOutcome, SelectionAnchor, SelectionUnit};
use crate::ui::wrap_styled_spans;

mod rendered;
mod types;
pub(crate) use rendered::RenderedNode;
use rendered::{
    build_rendered_nodes, clamp_range, col_to_byte, count_occurrences_before, find_unit_at,
    newlines_before_byte, nth_occurrence, wrap_line_byte_ranges,
};
pub(crate) use types::{
    AnnotationTargetCapture, CodeBlockDisplayRow, CodeBlockStyleRequest, DisplaySpanStyleRequest,
    SourceActionContext, SourceLineContext, VisibleRowMap, WrappedDisplayRow,
};
use types::{CodeBlockLineStyleRequest, CodeBlockRenderLine};

/// Owns the parsed source and the derived views used by app input, rendering,
/// hit testing, and output context.
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

    #[cfg(test)]
    pub(crate) fn visible_rows(&self) -> &[Option<VisibleRowMap>] {
        &self.visible_rows
    }

    pub(crate) fn set_visible_rows(
        &mut self,
        rows: impl IntoIterator<Item = (usize, Range<usize>)>,
        gutter_cols: u16,
    ) {
        self.visible_rows = rows
            .into_iter()
            .map(|(node_idx, byte_range)| {
                Some(VisibleRowMap {
                    node_idx,
                    byte_range,
                    gutter_cols,
                })
            })
            .collect();
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
    fn display_range_for_unit(
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

    pub(crate) fn styled_display_spans(
        &self,
        request: DisplaySpanStyleRequest<'_>,
    ) -> Option<Vec<Span<'static>>> {
        let rn = self.rendered_nodes.get(request.node_idx)?;
        let plain = rn.plain.as_str();
        let plain_len = plain.len();

        if plain.is_empty() {
            return Some(vec![Span::styled(
                " ",
                Style::default().add_modifier(Modifier::DIM),
            )]);
        }

        let mut segments: Vec<(usize, usize, Style)> = Vec::new();
        let mut offset = 0usize;
        for span in &rn.spans {
            let len = span.content.len();
            if len == 0 {
                continue;
            }
            let end = (offset + len).min(plain_len);
            if offset < end {
                segments.push((offset, end, span.style));
            }
            offset = end;
        }
        if segments.is_empty() {
            segments.push((0, plain_len, Style::default()));
        }

        let highlight = if request.section_highlight_active {
            Some(0..plain_len)
        } else if request.active_anchor.node_idx == request.node_idx {
            self.display_range_for_unit(
                request.node_idx,
                request.active_anchor.unit,
                request.active_anchor.unit_idx,
            )
        } else {
            None
        };

        let strike_ranges: Vec<Range<usize>> = request
            .strike_units
            .iter()
            .filter_map(|&(unit, idx)| self.display_range_for_unit(request.node_idx, unit, idx))
            .collect();

        let mut bounds = vec![0, plain_len];
        for &(start, end, _) in &segments {
            bounds.push(start);
            bounds.push(end);
        }
        for range in &rn.sentence_ranges {
            bounds.push(range.start.min(plain_len));
            bounds.push(range.end.min(plain_len));
        }
        if let Some(range) = &highlight {
            bounds.push(range.start.min(plain_len));
            bounds.push(range.end.min(plain_len));
        }
        for range in &strike_ranges {
            bounds.push(range.start.min(plain_len));
            bounds.push(range.end.min(plain_len));
        }
        bounds.sort_unstable();
        bounds.dedup();

        let mut spans = Vec::new();
        for pair in bounds.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            if start >= end {
                continue;
            }
            let Some(text) = plain.get(start..end) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }

            let mut style = segments
                .iter()
                .find(|&&(segment_start, segment_end, _)| {
                    start >= segment_start && start < segment_end
                })
                .map(|&(_, _, style)| style)
                .unwrap_or_default();

            if highlight
                .as_ref()
                .is_some_and(|range| start < range.end && end > range.start)
            {
                style = style.patch(
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                );
            }

            if strike_ranges
                .iter()
                .any(|range| start < range.end && end > range.start)
            {
                style = style.patch(
                    Style::default()
                        .fg(Color::Red)
                        .add_modifier(Modifier::CROSSED_OUT | Modifier::DIM),
                );
            }

            spans.push(Span::styled(text.to_string(), style));
        }

        if spans.is_empty() {
            spans.push(Span::raw(plain.to_string()));
        }
        Some(spans)
    }

    pub(crate) fn wrapped_display_rows(
        &self,
        node_idx: usize,
        spans: Vec<Span<'static>>,
        width: usize,
    ) -> Vec<WrappedDisplayRow> {
        let wrapped = wrap_styled_spans(spans, width);
        let plain = self
            .rendered_nodes
            .get(node_idx)
            .map_or("", |rn| rn.plain.as_str());
        let byte_ranges = wrap_line_byte_ranges(plain, &wrapped);
        wrapped
            .into_iter()
            .zip(byte_ranges)
            .map(|(spans, byte_range)| WrappedDisplayRow { spans, byte_range })
            .collect()
    }

    pub(crate) fn styled_code_block_rows(
        &self,
        request: CodeBlockStyleRequest<'_>,
    ) -> Option<Vec<CodeBlockDisplayRow>> {
        let rows = self.code_block_render_lines(request.node_idx)?;
        Some(
            rows.into_iter()
                .map(|row| {
                    let base_style = if row.is_fence {
                        Style::default().fg(Color::DarkGray)
                    } else {
                        Style::default().fg(Color::White).bg(Color::DarkGray)
                    };
                    let spans = self.styled_code_block_line_spans(CodeBlockLineStyleRequest {
                        node_idx: request.node_idx,
                        source_line: row.source_line,
                        line: row.text,
                        base_style,
                        active_anchor: request.active_anchor,
                        section_highlight_active: request.section_highlight_active,
                        strike_units: request.strike_units,
                    });
                    CodeBlockDisplayRow {
                        spans,
                        byte_range: row.byte_range,
                    }
                })
                .collect(),
        )
    }

    fn styled_code_block_line_spans(
        &self,
        request: CodeBlockLineStyleRequest<'_>,
    ) -> Vec<Span<'static>> {
        let highlight_local = if request.section_highlight_active {
            Some(0..request.line.len())
        } else if request.active_anchor.node_idx == request.node_idx {
            self.selection_range_for_unit(
                request.node_idx,
                request.active_anchor.unit,
                request.active_anchor.unit_idx,
            )
            .and_then(|range| {
                self.code_line_local_range(request.node_idx, request.source_line, range)
            })
        } else {
            None
        };

        let strike_local: Vec<Range<usize>> = request
            .strike_units
            .iter()
            .filter_map(|&(unit, idx)| {
                self.selection_range_for_unit(request.node_idx, unit, idx)
                    .and_then(|range| {
                        self.code_line_local_range(request.node_idx, request.source_line, range)
                    })
            })
            .collect();

        let mut bounds = vec![0, request.line.len()];
        if let Some(range) = &highlight_local {
            bounds.push(range.start);
            bounds.push(range.end);
        }
        for range in &strike_local {
            bounds.push(range.start);
            bounds.push(range.end);
        }
        bounds.sort_unstable();
        bounds.dedup();

        let mut spans = Vec::new();
        for pair in bounds.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            if start >= end {
                continue;
            }
            let Some(text) = request.line.get(start..end) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }

            let mut style = request.base_style;
            if highlight_local
                .as_ref()
                .is_some_and(|range| start < range.end && end > range.start)
            {
                style = style.patch(
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                );
            }
            if strike_local
                .iter()
                .any(|range| start < range.end && end > range.start)
            {
                style = style.patch(
                    Style::default()
                        .fg(Color::Red)
                        .add_modifier(Modifier::CROSSED_OUT | Modifier::DIM),
                );
            }
            spans.push(Span::styled(text.to_string(), style));
        }

        if spans.is_empty() {
            spans.push(Span::styled(request.line.to_string(), request.base_style));
        }
        spans
    }

    fn selection_range_for_unit(
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

    fn code_line_local_range(
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

    fn source_line_for_anchor(&self, anchor: SelectionAnchor) -> usize {
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

    fn sentence_context(&self, anchor: SelectionAnchor) -> Option<(usize, String)> {
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

    pub(crate) fn sentence_index_for_anchor(&self, anchor: SelectionAnchor) -> Option<usize> {
        if anchor.unit != SelectionUnit::Sentence {
            return None;
        }
        self.selection_index
            .nodes
            .get(anchor.node_idx)?
            .sentence_ranges
            .get(anchor.unit_idx)?;
        Some(anchor.unit_idx)
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

    pub(crate) fn annotation_target_capture(
        &self,
        anchor: SelectionAnchor,
    ) -> AnnotationTargetCapture {
        let target = self.target_capture(anchor);
        AnnotationTargetCapture {
            sentence_index: target.as_ref().map(|(idx, _)| *idx),
            sentence_text: target.map(|(_, text)| text),
            source_line: self.source_line_for_anchor(anchor),
        }
    }

    pub(crate) fn annotation_action_context(
        &self,
        node_idx: usize,
        target_unit: SelectionUnit,
        unit_idx: Option<usize>,
        target_text: Option<&str>,
    ) -> SourceActionContext {
        self.action_context_for(node_idx, target_unit, unit_idx, target_text)
    }

    pub(crate) fn strike_action_context(
        &self,
        node_idx: usize,
        unit: SelectionUnit,
        unit_idx: usize,
    ) -> (String, SourceActionContext) {
        let target_text = self
            .target_text_for_unit(node_idx, unit, unit_idx)
            .unwrap_or_default();
        let context_target = (!target_text.is_empty()).then_some(target_text.as_str());
        let context = self.action_context_for(node_idx, unit, Some(unit_idx), context_target);
        (target_text, context)
    }

    fn target_text_for_unit(
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

    fn action_context_for(
        &self,
        node_idx: usize,
        target_unit: SelectionUnit,
        unit_idx: Option<usize>,
        target_text: Option<&str>,
    ) -> SourceActionContext {
        let node_first_line = self
            .document
            .nodes
            .get(node_idx)
            .map_or(0, |n| n.source_start_line());
        let line_text = self
            .source_lines
            .get(node_first_line)
            .map_or("", String::as_str);
        let where_line =
            self.where_for_annotation(target_unit, node_idx, unit_idx, node_first_line);
        let (previous_line, next_line) = self.neighboring_source_lines(where_line);

        SourceActionContext {
            where_line,
            target: target_text.unwrap_or(line_text).to_string(),
            previous_line: previous_line.to_string(),
            next_line: next_line.to_string(),
        }
    }

    fn where_for_annotation(
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

    fn neighboring_source_lines(&self, source_line: usize) -> (&str, &str) {
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

    fn code_block_render_lines(&self, node_idx: usize) -> Option<Vec<CodeBlockRenderLine<'_>>> {
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

    fn selection_anchor_for_row_click(
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

#[cfg(test)]
#[path = "document_view_tests.rs"]
mod tests;
