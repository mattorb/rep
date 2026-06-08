use super::*;

impl DocumentView {
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
}
