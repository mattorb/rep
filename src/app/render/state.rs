use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;

use super::*;

pub(crate) struct RenderState<'a> {
    pub(crate) source_path: &'a Path,
    pub(crate) view: &'a DocumentView,
    pub(crate) selection_state: SelectionState,
    pub(crate) section_highlight_range: Option<Range<usize>>,
    pub(crate) input_mode: &'a InputMode,
    pub(crate) status: &'a str,
    pub(crate) notification: Option<&'a str>,
    pub(crate) nav_feedback: Option<&'a str>,
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
    pub(crate) changes: &'a BTreeMap<usize, Vec<ChangeAnnotation>>,
    pub(crate) feedbacks: &'a BTreeMap<usize, Vec<FeedbackAnnotation>>,
    pub(crate) inserts_before: &'a BTreeMap<usize, Vec<InsertAnnotation>>,
    pub(crate) inserts_after: &'a BTreeMap<usize, Vec<InsertAnnotation>>,
    pub(crate) strikes: &'a BTreeMap<usize, BTreeSet<(SelectionUnit, usize)>>,
}

impl RenderState<'_> {
    pub(crate) fn annotation_counts(&self) -> (usize, usize, usize, usize) {
        let changes: usize = self.changes.values().map(|v| v.len()).sum();
        let feedbacks: usize = self.feedbacks.values().map(|v| v.len()).sum();
        let inserts: usize = self.inserts_before.values().map(|v| v.len()).sum::<usize>()
            + self.inserts_after.values().map(|v| v.len()).sum::<usize>();
        let strikes: usize = self.strikes.values().map(|v| v.len()).sum();
        (changes, feedbacks, inserts, strikes)
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
            show_help: self.show_help,
            ast_view_scroll: self.ast_view_scroll,
            ast_lines: &self.ast_lines,
            link_popup_urls: self.link_popup_urls.as_deref(),
            change_buffer: &self.change_buffer,
            feedback_buffer: &self.feedback_buffer,
            insert_buffer: &self.insert_buffer,
            search_buffer: &self.search_buffer,
            cached_node_heights: &self.cached_node_heights,
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
