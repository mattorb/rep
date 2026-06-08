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

    pub(super) fn begin_change_or_edit(&mut self) {
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

    pub(super) fn begin_feedback_or_edit(&mut self) {
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

    pub(super) fn begin_edit_annotation(&mut self) {
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

    pub(super) fn toggle_strike(&mut self) {
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
        self.view.links_for_anchor(self.selection_state.anchor)
    }
}
