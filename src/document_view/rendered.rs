use std::ops::Range;

use ratatui::prelude::*;
use unicode_width::UnicodeWidthChar;

use crate::document::{DocNode, Document};
use crate::markdown::{MarkdownLinkRange, render_markdown_line};

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

pub(crate) fn build_rendered_nodes(doc: &Document, source_lines: &[String]) -> Vec<RenderedNode> {
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

/// Walk `plain` alongside the wrapped output of `wrap_styled_spans` and
/// return one byte range per visible row indicating which slice of `plain`
/// that row's text occupies. Wrap drops some chars from `plain` (the joining
/// `\n`s and any leading whitespace on continuation lines), but every emitted
/// char remains a verbatim copy from the input.
pub(crate) fn wrap_line_byte_ranges(
    plain: &str,
    wrapped: &[Vec<Span<'static>>],
) -> Vec<Range<usize>> {
    let mut out = Vec::with_capacity(wrapped.len());
    let mut cursor = 0usize;
    for line in wrapped {
        let mut iter = line
            .iter()
            .flat_map(|span| span.content.chars())
            .filter(|ch| *ch != '\n');
        let Some(first) = iter.next() else {
            out.push(cursor..cursor);
            continue;
        };

        let mut start = None;
        while cursor < plain.len() {
            let Some(plain_char) = plain[cursor..].chars().next() else {
                break;
            };
            if plain_char == first {
                start = Some(cursor);
                cursor += plain_char.len_utf8();
                break;
            }
            cursor += plain_char.len_utf8();
        }
        let Some(start) = start else {
            out.push(plain.len()..plain.len());
            continue;
        };

        for ch in iter {
            while cursor < plain.len() {
                let Some(plain_char) = plain[cursor..].chars().next() else {
                    break;
                };
                if plain_char == ch {
                    cursor += plain_char.len_utf8();
                    break;
                }
                cursor += plain_char.len_utf8();
            }
        }
        out.push(start..cursor);
    }
    out
}

pub(crate) fn clamp_range(r: &Range<usize>, len: usize) -> Range<usize> {
    r.start.min(len)..r.end.min(len)
}

/// Count `\n` bytes in `plain[..byte]` - the number of source lines the
/// byte position is past inside the rendered display plain text.
pub(crate) fn newlines_before_byte(plain: &str, byte: usize) -> usize {
    plain
        .get(..byte)
        .map_or(0, |p| p.bytes().filter(|&b| b == b'\n').count())
}

/// Count occurrences of `needle` in `haystack[..before_byte]`.
pub(crate) fn count_occurrences_before(haystack: &str, needle: &str, before_byte: usize) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let stop = before_byte.min(haystack.len());
    let mut count = 0usize;
    let mut cursor = 0usize;
    while cursor < stop {
        match haystack[cursor..stop].find(needle) {
            Some(off) => {
                count += 1;
                cursor += off + needle.len();
            }
            None => break,
        }
    }
    count
}

/// Return the byte offset of the `n`th occurrence (0-indexed) of `needle`.
pub(crate) fn nth_occurrence(haystack: &str, needle: &str, n: usize) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let mut cursor = 0usize;
    for i in 0..=n {
        let off = haystack[cursor..].find(needle)?;
        let abs = cursor + off;
        if i == n {
            return Some(abs);
        }
        cursor = abs + needle.len();
    }
    None
}

/// Pick the unit_idx whose range contains `byte`, or when `byte` falls in a
/// gap, the closest preceding unit.
pub(crate) fn find_unit_at(ranges: &[Range<usize>], byte: usize) -> Option<usize> {
    if ranges.is_empty() {
        return None;
    }
    let count = ranges.partition_point(|r| r.start <= byte);
    Some(count.saturating_sub(1))
}

/// Walk `text` from byte 0 and return the byte index whose preceding chars sum
/// to strictly more than `target_cols` terminal columns.
pub(crate) fn col_to_byte(text: &str, target_cols: usize) -> usize {
    let mut used = 0usize;
    for (idx, ch) in text.char_indices() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w == 0 {
            continue;
        }
        if used >= target_cols {
            return idx;
        }
        used += w;
    }
    text.len()
}
