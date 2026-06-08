use super::*;

impl App {
    pub(super) fn draw_document(&mut self, frame: &mut Frame, area: Rect) -> Rect {
        let line_area_inner_width = area.width.saturating_sub(2) as usize;
        let wrapped_text_width = line_area_inner_width.saturating_sub(GUTTER_WIDTH).max(1);

        let block_title = {
            let state = self.render_state();
            let filename = state
                .source_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("markdown");
            let (change_count, feedback_count, insert_count, strike_count) =
                state.annotation_counts();
            if change_count == 0 && feedback_count == 0 && insert_count == 0 && strike_count == 0 {
                format!(" {filename} ")
            } else {
                let mut parts = Vec::new();
                if change_count > 0 {
                    parts.push(format!("{change_count}C"));
                }
                if feedback_count > 0 {
                    parts.push(format!("{feedback_count}F"));
                }
                if insert_count > 0 {
                    parts.push(format!("{insert_count}I"));
                }
                if strike_count > 0 {
                    parts.push(format!("{strike_count}X"));
                }
                format!(" {filename}  {} ", parts.join(" · "))
            }
        };

        let list_block = Block::default()
            .borders(Borders::ALL)
            .title(block_title)
            .border_style(Style::default().fg(Color::Gray));
        let list_inner = list_block.inner(area);
        self.list_inner = list_inner;

        let (node_rows, node_heights) = {
            let state = self.render_state();
            let node_count = state.view.node_count();
            let mut node_heights: Vec<u16> = Vec::with_capacity(node_count);
            let node_rows: Vec<Vec<RenderedDisplayRow>> = (0..node_count)
                .map(|node_idx| {
                    let (indicator, indicator_style) = Self::node_indicator(&state, node_idx);
                    // Add a blank trailing line when the NEXT node is a block start.
                    // This keeps the spacer at the END of the preceding item so that
                    // navigating to any node always shows content as the first line.
                    let add_spacer_after =
                        node_idx + 1 < node_count && state.view.is_block_start(node_idx + 1);

                    // Code blocks render line-by-line without sentence wrap logic.
                    let section_highlight_active = state
                        .section_highlight_range
                        .as_ref()
                        .is_some_and(|range| range.contains(&node_idx));
                    let strike_units: Vec<(SelectionUnit, usize)> = state
                        .strikes
                        .get(&node_idx)
                        .map(|set| set.iter().copied().collect())
                        .unwrap_or_default();
                    if let Some(code_rows) =
                        state.view.styled_code_block_rows(CodeBlockStyleRequest {
                            node_idx,
                            active_anchor: state.selection_state.anchor,
                            section_highlight_active,
                            strike_units: &strike_units,
                        })
                    {
                        let mut display_rows: Vec<RenderedDisplayRow> =
                            Vec::with_capacity(code_rows.len() + 1);
                        for (i, mut row) in code_rows.into_iter().enumerate() {
                            let mut spans = vec![if i == 0 {
                                Span::styled(format!("{indicator} "), indicator_style)
                            } else {
                                Span::raw("  ")
                            }];
                            spans.append(&mut row.spans);
                            display_rows.push(RenderedDisplayRow {
                                line: Line::from(spans),
                                byte_range: row.byte_range,
                            });
                        }
                        if add_spacer_after {
                            display_rows.push(RenderedDisplayRow::spacer());
                        }
                        let height = display_rows.len().max(1) as u16;
                        node_heights.push(height);
                        return display_rows;
                    }

                    let spans = Self::render_node_spans_for(&state, node_idx);
                    let mut display_rows: Vec<RenderedDisplayRow> = state
                        .view
                        .wrapped_display_rows(node_idx, spans, wrapped_text_width)
                        .into_iter()
                        .enumerate()
                        .map(|(seg_idx, mut row)| {
                            let mut line_spans = Vec::new();
                            if seg_idx == 0 {
                                line_spans
                                    .push(Span::styled(format!("{indicator} "), indicator_style));
                            } else {
                                line_spans.push(Span::raw("  "));
                            }
                            line_spans.append(&mut row.spans);
                            RenderedDisplayRow {
                                line: Line::from(line_spans),
                                byte_range: row.byte_range,
                            }
                        })
                        .collect();

                    if add_spacer_after {
                        display_rows.push(RenderedDisplayRow::spacer());
                    }
                    let height = display_rows.len().max(1) as u16;
                    node_heights.push(height);
                    display_rows
                })
                .collect();
            (node_rows, node_heights)
        };

        self.cached_node_heights = node_heights;
        self.adjust_scroll(list_inner.height);
        self.fill_partial_bottom(list_inner.height);

        // Render block border, then manually render lines so partial items are
        // clipped at the bottom rather than skipped entirely.
        frame.render_widget(list_block, area);
        let mut visible: Vec<Line<'static>> = Vec::new();
        let mut visible_row_ranges: Vec<(usize, Range<usize>)> = Vec::new();
        let mut count = 0u16;
        'outer: for (node_idx, rows) in node_rows.iter().enumerate().skip(self.scroll_offset) {
            for row in rows {
                if count >= list_inner.height {
                    break 'outer;
                }
                visible.push(row.line.clone());
                visible_row_ranges.push((node_idx, row.byte_range.clone()));
                count += 1;
            }
        }
        self.view
            .set_visible_rows(visible_row_ranges, GUTTER_WIDTH as u16);
        frame.render_widget(Paragraph::new(Text::from(visible)), list_inner);
        list_inner
    }

    fn adjust_scroll(&mut self, inner_height: u16) {
        let heights = &self.cached_node_heights;
        if heights.is_empty() {
            return;
        }
        let n = heights.len();
        self.scroll_offset = self.scroll_offset.min(n.saturating_sub(1));

        if self.selection_state.anchor.node_idx < self.scroll_offset {
            self.scroll_offset = self.selection_state.anchor.node_idx;
            return;
        }

        let cursor_height = heights
            .get(self.selection_state.anchor.node_idx)
            .copied()
            .unwrap_or(1);
        let rows_before: u16 = heights
            .get(self.scroll_offset..self.selection_state.anchor.node_idx)
            .map_or(0, |s| s.iter().copied().sum());

        // Cursor is fully visible — nothing to do.
        if rows_before + cursor_height <= inner_height {
            return;
        }

        // Cursor extends past the bottom (or is entirely off-screen). Reposition so
        // the cursor's bottom aligns with the screen bottom, maximising the number of
        // cursor lines shown. If the cursor is taller than the screen, put it at top.
        let target_start = inner_height.saturating_sub(cursor_height);

        let mut new_offset = self.selection_state.anchor.node_idx;
        let mut cum: u16 = 0;
        for i in (0..self.selection_state.anchor.node_idx).rev() {
            let h = heights.get(i).copied().unwrap_or(0);
            if cum + h > target_start {
                break;
            }
            cum += h;
            new_offset = i;
        }
        self.scroll_offset = new_offset;
    }

    /// After cursor positioning, pull any item that is partially visible at the
    /// bottom fully into view — as long as the cursor node remains visible.
    ///
    /// This covers the case where the cursor is on node N and node N+1 (or later)
    /// is partially clipped at the bottom: we scroll forward enough to show the
    /// partial item fully, provided the cursor itself stays in view.
    fn fill_partial_bottom(&mut self, inner_height: u16) {
        let heights = &self.cached_node_heights;
        if heights.is_empty() {
            return;
        }

        // Find the first partially-visible item at the bottom of the current view.
        let mut cum: u16 = 0;
        let mut partial: Option<(usize, u16)> = None;
        for (i, &h) in heights.iter().enumerate().skip(self.scroll_offset) {
            if cum + h > inner_height {
                partial = Some((i, h));
                break;
            }
            cum += h;
        }

        let (partial_idx, partial_h) = match partial {
            Some(p) if p.1 <= inner_height => p, // only handle items that can fit fully
            _ => return,
        };

        if partial_idx <= self.selection_state.anchor.node_idx {
            return; // cursor-based logic already handles this
        }

        // How many rows do we need to free up above to show the partial item fully?
        // Currently the item starts at `cum`; we need it at `inner_height - partial_h`.
        let needed = cum.saturating_sub(inner_height - partial_h);
        if needed == 0 {
            return;
        }

        // Try to advance scroll_offset by `needed` rows, while keeping cursor visible.
        let cursor_h = heights
            .get(self.selection_state.anchor.node_idx)
            .copied()
            .unwrap_or(1);
        let mut skipped: u16 = 0;
        let mut new_offset = self.scroll_offset;
        for i in self.scroll_offset..partial_idx {
            let h = heights.get(i).copied().unwrap_or(0);
            if skipped + h > needed {
                break;
            }
            // Verify cursor stays visible after advancing past item i.
            let candidate = i + 1;
            if candidate > self.selection_state.anchor.node_idx {
                break; // would push cursor above offset
            }
            let rows_before_cursor: u16 = heights
                .get(candidate..self.selection_state.anchor.node_idx)
                .map_or(0, |s| s.iter().copied().sum());
            if rows_before_cursor + cursor_h > inner_height {
                break; // cursor would go off-screen
            }
            skipped += h;
            new_offset = candidate;
        }
        self.scroll_offset = new_offset;
    }
}

struct RenderedDisplayRow {
    line: Line<'static>,
    byte_range: Range<usize>,
}

impl RenderedDisplayRow {
    fn spacer() -> Self {
        Self {
            line: Line::from(""),
            byte_range: 0..0,
        }
    }
}
