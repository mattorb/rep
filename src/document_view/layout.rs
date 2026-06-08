use super::*;

impl DocumentView {
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
