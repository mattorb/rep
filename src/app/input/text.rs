use super::*;

impl App {
    pub(super) fn handle_change_key(&mut self, key: KeyEvent) {
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
                    self.status =
                        self.review
                            .add_change(&self.view, Utc::now().to_rfc3339(), trimmed);
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

    pub(super) fn handle_insert_key(&mut self, key: KeyEvent, before: bool) {
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
                    self.status = self.review.add_insert(
                        &self.view,
                        Utc::now().to_rfc3339(),
                        trimmed,
                        before,
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

    pub(super) fn handle_search_key(&mut self, key: KeyEvent) {
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
    pub(super) fn handle_feedback_key(&mut self, key: KeyEvent) {
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
                    self.status =
                        self.review
                            .add_feedback(&self.view, Utc::now().to_rfc3339(), trimmed);
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

    pub(super) fn begin_change_or_edit(&mut self) {
        if let Some(change_idx) = self.review.existing_change_for_cursor(&self.view)
            && let Some(change) = self
                .review
                .annotations
                .changes
                .get(&self.review.selection_state.anchor.node_idx)
                .and_then(|changes| changes.get(change_idx))
        {
            self.change_buffer = change.change.clone();
            self.input_mode =
                InputMode::EditChange(self.review.selection_state.anchor.node_idx, change_idx);
            self.status = "Edit mode: Enter saves, Esc cancels.".to_string();
            return;
        }
        self.input_mode = InputMode::Change;
        self.change_buffer.clear();
        self.status = "Change mode: type text and press Enter. Esc cancels.".to_string();
    }

    pub(super) fn begin_feedback_or_edit(&mut self) {
        if let Some(feedback_idx) = self.review.existing_feedback_for_cursor(&self.view)
            && let Some(feedback) = self
                .review
                .annotations
                .feedbacks
                .get(&self.review.selection_state.anchor.node_idx)
                .and_then(|feedbacks| feedbacks.get(feedback_idx))
        {
            self.feedback_buffer = feedback.feedback.clone();
            self.input_mode =
                InputMode::EditFeedback(self.review.selection_state.anchor.node_idx, feedback_idx);
            self.status = "Edit mode: Enter saves, Esc cancels.".to_string();
            return;
        }
        self.input_mode = InputMode::Feedback;
        self.feedback_buffer.clear();
        self.status = "Feedback mode: type text and press Enter. Esc cancels.".to_string();
    }

    pub(super) fn begin_edit_annotation(&mut self) {
        match self.review.editable_annotation_at_cursor(&self.view) {
            Some(EditableAnnotation::Change(change_idx)) => {
                let Some(change) = self
                    .review
                    .annotations
                    .changes
                    .get(&self.review.selection_state.anchor.node_idx)
                    .and_then(|changes| changes.get(change_idx))
                else {
                    self.status = "No change or feedback to edit on this node.".to_string();
                    return;
                };
                self.change_buffer = change.change.clone();
                self.input_mode =
                    InputMode::EditChange(self.review.selection_state.anchor.node_idx, change_idx);
                self.status = "Edit mode: Enter saves, Esc cancels.".to_string();
            }
            Some(EditableAnnotation::Feedback(feedback_idx)) => {
                let Some(feedback) = self
                    .review
                    .annotations
                    .feedbacks
                    .get(&self.review.selection_state.anchor.node_idx)
                    .and_then(|feedbacks| feedbacks.get(feedback_idx))
                else {
                    self.status = "No change or feedback to edit on this node.".to_string();
                    return;
                };
                self.feedback_buffer = feedback.feedback.clone();
                self.input_mode = InputMode::EditFeedback(
                    self.review.selection_state.anchor.node_idx,
                    feedback_idx,
                );
                self.status = "Edit mode: Enter saves, Esc cancels.".to_string();
            }
            None => {
                self.status = "No change or feedback to edit on this node.".to_string();
            }
        }
    }

    pub(super) fn handle_edit_change_key(
        &mut self,
        key: KeyEvent,
        node_idx: usize,
        change_idx: usize,
    ) {
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
                } else if let Some(status) =
                    self.review.update_change(node_idx, change_idx, trimmed)
                {
                    self.status = status;
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

    pub(super) fn handle_edit_feedback_key(
        &mut self,
        key: KeyEvent,
        node_idx: usize,
        feedback_idx: usize,
    ) {
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
                } else if let Some(status) =
                    self.review.update_feedback(node_idx, feedback_idx, trimmed)
                {
                    self.status = status;
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

    pub(super) fn toggle_strike(&mut self) {
        self.status = self.review.toggle_strike(&self.view);
    }

    pub(super) fn reveal_links_for_current_sentence(&mut self) -> bool {
        let urls = self.current_sentence_links();
        if urls.is_empty() {
            return false;
        }
        let count = urls.len();
        self.link_popup_urls = Some(urls);
        self.status = format!("Showing {count} link(s) from current sentence.");
        true
    }

    pub(in crate::app) fn current_sentence_links(&self) -> Vec<String> {
        crate::review::document::ReviewDocument::links_for(
            &self.view,
            self.review.selection_state.anchor,
        )
        .into_iter()
        .map(|link| link.url)
        .collect()
    }
}
