use super::*;

impl App {
    pub(super) fn draw_document(&mut self, frame: &mut Frame, area: Rect) -> Rect {
        let line_area_inner_width = area.width.saturating_sub(2) as usize;
        let wrapped_text_width = line_area_inner_width.saturating_sub(GUTTER_WIDTH).max(1);

        let (block_title, rendered) = {
            let state = self.render_state();
            (
                crate::ui::render::document_block_title(&state),
                crate::ui::render::build_document_rows(&state, wrapped_text_width),
            )
        };

        let list_block = Block::default()
            .borders(Borders::ALL)
            .title(block_title)
            .border_style(Style::default().fg(Color::Gray));
        let list_inner = list_block.inner(area);
        self.list_inner = list_inner;

        self.render_cache.node_heights = rendered.node_heights;
        self.adjust_scroll(list_inner.height);
        self.fill_partial_bottom(list_inner.height);

        // Render block border, then manually render lines so partial items are
        // clipped at the bottom rather than skipped entirely.
        frame.render_widget(list_block, area);
        let (visible, visible_row_ranges) = crate::ui::render::visible_document_lines(
            &rendered.rows,
            self.scroll_offset,
            list_inner.height,
        );
        let cached_visible_rows = visible_row_ranges.clone();
        self.view
            .set_visible_rows(visible_row_ranges, GUTTER_WIDTH as u16);
        self.render_cache
            .replace_document_rows(self.render_cache.node_heights.clone(), cached_visible_rows);
        frame.render_widget(Paragraph::new(Text::from(visible)), list_inner);
        list_inner
    }

    #[cfg(test)]
    pub(in crate::app) fn render_node_spans(&self, node_idx: usize) -> Vec<Span<'static>> {
        let state = self.render_state();
        crate::ui::render::render_node_spans(&state, node_idx)
    }

    fn adjust_scroll(&mut self, inner_height: u16) {
        let heights = &self.render_cache.node_heights;
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
        let heights = &self.render_cache.node_heights;
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
