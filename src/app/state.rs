use std::time::{Duration, Instant};

use crate::selection::model::SelectionUnit;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum InputMode {
    Normal,
    Change,
    Feedback,
    InsertBefore,
    InsertAfter,
    Search,
    /// Editing the change at (node_idx, change_idx).
    EditChange(usize, usize),
    /// Editing the feedback at (node_idx, feedback_idx).
    EditFeedback(usize, usize),
}

#[derive(Debug, Clone)]
pub(super) struct ChangeAnnotation {
    pub(super) created_at: String,
    /// Selection unit at the moment of capture (Sentence / Line / Paragraph
    /// / Section / Word). Drives WHERE: format and target: source per
    /// modular_plan §"target".
    pub(super) target_unit: SelectionUnit,
    pub(super) sentence_index: Option<usize>,
    pub(super) sentence_text: Option<String>,
    pub(super) change: String,
}

#[derive(Debug, Clone)]
pub(super) struct FeedbackAnnotation {
    pub(super) created_at: String,
    pub(super) target_unit: SelectionUnit,
    pub(super) sentence_index: Option<usize>,
    pub(super) sentence_text: Option<String>,
    pub(super) feedback: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EditableAnnotation {
    Change(usize),
    Feedback(usize),
}

#[derive(Debug, Clone)]
pub(super) struct InsertAnnotation {
    pub(super) created_at: String,
    pub(super) target_unit: SelectionUnit,
    pub(super) sentence_index: Option<usize>,
    pub(super) sentence_text: Option<String>,
    pub(super) text: String,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct LastClick {
    pub(super) at: Instant,
    pub(super) row: u16,
    pub(super) col: u16,
    /// 1 = single, 2 = double, 3 = triple. Saturates at 3 — a fourth
    /// rapid click on the same cell drops back to 1.
    pub(super) count: u8,
}

pub(super) const CLICK_DOUBLE_INTERVAL: Duration = Duration::from_millis(500);
