use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;

use super::*;

pub(crate) struct RenderState<'a> {
    pub(crate) source_path: &'a Path,
    pub(crate) view: &'a DocumentView,
    pub(crate) selection_state: SelectionState,
    pub(crate) section_highlight_range: Option<Range<usize>>,
    pub(in crate::app) input_mode: &'a InputMode,
    pub(crate) status: &'a str,
    pub(crate) notification: Option<&'a str>,
    pub(crate) nav_feedback: Option<&'a str>,
    pub(crate) quit_confirm_pending: bool,
    pub(crate) show_help: bool,
    pub(crate) ast_view_scroll: Option<u16>,
    pub(crate) ast_lines: &'a [String],
    pub(crate) link_popup_urls: Option<&'a [String]>,
    pub(crate) change_buffer: &'a str,
    pub(crate) feedback_buffer: &'a str,
    pub(crate) insert_buffer: &'a str,
    pub(crate) search_buffer: &'a str,
    pub(crate) cached_node_heights: &'a [u16],
    pub(crate) scroll_offset: usize,
    pub(crate) mode_indicator: &'static str,
    pub(in crate::app) changes: &'a BTreeMap<usize, Vec<ChangeAnnotation>>,
    pub(in crate::app) feedbacks: &'a BTreeMap<usize, Vec<FeedbackAnnotation>>,
    pub(in crate::app) inserts_before: &'a BTreeMap<usize, Vec<InsertAnnotation>>,
    pub(in crate::app) inserts_after: &'a BTreeMap<usize, Vec<InsertAnnotation>>,
    pub(in crate::app) strikes: &'a BTreeMap<usize, BTreeSet<(SelectionUnit, usize)>>,
}

impl RenderState<'_> {
    pub(crate) fn input_popup_spec(
        &self,
    ) -> Option<(&'static str, Option<&'static str>, &'static str, &str)> {
        match self.input_mode {
            InputMode::Change => Some((" Change (literal) ", None, "> ", self.change_buffer)),
            InputMode::EditChange(..) => {
                Some((" Edit Change (literal) ", None, "> ", self.change_buffer))
            }
            InputMode::Feedback => Some((" Feedback (Intent) ", None, "> ", self.feedback_buffer)),
            InputMode::EditFeedback(..) => {
                Some((" Edit Feedback (Intent) ", None, "> ", self.feedback_buffer))
            }
            InputMode::InsertBefore => {
                Some((" Insert Before (literal) ", None, "> ", self.insert_buffer))
            }
            InputMode::InsertAfter => {
                Some((" Insert After (literal) ", None, "> ", self.insert_buffer))
            }
            InputMode::Search => Some((
                " Search ",
                Some("Search: Enter jump | Esc cancel | n/N next/prev"),
                "/",
                self.search_buffer,
            )),
            InputMode::Normal => None,
        }
    }

    pub(crate) fn annotation_counts(&self) -> (usize, usize, usize, usize) {
        let changes: usize = self.changes.values().map(|v| v.len()).sum();
        let feedbacks: usize = self.feedbacks.values().map(|v| v.len()).sum();
        let inserts: usize = self.inserts_before.values().map(|v| v.len()).sum::<usize>()
            + self.inserts_after.values().map(|v| v.len()).sum::<usize>();
        let strikes: usize = self.strikes.values().map(|v| v.len()).sum();
        (changes, feedbacks, inserts, strikes)
    }

    pub(crate) fn annotation_counts_for(&self, node_idx: usize) -> (usize, usize, usize, usize) {
        let changes = self.changes.get(&node_idx).map_or(0, |v| v.len());
        let feedbacks = self.feedbacks.get(&node_idx).map_or(0, |v| v.len());
        let inserts = self.inserts_before.get(&node_idx).map_or(0, |v| v.len())
            + self.inserts_after.get(&node_idx).map_or(0, |v| v.len());
        let strikes = self.strikes.get(&node_idx).map_or(0, |v| v.len());
        (changes, feedbacks, inserts, strikes)
    }

    pub(crate) fn strike_units_for(&self, node_idx: usize) -> Vec<(SelectionUnit, usize)> {
        self.strikes
            .get(&node_idx)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    pub(crate) fn section_highlight_active(&self, node_idx: usize) -> bool {
        self.section_highlight_range
            .as_ref()
            .is_some_and(|range| range.contains(&node_idx))
    }
}

impl App {
    pub(super) fn render_state(&self) -> RenderState<'_> {
        RenderState {
            source_path: &self.source_path,
            view: &self.view,
            selection_state: self.selection_state,
            section_highlight_range: self.section_highlight_range.clone(),
            input_mode: &self.input_mode,
            status: &self.status,
            notification: self.notification.as_deref(),
            nav_feedback: self.nav_feedback.as_deref(),
            quit_confirm_pending: self.quit_confirm_pending,
            show_help: self.show_help,
            ast_view_scroll: self.ast_view_scroll,
            ast_lines: &self.ast_lines,
            link_popup_urls: self.link_popup_urls.as_deref(),
            change_buffer: &self.change_buffer,
            feedback_buffer: &self.feedback_buffer,
            insert_buffer: &self.insert_buffer,
            search_buffer: &self.search_buffer,
            cached_node_heights: &self.render_cache.node_heights,
            scroll_offset: self.scroll_offset,
            mode_indicator: self.mode_indicator(),
            changes: &self.changes,
            feedbacks: &self.feedbacks,
            inserts_before: &self.inserts_before,
            inserts_after: &self.inserts_after,
            strikes: &self.strikes,
        }
    }
}
