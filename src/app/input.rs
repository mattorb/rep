use super::*;

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) {
        match self.input_mode.clone() {
            InputMode::Normal => self.handle_normal_key(key),
            InputMode::Change => self.handle_change_key(key),
            InputMode::Feedback => self.handle_feedback_key(key),
            InputMode::InsertBefore => self.handle_insert_key(key, true),
            InputMode::InsertAfter => self.handle_insert_key(key, false),
            InputMode::Search => self.handle_search_key(key),
            InputMode::EditChange(node_idx, change_idx) => {
                self.handle_edit_change_key(key, node_idx, change_idx);
            }
            InputMode::EditFeedback(node_idx, feedback_idx) => {
                self.handle_edit_feedback_key(key, node_idx, feedback_idx);
            }
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        self.notification = None;
        self.nav_feedback = None;

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        if self.quit_confirm_pending {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Char('q') | KeyCode::Enter => {
                    self.should_quit = true;
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.quit_confirm_pending = false;
                    self.status = "Quit cancelled.".to_string();
                }
                _ => {
                    // Other keys are ignored so a stray keystroke doesn't
                    // accidentally answer the prompt.
                }
            }
            return;
        }

        if let Some(scroll) = self.ast_view_scroll {
            match key.code {
                KeyCode::Esc | KeyCode::Char('I') => {
                    self.ast_view_scroll = None;
                    self.status = "Closed AST view.".to_string();
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.ast_view_scroll = Some(scroll.saturating_add(3));
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.ast_view_scroll = Some(scroll.saturating_sub(3));
                }
                _ => {}
            }
            return;
        }

        if self.link_popup_urls.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('O') => {
                    self.link_popup_urls = None;
                    self.status = "Closed link popup.".to_string();
                }
                _ => {
                    self.link_popup_urls = None;
                }
            }
            return;
        }

        if self.show_help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => {
                    self.show_help = false;
                    self.status = "Closed help.".to_string();
                }
                KeyCode::Char('/') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.show_help = false;
                    self.status = "Closed help.".to_string();
                }
                _ => {
                    self.show_help = false;
                }
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') => {
                self.quit_confirm_pending = true;
                self.status = "Are you sure? (results go to stdout)  y / n".to_string();
            }
            KeyCode::Char('Q') => {
                self.silent_quit = true;
                self.should_quit = true;
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                self.status = "Help open. Press ? or Esc to close.".to_string();
            }
            KeyCode::Char('/') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.show_help = true;
                self.status = "Help open. Press ? or Esc to close.".to_string();
            }
            KeyCode::Char('/') => {
                self.input_mode = InputMode::Search;
                self.search_buffer.clear();
                self.status = "Search: type pattern and press Enter. Esc cancels.".to_string();
            }
            KeyCode::Char('n') => self.jump_search(true),
            KeyCode::Char('N') => self.jump_search(false),
            KeyCode::Char('c') => self.begin_change_or_edit(),
            KeyCode::Char('f') => self.begin_feedback_or_edit(),
            KeyCode::Char('b') => {
                self.input_mode = InputMode::InsertBefore;
                self.insert_buffer.clear();
                self.status = "Insert before: type text and press Enter. Esc cancels.".to_string();
            }
            KeyCode::Char('a') => {
                self.input_mode = InputMode::InsertAfter;
                self.insert_buffer.clear();
                self.status = "Insert after: type text and press Enter. Esc cancels.".to_string();
            }
            KeyCode::Char('e') => self.begin_edit_annotation(),
            KeyCode::Char(' ') => self.mode_cycle(true),
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Right => self.move_active_unit(true),
            KeyCode::Char('k') | KeyCode::Up | KeyCode::Left => self.move_active_unit(false),
            KeyCode::Backspace => self.mode_cycle(false),
            // i / o adjust the active selection unit by one step
            // without wrapping. i = "in" / finer; o = "out" / coarser.
            KeyCode::Char('i') => self.mode_adjust(true),
            KeyCode::Char('o') => self.mode_adjust(false),
            // Capital variants are the popups: I opens the AST popup,
            // O reveals links from the current sentence.
            KeyCode::Char('I') => {
                self.ast_view_scroll = Some(0);
                self.status = "AST view. j/k scroll, I or Esc close.".to_string();
            }
            KeyCode::Char('O') if !self.reveal_links_for_current_sentence() => {
                self.status = "No markdown links in current sentence.".to_string();
            }
            KeyCode::Char('x') => self.toggle_strike(),
            KeyCode::Char('r') => {
                let output = self.to_human_output();
                self.notification = Some(match copy_to_clipboard(&output) {
                    ClipboardOutcome::OsCommand => "Copied to clipboard".to_string(),
                    ClipboardOutcome::Osc52 => "Sent via OSC 52".to_string(),
                    ClipboardOutcome::Failed => "Copy failed — no clipboard available".to_string(),
                });
            }
            KeyCode::Char('[') => self.jump_to_annotation(false),
            KeyCode::Char(']') => self.jump_to_annotation(true),
            _ => {}
        }
    }

    fn handle_change_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.change_buffer.clear();
                self.status = "Change cancelled.".to_string();
            }
            KeyCode::Enter => {
                let trimmed = self.change_buffer.trim().to_string();
                if trimmed.is_empty() {
                    self.status = "Change ignored because it was empty.".to_string();
                } else {
                    let (sentence_index, sentence_text) =
                        if let Some((idx, text)) = self.current_target_capture() {
                            (Some(idx), Some(text))
                        } else {
                            (None, None)
                        };
                    let annotation = ChangeAnnotation {
                        created_at: Utc::now().to_rfc3339(),
                        target_unit: self.selection_state.anchor.unit,
                        sentence_index,
                        sentence_text,
                        change: trimmed,
                    };
                    self.changes
                        .entry(self.selection_state.anchor.node_idx)
                        .or_default()
                        .push(annotation);
                    self.status = format!(
                        "Change saved on node {} (line {}).",
                        self.selection_state.anchor.node_idx + 1,
                        self.current_source_line() + 1
                    );
                }
                self.input_mode = InputMode::Normal;
                self.change_buffer.clear();
            }
            KeyCode::Backspace => {
                self.change_buffer.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.change_buffer.push(ch);
            }
            _ => {}
        }
    }

    fn handle_insert_key(&mut self, key: KeyEvent, before: bool) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.insert_buffer.clear();
                self.status = if before {
                    "Insert before cancelled.".to_string()
                } else {
                    "Insert after cancelled.".to_string()
                };
            }
            KeyCode::Enter => {
                let trimmed = self.insert_buffer.trim().to_string();
                if trimmed.is_empty() {
                    self.status = "Insert ignored because it was empty.".to_string();
                } else {
                    let (sentence_index, sentence_text) =
                        if let Some((idx, text)) = self.current_target_capture() {
                            (Some(idx), Some(text))
                        } else {
                            (None, None)
                        };
                    let annotation = InsertAnnotation {
                        created_at: Utc::now().to_rfc3339(),
                        target_unit: self.selection_state.anchor.unit,
                        sentence_index,
                        sentence_text,
                        text: trimmed,
                    };
                    let bucket = if before {
                        &mut self.inserts_before
                    } else {
                        &mut self.inserts_after
                    };
                    bucket
                        .entry(self.selection_state.anchor.node_idx)
                        .or_default()
                        .push(annotation);
                    let label = if before { "before" } else { "after" };
                    self.status = format!(
                        "Insert {label} saved on node {} (line {}).",
                        self.selection_state.anchor.node_idx + 1,
                        self.current_source_line() + 1
                    );
                }
                self.input_mode = InputMode::Normal;
                self.insert_buffer.clear();
            }
            KeyCode::Backspace => {
                self.insert_buffer.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.insert_buffer.push(ch);
            }
            _ => {}
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.search_buffer.clear();
                self.status = "Search cancelled.".to_string();
            }
            KeyCode::Enter => {
                let query = self.search_buffer.trim().to_string();
                self.input_mode = InputMode::Normal;
                self.search_buffer.clear();
                if query.is_empty() {
                    self.status = "Search cancelled (empty pattern).".to_string();
                    return;
                }
                self.run_search(&query, true);
                self.last_search = Some(query);
            }
            KeyCode::Backspace => {
                self.search_buffer.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_buffer.push(ch);
            }
            _ => {}
        }
    }

    /// Find every search hit across rendered nodes as (node, sentence) pairs.
    /// Smart-case: case-sensitive iff the query contains an ASCII uppercase letter.
    fn find_search_matches(&self, query: &str) -> Vec<(usize, usize)> {
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
                let sidx = rn
                    .sentence_ranges
                    .iter()
                    .position(|r| abs >= r.start && abs < r.end)
                    .unwrap_or(0);
                matches.push((ni, sidx));
                cursor = abs + needle.len();
            }
        }
        matches
    }

    fn run_search(&mut self, query: &str, forward: bool) {
        let matches = self.find_search_matches(query);
        if matches.is_empty() {
            self.status = format!("No matches for \"{query}\".");
            return;
        }
        let current = self.search_current_position();
        let target_idx = if forward {
            matches.iter().position(|m| *m >= current).unwrap_or(0)
        } else {
            matches
                .iter()
                .rposition(|m| *m <= current)
                .unwrap_or(matches.len() - 1)
        };
        self.apply_search_target(query, &matches, target_idx);
    }

    fn jump_search(&mut self, forward: bool) {
        let Some(query) = self.last_search.clone() else {
            self.status = "No previous search. Press / to search.".to_string();
            return;
        };
        let matches = self.find_search_matches(&query);
        if matches.is_empty() {
            self.status = format!("No matches for \"{query}\".");
            return;
        }
        let current = self.search_current_position();
        let target_idx = if forward {
            matches.iter().position(|m| *m > current).unwrap_or(0)
        } else {
            matches
                .iter()
                .rposition(|m| *m < current)
                .unwrap_or(matches.len() - 1)
        };
        self.apply_search_target(&query, &matches, target_idx);
    }

    /// `(node_idx, sentence_idx)` cursor position used to seed forward /
    /// backward search match lookup. In Sentence mode this is the literal
    /// anchor; in any other mode the unit_idx is not a sentence index, so
    /// we treat the cursor as positioned at sentence 0 of the current node
    /// — forward search finds the first match in or after this node, and
    /// backward search finds the last match strictly before this node.
    fn search_current_position(&self) -> (usize, usize) {
        let n = self.selection_state.anchor.node_idx;
        if self.selection_state.anchor.unit == SelectionUnit::Sentence {
            (n, self.selection_state.anchor.unit_idx)
        } else {
            (n, 0)
        }
    }

    fn apply_search_target(&mut self, query: &str, matches: &[(usize, usize)], target_idx: usize) {
        let (ni, si) = matches[target_idx];
        // find_search_matches returns sentence-keyed positions, so the
        // search jump always anchors at sentence granularity regardless of
        // the active unit at search time.
        self.selection_state.anchor = SelectionAnchor::new(ni, SelectionUnit::Sentence, si);
        self.clamp_sentence();
        self.status = format!(
            "Match {}/{} for \"{}\".",
            target_idx + 1,
            matches.len(),
            query
        );
    }

    fn handle_feedback_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.feedback_buffer.clear();
                self.status = "Feedback cancelled.".to_string();
            }
            KeyCode::Enter => {
                let trimmed = self.feedback_buffer.trim().to_string();
                if trimmed.is_empty() {
                    self.status = "Feedback ignored because it was empty.".to_string();
                } else {
                    let (sentence_index, sentence_text) =
                        if let Some((idx, text)) = self.current_target_capture() {
                            (Some(idx), Some(text))
                        } else {
                            (None, None)
                        };
                    let annotation = FeedbackAnnotation {
                        created_at: Utc::now().to_rfc3339(),
                        target_unit: self.selection_state.anchor.unit,
                        sentence_index,
                        sentence_text,
                        feedback: trimmed,
                    };
                    self.feedbacks
                        .entry(self.selection_state.anchor.node_idx)
                        .or_default()
                        .push(annotation);
                    self.status = format!(
                        "Feedback saved on node {} (line {}).",
                        self.selection_state.anchor.node_idx + 1,
                        self.current_source_line() + 1
                    );
                }
                self.input_mode = InputMode::Normal;
                self.feedback_buffer.clear();
            }
            KeyCode::Backspace => {
                self.feedback_buffer.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.feedback_buffer.push(ch);
            }
            _ => {}
        }
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    /// Mouse scroll: move through every content node (nodes that have at
    /// least one selection anchor). Arrow keys are bound to
    /// `move_active_unit` per the mode-switch keymap; this helper
    /// only serves the mouse wheel handler now.
    pub(super) fn move_node(&mut self, delta: isize) {
        if self.doc.node_count() == 0 || delta == 0 {
            return;
        }
        let steps = delta.unsigned_abs();
        let forward = delta.is_positive();
        let mut target = self.selection_state.anchor.node_idx;
        let mut moved = 0usize;

        for _ in 0..steps {
            let next = if forward {
                self.doc.next_content_node(target.saturating_add(1))
            } else {
                self.doc.prev_content_node(target)
            };
            let Some(idx) = next else { break };
            target = idx;
            moved += 1;
        }

        if moved == 0 {
            self.status = if forward {
                "Already at the last node.".to_string()
            } else {
                "Already at the first node.".to_string()
            };
            return;
        }
        self.selection_state.anchor.node_idx = target;
        self.clamp_sentence();
        self.status = format!(
            "Node {}/{}",
            self.selection_state.anchor.node_idx + 1,
            self.doc.node_count()
        );
    }

    /// j / k / Down / Up / Right / Left — move by the currently active
    /// selection unit. Pure delegate to `selection::navigator::next/prev`.
    /// On `Boundary`, set `nav_feedback` ("at end" / "at start") for one
    /// keypress in the right zone of the footer.
    fn move_active_unit(&mut self, forward: bool) {
        if self.doc.node_count() == 0 {
            return;
        }
        let outcome = if forward {
            crate::selection::navigator::next(&self.index, self.selection_state.anchor)
        } else {
            crate::selection::navigator::prev(&self.index, self.selection_state.anchor)
        };
        match outcome {
            crate::selection::model::NavOutcome::Moved(a) => {
                self.selection_state.anchor = a;
                self.refresh_section_highlight(a);
            }
            crate::selection::model::NavOutcome::Boundary => {
                // Zero-anchor units (e.g., section nav on a doc with no
                // headings): silent no-op per modular_plan §"Empty /
                // degenerate documents". Otherwise show "at end" / "at
                // start" feedback in the right zone. Selection state
                // (including section_highlight_range) stays put — the
                // user is still on the boundary section.
                if !self.unit_has_any_anchor(self.selection_state.anchor.unit) {
                    return;
                }
                self.nav_feedback = Some(if forward { "at end" } else { "at start" }.to_string());
            }
        }
    }

    fn unit_has_any_anchor(&self, unit: SelectionUnit) -> bool {
        match unit {
            SelectionUnit::Section => !self.index.sections.is_empty(),
            SelectionUnit::Paragraph => !self.index.paragraphs.is_empty(),
            SelectionUnit::Line => !self.index.lines.is_empty(),
            SelectionUnit::Sentence => !self.index.sentences.is_empty(),
            SelectionUnit::Word => !self.index.words.is_empty(),
        }
    }

    /// Space (forward) / Backspace (reverse) — cycle the active selection
    /// unit. Re-anchors via `navigator::clamp` per the pinned rules.
    fn mode_cycle(&mut self, forward: bool) {
        let order = SelectionUnit::CYCLE_ORDER;
        let i = order
            .iter()
            .position(|u| *u == self.selection_state.anchor.unit)
            .unwrap_or(0);
        let next_i = if forward {
            (i + 1) % order.len()
        } else {
            (i + order.len() - 1) % order.len()
        };
        let target = order[next_i];
        let new_anchor =
            crate::selection::navigator::clamp(&self.index, self.selection_state.anchor, target);
        self.selection_state.anchor = new_anchor;
        self.refresh_section_highlight(new_anchor);
    }

    /// i (finer) / o (coarser) — adjust the active selection unit by
    /// one step, stopping at the ends instead of wrapping around.
    fn mode_adjust(&mut self, finer: bool) {
        let order = SelectionUnit::CYCLE_ORDER;
        let i = order
            .iter()
            .position(|u| *u == self.selection_state.anchor.unit)
            .unwrap_or(0);
        let target = if finer {
            order.get(i + 1).copied()
        } else if i > 0 {
            order.get(i - 1).copied()
        } else {
            None
        };
        let Some(target) = target else {
            return;
        };
        let new_anchor =
            crate::selection::navigator::clamp(&self.index, self.selection_state.anchor, target);
        self.selection_state.anchor = new_anchor;
        self.refresh_section_highlight(new_anchor);
    }

    /// Refresh `section_highlight_range` for an anchor: set the span when
    /// the active unit is Section, clear otherwise. Used after every move
    /// or mode-cycle that lands on a new anchor.
    fn refresh_section_highlight(&mut self, anchor: SelectionAnchor) {
        if anchor.unit == SelectionUnit::Section {
            self.section_highlight_range = Some(self.section_span_for(anchor.node_idx));
        } else {
            self.section_highlight_range = None;
        }
    }

    /// Compute the inclusive-start, exclusive-end node range for the section
    /// starting at `node_idx`. Falls back to the rest of the document if the
    /// section table doesn't carry an entry for this node.
    fn section_span_for(&self, node_idx: usize) -> Range<usize> {
        let end = self
            .index
            .sections
            .iter()
            .find(|s| s.start_node_idx == node_idx)
            .map_or_else(|| self.doc.node_count(), |s| s.end_node_idx + 1);
        node_idx..end
    }

    /// Stable string for the mode indicator in the left zone of the footer.
    pub(super) const fn mode_indicator(&self) -> &'static str {
        self.selection_state.anchor.unit.mode_str()
    }

    fn jump_to_annotation(&mut self, forward: bool) {
        let from = if forward {
            self.selection_state.anchor.node_idx + 1
        } else {
            self.selection_state.anchor.node_idx
        };
        let n = self.doc.node_count();
        let target = if forward {
            (from..n).find(|&i| self.has_annotation(i))
        } else {
            (0..from).rev().find(|&i| self.has_annotation(i))
        };

        match target {
            Some(idx) => {
                self.selection_state.anchor.node_idx = idx;
                self.clamp_sentence();
                self.status = format!("Annotated node {}.", idx + 1);
            }
            None => {
                self.status = if forward {
                    "No annotated nodes after this one.".to_string()
                } else {
                    "No annotated nodes before this one.".to_string()
                };
            }
        }
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        // Mouse interactions clear transient nav/notification feedback the
        // same way keypresses do (see handle_normal_key) — otherwise stale
        // "at end" / clipboard messages could linger after a click or
        // scroll.
        self.notification = None;
        self.nav_feedback = None;
        // Swallow clicks when a popup is up or the user is typing into
        // an input mode — mouse activity shouldn't yank the selection
        // out from under their text entry.
        let popup_or_input_active = self.input_mode != InputMode::Normal
            || self.show_help
            || self.ast_view_scroll.is_some()
            || self.link_popup_urls.is_some()
            || self.quit_confirm_pending;
        match mouse.kind {
            MouseEventKind::ScrollUp if !popup_or_input_active => self.move_node(-1),
            MouseEventKind::ScrollDown if !popup_or_input_active => self.move_node(1),
            MouseEventKind::Down(MouseButton::Left) if !popup_or_input_active => {
                self.handle_left_click(mouse.row, mouse.column);
            }
            _ => {}
        }
    }

    fn handle_left_click(&mut self, row: u16, col: u16) {
        let count = self.bump_click_count(row, col);
        let Some((node_idx, display_byte)) = self.mouse_to_target(row, col) else {
            // Click outside the list area or on a non-text row: leave
            // the click count alone (the next click on a real cell will
            // start fresh anyway via the cell-change reset above).
            return;
        };
        let (unit, unit_idx) = self.click_target_unit(node_idx, display_byte, count);
        let anchor = SelectionAnchor::new(node_idx, unit, unit_idx);
        self.selection_state.anchor = anchor;
        self.refresh_section_highlight(anchor);
        self.status = format!(
            "Node {}/{}  {} {}",
            node_idx + 1,
            self.doc.node_count(),
            unit.mode_str(),
            unit_idx + 1,
        );
    }

    /// Increment the click count when the new click is on the same cell
    /// within `CLICK_DOUBLE_INTERVAL` of the previous one; otherwise
    /// reset to 1. Saturates at 3 so a fourth rapid click cycles back
    /// to a single-click selection (matches platform-typical behaviour).
    pub(super) fn bump_click_count(&mut self, row: u16, col: u16) -> u8 {
        let now = Instant::now();
        let count =
            match self.last_click {
                Some(prev)
                    if prev.row == row
                        && prev.col == col
                        && now.duration_since(prev.at) <= CLICK_DOUBLE_INTERVAL =>
                {
                    if prev.count >= 3 { 1 } else { prev.count + 1 }
                }
                _ => 1,
            };
        self.last_click = Some(LastClick {
            at: now,
            row,
            col,
            count,
        });
        count
    }

    /// Resolve a mouse coordinate to (node_idx, byte offset in display
    /// plain). Returns None for clicks outside `list_inner`, on spacer
    /// rows, or on the gutter/indicator prefix to the left of the text.
    fn mouse_to_target(&self, row: u16, col: u16) -> Option<(usize, usize)> {
        let inner = self.list_inner;
        if row < inner.y
            || row >= inner.y.saturating_add(inner.height)
            || col < inner.x
            || col >= inner.x.saturating_add(inner.width)
        {
            return None;
        }
        let visual_row = (row - inner.y) as usize;
        let map = self.visible_rows.get(visual_row)?.as_ref()?;
        let plain = self.rendered_nodes.get(map.node_idx)?.plain.as_str();
        if map.byte_range.start >= map.byte_range.end {
            // Spacer or empty row — no text to land on.
            return None;
        }
        // Skip the gutter cols, then walk the row's slice by terminal
        // width to find the byte the click landed on.
        let row_text = plain.get(map.byte_range.clone())?;
        let col_in_text = (col - inner.x).saturating_sub(map.gutter_cols) as usize;
        let local_byte = col_to_byte(row_text, col_in_text);
        Some((map.node_idx, map.byte_range.start + local_byte))
    }

    /// Pick the selection (unit, unit_idx) for a click at the given
    /// display byte on `node_idx`, dispatched by click count:
    ///   1 → Word, 2 → Sentence (or Line for nodes without sentence
    ///       semantics), 3 → Paragraph (whole node).
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

    /// True when the node has real sentence-level structure, i.e. its
    /// display plain contains terminal punctuation and `sentence_ranges`
    /// reflects more than a single whole-node fallback. Code blocks
    /// (which use `single_range`) and short list items / headings
    /// without a terminator fall through to Line on double-click.
    fn node_has_sentence_semantics(&self, node_idx: usize) -> bool {
        let Some(rn) = self.rendered_nodes.get(node_idx) else {
            return false;
        };
        if rn.sentence_ranges.is_empty() {
            return false;
        }
        match self.doc.nodes.get(node_idx) {
            Some(DocNode::CodeBlock { .. }) => false,
            Some(DocNode::Heading { .. }) | Some(DocNode::ListItem { .. }) => {
                rn.plain.chars().any(|c| matches!(c, '.' | '!' | '?'))
            }
            _ => true,
        }
    }

    fn has_annotation(&self, node_idx: usize) -> bool {
        self.changes.contains_key(&node_idx)
            || self.feedbacks.contains_key(&node_idx)
            || self.inserts_before.contains_key(&node_idx)
            || self.inserts_after.contains_key(&node_idx)
            || self.strikes.contains_key(&node_idx)
    }

    /// Snap the anchor to a valid Sentence position on the current node.
    ///
    /// Used by every "go to a different node" path (mouse click, mouse
    /// scroll, jump_to_annotation, search). Resets the active unit to
    /// Sentence so the unit_idx is interpreted consistently — without this
    /// reset, jumping to a different node while in Word/Line/Paragraph mode
    /// would leave a Sentence unit_idx in a slot the unit ought to read as
    /// a word/line/paragraph index.
    fn clamp_sentence(&mut self) {
        let total = self
            .rendered_nodes
            .get(self.selection_state.anchor.node_idx)
            .map_or(0, |rn| rn.sentence_ranges.len());
        let unit_idx = if total == 0 {
            0
        } else {
            self.selection_state.anchor.unit_idx.min(total - 1)
        };
        let new_anchor = SelectionAnchor::new(
            self.selection_state.anchor.node_idx,
            SelectionUnit::Sentence,
            unit_idx,
        );
        self.selection_state.anchor = new_anchor;
        // Forced unit change (always to Sentence) — clear any stale section
        // highlight from a prior Section-mode anchor; without this,
        // jump_to_annotation / mouse-click / search-jump from Section mode
        // would leave the section span painted.
        self.refresh_section_highlight(new_anchor);
    }

    fn current_source_line(&self) -> usize {
        // Return the source line where the active selection's text begins,
        // routed per the active unit. Used by status messages so the line
        // number shown matches the captured annotation's WHERE: line.
        let node_idx = self.selection_state.anchor.node_idx;
        let node_first_line = self
            .doc
            .nodes
            .get(node_idx)
            .map_or(0, |n| n.source_start_line());
        self.where_for_annotation(
            self.selection_state.anchor.unit,
            node_idx,
            Some(self.selection_state.anchor.unit_idx),
            node_first_line,
        )
    }

    pub(super) fn current_sentence_context(&self) -> Option<(usize, String)> {
        // Per modular_plan §"Internal representation" Req 11: emit consumes
        // the selection-plain-text view (markers stripped), not the display
        // view. Reading from rendered_nodes here used to leak `[ ]` task
        // markers, `[^N]` footnote refs, etc., into the captured target.
        let unit_idx = self.selection_state.anchor.unit_idx;
        let node = self.index.nodes.get(self.selection_state.anchor.node_idx)?;
        let range = node.sentence_ranges.get(unit_idx)?;
        let text = node
            .selection_plain_text
            .get(range.clone())?
            .trim()
            .to_string();
        Some((unit_idx, text))
    }

    /// Capture the `(unit_idx, target_text)` snapshot stored on the
    /// annotation. Routes by `selection_state.anchor.unit`:
    ///   - Sentence: rendered-display sentence text via current_sentence_context.
    ///   - Line: source line verbatim for non-ListItem; full item text
    ///     (markers stripped, soft-wrapped lines space-joined) for ListItem.
    ///   - Word: word's selection plain text (punctuation stripped per
    ///     word-boundary rules).
    ///   - Paragraph: full node selection plain text, internal newlines
    ///     collapsed to spaces.
    ///   - Section: constituent nodes' selection plain text, joined with
    ///     single spaces, internal newlines collapsed.
    fn current_target_capture(&self) -> Option<(usize, String)> {
        match self.selection_state.anchor.unit {
            SelectionUnit::Line => self.current_line_capture(),
            SelectionUnit::Word => self.current_word_capture(),
            SelectionUnit::Paragraph => self.current_paragraph_capture(),
            SelectionUnit::Section => self.current_section_capture(),
            SelectionUnit::Sentence => self.current_sentence_context(),
        }
    }

    fn current_paragraph_capture(&self) -> Option<(usize, String)> {
        let node_idx = self.selection_state.anchor.node_idx;
        let plain = self
            .index
            .nodes
            .get(node_idx)
            .map(|n| n.selection_plain_text.clone())?;
        // Per modular_plan §"target": Paragraph emit is single-line. The
        // index stores tables and other multi-line paragraph plain text
        // joined by `\n` for line-unit navigation; the emit collapses that
        // back to single space.
        Some((0, plain.replace('\n', " ")))
    }

    fn current_section_capture(&self) -> Option<(usize, String)> {
        let node_idx = self.selection_state.anchor.node_idx;
        let section = self
            .index
            .sections
            .iter()
            .find(|s| s.start_node_idx == node_idx)?;
        let mut parts: Vec<String> = Vec::new();
        for i in section.start_node_idx..=section.end_node_idx {
            if let Some(n) = self.index.nodes.get(i)
                && !n.selection_plain_text.is_empty()
            {
                // Constituent node text may contain `\n` (multi-line
                // paragraph or table) — collapse to single space per
                // modular_plan §"Section": no embedded newlines in
                // target:.
                parts.push(n.selection_plain_text.replace('\n', " "));
            }
        }
        Some((0, parts.join(" ")))
    }

    fn current_word_capture(&self) -> Option<(usize, String)> {
        let node_idx = self.selection_state.anchor.node_idx;
        let unit_idx = self.selection_state.anchor.unit_idx;
        let node = self.index.nodes.get(node_idx)?;
        let range = node.word_ranges.get(unit_idx)?;
        let text = node.selection_plain_text.get(range.clone())?.to_string();
        Some((unit_idx, text))
    }

    fn current_line_capture(&self) -> Option<(usize, String)> {
        let node_idx = self.selection_state.anchor.node_idx;
        let unit_idx = self.selection_state.anchor.unit_idx;
        if let DocNode::ListItem { .. } = self.doc.nodes.get(node_idx)? {
            // ListItem at line unit: full item text, markers already
            // stripped by the index's selection_plain_text.
            let plain = self
                .index
                .nodes
                .get(node_idx)
                .map(|n| n.selection_plain_text.clone())?;
            Some((unit_idx, plain))
        } else {
            // Non-ListItem: source line verbatim.
            let (line, _) = self
                .index
                .nodes
                .get(node_idx)?
                .source_line_ranges
                .get(unit_idx)?
                .clone();
            let line_text = self.source_lines.get(line)?.clone();
            Some((unit_idx, line_text))
        }
    }

    // ── Annotations ───────────────────────────────────────────────────────────

    fn existing_change_for_cursor(&self) -> Option<usize> {
        let changes = self.changes.get(&self.selection_state.anchor.node_idx)?;
        // Sentence-keyed match only fires in Sentence mode; in any other
        // unit the unit_idx isn't a sentence index. The fallback returns
        // the most recent change on the node, which keeps `c` -> edit
        // working when the user is on a node that has any existing change.
        if self.selection_state.anchor.unit == SelectionUnit::Sentence
            && let Some(idx) = self.current_sentence_context().map(|(idx, _)| idx)
        {
            changes.iter().rposition(|c| c.sentence_index == Some(idx))
        } else {
            changes.len().checked_sub(1)
        }
    }

    fn existing_feedback_for_cursor(&self) -> Option<usize> {
        let feedbacks = self.feedbacks.get(&self.selection_state.anchor.node_idx)?;
        if self.selection_state.anchor.unit == SelectionUnit::Sentence
            && let Some(idx) = self.current_sentence_context().map(|(idx, _)| idx)
        {
            feedbacks
                .iter()
                .rposition(|f| f.sentence_index == Some(idx))
        } else {
            feedbacks.len().checked_sub(1)
        }
    }

    fn begin_change_or_edit(&mut self) {
        if let Some(change_idx) = self.existing_change_for_cursor()
            && let Some(change) = self
                .changes
                .get(&self.selection_state.anchor.node_idx)
                .and_then(|changes| changes.get(change_idx))
        {
            self.change_buffer = change.change.clone();
            self.input_mode =
                InputMode::EditChange(self.selection_state.anchor.node_idx, change_idx);
            self.status = "Edit mode: Enter saves, Esc cancels.".to_string();
            return;
        }
        self.input_mode = InputMode::Change;
        self.change_buffer.clear();
        self.status = "Change mode: type text and press Enter. Esc cancels.".to_string();
    }

    fn begin_feedback_or_edit(&mut self) {
        if let Some(feedback_idx) = self.existing_feedback_for_cursor()
            && let Some(feedback) = self
                .feedbacks
                .get(&self.selection_state.anchor.node_idx)
                .and_then(|feedbacks| feedbacks.get(feedback_idx))
        {
            self.feedback_buffer = feedback.feedback.clone();
            self.input_mode =
                InputMode::EditFeedback(self.selection_state.anchor.node_idx, feedback_idx);
            self.status = "Edit mode: Enter saves, Esc cancels.".to_string();
            return;
        }
        self.input_mode = InputMode::Feedback;
        self.feedback_buffer.clear();
        self.status = "Feedback mode: type text and press Enter. Esc cancels.".to_string();
    }

    fn begin_edit_annotation(&mut self) {
        match self.editable_annotation_at_cursor() {
            Some(EditableAnnotation::Change(change_idx)) => {
                let Some(change) = self
                    .changes
                    .get(&self.selection_state.anchor.node_idx)
                    .and_then(|changes| changes.get(change_idx))
                else {
                    self.status = "No change or feedback to edit on this node.".to_string();
                    return;
                };
                self.change_buffer = change.change.clone();
                self.input_mode =
                    InputMode::EditChange(self.selection_state.anchor.node_idx, change_idx);
                self.status = "Edit mode: Enter saves, Esc cancels.".to_string();
            }
            Some(EditableAnnotation::Feedback(feedback_idx)) => {
                let Some(feedback) = self
                    .feedbacks
                    .get(&self.selection_state.anchor.node_idx)
                    .and_then(|feedbacks| feedbacks.get(feedback_idx))
                else {
                    self.status = "No change or feedback to edit on this node.".to_string();
                    return;
                };
                self.feedback_buffer = feedback.feedback.clone();
                self.input_mode =
                    InputMode::EditFeedback(self.selection_state.anchor.node_idx, feedback_idx);
                self.status = "Edit mode: Enter saves, Esc cancels.".to_string();
            }
            None => {
                self.status = "No change or feedback to edit on this node.".to_string();
            }
        }
    }

    fn pick_editable_annotation<'a>(
        change: Option<(usize, &'a ChangeAnnotation)>,
        feedback: Option<(usize, &'a FeedbackAnnotation)>,
    ) -> Option<EditableAnnotation> {
        match (change, feedback) {
            (Some((change_idx, change)), Some((feedback_idx, feedback))) => {
                if change.created_at >= feedback.created_at {
                    Some(EditableAnnotation::Change(change_idx))
                } else {
                    Some(EditableAnnotation::Feedback(feedback_idx))
                }
            }
            (Some((change_idx, _)), None) => Some(EditableAnnotation::Change(change_idx)),
            (None, Some((feedback_idx, _))) => Some(EditableAnnotation::Feedback(feedback_idx)),
            (None, None) => None,
        }
    }

    fn editable_annotation_at_cursor(&self) -> Option<EditableAnnotation> {
        // Sentence-keyed match only fires in Sentence mode; in any other
        // unit the unit_idx is not a sentence index. Mirrors the same
        // gate applied in existing_change/feedback_for_cursor.
        let sentence_idx = if self.selection_state.anchor.unit == SelectionUnit::Sentence {
            self.current_sentence_context().map(|(idx, _)| idx)
        } else {
            None
        };

        let sentence_match = sentence_idx.and_then(|idx| {
            let change = self
                .changes
                .get(&self.selection_state.anchor.node_idx)
                .and_then(|changes| {
                    changes
                        .iter()
                        .rposition(|c| c.sentence_index == Some(idx))
                        .map(|change_idx| (change_idx, &changes[change_idx]))
                });
            let feedback = self
                .feedbacks
                .get(&self.selection_state.anchor.node_idx)
                .and_then(|feedbacks| {
                    feedbacks
                        .iter()
                        .rposition(|f| f.sentence_index == Some(idx))
                        .map(|feedback_idx| (feedback_idx, &feedbacks[feedback_idx]))
                });
            Self::pick_editable_annotation(change, feedback)
        });

        sentence_match.or_else(|| {
            let change = self
                .changes
                .get(&self.selection_state.anchor.node_idx)
                .and_then(|changes| {
                    changes
                        .len()
                        .checked_sub(1)
                        .map(|change_idx| (change_idx, &changes[change_idx]))
                });
            let feedback = self
                .feedbacks
                .get(&self.selection_state.anchor.node_idx)
                .and_then(|feedbacks| {
                    feedbacks
                        .len()
                        .checked_sub(1)
                        .map(|feedback_idx| (feedback_idx, &feedbacks[feedback_idx]))
                });
            Self::pick_editable_annotation(change, feedback)
        })
    }

    fn remove_selected_annotation(&mut self) -> bool {
        let node_idx = self.selection_state.anchor.node_idx;
        match self.editable_annotation_at_cursor() {
            Some(EditableAnnotation::Change(change_idx)) => {
                let Some(changes) = self.changes.get_mut(&node_idx) else {
                    return false;
                };
                if change_idx >= changes.len() {
                    return false;
                }
                changes.remove(change_idx);
                if changes.is_empty() {
                    self.changes.remove(&node_idx);
                }
                self.status = format!("Removed change from node {}.", node_idx + 1);
                true
            }
            Some(EditableAnnotation::Feedback(feedback_idx)) => {
                let Some(feedbacks) = self.feedbacks.get_mut(&node_idx) else {
                    return false;
                };
                if feedback_idx >= feedbacks.len() {
                    return false;
                }
                feedbacks.remove(feedback_idx);
                if feedbacks.is_empty() {
                    self.feedbacks.remove(&node_idx);
                }
                self.status = format!("Removed feedback from node {}.", node_idx + 1);
                true
            }
            None => false,
        }
    }

    fn handle_edit_change_key(&mut self, key: KeyEvent, node_idx: usize, change_idx: usize) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.change_buffer.clear();
                self.status = "Edit cancelled.".to_string();
            }
            KeyCode::Enter => {
                let trimmed = self.change_buffer.trim().to_string();
                if trimmed.is_empty() {
                    self.status = "Edit ignored — change cannot be empty.".to_string();
                } else if let Some(changes) = self.changes.get_mut(&node_idx)
                    && let Some(annotation) = changes.get_mut(change_idx)
                {
                    annotation.change = trimmed;
                    self.status = format!("Change updated on node {}.", node_idx + 1);
                }
                self.input_mode = InputMode::Normal;
                self.change_buffer.clear();
            }
            KeyCode::Backspace => {
                self.change_buffer.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.change_buffer.push(ch);
            }
            _ => {}
        }
    }

    fn handle_edit_feedback_key(&mut self, key: KeyEvent, node_idx: usize, feedback_idx: usize) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.feedback_buffer.clear();
                self.status = "Edit cancelled.".to_string();
            }
            KeyCode::Enter => {
                let trimmed = self.feedback_buffer.trim().to_string();
                if trimmed.is_empty() {
                    self.status = "Edit ignored — feedback cannot be empty.".to_string();
                } else if let Some(feedbacks) = self.feedbacks.get_mut(&node_idx)
                    && let Some(annotation) = feedbacks.get_mut(feedback_idx)
                {
                    annotation.feedback = trimmed;
                    self.status = format!("Feedback updated on node {}.", node_idx + 1);
                }
                self.input_mode = InputMode::Normal;
                self.feedback_buffer.clear();
            }
            KeyCode::Backspace => {
                self.feedback_buffer.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.feedback_buffer.push(ch);
            }
            _ => {}
        }
    }

    fn toggle_strike(&mut self) {
        if self.remove_selected_annotation() {
            return;
        }

        let unit = self.selection_state.anchor.unit;
        let unit_idx = self.selection_state.anchor.unit_idx;
        let node_idx = self.selection_state.anchor.node_idx;

        // Verify the active anchor actually points at a real unit on this
        // node — otherwise an empty paragraph or out-of-range word index
        // would create a phantom strike.
        if self.current_target_capture().is_none() {
            self.status = format!(
                "Node {} has no {} target to strike.",
                node_idx + 1,
                unit.mode_str()
            );
            return;
        }

        let key = (unit, unit_idx);
        let entry = self.strikes.entry(node_idx).or_default();
        let unit_label = unit.mode_str();
        if entry.contains(&key) {
            entry.remove(&key);
            if entry.is_empty() {
                self.strikes.remove(&node_idx);
            }
            self.status = format!(
                "Removed strike from node {} ({unit_label} {}).",
                node_idx + 1,
                unit_idx + 1
            );
        } else {
            entry.insert(key);
            self.status = format!(
                "Struck node {} ({unit_label} {}).",
                node_idx + 1,
                unit_idx + 1
            );
        }
    }

    fn reveal_links_for_current_sentence(&mut self) -> bool {
        let urls = self.current_sentence_links();
        if urls.is_empty() {
            return false;
        }
        let count = urls.len();
        self.link_popup_urls = Some(urls);
        self.status = format!("Showing {count} link(s) from current sentence.");
        true
    }

    pub(super) fn current_sentence_links(&self) -> Vec<String> {
        let Some(rn) = self
            .rendered_nodes
            .get(self.selection_state.anchor.node_idx)
        else {
            return Vec::new();
        };
        // In Sentence mode, scope to the current sentence's byte range.
        // In any other mode, fall back to "all links in the current node"
        // — the unit_idx is a Word/Line/Paragraph/Section index that
        // doesn't translate cleanly to sentence_ranges.
        let scope: Option<Range<usize>> =
            if self.selection_state.anchor.unit == SelectionUnit::Sentence {
                rn.sentence_ranges
                    .get(self.selection_state.anchor.unit_idx)
                    .cloned()
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

    pub(super) fn annotation_counts(&self) -> (usize, usize, usize, usize) {
        let changes: usize = self.changes.values().map(|v| v.len()).sum();
        let feedbacks: usize = self.feedbacks.values().map(|v| v.len()).sum();
        let inserts: usize = self.inserts_before.values().map(|v| v.len()).sum::<usize>()
            + self.inserts_after.values().map(|v| v.len()).sum::<usize>();
        let strikes: usize = self.strikes.values().map(|v| v.len()).sum();
        (changes, feedbacks, inserts, strikes)
    }
}
