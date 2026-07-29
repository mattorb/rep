use std::ops::Range;

use crate::review::annotation::{
    AnnotationStore, ChangeAnnotation, EditableAnnotation, FeedbackAnnotation, InsertAnnotation,
};
use crate::review::command::{CommandResult, ReviewCommand};
use crate::review::document::ReviewDocument;
use crate::selection::model::{NavOutcome, SelectionAnchor, SelectionState, SelectionUnit};

/// Canonical review state and interaction semantics shared by every frontend.
///
/// Frontends own input modes, buffers, layout, and status presentation. They
/// send commands here and render the resulting state.
#[derive(Debug, Clone)]
pub(crate) struct ReviewSession {
    pub(crate) selection_state: SelectionState,
    pub(crate) section_highlight_range: Option<Range<usize>>,
    pub(crate) annotations: AnnotationStore,
    pub(crate) last_search: Option<String>,
    pub(crate) nav_feedback: Option<String>,
}

impl ReviewSession {
    pub(crate) fn new(initial_anchor: SelectionAnchor) -> Self {
        Self {
            selection_state: SelectionState::new(initial_anchor),
            section_highlight_range: None,
            annotations: AnnotationStore::default(),
            last_search: None,
            nav_feedback: None,
        }
    }

    pub(crate) const fn anchor(&self) -> SelectionAnchor {
        self.selection_state.anchor
    }

    pub(crate) const fn mode_indicator(&self) -> &'static str {
        self.selection_state.anchor.unit.mode_str()
    }

    pub(crate) fn apply(
        &mut self,
        document: &impl ReviewDocument,
        command: ReviewCommand,
    ) -> CommandResult {
        match command {
            ReviewCommand::MoveNode { delta } => self.move_node(document, delta),
            ReviewCommand::MoveActiveUnit { forward } => {
                self.move_active_unit(document, forward);
                CommandResult::default()
            }
            ReviewCommand::CycleUnit { forward } => {
                self.cycle_unit(document, forward);
                CommandResult::default()
            }
            ReviewCommand::AdjustUnit { finer } => {
                self.adjust_unit(document, finer);
                CommandResult::default()
            }
            ReviewCommand::Search { query, forward } => self.search(document, query, forward),
            ReviewCommand::JumpSearch { forward } => self.jump_search(document, forward),
            ReviewCommand::JumpAnnotation { forward } => self.jump_annotation(document, forward),
        }
    }

    pub(crate) fn set_anchor(&mut self, document: &impl ReviewDocument, anchor: SelectionAnchor) {
        self.selection_state.anchor = anchor;
        self.refresh_section_highlight(document);
    }

    pub(crate) fn clear_navigation_feedback(&mut self) {
        self.nav_feedback = None;
    }

    pub(crate) fn add_change(
        &mut self,
        document: &impl ReviewDocument,
        created_at: String,
        change: String,
    ) -> String {
        let target = document.capture_target(self.anchor());
        let node_idx = target.anchor.node_idx;
        self.annotations
            .changes
            .entry(node_idx)
            .or_default()
            .push(ChangeAnnotation {
                created_at,
                target_unit: target.anchor.unit,
                sentence_index: target.text.as_ref().map(|_| target.anchor.unit_idx),
                sentence_text: target.text,
                change,
            });
        format!(
            "Change saved on node {} (line {}).",
            node_idx + 1,
            target.locator.source_line() + 1
        )
    }

    pub(crate) fn add_feedback(
        &mut self,
        document: &impl ReviewDocument,
        created_at: String,
        feedback: String,
    ) -> String {
        let target = document.capture_target(self.anchor());
        let node_idx = target.anchor.node_idx;
        self.annotations
            .feedbacks
            .entry(node_idx)
            .or_default()
            .push(FeedbackAnnotation {
                created_at,
                target_unit: target.anchor.unit,
                sentence_index: target.text.as_ref().map(|_| target.anchor.unit_idx),
                sentence_text: target.text,
                feedback,
            });
        format!(
            "Feedback saved on node {} (line {}).",
            node_idx + 1,
            target.locator.source_line() + 1
        )
    }

    pub(crate) fn add_insert(
        &mut self,
        document: &impl ReviewDocument,
        created_at: String,
        text: String,
        before: bool,
    ) -> String {
        let target = document.capture_target(self.anchor());
        let node_idx = target.anchor.node_idx;
        let annotation = InsertAnnotation {
            created_at,
            target_unit: target.anchor.unit,
            sentence_index: target.text.as_ref().map(|_| target.anchor.unit_idx),
            sentence_text: target.text,
            text,
        };
        let bucket = if before {
            &mut self.annotations.inserts_before
        } else {
            &mut self.annotations.inserts_after
        };
        bucket.entry(node_idx).or_default().push(annotation);
        let label = if before { "before" } else { "after" };
        format!(
            "Insert {label} saved on node {} (line {}).",
            node_idx + 1,
            target.locator.source_line() + 1
        )
    }

    pub(crate) fn existing_change_for_cursor(
        &self,
        document: &impl ReviewDocument,
    ) -> Option<usize> {
        let changes = self.annotations.changes.get(&self.anchor().node_idx)?;
        if self.anchor().unit == SelectionUnit::Sentence
            && let Some(index) = document.sentence_index_for_anchor(self.anchor())
        {
            changes
                .iter()
                .rposition(|change| change.sentence_index == Some(index))
        } else {
            changes.len().checked_sub(1)
        }
    }

    pub(crate) fn existing_feedback_for_cursor(
        &self,
        document: &impl ReviewDocument,
    ) -> Option<usize> {
        let feedbacks = self.annotations.feedbacks.get(&self.anchor().node_idx)?;
        if self.anchor().unit == SelectionUnit::Sentence
            && let Some(index) = document.sentence_index_for_anchor(self.anchor())
        {
            feedbacks
                .iter()
                .rposition(|feedback| feedback.sentence_index == Some(index))
        } else {
            feedbacks.len().checked_sub(1)
        }
    }

    pub(crate) fn editable_annotation_at_cursor(
        &self,
        document: &impl ReviewDocument,
    ) -> Option<EditableAnnotation> {
        let sentence_match =
            document
                .sentence_index_for_anchor(self.anchor())
                .and_then(|sentence_idx| {
                    let change = self
                        .annotations
                        .changes
                        .get(&self.anchor().node_idx)
                        .and_then(|changes| {
                            changes
                                .iter()
                                .rposition(|change| change.sentence_index == Some(sentence_idx))
                                .map(|index| (index, &changes[index]))
                        });
                    let feedback = self
                        .annotations
                        .feedbacks
                        .get(&self.anchor().node_idx)
                        .and_then(|feedbacks| {
                            feedbacks
                                .iter()
                                .rposition(|feedback| feedback.sentence_index == Some(sentence_idx))
                                .map(|index| (index, &feedbacks[index]))
                        });
                    Self::pick_editable_annotation(change, feedback)
                });

        sentence_match.or_else(|| {
            let change = self
                .annotations
                .changes
                .get(&self.anchor().node_idx)
                .and_then(|changes| {
                    changes
                        .len()
                        .checked_sub(1)
                        .map(|index| (index, &changes[index]))
                });
            let feedback = self
                .annotations
                .feedbacks
                .get(&self.anchor().node_idx)
                .and_then(|feedbacks| {
                    feedbacks
                        .len()
                        .checked_sub(1)
                        .map(|index| (index, &feedbacks[index]))
                });
            Self::pick_editable_annotation(change, feedback)
        })
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

    pub(crate) fn remove_selected_annotation(
        &mut self,
        document: &impl ReviewDocument,
    ) -> Option<String> {
        let node_idx = self.anchor().node_idx;
        match self.editable_annotation_at_cursor(document) {
            Some(EditableAnnotation::Change(change_idx)) => {
                let changes = self.annotations.changes.get_mut(&node_idx)?;
                if change_idx >= changes.len() {
                    return None;
                }
                changes.remove(change_idx);
                if changes.is_empty() {
                    self.annotations.changes.remove(&node_idx);
                }
                Some(format!("Removed change from node {}.", node_idx + 1))
            }
            Some(EditableAnnotation::Feedback(feedback_idx)) => {
                let feedbacks = self.annotations.feedbacks.get_mut(&node_idx)?;
                if feedback_idx >= feedbacks.len() {
                    return None;
                }
                feedbacks.remove(feedback_idx);
                if feedbacks.is_empty() {
                    self.annotations.feedbacks.remove(&node_idx);
                }
                Some(format!("Removed feedback from node {}.", node_idx + 1))
            }
            None => None,
        }
    }

    pub(crate) fn update_change(
        &mut self,
        node_idx: usize,
        change_idx: usize,
        change: String,
    ) -> Option<String> {
        self.annotations
            .changes
            .get_mut(&node_idx)?
            .get_mut(change_idx)?
            .change = change;
        Some(format!("Change updated on node {}.", node_idx + 1))
    }

    pub(crate) fn update_feedback(
        &mut self,
        node_idx: usize,
        feedback_idx: usize,
        feedback: String,
    ) -> Option<String> {
        self.annotations
            .feedbacks
            .get_mut(&node_idx)?
            .get_mut(feedback_idx)?
            .feedback = feedback;
        Some(format!("Feedback updated on node {}.", node_idx + 1))
    }

    pub(crate) fn toggle_strike(&mut self, document: &impl ReviewDocument) -> String {
        if let Some(status) = self.remove_selected_annotation(document) {
            return status;
        }

        let anchor = self.anchor();
        if !document.has_target(anchor) {
            return format!(
                "Node {} has no {} target to strike.",
                anchor.node_idx + 1,
                anchor.unit.mode_str()
            );
        }

        let key = (anchor.unit, anchor.unit_idx);
        let entry = self.annotations.strikes.entry(anchor.node_idx).or_default();
        if entry.contains(&key) {
            entry.remove(&key);
            if entry.is_empty() {
                self.annotations.strikes.remove(&anchor.node_idx);
            }
            format!(
                "Removed strike from node {} ({} {}).",
                anchor.node_idx + 1,
                anchor.unit.mode_str(),
                anchor.unit_idx + 1
            )
        } else {
            entry.insert(key);
            format!(
                "Struck node {} ({} {}).",
                anchor.node_idx + 1,
                anchor.unit.mode_str(),
                anchor.unit_idx + 1
            )
        }
    }

    fn move_node(&mut self, document: &impl ReviewDocument, delta: isize) -> CommandResult {
        if document.node_count() == 0 || delta == 0 {
            return CommandResult::default();
        }
        let steps = delta.unsigned_abs();
        let forward = delta.is_positive();
        let mut target = self.anchor().node_idx;
        let mut moved = 0usize;

        for _ in 0..steps {
            let next = if forward {
                document.next_content_node(target.saturating_add(1))
            } else {
                document.prev_content_node(target)
            };
            let Some(idx) = next else { break };
            target = idx;
            moved += 1;
        }

        if moved == 0 {
            return CommandResult::with_status(if forward {
                "Already at the last node."
            } else {
                "Already at the first node."
            });
        }
        self.selection_state.anchor.node_idx = target;
        self.clamp_sentence(document);
        CommandResult::with_status(format!(
            "Node {}/{}",
            self.selection_state.anchor.node_idx + 1,
            document.node_count()
        ))
    }

    fn move_active_unit(&mut self, document: &impl ReviewDocument, forward: bool) {
        if document.node_count() == 0 {
            return;
        }
        match document.navigate(self.anchor(), forward) {
            NavOutcome::Moved(anchor) => self.set_anchor(document, anchor),
            NavOutcome::Boundary => {
                if document.has_any_anchor(self.anchor().unit) {
                    self.nav_feedback =
                        Some(if forward { "at end" } else { "at start" }.to_string());
                }
            }
        }
    }

    fn cycle_unit(&mut self, document: &impl ReviewDocument, forward: bool) {
        let order = SelectionUnit::CYCLE_ORDER;
        let current = order
            .iter()
            .position(|unit| *unit == self.anchor().unit)
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % order.len()
        } else {
            (current + order.len() - 1) % order.len()
        };
        let anchor = document.clamp_anchor(self.anchor(), order[next]);
        self.set_anchor(document, anchor);
    }

    fn adjust_unit(&mut self, document: &impl ReviewDocument, finer: bool) {
        let order = SelectionUnit::CYCLE_ORDER;
        let current = order
            .iter()
            .position(|unit| *unit == self.anchor().unit)
            .unwrap_or(0);
        let target = if finer {
            order.get(current + 1).copied()
        } else if current > 0 {
            order.get(current - 1).copied()
        } else {
            None
        };
        let Some(target) = target else {
            return;
        };
        let anchor = document.clamp_anchor(self.anchor(), target);
        self.set_anchor(document, anchor);
    }

    fn search(
        &mut self,
        document: &impl ReviewDocument,
        query: String,
        forward: bool,
    ) -> CommandResult {
        let matches = document.search_matches(&query);
        self.last_search = Some(query.clone());
        if matches.is_empty() {
            return CommandResult::with_status(format!("No matches for \"{query}\"."));
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
        self.apply_search_target(document, &query, &matches, target_idx)
    }

    fn jump_search(&mut self, document: &impl ReviewDocument, forward: bool) -> CommandResult {
        let Some(query) = self.last_search.clone() else {
            return CommandResult::with_status("No previous search. Press / to search.");
        };
        let matches = document.search_matches(&query);
        if matches.is_empty() {
            return CommandResult::with_status(format!("No matches for \"{query}\"."));
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
        self.apply_search_target(document, &query, &matches, target_idx)
    }

    fn search_current_position(&self) -> (usize, usize) {
        let anchor = self.anchor();
        if anchor.unit == SelectionUnit::Sentence {
            (anchor.node_idx, anchor.unit_idx)
        } else {
            (anchor.node_idx, 0)
        }
    }

    fn apply_search_target(
        &mut self,
        document: &impl ReviewDocument,
        query: &str,
        matches: &[(usize, usize)],
        target_idx: usize,
    ) -> CommandResult {
        let (node_idx, sentence_idx) = matches[target_idx];
        self.selection_state.anchor =
            SelectionAnchor::new(node_idx, SelectionUnit::Sentence, sentence_idx);
        self.clamp_sentence(document);
        CommandResult::with_status(format!(
            "Match {}/{} for \"{}\".",
            target_idx + 1,
            matches.len(),
            query
        ))
    }

    fn jump_annotation(&mut self, document: &impl ReviewDocument, forward: bool) -> CommandResult {
        let from = if forward {
            self.anchor().node_idx + 1
        } else {
            self.anchor().node_idx
        };
        let target = if forward {
            (from..document.node_count()).find(|&index| self.annotations.has_annotation(index))
        } else {
            (0..from)
                .rev()
                .find(|&index| self.annotations.has_annotation(index))
        };

        match target {
            Some(node_idx) => {
                self.selection_state.anchor.node_idx = node_idx;
                self.clamp_sentence(document);
                CommandResult::with_status(format!("Annotated node {}.", node_idx + 1))
            }
            None => CommandResult::with_status(if forward {
                "No annotated nodes after this one."
            } else {
                "No annotated nodes before this one."
            }),
        }
    }

    pub(crate) fn clamp_sentence(&mut self, document: &impl ReviewDocument) {
        let total = document.sentence_count_for_node(self.anchor().node_idx);
        let unit_idx = if total == 0 {
            0
        } else {
            self.anchor().unit_idx.min(total - 1)
        };
        self.selection_state.anchor =
            SelectionAnchor::new(self.anchor().node_idx, SelectionUnit::Sentence, unit_idx);
        self.refresh_section_highlight(document);
    }

    fn refresh_section_highlight(&mut self, document: &impl ReviewDocument) {
        self.section_highlight_range = if self.anchor().unit == SelectionUnit::Section {
            Some(document.section_span_for_start(self.anchor().node_idx))
        } else {
            None
        };
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::*;
    use crate::review::document::{
        ActionContext, CapturedTarget, DocumentFormat, NodeSourceContext, OutlineRow, ReviewLink,
        SourceLocator,
    };
    use crate::selection::index::SelectionIndex;

    #[derive(Debug)]
    struct SyntheticDocument {
        nodes: usize,
        anchors: BTreeMap<SelectionUnit, Vec<SelectionAnchor>>,
        search: BTreeMap<String, Vec<(usize, usize)>>,
    }

    impl SyntheticDocument {
        fn three_nodes() -> Self {
            let sentence_anchors = (0..3)
                .map(|node_idx| SelectionAnchor::new(node_idx, SelectionUnit::Sentence, 0))
                .collect();
            let word_anchors = (0..3)
                .flat_map(|node_idx| {
                    (0..2).map(move |unit_idx| {
                        SelectionAnchor::new(node_idx, SelectionUnit::Word, unit_idx)
                    })
                })
                .collect();
            Self {
                nodes: 3,
                anchors: BTreeMap::from([
                    (SelectionUnit::Sentence, sentence_anchors),
                    (SelectionUnit::Word, word_anchors),
                ]),
                search: BTreeMap::from([("hit".to_string(), vec![(0, 0), (2, 0)])]),
            }
        }

        fn anchors_for(&self, unit: SelectionUnit) -> &[SelectionAnchor] {
            self.anchors.get(&unit).map_or(&[], Vec::as_slice)
        }
    }

    impl ReviewDocument for SyntheticDocument {
        fn source_path(&self) -> &Path {
            Path::new("<synthetic>")
        }

        fn format(&self) -> DocumentFormat {
            DocumentFormat::Markdown
        }

        fn selection_index(&self) -> &SelectionIndex {
            static EMPTY: std::sync::LazyLock<SelectionIndex> =
                std::sync::LazyLock::new(SelectionIndex::default);
            &EMPTY
        }

        fn initial_anchor(&self) -> SelectionAnchor {
            SelectionAnchor::new(0, SelectionUnit::Sentence, 0)
        }

        fn node_count(&self) -> usize {
            self.nodes
        }

        fn next_content_node(&self, from: usize) -> Option<usize> {
            (from < self.nodes).then_some(from)
        }

        fn prev_content_node(&self, before: usize) -> Option<usize> {
            before.checked_sub(1)
        }

        fn navigate(&self, anchor: SelectionAnchor, forward: bool) -> NavOutcome {
            let anchors = self.anchors_for(anchor.unit);
            let Some(index) = anchors.iter().position(|candidate| *candidate == anchor) else {
                return NavOutcome::Boundary;
            };
            let target = if forward {
                index.checked_add(1).and_then(|next| anchors.get(next))
            } else {
                index
                    .checked_sub(1)
                    .and_then(|previous| anchors.get(previous))
            };
            target
                .copied()
                .map_or(NavOutcome::Boundary, NavOutcome::Moved)
        }

        fn clamp_anchor(&self, anchor: SelectionAnchor, target: SelectionUnit) -> SelectionAnchor {
            self.anchors_for(target)
                .iter()
                .copied()
                .find(|candidate| candidate.node_idx == anchor.node_idx)
                .or_else(|| self.anchors_for(target).first().copied())
                .unwrap_or(SelectionAnchor::new(anchor.node_idx, target, 0))
        }

        fn has_any_anchor(&self, unit: SelectionUnit) -> bool {
            !self.anchors_for(unit).is_empty()
        }

        fn section_span_for_start(&self, node_idx: usize) -> Range<usize> {
            node_idx..self.nodes
        }

        fn sentence_count_for_node(&self, _node_idx: usize) -> usize {
            1
        }

        fn sentence_index_for_anchor(&self, anchor: SelectionAnchor) -> Option<usize> {
            (anchor.unit == SelectionUnit::Sentence).then_some(anchor.unit_idx)
        }

        fn search_matches(&self, query: &str) -> Vec<(usize, usize)> {
            self.search.get(query).cloned().unwrap_or_default()
        }

        fn capture_target(&self, anchor: SelectionAnchor) -> CapturedTarget {
            CapturedTarget {
                anchor,
                text: Some(format!("node {}", anchor.node_idx)),
                locator: SourceLocator::MarkdownLine {
                    line: anchor.node_idx,
                },
            }
        }

        fn has_target(&self, anchor: SelectionAnchor) -> bool {
            anchor.node_idx < self.nodes
        }

        fn action_context(&self, target: &CapturedTarget) -> ActionContext {
            ActionContext {
                where_line: target.locator.source_line(),
                target: target.text.clone().unwrap_or_default(),
                previous: String::new(),
                next: String::new(),
                locator: target.locator.clone(),
            }
        }

        fn node_source_context(&self, node_idx: usize) -> NodeSourceContext {
            NodeSourceContext {
                source_line: node_idx,
                line_text: format!("node {node_idx}"),
                previous: None,
                next: None,
            }
        }

        fn links_for(&self, _anchor: SelectionAnchor) -> Vec<ReviewLink> {
            Vec::new()
        }

        fn node_outline(&self) -> Vec<OutlineRow> {
            Vec::new()
        }
    }

    #[test]
    fn command_transcript_keeps_navigation_search_and_boundaries_canonical() {
        let document = SyntheticDocument::three_nodes();
        let mut session = ReviewSession::new(SelectionAnchor::new(0, SelectionUnit::Sentence, 0));

        session.apply(&document, ReviewCommand::CycleUnit { forward: true });
        assert_eq!(
            session.anchor(),
            SelectionAnchor::new(0, SelectionUnit::Word, 0)
        );
        session.apply(&document, ReviewCommand::MoveActiveUnit { forward: true });
        assert_eq!(
            session.anchor(),
            SelectionAnchor::new(0, SelectionUnit::Word, 1)
        );

        let result = session.apply(
            &document,
            ReviewCommand::Search {
                query: "hit".to_string(),
                forward: true,
            },
        );
        assert_eq!(session.anchor().node_idx, 0);
        assert_eq!(session.anchor().unit, SelectionUnit::Sentence);
        assert_eq!(result.status.as_deref(), Some("Match 1/2 for \"hit\"."));

        session.apply(&document, ReviewCommand::JumpSearch { forward: true });
        assert_eq!(session.anchor().node_idx, 2);
        session.apply(&document, ReviewCommand::MoveActiveUnit { forward: true });
        assert_eq!(session.nav_feedback.as_deref(), Some("at end"));
    }

    #[test]
    fn annotation_jump_uses_shared_store_and_resets_to_sentence() {
        let document = SyntheticDocument::three_nodes();
        let mut session = ReviewSession::new(SelectionAnchor::new(0, SelectionUnit::Word, 1));
        session.annotations.changes.insert(2, Vec::new());

        let result = session.apply(&document, ReviewCommand::JumpAnnotation { forward: true });

        assert_eq!(
            session.anchor(),
            SelectionAnchor::new(2, SelectionUnit::Sentence, 0)
        );
        assert_eq!(result.status.as_deref(), Some("Annotated node 3."));
    }
}
