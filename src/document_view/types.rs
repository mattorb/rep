use std::ops::Range;

use ratatui::prelude::*;

use crate::selection::model::{SelectionAnchor, SelectionUnit};

/// Maps a single visible terminal row to a slice of one rendered node's display
/// plain text. Built each `draw()` from node rows so `handle_mouse` can resolve
/// click coordinates without re-walking the wrap pipeline.
#[derive(Debug, Clone)]
pub(crate) struct VisibleRowMap {
    pub(crate) node_idx: usize,
    /// Byte range in the node's rendered display text covering the chars shown
    /// on this row after the gutter prefix. Zero-width for spacer rows.
    pub(crate) byte_range: Range<usize>,
    /// Terminal columns to skip from the left edge of `list_inner` before text.
    pub(crate) gutter_cols: u16,
}

#[derive(Debug, Clone)]
pub(crate) struct SourceLineContext {
    pub(crate) source_line: usize,
    pub(crate) line_text: String,
    pub(crate) previous_line: Option<String>,
    pub(crate) next_line: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct AnnotationTargetCapture {
    pub(crate) sentence_index: Option<usize>,
    pub(crate) sentence_text: Option<String>,
    pub(crate) source_line: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct SourceActionContext {
    pub(crate) where_line: usize,
    pub(crate) target: String,
    pub(crate) previous_line: String,
    pub(crate) next_line: String,
}

#[derive(Debug, Clone)]
pub(super) struct CodeBlockRenderLine<'a> {
    pub(super) source_line: usize,
    pub(super) text: &'a str,
    pub(super) byte_range: Range<usize>,
    pub(super) is_fence: bool,
}

pub(crate) struct WrappedDisplayRow {
    pub(crate) spans: Vec<Span<'static>>,
    pub(crate) byte_range: Range<usize>,
}

pub(crate) struct CodeBlockDisplayRow {
    pub(crate) spans: Vec<Span<'static>>,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DisplaySpanStyleRequest<'a> {
    pub(crate) node_idx: usize,
    pub(crate) active_anchor: SelectionAnchor,
    pub(crate) section_highlight_active: bool,
    pub(crate) strike_units: &'a [(SelectionUnit, usize)],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CodeBlockStyleRequest<'a> {
    pub(crate) node_idx: usize,
    pub(crate) active_anchor: SelectionAnchor,
    pub(crate) section_highlight_active: bool,
    pub(crate) strike_units: &'a [(SelectionUnit, usize)],
}

#[derive(Debug, Clone, Copy)]
pub(super) struct CodeBlockLineStyleRequest<'a> {
    pub(super) node_idx: usize,
    pub(super) source_line: usize,
    pub(super) line: &'a str,
    pub(super) base_style: Style,
    pub(super) active_anchor: SelectionAnchor,
    pub(super) section_highlight_active: bool,
    pub(super) strike_units: &'a [(SelectionUnit, usize)],
}
