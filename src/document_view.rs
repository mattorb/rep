use std::ops::Range;

use anyhow::{Context, Result};
use ratatui::prelude::*;

use crate::document::{DocNode, Document};
use crate::markdown::{MarkdownLinkRange, render_markdown_line};
use crate::selection::index::SelectionIndex;

/// Owns the parsed source and the derived views used by app input, rendering,
/// and output. Initial migration keeps most coordinate logic at call sites;
/// later passes should move those conversions behind methods here.
#[derive(Debug)]
pub(crate) struct DocumentView {
    document: Document,
    source_lines: Vec<String>,
    rendered_nodes: Vec<RenderedNode>,
    selection_index: SelectionIndex,
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
        })
    }

    pub(crate) const fn document(&self) -> &Document {
        &self.document
    }

    pub(crate) fn source_lines(&self) -> &[String] {
        &self.source_lines
    }

    pub(crate) fn rendered_nodes(&self) -> &[RenderedNode] {
        &self.rendered_nodes
    }

    pub(crate) const fn index(&self) -> &SelectionIndex {
        &self.selection_index
    }

    pub(crate) fn node_count(&self) -> usize {
        self.document.node_count()
    }

    pub(crate) fn next_content_node(&self, from: usize) -> Option<usize> {
        self.document.next_content_node(from)
    }
}

/// Per-node rendering cache: styled spans for the joined source text plus
/// sentence byte-range boundaries within `plain`.
#[derive(Clone)]
pub(crate) struct RenderedNode {
    pub(crate) plain: String,
    pub(crate) spans: Vec<Span<'static>>,
    pub(crate) sentence_ranges: Vec<Range<usize>>,
    /// Byte ranges in `plain` for each line as the Line projection sees
    /// it. Aligned 1:1 with `SelectionIndex.nodes[i].source_line_ranges`.
    /// Empty when the node has no per-line breakdown (e.g. ThematicBreak).
    /// Populated at render time because pulldown-cmark applies
    /// smart-punctuation and emphasis-marker stripping, so a source line
    /// can't be searched verbatim inside display plain.
    pub(crate) line_ranges: Vec<Range<usize>>,
    /// Word byte ranges in display `plain` (re-segmented from display).
    /// Used by mouse-click resolution to map a clicked display byte to a
    /// word_idx; the existing index word ranges are in selection plain
    /// (markers stripped, smart-punctuation NOT applied), so their byte
    /// values don't index into display plain.
    pub(crate) display_word_ranges: Vec<Range<usize>>,
    pub(crate) links: Vec<MarkdownLinkRange>,
}

impl std::fmt::Debug for RenderedNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderedNode")
            .field("plain", &self.plain)
            .field("sentence_ranges", &self.sentence_ranges)
            .field("line_ranges", &self.line_ranges)
            .field("display_word_ranges", &self.display_word_ranges)
            .finish_non_exhaustive()
    }
}

fn build_rendered_nodes(doc: &Document, source_lines: &[String]) -> Vec<RenderedNode> {
    doc.nodes
        .iter()
        .map(|n| build_rendered_node(n, source_lines))
        .collect()
}

fn build_rendered_node(node: &DocNode, source_lines: &[String]) -> RenderedNode {
    let mut rn = match node {
        DocNode::Heading { source_line, .. } => {
            let text = source_lines.get(*source_line).cloned().unwrap_or_default();
            let r = render_markdown_line(&text);
            let ranges = single_range(&r.plain);
            let line_ranges = ranges.clone();
            RenderedNode {
                plain: r.plain,
                spans: r.spans,
                sentence_ranges: ranges,
                line_ranges,
                display_word_ranges: Vec::new(),
                links: r.links,
            }
        }
        DocNode::Paragraph {
            source_lines: range,
            ..
        } => {
            let src = &source_lines[clamp_range(range, source_lines.len())];
            let (plain, spans, links, line_ranges) = render_source_lines_with_breaks(src);
            if plain.is_empty() {
                let joined = join_node_source_lines(src);
                let r = render_markdown_line(&joined);
                let sentence_ranges = single_range(&r.plain);
                let line_ranges = sentence_ranges.clone();
                RenderedNode {
                    plain: r.plain,
                    spans: r.spans,
                    sentence_ranges,
                    line_ranges,
                    display_word_ranges: Vec::new(),
                    links: r.links,
                }
            } else {
                let sentence_ranges = crate::selection::segment::segment_sentences(&plain);
                RenderedNode {
                    plain,
                    spans,
                    sentence_ranges,
                    line_ranges,
                    display_word_ranges: Vec::new(),
                    links,
                }
            }
        }
        DocNode::ListItem {
            source_lines: range,
            ordered,
            depth,
            ..
        } => {
            let joined =
                join_node_source_lines(&source_lines[clamp_range(range, source_lines.len())]);
            let r = render_markdown_line(&joined);
            let ranges = single_range(&r.plain);
            let line_ranges = ranges.clone();
            // Top-level ordered items act as section headings - style them like one.
            let spans = if *ordered && *depth == 0 {
                let style = Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD);
                vec![Span::styled(r.plain.clone(), style)]
            } else {
                r.spans
            };
            RenderedNode {
                plain: r.plain,
                spans,
                sentence_ranges: ranges,
                line_ranges,
                display_word_ranges: Vec::new(),
                links: r.links,
            }
        }
        DocNode::CodeBlock {
            source_lines: range,
            ..
        } => {
            let raw = &source_lines[clamp_range(range, source_lines.len())];
            let plain = raw.join("\n");
            let ranges = single_range(&plain);
            let mut line_ranges = Vec::new();
            let mut cursor = 0usize;
            for line in raw {
                let is_fence = line.trim_start().starts_with("```");
                let end = cursor + line.len();
                if !is_fence {
                    line_ranges.push(cursor..end);
                }
                cursor = end + 1;
            }
            let spans = raw
                .iter()
                .flat_map(|line| {
                    let style = if line.trim_start().starts_with("```") {
                        Style::default().fg(Color::DarkGray)
                    } else {
                        Style::default().fg(Color::White).bg(Color::DarkGray)
                    };
                    [
                        Span::styled(line.clone(), style),
                        Span::raw("\n".to_owned()),
                    ]
                })
                .collect();
            RenderedNode {
                plain,
                spans,
                sentence_ranges: ranges,
                line_ranges,
                display_word_ranges: Vec::new(),
                links: vec![],
            }
        }
        DocNode::ThematicBreak { .. } => {
            let r = render_markdown_line("---");
            RenderedNode {
                plain: r.plain,
                spans: r.spans,
                sentence_ranges: vec![],
                line_ranges: vec![],
                display_word_ranges: Vec::new(),
                links: r.links,
            }
        }
    };
    rn.display_word_ranges = crate::selection::segment::segment_words(&rn.plain);
    rn
}

#[allow(clippy::single_range_in_vec_init)]
fn single_range(s: &str) -> Vec<Range<usize>> {
    if s.is_empty() {
        vec![]
    } else {
        vec![0..s.len()]
    }
}

fn join_node_source_lines(lines: &[String]) -> String {
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| if i == 0 { l.as_str() } else { l.trim() })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_source_lines_with_breaks(
    src_lines: &[String],
) -> (
    String,
    Vec<Span<'static>>,
    Vec<MarkdownLinkRange>,
    Vec<Range<usize>>,
) {
    let mut plain = String::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut links: Vec<MarkdownLinkRange> = Vec::new();
    let mut line_ranges: Vec<Range<usize>> = Vec::new();
    let first_indent = src_lines
        .first()
        .map_or(0, |l| l.len() - l.trim_start().len());
    for (i, line) in src_lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !plain.is_empty() {
            plain.push('\n');
            spans.push(Span::raw("\n".to_owned()));
        }
        let relative_indent = if i == 0 {
            0
        } else {
            let line_indent = line.len() - line.trim_start().len();
            line_indent.saturating_sub(first_indent)
        };
        if relative_indent > 0 {
            let prefix = " ".repeat(relative_indent);
            plain.push_str(&prefix);
            spans.push(Span::raw(prefix));
        }
        let line_start = plain.len() - relative_indent;
        let offset = plain.len();
        let r = render_markdown_line(trimmed);
        for link in r.links {
            links.push(MarkdownLinkRange {
                start: link.start + offset,
                end: link.end + offset,
                url: link.url,
            });
        }
        plain.push_str(&r.plain);
        spans.extend(r.spans);
        line_ranges.push(line_start..plain.len());
    }
    (plain, spans, links, line_ranges)
}

pub(crate) fn clamp_range(r: &Range<usize>, len: usize) -> Range<usize> {
    r.start.min(len)..r.end.min(len)
}
