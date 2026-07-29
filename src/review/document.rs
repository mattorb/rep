use std::ops::Range;
use std::path::Path;

use crate::selection::index::SelectionIndex;
use crate::selection::model::{NavOutcome, SelectionAnchor, SelectionUnit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // HTML construction and format-aware emit land in later stages.
pub(crate) enum DocumentFormat {
    Markdown,
    Html,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // The HTML variant is populated by HtmlReviewDocument.
pub(crate) enum SourceLocator {
    MarkdownLine {
        line: usize,
    },
    Html {
        line: usize,
        selector: String,
        text_fragment: Option<usize>,
    },
}

impl SourceLocator {
    pub(crate) const fn source_line(&self) -> usize {
        match self {
            Self::MarkdownLine { line } | Self::Html { line, .. } => *line,
        }
    }
}

/// Immutable selection facts captured at annotation creation time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapturedTarget {
    pub(crate) anchor: SelectionAnchor,
    pub(crate) text: Option<String>,
    pub(crate) locator: SourceLocator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActionContext {
    pub(crate) where_line: usize,
    pub(crate) target: String,
    pub(crate) previous: String,
    pub(crate) next: String,
    pub(crate) locator: SourceLocator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NodeSourceContext {
    pub(crate) source_line: usize,
    pub(crate) line_text: String,
    pub(crate) previous: Option<String>,
    pub(crate) next: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewLink {
    pub(crate) url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Consumed by the browser outline once the web frontend lands.
pub(crate) struct OutlineRow {
    pub(crate) node_idx: usize,
    pub(crate) level: u8,
    pub(crate) text: String,
}

/// The document-specific facts required by the shared review session.
///
/// Rendering, terminal layout, DOM ranges, and transport remain frontend
/// concerns. Implementations provide only canonical document semantics.
#[allow(dead_code)] // Some contract methods are first consumed by the web frontend.
pub(crate) trait ReviewDocument {
    fn source_path(&self) -> &Path;
    fn format(&self) -> DocumentFormat;
    fn selection_index(&self) -> &SelectionIndex;
    fn initial_anchor(&self) -> SelectionAnchor;
    fn node_count(&self) -> usize;
    fn next_content_node(&self, from: usize) -> Option<usize>;
    fn prev_content_node(&self, before: usize) -> Option<usize>;
    fn navigate(&self, anchor: SelectionAnchor, forward: bool) -> NavOutcome;
    fn clamp_anchor(&self, anchor: SelectionAnchor, target: SelectionUnit) -> SelectionAnchor;
    fn has_any_anchor(&self, unit: SelectionUnit) -> bool;
    fn section_span_for_start(&self, node_idx: usize) -> Range<usize>;
    fn sentence_count_for_node(&self, node_idx: usize) -> usize;
    fn sentence_index_for_anchor(&self, anchor: SelectionAnchor) -> Option<usize>;
    fn search_matches(&self, query: &str) -> Vec<(usize, usize)>;
    fn capture_target(&self, anchor: SelectionAnchor) -> CapturedTarget;
    fn has_target(&self, anchor: SelectionAnchor) -> bool;
    fn action_context(&self, target: &CapturedTarget) -> ActionContext;
    fn node_source_context(&self, node_idx: usize) -> NodeSourceContext;
    fn links_for(&self, anchor: SelectionAnchor) -> Vec<ReviewLink>;
    fn node_outline(&self) -> Vec<OutlineRow>;
}
