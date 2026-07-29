use std::time::{Duration, Instant};

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
