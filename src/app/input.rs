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
                    let target = self
                        .view
                        .annotation_target_capture(self.selection_state.anchor);
                    let annotation = ChangeAnnotation {
                        created_at: Utc::now().to_rfc3339(),
                        target_unit: self.selection_state.anchor.unit,
                        sentence_index: target.sentence_index,
                        sentence_text: target.sentence_text,
                        change: trimmed,
                    };
                    self.changes
                        .entry(self.selection_state.anchor.node_idx)
                        .or_default()
                        .push(annotation);
                    self.status = format!(
                        "Change saved on node {} (line {}).",
                        self.selection_state.anchor.node_idx + 1,
                        target.source_line + 1
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
                    let target = self
                        .view
                        .annotation_target_capture(self.selection_state.anchor);
                    let annotation = InsertAnnotation {
                        created_at: Utc::now().to_rfc3339(),
                        target_unit: self.selection_state.anchor.unit,
                        sentence_index: target.sentence_index,
                        sentence_text: target.sentence_text,
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
                        target.source_line + 1
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

    fn run_search(&mut self, query: &str, forward: bool) {
        let matches = self.view.search_matches(query);
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
        let matches = self.view.search_matches(&query);
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
        // search_matches returns sentence-keyed positions, so the
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
                    let target = self
                        .view
                        .annotation_target_capture(self.selection_state.anchor);
                    let annotation = FeedbackAnnotation {
                        created_at: Utc::now().to_rfc3339(),
                        target_unit: self.selection_state.anchor.unit,
                        sentence_index: target.sentence_index,
                        sentence_text: target.sentence_text,
                        feedback: trimmed,
                    };
                    self.feedbacks
                        .entry(self.selection_state.anchor.node_idx)
                        .or_default()
                        .push(annotation);
                    self.status = format!(
                        "Feedback saved on node {} (line {}).",
                        self.selection_state.anchor.node_idx + 1,
                        target.source_line + 1
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
        if self.view.node_count() == 0 || delta == 0 {
            return;
        }
        let steps = delta.unsigned_abs();
        let forward = delta.is_positive();
        let mut target = self.selection_state.anchor.node_idx;
        let mut moved = 0usize;

        for _ in 0..steps {
            let next = if forward {
                self.view.next_content_node(target.saturating_add(1))
            } else {
                self.view.prev_content_node(target)
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
            self.view.node_count()
        );
    }

    /// j / k / Down / Up / Right / Left — move by the currently active
    /// selection unit. Pure delegate to `selection::navigator::next/prev`.
    /// On `Boundary`, set `nav_feedback` ("at end" / "at start") for one
    /// keypress in the right zone of the footer.
    fn move_active_unit(&mut self, forward: bool) {
        if self.view.node_count() == 0 {
            return;
        }
        let outcome = self.view.navigate(self.selection_state.anchor, forward);
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
                if !self.view.has_any_anchor(self.selection_state.anchor.unit) {
                    return;
                }
                self.nav_feedback = Some(if forward { "at end" } else { "at start" }.to_string());
            }
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
        let new_anchor = self.view.clamp_anchor(self.selection_state.anchor, target);
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
        let new_anchor = self.view.clamp_anchor(self.selection_state.anchor, target);
        self.selection_state.anchor = new_anchor;
        self.refresh_section_highlight(new_anchor);
    }

    /// Refresh `section_highlight_range` for an anchor: set the span when
    /// the active unit is Section, clear otherwise. Used after every move
    /// or mode-cycle that lands on a new anchor.
    fn refresh_section_highlight(&mut self, anchor: SelectionAnchor) {
        if anchor.unit == SelectionUnit::Section {
            self.section_highlight_range = Some(self.view.section_span_for_start(anchor.node_idx));
        } else {
            self.section_highlight_range = None;
        }
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
        let n = self.view.node_count();
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
        let Some(anchor) = self.mouse_to_anchor(row, col, count) else {
            // Click outside the list area or on a non-text row: leave
            // the click count alone (the next click on a real cell will
            // start fresh anyway via the cell-change reset above).
            return;
        };
        self.selection_state.anchor = anchor;
        self.refresh_section_highlight(anchor);
        self.status = format!(
            "Node {}/{}  {} {}",
            anchor.node_idx + 1,
            self.view.node_count(),
            anchor.unit.mode_str(),
            anchor.unit_idx + 1,
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

    /// Resolve a mouse coordinate to a selection anchor. Returns None for
    /// clicks outside `list_inner`, on spacer rows, or on the
    /// gutter/indicator prefix to the left of the text.
    fn mouse_to_anchor(&self, row: u16, col: u16, click_count: u8) -> Option<SelectionAnchor> {
        self.view.hit_test(self.list_inner, row, col, click_count)
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
            .view
            .sentence_count_for_node(self.selection_state.anchor.node_idx);
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

    // ── Annotations ───────────────────────────────────────────────────────────

    fn existing_change_for_cursor(&self) -> Option<usize> {
        let changes = self.changes.get(&self.selection_state.anchor.node_idx)?;
        // Sentence-keyed match only fires in Sentence mode; in any other
        // unit the unit_idx isn't a sentence index. The fallback returns
        // the most recent change on the node, which keeps `c` -> edit
        // working when the user is on a node that has any existing change.
        if self.selection_state.anchor.unit == SelectionUnit::Sentence
            && let Some(idx) = self
                .view
                .sentence_index_for_anchor(self.selection_state.anchor)
        {
            changes.iter().rposition(|c| c.sentence_index == Some(idx))
        } else {
            changes.len().checked_sub(1)
        }
    }

    fn existing_feedback_for_cursor(&self) -> Option<usize> {
        let feedbacks = self.feedbacks.get(&self.selection_state.anchor.node_idx)?;
        if self.selection_state.anchor.unit == SelectionUnit::Sentence
            && let Some(idx) = self
                .view
                .sentence_index_for_anchor(self.selection_state.anchor)
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
        let sentence_idx = self
            .view
            .sentence_index_for_anchor(self.selection_state.anchor);

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
        if self
            .view
            .target_capture(self.selection_state.anchor)
            .is_none()
        {
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
        self.view.links_for_anchor(self.selection_state.anchor)
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
