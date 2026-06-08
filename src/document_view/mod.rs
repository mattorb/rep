use std::ops::Range;

use anyhow::{Context, Result};
use ratatui::prelude::*;

use crate::document::{DocNode, Document};
use crate::selection::index::SelectionIndex;
use crate::selection::model::{NavOutcome, SelectionAnchor, SelectionUnit};
use crate::ui::wrap_styled_spans;

mod code;
mod context;
mod layout;
mod links;
mod rendered;
mod types;
pub(crate) use rendered::RenderedNode;
use rendered::{
    build_rendered_nodes, clamp_range, col_to_byte, count_occurrences_before, find_unit_at,
    newlines_before_byte, nth_occurrence, wrap_line_byte_ranges,
};
pub(crate) use types::{
    AnnotationTargetCapture, CodeBlockDisplayRow, CodeBlockStyleRequest, DisplaySpanStyleRequest,
    SourceActionContext, SourceLineContext, VisibleRowMap, WrappedDisplayRow,
};
use types::{CodeBlockLineStyleRequest, CodeBlockRenderLine};

/// Owns the parsed source and the derived views used by app input, rendering,
/// hit testing, and output context.
#[derive(Debug)]
pub(crate) struct DocumentView {
    document: Document,
    source_lines: Vec<String>,
    rendered_nodes: Vec<RenderedNode>,
    selection_index: SelectionIndex,
    visible_rows: Vec<Option<VisibleRowMap>>,
}

impl DocumentView {
    pub(crate) fn parse(source: &str) -> Result<Self> {
        let source_lines: Vec<String> = source.lines().map(ToOwned::to_owned).collect();
        let document = Document::parse(source).context("failed to parse markdown document")?;
        let rendered_nodes = build_rendered_nodes(&document, &source_lines);
        let selection_index = SelectionIndex::build(&document, &source_lines);

        Ok(Self {
            document,
            source_lines,
            rendered_nodes,
            selection_index,
            visible_rows: Vec::new(),
        })
    }

    #[cfg(test)]
    pub(crate) const fn document(&self) -> &Document {
        &self.document
    }

    #[cfg(test)]
    pub(crate) fn rendered_nodes(&self) -> &[RenderedNode] {
        &self.rendered_nodes
    }

    pub(crate) fn node_count(&self) -> usize {
        self.document.node_count()
    }

    pub(crate) fn is_block_start(&self, node_idx: usize) -> bool {
        self.document.is_block_start(node_idx)
    }

    pub(crate) fn next_content_node(&self, from: usize) -> Option<usize> {
        self.document.next_content_node(from)
    }

    pub(crate) fn prev_content_node(&self, before: usize) -> Option<usize> {
        self.document.prev_content_node(before)
    }

    pub(crate) fn navigate(&self, anchor: SelectionAnchor, forward: bool) -> NavOutcome {
        if forward {
            crate::selection::navigator::next(&self.selection_index, anchor)
        } else {
            crate::selection::navigator::prev(&self.selection_index, anchor)
        }
    }

    pub(crate) fn clamp_anchor(
        &self,
        anchor: SelectionAnchor,
        target: SelectionUnit,
    ) -> SelectionAnchor {
        crate::selection::navigator::clamp(&self.selection_index, anchor, target)
    }

    pub(crate) fn has_any_anchor(&self, unit: SelectionUnit) -> bool {
        match unit {
            SelectionUnit::Section => !self.selection_index.sections.is_empty(),
            SelectionUnit::Paragraph => !self.selection_index.paragraphs.is_empty(),
            SelectionUnit::Line => !self.selection_index.lines.is_empty(),
            SelectionUnit::Sentence => !self.selection_index.sentences.is_empty(),
            SelectionUnit::Word => !self.selection_index.words.is_empty(),
        }
    }

    pub(crate) fn section_span_for_start(&self, node_idx: usize) -> Range<usize> {
        let end = self
            .selection_index
            .sections
            .iter()
            .find(|s| s.start_node_idx == node_idx)
            .map_or_else(|| self.node_count(), |s| s.end_node_idx + 1);
        node_idx..end
    }

    pub(crate) fn sentence_count_for_node(&self, node_idx: usize) -> usize {
        self.rendered_nodes
            .get(node_idx)
            .map_or(0, |rn| rn.sentence_ranges.len())
    }

    /// Find every search hit across rendered nodes as `(node, sentence)` pairs.
    /// Smart-case: case-sensitive iff the query contains an ASCII uppercase letter.
    pub(crate) fn search_matches(&self, query: &str) -> Vec<(usize, usize)> {
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
                let sentence_idx = rn
                    .sentence_ranges
                    .iter()
                    .position(|r| abs >= r.start && abs < r.end)
                    .unwrap_or(0);
                matches.push((ni, sentence_idx));
                cursor = abs + needle.len();
            }
        }
        matches
    }
}

#[cfg(test)]
#[path = "../document_view_tests.rs"]
mod tests;
