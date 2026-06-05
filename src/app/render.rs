use super::*;

impl App {
    // ── Drawing ───────────────────────────────────────────────────────────────

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(FOOTER_HEIGHT)])
            .split(area);
        let line_area_inner_width = layout[0].width.saturating_sub(2) as usize;
        let wrapped_text_width = line_area_inner_width.saturating_sub(GUTTER_WIDTH).max(1);

        let filename = self
            .source_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("markdown");
        let (change_count, feedback_count, insert_count, strike_count) = self.annotation_counts();
        let block_title =
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
            };

        let list_block = Block::default()
            .borders(Borders::ALL)
            .title(block_title)
            .border_style(Style::default().fg(Color::Gray));
        let list_inner = list_block.inner(layout[0]);
        self.list_inner = list_inner;

        let mut node_heights: Vec<u16> = Vec::with_capacity(self.view.node_count());

        let node_count = self.view.node_count();
        // Parallel to `node_lines`: one byte range per produced row in
        // the matching `Vec<Line>`, indexed by `(node_idx, row)`. The
        // trailing spacer (when present) carries an empty range. Used
        // to populate `visible_rows` after scroll-clipping.
        let mut node_row_byte_ranges: Vec<Vec<Range<usize>>> = Vec::with_capacity(node_count);
        let node_lines: Vec<Vec<Line<'static>>> = (0..node_count)
            .map(|node_idx| {
                let (indicator, indicator_style) = self.node_indicator(node_idx);
                // Add a blank trailing line when the NEXT node is a block start.
                // This keeps the spacer at the END of the preceding item so that
                // navigating to any node always shows content as the first line.
                let add_spacer_after =
                    node_idx + 1 < node_count && self.view.is_block_start(node_idx + 1);

                // Code blocks render line-by-line without sentence wrap logic.
                if let Some(code_rows) = self.view.code_block_render_lines(node_idx) {
                    let mut row_ranges: Vec<Range<usize>> = Vec::with_capacity(code_rows.len() + 1);
                    let mut display_lines: Vec<Line> = Vec::with_capacity(code_rows.len() + 1);
                    for (i, row) in code_rows.iter().enumerate() {
                        let base_style = if row.is_fence {
                            Style::default().fg(Color::DarkGray)
                        } else {
                            Style::default().fg(Color::White).bg(Color::DarkGray)
                        };
                        let mut spans = vec![if i == 0 {
                            Span::styled(format!("{indicator} "), indicator_style)
                        } else {
                            Span::raw("  ")
                        }];
                        // Overlay highlight + strikes on this source
                        // line by mapping the active anchor's
                        // selection-view byte range (and each strike
                        // range) into bytes within `line`. Without
                        // this, code blocks rendered with no visible
                        // cursor — the special draw path used to
                        // bypass render_node_spans entirely so
                        // word-mode highlight on a fenced code block
                        // (e.g. YAML frontmatter folded as a
                        // CodeBlock) showed nothing.
                        self.push_codeblock_line_spans(
                            &mut spans,
                            node_idx,
                            row.source_line,
                            row.text,
                            base_style,
                        );
                        row_ranges.push(row.byte_range.clone());
                        display_lines.push(Line::from(spans));
                    }
                    if add_spacer_after {
                        display_lines.push(Line::from(""));
                        row_ranges.push(0..0);
                    }
                    let height = display_lines.len().max(1) as u16;
                    node_heights.push(height);
                    node_row_byte_ranges.push(row_ranges);
                    return display_lines;
                }

                let spans = self.render_node_spans(node_idx);
                let wrapped = wrap_styled_spans(spans, wrapped_text_width);
                let plain = self.view.rendered_plain(node_idx).unwrap_or("");
                let mut row_ranges = wrap_line_byte_ranges(plain, &wrapped);

                let mut wrapped_lines: Vec<Line> = wrapped
                    .into_iter()
                    .enumerate()
                    .map(|(seg_idx, mut seg)| {
                        let mut line_spans = Vec::new();
                        if seg_idx == 0 {
                            line_spans.push(Span::styled(format!("{indicator} "), indicator_style));
                        } else {
                            line_spans.push(Span::raw("  "));
                        }
                        line_spans.append(&mut seg);
                        Line::from(line_spans)
                    })
                    .collect();

                if add_spacer_after {
                    wrapped_lines.push(Line::from(""));
                    row_ranges.push(0..0);
                }
                let height = wrapped_lines.len().max(1) as u16;
                node_heights.push(height);
                node_row_byte_ranges.push(row_ranges);

                wrapped_lines
            })
            .collect();

        self.cached_node_heights = node_heights;
        self.adjust_scroll(list_inner.height);
        self.fill_partial_bottom(list_inner.height);

        // Render block border, then manually render lines so partial items are
        // clipped at the bottom rather than skipped entirely.
        frame.render_widget(list_block, layout[0]);
        let mut visible: Vec<Line<'static>> = Vec::new();
        // Re-build the view-owned per-row map in lockstep so a click at a
        // visible row resolves to the correct node and display byte range.
        self.view.clear_visible_rows();
        let mut count = 0u16;
        'outer: for (node_idx, (lines, row_ranges)) in node_lines
            .iter()
            .zip(node_row_byte_ranges.iter())
            .enumerate()
            .skip(self.scroll_offset)
        {
            for (line, byte_range) in lines.iter().zip(row_ranges.iter()) {
                if count >= list_inner.height {
                    break 'outer;
                }
                visible.push(line.clone());
                self.view
                    .push_visible_row(node_idx, byte_range.clone(), GUTTER_WIDTH as u16);
                count += 1;
            }
        }
        frame.render_widget(Paragraph::new(Text::from(visible)), list_inner);

        // Two-zone footer: persistent left mode indicator + transient right
        // zone (nav feedback / notification / hint). Mode indicator is never
        // truncated; right zone shrinks first under width pressure.
        let mode_text = format!(" mode: {}", self.mode_indicator());
        let mode_style = Style::default().fg(Color::Cyan);
        let hint_style = Style::default().fg(Color::DarkGray);
        // Right zone priority: transient nav_feedback (one-keypress
        // boundary message) > transient notification (clipboard result) >
        // persistent status (current mode / last action context) > help
        // hint. The status field accumulates input-mode prompts and
        // action-confirmation messages; without showing it the user never
        // sees mode prompts like "Change mode: type text and press Enter."
        let right_text = if let Some(fb) = &self.nav_feedback {
            (fb.clone(), Style::default().fg(Color::Yellow))
        } else if let Some(note) = &self.notification {
            (note.clone(), Style::default().fg(Color::Green))
        } else if !self.status.is_empty() {
            (self.status.clone(), Style::default().fg(Color::Gray))
        } else {
            ("? for help ".to_string(), hint_style)
        };
        // Account for terminal column width (not byte length) so user-supplied
        // text in the right zone — e.g. a search query containing CJK or
        // emoji — doesn't underestimate the gap and squish the footer when
        // the message contains wide-width characters.
        let total_width = layout[1].width as usize;
        let mode_w = UnicodeWidthStr::width(mode_text.as_str());
        let right_avail = total_width.saturating_sub(mode_w + 1);
        let right_str = truncate_to_columns(&right_text.0, right_avail);
        let right_w = UnicodeWidthStr::width(right_str.as_str());
        let gap = total_width.saturating_sub(mode_w + right_w);
        let footer_line = Line::from(vec![
            Span::styled(mode_text, mode_style),
            Span::raw(" ".repeat(gap)),
            Span::styled(right_str, right_text.1),
        ]);
        frame.render_widget(Paragraph::new(footer_line), layout[1]);

        let popup_spec: Option<(&str, &str, &str, &str)> = match &self.input_mode {
            InputMode::Change => Some((
                " Change ",
                "Change mode: Enter save | Esc cancel",
                "Change> ",
                self.change_buffer.as_str(),
            )),
            InputMode::EditChange(..) => Some((
                " Edit Change ",
                "Edit mode: Enter save | Esc cancel",
                "Change> ",
                self.change_buffer.as_str(),
            )),
            InputMode::Feedback => Some((
                " Feedback ",
                "Feedback mode: Enter save | Esc cancel",
                "Feedback> ",
                self.feedback_buffer.as_str(),
            )),
            InputMode::EditFeedback(..) => Some((
                " Edit Feedback ",
                "Edit mode: Enter save | Esc cancel",
                "Feedback> ",
                self.feedback_buffer.as_str(),
            )),
            InputMode::InsertBefore => Some((
                " Insert Before ",
                "Insert before: Enter save | Esc cancel",
                "Before> ",
                self.insert_buffer.as_str(),
            )),
            InputMode::InsertAfter => Some((
                " Insert After ",
                "Insert after: Enter save | Esc cancel",
                "After> ",
                self.insert_buffer.as_str(),
            )),
            InputMode::Search => Some((
                " Search ",
                "Search: Enter jump | Esc cancel | n/N next/prev",
                "/",
                self.search_buffer.as_str(),
            )),
            InputMode::Normal => None,
        };
        if let Some((title, hint, prompt, buf)) = popup_spec {
            self.draw_input_popup(frame, list_inner, title, hint, prompt, buf);
        }

        if self.link_popup_urls.is_some() {
            self.draw_link_popup(frame, area);
        }

        if self.show_help {
            Self::draw_help(frame, area);
        }

        if self.ast_view_scroll.is_some() {
            self.draw_ast_popup(frame, area);
        }
    }

    fn draw_input_popup(
        &self,
        frame: &mut Frame,
        list_inner: Rect,
        title: &str,
        hint: &str,
        prompt: &str,
        buf: &str,
    ) {
        let heights = &self.cached_node_heights;
        if list_inner.width < 12
            || list_inner.height < 4
            || self.selection_state.anchor.node_idx >= heights.len()
        {
            return;
        }

        let list_offset = self.scroll_offset;
        if self.selection_state.anchor.node_idx < list_offset {
            return;
        }

        let selected_top: u16 = heights
            .iter()
            .skip(list_offset)
            .take(self.selection_state.anchor.node_idx - list_offset)
            .copied()
            .sum();
        let selected_height = heights[self.selection_state.anchor.node_idx].max(1);

        if selected_top >= list_inner.height {
            return;
        }

        let popup_width = list_inner.width.clamp(20, 80);
        let inner_width = popup_width.saturating_sub(2) as usize;

        let hint_height =
            wrap_styled_spans(vec![Span::raw(hint.to_owned())], inner_width).len() as u16;
        let body_height =
            wrap_styled_spans(vec![Span::raw(format!("{prompt}{buf}"))], inner_width).len() as u16;
        let needed_height = hint_height
            .max(1)
            .saturating_add(body_height.max(1))
            .saturating_add(2);
        let max_popup_height = list_inner.height.saturating_sub(2).max(4);
        let popup_height = needed_height.clamp(4, max_popup_height);

        let list_bottom = list_inner.y + list_inner.height;
        let preferred_below_y = list_inner.y
            + selected_top
                .saturating_add(selected_height)
                .min(list_inner.height.saturating_sub(1));
        let anchor_above_top = list_inner.y + selected_top;
        let y = if preferred_below_y.saturating_add(popup_height) <= list_bottom {
            preferred_below_y
        } else if anchor_above_top >= list_inner.y.saturating_add(popup_height) {
            anchor_above_top - popup_height
        } else {
            list_bottom.saturating_sub(popup_height).max(list_inner.y)
        };

        let popup = Rect {
            x: list_inner.x,
            y,
            width: popup_width,
            height: popup_height,
        };

        let lines = vec![
            Line::from(Span::styled(
                hint.to_owned(),
                Style::default().fg(Color::Yellow),
            )),
            Line::from(format!("{prompt}{buf}")),
        ];

        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(Text::from(lines))
                .block(
                    Block::default()
                        .title(title.to_owned())
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Yellow)),
                )
                .wrap(Wrap { trim: false }),
            popup,
        );
    }

    fn draw_help(frame: &mut Frame, area: Rect) {
        let help_lines = vec![
            Line::from(""),
            Line::from(Span::styled("  Navigate", Style::default().fg(Color::Cyan))),
            Line::from("  j, k    next/prev unit"),
            Line::from("  i, o    finer/coarser units"),
            Line::from(""),
            Line::from(Span::styled("  Annotate", Style::default().fg(Color::Cyan))),
            Line::from("  c       change (literal)"),
            Line::from("  f       feedback (intent)"),
            Line::from("  b, a    insert before/after"),
            Line::from("  x       clear or strike"),
            Line::from(""),
            Line::from("  [, ]    prev/next annotation"),
            Line::from("  e       edit annotation"),
            Line::from("  /       search"),
            Line::from("  n, N    next/prev search match"),
            Line::from(""),
            Line::from("  r       copy annotations to clipboard"),
            Line::from("  q       quit; printing changes to stdout"),
            Line::from("  Q       silent quit (discard annotations)"),
        ];

        let content_width: u16 = help_lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(40) as u16;
        let content_height = help_lines.len() as u16;
        let popup_width = (content_width + 2).max(72).min(area.width);
        let popup_height = (content_height + 2).min(area.height);
        let popup = Rect {
            x: area.x + area.width.saturating_sub(popup_width) / 2,
            y: area.y + area.height.saturating_sub(popup_height) / 2,
            width: popup_width,
            height: popup_height,
        };

        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(Text::from(help_lines))
                .block(
                    Block::default()
                        .title(" Help ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .wrap(Wrap { trim: false }),
            popup,
        );
    }

    fn draw_ast_popup(&self, frame: &mut Frame, area: Rect) {
        let popup_width = (area.width * 4 / 5).max(40).min(area.width);
        let popup_height = (area.height * 4 / 5).max(6).min(area.height);
        let popup = Rect {
            x: area.x + area.width.saturating_sub(popup_width) / 2,
            y: area.y + area.height.saturating_sub(popup_height) / 2,
            width: popup_width,
            height: popup_height,
        };

        let lines: Vec<Line> = self
            .ast_lines
            .iter()
            .map(|l| Line::from(Span::raw(l.clone())))
            .collect();

        let total = self.ast_lines.len() as u16;
        // With wrap enabled the scroll axis is display-rows, not
        // source-lines; long lines wrap to multiple rows so the user
        // can drift past `total` worth of "lines" before exhausting
        // visible content. Cap scroll to total source lines as a
        // reasonable upper bound — overshoot just shows blank rows
        // at the bottom rather than truncating right-edge content.
        let inner_height = popup_height.saturating_sub(2);
        let max_scroll = total.saturating_sub(inner_height);
        let scroll = self.ast_view_scroll.unwrap_or(0).min(max_scroll);

        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(Text::from(lines))
                .block(
                    Block::default()
                        .title(format!(
                            " AST  [{}/{}]  j/k scroll · I/Esc close ",
                            scroll + 1,
                            total
                        ))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Green)),
                )
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0)),
            popup,
        );
    }

    fn draw_link_popup(&self, frame: &mut Frame, area: Rect) {
        // Caller in draw() gates on link_popup_urls.is_some(), so the
        // None case here is unreachable; default to an empty slice if
        // it ever fires.
        let urls: &[String] = self.link_popup_urls.as_deref().unwrap_or(&[]);
        let popup_width = area.width.saturating_sub(10).clamp(40, 100);
        let max_height = area.height.saturating_sub(6).max(6);
        let desired_height = (urls.len() as u16).saturating_add(5).clamp(6, max_height);
        let popup = Rect {
            x: area.x + area.width.saturating_sub(popup_width) / 2,
            y: area.y + area.height.saturating_sub(desired_height) / 2,
            width: popup_width,
            height: desired_height,
        };

        let mut lines = Vec::new();
        lines.push(Line::from("Links in current sentence:"));
        lines.push(Line::from(""));
        for (idx, url) in urls.iter().enumerate() {
            lines.push(Line::from(format!("{}. {}", idx + 1, url)));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Press i or Esc to close",
            Style::default().fg(Color::Gray),
        )));

        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(Text::from(lines))
                .block(
                    Block::default()
                        .title(" Link ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .wrap(Wrap { trim: false }),
            popup,
        );
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

    fn node_indicator(&self, node_idx: usize) -> (&'static str, Style) {
        let change_count = self.changes.get(&node_idx).map_or(0, |v| v.len());
        let feedback_count = self.feedbacks.get(&node_idx).map_or(0, |v| v.len());
        let insert_count = self.inserts_before.get(&node_idx).map_or(0, |v| v.len())
            + self.inserts_after.get(&node_idx).map_or(0, |v| v.len());
        let strike_count = self.strikes.get(&node_idx).map_or(0, |v| v.len());

        let has_change = change_count > 0;
        let has_feedback = feedback_count > 0;
        let has_insert = insert_count > 0;
        let has_strike = strike_count > 0;

        let total = change_count + feedback_count + insert_count + strike_count;
        if total > 1 {
            return (
                "*",
                Style::default()
                    .fg(Color::LightMagenta)
                    .add_modifier(Modifier::BOLD),
            );
        }
        if has_change {
            return (
                "C",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );
        }
        if has_feedback {
            return (
                "F",
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            );
        }
        if has_insert {
            return (
                "+",
                Style::default()
                    .fg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            );
        }
        if has_strike {
            return ("X", Style::default().fg(Color::LightRed));
        }
        (" ", Style::default().fg(Color::DarkGray))
    }

    /// Compute the byte range in the rendered display plain text that the active
    /// selection unit should paint. Section selections paint whole nodes via
    /// `section_highlight_range` in the caller.
    fn unit_highlight_for(&self, node_idx: usize) -> Option<Range<usize>> {
        self.view.display_range_for_unit(
            node_idx,
            self.selection_state.anchor.unit,
            self.selection_state.anchor.unit_idx,
        )
    }

    /// Push styled span(s) for one source line of a code block, overlaying
    /// the active highlight and any strike ranges that intersect this line.
    /// `node_idx` identifies the code block; `source_line` is the absolute
    /// source line index; `line` is its raw text;
    /// `base_style` is the code-block paint (DarkGray fence vs
    /// White-on-DarkGray content). The active anchor and strike entries
    /// store byte ranges in selection_plain_text — we map each into bytes
    /// within `line` via the index's source_line_ranges table, then split
    /// the span at the overlap so the highlight paints precisely.
    fn push_codeblock_line_spans(
        &self,
        spans: &mut Vec<Span<'static>>,
        node_idx: usize,
        source_line: usize,
        line: &str,
        base_style: Style,
    ) {
        // Active anchor → highlight range on this line. Section-mode
        // whole-node highlight is handled separately below.
        let highlight_local = if self
            .section_highlight_range
            .as_ref()
            .is_some_and(|r| r.contains(&node_idx))
        {
            Some(0..line.len())
        } else if node_idx == self.selection_state.anchor.node_idx {
            let unit = self.selection_state.anchor.unit;
            let unit_idx = self.selection_state.anchor.unit_idx;
            self.view
                .selection_range_for_unit(node_idx, unit, unit_idx)
                .and_then(|range| {
                    self.view
                        .code_line_local_range(node_idx, source_line, range)
                })
        } else {
            None
        };

        // Strike ranges on this line.
        let strike_local: Vec<Range<usize>> = self
            .strikes
            .get(&node_idx)
            .map(|set| {
                set.iter()
                    .filter_map(|&(unit, idx)| {
                        self.view
                            .selection_range_for_unit(node_idx, unit, idx)
                            .and_then(|range| {
                                self.view
                                    .code_line_local_range(node_idx, source_line, range)
                            })
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Build segment boundaries.
        let mut bounds = vec![0, line.len()];
        if let Some(r) = &highlight_local {
            bounds.push(r.start);
            bounds.push(r.end);
        }
        for r in &strike_local {
            bounds.push(r.start);
            bounds.push(r.end);
        }
        bounds.sort_unstable();
        bounds.dedup();

        for pair in bounds.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            if start >= end {
                continue;
            }
            let Some(text) = line.get(start..end) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            let mut style = base_style;
            if highlight_local
                .as_ref()
                .is_some_and(|r| start < r.end && end > r.start)
            {
                style = style.patch(
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                );
            }
            if strike_local.iter().any(|r| start < r.end && end > r.start) {
                style = style.patch(
                    Style::default()
                        .fg(Color::Red)
                        .add_modifier(Modifier::CROSSED_OUT | Modifier::DIM),
                );
            }
            spans.push(Span::styled(text.to_string(), style));
        }

        if spans.is_empty() {
            spans.push(Span::styled(line.to_string(), base_style));
        }
    }

    pub(super) fn render_node_spans(&self, node_idx: usize) -> Vec<Span<'static>> {
        // Resolve every strike anchor on this node to a display byte
        // range. Empty when nothing is struck.
        let strike_ranges: Vec<Range<usize>> = self
            .strikes
            .get(&node_idx)
            .map(|set| {
                set.iter()
                    .filter_map(|&(unit, idx)| {
                        self.view.display_range_for_unit(node_idx, unit, idx)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let highlight = if self
            .section_highlight_range
            .as_ref()
            .is_some_and(|r| r.contains(&node_idx))
        {
            let plain_len = self.view.rendered_plain(node_idx).map_or(0, str::len);
            Some(0..plain_len)
        } else if node_idx == self.selection_state.anchor.node_idx {
            self.unit_highlight_for(node_idx)
        } else {
            None
        };

        self.view
            .styled_display_spans(node_idx, highlight, &strike_ranges)
            .unwrap_or_else(|| {
                vec![Span::styled(
                    " ",
                    Style::default().add_modifier(Modifier::DIM),
                )]
            })
    }
}
